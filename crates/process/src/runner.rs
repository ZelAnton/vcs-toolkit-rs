//! The execution boundary as an async trait, so consumers can inject a fake
//! process runner in tests instead of spawning real binaries.
//!
//! - [`JobRunner`] is the real, job-backed runner (the default).
//! - [`ScriptedRunner`] is a dependency-free test double: map a command to a
//!   canned [`Output`] by argument prefix or by an arbitrary predicate.
//! - [`RecordingRunner`] wraps any runner and captures every [`Exec`] as an
//!   [`Invocation`] for exact post-hoc assertions (full args, cwd, env, stdin —
//!   and flag *absence*, which prefix matching can't express).
//! - With the `mock` feature, `mockall` also generates a `MockRunner`.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::{Exec, Output};

/// Runs a prepared [`Exec`] and returns its captured [`Output`].
///
/// Wrapper crates execute every command through a `Runner`, so a test can pass a
/// [`ScriptedRunner`] (or a `mockall` `MockRunner`) and exercise the real
/// argument-building and parsing without touching git/jj/gh.
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait::async_trait]
pub trait Runner: Send + Sync {
    /// Execute `exec` and capture its result.
    async fn run(&self, exec: &Exec) -> io::Result<Output>;
}

/// A shared reference to a runner is itself a runner — lets a test hand `&runner`
/// to a client (`with_runner(&rec)`) while keeping ownership to inspect it after.
#[async_trait::async_trait]
impl<R: Runner + ?Sized> Runner for &R {
    async fn run(&self, exec: &Exec) -> io::Result<Output> {
        (**self).run(exec).await
    }
}

/// The real runner: spawns the process inside a job (kill-on-close). The default
/// everywhere a `Runner` isn't explicitly supplied.
#[derive(Debug, Default, Clone, Copy)]
pub struct JobRunner;

#[async_trait::async_trait]
impl Runner for JobRunner {
    async fn run(&self, exec: &Exec) -> io::Result<Output> {
        exec.execute().await
    }
}

/// A predicate over an [`Exec`], for [`ScriptedRunner::when`].
type Predicate = Arc<dyn Fn(&Exec) -> bool + Send + Sync>;

/// One scripted matching rule.
#[derive(Clone)]
enum Rule {
    /// Matches when the run's arguments start with this prefix.
    Prefix(Vec<OsString>),
    /// Matches when the predicate accepts the whole [`Exec`] (args, cwd, env, …).
    When(Predicate),
}

/// A test double mapping a command to a canned [`Output`], matched by an
/// argument prefix ([`on`](ScriptedRunner::on)) or an arbitrary predicate
/// ([`when`](ScriptedRunner::when)). Build canned outputs with [`Output::ok`] /
/// [`Output::fail`] / [`Output::timeout`].
#[derive(Clone, Default)]
pub struct ScriptedRunner {
    rules: Vec<(Rule, Output)>,
    fallback: Option<Output>,
}

impl std::fmt::Debug for ScriptedRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Rules hold a boxed closure (not `Debug`); summarise instead.
        f.debug_struct("ScriptedRunner")
            .field("rules", &self.rules.len())
            .field("has_fallback", &self.fallback.is_some())
            .finish()
    }
}

impl ScriptedRunner {
    /// An empty runner that errors on any unmatched command.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reply with `out` when a run's arguments start with `args`.
    pub fn on<I, S>(mut self, args: I, out: Output) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let prefix = args
            .into_iter()
            .map(|a| a.as_ref().to_os_string())
            .collect();
        self.rules.push((Rule::Prefix(prefix), out));
        self
    }

    /// Reply with `out` when `pred` accepts the run — full access to args, cwd,
    /// env, and stdin, so a test can match on details a prefix can't express.
    pub fn when<F>(mut self, pred: F, out: Output) -> Self
    where
        F: Fn(&Exec) -> bool + Send + Sync + 'static,
    {
        self.rules.push((Rule::When(Arc::new(pred)), out));
        self
    }

    /// Reply with `out` for any command no other rule matched.
    pub fn fallback(mut self, out: Output) -> Self {
        self.fallback = Some(out);
        self
    }
}

#[async_trait::async_trait]
impl Runner for ScriptedRunner {
    async fn run(&self, exec: &Exec) -> io::Result<Output> {
        let actual = exec.arguments();
        for (rule, out) in &self.rules {
            let hit = match rule {
                Rule::Prefix(prefix) => {
                    actual.len() >= prefix.len() && actual[..prefix.len()] == prefix[..]
                }
                Rule::When(pred) => pred(exec),
            };
            if hit {
                return Ok(out.clone());
            }
        }
        self.fallback.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("ScriptedRunner: no rule for args {actual:?}"),
            )
        })
    }
}

/// An owned snapshot of one executed [`Exec`], captured by [`RecordingRunner`]
/// so a test can assert exactly what was run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// The program that was run.
    pub program: OsString,
    /// The arguments, in order.
    pub args: Vec<OsString>,
    /// The working-directory override, if set.
    pub cwd: Option<PathBuf>,
    /// Environment overrides, in insertion order.
    pub envs: Vec<(OsString, OsString)>,
    /// Buffered stdin input, if supplied.
    pub stdin: Option<Vec<u8>>,
}

impl Invocation {
    fn snapshot(exec: &Exec) -> Self {
        Invocation {
            program: exec.program().to_os_string(),
            args: exec.arguments().to_vec(),
            cwd: exec.working_dir().map(|p| p.to_path_buf()),
            envs: exec.env_vars().to_vec(),
            stdin: exec.stdin_bytes().map(<[u8]>::to_vec),
        }
    }

    /// The arguments as lossy UTF-8, for ergonomic assertions.
    pub fn args_str(&self) -> Vec<String> {
        self.args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    /// Whether `flag` appears anywhere in the arguments (use to assert a flag is
    /// present — or, negated, that it is *absent*).
    pub fn has_flag(&self, flag: impl AsRef<OsStr>) -> bool {
        let flag = flag.as_ref();
        self.args.iter().any(|a| a.as_os_str() == flag)
    }
}

/// A [`Runner`] that records every invocation, then delegates to an inner runner
/// for the canned [`Output`]. Wrap a [`ScriptedRunner`] (the default) or any
/// other runner; inspect [`calls`](RecordingRunner::calls) afterwards.
pub struct RecordingRunner<R: Runner = ScriptedRunner> {
    inner: R,
    calls: Mutex<Vec<Invocation>>,
}

impl<R: Runner> RecordingRunner<R> {
    /// Record into a recorder backed by `inner`.
    pub fn wrapping(inner: R) -> Self {
        RecordingRunner {
            inner,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Every invocation so far, in call order.
    pub fn calls(&self) -> Vec<Invocation> {
        self.calls.lock().expect("recorder mutex poisoned").clone()
    }

    /// The single invocation — panics if there were zero or more than one.
    pub fn only_call(&self) -> Invocation {
        let calls = self.calls.lock().expect("recorder mutex poisoned");
        assert_eq!(
            calls.len(),
            1,
            "expected exactly one invocation, got {}",
            calls.len()
        );
        calls[0].clone()
    }
}

impl RecordingRunner<ScriptedRunner> {
    /// A recorder over an empty [`ScriptedRunner`] that replies `out` to every
    /// command — the common shape for "run it, then assert what was built".
    pub fn replying(out: Output) -> Self {
        RecordingRunner::wrapping(ScriptedRunner::new().fallback(out))
    }
}

#[async_trait::async_trait]
impl<R: Runner> Runner for RecordingRunner<R> {
    async fn run(&self, exec: &Exec) -> io::Result<Output> {
        self.calls
            .lock()
            .expect("recorder mutex poisoned")
            .push(Invocation::snapshot(exec));
        self.inner.run(exec).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // A predicate rule can match on details a prefix can't (here, the cwd), and
    // is checked in registration order alongside prefix rules.
    #[tokio::test]
    async fn predicate_and_prefix_rules_both_match() {
        let runner = ScriptedRunner::new()
            .when(
                |e| e.working_dir() == Some(Path::new("/repo")),
                Output::ok("in-repo"),
            )
            .on(["status"], Output::ok("plain"));

        let scoped = Exec::new("git")
            .current_dir("/repo")
            .args(["status"])
            .output_with(&runner)
            .await
            .unwrap();
        assert_eq!(scoped.stdout, "in-repo");

        let plain = Exec::new("git")
            .args(["status"])
            .output_with(&runner)
            .await
            .unwrap();
        assert_eq!(plain.stdout, "plain");
    }

    // RecordingRunner captures the full invocation, so a test can assert exact
    // args, cwd, and — crucially — that a flag is ABSENT.
    #[tokio::test]
    async fn recording_runner_captures_args_cwd_and_absence() {
        let rec = RecordingRunner::replying(Output::ok("ok"));
        Exec::new("gh")
            .current_dir("/repo")
            .args(["pr", "create", "--title", "T"])
            .output_with(&rec)
            .await
            .unwrap();

        let call = rec.only_call();
        assert_eq!(call.program, OsString::from("gh"));
        assert_eq!(call.cwd.as_deref(), Some(Path::new("/repo")));
        assert_eq!(call.args_str(), ["pr", "create", "--title", "T"]);
        assert!(call.has_flag("--title"));
        assert!(!call.has_flag("--base"), "no base flag was passed");
    }
}
