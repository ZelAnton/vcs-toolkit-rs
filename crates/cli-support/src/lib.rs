#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-cli-support` — the [`processkit`]-coupled plumbing the CLI wrappers reuse.
//!
//! `vcs-git` / `vcs-jj` / `vcs-github` all drive a CLI through [`processkit`], so
//! they share three concerns that *touch* [`processkit::Error`]: an argv injection
//! guard, a fetch-retry policy, and a set of [`Error`] classifiers. Extracting them
//! here keeps the std-only `vcs-diff` clean of the `processkit` dependency, and —
//! more to the point — keeps the marker lists and classifier logic from drifting
//! between backends. The wrapper crates re-export these items (so you reach them
//! as `vcs_git::is_merge_conflict`, not via this crate's name) and rarely name
//! `vcs-cli-support` directly.
//!
//! # The surface
//!
//! - **[`reject_flag_like`]** — the injection guard for bare positional argv slots.
//!   A caller value that is empty/whitespace, or starts with `-`, is refused before
//!   spawning (the CLI would parse it as a flag); flag-*value* slots (`-m <msg>`)
//!   are consumed verbatim and skip the check. Wrappers call it with their own
//!   binary name so the surfaced [`ErrorReason::Spawn`] names the right `program`.
//! - **[`FETCH_ATTEMPTS`] / [`FETCH_BACKOFF`]** — the shared transient-retry policy
//!   for `fetch` (one try plus two retries, fixed backoff between them).
//! - **[`is_merge_conflict`] / [`is_nothing_to_commit`] / [`is_transient_fetch_error`]
//!   / [`is_lock_contention`]** — classify a returned [`Error`] so callers branch on
//!   *intent* ("conflict, resolve it"; "nothing to commit, no-op"; "transient,
//!   retry"; "another process holds the lock, retry") instead of matching on error
//!   internals. They inspect captured [`ErrorReason::Exit`] output against fixed marker
//!   lists; a [`processkit`] [`ErrorReason::Timeout`] is **not** treated as a transient
//!   fetch error (it already spent the full deadline — see
//!   [`is_transient_fetch_error`]); any unfamiliar `#[non_exhaustive]` variant falls
//!   through to "no".
//! - **[`RetryPolicy`] / [`retry_async`] / [`ManagedClient`]** — an opt-in retry
//!   strategy (attempts + exponential, jittered backoff) for **lock-contention**
//!   failures. `ManagedClient` wraps a [`processkit`] `CliClient` and applies the
//!   policy to every command, so the `vcs-git`/`vcs-jj` clients gain retry via
//!   `with_retry(...)` without changing a call site. Lock-acquisition failures are
//!   pre-execution, so retrying is safe even for mutating commands. A
//!   [`default_cancel_on`](ManagedClient::default_cancel_on) token also cuts the
//!   backoff short: cancelling mid-retry returns a structured [`ErrorReason::Cancelled`]
//!   at once instead of sleeping out the remaining delay.
//! - **[`CredentialProvider`] / [`Credential`] / [`Secret`]** — an opt-in seam for
//!   supplying a secret *per operation* (a CI token, a vault lookup) instead of
//!   relying on ambient CLI auth. `ManagedClient` injects the resolved token into
//!   each command (the forge `GH_TOKEN`/`GITLAB_TOKEN` env); git uses
//!   [`git_credential_helper`] to keep the secret out of `argv`. Default is no
//!   provider → ambient auth, unchanged. See the [`credentials`](mod@credentials)
//!   module for the full picture.
//!
//! # Recipes
//!
//! Classify a failed `fetch` to drive a retry decision — branch on intent, not on
//! the error's internals:
//!
//! ```no_run
//! use vcs_cli_support::{is_transient_fetch_error, FETCH_ATTEMPTS, FETCH_BACKOFF};
//! # fn run() -> Result<(), processkit::Error> { todo!() }
//! # fn demo() -> Result<(), processkit::Error> {
//! for attempt in 1..=FETCH_ATTEMPTS {
//!     match run() {
//!         Ok(()) => break,
//!         Err(e) if is_transient_fetch_error(&e) && attempt < FETCH_ATTEMPTS => {
//!             std::thread::sleep(FETCH_BACKOFF); // DNS / dropped connection — worth a retry
//!         }
//!         Err(e) => return Err(e),               // anything else: give up
//!     }
//! }
//! # Ok(()) }
//! ```

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::fmt;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use processkit::prelude::StreamExt;
use processkit::{
    CancellationToken, CliClient, Command, Error, ErrorReason, IntoCommand, JobRunner,
    OutputBufferPolicy, OverflowMode, ProcessResult, ProcessRunner, Result,
};

/// One lifecycle/progress event from a streamed CLI operation.
///
/// Re-exported from [`processkit`] so wrapper users do not need a second direct
/// dependency merely to inspect progress. The stream starts with
/// [`ProcessEvent::Started`], carries stdout/stderr lines, and ends with
/// [`ProcessEvent::Exited`]. The enum is non-exhaustive; consumers should keep a
/// wildcard match arm.
pub use processkit::ProcessEvent;

/// Object-safe callback accepted by the typed streaming operations.
///
/// `Send` lets an async trait future containing the borrowed callback remain
/// `Send`, preserving `GitApi`/`JjApi`/facade trait-object usability. A callback
/// panic is caught and disables further callback delivery for that run; output
/// draining and process completion continue, matching processkit's hardened
/// per-line handler contract.
pub type ProgressCallback<'a> = dyn FnMut(ProcessEvent) + Send + 'a;

/// Run one command while forwarding its lifecycle and output events.
///
/// `events()` and `finish()` are deliberately driven concurrently: the terminal
/// `Exited` event is published by the finisher, so draining the stream first
/// would deadlock. Output lines are also retained locally so a rejected exit is
/// promoted to the same structured processkit error shape as `run_unit`, with
/// the command's stdout/stderr available to classifiers.
///
/// This function observes exactly **one process lifecycle** and therefore does
/// not apply a command/client retry policy: `Exited` is always the final event.
/// A caller that wants to replay a failed streamed operation can decide that
/// explicitly after receiving the terminal event and returned error.
///
/// **Retention is unbounded**, the pre-budget behaviour: every streamed line is
/// kept until the process exits. Bound it with
/// [`run_with_progress_within`] — the streaming counterpart of
/// [`ManagedClient::budget_diagnostics`] — when the command can produce a lot of
/// output (`clone`/`fetch --progress` on a large repository).
pub async fn run_with_progress<R: ProcessRunner + ?Sized>(
    runner: &R,
    command: &Command,
    progress: &mut ProgressCallback<'_>,
) -> Result<()> {
    run_with_progress_within(runner, command, progress, OutputBudget::unlimited()).await
}

/// [`run_with_progress`] with an explicit [`OutputBudget`] bounding the output
/// it retains locally — the memory ceiling a long streamed run needs.
///
/// This is the streaming half of the contract
/// [`budget_diagnostics`](ManagedClient::budget_diagnostics) applies to the
/// *captured* twin of the same verbs. That one bounds the buffer **processkit**
/// retains for a non-streamed `clone`/`fetch`; this one bounds the copy **this
/// function** keeps in order to promote a rejected exit into a structured error.
/// A streamed run needs both: the event stream delivers every line regardless of
/// the command's own [`OutputBufferPolicy`], so without a ceiling here a
/// `git clone --progress` of a large repository re-introduces exactly the
/// unbounded retention the budget exists to prevent.
///
/// The ceiling is **drop-oldest and never fail-loud**, like
/// [`OutputBudget::diagnostic_policy`]: passing it truncates what is retained,
/// it never turns a successful (or plainly failed) run into
/// [`ErrorReason::OutputTooLarge`]. Each stream carries the ceiling
/// **independently** — `stdout` and `stderr` get their own budget, so the
/// worst-case retained memory is about twice the cap, matching how
/// [`OutputBudget`] rides a captured verb's two streams. The retained **tail**
/// is what a CLI's fatal line sits in, so a bounded run stays classifiable by
/// [`is_transient_fetch_error`] / [`is_lock_contention`], and a non-zero exit is
/// still promoted to a structured [`ErrorReason::Exit`] carrying that (truncated,
/// non-empty) text.
///
/// Only what *this function* retains is bounded: `progress` is still invoked for
/// every event, so a caller that wants more of the stream can keep it — bounded
/// however it chooses — from its own callback.
///
/// # What the byte ceiling counts here
///
/// [`OutputBudget::bytes`] is charged against **the retained text itself**: the
/// decoded content of the retained lines plus the single `\n` this function
/// inserts between each retained pair. The invariant is therefore as direct as
/// it looks — the `stdout`/`stderr` handed to the error is never longer than
/// `max_bytes` bytes.
///
/// That is deliberately a *third* unit, and the two processkit ones are neither
/// of them (see [`OutputBudget::bytes`] for those): the fail-loud content
/// ceiling counts raw pipe bytes with every terminator charged, processkit's own
/// drop-oldest retention counts decoded line content with none charged, and this
/// one charges exactly the separators it actually holds — one per retained pair.
/// For `n` retained LF-terminated lines that is one byte *less* than the raw
/// pipe bytes they arrived as, and `n - 1` bytes *more* than processkit's
/// drop-mode accounting of the same lines. [`OutputBudget::with_max_lines`]
/// caps the retained **line count** on top of that, dropping oldest-first too.
///
/// A single line longer than the byte cap is kept as its **own tail** (cut on a
/// UTF-8 char boundary), not dropped whole as processkit's drop-mode buffer
/// would drop it: under the default `\n` line framing, carriage-return progress
/// output — precisely what `--progress` emits — arrives as one ever-growing
/// line, and dropping it whole would retain nothing at all of the stream this
/// ceiling exists to bound.
pub async fn run_with_progress_within<R: ProcessRunner + ?Sized>(
    runner: &R,
    command: &Command,
    progress: &mut ProgressCallback<'_>,
    budget: OutputBudget,
) -> Result<()> {
    let program = command.program().to_string_lossy().into_owned();
    let ok_codes = command
        .configured_ok_codes()
        .map_or_else(|| vec![0], <[i32]>::to_vec);
    let absolute_timeout = command.configured_timeout();
    let inactivity_timeout = command.configured_inactivity_timeout();
    let started = Instant::now();

    let mut run = runner.start(command).await?;
    let mut events = run.events()?;
    // Each stream gets the whole budget, independently — the same shape a
    // captured verb's two streams carry (see `OutputBudget`).
    let mut stdout = RetainedStream::new(budget);
    let mut stderr = RetainedStream::new(budget);
    let forward = async {
        let mut callback_active = true;
        while let Some(event) = events.next().await {
            let target = match &event {
                ProcessEvent::Stdout(_) => Some(&mut stdout),
                ProcessEvent::Stderr(_) => Some(&mut stderr),
                _ => None,
            };
            if let (Some(target), Some(line)) = (target, event.text()) {
                target.push(line);
            }
            if callback_active
                && std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| progress(event)))
                    .is_err()
            {
                // A UI callback must not strand the child or its pipe pumps.
                // Disable it, keep draining, and let the real process outcome win.
                callback_active = false;
            }
        }
    };
    let (_, finished) = tokio::join!(forward, run.finish());
    let finished = finished?;
    let timeout = if finished.outcome.inactivity_timed_out() {
        inactivity_timeout
    } else {
        absolute_timeout
    };
    // Report our own drop-oldest truncation alongside processkit's: `truncated`
    // is a signal, not a verdict — `ensure_success` never reads it, so a bounded
    // stream still surfaces as `Exit`, never as `OutputTooLarge`.
    let truncated = finished.stderr_truncated || stdout.truncated() || stderr.truncated();
    ProcessResult::from_parts(
        program,
        stdout.into_string(),
        stderr.into_string(),
        finished.outcome,
        timeout,
        started.elapsed(),
        truncated,
        0,
        0,
        ok_codes,
    )
    .ensure_success()
    .map(drop)
}

/// What [`run_with_progress_within`] retains locally from one streamed channel
/// (`stdout` or `stderr`).
///
/// Two shapes, so the default costs nothing: with no ceiling the lines land in
/// one growing `String` exactly as they did before the budget existed, and only
/// a budgeted run pays for the drop-oldest bookkeeping.
enum RetainedStream {
    /// No ceiling — every line, appended verbatim.
    All(String),
    /// A drop-oldest tail bounded by the budget.
    Tail(RetainedTail),
}

/// The bounded half of [`RetainedStream`]: the retained lines, oldest dropped
/// first once a ceiling is passed.
struct RetainedTail {
    /// The retained lines, oldest at the front.
    lines: VecDeque<String>,
    /// The rendered length of `lines` — the sum of their byte lengths plus the
    /// one `\n` joining each retained pair. This is the quantity `max_bytes`
    /// caps (an upper bound on the rendered string: a *leading* empty line's
    /// separator is elided on render, so the estimate can only over-count).
    bytes: usize,
    max_bytes: Option<usize>,
    max_lines: Option<usize>,
    /// Whether anything was dropped (a whole line, or the head of an over-cap
    /// one) — reported through `ProcessResult::truncated`.
    truncated: bool,
}

impl RetainedStream {
    fn new(budget: OutputBudget) -> Self {
        if budget.is_unlimited() {
            Self::All(String::new())
        } else {
            Self::Tail(RetainedTail {
                lines: VecDeque::new(),
                bytes: 0,
                max_bytes: budget.max_bytes(),
                max_lines: budget.max_lines(),
                truncated: false,
            })
        }
    }

    /// Append one streamed line.
    fn push(&mut self, line: &str) {
        match self {
            Self::All(text) => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(line);
            }
            Self::Tail(tail) => tail.push(line),
        }
    }

    /// Whether the ceiling dropped anything.
    fn truncated(&self) -> bool {
        match self {
            Self::All(_) => false,
            Self::Tail(tail) => tail.truncated,
        }
    }

    /// The retained text: the retained lines joined by `\n`.
    fn into_string(self) -> String {
        match self {
            Self::All(text) => text,
            Self::Tail(tail) => {
                let mut text = String::with_capacity(tail.bytes);
                for line in &tail.lines {
                    // The same rule the unbounded path applies, so a budgeted
                    // run renders its tail exactly as an unbudgeted one renders
                    // the whole stream.
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(line);
                }
                text
            }
        }
    }
}

impl RetainedTail {
    fn push(&mut self, line: &str) {
        // An over-cap line is kept as its own tail rather than dropped whole:
        // `--progress` output is one ever-growing `\r`-updated line under the
        // default `\n` framing, and the tail is where the fatal message sits.
        let line = match self.max_bytes {
            Some(max) if line.len() > max => {
                self.truncated = true;
                let mut start = line.len() - max;
                while !line.is_char_boundary(start) {
                    start += 1;
                }
                &line[start..]
            }
            _ => line,
        };
        self.bytes += line.len() + usize::from(!self.lines.is_empty());
        self.lines.push_back(line.to_string());

        while self.max_lines.is_some_and(|max| self.lines.len() > max) {
            self.drop_oldest();
        }
        // Never drop the last line to satisfy the byte cap: it was cut to fit
        // above, so the buffer is already within budget once it stands alone.
        while self.max_bytes.is_some_and(|max| self.bytes > max) && self.lines.len() > 1 {
            self.drop_oldest();
        }
    }

    fn drop_oldest(&mut self) {
        if let Some(dropped) = self.lines.pop_front() {
            self.bytes -= dropped.len() + usize::from(!self.lines.is_empty());
            self.truncated = true;
        }
    }
}

pub mod credentials;
pub use credentials::{
    Credential, CredentialProvider, CredentialRequest, CredentialService, EnvToken, FnProvider,
    GitCredentialHelper, Secret, StaticCredential, git_credential_helper, https_host, provider_fn,
};

pub mod logging;
pub use logging::{
    CommandObserver, CommandRecord, CommandStatus, LoggingRunner, StderrObserver, redact_args,
};

/// JSON helpers shared by the forge wrappers, behind the `serde` feature — so the
/// three forge parsers share one `null -> ""` and parse-error convention.
#[cfg(feature = "serde")]
#[cfg_attr(docsrs, doc(cfg(feature = "serde")))]
pub mod json {
    use processkit::{Error, Result};
    use serde::Deserialize;
    use serde::de::DeserializeOwned;

    /// Deserialize a `String` a forge CLI may send as JSON `null` for an empty
    /// optional value: `null` -> empty string, same as an absent key. `#[serde(default)]`
    /// alone covers only an absent key; a present `null` would fail the whole-object
    /// parse. Use as `#[serde(deserialize_with = "vcs_cli_support::json::null_to_empty")]`.
    pub fn null_to_empty<'de, D>(deserializer: D) -> ::core::result::Result<String, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
    }

    /// Deserialize a forge CLI's `--json` output into `T`, mapping a parse failure to
    /// [`ErrorReason::Parse`](processkit::ErrorReason::Parse) tagged with `program`
    /// (the CLI's binary name).
    pub fn from_json<T: DeserializeOwned>(program: &str, json: &str) -> Result<T> {
        serde_json::from_str(json).map_err(|e| Error::parse(program, e.to_string()))
    }
}

/// A configurable ceiling on how much output a potentially large **content**
/// operation may buffer before it is refused — a diff (`diff_text`/`diff`), a
/// file's bytes at a revision (`show_file`/`file_show`), a forge PR/MR diff
/// (`pr_diff`), and the diagnostic (error/progress) output of `clone`/`fetch`.
///
/// This is the single, shared knob the CLI wrappers (`vcs-git`, `vcs-jj`, the
/// forge crates) and the facades (`vcs-core`, `vcs-forge`, the MCP server) all
/// use, so the limit is configured and reasoned about one way across the
/// workspace instead of one ad-hoc cap per client. Set a per-client default with
/// each client's `default_output_budget(...)` builder (inherited by any facade
/// built over that client); raise or lower it for a single call with the
/// `*_within` method variants (`diff_text_within`, `show_file_within`,
/// `pr_diff_within`, …). There is **no un-overridable global constant** — the
/// default is [`unlimited`](OutputBudget::unlimited) (retain everything, the
/// pre-budget behaviour), and every cap is a caller choice.
///
/// It projects onto two [`processkit`] [`OutputBufferPolicy`] shapes, so one
/// budget drives both kinds of bounded output:
///
/// - [`content_policy`](OutputBudget::content_policy) — a **fail-loud** ceiling
///   ([`OverflowMode::Error`]): once the cap is reached the run errors with
///   [`ErrorReason::OutputTooLarge`], carrying the actual (`total_lines`/`total_bytes`)
///   and allowed (`max_lines`/`max_bytes`) sizes — the reported `total_bytes` is
///   in the same unit as the ceiling that fired, see [`bytes`](OutputBudget::bytes).
///   The pipe is still drained (the child never blocks) and output past the
///   ceiling is **counted but never retained**, so memory stays bounded and a
///   truncated result is never handed back as if complete. This is what the
///   content verbs use.
/// - [`diagnostic_policy`](OutputBudget::diagnostic_policy) — a **drop-oldest**
///   tail bound: caps the retained error/progress output of a discard verb
///   (`clone`/`fetch`) *without* converting a real failure into
///   `OutputTooLarge`, so transient-failure classification still reads the
///   (tail-preserved) message. This is the same shape the `gh run watch` cap
///   uses.
///
/// The byte ceiling ([`bytes`](OutputBudget::bytes)) is the load-bearing memory
/// bound: the content verbs capture raw stdout (no line splitting), where the
/// byte cap — not the line cap — is what [`processkit`] enforces. A line ceiling
/// ([`with_max_lines`](OutputBudget::with_max_lines)) is an optional extra that
/// also bounds line-pumped output (a diagnostic stream, a verb's stderr).
///
/// A **streamed** run ([`run_with_progress_within`]) is bounded by the same
/// budget without going through an `OutputBufferPolicy` at all: its events carry
/// every line whatever the command's buffer does, so the drop-oldest tail is
/// applied to the copy that function retains, in its own documented unit. That
/// is what makes a streaming `clone`/`fetch` memory-bounded by the very knob
/// that bounds its captured twin.
///
/// Each captured stream carries the ceiling **independently**: a content verb's
/// raw stdout and its line-pumped stderr each get their own `max_bytes` budget
/// (so one call's worst-case retained memory is about twice the cap, not the
/// cap), and either one reaching it is what raises `OutputTooLarge`. What
/// counts as a "byte" is not the same on the two streams — see
/// [`bytes`](OutputBudget::bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputBudget {
    max_bytes: Option<usize>,
    max_lines: Option<usize>,
}

impl OutputBudget {
    /// No ceiling — retain everything (the default, and the pre-budget
    /// behaviour). [`content_policy`](Self::content_policy) /
    /// [`diagnostic_policy`](Self::diagnostic_policy) return `None`, leaving the
    /// command's own (unbounded) buffer untouched.
    pub const fn unlimited() -> Self {
        Self {
            max_bytes: None,
            max_lines: None,
        }
    }

    /// A byte ceiling of `max_bytes`, in the unit
    /// [`OutputBufferPolicy::max_bytes`] caps. The primary, memory-bounding
    /// knob: it applies to the raw-stdout content path where a line cap would
    /// not. Add a line ceiling with [`with_max_lines`](Self::with_max_lines).
    ///
    /// # What a byte counts as
    ///
    /// The unit differs by **stream**, because the two are captured differently.
    /// Both ceilings fire strictly *past* the cap: output sitting exactly on
    /// `max_bytes` is still accepted.
    ///
    /// - **Raw stdout** — what every content verb reads
    ///   ([`ManagedClient::run_untrimmed`]: `diff_text`, `show_file`, `pr_diff`,
    ///   `template_query`, …): the cap counts the bytes read from the pipe
    ///   **verbatim**, with no line framing to strip and nothing decoded. This
    ///   is what [`processkit`] has always counted on this path, so its 3.0
    ///   switch to raw-pipe-byte accounting did **not** move this ceiling — a
    ///   64 KiB content cap refuses exactly the same reads it did before.
    /// - **Line-pumped stderr** (and any line-captured stream): since processkit
    ///   3.0 the fail-loud ceiling counts the raw bytes read from the pipe —
    ///   **line terminators and invalid-UTF-8 bytes included** — where it
    ///   previously counted only the decoded line *content*. A plain LF stream
    ///   therefore counts one byte per line more than it used to (the LF is now
    ///   charged, not just a CRLF's extra `\r`), so a cap set against the old
    ///   unit trips marginally earlier on this stream.
    ///
    /// [`ErrorReason::OutputTooLarge`]'s reported `total_bytes` is in the same
    /// unit as whichever of those ceilings fired.
    ///
    /// The drop-oldest [`diagnostic_policy`](Self::diagnostic_policy) is a third
    /// case, unaffected by that change: what it *retains* is bounded by the
    /// decoded line-content bytes it holds, not by the raw bytes it saw. A
    /// **streamed** run's local tail ([`run_with_progress_within`]) is a fourth:
    /// it charges the decoded line content *plus* the one `\n` joining each
    /// retained pair — the bytes of the string it actually hands back.
    pub const fn bytes(max_bytes: usize) -> Self {
        Self {
            max_bytes: Some(max_bytes),
            max_lines: None,
        }
    }

    /// Add a line ceiling of `max_lines` (an extra bound on line-pumped output —
    /// diagnostics, a verb's stderr). Composes with any [`bytes`](Self::bytes)
    /// cap; whichever ceiling is reached first fires.
    #[must_use]
    pub const fn with_max_lines(mut self, max_lines: usize) -> Self {
        self.max_lines = Some(max_lines);
        self
    }

    /// Whether no ceiling is set (retain everything).
    pub const fn is_unlimited(&self) -> bool {
        self.max_bytes.is_none() && self.max_lines.is_none()
    }

    /// The configured byte ceiling, if any.
    pub const fn max_bytes(&self) -> Option<usize> {
        self.max_bytes
    }

    /// The configured line ceiling, if any.
    pub const fn max_lines(&self) -> Option<usize> {
        self.max_lines
    }

    /// The **fail-loud** [`OutputBufferPolicy`] for a content verb — errors with
    /// [`ErrorReason::OutputTooLarge`] once the ceiling is reached, never retaining or
    /// returning a truncated tail. `None` when [`unlimited`](Self::unlimited)
    /// (leave the command's default buffer).
    pub fn content_policy(&self) -> Option<OutputBufferPolicy> {
        if self.is_unlimited() {
            return None;
        }
        // A byte cap (Some) keeps the fail-loud ceiling honest even with no line
        // cap: `OverflowMode::Error` is "zero-tolerance" only when *neither* cap
        // is set, so setting `max_bytes` gives it a real ceiling to fire on.
        let mut policy = match self.max_lines {
            Some(lines) => OutputBufferPolicy::fail_loud(lines),
            None => OutputBufferPolicy::unbounded().with_overflow(OverflowMode::Error),
        };
        if let Some(bytes) = self.max_bytes {
            policy = policy.with_max_bytes(bytes);
        }
        Some(policy)
    }

    /// The **drop-oldest** [`OutputBufferPolicy`] for a discard verb's diagnostic
    /// output (`clone`/`fetch`): keeps the last `max_bytes`/`max_lines` (the tail,
    /// where a CLI's fatal line sits) and flags truncation, but does **not** raise
    /// [`ErrorReason::OutputTooLarge`] — so a genuine failure still surfaces as
    /// `ErrorReason::Exit` and stays classifiable ([`is_transient_fetch_error`],
    /// [`is_lock_contention`]). `None` when [`unlimited`](Self::unlimited).
    ///
    /// The retained tail is measured in **decoded line-content** bytes, which is
    /// [`processkit`]'s drop-mode accounting and is deliberately *not* the
    /// raw-pipe-byte unit its 3.0 release re-based the fail-loud
    /// [`OverflowMode::Error`] ceiling onto — so how much tail this projection
    /// keeps is unchanged by that release. Note also that a single line longer
    /// than `max_bytes` is never assembled by the pump and so is dropped whole
    /// (counted only as truncation): keep the byte cap comfortably above a
    /// plausible single fatal line.
    pub fn diagnostic_policy(&self) -> Option<OutputBufferPolicy> {
        if self.is_unlimited() {
            return None;
        }
        let mut policy = match self.max_lines {
            Some(lines) => OutputBufferPolicy::bounded(lines),
            None => OutputBufferPolicy::unbounded(),
        };
        if let Some(bytes) = self.max_bytes {
            policy = policy.with_max_bytes(bytes);
        }
        Some(policy)
    }
}

impl Default for OutputBudget {
    /// [`unlimited`](OutputBudget::unlimited) — the budget is opt-in.
    fn default() -> Self {
        Self::unlimited()
    }
}

/// Generate the cwd-bound forwarders for a CLI wrapper's `…At` view.
///
/// Each CLI wrapper (`vcs-git`, `vcs-jj`, `vcs-github`, `vcs-gitlab`, `vcs-gitea`)
/// exposes a cwd-bound view — `GitAt`, `JjAt`, `GitHubAt`, `GitLabAt`, `GiteaAt` —
/// that holds a reference to the client plus a pre-bound `dir`, and re-exposes the
/// client's methods with `dir` already supplied. The forwarder bodies are
/// byte-identical across the five backends but for a handful of names, so they live
/// here once instead of as a copied `macro_rules!` per crate:
///
/// - `$view` — the bound view type (e.g. `GitAt`). It must be generic over
///   `<'a, R: ProcessRunner>` and have a field named `$field` holding the client
///   plus a `dir: &'a Path` field.
/// - `$field` — the inner field naming the client (e.g. `git`, `gh`, `glab`,
///   `tea`).
/// - `$client` — a **string literal** naming the client type, used in the
///   generated doc strings and rendered as an intra-doc link (e.g. `"Git"` →
///   ``[`Git`]``).
/// - `bare { … }` — methods forwarded verbatim to `self.$field`. Reserve this for
///   the genuinely dir-*independent* calls (`version`, `capabilities`, a
///   `clone`/`git_clone` that names its own destination): the view drops `dir`
///   entirely, so a `bare` method never touches it.
/// - `dir  { … }` — methods that take `self.dir` as their first argument.
/// - `raw  { fn view(args…) -> Ret => target; … }` — the **raw escape hatches**
///   (`run`/`run_raw`/`run_args`/`run_raw_args`). These used to sit in `bare`, so
///   `git.at(dir).run(…)` silently ran in the *process* cwd, not the bound `dir` —
///   a bound handle whose raw call could hit a different repository (M15/T-035).
///   They are now **bound**: the view method `view` forwards to the client's
///   dir-taking `target` (`self.$field.target(self.dir, args…)`), so a raw call
///   *through the view* runs in `dir` like every other `…At` method. The
///   **process-cwd** escape hatch is still there — call `run`/`run_raw`/… on the
///   client itself (`git.run(…)`), not through `.at(dir)`.
///
/// The argument and return types in the method lists resolve in the **calling**
/// crate, so they are written exactly as that wrapper's own methods are. The
/// `ProcessRunner` bound is fully qualified (`::processkit::ProcessRunner`) so the
/// expansion compiles regardless of which items the caller has imported.
///
/// ```ignore
/// vcs_cli_support::at_forwarders! {
///     GitAt, git, "Git",
///     bare { fn version() -> Result<String>; }
///     dir  { fn status() -> Result<Vec<StatusEntry>>; }
///     raw  { fn run(args: &[String]) -> Result<String> => run_in; }
/// }
/// ```
#[macro_export]
macro_rules! at_forwarders {
    (
        $view:ident, $field:ident, $client:literal,
        bare { $( fn $bn:ident( $($ba:ident: $bt:ty),* $(,)? ) -> $br:ty; )* }
        dir  { $( fn $dn:ident( $($da:ident: $dt:ty),* $(,)? ) -> $dr:ty; )* }
        $( raw  { $( fn $rn:ident( $($ra:ident: $rt:ty),* $(,)? ) -> $rr:ty => $rtgt:ident; )* } )?
    ) => {
        impl<'a, R: ::processkit::ProcessRunner> $view<'a, R> {
            $(
                #[doc = concat!("Bound form of [`", $client, "`]'s `", stringify!($bn), "`.")]
                pub async fn $bn(&self, $($ba: $bt),*) -> $br {
                    self.$field.$bn($($ba),*).await
                }
            )*
            $(
                #[doc = concat!("Bound form of [`", $client, "`]'s `", stringify!($dn), "` (with `dir` pre-bound).")]
                pub async fn $dn(&self, $($da: $dt),*) -> $dr {
                    self.$field.$dn(self.dir, $($da),*).await
                }
            )*
            $($(
                #[doc = concat!(
                    "Bound form of [`", $client, "`]'s `", stringify!($rn),
                    "` raw escape hatch — runs the given argv **in the bound `dir`** \
                     (forwards to the client's `", stringify!($rtgt), "`). For the \
                     process-cwd escape hatch, call `", stringify!($rn),
                    "` on [`", $client, "`] directly."
                )]
                pub async fn $rn(&self, $($ra: $rt),*) -> $rr {
                    self.$field.$rtgt(self.dir, $($ra),*).await
                }
            )*)?
        }
    };
}

/// Emit the six **raw escape-hatch** helpers every CLI wrapper hand-writes on its
/// client — `run_args` / `run_raw_args` / `run_in` / `run_raw_in` / `run_args_in`
/// / `run_raw_args_in`.
///
/// These are the `&[&str]` and dir-bound twins of the object-safe `run`/`run_raw`
/// trait methods: `run_args`/`run_raw_args` take `&[&str]` (no `Vec<String>`
/// allocation), the `*_in` variants bind a `dir`, and the `run_raw_*` variants
/// never error on a non-zero exit. Their bodies are byte-identical across the five
/// backends — thin forwards into the `core: ManagedClient` field that
/// [`managed_client!`](crate::managed_client) generates — so, like
/// [`at_forwarders!`](crate::at_forwarders), they live here once instead of as a
/// copied block per crate.
///
/// The generated methods land in a fresh `impl<R: ProcessRunner> $name<R>` block
/// (so invoke this at module scope, next to the crate's other `impl` blocks), and
/// forward to `self.core.run` / `self.core.output_string` (`+ command_in` for the
/// `*_in` variants) — the same field `managed_client!` emits. All paths are fully
/// qualified, so the expansion compiles regardless of what the caller imported.
///
/// The doc strings are generated to match the hand-written ones, cross-links
/// included: `run_raw_args` → `run_args`, each `*_in` → its non-`_in` twin (and
/// back), the object-safe `run`/`run_raw` on `$name`Api, and the bound
/// `$name`At forwarders. The three type names — the client `$name`, its trait
/// `$name`Api, and its bound view `$name`At — are all derived from `$name`.
///
/// - `$name` — the wrapper client type (e.g. `Git`). Names the `impl` target and,
///   via `concat!`, the `…Api` / `…At` link targets (`GitApi`, `GitAt`).
/// - `$binary` — a **string literal** naming the CLI (e.g. `"git"`, `"gh"`), used
///   both as the program in the prose (`` `git <args>` ``) and as the example's
///   receiver (`` `git.run_args(…)` ``).
/// - `$args_example` — a **string literal** with the argv shown in `run_args`'
///   example, i.e. the contents of the `&[…]` (e.g. `"\"status\", \"-s\""`
///   renders `` `git.run_args(&["status", "-s"])` ``).
/// - `$in_infers` — a **string literal** spliced after "as its working directory"
///   in `run_in`'s doc, for backends that infer their target from `dir`'s remote
///   (`", so `gh` infers the repo from `dir`'s remote"`); `""` for the rest.
/// - `$in_flag_note` — a **string literal** for `run_in`'s trailing "Argv is
///   forwarded verbatim (…)" parenthetical — the backend-specific note on what is
///   (not) injected (e.g. ``"only the working directory is bound, no `-C`/extra
///   flag is injected"``).
///
/// ```ignore
/// vcs_cli_support::raw_run_forwarders! {
///     Git, "git", "\"status\", \"-s\"", "",
///     "the same unguarded escape hatch — only the working directory is bound, \
///      no `-C`/extra flag is injected"
/// }
/// ```
#[macro_export]
macro_rules! raw_run_forwarders {
    (
        $name:ident, $binary:literal, $args_example:literal, $in_infers:literal, $in_flag_note:literal $(,)?
    ) => {
        impl<R: ::processkit::ProcessRunner> $name<R> {
            #[doc = concat!(
                "Run `", $binary, " <args>` over string slices — `", $binary, ".run_args(&[",
                $args_example, "])` without allocating a `Vec<String>`. Inherent (not on the \
                 object-safe trait), so it can take `&[&str]`; forwards to the same path as [`",
                stringify!($name), "Api::run`]."
            )]
            pub async fn run_args(&self, args: &[&str]) -> ::processkit::Result<String> {
                self.core.run(args).await
            }

            #[doc = concat!(
                "Like [`run_args`](", stringify!($name), "::run_args) but never errors on a \
                 non-zero exit (mirrors [`", stringify!($name), "Api::run_raw`])."
            )]
            pub async fn run_raw_args(
                &self,
                args: &[&str],
            ) -> ::processkit::Result<::processkit::ProcessResult<String>> {
                self.core.output_string(args).await
            }

            #[doc = concat!(
                "Run `", $binary, " <args>` **in `dir`** (the process is spawned with `dir` as \
                 its working directory", $in_infers, "), returning trimmed stdout — the dir-bound \
                 twin of the process-cwd [`run`](", stringify!($name), "Api::run). This is what [`",
                stringify!($name), "At::run`] forwards to; call [`run`](", stringify!($name),
                "Api::run) on the client for the process-cwd escape hatch. Argv is forwarded \
                 verbatim (", $in_flag_note, ")."
            )]
            pub async fn run_in(
                &self,
                dir: &::std::path::Path,
                args: &[String],
            ) -> ::processkit::Result<String> {
                self.core.run(self.core.command_in(dir, args)).await
            }

            #[doc = concat!(
                "Like [`run_in`](", stringify!($name), "::run_in) but never errors on a non-zero \
                 exit — the dir-bound twin of [`run_raw`](", stringify!($name), "Api::run_raw). \
                 What [`", stringify!($name), "At::run_raw`] forwards to."
            )]
            pub async fn run_raw_in(
                &self,
                dir: &::std::path::Path,
                args: &[String],
            ) -> ::processkit::Result<::processkit::ProcessResult<String>> {
                self.core
                    .output_string(self.core.command_in(dir, args))
                    .await
            }

            #[doc = concat!(
                "Like [`run_args`](", stringify!($name), "::run_args) but **bound to `dir`** — the \
                 `&[&str]` twin of [`run_in`](", stringify!($name), "::run_in). What [`",
                stringify!($name), "At::run_args`] forwards to."
            )]
            pub async fn run_args_in(
                &self,
                dir: &::std::path::Path,
                args: &[&str],
            ) -> ::processkit::Result<String> {
                self.core.run(self.core.command_in(dir, args)).await
            }

            #[doc = concat!(
                "Like [`run_raw_args`](", stringify!($name), "::run_raw_args) but **bound to \
                 `dir`** — the `&[&str]` twin of [`run_raw_in`](", stringify!($name),
                "::run_raw_in). What [`", stringify!($name), "At::run_raw_args`] forwards to."
            )]
            pub async fn run_raw_args_in(
                &self,
                dir: &::std::path::Path,
                args: &[&str],
            ) -> ::processkit::Result<::processkit::ProcessResult<String>> {
                self.core
                    .output_string(self.core.command_in(dir, args))
                    .await
            }
        }
    };
}

/// Emit the common client scaffold every CLI wrapper hand-writes around a
/// [`ManagedClient`].
///
/// `vcs-git`, `vcs-jj`, `vcs-github`, and `vcs-gitlab` each wrap a
/// [`ManagedClient`] in a thin newtype that re-exposes the same handful of
/// constructors and default-applying builders — `new` / `Default` /
/// `with_runner` / `default_timeout` / `default_env` / `default_env_remove` /
/// `default_cancel_on` — with byte-identical bodies and doc strings. This macro
/// generates that shared part so it can't drift between backends; each wrapper
/// keeps its *capability* builders (`with_retry`, `with_credentials`, every verb,
/// the `…At` view, …) hand-written in a separate `impl` block.
///
/// The generated newtype is `struct $name<R: ProcessRunner = JobRunner>` with a
/// single private `core: ManagedClient<R>` field — accessible to the rest of the
/// wrapper crate (same module). All paths are fully qualified, so the expansion
/// compiles regardless of what the caller has imported.
///
/// - `$name` — the wrapper type (e.g. `Git`). The struct-level doc comment (and
///   any other attributes) written before `struct` are attached to it verbatim.
/// - `$binary` — the program the client drives (an expression, typically the
///   crate's `BINARY` const).
/// - `token_env = ($svc, $var)` — *optional*. When given, `new`/`with_runner`
///   chain [`ManagedClient::with_token_env`] so a resolved credential is injected
///   into the `$var` environment variable for service `$svc` (the forge case:
///   `GH_TOKEN`, `GITLAB_TOKEN`). Omit it for the ambient-auth backends (git, jj).
/// - `scrub_env = [ $var, … ]` — *optional*. When given, `new`/`with_runner`
///   chain [`ManagedClient::default_env_remove`] for each var, so **every** client
///   the macro generates drops those inherited environment variables by default
///   (`vcs-git` uses it to scrub the repo-redirector vars — `GIT_DIR`, … — so a
///   value leaking from the parent process can't retarget commands). Must come
///   *after* `token_env` when both are present.
///
/// ```ignore
/// vcs_cli_support::managed_client! {
///     /// The real GitHub client.
///     pub struct GitHub => BINARY, token_env = (CredentialService::GitHub, "GH_TOKEN")
/// }
/// vcs_cli_support::managed_client! {
///     /// The real Git client — scrubs the repo-redirector env vars by default.
///     pub struct Git => BINARY, scrub_env = ["GIT_DIR", "GIT_WORK_TREE"]
/// }
/// ```
#[macro_export]
macro_rules! managed_client {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident => $binary:expr
        $(, token_env = ($svc:expr, $var:expr) )?
        $(, scrub_env = [ $($scrub:expr),* $(,)? ] )?
        $(,)?
    ) => {
        $(#[$meta])*
        $vis struct $name<R: ::processkit::ProcessRunner = ::processkit::JobRunner> {
            core: $crate::ManagedClient<R>,
        }

        // Manual Debug: no `R: Debug` bound (matches `ManagedClient`'s own impl),
        // delegating straight to `core` — `ManagedClient::fmt` already redacts any
        // configured credential provider / token-env binding, so nothing secret
        // reaches `{:?}` here either.
        impl<R: ::processkit::ProcessRunner> ::core::fmt::Debug for $name<R> {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("core", &self.core)
                    .finish()
            }
        }

        impl $name<::processkit::JobRunner> {
            /// Create a client driving the real job-backed runner.
            pub fn new() -> Self {
                Self { core: $crate::ManagedClient::new($binary)
                    $(.with_token_env($svc, $var))?
                    $($(.default_env_remove($scrub))*)?
                }
            }
        }

        impl ::core::default::Default for $name<::processkit::JobRunner> {
            fn default() -> Self {
                Self::new()
            }
        }

        impl<R: ::processkit::ProcessRunner> $name<R> {
            /// Create a client driving `runner` — inject a fake in tests.
            pub fn with_runner(runner: R) -> Self {
                Self {
                    core: $crate::ManagedClient::with_runner($binary, runner)
                        $(.with_token_env($svc, $var))?
                        $($(.default_env_remove($scrub))*)?,
                }
            }

            /// Apply a default timeout to every command this client builds.
            pub fn default_timeout(mut self, timeout: ::core::time::Duration) -> Self {
                self.core = self.core.default_timeout(timeout);
                self
            }

            /// Set an environment variable on every command this client builds.
            pub fn default_env(
                mut self,
                key: impl ::core::convert::AsRef<::std::ffi::OsStr>,
                value: impl ::core::convert::AsRef<::std::ffi::OsStr>,
            ) -> Self {
                self.core = self.core.default_env(key, value);
                self
            }

            /// Remove an inherited environment variable on every command this client builds.
            pub fn default_env_remove(
                mut self,
                key: impl ::core::convert::AsRef<::std::ffi::OsStr>,
            ) -> Self {
                self.core = self.core.default_env_remove(key);
                self
            }

            /// Cancel every command this client builds when `token` fires.
            ///
            /// Network `fetch`/`push`/`clone` commands additionally use the shared
            /// [`FETCH_TIMEOUT_GRACE`] cancellation window when this token fires,
            /// including the Windows-only soft-trigger opt-in where processkit can
            /// deliver it. The process outcome remains
            /// [`ErrorReason::Cancelled`]; the grace changes only the teardown path.
            /// Other commands keep their existing immediate-cancellation policy.
            pub fn default_cancel_on(mut self, token: ::processkit::CancellationToken) -> Self {
                self.core = self.core.default_cancel_on(token);
                self
            }

            /// Apply a default [`OutputBudget`](vcs_cli_support::OutputBudget) to the
            /// potentially large **content** operations this client builds — the
            /// diff/show/pr-diff verbs and the `clone`/`fetch` diagnostic capture.
            /// Inherited by any facade built over this client. The default is
            /// [`OutputBudget::unlimited`](vcs_cli_support::OutputBudget::unlimited)
            /// (retain everything); a single call can still override it via the
            /// `*_within` method variants.
            pub fn default_output_budget(mut self, budget: $crate::OutputBudget) -> Self {
                self.core = self.core.default_output_budget(budget);
                self
            }
        }
    };
}

/// Injection guard for bare positional argv slots: a caller-supplied value with a
/// leading `-` would be parsed by the CLI as a *flag* (verified: `git checkout
/// -evil` → "unknown switch"; jj likewise), and an empty (or whitespace-only)
/// value silently changes most commands' meaning. Refuse both before anything
/// spawns, surfacing an [`ErrorReason::Spawn`] naming `program`. An interior NUL is
/// refused too (it can't be passed in argv and otherwise surfaces as an opaque
/// OS spawn error). Flag-VALUE positions (`-m <msg>`, `--branch <b>`) don't need
/// this — the CLI consumes the next token verbatim there.
///
/// The leading-`-` test is applied to the **trimmed** value, so a value like
/// `" --upload-pack=…"` (leading whitespace) is still refused — the empty-check
/// and the flag-check now agree on what "the value" is.
pub fn reject_flag_like(program: &str, what: &str, value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') || value.contains('\0') {
        return Err(Error::spawn(
            program,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{what} {value:?} would be parsed as a flag (or is empty / contains NUL) — \
                     refusing to pass it as a positional argument"
                ),
            ),
        ));
    }
    Ok(())
}

/// R7 clone-cleanup: whether `dest` is safe to remove if a `clone`/`git_clone`
/// about to run into it fails — either **provably absent** (`read_dir` fails
/// with `NotFound`), or an already-empty directory. Compute this **before**
/// running the clone, and pass the result to [`cleanup_failed_clone_dest`] on
/// the error path — `git`/`jj` both refuse to clone into a **non-empty**
/// existing directory, so if `dest` already had contents going in, a failure
/// means that refusal, and the caller's pre-existing data must never be
/// deleted. Re-checking emptiness *after* the clone ran would be wrong: a
/// failed clone can leave `dest` partially populated, so a post-hoc check
/// could wrongly call a partial clone's leftovers "empty" (or simply disagree
/// with the pre-clone state).
///
/// Any `read_dir` failure *other than* `NotFound` (permission denied, a
/// transient I/O error, `dest` being a plain file — `NotADirectory`) is
/// treated as **not** cleanable: it doesn't prove `dest` is absent, and
/// `dest` may well be a pre-existing non-empty directory the caller can't
/// read into right now. Deleting on an unproven guess would risk
/// `remove_dir_all`-ing a directory full of the caller's data; cleanup simply
/// becoming a no-op is the safe degradation (the clone itself already failed
/// with a clear git/jj error).
///
/// Shared by `vcs_git::clone_repo` and `vcs_jj::git_clone`, which previously
/// carried a byte-identical copy of this check plus its own best-effort
/// `remove_dir_all` on the error path.
pub fn clone_dest_cleanable(dest: &Path) -> bool {
    match std::fs::read_dir(dest) {
        Err(err) => err.kind() == std::io::ErrorKind::NotFound, // proven absent
        Ok(mut entries) => entries.next().is_none(),            // an empty directory
    }
}

/// Best-effort cleanup of a failed clone's partial `dest` (R7) — call only on
/// the clone's error path, passing `cleanable` as computed by
/// [`clone_dest_cleanable`] **before** the clone ran. A no-op when `cleanable`
/// is `false` — including whenever `dest`'s state couldn't be proven safe
/// (absent, or an already-empty directory): this never touches a non-empty
/// pre-existing `dest`, nor one `clone_dest_cleanable` simply failed to read.
/// Swallows a `remove_dir_all` failure (e.g. another process holding a file
/// open) — this is opportunistic tidy-up, not something a clone failure
/// should itself fail on.
pub fn cleanup_failed_clone_dest(dest: &Path, cleanable: bool) {
    if cleanable {
        let _ = std::fs::remove_dir_all(dest);
    }
}

/// Total attempts for a transient-retried `fetch` (1 try + 2 retries).
pub const FETCH_ATTEMPTS: u32 = 3;
/// Fixed backoff between fetch retries.
pub const FETCH_BACKOFF: Duration = Duration::from_millis(500);
/// Grace period for a network operation that times out **or is cancelled**: on Unix
/// processkit sends its graceful terminate signal and waits this long before
/// hard-killing. On Windows
/// the grace window is useful only when a child has a soft trigger: the shared
/// [`apply_fetch_completion_policy`] opts console children into `CTRL_BREAK`, and
/// processkit also tries `WM_CLOSE` for windowed children. A child without a
/// shared console, including one started with `create_no_window` or
/// `DETACHED_PROCESS`, skips the soft trigger and falls back to the atomic job
/// kill. The cancellation outcome is still [`ErrorReason::Cancelled`]. The policy
/// only takes effect when the relevant timeout or `default_cancel_on` token is set;
/// a network operation with neither is unaffected.
pub const FETCH_TIMEOUT_GRACE: Duration = Duration::from_secs(2);

/// Apply the shared completion policy to a network command.
///
/// The policy applies [`FETCH_TIMEOUT_GRACE`] to both timeout and cancellation. On
/// Windows it additionally opts the direct console child into `CTRL_BREAK`, so a
/// child that handles the event can flush buffers, close connections, and release
/// locks during the grace window. Delivery requires a console shared with the
/// child; GUI/service callers without one and children created with
/// [`Command::create_no_window`](processkit::Command::create_no_window) or
/// `DETACHED_PROCESS` receive no console event. Windowed children may still
/// receive processkit's best-effort `WM_CLOSE`, and any survivor is hard-killed
/// after the grace window. On Unix the Windows builder is a no-op, so the
/// existing signal → grace → hard-kill semantics are unchanged. The helper does not
/// alter the structured cancellation outcome.
pub fn apply_fetch_completion_policy(command: Command) -> Command {
    let command = command
        .timeout_grace(FETCH_TIMEOUT_GRACE)
        .cancel_grace(FETCH_TIMEOUT_GRACE);
    #[cfg(windows)]
    {
        command.windows_graceful_ctrl_break()
    }
    #[cfg(not(windows))]
    {
        command
    }
}

/// Lower-case substrings marking a merge that stopped on conflicts.
const CONFLICT_MARKERS: &[&str] = &["conflict (", "automatic merge failed"];
/// Lower-case substrings marking a commit that found nothing to record.
const NOTHING_TO_COMMIT_MARKERS: &[&str] = &["nothing to commit", "nothing added to commit"];
/// Lower-case substrings marking a transient (retryable) network/fetch failure.
/// The timeout markers are kept *specific* (`connection timed out` /
/// `operation timed out`) rather than a bare `timed out`, which would also match
/// unrelated, non-network "timed out" messages (a lock wait, a hook) and trigger a
/// spurious fetch retry.
const TRANSIENT_FETCH_MARKERS: &[&str] = &[
    "could not resolve host",
    "couldn't resolve host",
    "temporary failure in name resolution",
    "connection timed out",
    "connection refused",
    "operation timed out",
    "network is unreachable",
    "failed to connect",
    "could not read from remote repository",
    "the remote end hung up",
    "early eof",
    "rpc failed",
];

/// Whether `err` is an [`ErrorReason::Exit`] whose captured output contains any
/// marker.
///
/// Matches the reason rather than reading [`Error::stdout`]/[`Error::stderr`]: those
/// accessors also return the partial output of a `Timeout` or `Signalled` run, which
/// must not be scanned for a "conflict"/"nothing to commit"/lock marker — only a
/// completed non-zero exit carries a verdict the markers describe.
fn exit_output_matches(err: &Error, markers: &[&str]) -> bool {
    let ErrorReason::Exit { stdout, stderr, .. } = err.reason() else {
        return false;
    };
    let out = stdout.to_ascii_lowercase();
    let errt = stderr.to_ascii_lowercase();
    markers.iter().any(|m| out.contains(m) || errt.contains(m))
}

/// Whether a failed `merge`/`merge_commit` stopped on a merge conflict. (jj
/// surfaces conflicts as state rather than as errors, so this only fires on git
/// output — see `vcs_core::Error::is_merge_conflict`.)
pub fn is_merge_conflict(err: &Error) -> bool {
    exit_output_matches(err, CONFLICT_MARKERS)
}

/// Whether a failed `commit`/`commit_paths` reported nothing to commit (a clean
/// tree), as opposed to a real error.
pub fn is_nothing_to_commit(err: &Error) -> bool {
    exit_output_matches(err, NOTHING_TO_COMMIT_MARKERS)
}

/// Whether a failed `fetch`/`fetch_branch`/`remote_branch_exists` looks
/// transient (DNS, a dropped connection, a fast network blip) and is worth
/// retrying.
///
/// A processkit-level **timeout** is deliberately **not** classified transient
/// (R6). A `.timeout()`-bounded run that expired has already consumed the caller's
/// full deadline — retrying it would multiply the wall-clock by [`FETCH_ATTEMPTS`]
/// (e.g. a black-holed remote under a 120 s deadline would block ≈ 6 min, three
/// times the advertised ceiling). The deadline *is* the patience budget; a caller
/// who wants longer should raise the timeout, not have it silently tripled. Fast
/// transient failures (the io-level and marker cases below) still retry, because
/// they fail quickly and a retry is cheap.
pub fn is_transient_fetch_error(err: &Error) -> bool {
    // An io-level transient from the spawn itself (interrupted / would-block / busy),
    // which processkit classifies via `Error::is_transient()` (it covers `Spawn`/`Io`,
    // not `Exit`/`Timeout`, so it composes cleanly with the marker scan below).
    err.is_transient() || exit_output_matches(err, TRANSIENT_FETCH_MARKERS)
}

/// Lower-case substrings marking a **whole-repository / working-copy lock**
/// contention failure — another process held the *one* repo-wide lock, so the
/// command **never started** (clean, pre-execution) and touched nothing.
///
/// These are deliberately limited to the locks that guard the *entire* operation
/// up front, so retrying is safe even on a **mutating** command: the repo was not
/// modified at all. We intentionally do **not** include per-ref lock messages
/// (`cannot lock ref`, `<ref>.lock`/`packed-refs.lock: File exists`): a multi-ref
/// `push`/`fetch` updates refs sequentially, so a ref-lock failure can arrive
/// *after* earlier refs already moved — replaying that is not idempotent. Network
/// markers
/// ([`TRANSIENT_FETCH_MARKERS`]) and conflict/exit failures are likewise absent.
const LOCK_CONTENTION_MARKERS: &[&str] = &[
    // git: the whole-repo index lock (pre-write). Match the **locale-stable path
    // fragment** `index.lock`, not the translated `': File exists'` suffix — git
    // localizes its messages, so a `LANG=de_DE` runner would never match the full
    // English phrase. `index.lock` names the index lock specifically; per-ref locks
    // (`<ref>.lock`, `packed-refs.lock`) are ruled out by the `refs/` guard in
    // `is_lock_contention`. (This matches any `index.lock` *create* failure — a
    // held lock, or e.g. `Permission denied` — all pre-write, so retrying is safe.)
    "index.lock",
    // jj: the working-copy lock and the operation-heads lock (both pre-mutation).
    // These are jj's exact wordings (lower-cased for the classifier). NOTE: modern
    // jj generally **blocks** on these locks until they're free rather than failing,
    // so contention usually surfaces as a wait, not a classifiable error — these
    // markers catch only the residual cases where jj does surface a lock error.
    "failed to lock working copy",
    "failed to lock operation heads store",
];

/// Whether `err` is a **whole-repository lock-contention** failure — another
/// process held git's `index.lock` or jj's working-copy / op-heads lock, so the
/// command couldn't even start. Such a failure is *pre-execution* and therefore
/// safe to retry even on a **mutating** operation (the repo was never modified).
/// Per-ref lock failures (`cannot lock ref`, `<ref>.lock`) are deliberately **not**
/// classified here — they can occur mid-way through a multi-ref `push`/`fetch`,
/// where a retry would not be idempotent. Conflict, "nothing to commit", a real
/// non-zero exit, a timeout, a signal, or a missing binary are also **not** lock
/// contention and must not be retried this way.
pub fn is_lock_contention(err: &Error) -> bool {
    // Rule out a **per-ref** lock first: it is *not* safely retryable (a multi-ref
    // push/fetch can fail one ref's lock after earlier refs already moved). git's
    // per-ref lock lives under `refs/` (`…/refs/heads/<name>.lock`) and its message
    // names `refs/…`, whereas the whole-repo `index.lock` (`<gitdir>/index.lock`)
    // never does — so a `refs/` mention excludes it, locale-independently. This also
    // stops a branch literally named `index`/`reindex` (whose `…/reindex.lock`
    // contains the substring `index.lock`) from matching the bare `index.lock`
    // marker. (A repo whose *path* contains `refs/` then misses the index-lock retry
    // — a benign false-negative, safer than a wrong retry.)
    if exit_output_matches(err, &["refs/"]) {
        return false;
    }
    exit_output_matches(err, LOCK_CONTENTION_MARKERS)
}

/// Whether `err` is an **input rejection** — a bad caller argument, encoded as an
/// [`ErrorReason::Spawn`] whose source is `io::ErrorKind::InvalidInput`. This is the
/// pattern the toolkit's own argument guards raise ([`reject_flag_like`] and the
/// validating newtypes `RefName`/`RevSpec`/`RevsetExpr`) for a value that would be
/// misparsed as a flag, is empty, or contains a NUL — and it also covers the
/// spawn-time `InvalidInput` the OS raises for an un-spawnable argument (an interior
/// NUL in a flag-value, or Windows' batch-arg-escaping refusal). All are genuine
/// bad input, distinct from a real spawn failure (missing binary → `NotFound`, no
/// perms → `PermissionDenied`) or a non-zero exit. A binding maps this to a
/// `ValueError`; the facades re-expose it as `Error::is_invalid_input()`.
pub fn is_invalid_input(err: &Error) -> bool {
    matches!(
        err.reason(),
        ErrorReason::Spawn { source, .. } if source.kind() == std::io::ErrorKind::InvalidInput
    )
}

/// A bounded retry strategy: how many attempts, the (exponential) backoff between
/// them, and whether to add full jitter. Used by [`ManagedClient`] to retry
/// [`is_lock_contention`] failures. The [`Default`] is [`none`](RetryPolicy::none)
/// (no retry) — retry is **opt-in**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RetryPolicy {
    /// Total attempts including the first; `1` means no retry.
    pub attempts: u32,
    /// Delay before the first retry; doubles each subsequent retry (capped by
    /// [`max_backoff`](RetryPolicy::max_backoff)). `ZERO` means retry immediately.
    pub base_backoff: Duration,
    /// Upper bound on the (pre-jitter) backoff delay. `ZERO` means uncapped.
    pub max_backoff: Duration,
    /// Apply **full jitter** — the actual delay is uniform in `[0, computed]` — to
    /// avoid a thundering herd when many workers retry against one repository.
    pub jitter: bool,
}

impl RetryPolicy {
    /// No retry: a single attempt. The default.
    pub const fn none() -> Self {
        Self {
            attempts: 1,
            base_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
            jitter: false,
        }
    }

    /// A sensible default for repository lock contention: a handful of attempts
    /// with short, jittered, exponential backoff (25 ms → 500 ms).
    pub const fn lock_contention() -> Self {
        Self {
            attempts: 5,
            base_backoff: Duration::from_millis(25),
            max_backoff: Duration::from_millis(500),
            jitter: true,
        }
    }

    /// Set the total number of attempts (clamped to at least 1).
    pub fn attempts(mut self, attempts: u32) -> Self {
        self.attempts = attempts.max(1);
        self
    }

    /// Set the base backoff (the delay before the first retry).
    pub fn base_backoff(mut self, backoff: Duration) -> Self {
        self.base_backoff = backoff;
        self
    }

    /// Cap the (pre-jitter) backoff delay; `ZERO` leaves it uncapped.
    pub fn max_backoff(mut self, max: Duration) -> Self {
        self.max_backoff = max;
        self
    }

    /// Toggle full jitter on the backoff delay.
    pub fn with_jitter(mut self, jitter: bool) -> Self {
        self.jitter = jitter;
        self
    }
}

impl Default for RetryPolicy {
    /// No retry — retry is opt-in.
    fn default() -> Self {
        Self::none()
    }
}

/// The (possibly jittered) backoff before the `retry_index`-th retry (0 = first).
fn backoff_for(policy: &RetryPolicy, retry_index: u32) -> Duration {
    if policy.base_backoff.is_zero() {
        return Duration::ZERO;
    }
    let base = policy.base_backoff.as_nanos();
    let scaled = base.saturating_mul(1u128 << retry_index.min(20));
    let capped = if policy.max_backoff.is_zero() {
        scaled
    } else {
        scaled.min(policy.max_backoff.as_nanos())
    };
    let delay = Duration::from_nanos(capped.min(u64::MAX as u128) as u64);
    if policy.jitter {
        full_jitter(delay)
    } else {
        delay
    }
}

/// Full jitter: a uniform delay in `[0, max]`. Dependency-free randomness via the
/// OS-seeded [`RandomState`](std::collections::hash_map::RandomState) — good enough
/// to de-correlate retries, not cryptographic.
fn full_jitter(max: Duration) -> Duration {
    use std::hash::{BuildHasher, Hasher};
    let nanos = max.as_nanos();
    if nanos == 0 {
        return Duration::ZERO;
    }
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(nanos as u64);
    let r = hasher.finish() as u128;
    Duration::from_nanos((r % (nanos + 1)).min(u64::MAX as u128) as u64)
}

/// The structured [`ErrorReason::Cancelled`] to surface when a cancellation token
/// aborts the retry backoff, named for the same program as the attempt that just
/// failed — so it reads exactly like the `Cancelled` a [`processkit`] run raises when
/// its own [`default_cancel_on`](ManagedClient::default_cancel_on) token kills an
/// in-flight process. Falls back to an empty program name only if the last error
/// carried none (every real attempt error names its program).
fn cancelled_error(last_err: &Error) -> Error {
    ErrorReason::Cancelled {
        program: last_err.program().unwrap_or_default().to_owned(),
    }
    .into()
}

/// Run `op`, retrying its result while `should_retry` says so and `policy` has
/// attempts left, sleeping the (jittered, exponential) backoff between tries. The
/// op is re-invoked from scratch each attempt, so it must be idempotent for the
/// errors `should_retry` selects (lock-contention failures are — the command never
/// ran). Returns the first `Ok`, or the last `Err`.
///
/// When `cancel` is `Some`, the backoff between attempts is **cancellation-aware**:
/// if the token fires before or during a wait, the wait stops immediately and the
/// whole retry aborts with a structured [`ErrorReason::Cancelled`] (naming the
/// just-failed attempt's program). It does **not** sit out the rest of the delay,
/// and — crucially — it launches **no** further attempt, so a cancel can never race
/// a fresh op into flight (the attempt count stays deterministic). Pass `None` to
/// keep the plain, uninterruptible backoff (behaviour unchanged from before this
/// parameter existed).
///
/// The **first** attempt always runs; cancellation is only observed around the
/// backoff. An `op` bound to the same token (a [`ManagedClient`] built with
/// [`default_cancel_on`](ManagedClient::default_cancel_on)) still surfaces its own
/// `Cancelled` when the token was already fired as it ran — `should_retry` returns
/// `false` for that terminal error, so the loop returns it without a backoff anyway.
pub async fn retry_async<T, Fut>(
    policy: &RetryPolicy,
    cancel: Option<&CancellationToken>,
    should_retry: impl Fn(&Error) -> bool,
    mut op: impl FnMut() -> Fut,
) -> Result<T>
where
    Fut: Future<Output = Result<T>>,
{
    let attempts = policy.attempts.max(1);
    for attempt in 1..=attempts {
        match op().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                if attempt == attempts || !should_retry(&err) {
                    return Err(err);
                }
                let delay = backoff_for(policy, attempt - 1);
                match cancel {
                    // Cancellation-aware backoff. `run_until_cancelled` drops the
                    // pending sleep the instant the token fires (or returns at once
                    // if it is already fired), so a cancelled retry never waits out
                    // the full delay. We then abort with a structured `Cancelled`
                    // instead of looping into another attempt — the same check also
                    // covers a zero delay and a cancel that lands right as the wait
                    // ends, so no attempt is ever launched after the token fired.
                    Some(token) => {
                        if !delay.is_zero() {
                            let _ = token.run_until_cancelled(tokio::time::sleep(delay)).await;
                        }
                        if token.is_cancelled() {
                            return Err(cancelled_error(&err));
                        }
                    }
                    // No token: the original plain, uninterruptible backoff.
                    None => {
                        if !delay.is_zero() {
                            tokio::time::sleep(delay).await;
                        }
                    }
                }
            }
        }
    }
    unreachable!("the loop returns on the final attempt")
}

/// A [`CliClient`] wrapper that adds two opt-in concerns the CLI wrappers
/// (`vcs-git`, `vcs-jj`, `vcs-github`, `vcs-gitlab`) all share, without touching a
/// single call site:
///
/// 1. **Lock-contention retry** ([`is_lock_contention`]) per a [`RetryPolicy`] —
///    off by default ([`RetryPolicy::none`]); enable with
///    [`with_retry`](ManagedClient::with_retry). Safe even for mutating commands,
///    since lock contention is a clean pre-execution failure.
/// 2. **Credential injection** from an opt-in [`CredentialProvider`] — off by
///    default (no provider); attach one with
///    [`with_credentials`](ManagedClient::with_credentials). When a forge
///    *token-env* binding is configured
///    ([`with_token_env`](ManagedClient::with_token_env)), every command run
///    through this client gets the resolved token in that environment variable
///    (e.g. `GH_TOKEN`). Backends that inject the secret differently (git's
///    `credential.helper`) instead call
///    [`resolve_credential`](ManagedClient::resolve_credential) at the command
///    site. Resolution happens once per call, before the retry loop. A
///    [`with_expected_host`](ManagedClient::with_expected_host) binding travels as
///    the request's host so a **host-keyed** provider selects the right instance's
///    secret; the `Ok(None)` / `Err` fallback (defer to ambient vs. fail-closed
///    abort) is defined on
///    [`resolve_credential`](ManagedClient::resolve_credential).
///
/// Both default to inert, so a client with neither configured behaves exactly
/// like a bare `CliClient`.
pub struct ManagedClient<R: ProcessRunner = JobRunner> {
    inner: CliClient<R>,
    retry: RetryPolicy,
    credentials: Option<Arc<dyn CredentialProvider>>,
    /// When set, the token is auto-injected into this env var on every command,
    /// resolved for this service. Used by the forge clients (`GH_TOKEN`, …).
    token_env: Option<(CredentialService, &'static str)>,
    /// The remote host this client targets, set when a forge `with_host` builder
    /// bound one. It becomes the [`CredentialRequest`]'s host on the auto-injected
    /// token-env path (the forge case), so a **host-keyed** provider selects the
    /// secret for *this* host and never a neighbouring instance's. `None` leaves the
    /// request host unset — a host-keyed provider that can't place the request
    /// returns `Ok(None)` and the command falls back to ambient auth, rather than
    /// being handed the wrong host's secret.
    expected_host: Option<String>,
    /// A copy of the [`default_cancel_on`](Self::default_cancel_on) token, kept here
    /// (as well as on `inner`, which bounds the spawned *process*) so the retry loop
    /// can cut a lock-contention backoff short the instant cancellation fires,
    /// instead of sleeping out the full delay before the next attempt.
    cancel: Option<CancellationToken>,
    /// The default output budget applied to the potentially large **content**
    /// verbs this client builds (via [`run_untrimmed`](Self::run_untrimmed)), to
    /// the output a **streamed** run retains
    /// ([`run_with_progress`](Self::run_with_progress)) and, on request, to a
    /// discard verb's diagnostic capture
    /// ([`budget_diagnostics`](Self::budget_diagnostics)). Defaults to
    /// [`OutputBudget::unlimited`] — no ceiling — so a client that never sets one
    /// behaves exactly as before. A single call overrides it via
    /// [`run_untrimmed_within`](Self::run_untrimmed_within) /
    /// [`run_with_progress_within`](Self::run_with_progress_within).
    output_budget: OutputBudget,
    /// Optional resettable output-inactivity window applied to streamed runs.
    /// `None` preserves the pre-watchdog behaviour: a streamed command is bounded
    /// only by its absolute deadline or cancellation policy.
    inactivity_timeout: Option<Duration>,
}

impl<R: ProcessRunner> fmt::Debug for ManagedClient<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedClient")
            .field("inner", &self.inner)
            .field("retry", &self.retry)
            // Never render the provider itself (it may close over a secret); just
            // whether one is configured, plus the token-env binding.
            .field("credentials", &self.credentials.is_some())
            .field("token_env", &self.token_env)
            // A hostname, not a secret — safe to render; helps distinguish a
            // host-bound client's `{:?}` from an unbound one.
            .field("expected_host", &self.expected_host)
            // The token itself is not meaningfully renderable; whether one is set
            // matches `inner`'s own `has_default_cancel`, kept explicit here too.
            .field("has_cancel", &self.cancel.is_some())
            // A small plain cap (no secret) — safe to render.
            .field("output_budget", &self.output_budget)
            .field("inactivity_timeout", &self.inactivity_timeout)
            .finish()
    }
}

impl ManagedClient<JobRunner> {
    /// A retrying client driving `program` on the real job-backed runner (no retry
    /// until [`with_retry`](ManagedClient::with_retry)).
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            inner: CliClient::new(program),
            retry: RetryPolicy::none(),
            credentials: None,
            token_env: None,
            expected_host: None,
            cancel: None,
            output_budget: OutputBudget::unlimited(),
            inactivity_timeout: None,
        }
    }
}

impl<R: ProcessRunner> ManagedClient<R> {
    /// A retrying client driving `program` on `runner` — inject a fake in tests.
    pub fn with_runner(program: impl AsRef<OsStr>, runner: R) -> Self {
        Self {
            inner: CliClient::with_runner(program, runner),
            retry: RetryPolicy::none(),
            credentials: None,
            token_env: None,
            expected_host: None,
            cancel: None,
            output_budget: OutputBudget::unlimited(),
            inactivity_timeout: None,
        }
    }

    /// Set the lock-contention retry policy (opt-in; default is no retry).
    pub fn with_retry(mut self, policy: RetryPolicy) -> Self {
        self.retry = policy;
        self
    }

    /// The active retry policy.
    pub fn retry_policy(&self) -> RetryPolicy {
        self.retry
    }

    /// Attach a [`CredentialProvider`] (opt-in; default is none → ambient auth).
    /// The provider is consulted per operation: automatically when a
    /// [`with_token_env`](ManagedClient::with_token_env) binding is set, or
    /// on demand via [`resolve_credential`](ManagedClient::resolve_credential).
    ///
    /// **Precedence:** a resolved token is injected *after* any
    /// [`default_env`](ManagedClient::default_env), so the provider wins over a
    /// static default and over the ambient CLI login. **Cancellation:** a
    /// [`default_cancel_on`](ManagedClient::default_cancel_on) token bounds the
    /// spawned *process*, not provider resolution — if your provider does slow I/O
    /// (a vault lookup), bound it yourself.
    #[must_use]
    pub fn with_credentials(mut self, provider: Arc<dyn CredentialProvider>) -> Self {
        self.credentials = Some(provider);
        self
    }

    /// Bind the resolved token to an environment variable injected on **every**
    /// command this client runs (the forge case: `GH_TOKEN`, `GITLAB_TOKEN`). The
    /// `service` tags the [`CredentialRequest`]. No effect without a provider.
    #[must_use]
    pub fn with_token_env(mut self, service: CredentialService, var: &'static str) -> Self {
        self.token_env = Some((service, var));
        self
    }

    /// Bind the remote host this client targets (set by a forge `with_host`): it
    /// travels as the [`CredentialRequest`]'s host whenever the token-env path
    /// resolves a credential, so a **host-keyed** [`CredentialProvider`] returns the
    /// secret for *this* host and nothing else — one client can't inject a
    /// neighbouring instance's token. Without it the request host is unset (the
    /// pre-host-context behaviour). No effect without a provider and a
    /// [`with_token_env`](Self::with_token_env) binding.
    #[must_use]
    pub fn with_expected_host(mut self, host: impl Into<String>) -> Self {
        self.expected_host = Some(host.into());
        self
    }

    /// Whether a credential provider is configured.
    #[must_use]
    pub fn has_credentials(&self) -> bool {
        self.credentials.is_some()
    }

    /// Resolve a credential for `service`/`host` from the configured provider, or
    /// `Ok(None)` if no provider is set or it defers to ambient auth. Backends
    /// that inject the secret at the command site (git's `credential.helper`) call
    /// this directly; the forge token-env path uses it internally.
    ///
    /// **Fallback policy (identical for read and write operations):**
    /// - **No provider**, or the provider returns **`Ok(None)`** → `Ok(None)`:
    ///   defer to the CLI's ambient auth, exactly as if no provider were configured.
    /// - A credential whose secret is **empty / whitespace-only** → treated as
    ///   `Ok(None)` (ambient): injecting an empty token would *override* the ambient
    ///   login with nothing instead of deferring to it.
    /// - The provider returns **`Err`** → the error propagates and **aborts** the
    ///   operation (**fail-closed**). A provider that cannot resolve (a vault outage)
    ///   is never silently downgraded to ambient auth.
    ///
    /// Passing the operation's `host` is what lets a **host-keyed** provider return
    /// the secret for *that* host (or `Ok(None)` for one it does not handle) — so it
    /// never hands back a neighbouring instance's token when the host is known, and
    /// an unknown/absent host defers to ambient rather than substituting a default
    /// secret.
    pub async fn resolve_credential(
        &self,
        service: CredentialService,
        host: Option<&str>,
    ) -> Result<Option<Credential>> {
        let Some(provider) = &self.credentials else {
            return Ok(None);
        };
        let request = CredentialRequest { service, host };
        // An empty (or whitespace-only) secret is not a usable credential —
        // injecting an empty `GH_TOKEN`/`GITLAB_TOKEN` (or a `password=` line)
        // would *override* the ambient login with nothing rather than defer to it.
        // Treat it as `None` (ambient), keeping the "no usable credential ⇒
        // ambient auth" contract consistent regardless of which adapter produced
        // it (matching `EnvToken`'s own whitespace-only ⇒ unset rule).
        Ok(provider
            .credential(&request)
            .await?
            .filter(|cred| !cred.secret().expose().trim().is_empty()))
    }

    /// Materialize `call` into a [`Command`], injecting the forge token env if a
    /// [`with_token_env`](ManagedClient::with_token_env) binding and a provider
    /// are both configured. The single place the auto-injection happens, shared by
    /// every retrying verb.
    ///
    /// The request carries this client's
    /// [`expected_host`](ManagedClient::with_expected_host) (when a forge `with_host`
    /// set one), so a host-keyed provider picks the secret for that host. The
    /// resolution follows the [`resolve_credential`](ManagedClient::resolve_credential)
    /// fallback policy: `Ok(None)` (nothing for this host, or an empty secret) leaves
    /// the command on ambient auth — no env is set — while an `Err` **aborts** the
    /// command (fail-closed, via `?`). A provider that can't resolve is never
    /// silently downgraded to ambient, and a wrong host's secret is never
    /// substituted. This holds identically for read and write verbs (both route
    /// through here).
    async fn prepare(&self, call: impl IntoCommand<R>) -> Result<Command> {
        let cmd = call.into_command(&self.inner);
        let Some((service, var)) = self.token_env else {
            return Ok(cmd);
        };
        match self
            .resolve_credential(service, self.expected_host.as_deref())
            .await?
        {
            Some(cred) => Ok(cmd.env(var, cred.secret().expose())),
            None => Ok(cmd),
        }
    }

    /// Apply a default timeout to every command this client builds.
    pub fn default_timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.default_timeout(timeout);
        self
    }

    /// Set the resettable output-inactivity window for streamed runs.
    ///
    /// The default is disabled (`None`), preserving the pre-watchdog behaviour.
    /// The window is applied only by [`run_with_progress`](Self::run_with_progress)
    /// and its per-call budgeted sibling; captured commands keep their existing
    /// absolute-timeout, retry, credential, and cleanup semantics. A successful
    /// read from either output stream resets the window. For `jj` callers this is
    /// an explicit opt-in: unlike Git's `--progress`, jj does not force progress
    /// when stderr is piped, so a configured window is safe only when the selected
    /// jj version/environment emits progress under that transport.
    pub fn default_inactivity_timeout(mut self, timeout: Duration) -> Self {
        self.inactivity_timeout = Some(timeout);
        self
    }

    /// Set an environment variable on every command this client builds.
    pub fn default_env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.inner = self.inner.default_env(key, value);
        self
    }

    /// Remove an inherited environment variable on every command this client builds.
    pub fn default_env_remove(mut self, key: impl AsRef<OsStr>) -> Self {
        self.inner = self.inner.default_env_remove(key);
        self
    }

    /// Cancel every command this client builds when `token` fires — and cut a
    /// lock-contention retry backoff short the moment it does, so a cancelled
    /// operation returns promptly instead of sleeping out the remaining delay
    /// before its next attempt. Network `fetch`/`push`/`clone` commands add the
    /// shared [`FETCH_TIMEOUT_GRACE`] soft-completion window (and the Windows
    /// console trigger when available); their structured result remains
    /// [`ErrorReason::Cancelled`]. The token is applied to the spawned process
    /// (via `inner`) *and* observed by the retry loop. Other commands retain their
    /// existing cancellation policy.
    pub fn default_cancel_on(mut self, token: CancellationToken) -> Self {
        self.inner = self.inner.default_cancel_on(token.clone());
        self.cancel = Some(token);
        self
    }

    /// Set the default [`OutputBudget`] applied to the content verbs this client
    /// builds through [`run_untrimmed`](Self::run_untrimmed), to a discard verb's
    /// diagnostics via [`budget_diagnostics`](Self::budget_diagnostics), and to
    /// the output a streamed run retains
    /// ([`run_with_progress`](Self::run_with_progress)) — off by default
    /// ([`OutputBudget::unlimited`]). A single call can override it via
    /// [`run_untrimmed_within`](Self::run_untrimmed_within) /
    /// [`run_with_progress_within`](Self::run_with_progress_within).
    pub fn default_output_budget(mut self, budget: OutputBudget) -> Self {
        self.output_budget = budget;
        self
    }

    /// The active default output budget.
    pub fn output_budget(&self) -> OutputBudget {
        self.output_budget
    }

    /// Apply this client's default budget to `cmd` as a **diagnostic** (drop-oldest
    /// tail) bound, for a discard verb that only surfaces its output on failure
    /// (`clone`/`fetch`). Caps the retained error/progress buffer without turning a
    /// real failure into [`ErrorReason::OutputTooLarge`] — the tail (where a CLI's fatal
    /// line sits) is preserved, so [`is_transient_fetch_error`] /
    /// [`is_lock_contention`] still classify it. A no-op when the budget is
    /// [`unlimited`](OutputBudget::unlimited).
    ///
    /// This bounds what **processkit** retains for a captured run. The streamed
    /// twin of the same verbs bounds what the *event* consumer retains instead,
    /// under the same budget — see
    /// [`run_with_progress`](Self::run_with_progress); a streaming `clone`/`fetch`
    /// wants both, since a command buffer policy does not bound an event stream.
    pub fn budget_diagnostics(&self, cmd: Command) -> Command {
        match self.output_budget.diagnostic_policy() {
            Some(policy) => cmd.output_buffer(policy),
            None => cmd,
        }
    }

    /// Build a [`Command`] for this client's program (passthrough).
    pub fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.inner.command(args)
    }

    /// Build a [`Command`] bound to `dir` (passthrough).
    pub fn command_in<I, S>(&self, dir: &Path, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.inner.command_in(dir, args)
    }

    /// The underlying process runner (passthrough — e.g. for `output_all`).
    pub fn runner(&self) -> &R {
        self.inner.runner()
    }

    /// Like [`CliClient::run`], with credential injection and lock-retry.
    pub async fn run(&self, call: impl IntoCommand<R>) -> Result<String> {
        let cmd = self.prepare(call).await?;
        retry_async(
            &self.retry,
            self.cancel.as_ref(),
            is_lock_contention,
            || self.inner.run(cmd.clone()),
        )
        .await
    }

    /// Like [`CliClient::run_unit`], with credential injection and lock-retry.
    pub async fn run_unit(&self, call: impl IntoCommand<R>) -> Result<()> {
        let cmd = self.prepare(call).await?;
        retry_async(
            &self.retry,
            self.cancel.as_ref(),
            is_lock_contention,
            || self.inner.run_unit(cmd.clone()),
        )
        .await
    }

    /// Run one command while forwarding live process events to `progress`.
    ///
    /// Credential injection and all command defaults are applied exactly as for
    /// [`run_unit`](Self::run_unit). Unlike that captured-output path, this is a
    /// deliberately single-attempt lifecycle: neither this client's optional
    /// lock retry nor a command retry is applied, so the callback observes one
    /// `Started … Exited` sequence and `Exited` remains terminal. The returned
    /// error still carries the streamed stdout/stderr for normal processkit
    /// classification.
    ///
    /// **Output budget:** this client's default [`OutputBudget`]
    /// ([`default_output_budget`](Self::default_output_budget)) bounds the
    /// stdout/stderr this call retains, as a **drop-oldest tail** — the
    /// streaming counterpart of what
    /// [`budget_diagnostics`](Self::budget_diagnostics) applies to a captured
    /// `clone`/`fetch`, and the reason a streamed one is memory-bounded by the
    /// same knob rather than growing with the repository. Never fail-loud: a
    /// bounded run still surfaces its real outcome, with the classifiable tail
    /// of its output. Unlimited by default (unchanged behaviour); override for
    /// one call with
    /// [`run_with_progress_within`](Self::run_with_progress_within), whose docs
    /// (and [`crate::run_with_progress_within`]'s) define what the ceiling counts.
    pub async fn run_with_progress(
        &self,
        call: impl IntoCommand<R>,
        progress: &mut ProgressCallback<'_>,
    ) -> Result<()> {
        self.run_with_progress_within(call, progress, self.output_budget)
            .await
    }

    /// Like [`run_with_progress`](Self::run_with_progress), but with an explicit
    /// per-call [`OutputBudget`] instead of this client's default — the
    /// streaming sibling of
    /// [`run_untrimmed_within`](Self::run_untrimmed_within), for a call that
    /// wants a tighter tail than the client's default, or
    /// [`OutputBudget::unlimited`] to keep the whole stream for one operation.
    /// See [`crate::run_with_progress_within`] for the drop-oldest semantics and
    /// the unit the byte ceiling counts.
    pub async fn run_with_progress_within(
        &self,
        call: impl IntoCommand<R>,
        progress: &mut ProgressCallback<'_>,
        budget: OutputBudget,
    ) -> Result<()> {
        let mut cmd = self.prepare(call).await?;
        if cmd.configured_inactivity_timeout().is_none() {
            if let Some(timeout) = self.inactivity_timeout {
                cmd = cmd.inactivity_timeout(timeout);
            }
        }
        crate::run_with_progress_within(self.inner.runner(), &cmd, progress, budget).await
    }

    /// Like [`CliClient::output_string`], with credential injection. **No lock-retry:**
    /// `output_string` returns `Ok` on a non-zero exit (it captures the result), so a
    /// lock failure surfaces as an `Ok` here, not an `Err` the retry predicate could
    /// match — route mutations that need lock-retry through
    /// [`run`](Self::run)/[`run_unit`](Self::run_unit) instead.
    pub async fn output_string(&self, call: impl IntoCommand<R>) -> Result<ProcessResult<String>> {
        let cmd = self.prepare(call).await?;
        self.inner.output_string(cmd).await
    }

    /// Like [`CliClient::output_bytes`], with credential injection. Captures stdout
    /// as **raw bytes**, byte-exact — unlike [`output_string`](Self::output_string),
    /// which reassembles stdout from decoded lines and so drops a trailing newline.
    /// This is the byte-faithful path [`run_untrimmed`](Self::run_untrimmed) needs.
    /// **No lock-retry**, for the same reason as `output_string`: it returns `Ok`
    /// on a non-zero exit (it captures the result), so a lock failure surfaces as an
    /// `Ok` here rather than an `Err` the retry predicate could match.
    pub async fn output_bytes(&self, call: impl IntoCommand<R>) -> Result<ProcessResult<Vec<u8>>> {
        let cmd = self.prepare(call).await?;
        self.inner.output_bytes(cmd).await
    }

    /// Like [`run`](Self::run), but returns stdout **verbatim** — no `trim_end`.
    /// For **content**-returning verbs (a file's bytes at a rev, a diff, a raw
    /// template render) where the trailing newline(s) are part of the value, not
    /// noise: trimming them corrupts a read-modify-write round-trip and desyncs a
    /// diff's last hunk from its `@@` line count. Exit-checked like `run`; no
    /// lock-retry (a content read is not a mutation).
    ///
    /// Routed through [`output_bytes`](Self::output_bytes) (raw stdout), not
    /// `output_string`, so the exact bytes — trailing newline included — survive:
    /// `output_string` rebuilds stdout from decoded lines and would drop that final
    /// `\n`. The raw bytes are then decoded with
    /// [`String::from_utf8_lossy`], the same lossy raw-stdout-to-`String` convention
    /// used elsewhere in this workspace (e.g. `vcs-jj`).
    ///
    /// **Output budget:** this client's default [`OutputBudget`]
    /// ([`default_output_budget`](Self::default_output_budget)) is applied as a
    /// fail-loud byte ceiling — a content read past the cap errors with
    /// [`ErrorReason::OutputTooLarge`] (carrying the actual and allowed sizes) instead of
    /// buffering an unbounded blob, and a truncated read is never returned as if
    /// complete. Unlimited by default (unchanged behaviour). Override the ceiling
    /// for one call with [`run_untrimmed_within`](Self::run_untrimmed_within).
    ///
    /// The ceiling rides **both** captured streams independently: the raw stdout
    /// this verb returns, counted verbatim, and the command's line-pumped stderr,
    /// counted as raw pipe bytes (terminators included) since processkit 3.0 — so
    /// a command that floods stderr past the cap fails loud here too. See
    /// [`OutputBudget::bytes`] for the per-stream unit.
    pub async fn run_untrimmed(&self, call: impl IntoCommand<R>) -> Result<String> {
        self.run_untrimmed_within(call, self.output_budget).await
    }

    /// Like [`run_untrimmed`](Self::run_untrimmed), but with an explicit per-call
    /// [`OutputBudget`] instead of this client's default — the per-call override
    /// used by the `*_within` content methods (`diff_text_within`,
    /// `show_file_within`, `pr_diff_within`, …) to read a legitimately large
    /// file/diff (a higher ceiling, or [`OutputBudget::unlimited`]) or to tighten
    /// the cap for one call.
    pub async fn run_untrimmed_within(
        &self,
        call: impl IntoCommand<R>,
        budget: OutputBudget,
    ) -> Result<String> {
        let cmd = self.prepare(call).await?;
        // A fail-loud byte ceiling: `output_bytes` raises `ErrorReason::OutputTooLarge`
        // the moment the raw stdout passes the cap (drained but not retained), so
        // this never returns a truncated blob as if it were complete.
        let cmd = match budget.content_policy() {
            Some(policy) => cmd.output_buffer(policy),
            None => cmd,
        };
        let bytes = self
            .inner
            .output_bytes(cmd)
            .await?
            .ensure_success()?
            .into_stdout();
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Like [`CliClient::probe`] (zero-or-nonzero exit → `bool`), with credential
    /// injection and lock-retry.
    pub async fn probe(&self, call: impl IntoCommand<R>) -> Result<bool> {
        let cmd = self.prepare(call).await?;
        retry_async(
            &self.retry,
            self.cancel.as_ref(),
            is_lock_contention,
            || self.inner.probe(cmd.clone()),
        )
        .await
    }

    /// Like [`CliClient::exit_code`] (the raw exit code; a spawn failure or timeout
    /// still errors), with credential injection and lock-retry.
    pub async fn exit_code(&self, call: impl IntoCommand<R>) -> Result<i32> {
        let cmd = self.prepare(call).await?;
        retry_async(
            &self.retry,
            self.cancel.as_ref(),
            is_lock_contention,
            || self.inner.exit_code(cmd.clone()),
        )
        .await
    }

    /// Like [`CliClient::parse`] (credential injection applied; the `FnOnce` parser
    /// can't be re-run, so lock-retry does not — parsing is a read, where lock
    /// contention is not a concern anyway).
    pub async fn parse<T>(
        &self,
        call: impl IntoCommand<R>,
        parser: impl FnOnce(&str) -> T + Send,
    ) -> Result<T>
    where
        T: Send,
    {
        let cmd = self.prepare(call).await?;
        self.inner.parse(cmd, parser).await
    }

    /// Like [`parse`](Self::parse), but hands the parser **raw stdout bytes**
    /// instead of a lossily-decoded `&str`. This is the byte-faithful path a parser
    /// needs when a **path** (or any payload that need not be valid UTF-8) is part
    /// of the output: on Unix a filename can be arbitrary bytes, so decoding it
    /// through [`String::from_utf8_lossy`] first would substitute `U+FFFD` and make
    /// the path unusable to round-trip back into `add`/`commit_paths`. Routed
    /// through [`output_bytes`](Self::output_bytes) (byte-exact stdout) and
    /// exit-checked like [`parse`](Self::parse) (`ensure_success`); no lock-retry (a
    /// read). Text-only machine output (branch names, hashes, templated rows) should
    /// keep using [`parse`](Self::parse) — lossy decoding is acceptable there.
    pub async fn parse_bytes<T>(
        &self,
        call: impl IntoCommand<R>,
        parser: impl FnOnce(&[u8]) -> T + Send,
    ) -> Result<T>
    where
        T: Send,
    {
        let cmd = self.prepare(call).await?;
        let bytes = self
            .inner
            .output_bytes(cmd)
            .await?
            .ensure_success()?
            .into_stdout();
        Ok(parser(&bytes))
    }

    /// Like [`CliClient::try_parse`] (credential injection applied; `FnOnce` parser,
    /// and a read, so no lock-retry).
    pub async fn try_parse<T>(
        &self,
        call: impl IntoCommand<R>,
        parser: impl FnOnce(&str) -> Result<T> + Send,
    ) -> Result<T>
    where
        T: Send,
    {
        let cmd = self.prepare(call).await?;
        self.inner.try_parse(cmd, parser).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::testing::{Reply, ScriptedRunner};
    use proptest::prelude::*;
    use std::sync::Mutex;

    #[tokio::test]
    async fn streamed_run_replays_scripted_lifecycle_and_preserves_failure_output() {
        let runner = ScriptedRunner::new().on(
            ["tool", "network-op"],
            Reply::fail(23, "remote rejected").with_stdout("sent objects"),
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let mut progress = move |event| sink.lock().unwrap().push(event);

        let err = run_with_progress(
            &runner,
            &Command::new("tool").arg("network-op"),
            &mut progress,
        )
        .await
        .expect_err("the scripted non-zero exit stays an error");
        drop(progress);

        assert!(matches!(
            err.reason(),
            ErrorReason::Exit { code: 23, stdout, stderr, .. }
                if stdout == "sent objects" && stderr == "remote rejected"
        ));
        let events = seen.lock().unwrap();
        assert!(matches!(events.first(), Some(ProcessEvent::Started { .. })));
        assert!(
            events
                .iter()
                .any(|event| event.text() == Some("sent objects"))
        );
        assert!(
            events
                .iter()
                .any(|event| event.text() == Some("remote rejected"))
        );
        assert!(matches!(
            events.last(),
            Some(ProcessEvent::Exited(outcome)) if outcome.code() == Some(23)
        ));
    }

    #[test]
    fn fetch_completion_policy_records_grace_and_platform_soft_trigger() {
        let command = apply_fetch_completion_policy(Command::new("git").arg("fetch"));
        let debug = format!("{command:?}");

        assert!(debug.contains("timeout_grace: Some(2s)"), "{debug}");
        assert!(debug.contains("cancel_grace: Some(2s)"), "{debug}");
        #[cfg(windows)]
        assert!(
            debug.contains("windows_graceful_ctrl_break: true"),
            "Windows network commands must opt into CTRL_BREAK: {debug}"
        );
        #[cfg(not(windows))]
        assert!(
            debug.contains("windows_graceful_ctrl_break: false"),
            "the Windows-only trigger must remain a Unix no-op: {debug}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn managed_client_stream_watchdog_is_disabled_by_default() {
        let client = ManagedClient::with_runner(
            "tool",
            ScriptedRunner::new().on(
                ["tool", "network-op"],
                Reply::lines(["late"]).with_line_delay(Duration::from_secs(10)),
            ),
        );
        assert!(
            client
                .command(["network-op"])
                .configured_inactivity_timeout()
                .is_none(),
            "the new watchdog must not alter an unconfigured command"
        );

        let mut progress = |_event: ProcessEvent| {};
        client
            .run_with_progress(client.command(["network-op"]), &mut progress)
            .await
            .expect("the default-disabled stream remains compatible with slow output");
    }

    #[tokio::test(start_paused = true)]
    async fn managed_client_stream_watchdog_distinguishes_inactivity_from_deadline() {
        let delayed = || {
            ScriptedRunner::new().on(
                ["tool", "network-op"],
                Reply::lines(["late"]).with_line_delay(Duration::from_secs(10)),
            )
        };

        let watchdog = ManagedClient::with_runner("tool", delayed())
            .default_inactivity_timeout(Duration::from_secs(3));
        let mut progress = |_event: ProcessEvent| {};
        let err = watchdog
            .run_with_progress(watchdog.command(["network-op"]), &mut progress)
            .await
            .expect_err("the inactivity watchdog must fire before the delayed line");
        assert!(matches!(
            err.reason(),
            ErrorReason::Timeout {
                timeout,
                inactivity: true,
                ..
            } if *timeout == Duration::from_secs(3)
        ));

        let deadline = ManagedClient::with_runner("tool", delayed())
            .default_timeout(Duration::from_secs(3))
            .default_inactivity_timeout(Duration::from_secs(30));
        let err = deadline
            .run_with_progress(deadline.command(["network-op"]), &mut progress)
            .await
            .expect_err("the absolute deadline must remain a distinct outcome");
        assert!(matches!(
            err.reason(),
            ErrorReason::Timeout {
                timeout,
                inactivity: false,
                ..
            } if *timeout == Duration::from_secs(3)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn managed_client_stream_watchdog_resets_for_legitimately_slow_output() {
        let client = ManagedClient::with_runner(
            "tool",
            ScriptedRunner::new().on(
                ["tool", "network-op"],
                Reply::lines(["one", "two", "three"]).with_line_delay(Duration::from_secs(2)),
            ),
        )
        .default_inactivity_timeout(Duration::from_secs(3));
        let mut progress = |_event: ProcessEvent| {};

        client
            .run_with_progress(client.command(["network-op"]), &mut progress)
            .await
            .expect("each output line resets the watchdog");
    }

    #[tokio::test(start_paused = true)]
    async fn managed_client_stream_watchdog_preserves_explicit_command_timeout() {
        let client = ManagedClient::with_runner(
            "tool",
            ScriptedRunner::new().on(
                ["tool", "network-op"],
                Reply::lines(["late"]).with_line_delay(Duration::from_secs(10)),
            ),
        )
        .default_inactivity_timeout(Duration::from_secs(3));
        let command = client
            .command(["network-op"])
            .inactivity_timeout(Duration::from_secs(30));
        assert_eq!(
            command.configured_inactivity_timeout(),
            Some(Duration::from_secs(30))
        );

        let mut progress = |_event: ProcessEvent| {};
        client
            .run_with_progress(command, &mut progress)
            .await
            .expect("the client default must not override a per-command watchdog");
    }

    // Processkit 3.3 makes the late-cancellation boundary explicit: a token is
    // judged at the first exit observation, not at the instant the child died. The
    // wrapper's real streamed path is exactly `events()` joined with `finish()`;
    // keep this case here because a normal `run_with_progress_within` call has no
    // hook between `start()` and that join. A scripted reply has already exited by
    // the time `start()` returns, so firing the token before the joined finisher's
    // first observation must still produce `Cancelled`.
    #[tokio::test]
    async fn streamed_run_late_cancel_before_first_finish_observation_is_cancelled() {
        let token = CancellationToken::new();
        let runner =
            ScriptedRunner::new().on(["tool", "network-op"], Reply::ok("").with_stdout("done\n"));
        let mut run = runner
            .start(
                &Command::new("tool")
                    .arg("network-op")
                    .cancel_on(token.clone()),
            )
            .await
            .expect("scripted start");
        let mut events = run.events().expect("events stream");

        // The scripted child is already exited, but `events()` is not the exit
        // observer. This is the late-token/first-finisher boundary under test.
        token.cancel();
        let forward = async { while events.next().await.is_some() {} };
        let (_, finished) = tokio::join!(forward, run.finish());

        assert!(
            matches!(
                finished.as_ref().map_err(Error::reason),
                Err(ErrorReason::Cancelled { program }) if program == "tool"
            ),
            "first finish observation must classify the fired token: {finished:?}"
        );
    }

    #[tokio::test]
    async fn streamed_run_isolates_a_panicking_progress_callback() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let runner = ScriptedRunner::new().on(
            ["tool", "network-op"],
            Reply::ok("out").with_stderr("progress"),
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let mut progress = move |_event| {
            seen.fetch_add(1, Ordering::SeqCst);
            panic!("broken UI callback");
        };

        run_with_progress(
            &runner,
            &Command::new("tool").arg("network-op"),
            &mut progress,
        )
        .await
        .expect("the process outcome wins over a callback panic");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "callback is disabled");
    }

    #[test]
    fn rejects_empty_and_leading_dash() {
        assert!(reject_flag_like("git", "branch name", "-evil").is_err());
        assert!(reject_flag_like("git", "branch name", "").is_err());
        // Whitespace-only is as meaning-changing as empty — refuse it too.
        assert!(reject_flag_like("git", "branch name", "  ").is_err());
        assert!(reject_flag_like("git", "branch name", "\t").is_err());
        assert!(reject_flag_like("git", "branch name", "feature").is_ok());
        // Leading whitespace before a dash is still refused (the flag-check trims).
        assert!(reject_flag_like("git", "remote", " --upload-pack=evil").is_err());
        assert!(reject_flag_like("git", "remote", "\t-x").is_err());
        // An interior NUL is refused (can't go in argv; opaque OS error otherwise).
        assert!(reject_flag_like("git", "path", "a\0b").is_err());
        // A leading-whitespace non-flag value is still accepted (not flag-like).
        assert!(reject_flag_like("git", "branch name", "  feature").is_ok());
        // The error names the program and surfaces as a spawn-side refusal.
        let err = reject_flag_like("jj", "revset", "--remote").unwrap_err();
        assert!(matches!(err.reason(), ErrorReason::Spawn { program, .. } if program == "jj"));
    }

    #[test]
    fn classifies_merge_conflict() {
        let on_stdout = Error::exit("git", 1, "CONFLICT (content): Merge conflict in a.rs", "");
        let on_stderr = Error::exit(
            "git",
            1,
            "",
            "Automatic merge failed; fix conflicts and then commit",
        );
        let unrelated = Error::exit("git", 128, "", "fatal: not a git repository");
        assert!(is_merge_conflict(&on_stdout));
        assert!(is_merge_conflict(&on_stderr));
        assert!(!is_merge_conflict(&unrelated));
        assert!(!is_nothing_to_commit(&on_stdout));
    }

    #[test]
    fn classifies_nothing_to_commit_and_transient_fetch() {
        let nothing = Error::exit("git", 1, "nothing to commit, working tree clean", "");
        assert!(is_nothing_to_commit(&nothing));

        let dns = Error::exit(
            "git",
            128,
            "",
            "fatal: unable to access 'https://x/': Could not resolve host: x",
        );
        assert!(is_transient_fetch_error(&dns));
        assert!(!is_transient_fetch_error(&nothing));

        // A processkit timeout is deliberately NOT retried (R6): it already consumed
        // the caller's full deadline, so retrying would multiply the wall-clock by
        // FETCH_ATTEMPTS. The deadline is the patience budget; raise it, don't triple it.
        let timeout = Error::timeout("git", Duration::from_secs(10), "", "");
        assert!(!is_transient_fetch_error(&timeout));
    }

    // R9: an io-level transient from the spawn (EINTR / EAGAIN / busy) is fetch-
    // retryable too, via processkit's `Error::is_transient()`.
    #[test]
    fn classifies_io_transient_as_fetch_retryable() {
        let interrupted =
            Error::spawn("git", std::io::Error::from(std::io::ErrorKind::Interrupted));
        assert!(
            interrupted.is_transient(),
            "processkit treats Interrupted as a transient io error"
        );
        assert!(is_transient_fetch_error(&interrupted));
        // A non-transient io error (e.g. NotFound — the binary is missing) is not retried.
        let missing = Error::spawn("git", std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(!is_transient_fetch_error(&missing));
    }

    // R2: regression for the processkit 0.9.1 untruncated-`ErrorReason::Exit` fix. A large
    // output (well past the old 4 KiB cap) with the decisive marker near the END must
    // still classify — proving the classifiers see the whole captured stream.
    #[test]
    fn classifies_on_large_output_past_the_old_4kib_cap() {
        let padding = "noise line that says nothing\n".repeat(500); // ~14 KiB
        let conflict = Error::exit(
            "git",
            1,
            format!("{padding}CONFLICT (content): Merge conflict in late.rs"),
            "",
        );
        assert!(
            is_merge_conflict(&conflict),
            "a conflict marker past 4 KiB must still classify"
        );

        let transient = Error::exit(
            "git",
            128,
            "",
            format!("{padding}fatal: unable to access: Could not resolve host: x"),
        );
        assert!(is_transient_fetch_error(&transient));
    }

    // processkit's `ErrorReason` is `#[non_exhaustive]` and grows variants over time
    // (`NotReady`/`Unsupported`/`CassetteMiss`/`NotFound`/`Signalled`/`Cancelled`/
    // `ResourceLimit`). Unfamiliar variants must fall through every classifier to
    // "no" — a not-ready or unsupported run is neither a conflict, nor a clean
    // tree, nor worth a fetch retry.
    #[test]
    fn unfamiliar_error_variants_are_not_classified() {
        let not_ready = Error::from(ErrorReason::NotReady {
            program: "git".into(),
            timeout: Duration::from_secs(5),
        });
        let unsupported = Error::from(ErrorReason::Unsupported {
            operation: "suspend".into(),
        });
        for err in [&not_ready, &unsupported] {
            assert!(!is_merge_conflict(err));
            assert!(!is_nothing_to_commit(err));
            assert!(!is_transient_fetch_error(err));
        }
    }

    // `ErrorReason::Cancelled` (a client-level `default_cancel_on` killing an
    // in-flight run; always available since cancellation became core in processkit
    // 0.10) must fall through every classifier to "no" — a cancelled fetch was
    // *deliberately* stopped, so replaying it would fight the cancellation. (Behaviour
    // already held via the `#[non_exhaustive]` fall-through above; this pins it as a
    // first-class assertion.)
    #[test]
    fn cancelled_is_not_transient_or_otherwise_classified() {
        let cancelled = Error::from(ErrorReason::Cancelled {
            program: "git".into(),
        });
        assert!(!is_transient_fetch_error(&cancelled));
        assert!(!is_merge_conflict(&cancelled));
        assert!(!is_nothing_to_commit(&cancelled));
    }

    // `ErrorReason::Signalled` (a process killed by a signal — e.g. an external SIGTERM/
    // SIGKILL, surfaced first-class since processkit 0.9.2 and carrying partial
    // `stdout`/`stderr` since 0.10) is *terminal*, not transient: a deliberate kill
    // should not be auto-retried, and a signal death is neither a merge conflict nor
    // a clean tree. processkit's own `is_transient()` agrees (false for `Signalled`),
    // so it falls through every classifier to "no" — pinned here, including the case
    // where the captured stderr happens to contain an otherwise-transient marker (a
    // killed fetch is still not ours to silently replay).
    #[test]
    fn signalled_is_terminal_not_transient() {
        let signalled = Error::signalled(
            "git",
            Some(15),
            "",
            "fatal: unable to access: Could not resolve host: x",
        );
        assert!(!signalled.is_transient());
        assert!(!is_transient_fetch_error(&signalled));
        assert!(!is_merge_conflict(&signalled));
        assert!(!is_nothing_to_commit(&signalled));
    }

    fn exit(program: &str, code: i32, stderr: &str) -> Error {
        Error::exit(program, code, "", stderr)
    }

    // `is_lock_contention` recognises ONLY the *whole-repo* / working-copy lock
    // failures (git index.lock, jj working-copy/op-heads lock) — the ones where the
    // command did nothing, so a retry is idempotent even on a mutation. Per-ref lock
    // failures and conflicts/timeouts are deliberately NOT classified (a multi-ref
    // op can fail a ref lock mid-way, where a retry would not be idempotent).
    #[test]
    fn classifies_lock_contention() {
        let lock_failures = [
            // git always names `index.lock` (locale-stable) in the lock-contention
            // message, even on a non-English runner where the surrounding prose is
            // translated.
            exit(
                "git",
                128,
                "fatal: Unable to create '/r/.git/index.lock': File exists.",
            ),
            // A German runner: the path fragment `index.lock` still matches.
            exit(
                "git",
                128,
                "fatal: Konnte '/r/.git/index.lock' nicht erstellen: Datei existiert bereits",
            ),
            // jj's *actual* wordings (verified against jj source) — note no "the".
            exit("jj", 1, "Error: Failed to lock working copy"),
            exit("jj", 1, "Error: Failed to lock operation heads store"),
        ];
        for e in &lock_failures {
            assert!(is_lock_contention(e), "should be lock contention: {e:?}");
            // A lock failure is NOT a transient *fetch* error — different class.
            assert!(!is_transient_fetch_error(e), "not a fetch error: {e:?}");
        }
        let not_locks = [
            exit("git", 1, "CONFLICT (content): Merge conflict in a.rs"),
            exit("git", 1, "error: pathspec 'x' did not match any file(s)"),
            exit("git", 128, "fatal: not a git repository"),
            // Per-ref locks are NOT classified — a multi-ref push/fetch can fail a
            // ref lock after earlier refs already moved (non-idempotent to replay).
            exit(
                "git",
                1,
                "error: cannot lock ref 'refs/heads/x': reference already exists",
            ),
            exit(
                "git",
                128,
                "Unable to create '/r/.git/packed-refs.lock': File exists.",
            ),
            // A per-ref lock for a branch literally named `index`: its
            // `…/refs/heads/index.lock` path contains the substring `index.lock`,
            // but the `refs/` mention correctly rules it out (not a whole-repo lock).
            exit(
                "git",
                128,
                "error: cannot lock ref 'refs/heads/index': Unable to create \
                 '/r/.git/refs/heads/index.lock': File exists.",
            ),
            Error::timeout("git", Duration::from_secs(1), "", ""),
        ];
        for e in &not_locks {
            assert!(
                !is_lock_contention(e),
                "should NOT be lock contention: {e:?}"
            );
        }
    }

    #[test]
    fn classifies_invalid_input_from_the_guards() {
        // What `reject_flag_like` / the newtypes actually produce.
        let rejected = reject_flag_like("git", "reference", "-x").unwrap_err();
        assert!(
            is_invalid_input(&rejected),
            "guard rejection is invalid input"
        );
        assert!(is_invalid_input(
            &reject_flag_like("git", "x", "").unwrap_err()
        ));

        // A real spawn failure (missing binary), a non-zero exit, and a timeout are
        // NOT invalid input — they're environment/usage failures, not a bad argument.
        let not_input = [
            Error::spawn("git", std::io::Error::from(std::io::ErrorKind::NotFound)),
            exit("git", 1, "fatal: not a git repository"),
            Error::timeout("git", Duration::from_secs(1), "", ""),
        ];
        for e in &not_input {
            assert!(!is_invalid_input(e), "should NOT be invalid input: {e:?}");
        }
    }

    // R7 (T-085): `clone_dest_cleanable` must return `true` only when `dest`'s
    // absence/emptiness is actually proven, never on an unrelated `read_dir`
    // failure — that path used to be `Err(_) => true`, which could tell
    // `cleanup_failed_clone_dest` to `remove_dir_all` a pre-existing, non-empty
    // directory it merely failed to read (permission denied, transient I/O).
    #[test]
    fn clone_dest_cleanable_requires_proven_absence_or_emptiness() {
        use vcs_testkit::TempDir;

        // Absent (NotFound) → cleanable. `TempDir::new` creates its own dir
        // eagerly, so the proven-absent case has to be an as-yet-unjoined
        // subpath of a live `TempDir`: the join alone creates nothing, so
        // `absent` stays absent, while the parent `TempDir` still guarantees
        // cleanup (mirrors `crates/core/tests/repo.rs`'s
        // `tmp.path().join("dest") // never created` pattern).
        let absent_parent = TempDir::new("clone-dest-absent");
        let absent = absent_parent.path().join("absent");
        assert!(clone_dest_cleanable(&absent));

        // An existing, empty directory → cleanable.
        let empty = TempDir::new("clone-dest-empty");
        assert!(clone_dest_cleanable(empty.path()));

        // An existing, non-empty directory → NOT cleanable.
        let nonempty = TempDir::new("clone-dest-nonempty");
        std::fs::write(nonempty.path().join("keep.txt"), b"user data").expect("write file");
        assert!(!clone_dest_cleanable(nonempty.path()));

        // `dest` is a plain file, not a directory: `read_dir` fails with
        // `NotADirectory`/similar — NOT `NotFound` — so this must NOT be
        // classified as cleanable, even though `remove_dir_all` would in fact
        // fail harmlessly on a file. The point is the classification must not
        // rely on that coincidence.
        let file_parent = TempDir::new("clone-dest-file");
        let file = file_parent.path().join("not-a-dir");
        std::fs::write(&file, b"not a directory").expect("write file");
        let err = std::fs::read_dir(&file).expect_err("read_dir on a file fails");
        assert_ne!(
            err.kind(),
            std::io::ErrorKind::NotFound,
            "must be a genuine NotADirectory-style failure, not NotFound"
        );
        assert!(!clone_dest_cleanable(&file));

        // And `cleanup_failed_clone_dest` must leave that file untouched when
        // called with `cleanable = false`.
        cleanup_failed_clone_dest(&file, false);
        assert!(
            file.is_file(),
            "cleanup must not touch a non-cleanable dest"
        );
    }

    // Backoff is exponential off the base, capped at `max_backoff`, and zero when
    // there's no base (immediate retry).
    #[test]
    fn backoff_is_exponential_capped_and_zero_without_base() {
        let p = RetryPolicy::none()
            .attempts(6)
            .base_backoff(Duration::from_millis(10))
            .max_backoff(Duration::from_millis(80));
        assert_eq!(backoff_for(&p, 0), Duration::from_millis(10));
        assert_eq!(backoff_for(&p, 1), Duration::from_millis(20));
        assert_eq!(backoff_for(&p, 2), Duration::from_millis(40));
        assert_eq!(backoff_for(&p, 3), Duration::from_millis(80));
        assert_eq!(
            backoff_for(&p, 4),
            Duration::from_millis(80),
            "capped at max"
        );
        assert_eq!(
            backoff_for(&RetryPolicy::none(), 3),
            Duration::ZERO,
            "no base → no wait"
        );
    }

    // Full jitter (used by `RetryPolicy::lock_contention`): every sampled backoff
    // stays within `[0, exponential cap]`, and successive samples de-correlate
    // (more than one distinct value) so retries don't thunder together. Pins the
    // jitter path, which the exponential test above deliberately turns off.
    #[test]
    fn jitter_stays_within_cap_and_decorrelates() {
        let p = RetryPolicy::none()
            .attempts(8)
            .base_backoff(Duration::from_millis(10))
            .max_backoff(Duration::from_millis(80))
            .with_jitter(true);
        // The cap at retry_index 3 is the full 80ms exponential value.
        let cap = Duration::from_millis(80);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let d = backoff_for(&p, 3);
            assert!(
                d <= cap,
                "jittered backoff {d:?} must stay within the cap {cap:?}"
            );
            seen.insert(d.as_nanos());
        }
        assert!(
            seen.len() > 1,
            "full jitter must produce a spread of delays, not a constant"
        );
        // A zero base still short-circuits to zero even with jitter on.
        assert_eq!(
            backoff_for(&RetryPolicy::none().with_jitter(true), 2),
            Duration::ZERO
        );
    }

    // The executor: retries while the predicate matches and attempts remain, returns
    // the first Ok, doesn't retry a non-matching error, and exhausts to the last Err.
    #[tokio::test]
    async fn retry_async_retries_then_succeeds_and_respects_the_predicate() {
        use std::sync::atomic::{AtomicU32, Ordering};
        // Zero backoff → no sleep, deterministic & fast.
        let policy = RetryPolicy::none().attempts(4);
        let lock = || {
            exit(
                "git",
                128,
                "Unable to create '/r/.git/index.lock': File exists.",
            )
        };

        // Fails twice with a lock error, then succeeds — retried to success.
        let calls = AtomicU32::new(0);
        let out: Result<u32> = retry_async(&policy, None, is_lock_contention, || {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            let lock = lock();
            async move { if n < 2 { Err(lock) } else { Ok(n) } }
        })
        .await;
        assert_eq!(out.unwrap(), 2);
        assert_eq!(calls.load(Ordering::SeqCst), 3, "1 try + 2 retries");

        // A non-lock error is returned immediately (not retried).
        let calls = AtomicU32::new(0);
        let out: Result<u32> = retry_async(&policy, None, is_lock_contention, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(exit("git", 1, "real, deterministic failure")) }
        })
        .await;
        assert!(out.is_err());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "non-retryable → single attempt"
        );

        // Persistent lock contention exhausts the attempt budget.
        let calls = AtomicU32::new(0);
        let out: Result<u32> = retry_async(&policy, None, is_lock_contention, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(exit("git", 128, "index.lock': File exists")) }
        })
        .await;
        assert!(out.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 4, "all attempts used");
    }

    // A persistent lock error always retryable, for the cancellation tests below.
    fn lock_err() -> Error {
        exit(
            "git",
            128,
            "Unable to create '/r/.git/index.lock': File exists.",
        )
    }

    /// The [`ErrorReason`] behind a failed result, so the assertions below can match
    /// a variant on the now-opaque [`Error`] wrapper without unwrapping by hand.
    fn err_reason<T>(out: &Result<T>) -> Option<&ErrorReason> {
        out.as_ref().err().map(Error::reason)
    }

    // Cancellation scenario 1 — the token is **already fired** when the backoff is
    // about to begin: `retry_async` must not sleep out the (long) delay, and must
    // abort with a structured `Cancelled` after the single attempt that already ran,
    // launching no second one. On a paused clock the virtual time must not advance —
    // proving the full backoff was skipped, not merely fast.
    #[tokio::test(start_paused = true)]
    async fn cancel_before_backoff_aborts_without_waiting_or_retrying() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let token = CancellationToken::new();
        token.cancel(); // already cancelled before we even start
        let policy = RetryPolicy::none()
            .attempts(5)
            .base_backoff(Duration::from_secs(3600)); // huge — must never be waited
        let calls = AtomicU32::new(0);

        let start = tokio::time::Instant::now();
        let out: Result<u32> = retry_async(&policy, Some(&token), is_lock_contention, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(lock_err()) }
        })
        .await;

        assert!(
            matches!(err_reason(&out), Some(ErrorReason::Cancelled { program }) if program == "git"),
            "a fired token aborts with a program-named Cancelled, got {out:?}"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "one attempt ran; the cancel launched no retry"
        );
        assert_eq!(
            start.elapsed(),
            Duration::ZERO,
            "the backoff was cut short — no virtual time elapsed"
        );
    }

    // Cancellation scenario 2 — the token fires **while the backoff sleep is
    // parked**. With a paused clock the (long) sleep cannot elapse on its own, so a
    // spawned task cancelling the token is what resolves the wait: the retry must
    // wake early and return `Cancelled` without a second attempt.
    #[tokio::test(start_paused = true)]
    async fn cancel_during_backoff_wakes_early_and_does_not_retry() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let token = CancellationToken::new();
        let policy = RetryPolicy::none()
            .attempts(5)
            .base_backoff(Duration::from_secs(3600)); // never elapses under paused time
        let calls = AtomicU32::new(0);

        let start = tokio::time::Instant::now();
        let out: Result<u32> = retry_async(&policy, Some(&token), is_lock_contention, || {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            let token = token.clone();
            async move {
                // On the first failure, schedule the cancel to land while we are
                // parked in the backoff sleep (the sleep can't fire under paused time,
                // so this is what unblocks the wait).
                if n == 0 {
                    tokio::spawn(async move { token.cancel() });
                }
                Err(lock_err())
            }
        })
        .await;

        assert!(
            matches!(err_reason(&out), Some(ErrorReason::Cancelled { program }) if program == "git"),
            "a cancel during the sleep aborts with Cancelled, got {out:?}"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "cancel woke the sleep early — no second attempt"
        );
        assert_eq!(
            start.elapsed(),
            Duration::ZERO,
            "woke on the cancel, not after the 1 h delay"
        );
    }

    // Cancellation scenario 3 — the token fires such that it is observed **right
    // before the next attempt** would launch. With a zero backoff there is no sleep
    // to interrupt, so the op cancels the token as it fails; the guard between the
    // (no-op) backoff and the next attempt must still abort with `Cancelled` rather
    // than spinning up attempt #2.
    #[tokio::test(start_paused = true)]
    async fn cancel_right_before_next_attempt_aborts() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let token = CancellationToken::new();
        let policy = RetryPolicy::none().attempts(5); // zero backoff → no sleep
        let calls = AtomicU32::new(0);

        let out: Result<u32> = retry_async(&policy, Some(&token), is_lock_contention, || {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            let token = token.clone();
            async move {
                // Cancel as the first attempt fails: the post-backoff guard must catch
                // it before launching the next attempt.
                if n == 0 {
                    token.cancel();
                }
                Err(lock_err())
            }
        })
        .await;

        assert!(
            matches!(err_reason(&out), Some(ErrorReason::Cancelled { program }) if program == "git"),
            "a cancel observed before the next attempt aborts with Cancelled, got {out:?}"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the guard stopped attempt #2 from launching"
        );
    }

    // Without a token the backoff is unchanged: a persistent lock error still
    // exhausts every attempt (no early exit, `None` path preserved).
    #[tokio::test]
    async fn no_token_backoff_is_unchanged() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let policy = RetryPolicy::none().attempts(3); // zero backoff, fast
        let calls = AtomicU32::new(0);
        let out: Result<u32> = retry_async(&policy, None, is_lock_contention, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err(lock_err()) }
        })
        .await;
        assert!(
            matches!(err_reason(&out), Some(ErrorReason::Exit { .. })),
            "last error is the lock exit, not Cancelled"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "all attempts used with no token"
        );
    }

    // `resolve_credential` returns `None` until a provider is attached, then the
    // provider's credential. (No process is spawned, so the real runner is fine.)
    #[tokio::test]
    async fn retrying_client_resolves_credential_opt_in() {
        let client = ManagedClient::new("git");
        assert!(!client.has_credentials());
        assert!(
            client
                .resolve_credential(CredentialService::Git, None)
                .await
                .unwrap()
                .is_none(),
            "no provider → ambient (None)"
        );

        let client = client.with_credentials(Arc::new(StaticCredential::token("t0k")));
        assert!(client.has_credentials());
        let got = client
            .resolve_credential(CredentialService::Git, None)
            .await
            .unwrap()
            .expect("provider yields a credential");
        assert_eq!(got.secret().expose(), "t0k");
    }

    // An empty (or whitespace-only) secret is treated as `None` (ambient):
    // injecting an empty token would override the ambient login with nothing
    // instead of deferring to it. Mirrors `EnvToken`'s whitespace-only ⇒ unset rule.
    #[tokio::test]
    async fn resolve_credential_treats_empty_secret_as_ambient() {
        // Service-agnostic: both the forge (token-env) and git (helper) paths route
        // through this chokepoint, so a blank secret is ambient for either.
        for blank in ["", "   ", "\t\n"] {
            let client = ManagedClient::new("git")
                .with_credentials(Arc::new(StaticCredential::token(blank)));
            for service in [CredentialService::GitHub, CredentialService::Git] {
                assert!(
                    client
                        .resolve_credential(service, None)
                        .await
                        .unwrap()
                        .is_none(),
                    "blank secret {blank:?} → ambient (None) for {service:?}"
                );
            }
        }
    }

    // The resolved request carries the operation's host, so a HOST-KEYED provider
    // returns the secret for exactly that host — and `Ok(None)` (deferring to
    // ambient) for a host it does not place or an absent one, never a wrong-host
    // secret. This is the seam `prepare` (forge token-env) and git's
    // `remote_credentials` both feed the target host into. (T-045)
    #[tokio::test]
    async fn resolve_credential_routes_on_request_host() {
        let provider = provider_fn(|r: &CredentialRequest<'_>| {
            Ok(match r.host {
                Some("github.com") => Some(Credential::token("saas")),
                Some("ghe.example.com") => Some(Credential::token("ent")),
                // An unknown or absent host defers to ambient rather than a default.
                _ => None,
            })
        });
        let client = ManagedClient::new("gh").with_credentials(Arc::new(provider));
        let resolve =
            |host: Option<&'static str>| client.resolve_credential(CredentialService::GitHub, host);

        assert_eq!(
            resolve(Some("github.com"))
                .await
                .unwrap()
                .unwrap()
                .secret()
                .expose(),
            "saas"
        );
        assert_eq!(
            resolve(Some("ghe.example.com"))
                .await
                .unwrap()
                .unwrap()
                .secret()
                .expose(),
            "ent"
        );
        assert!(
            resolve(Some("other.example")).await.unwrap().is_none(),
            "a host the provider doesn't place → ambient (None), not a wrong secret"
        );
        assert!(
            resolve(None).await.unwrap().is_none(),
            "an absent host → ambient (None)"
        );
    }

    // Fail-closed: a provider `Err` propagates out of `resolve_credential` (and so
    // aborts the command in `prepare` / `remote_credentials`) for any host — it is
    // never swallowed into a silent ambient fallback. (T-045 fallback policy)
    #[tokio::test]
    async fn resolve_credential_propagates_provider_error_fail_closed() {
        let provider = provider_fn(|_r: &CredentialRequest<'_>| {
            Err(Error::spawn(
                "vault",
                std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "vault unreachable"),
            ))
        });
        let client = ManagedClient::new("gh").with_credentials(Arc::new(provider));
        for host in [Some("github.com"), None] {
            assert!(
                client
                    .resolve_credential(CredentialService::GitHub, host)
                    .await
                    .is_err(),
                "provider error must propagate (fail-closed), host={host:?}"
            );
        }
    }

    // The default budget is unlimited — no ceiling, so a client that never sets
    // one keeps its pre-budget (unbounded) capture behaviour, and both policy
    // projections are `None` (leave the command's own buffer untouched).
    #[test]
    fn output_budget_default_is_unlimited() {
        let b = OutputBudget::default();
        assert!(b.is_unlimited());
        assert_eq!(b, OutputBudget::unlimited());
        assert_eq!(b.max_bytes(), None);
        assert_eq!(b.max_lines(), None);
        assert!(b.content_policy().is_none());
        assert!(b.diagnostic_policy().is_none());
    }

    // A byte cap projects onto a FAIL-LOUD content policy (errors past the cap,
    // never truncates) and a DROP-OLDEST diagnostic policy (bounded tail, never
    // errors) — the two shapes one budget drives.
    #[test]
    fn output_budget_bytes_projects_to_both_policies() {
        let b = OutputBudget::bytes(4096);
        assert!(!b.is_unlimited());
        assert_eq!(b.max_bytes(), Some(4096));

        let content = b
            .content_policy()
            .expect("a byte budget yields a content policy");
        assert_eq!(
            content.overflow,
            OverflowMode::Error,
            "content is fail-loud"
        );
        assert_eq!(content.max_bytes, Some(4096));
        // No line cap set, so the fail-loud ceiling rests entirely on the byte cap
        // (which is exactly what the raw-stdout content path enforces).
        assert_eq!(content.max_lines, None);

        let diag = b
            .diagnostic_policy()
            .expect("a byte budget yields a diagnostic policy");
        assert_eq!(
            diag.overflow,
            OverflowMode::DropOldest,
            "diagnostics keep the tail, never OutputTooLarge"
        );
        assert_eq!(diag.max_bytes, Some(4096));
    }

    // A line ceiling composes with the byte cap on both projections.
    #[test]
    fn output_budget_with_max_lines_composes() {
        let b = OutputBudget::bytes(4096).with_max_lines(200);
        assert_eq!(b.max_lines(), Some(200));
        let content = b.content_policy().unwrap();
        assert_eq!(content.max_lines, Some(200));
        assert_eq!(content.max_bytes, Some(4096));
        assert_eq!(content.overflow, OverflowMode::Error);
        let diag = b.diagnostic_policy().unwrap();
        assert_eq!(diag.max_lines, Some(200));
        assert_eq!(diag.max_bytes, Some(4096));
        assert_eq!(diag.overflow, OverflowMode::DropOldest);
    }

    // The client-level default budget round-trips through the builder/getter, and
    // `budget_diagnostics` applies (or, when unlimited, leaves) a command's buffer.
    #[test]
    fn managed_client_default_output_budget_round_trips() {
        let client = ManagedClient::new("git");
        assert!(client.output_budget().is_unlimited());
        let client = client.default_output_budget(OutputBudget::bytes(1 << 20));
        assert_eq!(client.output_budget(), OutputBudget::bytes(1 << 20));
    }

    // T-130 (processkit 3.0 raw-byte-accounting audit): pin the *unit* of the byte
    // ceiling on the raw-stdout content path — the path every content verb reads
    // through. There is no line framing here, so the cap counts pipe bytes
    // verbatim (terminators included, nothing decoded) and fires strictly PAST the
    // cap. processkit counted exactly this on the raw path before 3.0 as well,
    // which is precisely why its switch to raw-pipe-byte accounting did NOT move
    // this ceiling; every pre-existing over-budget test across the wrapper crates
    // uses a fixture ~2x its cap and so could not have detected a shift either
    // way. This one is exact on both sides of the boundary.
    #[tokio::test]
    async fn content_budget_counts_raw_stdout_bytes_verbatim() {
        // 8 raw bytes: 6 of line content plus the two LF terminators.
        let out = "abc\ndef\n";
        assert_eq!(out.len(), 8);
        let client = ManagedClient::with_runner(
            "tool",
            ScriptedRunner::new().on(["tool", "read"], Reply::ok(out)),
        );

        // Sitting exactly on the cap is within budget, and the bytes come back
        // verbatim (the trailing LF is content here, not a stripped terminator).
        let got = client
            .run_untrimmed_within(client.command(["read"]), OutputBudget::bytes(8))
            .await
            .expect("stdout exactly on the cap is within budget");
        assert_eq!(got, out);

        // A cap of 6 — the decoded line-CONTENT total, i.e. what the pre-3.0 line
        // accounting would have measured — is not the unit on this path: the raw
        // 8 bytes trip it, and the reported total is the raw one.
        match client
            .run_untrimmed_within(client.command(["read"]), OutputBudget::bytes(6))
            .await
            .map_err(Error::into_reason)
        {
            Err(ErrorReason::OutputTooLarge {
                max_bytes,
                total_bytes,
                max_lines,
                total_lines,
                ..
            }) => {
                assert_eq!(max_bytes, Some(6), "the allowed ceiling");
                assert_eq!(total_bytes, 8, "raw pipe bytes, both terminators charged");
                // Raw stdout has no lines, so only the byte ceiling is reported.
                assert_eq!(max_lines, None);
                assert_eq!(total_lines, 0);
            }
            other => panic!("expected OutputTooLarge, got {other:?}"),
        }
    }

    // T-130: the same content ceiling also rides the command's line-pumped STDERR,
    // and *that* stream is where processkit 3.0's accounting change is observable:
    // the fail-loud ceiling now charges every line terminator, so a stderr flood
    // whose decoded content would still have fitted the cap trips it. This is a
    // real behavioural change this workspace's budget inherited from the 3.0 bump
    // (T-129) that no test covered — the audit's genuinely uncovered gap.
    #[tokio::test]
    async fn content_budget_charges_stderr_line_terminators() {
        // 18 raw stderr bytes: 16 of line content plus the two LF terminators.
        let warnings = "warn one\nwarn two\n";
        assert_eq!(warnings.len(), 18);
        let client = ManagedClient::with_runner(
            "tool",
            ScriptedRunner::new().on(["tool", "read"], Reply::ok("ok\n").with_stderr(warnings)),
        );

        // 17 sits above the 16 decoded content bytes and below the 18 raw ones, so
        // it is exactly the cap the old accounting would have let through.
        match client
            .run_untrimmed_within(client.command(["read"]), OutputBudget::bytes(17))
            .await
            .map_err(Error::into_reason)
        {
            Err(ErrorReason::OutputTooLarge {
                max_bytes,
                total_bytes,
                total_lines,
                ..
            }) => {
                assert_eq!(max_bytes, Some(17), "the allowed ceiling");
                assert_eq!(total_bytes, 18, "raw stderr bytes, terminators charged");
                assert_eq!(total_lines, 2, "every line is counted, dropped or not");
            }
            other => panic!("expected OutputTooLarge from stderr, got {other:?}"),
        }

        // One byte more of budget accepts the same stderr (the ceiling fires
        // strictly past the cap) and hands back stdout untouched.
        let got = client
            .run_untrimmed_within(client.command(["read"]), OutputBudget::bytes(18))
            .await
            .expect("stderr exactly on the cap is within budget");
        assert_eq!(got, "ok\n");
    }

    // --- T-148: the streamed run's own retention ceiling ------------------
    //
    // `run_with_progress` keeps its own copy of every streamed line so a rejected
    // exit can be promoted to a structured error. A command's `OutputBufferPolicy`
    // does not bound an event stream, so that copy is where a streaming
    // `clone`/`fetch --progress` used to grow without limit. These tests pin the
    // ceiling that now bounds it: what it counts, that it drops oldest-first
    // rather than failing loud, and that the default stays unbounded.

    // Drive one scripted streamed run under `budget`; hand back the error the
    // non-zero exit is promoted to — the only place the locally retained
    // stdout/stderr is observable.
    async fn streamed_failure(reply: Reply, budget: OutputBudget) -> Error {
        let runner = ScriptedRunner::new().on(["tool", "network-op"], reply);
        let mut progress = |_event: ProcessEvent| {};
        run_with_progress_within(
            &runner,
            &Command::new("tool").arg("network-op"),
            &mut progress,
            budget,
        )
        .await
        .expect_err("the scripted non-zero exit stays an error")
    }

    // The retained streams `ensure_success` carried into the structured error.
    fn exit_streams(err: &Error) -> (&str, &str) {
        match err.reason() {
            ErrorReason::Exit { stdout, stderr, .. } => (stdout, stderr),
            other => panic!("expected a structured Exit, got {other:?}"),
        }
    }

    // The unit question, pinned exactly (the [[K-073]] `ScriptedRunner` pattern:
    // the canned output flows through the REAL pump, so a one-byte accounting
    // shift is observable). Three lines admit three different answers to "how big
    // is this?", and the two caps below straddle ours:
    //
    //   12 — the raw pipe bytes, every terminator charged. What processkit 3.0's
    //        fail-loud CONTENT ceiling counts on a line-pumped stream ([[K-070]]).
    //    9 — the decoded line content alone, no terminator charged. What
    //        processkit's own drop-oldest retention counts.
    //   11 — the string this function retains: the content plus the one `\n` it
    //        puts between each retained PAIR. **This** is the unit here.
    //
    // A cap of 11 would already be over budget under the raw-byte unit, and a cap
    // of 10 would still be within budget under the content-only one — so the pair
    // of assertions admits no other reading.
    #[tokio::test]
    async fn streamed_tail_counts_the_bytes_it_retains() {
        let stream = "aaa\nbbb\nccc\n";
        assert_eq!(stream.len(), 12, "raw pipe bytes");

        let err = streamed_failure(Reply::fail(1, stream), OutputBudget::bytes(11)).await;
        assert_eq!(
            exit_streams(&err).1,
            "aaa\nbbb\nccc",
            "11 bytes is the whole retained tail — on the cap, not past it"
        );

        let err = streamed_failure(Reply::fail(1, stream), OutputBudget::bytes(10)).await;
        assert_eq!(
            exit_streams(&err).1,
            "bbb\nccc",
            "one byte less drops exactly the oldest line"
        );
    }

    // The point of the ceiling: a flood far past the cap is bounded on BOTH
    // streams — each carrying the budget independently, as a captured verb's two
    // streams do ([[K-072]]) — while the tail, where a CLI's fatal line sits,
    // survives intact. The non-zero exit is still promoted to a structured error
    // over that truncated text, and it still classifies.
    #[tokio::test]
    async fn streamed_budget_bounds_both_streams_and_keeps_the_tail_classifiable() {
        const CAP: usize = 128;
        let mut stdout = String::new();
        let mut stderr = String::new();
        for i in 0..500 {
            stdout.push_str(&format!("Receiving objects:  {i}% (0/0)\n"));
            stderr.push_str(&format!("remote: Counting objects: {i}\n"));
        }
        stdout.push_str("fatal: the remote end hung up unexpectedly\n");
        stderr.push_str("fatal: early EOF\n");
        assert!(stdout.len() > 20 * CAP && stderr.len() > 20 * CAP);

        let err = streamed_failure(
            Reply::fail(128, stderr).with_stdout(stdout),
            OutputBudget::bytes(CAP),
        )
        .await;
        let (out, err_text) = exit_streams(&err);
        assert!(out.len() <= CAP, "stdout bounded: {} bytes", out.len());
        assert!(
            err_text.len() <= CAP,
            "stderr bounded independently: {} bytes",
            err_text.len()
        );
        assert!(!out.contains("Receiving objects:  0%"), "oldest dropped");
        assert!(
            !err_text.contains("Counting objects: 0\n"),
            "oldest dropped"
        );
        assert!(out.ends_with("fatal: the remote end hung up unexpectedly"));
        assert!(err_text.ends_with("fatal: early EOF"));
        assert!(
            matches!(err.reason(), ErrorReason::Exit { code: 128, .. }),
            "a truncated capture is still promoted to a structured exit error"
        );
        assert!(
            is_transient_fetch_error(&err),
            "the retained tail still carries the transient marker"
        );
    }

    // The same for the other stderr-text classifier, and with a filler that a
    // real repository could produce: `is_lock_contention` reads the tail it is
    // left with, not the flood that preceded it.
    #[tokio::test]
    async fn streamed_budget_keeps_lock_contention_classifiable() {
        let mut stderr = String::new();
        for i in 0..200 {
            stderr.push_str(&format!("warning: unable to rmdir stale-{i}\n"));
        }
        stderr.push_str("fatal: Unable to create '/w/.git/index.lock': File exists.\n");

        let err = streamed_failure(Reply::fail(128, stderr), OutputBudget::bytes(96)).await;
        let retained = exit_streams(&err).1;
        assert!(retained.len() <= 96, "bounded: {} bytes", retained.len());
        assert!(is_lock_contention(&err), "retained tail: {retained:?}");
    }

    // Drop-oldest, never fail-loud (the `diagnostic_policy` half of the contract,
    // not the `content_policy` one): passing the ceiling truncates what is kept
    // and leaves the run's real outcome alone — a successful stream stays `Ok`
    // instead of becoming `OutputTooLarge`. Delivery is untouched too: the
    // callback still observes every line, only *retention* is bounded.
    #[tokio::test]
    async fn streamed_budget_truncates_without_failing_the_run() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let mut stream = String::new();
        for i in 0..400 {
            stream.push_str(&format!("Receiving objects:  {i}% (0/0)\n"));
        }
        let runner = ScriptedRunner::new().on(
            ["tool", "network-op"],
            Reply::ok(stream.clone()).with_stderr(stream),
        );
        let delivered = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&delivered);
        let mut progress = move |event: ProcessEvent| {
            if matches!(event, ProcessEvent::Stdout(_)) {
                seen.fetch_add(1, Ordering::SeqCst);
            }
        };

        run_with_progress_within(
            &runner,
            &Command::new("tool").arg("network-op"),
            &mut progress,
            OutputBudget::bytes(64),
        )
        .await
        .expect("a bounded stream is truncated, not failed");
        drop(progress);
        assert_eq!(
            delivered.load(Ordering::SeqCst),
            400,
            "the ceiling bounds retention, not what the callback is shown"
        );
    }

    // An over-cap single line keeps its own tail (cut on a UTF-8 char boundary)
    // rather than being dropped whole the way processkit's drop-mode buffer drops
    // it: under the default `\n` framing, `--progress`'s carriage-return output is
    // ONE ever-growing line, so dropping it whole would retain nothing at all of
    // the stream this ceiling exists to bound. The cut here lands mid-`☃` (a
    // 3-byte char straddling the boundary), so the walk to the next boundary is
    // what keeps the slice valid — and shorter than the cap, never longer.
    #[tokio::test]
    async fn streamed_tail_cuts_an_over_cap_line_on_a_char_boundary() {
        let line = format!("{}☃fatal: early EOF", "a".repeat(10));
        assert_eq!(line.len(), 29);
        // 29 - 18 = 11 is a continuation byte of the snowman (bytes 10..13).
        assert!(!line.is_char_boundary(11));

        let err = streamed_failure(Reply::fail(128, line), OutputBudget::bytes(18)).await;
        let retained = exit_streams(&err).1;
        assert_eq!(
            retained, "fatal: early EOF",
            "the tail survives; the straddling char is dropped whole"
        );
        assert!(retained.len() <= 18);
        assert!(is_transient_fetch_error(&err));
    }

    // The line ceiling composes, dropping oldest-first like the byte one.
    #[tokio::test]
    async fn streamed_tail_honours_a_line_ceiling() {
        let err = streamed_failure(
            Reply::fail(1, "one\ntwo\nthree\nfour\nfive\n"),
            OutputBudget::bytes(1024).with_max_lines(2),
        )
        .await;
        assert_eq!(exit_streams(&err).1, "four\nfive");
    }

    // The regression anchor for the default path: with no budget — the free
    // function, or a client that never set one — retention is unbounded, exactly
    // as it was before the ceiling existed. Every line of both streams is kept,
    // joined by `\n` with no trailing terminator.
    #[tokio::test]
    async fn streamed_run_without_a_budget_retains_everything() {
        let lines: Vec<String> = (0..300)
            .map(|i| format!("remote: Counting objects: {i}"))
            .collect();
        let expected = lines.join("\n");
        let stream = format!("{expected}\n");
        assert!(stream.len() > 8000);

        let runner = ScriptedRunner::new().on(
            ["tool", "network-op"],
            Reply::fail(1, stream.clone()).with_stdout(stream),
        );
        let mut progress = |_event: ProcessEvent| {};
        let err = run_with_progress(
            &runner,
            &Command::new("tool").arg("network-op"),
            &mut progress,
        )
        .await
        .expect_err("the scripted non-zero exit stays an error");
        assert_eq!(exit_streams(&err), (expected.as_str(), expected.as_str()));

        // The client path defaults to the same unlimited budget.
        let client = ManagedClient::with_runner("tool", runner);
        assert!(client.output_budget().is_unlimited());
        let err = client
            .run_with_progress(client.command(["network-op"]), &mut progress)
            .await
            .expect_err("the scripted non-zero exit stays an error");
        assert_eq!(exit_streams(&err), (expected.as_str(), expected.as_str()));
    }

    // The client plumbs its own default budget into the streamed path — the same
    // knob that bounds a captured `clone`/`fetch`'s diagnostics — and a single
    // call can override it in either direction, like `run_untrimmed_within`.
    #[tokio::test]
    async fn managed_client_streams_within_its_budget() {
        let runner =
            ScriptedRunner::new().on(["tool", "network-op"], Reply::fail(1, "aaa\nbbb\nccc\n"));
        let client = ManagedClient::with_runner("tool", runner)
            .default_output_budget(OutputBudget::bytes(7));
        let mut progress = |_event: ProcessEvent| {};

        let err = client
            .run_with_progress(client.command(["network-op"]), &mut progress)
            .await
            .expect_err("the scripted non-zero exit stays an error");
        assert_eq!(
            exit_streams(&err).1,
            "bbb\nccc",
            "the client default applies"
        );

        let err = client
            .run_with_progress_within(
                client.command(["network-op"]),
                &mut progress,
                OutputBudget::bytes(3),
            )
            .await
            .expect_err("the scripted non-zero exit stays an error");
        assert_eq!(exit_streams(&err).1, "ccc", "a tighter per-call cap wins");

        let err = client
            .run_with_progress_within(
                client.command(["network-op"]),
                &mut progress,
                OutputBudget::unlimited(),
            )
            .await
            .expect_err("the scripted non-zero exit stays an error");
        assert_eq!(
            exit_streams(&err).1,
            "aaa\nbbb\nccc",
            "and so does lifting the cap for one call"
        );
    }

    // The separator bookkeeping behind that ceiling is the one place a
    // drop-oldest tail goes subtly wrong: an off-by-one surfaces only on a
    // particular sequence of drops, and the exact-boundary test above pins one
    // such sequence. This pins the **invariant** over arbitrary ones, including
    // the shapes a fixture rarely reaches — empty lines (whose separator the
    // renderer elides, so the running estimate over-counts and must stay an
    // upper bound), lines far past the cap (kept as their own tail), and
    // multi-byte text (a cut that has to walk to a char boundary). Driven
    // against the retention buffer directly: no runtime, no scripted process,
    // just the arithmetic.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn a_streamed_tail_stays_within_its_budget_whatever_the_stream(
            lines in prop::collection::vec("[a-z ☃]{0,16}", 1..40),
            max_bytes in 1usize..48,
            line_cap in prop::option::of(1usize..12),
        ) {
            let budget = match line_cap {
                Some(max_lines) => OutputBudget::bytes(max_bytes).with_max_lines(max_lines),
                None => OutputBudget::bytes(max_bytes),
            };
            let mut retained = RetainedStream::new(budget);
            for line in &lines {
                retained.push(line);
            }
            let text = retained.into_string();

            // The whole point: what is handed to the error is bounded, always.
            prop_assert!(
                text.len() <= max_bytes,
                "retained {} bytes over a {max_bytes}-byte cap: {text:?}",
                text.len()
            );
            if let Some(max_lines) = line_cap {
                // Rendered separators are at most one per retained pair, so the
                // split count can only under-report the retained lines.
                prop_assert!(text.split('\n').count() <= max_lines);
            }
            // The tail is the part worth keeping (a CLI's fatal line lands
            // last), so the newest line survives whenever it fits at all.
            let last = lines.last().expect("the strategy generates at least one line");
            if last.len() <= max_bytes {
                prop_assert!(
                    text.ends_with(last.as_str()),
                    "the newest line {last:?} fits the cap but is not the tail of {text:?}"
                );
            }
        }
    }
}
