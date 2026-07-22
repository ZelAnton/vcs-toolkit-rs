//! A command-logging [`ProcessRunner`] decorator and its argv redaction.
//!
//! [`LoggingRunner`] wraps any real [`ProcessRunner`] (the default [`JobRunner`],
//! a `ManagedClient`'s inner runner, a test double) and reports every command it
//! runs — program, argv, working directory, exit code, and duration — to a
//! [`CommandObserver`]. Because it sits on the single seam every wrapper spawns
//! through, coverage is complete *by construction*: it observes all of `vcs-git`
//! / `vcs-jj` / the forge wrappers without any per-call-site instrumentation, and
//! it can't drift out of date when a new operation is added.
//!
//! # Why this is a security boundary
//!
//! Logging argv is delicate: the value slots can carry a PR/issue body, a commit
//! message, a clone URL, or — in principle — a secret. This module never emits a
//! value verbatim. [`redact_args`] applies a **fail-closed** policy before anything
//! reaches an observer:
//!
//! - The value after a **sensitive flag** (`--token`, `--password`, `--secret`,
//!   `--authorization`, …) is replaced with `<redacted>`, as is the value of a
//!   `--flag=value` form of one.
//! - A value that **looks like a secret** (a `ghp_`/`github_pat_`/`glpat-`/… token
//!   prefix, an `x-access-token:` embed) is replaced with `<redacted>`.
//! - A **URL with embedded credentials** (`scheme://user:pass@host/…`) keeps its
//!   host/path but masks the userinfo (`scheme://<redacted>@host/…`).
//! - Any **long free-text** value (a PR/issue body, a commit message) is truncated
//!   to [`MAX_VALUE_LEN`] characters plus a length marker.
//!
//! This is defence in depth on top of the workspace's existing "the token never
//! rides in argv" contract (forge tokens travel in `GH_TOKEN`/`GITLAB_TOKEN`
//! *environment*, git's secret via `credential.helper`) — the decorator never logs
//! the environment at all, so the token-carrying channel is out of scope for the
//! log by construction, and the argv redaction guards the residual risk.
//!
//! The default [`StderrObserver`] writes a one-line summary to **stderr**, never
//! stdout — so a JSON-RPC transport sharing the process's stdout (the `vcs-mcp`
//! server) stays a clean transport. Supply your own [`CommandObserver`] to route
//! the same structured record into `tracing`, a file, or a test buffer instead.

use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use processkit::{Command, Error, JobRunner, ProcessResult, ProcessRunner, Result, RunningProcess};

/// The longest a single free-text argv value is rendered before it is truncated
/// with a `…(<n> chars)` marker. Normal argv (subcommands, flags, refs, paths,
/// revsets) sits well under this, so it only ever clips genuinely large values —
/// a PR/issue body, a long commit message — keeping the log both readable and
/// free of bulk user text. The exact number is not load-bearing.
pub const MAX_VALUE_LEN: usize = 160;

/// Long-flag names (without the leading dashes, lower-cased) whose *value* is
/// treated as a secret and masked. Deliberately only unambiguous long names — a
/// short flag like `-p` means different things per tool (`git log -p` is a patch,
/// not a password), so masking the token after it would corrupt diagnostics for
/// no real safety gain. Over-masking a genuine value here is the safe direction
/// (a redacted diagnostic vs. a leaked secret), so the list errs toward inclusion.
const SENSITIVE_FLAGS: &[&str] = &[
    "token",
    "password",
    "passwd",
    "secret",
    "auth",
    "authorization",
    "credential",
    "credentials",
    "api-key",
    "apikey",
    "access-token",
    "private-token",
    "gh-token",
    "github-token",
    "gitlab-token",
    "bearer",
    "otp",
    "pat",
];

/// Case-insensitive prefixes that mark a value as a known secret/token shape, so
/// it is masked wholesale even in a positional slot. Covers the forge PATs the
/// workspace touches plus a few common provider tokens; extend as needed.
const SECRET_PREFIXES: &[&str] = &[
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "glptt-",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxr-",
];

/// How an observed command finished. Carries no captured stdout/stderr — only a
/// coarse, allocation-free category — so an observer can never leak process
/// output (which could echo user text) into a log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandStatus {
    /// The process exited with this code (`0` for success; any code, since a
    /// non-zero exit is not an error at the runner seam).
    Exited(i32),
    /// Terminated by a signal (Unix); the number when the kernel exposed one.
    Signalled(Option<i32>),
    /// Killed for exceeding its timeout.
    TimedOut,
    /// A live streaming handle was returned ([`ProcessRunner::start`]); the
    /// command's completion, exit code, and duration are observed by whoever
    /// drives the handle, not here.
    Started,
    /// The run failed before producing an exit code (spawn/launch/IO error). The
    /// `&'static str` is a stable category — never the error's captured output.
    Failed(&'static str),
}

/// A display-safe, already-redacted record of one command a [`LoggingRunner`] ran,
/// handed to a [`CommandObserver`]. Every field is safe to print: `args` has been
/// through [`redact_args`], and `status` carries no captured process output.
///
/// [`Display`](fmt::Display) renders the canonical one-line summary the built-in
/// [`StderrObserver`] uses (minus its tag), so a custom observer can reuse the
/// exact formatting or read the structured fields directly.
#[derive(Debug)]
pub struct CommandRecord<'a> {
    /// The program launched (its path/name as given — not a secret).
    pub program: &'a str,
    /// The arguments, already redacted by [`redact_args`].
    pub args: &'a [String],
    /// The working directory the command ran in, if one was bound.
    pub working_dir: Option<&'a Path>,
    /// How the run finished.
    pub status: CommandStatus,
    /// Wall-clock time the run took. [`Duration::ZERO`] for a
    /// [`CommandStatus::Started`] record (completion is observed elsewhere).
    pub duration: Duration,
}

impl fmt::Display for CommandRecord<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.program)?;
        for arg in self.args {
            write!(f, " {arg}")?;
        }
        if let Some(dir) = self.working_dir {
            write!(f, " (cwd: {})", dir.display())?;
        }
        match self.status {
            CommandStatus::Started => write!(f, " -> started (streaming)"),
            CommandStatus::Exited(code) => write!(f, " -> exit {code} in {:?}", self.duration),
            CommandStatus::Signalled(Some(sig)) => {
                write!(f, " -> signal {sig} in {:?}", self.duration)
            }
            CommandStatus::Signalled(None) => write!(f, " -> signalled in {:?}", self.duration),
            CommandStatus::TimedOut => write!(f, " -> timed out in {:?}", self.duration),
            CommandStatus::Failed(kind) => write!(f, " -> failed: {kind} in {:?}", self.duration),
        }
    }
}

/// A sink for the command records a [`LoggingRunner`] produces. Implement it to
/// route the (already-redacted) [`CommandRecord`] into `tracing`, a file, a
/// metrics counter, or a test buffer; the built-in [`StderrObserver`] writes a
/// one-line summary to stderr.
pub trait CommandObserver: Send + Sync {
    /// Called once per observed command, synchronously, after it finishes (or,
    /// for a streaming [`ProcessRunner::start`], right after the handle is
    /// returned). Keep it cheap and non-blocking; it runs on the calling task.
    fn on_command(&self, record: &CommandRecord<'_>);
}

/// The default [`CommandObserver`]: writes one line per command to **stderr**
/// (never stdout, so a stdout JSON-RPC transport stays clean), prefixed with a
/// short tag. Format: `` `<tag>: <program> <args…> (cwd: <dir>) -> <status> in <dur>` ``.
#[derive(Debug, Clone)]
pub struct StderrObserver {
    tag: Arc<str>,
}

impl StderrObserver {
    /// A stderr observer tagged `tag` (a short prefix that identifies the source,
    /// e.g. the server binary name).
    pub fn new(tag: impl Into<Arc<str>>) -> Self {
        Self { tag: tag.into() }
    }
}

impl Default for StderrObserver {
    /// Tagged `command`.
    fn default() -> Self {
        Self::new("command")
    }
}

impl CommandObserver for StderrObserver {
    fn on_command(&self, record: &CommandRecord<'_>) {
        eprintln!("{}: {record}", self.tag);
    }
}

/// A [`ProcessRunner`] decorator that reports every command it runs to a
/// [`CommandObserver`], then forwards the real runner's result unchanged.
///
/// It adds only observation — the wrapped runner's behaviour, results, and errors
/// are passed through verbatim. Construct one with [`new`](Self::new) (a
/// [`StderrObserver`]) or [`with_observer`](Self::with_observer) (a custom sink),
/// then hand it to any client's `with_runner` builder:
///
/// ```no_run
/// use processkit::JobRunner;
/// use vcs_cli_support::logging::LoggingRunner;
/// // A boxed runner erases the concrete type, so the same client type works
/// // whether or not logging is enabled.
/// let runner: Box<dyn processkit::ProcessRunner> =
///     Box::new(LoggingRunner::new(JobRunner::new(), "vcs-mcp"));
/// ```
pub struct LoggingRunner<R: ProcessRunner = JobRunner> {
    inner: R,
    observer: Arc<dyn CommandObserver>,
}

impl<R: ProcessRunner> LoggingRunner<R> {
    /// Wrap `inner`, logging each command to stderr with the tag `tag` (via
    /// [`StderrObserver`]).
    pub fn new(inner: R, tag: impl Into<Arc<str>>) -> Self {
        Self::with_observer(inner, Arc::new(StderrObserver::new(tag)))
    }

    /// Wrap `inner`, reporting each command to `observer`.
    pub fn with_observer(inner: R, observer: Arc<dyn CommandObserver>) -> Self {
        Self { inner, observer }
    }

    /// The observer this runner reports to.
    pub fn observer(&self) -> &Arc<dyn CommandObserver> {
        &self.observer
    }

    /// A reference to the wrapped runner.
    pub fn inner(&self) -> &R {
        &self.inner
    }

    /// Build a redacted record for `command` and hand it to the observer. The
    /// program path and working directory are not secrets; the argv is passed
    /// through [`redact_args`] first, and `status` carries no captured output.
    fn observe(&self, command: &Command, status: CommandStatus, duration: Duration) {
        let program = command.program().to_string_lossy();
        let args = redact_args(command.arguments());
        let record = CommandRecord {
            program: program.as_ref(),
            args: &args,
            working_dir: command.working_dir(),
            status,
            duration,
        };
        self.observer.on_command(&record);
    }
}

impl<R: ProcessRunner + fmt::Debug> fmt::Debug for LoggingRunner<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The observer is a trait object with no meaningful rendering; report only
        // that one is attached, and delegate to the inner runner's own Debug.
        f.debug_struct("LoggingRunner")
            .field("inner", &self.inner)
            .field("observer", &"<dyn CommandObserver>")
            .finish()
    }
}

#[async_trait]
impl<R: ProcessRunner> ProcessRunner for LoggingRunner<R> {
    async fn output_string(&self, command: &Command) -> Result<ProcessResult<String>> {
        let started = Instant::now();
        let result = self.inner.output_string(command).await;
        self.observe(command, status_of(&result), started.elapsed());
        result
    }

    async fn output_bytes(&self, command: &Command) -> Result<ProcessResult<Vec<u8>>> {
        let started = Instant::now();
        let result = self.inner.output_bytes(command).await;
        self.observe(command, status_of(&result), started.elapsed());
        result
    }

    async fn start(&self, command: &Command) -> Result<RunningProcess> {
        // A streaming handle: the command's completion (exit code, duration) is
        // observed by whoever drives the handle, not here — so log the spawn with
        // `Started` (or the launch failure) and let the caller own the rest.
        let result = self.inner.start(command).await;
        let status = match &result {
            Ok(_) => CommandStatus::Started,
            Err(err) => CommandStatus::Failed(error_category(err)),
        };
        self.observe(command, status, Duration::ZERO);
        result
    }
}

/// Map a finished-run result to a [`CommandStatus`]. A non-zero exit is an `Ok`
/// result here (the runner seam does not raise on it), so `Ok` maps to the
/// process outcome and `Err` to a launch/IO failure category.
fn status_of<T>(result: &Result<ProcessResult<T>>) -> CommandStatus {
    match result {
        Ok(res) => {
            // Use the accessors rather than matching the `#[non_exhaustive]`
            // `Outcome`: `code()` is `Some` only for a real exit, `timed_out()`
            // for a deadline kill, otherwise it was a signal.
            if let Some(code) = res.code() {
                CommandStatus::Exited(code)
            } else if res.timed_out() {
                CommandStatus::TimedOut
            } else {
                CommandStatus::Signalled(res.signal())
            }
        }
        Err(err) => CommandStatus::Failed(error_category(err)),
    }
}

/// A stable, output-free category for a runner error — never the error's captured
/// stdout/stderr (which could echo user text). A conservative wildcard keeps a
/// future `#[non_exhaustive]` variant safe.
fn error_category(err: &Error) -> &'static str {
    match err {
        Error::NotFound { .. } => "program not found",
        Error::Spawn { .. } => "spawn failed",
        Error::Timeout { .. } => "timed out",
        Error::Cancelled { .. } => "cancelled",
        Error::Unsupported { .. } => "unsupported",
        Error::OutputTooLarge { .. } => "output too large",
        Error::Exit { .. } => "non-zero exit",
        Error::Io(_) => "io error",
        _ => "error",
    }
}

/// Redact a command's argv for display: mask secret-bearing values, mask the
/// userinfo of a credentialed URL, and truncate long free text — see the
/// [module docs](self) for the full policy. Returns one display string per input
/// argument, in order. The policy is **fail-closed**: when in doubt it masks.
///
/// This is sequence-aware (the value *after* a sensitive flag is masked), so pass
/// the whole argv, not one argument at a time.
pub fn redact_args(args: &[OsString]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    // Set when the previous token was a bare sensitive flag (`--token`), so the
    // next token (its value) is masked.
    let mut mask_next = false;
    for arg in args {
        let s = arg.to_string_lossy();
        if mask_next {
            out.push(REDACTED.to_string());
            mask_next = false;
            continue;
        }
        if s.starts_with('-') {
            let name_part = s.trim_start_matches('-');
            let dashes_len = s.len() - name_part.len();
            if let Some(eq) = name_part.find('=') {
                // `--flag=value`: mask the value if the flag is sensitive, else
                // redact the value as ordinary free text (secret-scan/truncate).
                let name = &name_part[..eq];
                let value = &name_part[eq + 1..];
                if is_sensitive_flag(name) {
                    out.push(format!("{}{name}={REDACTED}", &s[..dashes_len]));
                } else {
                    out.push(format!(
                        "{}{name}={}",
                        &s[..dashes_len],
                        redact_value(value)
                    ));
                }
            } else {
                // A bare flag is structural and safe to show verbatim; if it is a
                // sensitive flag, mask whatever value follows it.
                if is_sensitive_flag(name_part) {
                    mask_next = true;
                }
                out.push(s.into_owned());
            }
        } else {
            out.push(redact_value(&s).into_owned());
        }
    }
    out
}

/// The placeholder emitted in place of a masked value.
const REDACTED: &str = "<redacted>";

/// Whether `name` (a flag name without leading dashes) is one whose value must be
/// masked. Case-insensitive.
fn is_sensitive_flag(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    SENSITIVE_FLAGS.contains(&name.as_str())
}

/// Redact a single free-text value: mask it wholesale if it looks like a secret,
/// mask the userinfo of a credentialed URL, then truncate if it is long.
fn redact_value(value: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    if value.is_empty() {
        return Cow::Borrowed(value);
    }
    let lower = value.to_ascii_lowercase();
    if SECRET_PREFIXES.iter().any(|p| lower.starts_with(p)) || lower.contains("x-access-token:") {
        return Cow::Owned(REDACTED.to_string());
    }
    if let Some(masked) = mask_url_userinfo(value) {
        return Cow::Owned(truncate(&masked));
    }
    match truncate_cow(value) {
        Some(t) => Cow::Owned(t),
        None => Cow::Borrowed(value),
    }
}

/// If `value` is a URL of the form `scheme://userinfo@host/…` whose `userinfo`
/// carries a password (`user:secret`), return it with the userinfo masked
/// (`scheme://<redacted>@host/…`); otherwise `None`. Keeps the host/path visible
/// for diagnostics while never printing an embedded credential.
fn mask_url_userinfo(value: &str) -> Option<String> {
    let scheme_end = value.find("://")?;
    let after = &value[scheme_end + 3..];
    let at = after.find('@')?;
    let userinfo = &after[..at];
    // Only mask when there is a password component (`user:secret`); a bare
    // `user@host` (no colon) is not a secret and stays visible.
    if !userinfo.contains(':') {
        return None;
    }
    Some(format!(
        "{}://{REDACTED}@{}",
        &value[..scheme_end],
        &after[at + 1..]
    ))
}

/// Truncate `value` to [`MAX_VALUE_LEN`] characters plus a `…(<n> chars)` marker,
/// or `None` if it already fits. Char-boundary safe.
fn truncate_cow(value: &str) -> Option<String> {
    let count = value.chars().count();
    if count <= MAX_VALUE_LEN {
        return None;
    }
    let head: String = value.chars().take(MAX_VALUE_LEN).collect();
    Some(format!("{head}…({count} chars)"))
}

/// Truncate an already-owned value the same way, in place of a no-op when it fits.
fn truncate(value: &str) -> String {
    truncate_cow(value).unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::testing::{RecordingRunner, Reply};
    use std::sync::Mutex;

    /// Build an `OsString` argv from `&str`s.
    fn argv(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    /// A `CommandObserver` that captures each record's rendered line, for asserting
    /// on what would actually be logged.
    #[derive(Default)]
    struct Capture(Mutex<Vec<String>>);

    impl CommandObserver for Capture {
        fn on_command(&self, record: &CommandRecord<'_>) {
            self.0.lock().unwrap().push(record.to_string());
        }
    }

    #[test]
    fn ordinary_argv_is_shown_verbatim() {
        let out = redact_args(&argv(&["status", "--porcelain", "-z"]));
        assert_eq!(out, vec!["status", "--porcelain", "-z"]);
    }

    #[test]
    fn value_after_a_sensitive_flag_is_masked() {
        let out = redact_args(&argv(&["--token", "ghp_supersecretvalue", "pr", "list"]));
        assert_eq!(out, vec!["--token", "<redacted>", "pr", "list"]);
        // The `-p` short flag is NOT sensitive (git log -p is a patch), so the
        // following value is not masked by the flag rule.
        let out = redact_args(&argv(&["log", "-p", "HEAD~1"]));
        assert_eq!(out, vec!["log", "-p", "HEAD~1"]);
    }

    #[test]
    fn inline_sensitive_flag_value_is_masked() {
        let out = redact_args(&argv(&["--password=hunter2", "--auth=Bearer xyz"]));
        assert_eq!(out, vec!["--password=<redacted>", "--auth=<redacted>"]);
    }

    #[test]
    fn secret_looking_positional_is_masked_even_without_a_flag() {
        // A bare token that matched no sensitive flag is still masked by shape.
        let out = redact_args(&argv(&[
            "push",
            "glpat-abcdEFGH1234 ",
            "github_pat_11ABCDEF",
        ]));
        assert_eq!(out[0], "push");
        assert_eq!(out[1], "<redacted>");
        assert_eq!(out[2], "<redacted>");
    }

    #[test]
    fn url_userinfo_credentials_are_masked_but_host_kept() {
        let out = redact_args(&argv(&[
            "clone",
            "https://user:tokensecret@github.com/o/r.git",
        ]));
        assert_eq!(out[0], "clone");
        assert_eq!(out[1], "https://<redacted>@github.com/o/r.git");
        assert!(!out[1].contains("tokensecret"));
        // A bare `user@host` (no password component) stays visible.
        let out = redact_args(&argv(&["fetch", "ssh://git@github.com/o/r.git"]));
        assert_eq!(out[1], "ssh://git@github.com/o/r.git");
    }

    #[test]
    fn long_free_text_is_truncated_not_dumped() {
        let body = "x".repeat(MAX_VALUE_LEN + 50);
        let out = redact_args(&argv(&["pr", "create", "--body", &body]));
        assert_eq!(&out[..3], &["pr", "create", "--body"]);
        let shown = &out[3];
        assert!(shown.len() < body.len(), "the body was truncated");
        assert!(shown.contains("chars)"), "carries a length marker: {shown}");
        // And the inline `--body=<huge>` form is truncated too (flag kept).
        let out = redact_args(&argv(&["pr", "create", &format!("--body={body}")]));
        assert!(out[2].starts_with("--body=x"));
        assert!(out[2].contains("chars)"));
    }

    #[tokio::test]
    async fn runner_observes_a_command_without_leaking_a_secret() {
        // A hermetic inner runner: replies with canned output, records the calls.
        let inner = RecordingRunner::replying(Reply::ok("ok"));
        let capture = Arc::new(Capture::default());
        let runner = LoggingRunner::with_observer(&inner, capture.clone());

        // A command whose argv carries a value we must never see in the log.
        let secret = "ghp_THIS_MUST_NOT_APPEAR";
        let command = Command::new("gh")
            .args([
                "pr",
                "create",
                "--token",
                secret,
                "--body",
                &"z".repeat(400),
            ])
            .current_dir("/tmp/work");

        let result = runner
            .output_string(&command)
            .await
            .expect("the inner runner replied ok");
        assert_eq!(result.stdout(), "ok");
        // The decorator forwarded the real call unchanged.
        assert_eq!(inner.calls().len(), 1);

        let lines = capture.0.lock().unwrap();
        assert_eq!(lines.len(), 1, "exactly one record per command");
        let line = &lines[0];
        // The core safety property: the secret never reaches the observer.
        assert!(
            !line.contains(secret),
            "the secret must not appear in the log line: {line}"
        );
        assert!(
            line.contains("<redacted>"),
            "the token value is masked: {line}"
        );
        // The useful diagnostics ARE present: program, subcommand, cwd, exit code.
        assert!(line.contains("gh"), "shows the program: {line}");
        assert!(line.contains("pr create"), "shows the subcommand: {line}");
        assert!(
            line.contains("cwd: "),
            "shows the working directory: {line}"
        );
        assert!(line.contains("exit 0"), "shows the exit code: {line}");
        // The long body was truncated, not dumped whole.
        assert!(
            !line.contains(&"z".repeat(400)),
            "the body is not dumped: {line}"
        );
    }

    #[tokio::test]
    async fn the_streaming_start_path_logs_the_spawn() {
        // The streaming seam is instrumented too (so a `first_line`-style verb is
        // observed), reported as a `Started` record — completion is owned by the
        // handle's driver, not this decorator.
        let inner = RecordingRunner::replying(Reply::ok("a line\n"));
        let capture = Arc::new(Capture::default());
        let runner = LoggingRunner::with_observer(&inner, capture.clone());

        let command = Command::new("gh").args(["run", "watch"]);
        let _ = runner.start(&command).await;

        let lines = capture.0.lock().unwrap();
        assert_eq!(lines.len(), 1, "the spawn is logged exactly once");
        assert!(
            lines[0].contains("gh run watch"),
            "logs the spawn: {}",
            lines[0]
        );
        assert!(
            lines[0].contains("started"),
            "reports the streaming spawn: {}",
            lines[0]
        );
    }
}
