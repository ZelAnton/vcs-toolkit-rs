//! Integration tests for `vcs-process`: real subprocesses exercise `Exec`
//! options and the job's kill-on-close guarantee across backends.
//!
//! All ignored by default (they spawn real processes); run with
//! `cargo test -p vcs-process -- --ignored`.

mod common;

use std::process::Stdio;
use std::time::{Duration, Instant};

use common::TempDir;
use tokio::process::Command;
use vcs_process::{Exec, Job};

/// A ~30s sleeper command with output suppressed, per platform.
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
async fn exec_runs_in_current_dir() {
    let tmp = TempDir::new("cwd");
    let segment = tmp
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();

    let out = if cfg!(windows) {
        Exec::new("cmd").args(["/c", "cd"])
    } else {
        // `-P` forces the physical path: `current_dir` chdir()s but doesn't update
        // the inherited `PWD`, so plain `pwd` could print the parent's directory.
        Exec::new("pwd").arg("-P")
    }
    .current_dir(tmp.path())
    .run()
    .await
    .expect("run in cwd");

    assert!(
        out.to_lowercase().contains(&segment.to_lowercase()),
        "cwd output {out:?} should mention {segment:?}"
    );
}

#[tokio::test]
#[ignore = "spawns a real subprocess"]
async fn exec_passes_env() {
    let exec = if cfg!(windows) {
        Exec::new("cmd").args(["/c", "echo %VCS_TEST_ENV%"])
    } else {
        Exec::new("sh").args(["-c", "printf %s \"$VCS_TEST_ENV\""])
    };
    let out = exec
        .env("VCS_TEST_ENV", "hello-env")
        .run()
        .await
        .expect("run with env");
    assert!(out.contains("hello-env"), "got {out:?}");
}

#[tokio::test]
#[ignore = "spawns a real subprocess"]
async fn exec_feeds_stdin() {
    // `sort` exists on both Windows and Unix and echoes a single line unchanged.
    let out = Exec::new("sort")
        .stdin("ping-pong")
        .run()
        .await
        .expect("feed stdin to sort");
    assert_eq!(out, "ping-pong");
}

#[tokio::test]
#[ignore = "spawns a real subprocess"]
async fn exec_spawn_feeds_and_closes_stdin() {
    // Regression guard: spawn() must write the buffered stdin and close the pipe.
    let (_job, child) = Exec::new("sort")
        .stdin("ping-pong")
        .spawn()
        .await
        .expect("spawn sort");
    let out = child.wait_with_output().await.expect("wait");
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("ping-pong"),
        "stdin was not fed through to stdout"
    );
}

#[tokio::test]
#[ignore = "spawns a real subprocess"]
async fn output_captures_nonzero_exit() {
    let out = if cfg!(windows) {
        Exec::new("cmd").args(["/c", "exit 3"])
    } else {
        Exec::new("sh").args(["-c", "exit 3"])
    }
    .output()
    .await
    .expect("output() should not error on non-zero exit");

    assert!(!out.success());
    assert!(!out.timed_out());
    assert_eq!(out.code(), Some(3));
}

#[tokio::test]
#[ignore = "spawns a real subprocess and waits for the timeout"]
async fn timeout_flags_and_kills() {
    let out = Exec::new(if cfg!(windows) { "cmd" } else { "sleep" })
        .args(if cfg!(windows) {
            vec!["/c", "ping", "-n", "30", "127.0.0.1"]
        } else {
            vec!["30"]
        })
        .timeout(Duration::from_millis(300))
        .output()
        .await
        .expect("output");
    assert!(out.timed_out(), "should be flagged timed out");
    assert!(!out.success());
}

// Regression guard for the documented guarantee that the timeout covers the
// *stdin write* too: feed more than a pipe buffer's worth of input to a child
// that never reads it. The blocking write_all must not outlive the deadline —
// it has to be cancelled and the job killed, not hang.
#[tokio::test]
#[ignore = "spawns a real subprocess and waits for the timeout"]
async fn timeout_cancels_a_blocking_stdin_write() {
    // 1 MiB dwarfs the OS pipe buffer, so write_all blocks once the buffer fills.
    let payload = vec![b'x'; 1024 * 1024];
    let start = Instant::now();
    let out = Exec::new(if cfg!(windows) { "cmd" } else { "sleep" })
        .args(if cfg!(windows) {
            vec!["/c", "ping", "-n", "30", "127.0.0.1"]
        } else {
            vec!["30"]
        })
        .stdin(payload)
        .timeout(Duration::from_millis(300))
        .output()
        .await
        .expect("output");
    assert!(
        out.timed_out(),
        "blocking stdin write should still time out"
    );
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "stdin write hung past the deadline instead of being cancelled"
    );
}

// The core streaming guarantee: stdout is readable *as it is produced*, not only
// after the process exits. The child prints line1, sleeps, then line2 — so line1
// must arrive well before the full run completes.
#[tokio::test]
#[ignore = "spawns a real subprocess"]
async fn stream_reads_stdout_before_exit() {
    let exec = if cfg!(windows) {
        Exec::new("cmd").args(["/c", "echo line1&ping -n 3 127.0.0.1 >NUL&echo line2"])
    } else {
        Exec::new("sh").args(["-c", "echo line1; sleep 2; echo line2"])
    };
    let start = Instant::now();
    let mut s = exec.stream().await.expect("stream");

    let first = s.next_line().await.expect("read line1");
    let elapsed = start.elapsed();
    assert_eq!(first.as_deref(), Some("line1"));
    assert!(
        elapsed < Duration::from_millis(1500),
        "line1 should arrive immediately, not after the full run (took {elapsed:?})"
    );

    assert_eq!(
        s.next_line().await.expect("read line2").as_deref(),
        Some("line2")
    );
    assert!(s.next_line().await.expect("eof").is_none());
    let (status, _stderr) = s.finish().await.expect("finish");
    assert!(status.success());
}

// The open async stdin writer: write through `pipe_stdin`, close it (EOF), and
// read the result back over the streaming reader.
#[tokio::test]
#[ignore = "spawns a real subprocess"]
async fn stream_writes_stdin_then_reads_output() {
    use tokio::io::AsyncWriteExt;
    // `sort` (present on both platforms) reads stdin to EOF, then emits.
    let mut s = Exec::new("sort")
        .pipe_stdin()
        .stream()
        .await
        .expect("stream");
    s.stdin()
        .expect("stdin should be piped")
        .write_all(b"hello\n")
        .await
        .expect("write stdin");
    s.close_stdin();
    assert_eq!(s.next_line().await.expect("read").as_deref(), Some("hello"));
    let (status, _stderr) = s.finish().await.expect("finish");
    assert!(status.success());
}

// `finish` returns the exit status and the background-drained stderr — proving
// stderr is captured without the caller draining it (no deadlock).
#[tokio::test]
#[ignore = "spawns a real subprocess"]
async fn stream_finish_returns_status_and_stderr() {
    let exec = if cfg!(windows) {
        Exec::new("cmd").args(["/c", "echo out&echo err 1>&2&exit 3"])
    } else {
        Exec::new("sh").args(["-c", "echo out; echo err 1>&2; exit 3"])
    };
    let mut s = exec.stream().await.expect("stream");
    assert_eq!(
        s.next_line().await.expect("read out").as_deref(),
        Some("out")
    );
    assert!(s.next_line().await.expect("eof").is_none());

    let (status, stderr) = s.finish().await.expect("finish");
    assert_eq!(status.code(), Some(3));
    assert!(
        stderr.contains("err"),
        "stderr should be captured: {stderr:?}"
    );
}

// Regression guard: the child writes far more than a pipe buffer (~64 KB) to
// stdout, which we never read. If `finish` didn't drain it, the child would
// block on the full pipe and `wait()` would hang forever. The outer timeout
// turns a regression into a fast failure instead of a hung suite.
#[tokio::test]
#[ignore = "spawns a real subprocess"]
async fn stream_finish_drains_unread_stdout() {
    let exec = if cfg!(windows) {
        Exec::new("powershell").args([
            "-NoProfile",
            "-Command",
            "[Console]::Out.Write('x' * 200000)",
        ])
    } else {
        Exec::new("sh").args(["-c", "head -c 200000 /dev/zero"])
    };
    let s = exec.stream().await.expect("stream");
    let (status, _stderr) = tokio::time::timeout(Duration::from_secs(10), s.finish())
        .await
        .expect("finish must not hang on unread stdout")
        .expect("finish");
    assert!(status.success());
}

#[tokio::test]
#[ignore = "spawns long-lived subprocesses and asserts kill-on-close"]
async fn job_kills_multiple_children() {
    // Job kill-on-close only exists on the containment platforms (the `other`
    // path has no job to kill).
    if cfg!(not(any(windows, target_os = "linux"))) {
        return;
    }
    let job = Job::new().expect("job");
    let mut a = job.spawn(&mut sleeper()).expect("spawn a");
    let mut b = job.spawn(&mut sleeper()).expect("spawn b");
    assert!(a.try_wait().unwrap().is_none() && b.try_wait().unwrap().is_none());

    drop(job); // kill-on-close should reap both

    let start = Instant::now();
    loop {
        let a_done = a.try_wait().unwrap().is_some();
        let b_done = b.try_wait().unwrap().is_some();
        if a_done && b_done {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "children outlived the job"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// The defining stray case: a grandchild that outlives its parent must still die
// with the job. Linux-only — `sh ... &` cleanly backgrounds a sleeper and exposes
// its pid, and /proc gives a dependency-free liveness check. On Windows the Job
// Object kills the whole tree by construction (covered by the multi-child test);
// macOS/BSD use the no-containment `other` path, so there is nothing to assert.
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "spawns a detached grandchild and asserts it is killed"]
async fn stray_grandchild_killed_on_drop() {
    let tmp = TempDir::new("grandchild");
    let pidfile = tmp.path().join("pid");

    let script = format!("sleep 30 & echo $! > {}", pidfile.display());
    let (job, mut child) = Exec::new("sh")
        .args(["-c", &script])
        .spawn()
        .await
        .expect("spawn parent shell");
    child.wait().await.expect("parent shell exits");

    let pid: u32 = std::fs::read_to_string(&pidfile)
        .expect("pidfile")
        .trim()
        .parse()
        .expect("pid");
    assert!(is_alive(pid), "grandchild should be running before drop");

    drop(job); // kill-on-close should reach the reparented grandchild

    let start = Instant::now();
    while is_alive(pid) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "grandchild {pid} outlived the job"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(target_os = "linux")]
fn is_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}
