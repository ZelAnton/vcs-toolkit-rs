//! `vcs-process` — launch child processes inside an OS *job* so the whole
//! process tree dies with the parent. No orphaned `git`/`jj`/`gh` descendants
//! survive a crashing or exiting parent.
//!
//! Async (tokio): every run returns a future. The containment mechanism is
//! platform-specific (see [`Mechanism`]):
//!
//! - **Windows**: a [Job Object] with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
//! - **Linux**: a [cgroup v2] killed via `cgroup.kill`, falling back to a POSIX
//!   process group when no writable cgroup is available.
//! - **other**: a plain spawn with no containment.
//!
//! v1 guarantees **kill-on-close**: terminating or dropping a [`Job`] kills
//! every process still inside it. [`Exec::timeout`] adds a deadline that kills
//! the job. Errors are reported as the structured [`CommandError`].
//!
//! [Job Object]: https://learn.microsoft.com/windows/win32/procthread/job-objects
//! [cgroup v2]: https://docs.kernel.org/admin-guide/cgroup-v2.html

use std::ffi::OsStr;
use std::io;

use tokio::process::Command;

// One platform module is compiled per target; each exposes the same `Job` shape
// (`new`/`spawn`/`kill_all`/`mechanism` + a kill-on-close `Drop`).
#[cfg_attr(windows, path = "windows.rs")]
#[cfg_attr(target_os = "linux", path = "linux.rs")]
#[cfg_attr(not(any(windows, target_os = "linux")), path = "other.rs")]
mod imp;

mod client;
pub use client::CliClient;

mod error;
pub use error::{CommandError, Result};

mod exec;
pub use exec::{Exec, Output, Termination};

mod runner;
#[cfg(feature = "mock")]
pub use runner::MockRunner;
pub use runner::{Invocation, JobRunner, RecordingRunner, Runner, ScriptedRunner};

/// Which OS mechanism a [`Job`] is actually using to contain its processes.
///
/// Surfaced so callers can tell when Linux silently fell back from a cgroup to a
/// process group (e.g. on a CI runner without cgroup delegation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Mechanism {
    /// Windows Job Object with kill-on-close.
    JobObject,
    /// Linux cgroup v2 torn down via `cgroup.kill`.
    CgroupV2,
    /// POSIX process group — the Linux fallback when no cgroup is writable.
    ProcessGroup,
    /// No containment: the child is spawned directly (non-Windows/Linux).
    None,
}

/// A handle to an OS job that owns a tree of child processes.
///
/// Dropping the `Job` kills every process still inside it (kill-on-close), so an
/// exiting or panicking parent never leaks subprocesses.
pub struct Job(imp::Job);

impl Job {
    /// Create a fresh, empty job.
    pub fn new() -> io::Result<Self> {
        imp::Job::new().map(Job)
    }

    /// Spawn `cmd` as a member of this job and return its handle.
    ///
    /// The child — and any process it later spawns — belongs to the job and is
    /// reaped when the job is killed or dropped. Spawning itself is synchronous;
    /// await the returned [`Child`] to drive it.
    pub fn spawn(&self, cmd: &mut Command) -> io::Result<Child> {
        self.0.spawn(cmd).map(Child)
    }

    /// Kill every process currently in the job. Idempotent.
    pub fn kill_all(&self) -> io::Result<()> {
        self.0.kill_all()
    }

    /// The containment mechanism actually in effect (see [`Mechanism`]).
    pub fn mechanism(&self) -> Mechanism {
        self.0.mechanism()
    }
}

/// A child process spawned into a [`Job`]. Thin wrapper over
/// [`tokio::process::Child`].
pub struct Child(tokio::process::Child);

impl Child {
    /// Wait for the process to exit.
    pub async fn wait(&mut self) -> io::Result<std::process::ExitStatus> {
        self.0.wait().await
    }

    /// Wait for exit and collect captured stdout/stderr (consumes the handle).
    pub async fn wait_with_output(self) -> io::Result<std::process::Output> {
        self.0.wait_with_output().await
    }

    /// Check whether the process has exited yet, without blocking.
    pub fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.0.try_wait()
    }

    /// Send a kill signal without waiting for the process to exit. The job still
    /// governs the rest of the tree.
    pub fn start_kill(&mut self) -> io::Result<()> {
        self.0.start_kill()
    }

    /// Borrow the underlying [`tokio::process::Child`] (e.g. for its stdio pipes).
    pub fn inner_mut(&mut self) -> &mut tokio::process::Child {
        &mut self.0
    }
}

/// Run `binary <args>` inside a one-shot job and return trimmed stdout on
/// success, or a [`CommandError`] on a non-zero exit / timeout / spawn failure.
///
/// A thin shim over [`Exec`]; use the builder directly for a working directory,
/// env vars, stdin input, or a timeout.
pub async fn run<I, S>(binary: &str, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Exec::new(binary).args(args).run().await
}

/// Run `binary <args>` inside a one-shot job and capture its [`Output`] without
/// erroring on a non-zero exit — for commands whose exit code is meaningful.
pub async fn output<I, S>(binary: &str, args: I) -> io::Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Exec::new(binary).args(args).output().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use std::time::Duration;

    // --- Hermetic unit tests (no subprocess) -------------------------------

    #[test]
    fn output_helpers_reflect_status() {
        let ok = Output::ok("  value\n");
        assert!(ok.success());
        assert_eq!(ok.combined(), "  value\n");

        let bad = Output::fail(1, "boom");
        assert!(!bad.success());
        assert_eq!(bad.stderr, "boom");
    }

    // --- Subprocess tests (ignored on CI) ----------------------------------

    // A ~30s sleeper command with output suppressed, per platform.
    fn sleeper() -> Command {
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/c", "ping", "-n", "30", "127.0.0.1"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        };
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        cmd
    }

    #[tokio::test]
    #[ignore = "spawns a real subprocess"]
    async fn run_captures_stdout() {
        let out = run("cargo", ["--version"])
            .await
            .expect("cargo should be installed");
        assert!(out.to_lowercase().contains("cargo"), "unexpected: {out}");
    }

    #[tokio::test]
    #[ignore = "creates an OS job object / cgroup"]
    async fn job_reports_a_known_mechanism() {
        let job = Job::new().expect("job creation should succeed");
        assert!(matches!(
            job.mechanism(),
            Mechanism::JobObject | Mechanism::CgroupV2 | Mechanism::ProcessGroup | Mechanism::None
        ));
    }

    #[tokio::test]
    #[ignore = "spawns a long-lived subprocess and asserts kill-on-close"]
    async fn dropping_job_kills_children() {
        // Job kill-on-close only exists on the containment platforms; the `other`
        // path (macOS/BSD) has no job to kill, so this can't be asserted there.
        if cfg!(not(any(windows, target_os = "linux"))) {
            return;
        }
        let job = Job::new().expect("job creation");
        let mut child = job.spawn(&mut sleeper()).expect("spawn sleeper");
        assert!(
            child.try_wait().expect("try_wait").is_none(),
            "sleeper should still be running right after spawn"
        );

        drop(job); // kill-on-close should reap the child promptly

        let start = std::time::Instant::now();
        while child.try_wait().expect("try_wait").is_none() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "child outlived its job — kill-on-close did not fire"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    #[ignore = "spawns a real subprocess and waits for the timeout"]
    async fn timeout_kills_and_flags() {
        let program = if cfg!(windows) { "cmd" } else { "sleep" };
        let exec = if cfg!(windows) {
            Exec::new(program).args(["/c", "ping", "-n", "30", "127.0.0.1"])
        } else {
            Exec::new(program).arg("30")
        }
        .timeout(Duration::from_millis(300));

        let out = exec.output().await.expect("output");
        assert!(out.timed_out(), "should be flagged as timed out");
        assert!(!out.success());
    }
}
