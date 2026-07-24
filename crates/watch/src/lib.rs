#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-watch` — filesystem-watch a git/jj repository and emit typed state-change
//! events.
//!
//! A [`RepoWatcher`] watches a repository's `.git`/`.jj` state directory (and,
//! optionally, the working tree), **debounces** the burst of writes a VCS
//! operation makes, **re-queries** the repo state through
//! [`vcs-core`](vcs_core)'s batched [`snapshot`](vcs_core::Repo::snapshot), and
//! **diffs** it against the previous state to yield typed [`RepoEvent`]s. Each
//! settled change arrives as a [`RepoChange`] carrying both the new
//! [`RepoSnapshot`] (to render a prompt/status line) and the deltas (to react).
//! It's the foundation for prompts, status bars, TUIs, and repo daemons.
//!
//! Re-query-and-diff — rather than interpreting raw filesystem events — is what
//! makes it robust: git's ref temp-file renames, `index.lock` churn, and reflog
//! noise all just coalesce into one "re-check the settled state" instead of being
//! (mis)read as events. Noise that doesn't move observable state emits nothing,
//! and every emission carries the true current state, so a stray event can't
//! desync the consumer.
//!
//! # The surface
//!
//! - **[`RepoWatcher`]** — a live watch over one repository. Start it with
//!   [`RepoWatcher::watch`] (defaults) or the [`Builder`]; drop it to stop the OS
//!   watch and the background task.
//! - **[`Builder`]** ([`RepoWatcher::builder`]) — set the watch scope and timing,
//!   then [`build`](Builder::build): [`working_tree`](Builder::working_tree) to
//!   also watch the tree recursively, [`debounce`](Builder::debounce) (the quiet
//!   window), [`max_wait`](Builder::max_wait) (the re-query ceiling under a
//!   continuous stream), [`requery_timeout`](Builder::requery_timeout) (the
//!   per-re-query deadline). The [`DEFAULT_REQUERY_TIMEOUT`] et al. name the
//!   defaults.
//! - **[`RepoEvent`]** — one typed delta, derived by diffing two snapshots:
//!   [`HeadMoved`](RepoEvent::HeadMoved),
//!   [`BranchSwitched`](RepoEvent::BranchSwitched),
//!   [`BranchCreated`](RepoEvent::BranchCreated) /
//!   [`BranchDeleted`](RepoEvent::BranchDeleted),
//!   [`WorkingCopyChanged`](RepoEvent::WorkingCopyChanged), and the
//!   upstream/ahead-behind/operation/conflict variants (`#[non_exhaustive]`).
//! - **[`RepoChange`]** — a settled change: the fresh [`RepoSnapshot`] (render a
//!   status line off it) plus the non-empty `events` vec (react to it).
//! - **Consumption** — pull changes with [`recv`](RepoWatcher::recv)
//!   (`Option<RepoChange>`; `None` once the watch backend dies, is dropped, or
//!   otherwise ends), or, under the **`stream`** feature, poll the watcher as a
//!   `futures_core::Stream`. Both pull from the same channel and advance
//!   [`current`](RepoWatcher::current), the last-pulled snapshot. A timed-out or
//!   transiently failed re-query is **retried automatically** with bounded
//!   exponential backoff — even with no new filesystem event — so a miss on the
//!   last signal isn't stuck until the next one; a *permanent* OS-watch backend
//!   failure (e.g. the watched `.git`/`.jj` dir was removed) closes this channel,
//!   so `recv`/the stream observe it directly instead of requiring separate
//!   stats polling.
//! - **[`WatcherStats`]** ([`stats`](RepoWatcher::stats)) — lock-free health
//!   counters (re-queries run, changes emitted, skips, retries, recoveries,
//!   terminal failures, and the last skip's [`WatcherErrorKind`]). Climbing
//!   [`skipped`](WatcherStats::skipped) with flat [`changes`](WatcherStats::changes)
//!   means a wedged repo — poll it from a health check rather than inferring
//!   health from event silence.
//! - **[`Error`]** — a setup/build failure: a [`Vcs`](Error::Vcs) baseline
//!   re-query error, an [`Io`](Error::Io) filesystem error, or a
//!   [`Notify`](Error::Notify) filesystem-watch backend failure. The watch
//!   backend is a **private** dependency, so its failures are the opaque
//!   [`WatchError`] — classify them (`is_path_not_found` / `is_watch_limit` /
//!   `io_error`) and source-chain them through `vcs-watch` alone, with no direct
//!   dependency on the third-party watch crate to keep version-matched.
//!
//! # Recipes
//!
//! Watch with the defaults and react to each settled change:
//!
//! ```no_run
//! use vcs_core::Repo;
//! use vcs_watch::RepoWatcher;
//! # async fn run() -> vcs_watch::Result<()> {
//! let repo = Repo::discover(".")?;
//! let mut watcher = RepoWatcher::watch(repo).await?;
//! while let Some(change) = watcher.recv().await {
//!     for event in &change.events {
//!         println!("{event:?}");
//!     }
//!     // `change.snapshot` is the fresh full state — render a status line off it.
//! }
//! # Ok(()) }
//! ```
//!
//! Under the **`stream`** feature the watcher *is* a `futures_core::Stream`,
//! so it drops into stream combinators and `tokio::select!` directly (needs
//! `futures`/`tokio-stream`'s `StreamExt` in scope):
//!
//! ```ignore
//! use futures::StreamExt;
//! use vcs_core::Repo;
//! use vcs_watch::RepoWatcher;
//! # async fn run() -> vcs_watch::Result<()> {
//! let repo = Repo::discover(".")?;
//! let mut watcher = RepoWatcher::watch(repo).await?;
//! while let Some(change) = watcher.next().await {
//!     println!("{} event(s)", change.events.len());
//! }
//! # Ok(()) }
//! ```
//!
//! **Runtime:** unlike the rest of the toolkit (which hides tokio behind
//! `processkit`), `vcs-watch` uses **tokio at runtime** — the watch task and the
//! debounce timer run on the caller's tokio runtime, so build/await it from
//! within one.
//!
//! # Testing
//!
//! The debounce → ceiling → re-query pipeline is a free function over injected
//! seams, so it is exercised hermetically on a **paused clock** (no real
//! filesystem or sleeps); a consumer's own watch code tests the same way it tests
//! any [`vcs-core`](vcs_core) consumer — build the [`Repo`](vcs_core::Repo) over a
//! fake runner (processkit's `ScriptedRunner`) so the re-query returns canned
//! state. See
//! [vcs-testkit's guide](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/).
//!
//! # In-depth guide
//!
//! Beyond this page, this crate ships a full how-to guide — rendered on docs.rs
//! from `docs/`. See the [`guide`] module.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use tokio::sync::mpsc;
use vcs_core::{BackendKind, VcsRepo};

mod error;
mod event;

pub use error::{Error, Result, WatchError};
pub use event::{RepoChange, RepoEvent};
// Re-export the snapshot types a consumer reads off a `RepoChange`, so depending
// on `vcs-watch` alone suffices.
pub use vcs_core::{OperationState, RepoSnapshot};
// Re-export `processkit` so a `vcs-watch`-only consumer can name the
// `Error::processkit_error()` return type without a direct `processkit`
// dependency (mirrors `vcs_core::processkit` / `vcs_forge::processkit`).
pub use processkit;

/// Default quiet window: a re-query fires once the watched dir has been silent
/// for this long after the last event.
const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(250);
/// Default ceiling: even under a continuous stream of events, re-query at least
/// this often (so a long bulk operation still reports progress).
const DEFAULT_MAX_WAIT: Duration = Duration::from_secs(1);
/// Upper clamp for [`max_wait`](Builder::max_wait) when it is turned into an
/// `Instant` deadline. `Instant + Duration` *panics* on overflow, and `max_wait`
/// is caller-settable with no bound (`.max_wait(Duration::MAX)` is a natural
/// "disable the ceiling" idiom), so cap the addend at an effectively-unbounded
/// one year — a huge value then disables the ceiling instead of panicking the
/// spawned watch loop, which would drop the output channel and kill the watcher
/// silently.
const MAX_WAIT_CEILING: Duration = Duration::from_secs(60 * 60 * 24 * 365);
/// Default deadline on a single re-query (`snapshot` + branch list): a wedged
/// command (e.g. a held `index.lock` with no client timeout configured) is
/// killed and skipped instead of stalling the watch loop forever.
pub const DEFAULT_REQUERY_TIMEOUT: Duration = Duration::from_secs(30);
/// Bounded output channel: a slow consumer applies backpressure (the loop pauses
/// re-querying), and pending filesystem signals coalesce into one catch-up query.
const OUTPUT_CAPACITY: usize = 64;
const REQUERY_RETRY_LIMIT: u32 = 3;
const REQUERY_RETRY_BACKOFF: Duration = Duration::from_millis(100);
const REQUERY_RETRY_BACKOFF_MAX: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
enum WatchSignal {
    Change,
    BackendFailed,
}

/// The timing/capacity knobs the background loop runs under — bundled so the
/// loop signature stays small and the hermetic tests can vary them (notably
/// `output_capacity`, which the backpressure test shrinks to 1).
struct LoopConfig {
    debounce: Duration,
    max_wait: Duration,
    /// `None` disables the per-re-query deadline.
    requery_timeout: Option<Duration>,
    /// Whether a re-query may snapshot the jj working copy (opt-in mutation) or
    /// must stay read-only (the default). See [`Builder::snapshot_working_copy`].
    snapshot_working_copy: bool,
    output_capacity: usize,
    retry_limit: u32,
    retry_backoff: Duration,
}

/// Builder for a [`RepoWatcher`] — set the watch scope and debounce timing, then
/// [`build`](Builder::build).
pub struct Builder {
    repo: Box<dyn VcsRepo>,
    working_tree: bool,
    snapshot_working_copy: bool,
    debounce: Duration,
    max_wait: Duration,
    requery_timeout: Option<Duration>,
}

impl Builder {
    /// Also watch the **working tree** recursively, so a bare unstaged edit
    /// (`vim file`) fires [`WorkingCopyChanged`](RepoEvent::WorkingCopyChanged)
    /// immediately. Off by default (only the `.git`/`.jj` state dir is watched,
    /// which catches an unstaged edit once it touches the index / a jj snapshot).
    ///
    /// Note: `notify` is `.gitignore`-unaware, so this also watches ignored and
    /// build directories — heavier on a large tree.
    ///
    /// **jj note:** on jj, a bare working-tree edit only becomes an observable
    /// state change once *something* snapshots the working copy. The re-query is
    /// **read-only by default** (it must not itself snapshot — see
    /// [`snapshot_working_copy`](Self::snapshot_working_copy)), so watching the
    /// tree alone will not surface an unsnapshotted edit as a
    /// [`WorkingCopyChanged`](RepoEvent::WorkingCopyChanged): the event fires once
    /// a jj command (or another watcher opted into
    /// [`snapshot_working_copy`](Self::snapshot_working_copy)) records it. Opt into
    /// [`snapshot_working_copy(true)`](Self::snapshot_working_copy) to have the
    /// re-query itself snapshot, at the cost of the watcher recording jj
    /// operations.
    pub fn working_tree(mut self, yes: bool) -> Self {
        self.working_tree = yes;
        self
    }

    /// Whether each re-query may let **jj snapshot the working copy** — off by
    /// default, which keeps the watcher a pure *observer*.
    ///
    /// By default (`false`) the re-query is **read-only**: on jj it passes
    /// `--ignore-working-copy` (via
    /// [`Repo::snapshot_readonly`](vcs_core::Repo::snapshot_readonly)), so
    /// observing the repo records **no** jj operation and never moves `@`. This is
    /// almost always what you want: an ordinary jj query snapshots the working
    /// copy as a side effect (taking the working-copy lock, recording an
    /// operation, possibly moving `@`), so a naive watcher would *mutate* the very
    /// state it reports — and, worse, a [`requery_timeout`](Self::requery_timeout)
    /// firing mid-snapshot would abort that mutation.
    ///
    /// The trade-off (jj only): a bare working-tree edit that no jj command has
    /// snapshotted yet is **not** reflected until a real jj operation records it.
    /// If your consumer genuinely needs to observe such unsnapshotted edits (e.g.
    /// a live "dirty" indicator driven purely by filesystem edits), set this
    /// `true`: each re-query then snapshots the working copy (via
    /// [`Repo::snapshot`](vcs_core::Repo::snapshot)), **recording a jj operation
    /// and possibly moving `@`** — an explicit, opt-in mutation, not a hidden side
    /// effect of reading. Pair it with [`working_tree(true)`](Self::working_tree)
    /// so the tree edits actually trigger a re-query.
    ///
    /// On **git** this knob has no effect — git's status/branch queries never
    /// record operations or move refs, so both modes behave identically.
    pub fn snapshot_working_copy(mut self, yes: bool) -> Self {
        self.snapshot_working_copy = yes;
        self
    }

    /// The quiet window: re-query once the watched dir has been silent this long
    /// after the last event (default 250 ms). Coalesces an operation's write
    /// burst into one re-check.
    pub fn debounce(mut self, window: Duration) -> Self {
        self.debounce = window;
        self
    }

    /// The ceiling on how long a continuous event stream defers the re-query
    /// (default 1 s) — a long bulk operation still reports at this cadence.
    pub fn max_wait(mut self, ceiling: Duration) -> Self {
        self.max_wait = ceiling;
        self
    }

    /// Deadline on a single re-query (the `snapshot` + branch-list pair), default
    /// [`DEFAULT_REQUERY_TIMEOUT`] (30 s); `None` disables it. Orthogonal to
    /// [`max_wait`](Self::max_wait): that bounds how long signals may *defer* a
    /// re-query, this bounds how long one re-query may *run*. On overrun the
    /// spawned commands are killed (kill-on-drop) and the re-query is retried
    /// three times with bounded exponential backoff, even if no new filesystem
    /// event arrives.
    ///
    /// It **also bounds the startup baseline** captured by [`build`](Self::build): a
    /// baseline that overruns fails `build()` with a transient `Io` `TimedOut`
    /// (`Error::is_transient()`), rather than hanging the caller — so a wedged repo
    /// can't stall `build()` any more than it can stall the loop.
    ///
    /// Note: on a very large repository a *cold-cache* `git status` (first run
    /// after a `gc`, or on a slow disk) can legitimately exceed the 30 s default
    /// — raise it (or pass `None`) there; a watcher whose every re-query is
    /// being killed shows up as climbing [`WatcherStats::skipped`] with flat
    /// `changes`.
    pub fn requery_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.requery_timeout = timeout;
        self
    }

    /// Start watching. Captures the baseline state, registers the filesystem
    /// watch, and spawns the background re-query task on the current tokio
    /// runtime.
    ///
    /// The baseline capture is bounded by [`requery_timeout`](Self::requery_timeout),
    /// so on a wedged repo `build()` returns a transient `Io` `TimedOut`
    /// (`Error::is_transient()`) instead of hanging at startup — retry, or raise the
    /// timeout.
    pub async fn build(self) -> Result<RepoWatcher> {
        let root = self.repo.root().to_path_buf();
        // The dirs whose writes mean "re-check": the `.git`/`.jj` state dir, plus
        // — for a linked git worktree — the *shared* git dir it points at via
        // `commondir` (where `refs/heads/*` and `packed-refs` actually live, so
        // branch create/delete is seen). See `state_dirs`.
        let state_dirs = state_dirs(self.repo.kind(), &root)?;

        // Bridge: notify's callback thread pushes a unit "something changed" signal
        // per event; the debounce loop drains it. The channel is **capacity 1** and
        // the callback uses `try_send`, so a burst *coalesces* into a single pending
        // signal (extra events while one is pending are dropped — the loop re-queries
        // the full snapshot anyway, so no state is lost). This bounds memory: an
        // unbounded channel would grow without limit if the consumer stopped draining
        // the output while a filesystem storm churned (R2). Build the watcher and
        // register paths *before* the baseline snapshot, so a change racing the
        // baseline is queued, not lost.
        let (raw_tx, raw_rx) = mpsc::channel::<WatchSignal>(1);
        let stats = Arc::new(StatsInner::default());
        let cb_stats = Arc::clone(&stats);
        // Sticky because the capacity-1 coalescing channel may already contain a
        // change when notify reports its terminal backend error.
        let watch_failed = Arc::new(AtomicBool::new(false));
        let cb_watch_failed = Arc::clone(&watch_failed);
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                // A backend error is sticky and terminal: the loop closes its
                // public output channel so recv/Stream observes the failure
                // without requiring stats polling. Ordinary events just mean
                // "re-check"; their content is irrelevant.
                if res.is_err() {
                    cb_stats.note_watch_error();
                    if !cb_watch_failed.swap(true, Ordering::AcqRel) {
                        cb_stats.note_terminal_failure();
                    }
                }
                // `try_send` on the capacity-1 channel: succeeds when no signal is
                // pending, drops (coalesces) when one already is. Never blocks the
                // notify callback thread; `Err` (full or loop-ended) is intentionally
                // ignored.
                let signal = if res.is_err() {
                    WatchSignal::BackendFailed
                } else {
                    WatchSignal::Change
                };
                let _ = raw_tx.try_send(signal);
            })?;
        if self.working_tree {
            watcher.watch(&root, RecursiveMode::Recursive)?;
            // A worktree gitlink puts the real (private and shared) git dirs
            // outside `root`; cover any not already under the recursive root watch.
            for dir in &state_dirs {
                if !dir.starts_with(&root) {
                    watcher.watch(dir, RecursiveMode::Recursive)?;
                }
            }
        } else {
            for dir in &state_dirs {
                watcher.watch(dir, RecursiveMode::Recursive)?;
            }
        }

        // Capture the baseline under the same `requery_timeout` deadline the loop
        // applies to every re-query (R4) — otherwise a snapshot that wedges (a hung
        // fsmonitor, a network filesystem, a held jj lock) on a `Repo` built without
        // its own `default_timeout` would hang `build()` at startup, the very failure
        // the loop-side deadline exists to prevent.
        let (snapshot, branches) = capture_baseline(
            &*self.repo,
            self.requery_timeout,
            self.snapshot_working_copy,
        )
        .await?;
        let baseline = snapshot.clone();
        let prev = event::WatchState::from_snapshot(&snapshot, branches);

        let config = LoopConfig {
            debounce: self.debounce,
            max_wait: self.max_wait,
            requery_timeout: self.requery_timeout,
            snapshot_working_copy: self.snapshot_working_copy,
            output_capacity: OUTPUT_CAPACITY,
            retry_limit: REQUERY_RETRY_LIMIT,
            retry_backoff: REQUERY_RETRY_BACKOFF,
        };
        let (out_tx, out_rx) = mpsc::channel::<RepoChange>(config.output_capacity);
        let task = tokio::spawn(watch_loop(
            self.repo,
            raw_rx,
            out_tx,
            prev,
            config,
            Arc::clone(&stats),
            watch_failed,
        ));

        Ok(RepoWatcher {
            rx: out_rx,
            current: baseline,
            stats,
            _watcher: watcher,
            task,
        })
    }
}

// --- Watcher health counters --------------------------------------------------

/// What the last skipped re-query failed on (see [`WatcherStats::last_error`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WatcherErrorKind {
    /// The snapshot re-query returned an error (e.g. a transiently held lock).
    Snapshot,
    /// The branch-list re-query returned an error.
    Branches,
    /// The re-query exceeded [`Builder::requery_timeout`] and was killed.
    Timeout,
}

/// A cheap point-in-time copy of the watcher's health counters — see
/// [`RepoWatcher::stats`]. Lets a long-running consumer notice a watcher that is
/// silently skipping re-queries (e.g. a permanently wedged repository) instead
/// of inferring health from event silence.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct WatcherStats {
    /// Re-query attempts started, including automatic retry attempts.
    pub requeries: u64,
    /// Re-queries that emitted a [`RepoChange`] (the rest found no difference).
    pub changes: u64,
    /// Re-queries skipped — transient query failures plus deadline overruns.
    pub skipped: u64,
    /// Automatic re-query retries scheduled after transient failures/timeouts.
    pub retries: u64,
    /// Failed re-query sequences that later succeeded during automatic retry.
    pub recoveries: u64,
    /// Terminal filesystem-watch backend failures. When this increments, the
    /// output channel is closed and [`RepoWatcher::recv`] returns `None`.
    pub terminal_failures: u64,
    /// What the most recent skip failed on; `None` when nothing was ever skipped.
    pub last_error: Option<WatcherErrorKind>,
    /// Filesystem-watch **errors** reported by the OS backend (via `notify`). A
    /// non-zero — especially *climbing* — count means the underlying watch is
    /// failing: most often the watched `.git`/`.jj` directory was **removed and
    /// re-created** (a re-clone / `jj git init`), which invalidates the OS watch on
    /// the old directory. Such a reported error terminates the watch: `recv`
    /// returns `None` (and the stream ends), so the consumer can rebuild it.
    ///
    /// **Best-effort, platform-dependent.** It is reliable on **Windows**, where
    /// removing the watched directory fails `ReadDirectoryChangesW` and `notify`
    /// reports an error. On **Linux** (`inotify`) a removed/re-created directory may
    /// surface as an ordinary event or a silent watch teardown rather than an error,
    /// so `watch_errors` can stay `0` even as the watcher goes deaf — don't rely on
    /// it as the sole liveness signal there.
    pub watch_errors: u64,
}

/// Lock-free counter cell shared between the loop and `stats()` readers. Relaxed
/// ordering is enough: the counters are independent monotonic telemetry, not a
/// synchronization protocol.
#[derive(Default)]
struct StatsInner {
    requeries: AtomicU64,
    changes: AtomicU64,
    skipped: AtomicU64,
    /// 0 = none, else `WatcherErrorKind as u8 + 1`.
    last_error: AtomicU8,
    watch_errors: AtomicU64,
    retries: AtomicU64,
    recoveries: AtomicU64,
    terminal_failures: AtomicU64,
}

impl StatsInner {
    fn note_requery(&self) {
        self.requeries.fetch_add(1, Ordering::Relaxed);
    }

    fn note_change(&self) {
        self.changes.fetch_add(1, Ordering::Relaxed);
    }

    fn note_watch_error(&self) {
        self.watch_errors.fetch_add(1, Ordering::Relaxed);
    }

    fn note_retry(&self) {
        self.retries.fetch_add(1, Ordering::Relaxed);
    }

    fn note_recovery(&self) {
        self.recoveries.fetch_add(1, Ordering::Relaxed);
    }

    fn note_terminal_failure(&self) {
        self.terminal_failures.fetch_add(1, Ordering::Relaxed);
    }

    fn note_skip(&self, kind: WatcherErrorKind) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
        let code = match kind {
            WatcherErrorKind::Snapshot => 1,
            WatcherErrorKind::Branches => 2,
            WatcherErrorKind::Timeout => 3,
        };
        self.last_error.store(code, Ordering::Relaxed);
    }

    fn snapshot(&self) -> WatcherStats {
        let last_error = match self.last_error.load(Ordering::Relaxed) {
            1 => Some(WatcherErrorKind::Snapshot),
            2 => Some(WatcherErrorKind::Branches),
            3 => Some(WatcherErrorKind::Timeout),
            _ => None,
        };
        WatcherStats {
            requeries: self.requeries.load(Ordering::Relaxed),
            changes: self.changes.load(Ordering::Relaxed),
            skipped: self.skipped.load(Ordering::Relaxed),
            retries: self.retries.load(Ordering::Relaxed),
            recoveries: self.recoveries.load(Ordering::Relaxed),
            terminal_failures: self.terminal_failures.load(Ordering::Relaxed),
            last_error,
            watch_errors: self.watch_errors.load(Ordering::Relaxed),
        }
    }
}

/// A live watch over a repository, yielding [`RepoChange`]s as the repo's state
/// changes. Dropping it stops the filesystem watch and the background task.
pub struct RepoWatcher {
    rx: mpsc::Receiver<RepoChange>,
    current: RepoSnapshot,
    stats: Arc<StatsInner>,
    // Held to keep the OS watch alive; dropping it ends the watch (and the loop).
    _watcher: notify::RecommendedWatcher,
    task: tokio::task::JoinHandle<()>,
}

impl RepoWatcher {
    /// A builder over `repo` (any [`VcsRepo`] — e.g. a [`vcs_core::Repo`]).
    pub fn builder(repo: impl VcsRepo + 'static) -> Builder {
        Builder {
            repo: Box::new(repo),
            working_tree: false,
            // Read-only re-query by default: an observer must not snapshot the jj
            // working copy (record an operation / move `@`) merely by looking.
            snapshot_working_copy: false,
            debounce: DEFAULT_DEBOUNCE,
            max_wait: DEFAULT_MAX_WAIT,
            requery_timeout: Some(DEFAULT_REQUERY_TIMEOUT),
        }
    }

    /// Start watching `repo` with the defaults (state dir only, 250 ms debounce).
    pub async fn watch(repo: impl VcsRepo + 'static) -> Result<RepoWatcher> {
        Self::builder(repo).build().await
    }

    /// Await the next settled change. Returns `None` when the filesystem backend
    /// reports a terminal error, the watcher is dropped, or its task otherwise
    /// ends. A backend error also increments
    /// [`WatcherStats::terminal_failures`].
    pub async fn recv(&mut self) -> Option<RepoChange> {
        let change = self.rx.recv().await?;
        self.current = change.snapshot.clone();
        Some(change)
    }

    /// The most recent known snapshot — the baseline captured at
    /// [`build`](Builder::build), then the snapshot from each [`recv`](Self::recv).
    /// It advances **only when you call [`recv`](Self::recv)**, so it is as fresh
    /// as your last `recv`, not a live view.
    pub fn current(&self) -> &RepoSnapshot {
        &self.current
    }

    /// The watcher's health counters (re-queries run / changes emitted / skips,
    /// retry/recovery/terminal outcomes, the last skip, and OS-watch errors).
    /// Cheap relaxed-atomic
    /// reads — poll it from a health check or log it periodically; a climbing
    /// [`skipped`](WatcherStats::skipped) with flat
    /// [`changes`](WatcherStats::changes) means the repository is wedged, and a
    /// non-zero [`terminal_failures`](WatcherStats::terminal_failures) means the
    /// output channel has terminated after an OS-watch backend error.
    pub fn stats(&self) -> WatcherStats {
        self.stats.snapshot()
    }
}

/// Yields each settled [`RepoChange`] as a stream item (the `stream` feature).
/// Equivalent to looping [`recv`](RepoWatcher::recv) — both pull from the same
/// underlying channel (an item is delivered to whichever is polled first, never
/// duplicated) and both advance [`current`](RepoWatcher::current).
#[cfg(feature = "stream")]
#[cfg_attr(docsrs, doc(cfg(feature = "stream")))]
impl futures_core::Stream for RepoWatcher {
    type Item = RepoChange;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<RepoChange>> {
        // All fields are Unpin, so the watcher is Unpin and get_mut is sound.
        let this = self.get_mut();
        match this.rx.poll_recv(cx) {
            std::task::Poll::Ready(Some(change)) => {
                this.current = change.snapshot.clone();
                std::task::Poll::Ready(Some(change))
            }
            other => other,
        }
    }
}

impl Drop for RepoWatcher {
    fn drop(&mut self) {
        // The dropped `_watcher` already closes the signal channel (ending the
        // loop); abort is belt-and-braces for prompt teardown.
        self.task.abort();
    }
}

/// The batched state read a re-query (and the startup baseline) performs: the
/// snapshot plus the local-branch set. Routed through the **read-only** facade
/// methods by default (`snapshot_working_copy == false`) so an observer never
/// snapshots the jj working copy — no operation recorded, `@` unmoved. When the
/// consumer opts into [`Builder::snapshot_working_copy`], it uses the ordinary
/// (working-copy-snapshotting) facade methods instead, an explicit mutation.
///
/// The two calls are sequenced (branches after the snapshot) so both reflect the
/// same observation, matching the previous behaviour.
async fn read_state(
    repo: &dyn VcsRepo,
    snapshot_working_copy: bool,
) -> vcs_core::Result<(vcs_core::RepoSnapshot, Vec<String>)> {
    if snapshot_working_copy {
        let snapshot = repo.snapshot().await?;
        let branches = repo.local_branches().await?;
        Ok((snapshot, branches))
    } else {
        let snapshot = repo.snapshot_readonly().await?;
        let branches = repo.local_branches_readonly().await?;
        Ok((snapshot, branches))
    }
}

/// Capture the startup baseline (snapshot + local branches) under `requery_timeout`
/// (R4). A `Some(limit)` bounds the whole capture with `tokio::time::timeout`; on
/// expiry it returns [`Error::Io`] `TimedOut` and dropping the future kills the
/// underlying process (kill-on-drop), exactly as the loop does for a re-query — so a
/// wedged snapshot can't hang `build()` forever. `None` leaves it unbounded.
///
/// `snapshot_working_copy` picks the read-only vs working-copy-snapshotting facade
/// methods (see [`read_state`]), so the baseline is captured under the **same**
/// observation contract the loop then uses for every re-query.
async fn capture_baseline(
    repo: &dyn VcsRepo,
    requery_timeout: Option<Duration>,
    snapshot_working_copy: bool,
) -> Result<(vcs_core::RepoSnapshot, Vec<String>)> {
    let query = async {
        read_state(repo, snapshot_working_copy)
            .await
            .map_err(Error::from)
    };
    match requery_timeout {
        Some(limit) => match tokio::time::timeout(limit, query).await {
            Ok(result) => result,
            Err(_elapsed) => Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("baseline snapshot exceeded the {limit:?} requery_timeout"),
            ))),
        },
        None => query.await,
    }
}

/// The background loop: coalesce a burst of filesystem signals, re-query the
/// settled state, diff against the previous, and emit a [`RepoChange`] when
/// anything changed.
///
/// A free function over plain channels + a [`VcsRepo`] (not a method) on
/// purpose: the hermetic pipeline tests below drive it directly — a fake signal
/// channel in, a `ScriptedRunner`-backed `Repo`, a paused tokio clock — pinning
/// the debounce/ceiling/skip semantics without any real filesystem or process.
async fn watch_loop(
    repo: Box<dyn VcsRepo>,
    mut raw_rx: mpsc::Receiver<WatchSignal>,
    out_tx: mpsc::Sender<RepoChange>,
    mut prev: event::WatchState,
    config: LoopConfig,
    stats: Arc<StatsInner>,
    watch_failed: Arc<AtomicBool>,
) {
    'watch: loop {
        // Block until the first signal (or exit when the watcher is dropped).
        match raw_rx.recv().await {
            None | Some(WatchSignal::BackendFailed) => return,
            Some(WatchSignal::Change) if watch_failed.load(Ordering::Acquire) => return,
            Some(WatchSignal::Change) => {}
        }
        // Coalesce the burst: reset a `debounce` quiet-timer on every new signal,
        // but never wait past `max_wait` total. The dedicated `sleep_until` arm
        // makes the ceiling exact (it fires even when no further signal arrives);
        // the in-arm deadline check guards against a signal stream so dense that
        // the `biased` select never polls the timer arms.
        if drain(&mut raw_rx) || watch_failed.load(Ordering::Acquire) {
            return;
        }
        // Clamp the addend: `Instant + Duration` panics on overflow, and a huge
        // caller `max_wait` (e.g. `Duration::MAX`) must disable the ceiling, not
        // crash the loop. See [`MAX_WAIT_CEILING`].
        let deadline = tokio::time::Instant::now() + config.max_wait.min(MAX_WAIT_CEILING);
        loop {
            tokio::select! {
                biased;
                sig = raw_rx.recv() => {
                    match sig {
                        None | Some(WatchSignal::BackendFailed) => return,
                        Some(WatchSignal::Change) => {}
                    }
                    if watch_failed.load(Ordering::Acquire) {
                        return;
                    }
                    // Collapse the queued backlog: under a notify storm each
                    // queued unit signal would otherwise cost a select iteration
                    // that re-creates BOTH timer futures — a burst is one
                    // "still busy" observation, not N.
                    if drain(&mut raw_rx) {
                        return;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        break; // ceiling reached — re-query now
                    }
                    // else: another event — loop resets the quiet timer
                }
                _ = tokio::time::sleep_until(deadline) => break, // ceiling
                _ = tokio::time::sleep(config.debounce) => break, // settled
            }
        }

        // Re-query the settled state, bounded by the configured deadline — a
        // wedged command (a held `index.lock` on a client with no timeout) must
        // not stall the watch forever. Dropping the overrun future kills the
        // spawned process tree (processkit's kill-on-drop group), so a timed-out
        // query leaves no orphan. Failures and overruns are transient skips:
        // counted, traced, and retried with bounded exponential backoff.
        //
        // Deadline safety (jj): the default re-query is **read-only**
        // (`snapshot_working_copy == false` → `snapshot_readonly`/
        // `local_branches_readonly`, i.e. jj `--ignore-working-copy`), so it takes
        // no working-copy lock and records no operation — a `requery_timeout`
        // kill-on-drop can only interrupt a pure read, never a working-copy
        // snapshot mid-write. Only the explicit opt-in
        // (`snapshot_working_copy == true`) runs a mutating snapshot here, and that
        // is the caller's documented choice, not a read masquerading as read-only.
        let mut retry = 0;
        let (snapshot, branches) = loop {
            stats.note_requery();
            let requery = async {
                let (snapshot, branches) = if config.snapshot_working_copy {
                    let snapshot = repo
                        .snapshot()
                        .await
                        .map_err(|e| (WatcherErrorKind::Snapshot, e))?;
                    let branches = repo
                        .local_branches()
                        .await
                        .map_err(|e| (WatcherErrorKind::Branches, e))?;
                    (snapshot, branches)
                } else {
                    let snapshot = repo
                        .snapshot_readonly()
                        .await
                        .map_err(|e| (WatcherErrorKind::Snapshot, e))?;
                    let branches = repo
                        .local_branches_readonly()
                        .await
                        .map_err(|e| (WatcherErrorKind::Branches, e))?;
                    (snapshot, branches)
                };
                Ok::<_, (WatcherErrorKind, vcs_core::Error)>((snapshot, branches))
            };
            let outcome = match config.requery_timeout {
                Some(limit) => match tokio::time::timeout(limit, requery).await {
                    Ok(result) => result.map_err(Some),
                    Err(_elapsed) => {
                        stats.note_skip(WatcherErrorKind::Timeout);
                        #[cfg(feature = "tracing")]
                        tracing::debug!(
                            timeout = ?limit,
                            retry,
                            "vcs-watch: re-query exceeded its deadline; scheduling retry"
                        );
                        Err(None)
                    }
                },
                None => requery.await.map_err(Some),
            };
            let result = match outcome {
                Ok(pair) => Some(pair),
                Err(Some((kind, _e))) => {
                    stats.note_skip(kind);
                    #[cfg(feature = "tracing")]
                    tracing::debug!(
                        error = %_e,
                        retry,
                        "vcs-watch: re-query failed; scheduling retry"
                    );
                    None
                }
                Err(None) => None,
            };
            if let Some(pair) = result {
                if retry > 0 {
                    stats.note_recovery();
                }
                break pair;
            }
            if retry >= config.retry_limit {
                // The bounded sequence is exhausted. A future filesystem event
                // starts a fresh sequence, preserving long-term recovery without
                // spinning forever on a permanently broken repository.
                continue 'watch;
            }
            stats.note_retry();
            let delay = retry_backoff(config.retry_backoff, retry);
            retry += 1;
            let deadline = tokio::time::Instant::now() + delay;
            loop {
                tokio::select! {
                    signal = raw_rx.recv() => match signal {
                        None | Some(WatchSignal::BackendFailed) => return,
                        Some(WatchSignal::Change) => {
                            if drain(&mut raw_rx) || watch_failed.load(Ordering::Acquire) {
                                return;
                            }
                            // Coalesce new changes into the already-scheduled
                            // catch-up query, but retain the backoff deadline.
                        }
                    },
                    _ = tokio::time::sleep_until(deadline) => break,
                }
            }
        };

        if watch_failed.load(Ordering::Acquire) {
            return;
        }
        let next = event::WatchState::from_snapshot(&snapshot, branches);
        let events = event::diff(&prev, &next);
        prev = next;
        if events.is_empty() {
            continue;
        }
        if out_tx.send(RepoChange { snapshot, events }).await.is_err() {
            return; // receiver dropped — stop
        }
        stats.note_change();
    }
}

/// Drop every already-queued unit signal — the burst is one observation. Leaves
/// channel-closed detection to the caller's next `recv` (a drained-empty and a
/// closed channel both just stop yielding here).
fn drain(raw_rx: &mut mpsc::Receiver<WatchSignal>) -> bool {
    let mut failed = false;
    while let Ok(signal) = raw_rx.try_recv() {
        failed |= matches!(signal, WatchSignal::BackendFailed);
    }
    failed
}

fn retry_backoff(base: Duration, retry: u32) -> Duration {
    base.saturating_mul(1_u32.checked_shl(retry).unwrap_or(u32::MAX))
        .min(REQUERY_RETRY_BACKOFF_MAX)
}

/// The directories to watch for a backend, deduplicated. Normally one — the
/// `.git`/`.jj` state dir (see [`state_dir`]) — but a **linked git worktree** has
/// two: its private gitdir (HEAD/index/logs) *and* the shared git dir it points
/// at via `commondir` (`refs/heads/*` and `packed-refs`, where branch
/// create/delete actually lands). Watching only the private dir would miss every
/// `BranchCreated`/`BranchDeleted` on a worktree, since the shared dir is a
/// *sibling*, not nested under it (see [`common_dir`]).
///
/// A colocated jj repository also watches its `.git` directory (or resolved
/// gitlink). Git-only operations do not touch `.jj`; the `.git` event provides
/// the re-query signal even though jj imports that data only when the next jj
/// snapshot triggers auto-import.
///
/// Overlapping watches are harmless — the re-query+debounce coalesces duplicate
/// signals — but we drop a second dir whose normalized path equals the first, so
/// `notify` isn't asked to watch the same path twice.
fn state_dirs(kind: BackendKind, root: &Path) -> Result<Vec<PathBuf>> {
    let primary_state_dir = state_dir(kind, root)?;
    let mut dirs = vec![primary_state_dir.clone()];

    let mut add_git_dirs = |git_dir: PathBuf| {
        if !dirs.iter().any(|dir| normalize(dir) == normalize(&git_dir)) {
            dirs.push(git_dir.clone());
        }
        if let Some(shared) = common_dir(&git_dir)
            && !dirs.iter().any(|dir| normalize(dir) == normalize(&shared))
        {
            dirs.push(shared);
        }
    };

    match kind {
        BackendKind::Git => add_git_dirs(primary_state_dir),
        BackendKind::Jj if root.join(".git").exists() => {
            add_git_dirs(state_dir(BackendKind::Git, root)?)
        }
        _ => {}
    }
    Ok(dirs)
}

/// The directory to watch for a backend: `.jj` for jj, `.git` for git. A
/// worktree's `.git` is a gitlink *file* (`gitdir: <path>`); resolve it to the
/// real git directory. Best-effort — falls back to the `.git` path itself.
fn state_dir(kind: BackendKind, root: &Path) -> Result<PathBuf> {
    match kind {
        BackendKind::Jj => Ok(root.join(".jj")),
        BackendKind::Git => {
            let dot_git = root.join(".git");
            if dot_git.is_file() {
                let content = std::fs::read_to_string(&dot_git)?;
                if let Some(rest) = content.trim().strip_prefix("gitdir:") {
                    let p = PathBuf::from(rest.trim());
                    return Ok(if p.is_absolute() { p } else { root.join(p) });
                }
            }
            Ok(dot_git)
        }
        // `BackendKind` is `#[non_exhaustive]`; for an unknown future backend
        // watch the repo root itself — coarser, but it can't miss the state dir.
        _ => Ok(root.to_path_buf()),
    }
}

/// The **shared** git directory for a linked worktree, or `None` for a plain
/// repo. A linked worktree's resolved gitdir holds a `commondir` file whose
/// content is a path (typically relative, e.g. `../..`) to the shared `.git` —
/// where `refs/heads/*` and `packed-refs` live. We join it to the gitdir and
/// resolve `..` (lexically, matching the no-canonicalize style of [`state_dir`],
/// so the registered path stays plain rather than a Windows `\\?\` verbatim one).
/// A plain repo has no `commondir` file, so this is `None` and behaviour is
/// unchanged.
fn common_dir(state_dir: &Path) -> Option<PathBuf> {
    let commondir = state_dir.join("commondir");
    let content = std::fs::read_to_string(&commondir).ok()?;
    let rel = content.trim();
    if rel.is_empty() {
        return None;
    }
    let p = PathBuf::from(rel);
    let joined = if p.is_absolute() {
        p
    } else {
        state_dir.join(p)
    };
    Some(lexically_normalized(&joined))
}

/// Resolve `.`/`..` components without touching the filesystem, keeping the path
/// in its original (non-verbatim) form — `commondir`'s `../..` plus a Windows
/// gitdir would otherwise leave literal `..` segments in the watched path.
fn lexically_normalized(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                // Pop a real segment; keep a leading `..` that can't be resolved.
                if !out.pop() {
                    out.push(comp);
                }
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Canonicalize for comparison and strip the Windows verbatim prefix (`\\?\…`,
/// which `canonicalize` adds), so two spellings of the same dir dedup. Mirrors
/// `vcs-core`'s path-compare normalization; falls back to the input when the path
/// can't be canonicalized (then equal paths still compare equal byte-for-byte).
fn normalize(p: &Path) -> PathBuf {
    let canonical = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    #[cfg(windows)]
    {
        let s = canonical.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\")
            && !rest.starts_with("UNC\\")
        {
            return PathBuf::from(rest.to_string());
        }
    }
    canonical
}

#[cfg(test)]
mod tests {
    use super::*;
    use vcs_testkit::TempDir;

    // A plain (non-worktree) git dir has no `commondir` file → no shared dir, so
    // behaviour is exactly today's single-dir watch.
    #[test]
    fn no_commondir_file_yields_none() {
        let scratch = TempDir::new("watch-commondir");
        let git_dir = scratch.path().join(".git");
        std::fs::create_dir_all(&git_dir).expect("mkdir .git");
        assert_eq!(common_dir(&git_dir), None);
    }

    // A linked-worktree layout: the private gitdir holds `commondir` = `../..`
    // (git's actual content), which must resolve to the sibling shared `.git`.
    #[test]
    fn relative_commondir_resolves_to_shared_git_dir() {
        let scratch = TempDir::new("watch-commondir");
        let shared = scratch.path().join(".git");
        let private = shared.join("worktrees").join("wt");
        std::fs::create_dir_all(&private).expect("mkdir private gitdir");
        // git writes `../..` (relative to the private dir) here.
        std::fs::write(private.join("commondir"), "../..\n").expect("write commondir");

        let resolved = common_dir(&private).expect("Some(shared dir)");
        // `<shared>/worktrees/wt` + `../..` == `<shared>` (lexically, no `..` left).
        assert_eq!(resolved, lexically_normalized(&shared));
        assert!(
            !resolved.to_string_lossy().contains(".."),
            "the `..` segments must be resolved, got {}",
            resolved.display()
        );
    }

    // An absolute `commondir` (git permits it) is taken as-is.
    #[test]
    fn absolute_commondir_is_used_verbatim() {
        let scratch = TempDir::new("watch-commondir");
        let shared = scratch.path().join("shared-git");
        let private = scratch.path().join("private");
        std::fs::create_dir_all(&private).expect("mkdir private");
        std::fs::write(private.join("commondir"), format!("{}\n", shared.display()))
            .expect("write commondir");

        assert_eq!(common_dir(&private), Some(lexically_normalized(&shared)));
    }

    // `state_dirs` returns both the private and shared dirs for a worktree, and
    // the shared dir is not the private one (so two distinct watches register).
    #[test]
    fn state_dirs_includes_private_and_shared_for_worktree() {
        let scratch = TempDir::new("watch-commondir");
        let root = scratch.path().join("wt-worktree");
        let shared = scratch.path().join(".git");
        let private = shared.join("worktrees").join("wt");
        std::fs::create_dir_all(&private).expect("mkdir private gitdir");
        std::fs::create_dir_all(&root).expect("mkdir worktree root");
        std::fs::write(private.join("commondir"), "../..\n").expect("write commondir");
        // The worktree's `.git` gitlink file points at the private dir.
        std::fs::write(
            root.join(".git"),
            format!("gitdir: {}\n", private.display()),
        )
        .expect("write gitlink");

        let dirs = state_dirs(BackendKind::Git, &root).expect("state_dirs");
        assert_eq!(dirs.len(), 2, "private + shared, got {dirs:?}");
        assert_eq!(normalize(&dirs[0]), normalize(&private));
        assert_eq!(normalize(&dirs[1]), normalize(&shared));
    }

    #[test]
    fn state_dirs_includes_git_dir_for_colocated_jj_repo() {
        let scratch = TempDir::new("watch-commondir");
        let root = scratch.path().join("colocated");
        std::fs::create_dir_all(root.join(".jj")).expect("mkdir .jj");
        std::fs::create_dir_all(root.join(".git")).expect("mkdir .git");

        let dirs = state_dirs(BackendKind::Jj, &root).expect("state_dirs");
        assert_eq!(dirs, vec![root.join(".jj"), root.join(".git")]);
    }

    #[test]
    fn state_dirs_excludes_missing_git_dir_for_pure_jj_repo() {
        let scratch = TempDir::new("watch-commondir");
        let root = scratch.path().join("pure-jj");
        std::fs::create_dir_all(root.join(".jj")).expect("mkdir .jj");

        let dirs = state_dirs(BackendKind::Jj, &root).expect("state_dirs");
        assert_eq!(dirs, vec![root.join(".jj")]);
    }

    // When `commondir` resolves back to the state dir itself (degenerate), the
    // duplicate is dropped — we never register the same path twice.
    #[test]
    fn self_referential_commondir_is_deduped() {
        let scratch = TempDir::new("watch-commondir");
        let git_dir = scratch.path().join(".git");
        std::fs::create_dir_all(&git_dir).expect("mkdir .git");
        // `.` resolves to the dir itself.
        std::fs::write(git_dir.join("commondir"), ".\n").expect("write commondir");
        // The gitlink points the worktree root at this very dir.
        let root = scratch.path().join("root");
        std::fs::create_dir_all(&root).expect("mkdir root");
        std::fs::write(
            root.join(".git"),
            format!("gitdir: {}\n", git_dir.display()),
        )
        .expect("write gitlink");

        let dirs = state_dirs(BackendKind::Git, &root).expect("state_dirs");
        assert_eq!(dirs.len(), 1, "self-reference deduped, got {dirs:?}");
    }

    // R3: verify the `watch_errors` counter→`snapshot()` plumbing (the notify
    // callback that calls `note_watch_error` on a backend `Err` can't be driven from
    // a unit test, so this pins the counter is wired in and stays independent).
    #[test]
    fn stats_counts_watch_errors_independently() {
        let stats = StatsInner::default();
        assert_eq!(stats.snapshot().watch_errors, 0);
        stats.note_watch_error();
        stats.note_watch_error();
        let snap = stats.snapshot();
        assert_eq!(snap.watch_errors, 2, "watch errors counted");
        assert_eq!(
            (snap.requeries, snap.changes, snap.skipped),
            (0, 0, 0),
            "other counters unaffected"
        );
        assert_eq!(
            (snap.retries, snap.recoveries, snap.terminal_failures),
            (0, 0, 0),
            "retry lifecycle counters unaffected"
        );
        assert!(snap.last_error.is_none());
    }
}

/// Hermetic tests of the debounce → ceiling → re-query → diff pipeline itself:
/// `watch_loop` is driven directly with a fake signal channel, a
/// `ScriptedRunner`-backed `Repo`, and a **paused tokio clock** — no real
/// filesystem watch, no real process, no real sleeps. These pin the *loop's*
/// timing contract; the notify→signal bridge stays covered by the `#[ignore]`
/// integration tests (fake time says nothing about real OS event batching).
#[cfg(test)]
mod pipeline_tests {
    use super::*;
    use vcs_testkit::TempDir;
    use processkit::ProcessRunner;
    use processkit::testing::{Reply, ScriptedRunner};
    use vcs_core::Repo;
    use vcs_core::vcs_git::Git;

    /// Porcelain-v2 (NUL-separated) status output for a repo at `head`, clean.
    fn v2(head: &str) -> String {
        format!("# branch.oid {head}\0# branch.head main\0")
    }

    /// The exact command set one snapshot+branches re-query issues, scripted:
    /// `status --porcelain=v2`, the `rev-parse --git-dir` probe (must point at a
    /// real dir — the op-state probe reads `MERGE_HEAD` off the filesystem), and
    /// `branch --no-column`.
    fn scripted(gitdir: &Path, head: &str) -> ScriptedRunner {
        ScriptedRunner::new()
            .on(["git", "status"], Reply::ok(v2(head)))
            .on(
                ["git", "rev-parse"],
                Reply::ok(format!("{}\n", gitdir.display())),
            )
            .on(["git", "branch"], Reply::ok("* main\n"))
    }

    fn scripted_repo(gitdir: &Path, head: &str) -> Box<dyn VcsRepo> {
        Box::new(Repo::from_git(
            "/r",
            "/r",
            Git::with_runner(scripted(gitdir, head)),
        ))
    }

    /// The baseline `prev` state the loop diffs against, taken through the same
    /// snapshot path `Builder::build` uses.
    async fn baseline(gitdir: &Path, head: &str) -> event::WatchState {
        let repo = scripted_repo(gitdir, head);
        let snap = repo.snapshot().await.expect("baseline snapshot");
        let branches = repo.local_branches().await.expect("baseline branches");
        event::WatchState::from_snapshot(&snap, branches)
    }

    fn defaults() -> LoopConfig {
        LoopConfig {
            debounce: Duration::from_millis(250),
            max_wait: Duration::from_secs(1),
            requery_timeout: Some(Duration::from_secs(30)),
            // The hermetic pipeline drives a git-backed scripted repo, where
            // read-only and snapshotting re-queries issue the same commands; the
            // default (read-only) mirrors production.
            snapshot_working_copy: false,
            output_capacity: 64,
            retry_limit: REQUERY_RETRY_LIMIT,
            retry_backoff: REQUERY_RETRY_BACKOFF,
        }
    }

    struct Harness {
        sig: mpsc::Sender<WatchSignal>,
        out: mpsc::Receiver<RepoChange>,
        stats: Arc<StatsInner>,
        watch_failed: Arc<AtomicBool>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Harness {
        // Mirror the production notify callback: fire-and-forget `try_send` on the
        // capacity-1 bridge (a pending signal coalesces the next one). `Err` (full or
        // loop-ended) is intentionally ignored — a still-pending signal already
        // triggers the re-query the caller wants.
        fn signal(&self) {
            let _ = self.sig.try_send(WatchSignal::Change);
        }

        fn backend_failed(&self) {
            self.stats.note_watch_error();
            if !self.watch_failed.swap(true, Ordering::AcqRel) {
                self.stats.note_terminal_failure();
            }
            let _ = self.sig.try_send(WatchSignal::BackendFailed);
        }
    }

    fn spawn_loop(repo: Box<dyn VcsRepo>, prev: event::WatchState, config: LoopConfig) -> Harness {
        let (sig, raw_rx) = mpsc::channel(1);
        let (out_tx, out) = mpsc::channel(config.output_capacity);
        let stats = Arc::new(StatsInner::default());
        let watch_failed = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(watch_loop(
            repo,
            raw_rx,
            out_tx,
            prev,
            config,
            Arc::clone(&stats),
            Arc::clone(&watch_failed),
        ));
        Harness {
            sig,
            out,
            stats,
            watch_failed,
            task,
        }
    }

    /// Let the loop task run to a quiescent point without advancing time —
    /// paused-clock auto-advance only triggers when every task idles on a timer,
    /// so a bounded yield burst (never a spin-until loop) is the safe way to let
    /// an already-runnable re-query complete.
    async fn settle() {
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
    }

    // A burst of sub-debounce signals coalesces into exactly one re-query and
    // one emitted change.
    #[tokio::test(start_paused = true)]
    async fn debounce_coalesces_burst() {
        let scratch = TempDir::new("watch-commondir");
        let prev = baseline(scratch.path(), "aaa").await;
        let mut h = spawn_loop(scripted_repo(scratch.path(), "bbb"), prev, defaults());

        for _ in 0..5 {
            h.signal();
            tokio::time::advance(Duration::from_millis(10)).await;
        }
        let change = h.out.recv().await.expect("one coalesced change");
        assert!(
            change
                .events
                .iter()
                .any(|e| matches!(e, RepoEvent::HeadMoved { .. })),
            "expected HeadMoved, got {:?}",
            change.events
        );

        // Long quiet: nothing else arrives, and exactly one re-query ran.
        tokio::time::advance(Duration::from_secs(5)).await;
        settle().await;
        assert!(
            h.out.try_recv().is_err(),
            "burst must coalesce to one change"
        );
        let stats = h.stats.snapshot();
        assert_eq!((stats.requeries, stats.changes), (1, 1));
    }

    // Signals arriving faster than the quiet window forever: the `max_wait`
    // ceiling still forces a re-query at its cadence (the dedicated
    // `sleep_until` arm — not just "on the next signal after the deadline").
    #[tokio::test(start_paused = true)]
    async fn max_wait_caps_continuous_signals() {
        let scratch = TempDir::new("watch-commondir");
        let prev = baseline(scratch.path(), "aaa").await;
        let h_config = defaults();
        let mut h = spawn_loop(scripted_repo(scratch.path(), "bbb"), prev, h_config);

        // A pump that fires a signal every 100 ms — always inside the 250 ms
        // quiet window, so only the ceiling can break the burst.
        let pump_sig = h.sig.clone();
        let pump = tokio::spawn(async move {
            loop {
                // `try_send` mirrors the notify callback. `Full` means our previous
                // signal is still pending (coalesced) — keep pumping; `Closed` means
                // the loop ended — stop.
                if let Err(mpsc::error::TrySendError::Closed(WatchSignal::Change)) =
                    pump_sig.try_send(WatchSignal::Change)
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });

        let change = tokio::time::timeout(Duration::from_secs(2), h.out.recv())
            .await
            .expect("the ceiling must fire within max_wait")
            .expect("change");
        assert!(
            change
                .events
                .iter()
                .any(|e| matches!(e, RepoEvent::HeadMoved { .. })),
            "got {:?}",
            change.events
        );
        pump.abort();
    }

    // P1: a caller "disabling the ceiling" with `Duration::MAX` must not overflow
    // the `Instant + max_wait` deadline and panic the spawned loop (which would
    // drop the output channel and kill the watcher silently). The clamp keeps it
    // running; the debounce timer still fires normally.
    #[tokio::test(start_paused = true)]
    async fn max_wait_duration_max_does_not_panic_the_loop() {
        let scratch = TempDir::new("watch-commondir");
        let prev = baseline(scratch.path(), "aaa").await;
        let config = LoopConfig {
            max_wait: Duration::MAX,
            ..defaults()
        };
        let mut h = spawn_loop(scripted_repo(scratch.path(), "bbb"), prev, config);
        h.signal();
        tokio::time::advance(Duration::from_millis(300)).await; // past the 250 ms debounce
        let change = h
            .out
            .recv()
            .await
            .expect("the loop survives a Duration::MAX max_wait and still re-queries");
        assert!(!change.events.is_empty(), "got {:?}", change.events);
    }

    // The base case: one signal, a quiet gap, one re-query.
    #[tokio::test(start_paused = true)]
    async fn quiet_gap_triggers_requery() {
        let scratch = TempDir::new("watch-commondir");
        let prev = baseline(scratch.path(), "aaa").await;
        let mut h = spawn_loop(scripted_repo(scratch.path(), "bbb"), prev, defaults());

        h.signal();
        let change = h.out.recv().await.expect("change after the quiet gap");
        assert!(
            change
                .events
                .iter()
                .any(|e| matches!(e, RepoEvent::HeadMoved { .. }))
        );
    }

    // A re-query that finds the same state emits nothing — but it *ran* (the
    // stats distinguish "no change" from "never re-queried").
    #[tokio::test(start_paused = true)]
    async fn no_change_yields_no_emission() {
        let scratch = TempDir::new("watch-commondir");
        let prev = baseline(scratch.path(), "aaa").await;
        // Same head as the baseline → empty diff.
        let mut h = spawn_loop(scripted_repo(scratch.path(), "aaa"), prev, defaults());

        h.signal();
        settle().await; // let the loop register its quiet timer first
        tokio::time::advance(Duration::from_millis(300)).await; // past debounce
        settle().await; // let the re-query run

        let stats = h.stats.snapshot();
        assert_eq!((stats.requeries, stats.changes, stats.skipped), (1, 0, 0));
        assert!(
            h.out.try_recv().is_err(),
            "no events for an unchanged state"
        );
    }

    /// Fails the first `status` call (a transiently held lock), then behaves —
    /// `ScriptedRunner` rules are stateless, so the two-phase behaviour needs a
    /// tiny stateful runner delegating to throwaway scripted ones.
    struct FlakyStatus {
        fails_left: AtomicU64,
        gitdir: PathBuf,
        head: &'static str,
    }

    #[async_trait::async_trait]
    impl ProcessRunner for FlakyStatus {
        async fn output_string(
            &self,
            command: &processkit::Command,
        ) -> processkit::Result<processkit::ProcessResult<String>> {
            let is_status = command.arguments().first().map(|a| a == "status") == Some(true);
            if is_status && self.fails_left.load(Ordering::Relaxed) > 0 {
                self.fails_left.fetch_sub(1, Ordering::Relaxed);
                return Err(processkit::Error::exit(
                    "git",
                    128,
                    "",
                    "fatal: Unable to create '.git/index.lock'",
                ));
            }
            scripted(&self.gitdir, self.head)
                .output_string(command)
                .await
        }
    }

    // A transient re-query failure is skipped (counted, no emission); the next
    // signal re-checks and recovers.
    #[tokio::test(start_paused = true)]
    async fn transient_failure_skips_then_recovers() {
        let scratch = TempDir::new("watch-commondir");
        let prev = baseline(scratch.path(), "aaa").await;
        let repo = Box::new(Repo::from_git(
            "/r",
            "/r",
            Git::with_runner(FlakyStatus {
                fails_left: AtomicU64::new(1),
                gitdir: scratch.path().to_path_buf(),
                head: "bbb",
            }),
        ));
        let mut h = spawn_loop(repo, prev, defaults());

        // First attempt: the snapshot fails → skip, nothing emitted.
        h.signal();
        settle().await; // loop registers the quiet timer
        tokio::time::advance(Duration::from_millis(300)).await;
        settle().await; // the (failing) re-query runs
        let stats = h.stats.snapshot();
        assert_eq!((stats.requeries, stats.skipped, stats.changes), (1, 1, 0));
        assert_eq!(stats.last_error, Some(WatcherErrorKind::Snapshot));
        assert!(h.out.try_recv().is_err());

        // Second signal: the lock "cleared" — the re-query recovers and emits.
        h.signal();
        let change = h.out.recv().await.expect("recovered change");
        assert!(
            change
                .events
                .iter()
                .any(|e| matches!(e, RepoEvent::HeadMoved { .. }))
        );
        let stats = h.stats.snapshot();
        assert_eq!((stats.requeries, stats.changes), (2, 1));
    }

    /// Delays every reply by `delay` (virtual time — `tokio::time::sleep`, NOT a
    /// thread sleep, so the paused clock controls it). `ScriptedRunner` replies
    /// instantly, so this is the only way to exercise the `requery_timeout`
    /// wrapper — a scripted `Reply::timeout()` resolves immediately and would
    /// test the *error* path, not the deadline.
    struct Sleepy {
        delay: Duration,
        gitdir: PathBuf,
        head: &'static str,
    }

    #[async_trait::async_trait]
    impl ProcessRunner for Sleepy {
        async fn output_string(
            &self,
            command: &processkit::Command,
        ) -> processkit::Result<processkit::ProcessResult<String>> {
            tokio::time::sleep(self.delay).await;
            scripted(&self.gitdir, self.head)
                .output_string(command)
                .await
        }
    }

    /// Only the first status query is slow. Its timeout drops the sleeping
    /// future after the counter has advanced, so the automatic retry succeeds.
    struct SlowFirstStatus {
        slow_left: AtomicBool,
        delay: Duration,
        gitdir: PathBuf,
        head: &'static str,
    }

    #[async_trait::async_trait]
    impl ProcessRunner for SlowFirstStatus {
        async fn output_string(
            &self,
            command: &processkit::Command,
        ) -> processkit::Result<processkit::ProcessResult<String>> {
            let is_status = command.arguments().first().map(|a| a == "status") == Some(true);
            if is_status && self.slow_left.swap(false, Ordering::Relaxed) {
                tokio::time::sleep(self.delay).await;
            }
            scripted(&self.gitdir, self.head)
                .output_string(command)
                .await
        }
    }

    // A timeout on the final filesystem signal schedules its own retry. No new
    // signal is needed to observe the state that the timed-out query missed.
    #[tokio::test(start_paused = true)]
    async fn timeout_on_last_signal_recovers_via_backoff_retry() {
        let scratch = TempDir::new("watch-commondir");
        let prev = baseline(scratch.path(), "aaa").await;
        let repo = Box::new(Repo::from_git(
            "/r",
            "/r",
            Git::with_runner(SlowFirstStatus {
                slow_left: AtomicBool::new(true),
                delay: Duration::from_secs(10),
                gitdir: scratch.path().to_path_buf(),
                head: "bbb",
            }),
        ));
        let config = LoopConfig {
            requery_timeout: Some(Duration::from_secs(5)),
            retry_backoff: Duration::from_secs(1),
            ..defaults()
        };
        let mut h = spawn_loop(repo, prev, config);

        h.signal();
        settle().await;
        tokio::time::advance(Duration::from_millis(300)).await;
        settle().await;
        tokio::time::advance(Duration::from_secs(5)).await;
        settle().await;
        assert_eq!(
            (h.stats.snapshot().requeries, h.stats.snapshot().retries),
            (1, 1)
        );
        assert!(h.out.try_recv().is_err());

        // Only virtual time advances here: there is deliberately no h.signal().
        tokio::time::advance(Duration::from_secs(1)).await;
        settle().await;
        let change = h.out.try_recv().expect("retry emits the missed change");
        assert!(
            change
                .events
                .iter()
                .any(|e| matches!(e, RepoEvent::HeadMoved { .. }))
        );
        let stats = h.stats.snapshot();
        assert_eq!((stats.requeries, stats.skipped, stats.retries), (2, 1, 1));
        assert_eq!((stats.recoveries, stats.changes), (1, 1));
    }

    // Retry exhaustion is bounded and then becomes idle until another event.
    #[tokio::test(start_paused = true)]
    async fn persistent_requery_failure_exhausts_retries_without_busy_loop() {
        let scratch = TempDir::new("watch-commondir");
        let prev = baseline(scratch.path(), "aaa").await;
        let repo = Box::new(Repo::from_git(
            "/r",
            "/r",
            Git::with_runner(FlakyStatus {
                fails_left: AtomicU64::new(100),
                gitdir: scratch.path().to_path_buf(),
                head: "bbb",
            }),
        ));
        let config = LoopConfig {
            retry_limit: 2,
            retry_backoff: Duration::from_millis(100),
            ..defaults()
        };
        let h = spawn_loop(repo, prev, config);

        h.signal();
        settle().await;
        tokio::time::advance(Duration::from_millis(300)).await;
        settle().await;
        tokio::time::advance(Duration::from_millis(100)).await;
        settle().await;
        tokio::time::advance(Duration::from_millis(200)).await;
        settle().await;
        let stats = h.stats.snapshot();
        assert_eq!((stats.requeries, stats.skipped, stats.retries), (3, 3, 2));
        assert_eq!(stats.recoveries, 0);

        tokio::time::advance(Duration::from_secs(60 * 60)).await;
        settle().await;
        assert_eq!(
            h.stats.snapshot().requeries,
            3,
            "exhaustion must park on the signal receiver"
        );
    }

    // Closing the producer while parked in backoff cancels the retry promptly.
    #[tokio::test(start_paused = true)]
    async fn drop_teardown_during_retry_backoff() {
        let scratch = TempDir::new("watch-commondir");
        let prev = baseline(scratch.path(), "aaa").await;
        let repo = Box::new(Repo::from_git(
            "/r",
            "/r",
            Git::with_runner(FlakyStatus {
                fails_left: AtomicU64::new(1),
                gitdir: scratch.path().to_path_buf(),
                head: "bbb",
            }),
        ));
        let config = LoopConfig {
            retry_backoff: Duration::from_secs(60 * 60),
            ..defaults()
        };
        let Harness {
            sig,
            mut out,
            stats,
            watch_failed: _,
            task,
        } = spawn_loop(repo, prev, config);

        sig.try_send(WatchSignal::Change).expect("send");
        settle().await;
        tokio::time::advance(Duration::from_millis(300)).await;
        settle().await;
        assert_eq!((stats.snapshot().skipped, stats.snapshot().retries), (1, 1));
        drop(sig);
        task.await.expect("loop exits while retry timer is pending");
        assert!(out.recv().await.is_none());
    }

    // A notify backend death is terminal through the primary API: recv/Stream
    // observes channel closure, without requiring separate stats polling.
    #[tokio::test(start_paused = true)]
    async fn permanent_backend_failure_closes_main_channel() {
        let scratch = TempDir::new("watch-commondir");
        let prev = baseline(scratch.path(), "aaa").await;
        let mut h = spawn_loop(scripted_repo(scratch.path(), "bbb"), prev, defaults());

        h.backend_failed();
        assert!(h.out.recv().await.is_none(), "backend death closes recv");
        let stats = h.stats.snapshot();
        assert_eq!((stats.watch_errors, stats.terminal_failures), (1, 1));
        assert_eq!((stats.retries, stats.recoveries), (0, 0));
    }

    // A re-query exceeding the configured deadline is killed and skipped as
    // transient; the loop survives (a later attempt runs and is also bounded).
    #[tokio::test(start_paused = true)]
    async fn requery_timeout_skips_as_transient() {
        let scratch = TempDir::new("watch-commondir");
        let prev = baseline(scratch.path(), "aaa").await;
        let repo = Box::new(Repo::from_git(
            "/r",
            "/r",
            Git::with_runner(Sleepy {
                delay: Duration::from_secs(10),
                gitdir: scratch.path().to_path_buf(),
                head: "bbb",
            }),
        ));
        let config = LoopConfig {
            requery_timeout: Some(Duration::from_secs(5)),
            ..defaults()
        };
        let mut h = spawn_loop(repo, prev, config);

        h.signal();
        settle().await; // loop registers the quiet timer
        tokio::time::advance(Duration::from_millis(300)).await; // debounce
        settle().await; // re-query starts; Sleepy + the deadline register timers
        tokio::time::advance(Duration::from_secs(6)).await; // past the deadline
        settle().await;
        let stats = h.stats.snapshot();
        assert_eq!((stats.requeries, stats.skipped, stats.changes), (1, 1, 0));
        assert_eq!(stats.last_error, Some(WatcherErrorKind::Timeout));
        assert!(h.out.try_recv().is_err());

        // The loop is alive: a second attempt runs (and times out the same way).
        h.signal();
        settle().await;
        tokio::time::advance(Duration::from_millis(300)).await;
        settle().await;
        tokio::time::advance(Duration::from_secs(6)).await;
        settle().await;
        assert_eq!(h.stats.snapshot().requeries, 2);
    }

    // R4: the startup baseline honors `requery_timeout` — a snapshot that wedges (a
    // `Sleepy` repo far past the deadline) errors with `TimedOut` instead of hanging
    // `build()` forever. Exercises `capture_baseline` directly (the `build()` path is
    // only reachable with a real notify watcher).
    #[tokio::test(start_paused = true)]
    async fn baseline_capture_honors_requery_timeout() {
        let scratch = TempDir::new("watch-commondir");
        let repo = Repo::from_git(
            "/r",
            "/r",
            Git::with_runner(Sleepy {
                delay: Duration::from_secs(10),
                gitdir: scratch.path().to_path_buf(),
                head: "bbb",
            }),
        );
        let err = capture_baseline(&repo, Some(Duration::from_secs(5)), false)
            .await
            .expect_err("a wedged baseline must time out, not hang");
        assert!(
            matches!(&err, Error::Io(e) if e.kind() == std::io::ErrorKind::TimedOut),
            "expected an Io TimedOut, got {err:?}"
        );
        // A wedged baseline is retryable — `build()` agrees with the loop's transient
        // treatment of a re-query timeout.
        assert!(err.is_transient(), "a baseline timeout is transient");

        // With no deadline the same query completes (Sleepy still returns, just late);
        // advancing the clock lets it finish so we prove the timeout — not the repo —
        // is what produced the error above.
        let ok = capture_baseline(&repo, None, false).await;
        assert!(ok.is_ok(), "an unbounded baseline still succeeds: {ok:?}");
    }

    // Closing the signal channel mid-debounce ends the loop promptly and closes
    // the output channel.
    #[tokio::test(start_paused = true)]
    async fn drop_teardown_mid_debounce() {
        let scratch = TempDir::new("watch-commondir");
        let prev = baseline(scratch.path(), "aaa").await;
        let Harness {
            sig,
            mut out,
            stats: _,
            watch_failed: _,
            task,
        } = spawn_loop(scripted_repo(scratch.path(), "bbb"), prev, defaults());

        sig.try_send(WatchSignal::Change).expect("send");
        tokio::time::advance(Duration::from_millis(100)).await; // mid-debounce
        drop(sig);

        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("loop ends promptly")
            .expect("loop task joins cleanly");
        assert!(out.recv().await.is_none(), "output closes with the loop");
    }

    /// Reports a different head on every `status` call, so every re-query
    /// produces a `HeadMoved` — the emission generator the backpressure test
    /// needs to fill the bounded output channel.
    struct VaryingHead {
        statuses: AtomicU64,
        gitdir: PathBuf,
    }

    #[async_trait::async_trait]
    impl ProcessRunner for VaryingHead {
        async fn output_string(
            &self,
            command: &processkit::Command,
        ) -> processkit::Result<processkit::ProcessResult<String>> {
            let is_status = command.arguments().first().map(|a| a == "status") == Some(true);
            let n = if is_status {
                self.statuses.fetch_add(1, Ordering::Relaxed)
            } else {
                self.statuses.load(Ordering::Relaxed)
            };
            scripted(&self.gitdir, &format!("h{n}"))
                .output_string(command)
                .await
        }
    }

    // A full output channel parks the loop at `send` (backpressure) instead of
    // dropping or buffering unboundedly; draining one item unparks it.
    #[tokio::test(start_paused = true)]
    async fn backpressure_parks_loop() {
        let scratch = TempDir::new("watch-commondir");
        let prev = baseline(scratch.path(), "base").await;
        let repo = Box::new(Repo::from_git(
            "/r",
            "/r",
            Git::with_runner(VaryingHead {
                statuses: AtomicU64::new(0),
                gitdir: scratch.path().to_path_buf(),
            }),
        ));
        let config = LoopConfig {
            output_capacity: 1,
            ..defaults()
        };
        let mut h = spawn_loop(repo, prev, config);

        // First change fills the capacity-1 channel.
        h.signal();
        settle().await; // loop registers the quiet timer
        tokio::time::advance(Duration::from_millis(300)).await;
        settle().await; // re-query runs; emission 1 fills the channel
        // Second re-query produces another change; the send parks (channel full):
        // the re-query ran but the emission hasn't landed.
        h.signal();
        settle().await;
        tokio::time::advance(Duration::from_millis(300)).await;
        settle().await;
        let stats = h.stats.snapshot();
        assert_eq!(
            (stats.requeries, stats.changes),
            (2, 1),
            "second emission must be parked on the full channel"
        );

        // Draining unparks the loop; both changes arrive in order.
        let first = h.out.recv().await.expect("first change");
        assert!(
            first
                .events
                .iter()
                .any(|e| matches!(e, RepoEvent::HeadMoved { .. }))
        );
        let second = h.out.recv().await.expect("second change");
        assert!(
            second
                .events
                .iter()
                .any(|e| matches!(e, RepoEvent::HeadMoved { .. }))
        );
        settle().await;
        assert_eq!(h.stats.snapshot().changes, 2);
    }

    // The `stream` feature: `StreamExt::next` on the REAL `RepoWatcher` yields
    // what `recv` would and advances `current()` identically. The watcher is
    // assembled directly (same crate) around the loop harness's channel, with an
    // idle notify watcher standing in for the OS watch.
    #[cfg(feature = "stream")]
    #[tokio::test(start_paused = true)]
    async fn stream_yields_changes_and_advances_current() {
        use tokio_stream::StreamExt;

        let scratch = TempDir::new("watch-commondir");
        let prev = baseline(scratch.path(), "aaa").await;
        let h = spawn_loop(scripted_repo(scratch.path(), "bbb"), prev, defaults());

        let baseline_snap = scripted_repo(scratch.path(), "aaa")
            .snapshot()
            .await
            .expect("baseline snapshot");
        let mut watcher = RepoWatcher {
            rx: h.out,
            current: baseline_snap,
            stats: h.stats,
            _watcher: notify::recommended_watcher(|_res| {}).expect("idle watcher"),
            task: h.task,
        };
        assert_eq!(watcher.current().head.as_deref(), Some("aaa"));

        // `h` is partially moved into `watcher` above, so reach the remaining `sig`
        // field directly rather than through the `h.signal()` method (which would
        // borrow all of `h`).
        let _ = h.sig.try_send(WatchSignal::Change);
        let change = watcher.next().await.expect("stream item");
        assert!(
            change
                .events
                .iter()
                .any(|e| matches!(e, RepoEvent::HeadMoved { .. })),
            "got {:?}",
            change.events
        );
        // Polling through the Stream advanced `current()` exactly like `recv`.
        assert_eq!(watcher.current().head.as_deref(), Some("bbb"));
    }
}

// Long-form how-to guides, rendered from this crate's docs/*.md on docs.rs.
#[doc = include_str!("../docs/watch.md")]
#[allow(rustdoc::broken_intra_doc_links)]
pub mod guide {}
