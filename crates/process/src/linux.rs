//! Linux implementation: a [cgroup v2] killed via `cgroup.kill`, with a POSIX
//! process-group fallback when no writable cgroup is available (e.g. a CI runner
//! without cgroup delegation).
//!
//! [cgroup v2]: https://docs.kernel.org/admin-guide/cgroup-v2.html

use std::ffi::{CStr, CString};
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::Mechanism;

/// Process-wide counter so concurrent jobs get distinct cgroup names.
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

pub struct Job {
    backend: Backend,
}

enum Backend {
    /// All children live in this cgroup; killed via `cgroup.kill`.
    Cgroup(Cgroup),
    /// Fallback: each spawned child leads its own process group; we track the
    /// group ids (== child pids) and `killpg` them on teardown.
    ProcessGroup(Mutex<Vec<i32>>),
}

impl Job {
    pub fn new() -> io::Result<Self> {
        // Prefer a cgroup; degrade to a process group if we can't make one
        // (no cgroup v2, no delegation, read-only fs, …). The choice is
        // observable via `mechanism()` — never silent.
        let backend = match Cgroup::create() {
            Ok(cg) => Backend::Cgroup(cg),
            Err(_) => Backend::ProcessGroup(Mutex::new(Vec::new())),
        };
        Ok(Job { backend })
    }

    pub fn spawn(&self, cmd: &mut Command) -> io::Result<std::process::Child> {
        match &self.backend {
            Backend::Cgroup(cg) => {
                let procs = CString::new(cg.path.join("cgroup.procs").into_os_string().into_vec())
                    .map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "cgroup path contains NUL")
                    })?;
                // Join the cgroup in the forked child *before* exec, so there is
                // no window in which the child (or its children) escape the
                // cgroup. The closure only makes async-signal-safe libc calls.
                // SAFETY: see `write_self_pid`.
                unsafe {
                    cmd.pre_exec(move || write_self_pid(procs.as_c_str()));
                }
                cmd.spawn()
            }
            Backend::ProcessGroup(pgids) => {
                // Own process group per child → killpg reaps it and its
                // descendants. setpgid(0, 0): the child becomes group leader.
                cmd.process_group(0);
                let child = cmd.spawn()?;
                if let Ok(mut g) = pgids.lock() {
                    g.push(child.id() as i32);
                }
                Ok(child)
            }
        }
    }

    pub fn kill_all(&self) -> io::Result<()> {
        match &self.backend {
            Backend::Cgroup(cg) => cg.kill(),
            Backend::ProcessGroup(pgids) => {
                kill_groups(pgids);
                Ok(())
            }
        }
    }

    pub fn mechanism(&self) -> Mechanism {
        match &self.backend {
            Backend::Cgroup(_) => Mechanism::CgroupV2,
            Backend::ProcessGroup(_) => Mechanism::ProcessGroup,
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        match &self.backend {
            Backend::Cgroup(cg) => {
                let _ = cg.kill();
                // Best-effort: an emptied cgroup dir can be removed.
                let _ = std::fs::remove_dir(&cg.path);
            }
            Backend::ProcessGroup(pgids) => kill_groups(pgids),
        }
    }
}

/// SIGKILL every tracked process group (fallback teardown).
///
/// Caveat of this fallback (the cgroup path doesn't share it): a group id is the
/// leader's pid, so if the leader was already reaped and its pid recycled before
/// we fire, `killpg` could in theory hit an unrelated group. The window is a few
/// instructions wide, so this is accepted for the no-cgroup degraded path.
fn kill_groups(pgids: &Mutex<Vec<i32>>) {
    if let Ok(g) = pgids.lock() {
        for &pgid in g.iter() {
            // SAFETY: killpg on a positive group id is always a sound call; a
            // group that's already gone simply returns ESRCH.
            unsafe {
                libc::killpg(pgid, libc::SIGKILL);
            }
        }
    }
}

struct Cgroup {
    path: PathBuf,
}

impl Cgroup {
    fn create() -> io::Result<Self> {
        // Only the cgroup v2 unified hierarchy exposes this file at the root.
        let root = Path::new("/sys/fs/cgroup");
        if !root.join("cgroup.controllers").exists() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "cgroup v2 not mounted",
            ));
        }

        // Our own cgroup: on v2 `/proc/self/cgroup` is a single `0::<path>` line.
        let self_cgroup = std::fs::read_to_string("/proc/self/cgroup")?;
        let rel = self_cgroup
            .lines()
            .find_map(|line| line.strip_prefix("0::"))
            .unwrap_or("/")
            .trim();
        let parent = root.join(rel.trim_start_matches('/'));

        let name = format!(
            "vcs-job-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        let path = parent.join(name);
        // No controllers enabled — `cgroup.kill` needs none, and that sidesteps
        // the "no internal processes" rule. mkdir is the permission gate that
        // triggers the process-group fallback when delegation is absent.
        std::fs::create_dir(&path)?;
        Ok(Cgroup { path })
    }

    fn kill(&self) -> io::Result<()> {
        // `cgroup.kill` (kernel ≥ 5.14): write "1" to SIGKILL the whole subtree
        // atomically.
        if std::fs::write(self.path.join("cgroup.kill"), b"1").is_ok() {
            return Ok(());
        }
        // Older kernels: SIGKILL each member until the cgroup drains. Bounded so
        // teardown (incl. Drop) can never hang on un-reaped zombies.
        for _ in 0..100 {
            let procs = std::fs::read_to_string(self.path.join("cgroup.procs"))?;
            let mut any = false;
            for pid in procs.lines().filter_map(|l| l.trim().parse::<i32>().ok()) {
                any = true;
                // SAFETY: a plain SIGKILL to a pid read from cgroup.procs; a
                // race where the pid already exited just yields ESRCH.
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
            if !any {
                break;
            }
        }
        Ok(())
    }
}

/// Append the calling process's own pid to the opened `cgroup.procs`, joining
/// the cgroup. Runs in the forked child after `fork()` and before `exec()`.
///
/// # Safety
///
/// Must stay async-signal-safe: it only calls `open`/`getpid`/`write`/`close`
/// and formats the pid into a stack buffer — no allocation, no locks.
fn write_self_pid(path: &CStr) -> io::Result<()> {
    // SAFETY: all calls below are async-signal-safe and operate on a valid,
    // NUL-terminated path; the fd is closed on every return path.
    unsafe {
        let fd = libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        // Format the (positive) pid as decimal into a stack buffer.
        let mut buf = [0u8; 12];
        let mut i = buf.len();
        let mut v = libc::getpid() as u32;
        loop {
            i -= 1;
            buf[i] = b'0' + (v % 10) as u8;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        let bytes = &buf[i..];

        let written = libc::write(fd, bytes.as_ptr().cast(), bytes.len());
        let werr = io::Error::last_os_error();
        libc::close(fd);
        if written < 0 {
            return Err(werr);
        }
        Ok(())
    }
}
