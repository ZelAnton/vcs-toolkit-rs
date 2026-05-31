//! A small generic client core shared by the CLI wrappers (`vcs-git`, `vcs-jj`,
//! `vcs-github`). It owns the binary name, the [`Runner`], and an optional
//! default timeout, hands back preconfigured [`Exec`] builders, and provides the
//! terminal run/parse helpers every wrapper otherwise repeats. A wrapper then
//! reduces to a typed facade over its parsers, with no process-plumbing
//! boilerplate — and adding a fourth wrapper is just a `const BINARY`, a `core`
//! field, three constructors, and the typed methods.
//!
//! All the generic, ergonomic argument types live here, never on the wrappers'
//! object-safe `*Api` traits, so `&dyn GitApi`, `#[async_trait]`, and `mockall`
//! keep working.

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::time::Duration;

use crate::{Exec, JobRunner, Output, Result, Runner};

/// Owns the binary name, runner, and default timeout for a CLI wrapper, and
/// builds/executes [`Exec`]s against them. Generic over the [`Runner`] so tests
/// can inject a fake; [`CliClient::new`] uses the real job-backed runner.
pub struct CliClient<R: Runner = JobRunner> {
    binary: &'static str,
    runner: R,
    timeout: Option<Duration>,
}

impl CliClient<JobRunner> {
    /// A client driving `binary` through the real job-backed runner.
    pub fn new(binary: &'static str) -> Self {
        CliClient {
            binary,
            runner: JobRunner,
            timeout: None,
        }
    }
}

impl<R: Runner> CliClient<R> {
    /// A client driving `binary` through `runner` — pass a fake in tests.
    pub fn with_runner(binary: &'static str, runner: R) -> Self {
        CliClient {
            binary,
            runner,
            timeout: None,
        }
    }

    /// Apply a default timeout to every command this client builds.
    pub fn default_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// The injected runner — for commands that need the raw [`Runner`] seam
    /// (e.g. [`Exec::code_with`]).
    pub fn runner(&self) -> &R {
        &self.runner
    }

    /// The default timeout, if one was set.
    pub fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// An [`Exec`] for `binary <args>` in the current directory, default timeout
    /// pre-applied. Chain more [`Exec`] builders (`.arg`, `.stdin`, …) for the
    /// dynamic-argument commands.
    pub fn exec<I, S>(&self, args: I) -> Exec
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Exec::new(self.binary)
            .maybe_timeout(self.timeout)
            .args(args)
    }

    /// An [`Exec`] for `binary <args>` run in `dir`, default timeout pre-applied.
    pub fn exec_in<I, S>(&self, dir: &Path, args: I) -> Exec
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Exec::new(self.binary)
            .maybe_timeout(self.timeout)
            .current_dir(dir)
            .args(args)
    }

    /// Run `exec`, returning trimmed stdout on success (errors on a non-zero exit).
    pub async fn run_text(&self, exec: Exec) -> Result<String> {
        Ok(exec
            .checked_with(&self.runner)
            .await?
            .stdout
            .trim()
            .to_string())
    }

    /// Run `exec`, capturing [`Output`] without erroring on a non-zero exit.
    pub async fn run_raw(&self, exec: Exec) -> io::Result<Output> {
        exec.output_with(&self.runner).await
    }

    /// Run `exec` for its side effect, discarding stdout (errors on a non-zero exit).
    pub async fn run_unit(&self, exec: Exec) -> Result<()> {
        exec.checked_with(&self.runner).await.map(drop)
    }

    /// Run `exec` (errors on a non-zero exit) and feed its stdout to an infallible
    /// `parse` — the shape of git/jj's struct-returning commands.
    pub async fn parsed<T>(&self, exec: Exec, parse: impl FnOnce(&str) -> T) -> Result<T> {
        let out = exec.checked_with(&self.runner).await?;
        Ok(parse(&out.stdout))
    }

    /// Run `exec` (errors on a non-zero exit) and feed its stdout to a *fallible*
    /// `parse` — the shape of github's JSON deserialization, where a parse error
    /// becomes a [`CommandError::Parse`](crate::CommandError::Parse).
    pub async fn parsed_try<T>(
        &self,
        exec: Exec,
        parse: impl FnOnce(&str) -> Result<T>,
    ) -> Result<T> {
        let out = exec.checked_with(&self.runner).await?;
        parse(&out.stdout)
    }
}
