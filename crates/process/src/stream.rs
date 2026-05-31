//! Streaming process I/O: read stdout as it is produced and write stdin
//! incrementally, instead of buffering everything until the process exits.
//!
//! [`Exec::stream`] spawns the child inside a [`Job`] (kill-on-close still holds),
//! hands back a [`Streaming`] with a live stdout reader and an optional open stdin
//! writer, and drains stderr in a background task so a caller reading only stdout
//! can never deadlock on a full stderr pipe. stderr is returned, fully collected,
//! by [`Streaming::finish`].

use std::io;
use std::process::ExitStatus;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader, copy, sink};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::task::JoinHandle;

use crate::{Child, Exec, Job};

impl Exec {
    /// Spawn the command for **streaming** I/O: read stdout live via
    /// [`Streaming::next_line`] / [`Streaming::stdout`] and, with
    /// [`pipe_stdin`](Exec::pipe_stdin), write stdin incrementally via
    /// [`Streaming::stdin`]. stderr is drained in the background and returned by
    /// [`Streaming::finish`].
    ///
    /// The child runs inside a [`Job`], so dropping the returned [`Streaming`]
    /// kills it (kill-on-close). The builder's [`timeout`](Exec::timeout) does
    /// **not** apply here — drive timing yourself (wrap reads in
    /// [`tokio::time::timeout`], or call [`Streaming::job`]`().kill_all()`). Any
    /// bytes from [`Exec::stdin`] are ignored; use `pipe_stdin` + the writer.
    ///
    /// ```no_run
    /// # async fn example() -> std::io::Result<()> {
    /// let mut s = vcs_process::Exec::new("git").args(["log"]).stream().await?;
    /// while let Some(line) = s.next_line().await? {
    ///     // each line arrives as git emits it, before the process exits
    ///     let _ = line;
    /// }
    /// let (status, stderr) = s.finish().await?;
    /// # let _ = (status, stderr); Ok(()) }
    /// ```
    pub async fn stream(self) -> io::Result<Streaming> {
        let job = Job::new()?;
        let mut cmd = self.build();
        let mut child = job.spawn(&mut cmd)?;
        // build() always pipes both stdout and stderr.
        let stdout = child
            .stdout()
            .expect("stream(): stdout is always piped by build()");
        let stderr = child
            .stderr()
            .expect("stream(): stderr is always piped by build()");
        let stdin = child.stdin(); // Some only when pipe_stdin()/stdin() was set
        Ok(Streaming::new(job, child, stdout, stderr, stdin))
    }
}

/// A spawned child wired for streaming I/O — see [`Exec::stream`].
///
/// Owns the [`Job`] (kill-on-close), a buffered stdout reader, an optional open
/// stdin writer, and a background task collecting stderr.
pub struct Streaming {
    job: Job,
    child: Child,
    stdout: BufReader<ChildStdout>,
    stdin: Option<ChildStdin>,
    stderr: JoinHandle<Vec<u8>>,
}

impl Streaming {
    fn new(
        job: Job,
        child: Child,
        stdout: ChildStdout,
        stderr: ChildStderr,
        stdin: Option<ChildStdin>,
    ) -> Self {
        // Drain stderr concurrently so a caller reading only stdout never blocks
        // the child on a full stderr pipe.
        let stderr = tokio::spawn(async move {
            let mut buf = Vec::new();
            let mut pipe = stderr;
            let _ = pipe.read_to_end(&mut buf).await;
            buf
        });
        Streaming {
            job,
            child,
            stdout: BufReader::new(stdout),
            stdin,
            stderr,
        }
    }

    /// The open stdin writer, present when the command was built with
    /// [`Exec::pipe_stdin`]. Write incrementally; [`close_stdin`](Streaming::close_stdin)
    /// or [`finish`](Streaming::finish) sends EOF. Implements [`tokio::io::AsyncWrite`].
    pub fn stdin(&mut self) -> Option<&mut ChildStdin> {
        self.stdin.as_mut()
    }

    /// The buffered stdout reader, for live byte / [`AsyncRead`](tokio::io::AsyncRead)
    /// access (e.g. binary output). For line-oriented output prefer
    /// [`next_line`](Streaming::next_line).
    pub fn stdout(&mut self) -> &mut BufReader<ChildStdout> {
        &mut self.stdout
    }

    /// Read the next line of stdout as it arrives (trailing `\n` / `\r\n`
    /// stripped); `None` at end of output. Returns before the process exits.
    pub async fn next_line(&mut self) -> io::Result<Option<String>> {
        let mut line = String::new();
        if self.stdout.read_line(&mut line).await? == 0 {
            return Ok(None);
        }
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        Ok(Some(line))
    }

    /// Close the stdin pipe, sending EOF to a child waiting for input. No-op if
    /// stdin was not piped or is already closed.
    pub fn close_stdin(&mut self) {
        self.stdin = None;
    }

    /// The owning [`Job`] — e.g. `streaming.job().kill_all()` to stop early.
    pub fn job(&self) -> &Job {
        &self.job
    }

    /// Close stdin, wait for the process to exit, and return its status together
    /// with the fully-drained stderr (lossily decoded as UTF-8). The job is
    /// dropped on return (kill-on-close).
    pub async fn finish(mut self) -> io::Result<(ExitStatus, String)> {
        self.stdin = None; // EOF, so a child reading stdin can finish
        // Drain any stdout the caller didn't read, so the child can never wedge
        // on a full stdout pipe and hang `wait()` (stderr is already drained in
        // the background). The remainder is discarded — `finish` returns stderr.
        let _ = copy(&mut self.stdout, &mut sink()).await;
        let status = self.child.wait().await?;
        let stderr = self.stderr.await.unwrap_or_default();
        Ok((status, String::from_utf8_lossy(&stderr).into_owned()))
    }
}
