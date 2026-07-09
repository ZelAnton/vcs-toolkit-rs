#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-jj` — automate Jujutsu (`jj`) from Rust by driving the `jj` CLI.
//!
//! You call typed `async` methods; `vcs-jj` runs the real `jj`, parses its
//! templated output, and hands you structured values — so you get *jj's own*
//! behaviour and config, not a reimplementation of the operation log or backend.
//! Async, structured errors, mockable. Every command runs inside an OS **job** (an
//! OS-level container that kills the whole process tree if your program exits, via
//! [`processkit`]) so a `jj` subprocess is never orphaned, with an optional
//! per-client [timeout](Jj::default_timeout).
//!
//! # What you can do
//!
//! Working-copy status & the change log · describe / new change · bookmarks · the
//! operation log (restore / undo — jj's safety net) · workspaces · squash / split /
//! absorb / duplicate / abandon · diff & template queries · git sync (fetch / push
//! / clone / import) · parse & resolve jj's native conflict markers · transactions
//! that roll the op log back on error. One tiny call to start:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_jj::{Jj, JjApi};
//! # async fn demo() -> Result<(), processkit::Error> {
//! let jj = Jj::new();
//! // the working-copy change `@`:
//! println!("{}", jj.current_change(Path::new(".")).await?.change_id);
//! # Ok(()) }
//! ```
//!
//! # The surface (engineering reference)
//!
//! - **[`JjApi`]** — the object-safe trait every operation lives on. Depend on
//!   `&dyn JjApi` (or generically on `impl JjApi`) so a test can swap the real
//!   client for a double. Most methods take the working directory as the first
//!   argument and return typed results ([`Change`], [`Bookmark`],
//!   [`BookmarkRef`], [`Operation`], [`Workspace`], [`ChangedPath`],
//!   [`FileDiff`], [`AnnotationLine`], …) or a structured [`Error`]. The groups:
//!   changes ([`status`](JjApi::status), [`log`](JjApi::log),
//!   [`describe`](JjApi::describe), [`new_change`](JjApi::new_change)),
//!   bookmarks ([`bookmarks`](JjApi::bookmarks),
//!   [`bookmark_create`](JjApi::bookmark_create),
//!   [`bookmark_move`](JjApi::bookmark_move), …), the operation log
//!   ([`op_log`](JjApi::op_log), [`op_head`](JjApi::op_head),
//!   [`op_restore`](JjApi::op_restore), [`op_undo`](JjApi::op_undo)),
//!   diff/query ([`diff`](JjApi::diff), [`diff_stat`](JjApi::diff_stat),
//!   [`evolog`](JjApi::evolog), [`file_annotate`](JjApi::file_annotate),
//!   [`template_query`](JjApi::template_query)), mutations
//!   ([`rebase`](JjApi::rebase), [`squash_paths`](JjApi::squash_paths),
//!   [`split_paths`](JjApi::split_paths), [`absorb`](JjApi::absorb),
//!   [`abandon`](JjApi::abandon)), git sync
//!   ([`git_fetch`](JjApi::git_fetch), [`git_push`](JjApi::git_push),
//!   [`git_clone`](JjApi::git_clone), [`git_import`](JjApi::git_import)), and
//!   workspaces ([`workspace_list`](JjApi::workspace_list),
//!   [`workspace_root`](JjApi::workspace_root),
//!   [`workspace_add`](JjApi::workspace_add)).
//! - **[`Jj`]** — the real client. [`Jj::new`] uses the job-backed runner;
//!   [`Jj::with_runner`] injects a fake one for tests. It is generic over the
//!   [`ProcessRunner`] seam, defaulting to the production runner.
//! - **[`JjAt`]** — a cwd-bound view ([`Jj::at`]) whose methods drop the leading
//!   `dir`, so `jj.at(dir).status()` reads as `jj.status(dir)` — handy when one
//!   client drives one checkout.
//! - **[`Jj::transaction`]** — run a mutation sequence with op-log rollback:
//!   capture the current operation, run a closure, and on `Err` restore the repo
//!   to it. The op log is jj's safety net; this wraps it as a scope.
//!   [`Jj::workspace_roots`] is a sibling inherent method — a bounded fan-out
//!   resolving many workspace roots at once.
//! - **Builder specs** for the multi-option commands — [`WorkspaceAdd`],
//!   [`SquashPaths`], [`BookmarkMove`], [`SquashInto`], [`GitClone`] — each
//!   `#[non_exhaustive]`, built with a constructor +
//!   chained setters, named after the flags they emit. [`JjFileset`] wraps a
//!   workspace-root-relative path as an exact-path `root-file:"…"` fileset;
//!   [`RevsetExpr`] is an optional up-front-validated revset newtype for untrusted input.
//! - **[`conflict`]** — a typed model of jj's *native* conflict markers (the
//!   `diff`/`snapshot` styles): parse a materialized file into structured
//!   regions, re-render byte-exact, and resolve to a chosen side. (Files
//!   materialized in the `git` style are parsed by `vcs_git::conflict` instead.)
//! - **[`capabilities`](JjApi::capabilities)** — probe the installed binary's
//!   version against this crate's validated floor (jj ≥ 0.38); see
//!   [`JjCapabilities`].
//!
//! There is deliberately **no `Jj::hardened()`** counterpart to vcs-git's
//! untrusted-repo profile: jj has no repo-local hooks, and its config comes from
//! the user/repo TOML files jj itself trusts. In a *colocated* repo the risk
//! lives on the git side — git hooks fire when **git** commands run there, so
//! harden the `Git` client you point at it.
//!
//! # Recipes
//!
//! Read state — depend on the trait so the same code takes a real client or a mock:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_jj::{Jj, JjApi};
//! # async fn demo() -> Result<(), processkit::Error> {
//! let jj = Jj::new();
//! let dir = Path::new(".");
//! let current = jj.current_change(dir).await?;       // the working-copy change `@`
//! let dirty = !jj.status(dir).await?.is_empty();     // any working-copy edit?
//! # let _ = (current, dirty); Ok(()) }
//! ```
//!
//! Mutate inside a [`transaction`](Jj::transaction) — an `Err` rolls the op log back:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_jj::Jj;
//! # async fn demo(jj: &Jj) -> Result<(), processkit::Error> {
//! let dir = Path::new(".");
//! jj.transaction(dir, |tx| async move {
//!     tx.describe("wip").await?;
//!     tx.new_change("next").await        // an Err here undoes the describe
//! })
//! .await?;
//! # Ok(()) }
//! ```
//!
//! A binding (or any caller that can't pass a Rust closure) drives the same
//! rollback imperatively with the primitives [`transaction`](Jj::transaction)
//! wraps — [`op_head`](JjApi::op_head) to capture a savepoint and
//! [`op_restore`](JjApi::op_restore) to roll back to it on failure (both on the
//! object-safe [`JjApi`]).
//!
//! # Testing
//!
//! Two seams: enable the **`mock`** feature for a `mockall`-generated
//! `MockJjApi` (stub whole methods), or inject a
//! [`ScriptedRunner`](processkit::testing::ScriptedRunner) with [`Jj::with_runner`] to
//! exercise the *real* argv-building and parsing against canned output. The
//! cross-cutting testing patterns live in
//! [vcs-testkit's guide](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/).
//!
//! # Safety
//!
//! Every caller value placed in a bare positional argv slot (bookmark name,
//! revset, operation id, merge parent, …) is refused before spawning if it is
//! empty or starts with `-` (jj would parse it as a flag); flag-value slots
//! (`-r <revset>`, `-m <msg>`) and the `run`/`run_raw` escape hatches are not
//! guarded. For eager validation at an input boundary, [`RevsetExpr`] validates
//! up front. Paths go through the exact-path [`JjFileset`] form.
//!
//! # In-depth guide
//!
//! Beyond this page, this crate ships a full how-to guide — rendered on docs.rs
//! from `docs/`. See the [`guide`] module. The conflict model is covered by
//! [vcs-git's conflicts guide](https://docs.rs/vcs-git/latest/vcs_git/guide/conflicts/),
//! which spans both backends.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

// Re-export the processkit types in this crate's public API, so consumers needn't
// depend on processkit directly — incl. `ProcessRunner` (the `with_runner`/`Jj<R>`
// seam) and the `JobRunner` default. (Also brings `Error`/`Result`/`ProcessResult`/
// `ProcessRunner` into scope here.)
pub use processkit::{Error, JobRunner, ProcessResult, ProcessRunner, Result};
// Re-exported so a consumer can name the token for `default_cancel_on` without
// taking a direct `processkit` dependency.
pub use processkit::CancellationToken;

pub mod conflict;
mod parse;
pub use parse::{AnnotationLine, Bookmark, BookmarkRef, Change, ChangedPath, Operation, Workspace};
// The git-format diff model + parser and the version type are shared with
// `vcs-git` (identical output) — re-exported so `vcs_jj::FileDiff`,
// `vcs_jj::parse_diff`, `vcs_jj::JjVersion`, … still resolve.
pub use vcs_diff::{
    ChangeKind, DiffLine, DiffSpec, DiffStat, FileDiff, Hunk, Version as JjVersion, parse_diff,
};
// The error classifiers live in the shared plumbing crate — re-exported so
// `vcs_jj::is_transient_fetch_error`, `vcs_jj::is_lock_contention` still resolve.
pub use vcs_cli_support::{RetryPolicy, is_lock_contention, is_transient_fetch_error};

/// Name of the underlying CLI binary this crate drives.
pub const BINARY: &str = "jj";

/// How a new workspace inherits sparse patterns (`jj workspace add
/// --sparse-patterns <mode>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SparseMode {
    /// Copy all sparse patterns from the current workspace (jj's default).
    Copy,
    /// Include every file in the new workspace.
    Full,
    /// Start with no files — the caller sets patterns afterwards (CoW flow).
    Empty,
}

impl SparseMode {
    /// The `--sparse-patterns` value jj expects.
    fn as_arg(self) -> &'static str {
        match self {
            SparseMode::Copy => "copy",
            SparseMode::Full => "full",
            SparseMode::Empty => "empty",
        }
    }
}

/// An exact-path jj fileset (`root-file:"<path>"`), so path metacharacters like `(`,
/// `)`, `|`, `*` are treated literally rather than as fileset operators.
///
/// Build it with [`JjFileset::path`]; the path is **workspace-root-relative** and
/// resolved as such regardless of the command's working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JjFileset(String);

impl JjFileset {
    /// Wrap a workspace-root-relative `path` as an exact-path fileset. Uses jj's
    /// **`root-file:`** anchor (not the cwd-relative `file:`), so the path is
    /// interpreted relative to the workspace root even when the command runs from a
    /// subdirectory (`dir` ≠ root) — a plain `file:` there would silently target a
    /// same-named file under `dir`, or nothing (M2). **On Windows** the caller's `\`
    /// path separators are normalised to jj's forward slash (so `src\a.rs` matches);
    /// **on Unix** `\` is a legitimate filename byte and is left intact — rewriting it
    /// there would corrupt a real path (matching `vcs-git`'s twin, which also gates
    /// the rewrite on Windows). Then `\` and `"` are escaped for the string literal.
    pub fn path(path: impl AsRef<str>) -> Self {
        let path = path.as_ref();
        #[cfg(windows)]
        let normalised = path.replace('\\', "/");
        #[cfg(not(windows))]
        let normalised = path.to_string();
        let escaped = normalised.replace('\\', "\\\\").replace('"', "\\\"");
        JjFileset(format!("root-file:\"{escaped}\""))
    }

    /// The rendered `root-file:"…"` expression.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Options for [`JjApi::workspace_add`] (`jj workspace add`).
///
/// `#[non_exhaustive]`, so build it through [`WorkspaceAdd::new`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WorkspaceAdd {
    /// Name for the new workspace.
    pub name: String,
    /// Revision the workspace's working copy starts at (`-r <base>`).
    pub base: RevsetExpr,
    /// Filesystem path for the new workspace.
    pub path: PathBuf,
    /// How to seed the new workspace's sparse patterns (`--sparse-patterns`);
    /// `None` leaves jj's default (inherit from the current workspace).
    pub sparse_patterns: Option<SparseMode>,
}

impl WorkspaceAdd {
    /// A workspace named `name`, based at `base`, materialised at `path`.
    pub fn new(name: impl Into<String>, base: RevsetExpr, path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            base,
            path: path.into(),
            sparse_patterns: None,
        }
    }

    /// Seed the new workspace's sparse patterns with `mode` (`--sparse-patterns`).
    pub fn sparse(mut self, mode: SparseMode) -> Self {
        self.sparse_patterns = Some(mode);
        self
    }
}

/// Options for [`JjApi::squash_paths`] (`jj squash --from <from> --into <into>
/// [--use-destination-message] <filesets>`).
///
/// `#[non_exhaustive]`, so build it through [`SquashPaths::new`] and the chained
/// setters rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SquashPaths {
    /// Source revision the filesets are squashed out of (`--from`).
    pub from: RevsetExpr,
    /// Destination revision the filesets are squashed into (`--into`).
    pub into: RevsetExpr,
    /// The exact filesets to move; empty squashes the whole `from` change.
    pub filesets: Vec<JjFileset>,
    /// Keep the destination's description rather than combining the two
    /// (`--use-destination-message`).
    pub use_destination_message: bool,
}

impl SquashPaths {
    /// Squash from `from` into `into`, with no filesets selected yet.
    pub fn new(from: RevsetExpr, into: RevsetExpr) -> Self {
        Self {
            from,
            into,
            filesets: Vec::new(),
            use_destination_message: false,
        }
    }

    /// Set the filesets to move (replacing any already added).
    pub fn filesets(mut self, filesets: impl IntoIterator<Item = JjFileset>) -> Self {
        self.filesets = filesets.into_iter().collect();
        self
    }

    /// Keep the destination's description (`--use-destination-message`) instead
    /// of combining the two.
    pub fn use_destination_message(mut self) -> Self {
        self.use_destination_message = true;
        self
    }
}

/// Options for [`JjApi::bookmark_move`] (`jj bookmark move <name> --to <rev>`).
///
/// `#[non_exhaustive]`, so build it through [`BookmarkMove::new`] and the chained
/// [`allow_backwards`](BookmarkMove::allow_backwards) setter rather than a bare
/// `bool` (`bookmark_move(name, to, true)` doesn't say what `true` permits).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BookmarkMove {
    /// The bookmark to move.
    pub name: BookmarkName,
    /// The revision to move it to (`--to`).
    pub to: RevsetExpr,
    /// Allow moving the bookmark to a commit that is not a descendant of its
    /// current target (`--allow-backwards`).
    pub allow_backwards: bool,
}

impl BookmarkMove {
    /// Move bookmark `name` to revision `to`; a backwards move is refused.
    pub fn new(name: BookmarkName, to: RevsetExpr) -> Self {
        Self {
            name,
            to,
            allow_backwards: false,
        }
    }

    /// Allow moving to a commit that is not a descendant of the current target
    /// (`--allow-backwards`).
    pub fn allow_backwards(mut self) -> Self {
        self.allow_backwards = true;
        self
    }
}

/// Options for [`JjApi::squash_into`] (`jj squash --into <rev>`).
///
/// `#[non_exhaustive]`, so build it through [`SquashInto::new`] and the chained
/// [`use_destination_message`](SquashInto::use_destination_message) setter rather
/// than a bare `bool`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SquashInto {
    /// The destination revision the working copy is squashed into (`--into`).
    pub into: RevsetExpr,
    /// Keep the destination's description rather than combining the two
    /// (`--use-destination-message`).
    pub use_destination_message: bool,
}

impl SquashInto {
    /// Squash the working copy into `into`, combining the two descriptions.
    pub fn new(into: RevsetExpr) -> Self {
        Self {
            into,
            use_destination_message: false,
        }
    }

    /// Keep the destination's description (`--use-destination-message`) instead
    /// of combining the two.
    pub fn use_destination_message(mut self) -> Self {
        self.use_destination_message = true;
        self
    }
}

/// Colocation choice for [`JjApi::git_clone`] (`jj git clone
/// --colocate|--no-colocate`).
///
/// The flag is **always** passed explicitly — jj's default flipped across versions
/// and is overridable via `git.colocate` config — so there is deliberately no
/// default: pick [`GitClone::colocated`] or [`GitClone::separate`].
/// `#[non_exhaustive]`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GitClone {
    /// Create a visible `.git` alongside `.jj` (`--colocate`) rather than a
    /// jj-only checkout (`--no-colocate`).
    pub colocate: bool,
}

impl GitClone {
    /// A colocated clone — a visible `.git` beside `.jj` (`--colocate`).
    pub fn colocated() -> Self {
        Self { colocate: true }
    }

    /// A non-colocated clone — jj-only, no `.git` (`--no-colocate`).
    pub fn separate() -> Self {
        Self { colocate: false }
    }
}

/// The first bookmark name from a comma-joined [`BOOKMARKS_TEMPLATE`](parse::BOOKMARKS_TEMPLATE)
/// render; `None` when the commit carries no local bookmark.
fn first_bookmark(rendered: &str) -> Option<String> {
    let rendered = rendered.trim();
    (!rendered.is_empty()).then(|| rendered.split(',').next().unwrap_or(rendered).to_string())
}

/// Injection guard for bare positional argv slots: a caller-supplied value
/// with a leading `-` is parsed by jj's CLI as a *flag* (verified: `jj edit
/// -evil` → "unexpected argument"), and an empty value changes a command's
/// meaning. Refuse both before anything spawns. Flag-VALUE positions
/// (`-r <revset>`, `-m <msg>`) need no guard — jj itself rejects dash-values
/// there with a clear error rather than misparsing them.
fn reject_flag_like(what: &str, value: &str) -> Result<()> {
    vcs_cli_support::reject_flag_like(BINARY, what, value)
}

/// The working-copy revset `@` as a validated [`RevsetExpr`]. Infallible — `@`
/// is always a valid revset — for the internal helpers that query `@` directly.
fn at_revset() -> RevsetExpr {
    RevsetExpr::new("@").expect("`@` is a valid revset")
}

/// Wrap a caller-supplied bookmark/branch/remote name as jj's `exact:` string
/// pattern. jj treats a bare `<NAMES>` / `-b <BOOKMARK>` / `--remote <REMOTE>`
/// argument as a **glob** pattern (verified on 0.42: `bookmark delete '*'`
/// deletes every bookmark; `git push -b '*'` pushes them all), so a name that
/// happens to contain `*`/`?` — or a hostile `"*"` from a UI/bot — would fan the
/// operation out across every matching ref. `exact:` forces a literal match of
/// exactly this name (verified: `exact:foo1` deletes only `foo1`, and a literal
/// `*` in a name is matched verbatim under `exact:`), so these typed methods
/// mutate exactly the one ref the caller named.
fn exact(name: &str) -> String {
    format!("exact:{name}")
}

/// Injection guard for the remote segment of jj's positional `<name>@<remote>`
/// bookmark-tracking pattern. Unlike a bare `<NAMES>`/`--remote` slot, the
/// remote segment of this composite form is **not** itself parsed as a
/// string-pattern: a `exact:`/`glob:` prefix on it is taken as part of the
/// *literal* remote name instead of being interpreted (verified on jj 0.42:
/// `bookmark track exact:main@exact:origin` warns "No matching remote
/// bookmarks for names: main@\"exact:origin\"" and tracks nothing — a silent
/// no-op, not an error — and `main@glob:origin` is rejected outright with
/// "remote bookmark must be specified in bookmark@remote form"). The segment
/// is, however, still glob-matched positionally (`main@ori?in` tracks
/// `origin`), so a hostile/glob-bearing remote name must be rejected before
/// spawn rather than wrapped in `exact:`.
fn reject_glob_like(what: &str, value: &str) -> Result<()> {
    if value.contains(['*', '?', '[', ']']) {
        return Err(Error::spawn(
            BINARY,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{what} {value:?} contains a glob metacharacter and could fan out across \
                     remotes — refusing to pass it as a positional argument"
                ),
            ),
        ));
    }
    Ok(())
}

/// Pin `LC_ALL=C` on a command whose failure output is classified by matching
/// **untranslated English substrings** — the transient-fetch markers
/// (`is_transient_fetch_error`). jj's `git fetch` surfaces libc/gai/curl network
/// errors ("Temporary failure in name resolution"), which a localized environment
/// would translate — silently turning a retryable transient failure into an
/// unclassified one that is *not* retried. Mirrors `vcs-git`'s `c_locale`.
fn c_locale(cmd: processkit::Command) -> processkit::Command {
    cmd.env("LC_ALL", "C")
}

/// A validated revset expression. Every [`JjApi`] operation that resolves a
/// revision/revset takes a `RevsetExpr` (directly or inside its options struct),
/// so a revset from untrusted input (UIs, bots, agents) is validated once, at
/// construction, and the type is the flag-injection barrier from then on.
/// Deliberately *minimal* — jj's revset grammar is too rich to validate here —
/// it only guarantees the expression is non-empty and cannot be parsed as a flag
/// (no leading `-`). A rejected expression is an [`Error::is_invalid_input`]
/// failure. For a value that must be a bookmark **name** (create/move/delete a
/// bookmark) use [`BookmarkName`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RevsetExpr(String);

impl RevsetExpr {
    /// Validate `revset` (non-empty, no leading `-`).
    pub fn new(revset: impl Into<String>) -> Result<Self> {
        let revset = revset.into();
        reject_flag_like("revset", &revset)?;
        Ok(RevsetExpr(revset))
    }

    /// The validated expression.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RevsetExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for RevsetExpr {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Self::new(s)
    }
}

/// A validated jj bookmark name (jj's equivalent of a git branch). Every
/// [`JjApi`] operation that names a bookmark to create, move, rename, delete,
/// track, fetch, or push takes a `BookmarkName`, so a name from untrusted input
/// is validated once, at construction. jj bookmark names are permissive, so the
/// guarantee is the load-bearing one: non-empty and not flag-shaped (no leading
/// `-`), matching the injection guard these operations applied internally before.
/// The typed methods additionally wrap the name in jj's `exact:` string pattern
/// so a `*`/`?` in a name can never fan the operation out across every bookmark.
/// A rejected name is an [`Error::is_invalid_input`] failure.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BookmarkName(String);

impl BookmarkName {
    /// Validate `name` as a bookmark name (non-empty, no leading `-`).
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        reject_flag_like("bookmark name", &name)?;
        Ok(BookmarkName(name))
    }

    /// The validated name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BookmarkName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for BookmarkName {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Self::new(s)
    }
}

/// What the installed `jj` binary supports, probed via
/// [`JjApi::capabilities`]. A value type — the client holds no state, so probe
/// once and keep the result (callers cache it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct JjCapabilities {
    /// The binary's parsed version.
    pub version: JjVersion,
}

/// The validated jj floor: every parser and flag in this crate was verified
/// empirically against this release. jj's CLI moves fast, so the floor is a full
/// version pinned to a validated release; vcs-git instead gates on the highest
/// version its own argv requires (`2.31`).
const MIN_SUPPORTED: JjVersion = JjVersion {
    major: 0,
    minor: 38,
    patch: 0,
};

impl JjCapabilities {
    /// Whether the binary meets the validated floor (jj ≥ 0.38).
    pub fn is_supported(&self) -> bool {
        self.version >= MIN_SUPPORTED
    }

    /// Error unless [`is_supported`](Self::is_supported) — a clear "needs jj
    /// ≥ 0.38, found 0.35.0" instead of a cryptic argv/template failure later.
    pub fn ensure_supported(&self) -> Result<()> {
        if self.is_supported() {
            return Ok(());
        }
        Err(Error::spawn(
            BINARY,
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!(
                    "vcs-jj requires jj >= {MIN_SUPPORTED} (the validated floor), found {}",
                    self.version
                ),
            ),
        ))
    }
}

/// The jj operations this crate exposes — the interface consumers code against
/// and mock in tests.
///
/// **Injection safety:** bookmark names and revsets are taken as the validated
/// [`BookmarkName`] / [`RevsetExpr`] newtypes (directly or inside an options
/// struct), so a flag-like or malformed value is rejected at construction,
/// before it can reach an argv slot. The remaining caller-supplied bare
/// positionals that are *not* bookmarks/revsets — remote names and operation
/// ids — keep an internal guard: a value that is empty or begins with `-` is
/// rejected with an [`Error::Spawn`] *before* spawning. Flag-value slots
/// (`-m <msg>`) and the `run`/`run_raw` escape hatches are not guarded.
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait::async_trait]
pub trait JjApi: Send + Sync {
    /// Run `jj <args>`, returning trimmed stdout (throws on a non-zero exit).
    ///
    /// **Unguarded escape hatch — you own its safety.** `args` is forwarded
    /// verbatim, so never pass untrusted tokens here: jj's `--config`/
    /// `--config-toml` and user-defined aliases can reach code execution. The
    /// guarded typed methods are the safe path.
    async fn run(&self, args: &[String]) -> Result<String>;
    /// Like [`JjApi::run`] but never errors on a non-zero exit — returns the
    /// captured [`ProcessResult`]. Same unguarded-escape-hatch caveat as
    /// [`run`](JjApi::run): never forward untrusted argv.
    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>>;
    /// Installed Jujutsu version (`jj --version`).
    async fn version(&self) -> Result<String>;
    /// The installed binary's parsed version, as [`JjCapabilities`]
    /// (`jj --version`). A value type — probe once and keep it; an
    /// unrecognisable version string is an [`Error::Parse`].
    async fn capabilities(&self) -> Result<JjCapabilities>;
    /// Parsed working-copy changes — the files changed in `@`
    /// (`jj diff -r @ --summary`), mirroring `vcs_git` `status`.
    async fn status(&self, dir: &Path) -> Result<Vec<ChangedPath>>;
    /// Raw `jj status` text (human-readable) — the unparsed counterpart of
    /// [`status`](JjApi::status), mirroring `vcs_git` `status_text`.
    async fn status_text(&self, dir: &Path) -> Result<String>;
    /// Changes matching `revset`, newest first, up to `max` (`jj log`).
    async fn log(&self, dir: &Path, revset: &RevsetExpr, max: usize) -> Result<Vec<Change>>;
    /// Like [`log`](JjApi::log), but scoped to changes that touched `filesets`
    /// (`jj log -r <revset> <filesets>`) — e.g. "who changed this module".
    /// Build filesets with [`JjFileset::path`] (same primitive as
    /// [`commit_paths`](JjApi::commit_paths)/[`squash_paths`](JjApi::squash_paths)).
    /// An empty `filesets` is refused *before spawning*: silently falling back
    /// to [`log`](JjApi::log)'s unrestricted history would defeat the "scoped
    /// to these paths" contract. Mirrors
    /// [`GitApi::log_paths`](../vcs_git/trait.GitApi.html#tymethod.log_paths),
    /// which takes pathspecs instead of filesets.
    async fn log_paths(
        &self,
        dir: &Path,
        revset: &RevsetExpr,
        max: usize,
        filesets: &[JjFileset],
    ) -> Result<Vec<Change>>;
    /// The working-copy change (`jj log -r @`).
    async fn current_change(&self, dir: &Path) -> Result<Change>;
    /// Set the working-copy change's description (`jj describe -m`).
    async fn describe(&self, dir: &Path, message: &str) -> Result<()>;
    /// Set the description of an arbitrary revision (`jj describe -r <revset> -m`).
    async fn describe_rev(&self, dir: &Path, revset: &RevsetExpr, message: &str) -> Result<()>;
    /// Start a new change on top of the working copy (`jj new -m`).
    async fn new_change(&self, dir: &Path, message: &str) -> Result<()>;
    /// Start a new undescribed change on top of `parent` (`jj new <parent>`).
    async fn new_child(&self, dir: &Path, parent: &RevsetExpr) -> Result<()>;
    /// Local bookmarks (`jj bookmark list`).
    async fn bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>>;
    /// Local *and* remote-tracking bookmarks (`jj bookmark list -a`).
    async fn bookmarks_all(&self, dir: &Path) -> Result<Vec<BookmarkRef>>;
    /// Local bookmarks on the nearest commits reachable from `@`
    /// (`log -r 'heads(::@ & bookmarks())'`) — the candidate targets a commit
    /// "belongs to". A commit carrying several bookmarks yields one entry each.
    async fn reachable_bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>>;
    /// Track a remote bookmark (`jj bookmark track <name>@<remote>`).
    async fn bookmark_track(&self, dir: &Path, name: &BookmarkName, remote: &str) -> Result<()>;
    /// Point a bookmark at `revision` (`jj bookmark set <name> -r <revision>`).
    async fn bookmark_set(&self, dir: &Path, name: &BookmarkName, revision: &RevsetExpr) -> Result<()>;
    /// Fetch from the git remote (`jj git fetch`); transient (network) failures
    /// are retried (3 attempts, 500 ms backoff).
    async fn git_fetch(&self, dir: &Path) -> Result<()>;
    /// Fetch from a *named* git remote (`jj git fetch --remote <remote>`);
    /// transient failures are retried like [`git_fetch`](JjApi::git_fetch).
    async fn git_fetch_from(&self, dir: &Path, remote: &str) -> Result<()>;
    /// Push to the git remote (`jj git push`, optionally `-b <bookmark>`). The
    /// bookmark is owned (`Option<BookmarkName>`) to keep the trait `mockall`-friendly.
    async fn git_push(&self, dir: &Path, bookmark: Option<BookmarkName>) -> Result<()>;

    // --- Discovery / identity ------------------------------------------------

    /// Working-copy root of the current workspace (`jj root`).
    async fn root(&self, dir: &Path) -> Result<PathBuf>;
    /// The local bookmark on the working-copy change `@`, if exactly one (or the
    /// first of several); `None` when `@` carries no bookmark. `ws` enforces the
    /// one-bookmark policy on top.
    async fn current_bookmark(&self, dir: &Path) -> Result<Option<String>>;
    /// The trunk bookmark (`jj log -r 'trunk()'`); `None` when unresolved.
    async fn trunk(&self, dir: &Path) -> Result<Option<String>>;

    // --- Bookmarks -----------------------------------------------------------

    /// Create a bookmark at a revision (`bookmark create <name> -r <rev>`).
    async fn bookmark_create(&self, dir: &Path, name: &BookmarkName, revision: &RevsetExpr) -> Result<()>;
    /// Rename a bookmark (`bookmark rename <old> <new>`).
    async fn bookmark_rename(&self, dir: &Path, old: &BookmarkName, new: &BookmarkName) -> Result<()>;
    /// Delete a bookmark (`bookmark delete <name>`).
    async fn bookmark_delete(&self, dir: &Path, name: &BookmarkName) -> Result<()>;
    /// Move a bookmark to a revision (`bookmark move <name> --to <rev>
    /// [--allow-backwards]`); see [`BookmarkMove`].
    async fn bookmark_move(&self, dir: &Path, spec: BookmarkMove) -> Result<()>;

    // --- Diff / query / state ------------------------------------------------

    /// Per-file change summary for a range (`diff -r <from>..<to> --summary`).
    async fn diff_summary(&self, dir: &Path, from: &RevsetExpr, to: &RevsetExpr) -> Result<Vec<ChangedPath>>;
    /// Aggregate change stats for a revset (`diff -r <revset> --stat`).
    async fn diff_stat(&self, dir: &Path, revset: &RevsetExpr) -> Result<DiffStat>;
    /// Raw git-format unified diff text for `spec` (`diff -r <spec> --git`) —
    /// stable machine output, returned **verbatim** (a trailing blank context line
    /// is preserved, so the last hunk stays in sync with its `@@` line count).
    async fn diff_text(&self, dir: &Path, spec: DiffSpec) -> Result<String>;
    /// Parsed per-file unified diff for `spec`, layered on [`diff_text`](JjApi::diff_text).
    async fn diff(&self, dir: &Path, spec: DiffSpec) -> Result<Vec<FileDiff>>;
    /// Count commits in a revset (`log -r <revset> --no-graph`, one id per line).
    async fn commit_count(&self, dir: &Path, revset: &RevsetExpr) -> Result<usize>;
    /// Whether the commit a revset resolves to has a conflict.
    async fn is_conflicted(&self, dir: &Path, revset: &RevsetExpr) -> Result<bool>;
    /// Whether the working copy has unresolved conflicts (`jj status`).
    async fn has_workingcopy_conflict(&self, dir: &Path) -> Result<bool>;
    /// Paths with unresolved conflicts in `revset` (`jj resolve --list -r <revset>`).
    /// Empty when there are none.
    async fn resolve_list(&self, dir: &Path, revset: &RevsetExpr) -> Result<Vec<String>>;
    /// Run an arbitrary templated `jj log` query and return raw stdout
    /// (`log -r <revset> --no-graph [--limit n] -T <template>`).
    async fn template_query(
        &self,
        dir: &Path,
        revset: &RevsetExpr,
        template: &str,
        limit: Option<usize>,
    ) -> Result<String>;
    /// The full (possibly multiline) description of the commit `revset` resolves
    /// to, trailing whitespace trimmed; empty for an undescribed change — or for
    /// a revset matching no commit (an *invalid* revset still errors). A
    /// multi-commit revset yields only the newest commit's description
    /// (`jj log` order, `--limit 1`).
    async fn description(&self, dir: &Path, revset: &RevsetExpr) -> Result<String>;
    /// How the commit a revset resolves to evolved, newest snapshot first, up
    /// to `max` (`jj evolog -r <revset>`) — one [`Change`] row per recorded
    /// predecessor.
    async fn evolog(&self, dir: &Path, revset: &RevsetExpr, max: usize) -> Result<Vec<Change>>;
    /// Per-line authorship of `path` (`jj file annotate <path> [-r <revset>]`;
    /// `None` = `@`): which change introduced each line.
    async fn file_annotate(
        &self,
        dir: &Path,
        path: &str,
        revset: Option<RevsetExpr>,
    ) -> Result<Vec<AnnotationLine>>;
    /// A file's content at a revision (`jj file show -r <revset>
    /// root-file:"<path>"` — the path is wrapped as a workspace-root-relative
    /// exact-path fileset, so fileset metacharacters in the name stay literal). Content is decoded
    /// lossily — a binary file comes back mangled rather than erroring — and
    /// returned **verbatim**: the file's trailing newline(s) are preserved (not
    /// trimmed), so a read-modify-write round-trip is byte-exact.
    async fn file_show(&self, dir: &Path, revset: &RevsetExpr, path: &str) -> Result<String>;

    // --- Mutations -----------------------------------------------------------

    /// Rebase the working-copy change and its branch onto `<onto>` (`rebase
    /// -d <onto>`, i.e. jj's default `-b @`). jj's branch set is `(onto..@)::` —
    /// the fork-point-to-`@` line **and its whole descendant closure**: `@`,
    /// everything stacked on top of `@`, and any sibling that branches off an
    /// *intermediate* commit of that line all move onto `<onto>`.
    ///
    /// This is **not** identical to git's `rebase <onto>`, which moves only
    /// `merge-base(@,onto)..@` — `@`'s own ancestor line — and leaves commits
    /// stacked on `@` (and intermediate-fork siblings) where they are. On a
    /// linear `@` the two agree; on a **stacked or intermediate-fork** layout jj
    /// moves strictly more. A sibling that branches off the **fork point itself**
    /// is untouched by both (it is not in `(onto..@)::`). Use
    /// [`rebase_branch`](JjApi::rebase_branch) with an explicit revset for
    /// narrower control.
    async fn rebase(&self, dir: &Path, onto: &RevsetExpr) -> Result<()>;
    /// Rebase a whole branch onto a destination (`rebase -b <branch> -d <dest>`).
    async fn rebase_branch(&self, dir: &Path, branch: &RevsetExpr, dest: &RevsetExpr) -> Result<()>;
    /// Move the working copy to a revision (`edit <rev>`).
    async fn edit(&self, dir: &Path, revset: &RevsetExpr) -> Result<()>;
    /// Squash the working copy into a revision (`squash --into <rev>
    /// [--use-destination-message]`); see [`SquashInto`].
    async fn squash_into(&self, dir: &Path, spec: SquashInto) -> Result<()>;
    /// Finalise a commit from exactly these filesets (`commit -m <message>
    /// <filesets>`); the rest stay in the new working-copy change. An **empty**
    /// `filesets` slice is refused with `Error::Spawn`/`InvalidInput` before spawning
    /// (a bare `jj commit` would commit the whole working copy, not "exactly these").
    async fn commit_paths(&self, dir: &Path, filesets: &[JjFileset], message: &str) -> Result<()>;
    /// Squash exactly these filesets from one revision into another
    /// (`squash --from <from> --into <into> [--use-destination-message] <filesets>`).
    async fn squash_paths(&self, dir: &Path, spec: SquashPaths) -> Result<()>;
    /// Set the working copy's sparse patterns to exactly `patterns`
    /// (`sparse set --clear --add <p>…`); an empty list clears the working copy.
    async fn sparse_set(&self, dir: &Path, patterns: &[String]) -> Result<()>;
    /// Create a new change with the given parents (`new -m <msg> <p1> <p2> …`).
    async fn new_merge(&self, dir: &Path, message: &str, parents: Vec<RevsetExpr>) -> Result<()>;
    /// Abandon a revision (`abandon <rev>`).
    async fn abandon(&self, dir: &Path, revset: &RevsetExpr) -> Result<()>;
    /// Fetch a single bookmark from origin (`git fetch --remote origin -b <branch>`);
    /// transient failures are retried (3×, 500 ms).
    async fn git_fetch_branch(&self, dir: &Path, branch: &BookmarkName) -> Result<()>;
    /// Import git refs into jj (`jj git import`) — colocated-repo sync.
    async fn git_import(&self, dir: &Path) -> Result<()>;
    /// Clone a git repository into `dest` (`jj git clone <url> <dest>
    /// --colocate|--no-colocate`). Runs without a working directory — pass an
    /// **absolute** `dest`. The flag is always passed explicitly: whether
    /// colocation (a visible `.git` alongside `.jj`) is jj's default depends
    /// on the jj version *and* the user's `git.colocate` config, so the
    /// [`GitClone`] choice decides deterministically.
    async fn git_clone(&self, url: &str, dest: &Path, spec: GitClone) -> Result<()>;
    /// Fold working-copy edits into the mutable ancestors that introduced the
    /// touched lines (`absorb [--from <revset>] [<filesets>…]`); empty
    /// `filesets` absorbs everything.
    async fn absorb(&self, dir: &Path, from: Option<RevsetExpr>, filesets: &[JjFileset]) -> Result<()>;
    /// Split exactly these filesets out of `@` into their own commit described
    /// by `message` (`split -m <message> <filesets>…`); the remainder stays
    /// behind. `filesets` must be non-empty — a fileset-less split opens jj's
    /// interactive diff editor (a headless hang), so it is refused with an
    /// error before spawning.
    async fn split_paths(&self, dir: &Path, filesets: &[JjFileset], message: &str) -> Result<()>;
    /// Duplicate the commits a revset resolves to (`duplicate <revset>`).
    async fn duplicate(&self, dir: &Path, revset: &RevsetExpr) -> Result<()>;

    // --- Operation log -------------------------------------------------------

    /// The current operation id (`op log --no-graph --limit 1`) — capture before
    /// a risky sequence to roll back to.
    async fn op_head(&self, dir: &Path) -> Result<String>;
    /// The newest `limit` operations, newest first (`op log --no-graph
    /// --limit n`).
    async fn op_log(&self, dir: &Path, limit: usize) -> Result<Vec<Operation>>;
    /// Restore the repo to an operation (`op restore <id>`).
    async fn op_restore(&self, dir: &Path, op_id: &str) -> Result<()>;
    /// Undo the latest operation (`op undo`).
    async fn op_undo(&self, dir: &Path) -> Result<()>;

    // --- Workspaces ----------------------------------------------------------

    /// List workspaces (`workspace list`).
    async fn workspace_list(&self, dir: &Path) -> Result<Vec<Workspace>>;
    /// Resolve a workspace's root path (`workspace root [--name <name>]`).
    async fn workspace_root(&self, dir: &Path, name: Option<String>) -> Result<PathBuf>;
    /// Add a workspace (`workspace add --name <name> -r <base> <path>`).
    async fn workspace_add(&self, dir: &Path, spec: WorkspaceAdd) -> Result<()>;
    /// Forget a workspace (`workspace forget <name>`).
    async fn workspace_forget(&self, dir: &Path, name: &str) -> Result<()>;
}

vcs_cli_support::managed_client! {
    /// The real jj client. Generic over the [`ProcessRunner`] so tests can inject a
    /// fake process executor; [`Jj::new`] uses the real job-backed runner.
    ///
    /// Wraps a [`ManagedClient`](vcs_cli_support::ManagedClient): enable lock-contention retry with
    /// [`with_retry`](Jj::with_retry) (opt-in; off by default).
    ///
    /// **Remote authentication is ambient.** Unlike `vcs-git` (which accepts a
    /// per-operation `CredentialProvider` via `with_credentials`), `jj`'s git remote
    /// support runs through its own in-process backend, which offers no per-invocation
    /// credential override — `jj git fetch`/`push` authenticate from the ambient git
    /// credential helpers / SSH agent. Configure those out of band.
    pub struct Jj => BINARY
}

impl<R: ProcessRunner> Jj<R> {
    /// Retry **lock-contention** failures (another process holds jj's working-copy
    /// lock) per `policy` — opt-in, off by default. Safe even for mutating commands:
    /// a lock-acquisition failure is pre-execution (jj never ran). See [`RetryPolicy`]
    /// and [`is_lock_contention`]. Note jj's operation log already auto-resolves most
    /// concurrency, so hard lock failures are rarer than with git.
    ///
    /// **Caveat:** modern jj generally *blocks* on the working-copy / operation-heads
    /// lock until it is free, rather than failing — so contention usually surfaces as
    /// a wait (bounded by the client's `default_timeout`), not a retryable error. This
    /// retry therefore catches only the residual cases where jj surfaces a lock error;
    /// for most jj concurrency the blocking behavior is what serializes access.
    pub fn with_retry(mut self, policy: RetryPolicy) -> Self {
        self.core = self.core.with_retry(policy);
        self
    }
}

impl<R: ProcessRunner> Jj<R> {
    /// A repo-scoped `jj` command with `--color never` forced on. jj honours
    /// `ui.color = "always"` from user config even when its output is piped, which
    /// would wrap our templated output — and the command error text we classify —
    /// in ANSI escapes and break parsing; `--color never` is the only thing that
    /// overrides that config (`NO_COLOR`/`CLICOLOR` do not). It is a global flag,
    /// appended here (no jj subcommand takes a trailing `--`, so this is safe).
    fn cmd_in<I, S>(&self, dir: &Path, args: I) -> processkit::Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.core.command_in(dir, args).arg("--color").arg("never")
    }
}

#[async_trait::async_trait]
impl<R: ProcessRunner> JjApi for Jj<R> {
    async fn run(&self, args: &[String]) -> Result<String> {
        self.core.run(args).await
    }

    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>> {
        self.core.output_string(args).await
    }

    async fn version(&self) -> Result<String> {
        self.core.run(["--version"]).await
    }

    async fn capabilities(&self) -> Result<JjCapabilities> {
        let raw = self.version().await?;
        let version = parse::parse_jj_version(&raw).ok_or_else(|| {
            Error::parse(
                BINARY,
                format!("unrecognisable `jj --version` output: {raw:?}"),
            )
        })?;
        Ok(JjCapabilities { version })
    }

    async fn status(&self, dir: &Path) -> Result<Vec<ChangedPath>> {
        // `diff -r @ --summary` is the machine-stable form of the working-copy
        // changes that `jj status` renders for humans: one `<letter> <path>` line.
        self.core
            .parse(
                self.cmd_in(dir, ["diff", "-r", "@", "--summary"]),
                parse::parse_diff_summary,
            )
            .await
    }

    async fn status_text(&self, dir: &Path) -> Result<String> {
        self.core.run(self.cmd_in(dir, ["status"])).await
    }

    async fn log(&self, dir: &Path, revset: &RevsetExpr, max: usize) -> Result<Vec<Change>> {
        let n = format!("-n{max}");
        self.core
            .parse(
                self.cmd_in(
                    dir,
                    [
                        "log",
                        "-r",
                        revset.as_str(),
                        n.as_str(),
                        "--no-graph",
                        "-T",
                        parse::CHANGE_TEMPLATE,
                    ],
                ),
                parse::parse_changes,
            )
            .await
    }

    async fn log_paths(
        &self,
        dir: &Path,
        revset: &RevsetExpr,
        max: usize,
        filesets: &[JjFileset],
    ) -> Result<Vec<Change>> {
        // An empty fileset slice would degrade `jj log -r <revset> <filesets…>`
        // to a bare `jj log -r <revset>` — UNRESTRICTED history, the opposite
        // of "scoped to these paths". Refuse before spawning (mirrors
        // `commit_paths`/`split_paths`).
        if filesets.is_empty() {
            return Err(Error::spawn(
                BINARY,
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "log_paths requires at least one fileset — an empty set would log \
                     unrestricted history, not history scoped to the named paths",
                ),
            ));
        }
        let n = format!("-n{max}");
        let mut args: Vec<String> = vec![
            "log".into(),
            "-r".into(),
            revset.as_str().into(),
            n,
            "--no-graph".into(),
            "-T".into(),
            parse::CHANGE_TEMPLATE.into(),
        ];
        args.extend(filesets.iter().map(|f| f.as_str().to_string()));
        self.core
            .parse(self.cmd_in(dir, args), parse::parse_changes)
            .await
    }

    async fn current_change(&self, dir: &Path) -> Result<Change> {
        let mut changes = self.log(dir, &at_revset(), 1).await?;
        changes
            .pop()
            .ok_or_else(|| Error::parse(BINARY, "no working-copy change found"))
    }

    async fn describe(&self, dir: &Path, message: &str) -> Result<()> {
        self.core
            .run_unit(self.cmd_in(dir, ["describe", "-m", message]))
            .await
    }

    async fn describe_rev(&self, dir: &Path, revset: &RevsetExpr, message: &str) -> Result<()> {
        self.core
            .run_unit(self.cmd_in(dir, ["describe", "-r", revset.as_str(), "-m", message]))
            .await
    }

    async fn new_change(&self, dir: &Path, message: &str) -> Result<()> {
        self.core
            .run_unit(self.cmd_in(dir, ["new", "-m", message]))
            .await
    }

    async fn new_child(&self, dir: &Path, parent: &RevsetExpr) -> Result<()> {
        self.core.run_unit(self.cmd_in(dir, ["new", parent.as_str()])).await
    }

    async fn bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>> {
        self.core
            .parse(
                self.cmd_in(
                    dir,
                    ["bookmark", "list", "-T", parse::BOOKMARK_LIST_TEMPLATE],
                ),
                parse::parse_bookmarks,
            )
            .await
    }

    async fn bookmarks_all(&self, dir: &Path) -> Result<Vec<BookmarkRef>> {
        self.core
            .parse(
                self.cmd_in(
                    dir,
                    ["bookmark", "list", "-a", "-T", parse::BOOKMARK_ALL_TEMPLATE],
                ),
                parse::parse_bookmarks_all,
            )
            .await
    }

    async fn reachable_bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>> {
        self.core
            .parse(
                self.cmd_in(
                    dir,
                    [
                        "log",
                        "-r",
                        "heads(::@ & bookmarks())",
                        "--no-graph",
                        "-T",
                        parse::REACHABLE_BOOKMARKS_TEMPLATE,
                    ],
                ),
                parse::parse_reachable_bookmarks,
            )
            .await
    }

    async fn bookmark_track(&self, dir: &Path, name: &BookmarkName, remote: &str) -> Result<()> {
        // A leading-`-` name makes the whole token start with `-`, which jj
        // parses as a global flag (e.g. `--config`); guard it. The bookmark
        // segment is wrapped in `exact:` (a real string-pattern there), but
        // the remote segment of this `<name>@<remote>` positional form is
        // *not* itself pattern-syntax — a `exact:` prefix on it is taken as
        // part of the literal remote name and silently matches nothing
        // (verified on jj 0.42: see `reject_glob_like`'s doc comment) — so the
        // remote is validated against glob metacharacters instead of wrapped.
        reject_glob_like("remote", remote)?;
        let target = format!("exact:{}@{remote}", name.as_str());
        self.core
            .run_unit(self.cmd_in(dir, ["bookmark", "track", target.as_str()]))
            .await
    }

    async fn bookmark_set(
        &self,
        dir: &Path,
        name: &BookmarkName,
        revision: &RevsetExpr,
    ) -> Result<()> {
        self.core
            .run_unit(self.cmd_in(
                dir,
                ["bookmark", "set", name.as_str(), "-r", revision.as_str()],
            ))
            .await
    }

    async fn git_fetch(&self, dir: &Path) -> Result<()> {
        // Idempotent → `retry` replays it on a transient (network) failure.
        // `c_locale`: the retry decision classifies the failure's message (M28).
        let cmd = c_locale(self.cmd_in(dir, ["git", "fetch"]))
            // Graceful terminate-then-kill on a per-client timeout, so a timed-out
            // fetch can close its connection cleanly.
            .timeout_grace(FETCH_TIMEOUT_GRACE)
            .retry(FETCH_ATTEMPTS, FETCH_BACKOFF, is_transient_fetch_error);
        self.core.run_unit(cmd).await
    }

    async fn git_fetch_from(&self, dir: &Path, remote: &str) -> Result<()> {
        // `--remote` is glob-matched too, so `exact:` keeps a `*` remote from
        // fetching from every configured remote. Idempotent → `retry` replays it
        // on a transient (network) failure.
        let remote_pat = exact(remote);
        // `c_locale`: the retry decision classifies the failure's message (M28).
        let cmd = c_locale(self.cmd_in(dir, ["git", "fetch", "--remote", remote_pat.as_str()]))
            .timeout_grace(FETCH_TIMEOUT_GRACE)
            .retry(FETCH_ATTEMPTS, FETCH_BACKOFF, is_transient_fetch_error);
        self.core.run_unit(cmd).await
    }

    async fn git_push(&self, dir: &Path, bookmark: Option<BookmarkName>) -> Result<()> {
        let mut args = vec!["git", "push"];
        // `-b` is glob-matched, so `exact:` keeps a `*` bookmark from pushing
        // every local bookmark at once (a UI/bot-supplied `"*"`).
        let bookmark_pat = bookmark.as_ref().map(|b| exact(b.as_str()));
        if let Some(name) = bookmark_pat.as_deref() {
            args.push("-b");
            args.push(name);
        }
        // Graceful terminate-then-kill on a per-client timeout, so a timed-out
        // push doesn't leave the remote ref half-updated. No-op without a
        // deadline (matches `git_fetch`).
        let cmd = self.cmd_in(dir, args).timeout_grace(FETCH_TIMEOUT_GRACE);
        self.core.run_unit(cmd).await
    }

    async fn root(&self, dir: &Path) -> Result<PathBuf> {
        Ok(PathBuf::from(
            self.core.run(self.cmd_in(dir, ["root"])).await?,
        ))
    }

    async fn current_bookmark(&self, dir: &Path) -> Result<Option<String>> {
        let out = self
            .core
            .run(self.cmd_in(
                dir,
                [
                    "log",
                    "-r",
                    "@",
                    "--no-graph",
                    "--limit",
                    "1",
                    "-T",
                    parse::BOOKMARKS_TEMPLATE,
                ],
            ))
            .await?;
        Ok(first_bookmark(&out))
    }

    async fn trunk(&self, dir: &Path) -> Result<Option<String>> {
        let out = self
            .core
            .run(self.cmd_in(
                dir,
                [
                    "log",
                    "-r",
                    "trunk()",
                    "--no-graph",
                    "--limit",
                    "1",
                    "-T",
                    parse::BOOKMARKS_TEMPLATE,
                ],
            ))
            .await?;
        Ok(first_bookmark(&out))
    }

    async fn bookmark_create(&self, dir: &Path, name: &BookmarkName, revision: &RevsetExpr) -> Result<()> {
        self.core
            .run_unit(self.cmd_in(dir, ["bookmark", "create", name.as_str(), "-r", revision.as_str()]))
            .await
    }

    async fn bookmark_rename(&self, dir: &Path, old: &BookmarkName, new: &BookmarkName) -> Result<()> {
        self.core
            .run_unit(self.cmd_in(dir, ["bookmark", "rename", old.as_str(), new.as_str()]))
            .await
    }

    async fn bookmark_delete(&self, dir: &Path, name: &BookmarkName) -> Result<()> {
        let name_pat = exact(name.as_str());
        self.core
            .run_unit(self.cmd_in(dir, ["bookmark", "delete", name_pat.as_str()]))
            .await
    }

    async fn bookmark_move(&self, dir: &Path, spec: BookmarkMove) -> Result<()> {
        // `<NAMES>` is glob-matched, so `exact:` keeps a `*` name from moving
        // every bookmark. `to` is a revision, not a pattern — left as-is.
        let name_pat = exact(spec.name.as_str());
        let mut args = vec![
            "bookmark",
            "move",
            name_pat.as_str(),
            "--to",
            spec.to.as_str(),
        ];
        if spec.allow_backwards {
            args.push("--allow-backwards");
        }
        self.core.run_unit(self.cmd_in(dir, args)).await
    }

    async fn diff_summary(&self, dir: &Path, from: &RevsetExpr, to: &RevsetExpr) -> Result<Vec<ChangedPath>> {
        // Parenthesise each endpoint so a compound revset (e.g. `x | y`) keeps its
        // meaning inside the `..` range instead of binding by operator precedence.
        let range = format!("({})..({})", from.as_str(), to.as_str());
        self.core
            .parse(
                self.cmd_in(dir, ["diff", "-r", range.as_str(), "--summary"]),
                parse::parse_diff_summary,
            )
            .await
    }

    async fn diff_stat(&self, dir: &Path, revset: &RevsetExpr) -> Result<DiffStat> {
        self.core
            .parse(
                self.cmd_in(dir, ["diff", "-r", revset.as_str(), "--stat"]),
                parse::parse_diff_stat,
            )
            .await
    }

    async fn diff_text(&self, dir: &Path, spec: DiffSpec) -> Result<String> {
        // `@` selects the working-copy change; otherwise the caller's revset.
        // `--git` emits stable git-format output the shared parser understands.
        let revset = match spec {
            DiffSpec::WorkingTree => "@".to_string(),
            DiffSpec::Rev(rev) => rev,
        };
        // `run_untrimmed`: trimming the diff would drop a trailing blank context
        // line, desyncing the last hunk from its `@@` line count for a consumer
        // that re-parses/re-applies it — same as git's `diff_text` (H7).
        self.core
            .run_untrimmed(self.cmd_in(dir, ["diff", "-r", revset.as_str(), "--git"]))
            .await
    }

    async fn diff(&self, dir: &Path, spec: DiffSpec) -> Result<Vec<FileDiff>> {
        let text = self.diff_text(dir, spec).await?;
        Ok(parse_diff(&text))
    }

    async fn commit_count(&self, dir: &Path, revset: &RevsetExpr) -> Result<usize> {
        self.core
            .parse(
                self.cmd_in(
                    dir,
                    [
                        "log",
                        "-r",
                        revset.as_str(),
                        "--no-graph",
                        "-T",
                        parse::COUNT_TEMPLATE,
                    ],
                ),
                |s| s.lines().filter(|line| !line.is_empty()).count(),
            )
            .await
    }

    async fn is_conflicted(&self, dir: &Path, revset: &RevsetExpr) -> Result<bool> {
        let out = self
            .core
            .run(self.cmd_in(
                dir,
                [
                    "log",
                    "-r",
                    revset.as_str(),
                    "--no-graph",
                    "--limit",
                    "1",
                    "-T",
                    parse::CONFLICT_TEMPLATE,
                ],
            ))
            .await?;
        Ok(out.trim() == "1")
    }

    async fn has_workingcopy_conflict(&self, dir: &Path) -> Result<bool> {
        // Ask the template engine directly rather than string-matching localized
        // `jj status` prose: `@` is conflicted iff its `conflict` flag is set.
        self.is_conflicted(dir, &at_revset()).await
    }

    async fn resolve_list(&self, dir: &Path, revset: &RevsetExpr) -> Result<Vec<String>> {
        let res = self
            .core
            .output_string(self.cmd_in(dir, ["resolve", "--list", "-r", revset.as_str()]))
            .await?;
        match res.code() {
            Some(0) => Ok(parse::parse_resolve_list(res.stdout())),
            // jj exits non-zero with "No conflicts found …" when the revision is
            // conflict-free — the one non-zero we read as an empty list. Any other
            // failure (bad revset, not a repo, …) must surface, not masquerade as
            // "no conflicts". `resolve --list` has no exit-code contract that
            // distinguishes the two, so this matches the message; jj's output is
            // English-only (no localization), so the risk is version *wording* drift,
            // not locale — matched on the stable core phrase, case-insensitively, to
            // absorb a capitalization change.
            _ if res.stderr().to_ascii_lowercase().contains("no conflicts") => Ok(Vec::new()),
            _ => {
                let _ = res.ensure_success()?;
                Ok(Vec::new()) // unreachable: a non-zero exit always errors above.
            }
        }
    }

    async fn template_query(
        &self,
        dir: &Path,
        revset: &RevsetExpr,
        template: &str,
        limit: Option<usize>,
    ) -> Result<String> {
        let mut args: Vec<String> = vec![
            "log".into(),
            "-r".into(),
            revset.as_str().into(),
            "--no-graph".into(),
        ];
        if let Some(n) = limit {
            args.push("--limit".into());
            args.push(n.to_string());
        }
        args.push("-T".into());
        args.push(template.into());
        // `run_untrimmed`: `template_query` is documented to return the template's
        // **raw** stdout, so a template that deliberately ends in `\n\n` or trailing
        // spaces (e.g. fixed-width joins) is preserved, not silently stripped (H7).
        // Callers that want a scalar trim it themselves (see `description`).
        self.core.run_untrimmed(self.cmd_in(dir, args)).await
    }

    async fn description(&self, dir: &Path, revset: &RevsetExpr) -> Result<String> {
        // `template_query` is raw now (H7); `description` is a scalar, so strip the
        // trailing newline jj appends to the `description` keyword (preserving the
        // pre-H7 contract that this returns the description without a trailing EOL).
        let out = self
            .template_query(dir, revset, "description", Some(1))
            .await?;
        Ok(out.trim_end().to_string())
    }

    async fn evolog(&self, dir: &Path, revset: &RevsetExpr, max: usize) -> Result<Vec<Change>> {
        // Evolog templates render in a *commit* context (bare `change_id`
        // doesn't exist there) — EVOLOG_TEMPLATE uses the `commit.` method
        // form but emits the same columns CHANGE_TEMPLATE does.
        let limit = max.to_string();
        self.core
            .parse(
                self.cmd_in(
                    dir,
                    [
                        "evolog",
                        "-r",
                        revset.as_str(),
                        "--no-graph",
                        "--limit",
                        limit.as_str(),
                        "-T",
                        parse::EVOLOG_TEMPLATE,
                    ],
                ),
                parse::parse_changes,
            )
            .await
    }

    async fn file_annotate(
        &self,
        dir: &Path,
        path: &str,
        revset: Option<RevsetExpr>,
    ) -> Result<Vec<AnnotationLine>> {
        // `file annotate` takes a plain PATH (not a fileset — the `file:"…"`
        // form is rejected), so a leading-`-` path would be parsed as a flag.
        // The `--` separator before it keeps even a `-dash.txt` literal safe —
        // but global flags (`--color never`) MUST precede `--`, so this builds
        // the command directly instead of via `cmd_in` (which trails them).
        let mut args = vec!["file", "annotate"];
        if let Some(revset) = revset.as_ref() {
            args.push("-r");
            args.push(revset.as_str());
        }
        args.extend([
            "-T",
            parse::ANNOTATE_TEMPLATE,
            "--color",
            "never",
            "--",
            path,
        ]);
        self.core
            .parse(self.core.command_in(dir, args), parse::parse_annotate)
            .await
    }

    async fn file_show(&self, dir: &Path, revset: &RevsetExpr, path: &str) -> Result<String> {
        // `file show` takes FILESETS, so a bare path with a fileset
        // metacharacter (`(`, `*`, `~`, …) would be parsed as an expression —
        // wrap it in the exact-path form. (`file annotate` is the opposite: it
        // takes a plain PATH and rejects the `file:"…"` form.)
        let fileset = JjFileset::path(path);
        // `run_untrimmed`: a file's trailing newline(s) are part of its content;
        // trimming corrupts a read-modify-write round-trip (H7).
        self.core
            .run_untrimmed(self.cmd_in(dir, ["file", "show", "-r", revset.as_str(), fileset.as_str()]))
            .await
    }

    async fn rebase(&self, dir: &Path, onto: &RevsetExpr) -> Result<()> {
        self.core
            .run_unit(self.cmd_in(dir, ["rebase", "-d", onto.as_str()]))
            .await
    }

    async fn rebase_branch(&self, dir: &Path, branch: &RevsetExpr, dest: &RevsetExpr) -> Result<()> {
        self.core
            .run_unit(self.cmd_in(dir, ["rebase", "-b", branch.as_str(), "-d", dest.as_str()]))
            .await
    }

    async fn edit(&self, dir: &Path, revset: &RevsetExpr) -> Result<()> {
        self.core.run_unit(self.cmd_in(dir, ["edit", revset.as_str()])).await
    }

    async fn squash_into(&self, dir: &Path, spec: SquashInto) -> Result<()> {
        let mut command = self.cmd_in(dir, ["squash", "--into", spec.into.as_str()]);
        if spec.use_destination_message {
            command = command.arg("--use-destination-message");
        }
        self.core.run_unit(command).await
    }

    async fn commit_paths(&self, dir: &Path, filesets: &[JjFileset], message: &str) -> Result<()> {
        // An empty fileset slice would degrade `jj commit -m <msg> <filesets…>` to a
        // bare `jj commit -m <msg>`, which commits the ENTIRE working copy — the
        // opposite of the "exactly these filesets" contract. Refuse it before spawning
        // (mirrors `split_paths`).
        if filesets.is_empty() {
            return Err(Error::spawn(
                BINARY,
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "commit_paths requires at least one fileset — an empty set would \
                     commit the entire working copy, not just the named paths",
                ),
            ));
        }
        let mut args: Vec<String> = vec!["commit".into(), "-m".into(), message.into()];
        args.extend(filesets.iter().map(|f| f.as_str().to_string()));
        self.core.run_unit(self.cmd_in(dir, args)).await
    }

    async fn squash_paths(&self, dir: &Path, spec: SquashPaths) -> Result<()> {
        let mut args: Vec<String> = vec![
            "squash".into(),
            "--from".into(),
            spec.from.as_str().into(),
            "--into".into(),
            spec.into.as_str().into(),
        ];
        if spec.use_destination_message {
            args.push("--use-destination-message".into());
        }
        args.extend(spec.filesets.iter().map(|f| f.as_str().to_string()));
        self.core.run_unit(self.cmd_in(dir, args)).await
    }

    async fn sparse_set(&self, dir: &Path, patterns: &[String]) -> Result<()> {
        // `--clear` empties the working copy first, then each `--add` reinstates a
        // pattern — so the working copy ends up holding exactly `patterns`.
        let mut args: Vec<String> = vec!["sparse".into(), "set".into(), "--clear".into()];
        for pattern in patterns {
            args.push("--add".into());
            args.push(pattern.clone());
        }
        self.core.run_unit(self.cmd_in(dir, args)).await
    }

    async fn new_merge(&self, dir: &Path, message: &str, parents: Vec<RevsetExpr>) -> Result<()> {
        // Parents are bare positionals, but each is a validated `RevsetExpr`, so a
        // leading-`-` one (e.g. `--ignore-working-copy`) can never reach the argv.
        let mut args: Vec<String> = vec!["new".into(), "-m".into(), message.into()];
        args.extend(parents.iter().map(|p| p.as_str().to_string()));
        self.core.run_unit(self.cmd_in(dir, args)).await
    }

    async fn abandon(&self, dir: &Path, revset: &RevsetExpr) -> Result<()> {
        self.core
            .run_unit(self.cmd_in(dir, ["abandon", revset.as_str()]))
            .await
    }

    async fn git_fetch_branch(&self, dir: &Path, branch: &BookmarkName) -> Result<()> {
        // `-b` is glob-matched, so `exact:` keeps a `*` branch from fetching
        // every branch instead of erroring on a bogus name.
        let branch_pat = exact(branch.as_str());
        // `c_locale`: the retry decision classifies the failure's message (M28).
        let cmd = c_locale(self.cmd_in(
            dir,
            [
                "git",
                "fetch",
                "--remote",
                "origin",
                "-b",
                branch_pat.as_str(),
            ],
        ))
        .timeout_grace(FETCH_TIMEOUT_GRACE)
        .retry(FETCH_ATTEMPTS, FETCH_BACKOFF, is_transient_fetch_error);
        self.core.run_unit(cmd).await
    }

    async fn git_import(&self, dir: &Path) -> Result<()> {
        self.core
            .run_unit(self.cmd_in(dir, ["git", "import"]))
            .await
    }

    async fn git_clone(&self, url: &str, dest: &Path, spec: GitClone) -> Result<()> {
        // A leading-`-` url is a bare positional — guard it (a real URL never
        // leads with `-`, so no false positives).
        reject_flag_like("url", url)?;
        // No working directory yet (the clone creates `dest`), so this builds
        // on the raw `command` and appends `--color never` at the end — the
        // `workspace_add` precedent for color-after-value-args. The colocate
        // flag is ALWAYS passed: jj's default flipped across versions and is
        // overridable via `git.colocate` config, so an omitted flag would make
        // `colocate: false` a lie on some setups.
        let command = self
            .core
            .command(["git", "clone", url])
            .arg(dest)
            .arg(if spec.colocate {
                "--colocate"
            } else {
                "--no-colocate"
            });
        // Graceful terminate-then-kill on a per-client timeout. No-op without a deadline.
        let command = command
            .arg("--color")
            .arg("never")
            .timeout_grace(FETCH_TIMEOUT_GRACE);

        // R7: like `vcs_git::clone_repo`, a failed clone can leave a partial `dest`
        // that blocks a retry ("destination already exists"); `timeout_grace` can't
        // prevent it (Windows' job-kill is atomic; the Unix grace is too short for a
        // multi-GB partial). Clean a `dest` we could have created — absent or an empty
        // dir — but never a non-empty pre-existing one (that's the caller's data,
        // untouched because jj/git refuses to clone into it). Best-effort, error path.
        let cleanable = match std::fs::read_dir(dest) {
            Err(_) => true,
            Ok(mut entries) => entries.next().is_none(),
        };
        let result = self.core.run_unit(command).await;
        if result.is_err() && cleanable {
            let _ = std::fs::remove_dir_all(dest);
        }
        result
    }

    async fn absorb(&self, dir: &Path, from: Option<RevsetExpr>, filesets: &[JjFileset]) -> Result<()> {
        let mut args: Vec<String> = vec!["absorb".into()];
        if let Some(from) = from.as_ref() {
            args.push("--from".into());
            args.push(from.as_str().into());
        }
        args.extend(filesets.iter().map(|f| f.as_str().to_string()));
        self.core.run_unit(self.cmd_in(dir, args)).await
    }

    async fn split_paths(&self, dir: &Path, filesets: &[JjFileset], message: &str) -> Result<()> {
        // A fileset-less `jj split` opens the interactive diff editor — even
        // with `-m` — which would hang a headless run indefinitely. Refuse
        // before spawning anything.
        if filesets.is_empty() {
            return Err(Error::spawn(
                BINARY,
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "split_paths requires at least one fileset — an empty split \
                     opens jj's interactive diff editor",
                ),
            ));
        }
        // `-m` doubles as the description-editor suppressor.
        let mut args: Vec<String> = vec!["split".into(), "-m".into(), message.into()];
        args.extend(filesets.iter().map(|f| f.as_str().to_string()));
        self.core.run_unit(self.cmd_in(dir, args)).await
    }

    async fn duplicate(&self, dir: &Path, revset: &RevsetExpr) -> Result<()> {
        self.core
            .run_unit(self.cmd_in(dir, ["duplicate", revset.as_str()]))
            .await
    }

    async fn op_head(&self, dir: &Path) -> Result<String> {
        self.core
            .run(self.cmd_in(
                dir,
                [
                    "op",
                    "log",
                    "--no-graph",
                    "--limit",
                    "1",
                    "-T",
                    "id.short()",
                ],
            ))
            .await
    }

    async fn op_log(&self, dir: &Path, limit: usize) -> Result<Vec<Operation>> {
        let limit = limit.to_string();
        self.core
            .parse(
                self.cmd_in(
                    dir,
                    [
                        "op",
                        "log",
                        "--no-graph",
                        "--limit",
                        limit.as_str(),
                        "-T",
                        parse::OP_TEMPLATE,
                    ],
                ),
                parse::parse_operations,
            )
            .await
    }

    async fn op_restore(&self, dir: &Path, op_id: &str) -> Result<()> {
        reject_flag_like("operation id", op_id)?;
        self.core
            .run_unit(self.cmd_in(dir, ["op", "restore", op_id]))
            .await
    }

    async fn op_undo(&self, dir: &Path) -> Result<()> {
        self.core.run_unit(self.cmd_in(dir, ["op", "undo"])).await
    }

    async fn workspace_list(&self, dir: &Path) -> Result<Vec<Workspace>> {
        self.core
            .parse(
                self.cmd_in(dir, ["workspace", "list", "-T", parse::WORKSPACE_TEMPLATE]),
                parse::parse_workspaces,
            )
            .await
    }

    async fn workspace_root(&self, dir: &Path, name: Option<String>) -> Result<PathBuf> {
        // Read-only: the root is static creation-time metadata, so this must not
        // snapshot the working copy — consistent with the batch `workspace_roots` (M10).
        let mut args: Vec<String> = vec![
            "--ignore-working-copy".into(),
            "workspace".into(),
            "root".into(),
        ];
        if let Some(n) = name.as_deref() {
            args.push("--name".into());
            args.push(n.to_string());
        }
        Ok(PathBuf::from(self.core.run(self.cmd_in(dir, args)).await?))
    }

    async fn workspace_add(&self, dir: &Path, spec: WorkspaceAdd) -> Result<()> {
        // Built directly on `command_in` (not `cmd_in`) because the trailing
        // `--color never` must come after the chained value args, not between
        // `--name` and its value.
        let mut command = self
            .core
            .command_in(dir, ["workspace", "add", "--name"])
            .arg(&spec.name)
            .arg("-r")
            .arg(spec.base.as_str());
        if let Some(mode) = spec.sparse_patterns {
            command = command.arg("--sparse-patterns").arg(mode.as_arg());
        }
        command = command.arg(&spec.path).arg("--color").arg("never");
        self.core.run_unit(command).await
    }

    async fn workspace_forget(&self, dir: &Path, name: &str) -> Result<()> {
        reject_flag_like("workspace name", name)?;
        self.core
            .run_unit(self.cmd_in(dir, ["workspace", "forget", name]))
            .await
    }
}

/// Total attempts / fixed backoff for a transient-retried fetch — the shared
/// policy from `vcs-cli-support`, aliased so the retry call sites read locally.
const FETCH_ATTEMPTS: u32 = vcs_cli_support::FETCH_ATTEMPTS;
const FETCH_BACKOFF: Duration = vcs_cli_support::FETCH_BACKOFF;
const FETCH_TIMEOUT_GRACE: Duration = vcs_cli_support::FETCH_TIMEOUT_GRACE;

/// How many `jj workspace root` lookups [`Jj::workspace_roots`] keeps in flight at
/// once — a cap so a repo with many workspaces doesn't spawn an unbounded burst of
/// processes, while still overlapping the (fast, network-free) calls.
const WORKSPACE_ROOTS_CONCURRENCY: usize = 8;

impl<R: ProcessRunner> Jj<R> {
    /// Run `jj <args>` over string slices — `jj.run_args(&["log", "-r", "@"])`
    /// without allocating a `Vec<String>`. Inherent (not on the object-safe
    /// trait), so it can take `&[&str]`; forwards to the same path as
    /// [`JjApi::run`].
    pub async fn run_args(&self, args: &[&str]) -> Result<String> {
        self.core.run(args).await
    }

    /// Resolve several workspaces' root paths in one **bounded fan-out** — one
    /// `jj workspace root --name <n>` per name, at most
    /// `WORKSPACE_ROOTS_CONCURRENCY` (8) live at a time — instead of awaiting each in
    /// turn. Per-name `Ok`/`Err` mirrors [`workspace_root`](JjApi::workspace_root)
    /// (a non-zero exit or spawn failure → `Err`); results come back in `names`
    /// order. Runs through this client's own runner, so a `ScriptedRunner` test
    /// drives it hermetically. Inherent (not on the object-safe trait): it's a
    /// throughput shape over the trait method, and the batch primitive isn't a
    /// mockable per-call seam.
    pub async fn workspace_roots(&self, dir: &Path, names: &[String]) -> Vec<Result<PathBuf>> {
        // `--ignore-working-copy`: read-only metadata probe (often on the Drop-cleanup
        // path), so it must not snapshot/lock the working copy (M10).
        let commands = names.iter().map(|n| {
            self.cmd_in(
                dir,
                [
                    "--ignore-working-copy",
                    "workspace",
                    "root",
                    "--name",
                    n.as_str(),
                ],
            )
        });
        processkit::output_all(commands, WORKSPACE_ROOTS_CONCURRENCY, self.core.runner())
            .await
            .into_iter()
            .map(|r| {
                r.and_then(|pr| pr.ensure_success())
                    // `trim_end` (not `trim`) for exact parity with the single
                    // `workspace_root`, which trims via `core.run`'s `trim_end`.
                    .map(|pr| PathBuf::from(pr.stdout().trim_end()))
            })
            .collect()
    }

    /// Like [`run_args`](Jj::run_args) but never errors on a non-zero exit
    /// (mirrors [`JjApi::run_raw`]).
    pub async fn run_raw_args(&self, args: &[&str]) -> Result<ProcessResult<String>> {
        self.core.output_string(args).await
    }

    /// Bind this client to `dir`, returning a [`JjAt`] handle whose methods omit
    /// the `dir` argument: `jj.at(dir).status()` runs [`status`](JjApi::status)
    /// against `dir`. The dir-taking [`JjApi`] methods stay on [`Jj`] for driving
    /// many directories (e.g. workspaces) from one client.
    pub fn at<'a>(&'a self, dir: &'a Path) -> JjAt<'a, R> {
        JjAt { jj: self, dir }
    }

    /// Run a mutation sequence with op-log rollback: capture the current
    /// operation ([`op_head`](JjApi::op_head)), run `f` with a [`JjAt`] bound to
    /// `dir`, and on `Err` restore the repo to the captured operation
    /// ([`op_restore`](JjApi::op_restore)) before returning the error.
    ///
    /// ```no_run
    /// # async fn demo(jj: &vcs_jj::Jj) -> Result<(), processkit::Error> {
    /// jj.transaction(std::path::Path::new("."), |tx| async move {
    ///     tx.describe("wip").await?;
    ///     tx.new_change("next").await // an Err here rolls back the describe
    /// })
    /// .await?;
    /// # Ok(()) }
    /// ```
    ///
    /// Inherent (not on the object-safe trait): the closure parameter is
    /// generic, which `mockall` / trait objects can't express.
    ///
    /// Caveats:
    /// - **Single-actor.** The rollback is `op_restore <pre>`, which restores the
    ///   **entire** repo view to the captured operation — so a change *another* jj
    ///   process landed (a `describe`, a `bookmark move`, a commit) between the
    ///   `op_head` capture and the restore is **also reverted**, not just `f`'s own
    ///   work. Use this only when one actor drives the repo for the transaction's span.
    /// - Rollback runs on `Err` only — **not** on panic or cancellation (a
    ///   dropped future); there is no async `Drop`. Convert panics to `Err`
    ///   inside `f` if you need that safety.
    /// - **A cancelled `f` also cancels the rollback.** If `f`'s `Err` is a *fired*
    ///   cancellation (on a client built with `default_cancel_on`), the restore is
    ///   dispatched to that same still-cancelled client and short-circuits before it
    ///   spawns — so it is skipped and the repo is left mid-transaction. If you need the
    ///   rollback to survive cancellation, run it yourself (capture `op_head`, then
    ///   `op_restore` on a client **without** the cancel token) instead of this helper.
    /// - If the restore itself fails, the *original* error from `f` is returned
    ///   and the repo may be left mid-transaction; re-probe
    ///   [`op_head`](JjApi::op_head) to detect that.
    ///
    /// **Non-closure / FFI callers**: the borrowed [`JjAt`] and the `'a`-bound
    /// future this closure form takes don't cross an FFI boundary cleanly, so a
    /// language binding replicates the rollback with the public primitives this
    /// method wraps — capture [`op_head`](JjApi::op_head) before the mutations, run
    /// them (through a [`JjAt`] or the dir-taking methods), then on failure call
    /// [`op_restore`](JjApi::op_restore) back to the captured id, best-effort
    /// (don't let its error mask the original — the same caveats apply). Both are
    /// on the object-safe [`JjApi`], so this also works through `&dyn JjApi`.
    pub async fn transaction<'a, T, F, Fut>(&'a self, dir: &'a Path, f: F) -> Result<T>
    where
        F: FnOnce(JjAt<'a, R>) -> Fut,
        Fut: Future<Output = Result<T>> + 'a,
    {
        let pre = self.op_head(dir).await?;
        match f(self.at(dir)).await {
            Ok(value) => Ok(value),
            Err(err) => {
                // Best-effort restore; the closure's error is the cause and is
                // what the caller must see even when the restore also fails.
                let _ = self.op_restore(dir, &pre).await;
                Err(err)
            }
        }
    }
}

/// A [`Jj`] client with a working directory bound, so calls drop the leading
/// `dir` argument — `jj.at(dir).status()` is `jj.status(dir)`. Construct one with
/// [`Jj::at`] (or, through the facade, `vcs_core::Repo::jj_at`). Cheap to copy: it
/// only borrows the client and the path.
pub struct JjAt<'a, R: ProcessRunner = processkit::JobRunner> {
    jj: &'a Jj<R>,
    dir: &'a Path,
}

// Hand-written rather than derived: holding only references, the view is `Copy`
// for *every* runner. `#[derive(Copy)]` would add a spurious `R: Copy` bound the
// default `JobRunner` doesn't satisfy, silently dropping `Copy` on the production
// handle.
impl<R: ProcessRunner> Clone for JjAt<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<R: ProcessRunner> Copy for JjAt<'_, R> {}

// Generate [`JjAt`] forwarders from a method list: `bare` methods forward
// verbatim, `dir` methods inject `self.dir` as the first argument. The shared
// macro lives in `vcs-cli-support` (see `vcs_cli_support::at_forwarders!`).
vcs_cli_support::at_forwarders! {
    JjAt, jj, "Jj",
    bare {
        fn run(args: &[String]) -> Result<String>;
        fn run_raw(args: &[String]) -> Result<ProcessResult<String>>;
        fn run_args(args: &[&str]) -> Result<String>;
        fn run_raw_args(args: &[&str]) -> Result<ProcessResult<String>>;
        fn version() -> Result<String>;
        fn capabilities() -> Result<JjCapabilities>;
        fn git_clone(url: &str, dest: &Path, spec: GitClone) -> Result<()>;
    }
    dir {
        fn status() -> Result<Vec<ChangedPath>>;
        fn status_text() -> Result<String>;
        fn log(revset: &RevsetExpr, max: usize) -> Result<Vec<Change>>;
        fn log_paths(revset: &RevsetExpr, max: usize, filesets: &[JjFileset]) -> Result<Vec<Change>>;
        fn current_change() -> Result<Change>;
        fn describe(message: &str) -> Result<()>;
        fn describe_rev(revset: &RevsetExpr, message: &str) -> Result<()>;
        fn new_change(message: &str) -> Result<()>;
        fn new_child(parent: &RevsetExpr) -> Result<()>;
        fn bookmarks() -> Result<Vec<Bookmark>>;
        fn bookmarks_all() -> Result<Vec<BookmarkRef>>;
        fn reachable_bookmarks() -> Result<Vec<Bookmark>>;
        fn bookmark_track(name: &BookmarkName, remote: &str) -> Result<()>;
        fn bookmark_set(name: &BookmarkName, revision: &RevsetExpr) -> Result<()>;
        fn git_fetch() -> Result<()>;
        fn git_fetch_from(remote: &str) -> Result<()>;
        fn git_push(bookmark: Option<BookmarkName>) -> Result<()>;
        fn root() -> Result<PathBuf>;
        fn current_bookmark() -> Result<Option<String>>;
        fn trunk() -> Result<Option<String>>;
        fn bookmark_create(name: &BookmarkName, revision: &RevsetExpr) -> Result<()>;
        fn bookmark_rename(old: &BookmarkName, new: &BookmarkName) -> Result<()>;
        fn bookmark_delete(name: &BookmarkName) -> Result<()>;
        fn bookmark_move(spec: BookmarkMove) -> Result<()>;
        fn diff_summary(from: &RevsetExpr, to: &RevsetExpr) -> Result<Vec<ChangedPath>>;
        fn diff_stat(revset: &RevsetExpr) -> Result<DiffStat>;
        fn diff_text(spec: DiffSpec) -> Result<String>;
        fn diff(spec: DiffSpec) -> Result<Vec<FileDiff>>;
        fn commit_count(revset: &RevsetExpr) -> Result<usize>;
        fn is_conflicted(revset: &RevsetExpr) -> Result<bool>;
        fn has_workingcopy_conflict() -> Result<bool>;
        fn resolve_list(revset: &RevsetExpr) -> Result<Vec<String>>;
        fn template_query(revset: &RevsetExpr, template: &str, limit: Option<usize>) -> Result<String>;
        fn description(revset: &RevsetExpr) -> Result<String>;
        fn evolog(revset: &RevsetExpr, max: usize) -> Result<Vec<Change>>;
        fn file_annotate(path: &str, revset: Option<RevsetExpr>) -> Result<Vec<AnnotationLine>>;
        fn file_show(revset: &RevsetExpr, path: &str) -> Result<String>;
        fn absorb(from: Option<RevsetExpr>, filesets: &[JjFileset]) -> Result<()>;
        fn split_paths(filesets: &[JjFileset], message: &str) -> Result<()>;
        fn duplicate(revset: &RevsetExpr) -> Result<()>;
        fn rebase(onto: &RevsetExpr) -> Result<()>;
        fn rebase_branch(branch: &RevsetExpr, dest: &RevsetExpr) -> Result<()>;
        fn edit(revset: &RevsetExpr) -> Result<()>;
        fn squash_into(spec: SquashInto) -> Result<()>;
        fn commit_paths(filesets: &[JjFileset], message: &str) -> Result<()>;
        fn squash_paths(spec: SquashPaths) -> Result<()>;
        fn sparse_set(patterns: &[String]) -> Result<()>;
        fn new_merge(message: &str, parents: Vec<RevsetExpr>) -> Result<()>;
        fn abandon(revset: &RevsetExpr) -> Result<()>;
        fn git_fetch_branch(branch: &BookmarkName) -> Result<()>;
        fn git_import() -> Result<()>;
        fn op_head() -> Result<String>;
        fn op_log(limit: usize) -> Result<Vec<Operation>>;
        fn op_restore(op_id: &str) -> Result<()>;
        fn op_undo() -> Result<()>;
        fn workspace_list() -> Result<Vec<Workspace>>;
        fn workspace_root(name: Option<String>) -> Result<PathBuf>;
        fn workspace_add(spec: WorkspaceAdd) -> Result<()>;
        fn workspace_forget(name: &str) -> Result<()>;
    }
}

// Manual forwarder: `transaction` takes a generic closure, which the declarative
// forwarder macro (fixed argument lists) cannot express.
impl<'a, R: ProcessRunner> JjAt<'a, R> {
    /// Bound form of [`Jj::transaction`] (with `dir` pre-bound): run `f` with
    /// op-log rollback on `Err`. See [`Jj::transaction`] for the caveats.
    pub async fn transaction<T, F, Fut>(&self, f: F) -> Result<T>
    where
        F: FnOnce(JjAt<'a, R>) -> Fut,
        Fut: Future<Output = Result<T>> + 'a,
    {
        self.jj.transaction(self.dir, f).await
    }
}

/// Synchronous, best-effort helpers for contexts that cannot `.await` — chiefly
/// a `Drop` guard. They shell out through `std::process` directly (no async, no
/// job-containment), so reserve them for short-lived cleanup.
pub mod blocking {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Forget a workspace synchronously (`jj workspace forget <name>`).
    pub fn workspace_forget(dir: &Path, name: &str) -> std::io::Result<()> {
        let status = Command::new(super::BINARY)
            .current_dir(dir)
            .args(["workspace", "forget", name])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "`jj workspace forget` exited with {status}"
            )))
        }
    }

    /// Resolve the workspace *name* whose root matches `path`, synchronously —
    /// for `Drop`, which can't `.await` the typed `workspace_list`/`workspace_root`.
    /// Lists workspaces (`workspace list -T name`), then matches each
    /// `workspace root --name <n>` against `path` (canonicalised, Windows
    /// verbatim-prefix stripped). `None` when jj is missing or nothing matches —
    /// the caller then skips the forget rather than guessing.
    pub fn workspace_name_for_path(dir: &Path, path: &Path) -> Option<String> {
        let target = normalize(path);
        let out = Command::new(super::BINARY)
            .current_dir(dir)
            // `--ignore-working-copy`: this is a **read-only** probe run from a Drop
            // guard, so it must NOT snapshot the working copy — a plain `workspace
            // list` takes the working-copy lock and writes a snapshot op (M10),
            // mutating the very repo being cleaned up and failing (→ leak) under lock
            // contention. The workspace list/root are static metadata, unaffected.
            // `--color never`: this raw probe bypasses `cmd_in`, so pin it here too
            // — `ui.color = "always"` would otherwise wrap names in ANSI escapes
            // and break the name->root match below (leaking the workspace on Drop).
            .args([
                "--ignore-working-copy",
                "workspace",
                "list",
                "-T",
                "name ++ \"\\n\"",
                "--color",
                "never",
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        for name in String::from_utf8_lossy(&out.stdout).lines() {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let root = Command::new(super::BINARY)
                .current_dir(dir)
                .args([
                    "--ignore-working-copy",
                    "workspace",
                    "root",
                    "--name",
                    name,
                    "--color",
                    "never",
                ])
                .output();
            if let Ok(r) = root
                && r.status.success()
            {
                let p = PathBuf::from(String::from_utf8_lossy(&r.stdout).trim().to_string());
                if normalize(&p) == target || p == target || p == path {
                    return Some(name.to_string());
                }
            }
        }
        None
    }

    /// Canonicalise + strip the Windows verbatim prefix (`\\?\…`, which
    /// `canonicalize` adds but jj never emits) for stable path comparison.
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::testing::{RecordingRunner, Reply, ScriptedRunner};

    // Terse constructors for the validated newtypes in test call sites; the
    // literals here are always valid, so `unwrap` is fine in tests.
    fn rv(s: &str) -> RevsetExpr {
        RevsetExpr::new(s).unwrap()
    }
    fn bn(s: &str) -> BookmarkName {
        BookmarkName::new(s).unwrap()
    }

    #[test]
    fn binary_name_is_jj() {
        assert_eq!(BINARY, "jj");
    }

    // Compile-time guard: the bound view stays `Copy` for the default `JobRunner`.
    #[allow(dead_code)]
    fn bound_view_is_copy_for_default_runner() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<JjAt<'static, processkit::JobRunner>>();
    }

    // The bound view (`jj.at(dir)`) must produce byte-identical argv to the
    // dir-taking call — including the forced `--color never`.
    #[tokio::test]
    async fn bound_view_matches_dir_taking_calls() {
        let dir = Path::new("/repo");
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);

        jj.bookmark_move(dir, BookmarkMove::new(bn("main"), rv("@")).allow_backwards())
            .await
            .unwrap();
        jj.at(dir)
            .bookmark_move(BookmarkMove::new(bn("main"), rv("@")).allow_backwards())
            .await
            .unwrap();
        jj.describe_rev(dir, &rv("feat"), "msg").await.unwrap();
        jj.at(dir).describe_rev(&rv("feat"), "msg").await.unwrap();
        jj.description(dir, &rv("@-")).await.unwrap();
        jj.at(dir).description(&rv("@-")).await.unwrap();
        // One of the §4 additions.
        jj.duplicate(dir, &rv("@-")).await.unwrap();
        jj.at(dir).duplicate(&rv("@-")).await.unwrap();

        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), calls[1].args_str());
        assert_eq!(calls[2].args_str(), calls[3].args_str());
        assert_eq!(calls[4].args_str(), calls[5].args_str());
        assert_eq!(calls[6].args_str(), calls[7].args_str());
        assert_eq!(calls[1].cwd.as_deref(), Some(dir));
    }

    #[tokio::test]
    async fn workspace_list_parses_template_rows() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(
            ["jj", "workspace", "list"],
            Reply::ok("default\te2aa3420\tmain\nws1\t12345678\t\n"),
        ));
        let got = jj.workspace_list(Path::new(".")).await.expect("list");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "default");
        assert_eq!(got[0].bookmarks, vec!["main".to_string()]);
        assert!(got[1].bookmarks.is_empty());
    }

    // `workspace_roots` fans out one `workspace root --name <n>` per name, returns
    // a path per slot in input order, and maps a non-zero exit to `Err` for that
    // slot (mirroring the single `workspace_root`). Runs through the scripted
    // runner, so it's hermetic.
    #[tokio::test]
    async fn workspace_roots_batches_per_name_and_maps_errors() {
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(
                    [
                        "jj",
                        "--ignore-working-copy",
                        "workspace",
                        "root",
                        "--name",
                        "default",
                    ],
                    Reply::ok("/repo\n"),
                )
                .on(
                    [
                        "jj",
                        "--ignore-working-copy",
                        "workspace",
                        "root",
                        "--name",
                        "ws1",
                    ],
                    Reply::ok("/repo/ws1\n"),
                )
                .on(
                    [
                        "jj",
                        "--ignore-working-copy",
                        "workspace",
                        "root",
                        "--name",
                        "gone",
                    ],
                    Reply::fail(1, "Error: No such workspace"),
                ),
        );
        let jj = Jj::with_runner(&rec);
        let roots = jj
            .workspace_roots(
                Path::new("/repo"),
                &["default".into(), "gone".into(), "ws1".into()],
            )
            .await;
        // Order matches the input, regardless of completion order.
        assert_eq!(roots.len(), 3);
        assert_eq!(roots[0].as_deref().unwrap(), Path::new("/repo"));
        assert!(roots[1].is_err(), "a non-zero `workspace root` is Err");
        assert_eq!(roots[2].as_deref().unwrap(), Path::new("/repo/ws1"));
        // Exactly one read-only `--ignore-working-copy workspace root --name <n>`
        // command per name (M10: the metadata probe must not snapshot the copy).
        let calls = rec.calls();
        assert_eq!(calls.len(), 3);
        assert!(
            calls
                .iter()
                .all(|c| c.args_str()[..3] == ["--ignore-working-copy", "workspace", "root"])
        );
    }

    // `workspace add` must build `--name <n> -r <base> <path>` in order.
    #[tokio::test]
    async fn workspace_add_builds_name_base_path() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.workspace_add(Path::new("/repo"), WorkspaceAdd::new("ws1", rv("main"), "/wt"))
            .await
            .expect("workspace add");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "workspace",
                "add",
                "--name",
                "ws1",
                "-r",
                "main",
                "/wt",
                "--color",
                "never"
            ]
        );
    }

    // `--sparse-patterns <mode>` lands between `-r <base>` and the path.
    #[tokio::test]
    async fn workspace_add_with_sparse_mode() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.workspace_add(
            Path::new("/repo"),
            WorkspaceAdd::new("ws1", rv("main"), "/wt").sparse(SparseMode::Empty),
        )
        .await
        .expect("workspace add");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "workspace",
                "add",
                "--name",
                "ws1",
                "-r",
                "main",
                "--sparse-patterns",
                "empty",
                "/wt",
                "--color",
                "never"
            ]
        );
    }

    #[test]
    fn fileset_quotes_metacharacters() {
        assert_eq!(
            JjFileset::path("src/a(b).rs").as_str(),
            "root-file:\"src/a(b).rs\""
        );
    }

    #[test]
    fn fileset_escapes_double_quote() {
        assert_eq!(JjFileset::path("a\"b").as_str(), "root-file:\"a\\\"b\"");
    }

    // M2: the fileset uses jj's `root-file:` anchor (workspace-root-relative), NOT the
    // cwd-relative `file:` — so a command run from a subdirectory (`dir` ≠ workspace
    // root) targets the intended root-relative path rather than a same-named file under
    // `dir`. (jj resolves `root-file:"x"` from the workspace root; `file:"x"` from cwd.)
    #[test]
    fn fileset_is_workspace_root_relative() {
        assert!(
            JjFileset::path("src/a.rs")
                .as_str()
                .starts_with("root-file:\"")
        );
        assert!(!JjFileset::path("src/a.rs").as_str().starts_with("file:"));
    }

    // M4: the `\`→`/` rewrite is Windows-only. On Windows a `\` is a path separator
    // (normalise it so jj matches); on Unix `\` is a legitimate filename byte and must
    // be preserved verbatim, else a real path is corrupted.
    #[test]
    #[cfg(windows)]
    fn fileset_normalises_backslash_on_windows() {
        assert_eq!(
            JjFileset::path("src\\a.rs").as_str(),
            "root-file:\"src/a.rs\""
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn fileset_escapes_backslashes_on_unix() {
        assert_eq!(
            JjFileset::path("a\\b.txt").as_str(),
            "root-file:\"a\\\\b.txt\""
        );
        assert_eq!(JjFileset::path("a\\").as_str(), "root-file:\"a\\\\\"");
        assert_eq!(
            JjFileset::path("a\\b\"c.txt").as_str(),
            "root-file:\"a\\\\b\\\"c.txt\""
        );
    }

    #[tokio::test]
    async fn commit_paths_builds_filesets() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.commit_paths(
            Path::new("."),
            &[JjFileset::path("x|y.rs"), JjFileset::path("z.rs")],
            "msg",
        )
        .await
        .expect("commit_paths");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "commit",
                "-m",
                "msg",
                "root-file:\"x|y.rs\"",
                "root-file:\"z.rs\"",
                "--color",
                "never"
            ]
        );
    }

    #[tokio::test]
    async fn squash_paths_builds_from_into_filesets() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.squash_paths(
            Path::new("."),
            SquashPaths::new(rv("@"), rv("feat")).filesets([JjFileset::path("a.rs")]),
        )
        .await
        .expect("squash_paths");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "squash",
                "--from",
                "@",
                "--into",
                "feat",
                "root-file:\"a.rs\"",
                "--color",
                "never"
            ]
        );
    }

    #[tokio::test]
    async fn squash_paths_keeps_destination_message() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.squash_paths(
            Path::new("."),
            SquashPaths::new(rv("@"), rv("feat"))
                .filesets([JjFileset::path("a.rs")])
                .use_destination_message(),
        )
        .await
        .expect("squash_paths");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "squash",
                "--from",
                "@",
                "--into",
                "feat",
                "--use-destination-message",
                "root-file:\"a.rs\"",
                "--color",
                "never"
            ]
        );
    }

    #[tokio::test]
    async fn jj_new_revision_scoped_ops_build_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.describe_rev(Path::new("."), &rv("feat"), "msg")
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["describe", "-r", "feat", "-m", "msg", "--color", "never"]
        );

        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.rebase_branch(Path::new("."), &rv("feat"), &rv("main"))
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["rebase", "-b", "feat", "-d", "main", "--color", "never"]
        );

        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.bookmark_track(Path::new("."), &bn("feat"), "origin")
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["bookmark", "track", "exact:feat@origin", "--color", "never"]
        );
    }

    #[tokio::test]
    async fn bookmark_track_rejects_glob_like_remote() {
        // Unlike the bookmark segment, the remote segment of jj's positional
        // `<name>@<remote>` pattern isn't itself pattern-syntax — wrapping it
        // in `exact:` would silently no-op (see `reject_glob_like`'s doc
        // comment) rather than exact-match, so a glob-bearing remote must be
        // rejected before spawn instead.
        for remote in ["*", "o?igin", "[origin]"] {
            let rec = RecordingRunner::replying(Reply::ok(""));
            let jj = Jj::with_runner(&rec);
            assert!(
                jj.bookmark_track(Path::new("."), &bn("main"), remote)
                    .await
                    .is_err(),
                "remote {remote:?} should be rejected before spawn"
            );
            assert!(
                rec.calls().is_empty(),
                "must not spawn for remote {remote:?}"
            );
        }
    }

    #[tokio::test]
    async fn bookmarks_uses_template_and_parses_rows() {
        let rec = RecordingRunner::replying(Reply::ok("main\tabc123\nfeature\tdef456\n"));
        let jj = Jj::with_runner(&rec);
        let marks = jj.bookmarks(Path::new(".")).await.unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            [
                "bookmark",
                "list",
                "-T",
                parse::BOOKMARK_LIST_TEMPLATE,
                "--color",
                "never"
            ]
        );
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].name, "main");
        assert_eq!(marks[0].target, "abc123");
        assert_eq!(marks[1].name, "feature");
    }

    #[tokio::test]
    async fn bookmarks_all_parses_local_and_remote() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(
            ["jj", "bookmark", "list"],
            Reply::ok("main\t\t0\tabc123\nmain\torigin\t1\tabc123\n"),
        ));
        let refs = jj.bookmarks_all(Path::new(".")).await.unwrap();
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].name, "main");
        assert!(refs[0].remote.is_none() && !refs[0].tracked);
        assert_eq!(refs[1].remote.as_deref(), Some("origin"));
        assert!(refs[1].tracked);
    }

    #[tokio::test]
    async fn sparse_set_clears_then_adds() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.sparse_set(Path::new("."), &["README.md".into(), "lib".into()])
            .await
            .expect("sparse_set");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "sparse",
                "set",
                "--clear",
                "--add",
                "README.md",
                "--add",
                "lib",
                "--color",
                "never"
            ]
        );
    }

    // Parsed status() is backed by `diff -r @ --summary`, not `jj status`.
    #[tokio::test]
    async fn status_parses_diff_summary() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(
            ["jj", "diff", "-r", "@", "--summary"],
            Reply::ok("M a.rs\nA b.rs\n"),
        ));
        let entries = jj.status(Path::new(".")).await.expect("status");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].status, 'M');
        assert_eq!(entries[1].path, "b.rs");
    }

    #[tokio::test]
    async fn status_text_is_raw_jj_status() {
        let jj = Jj::with_runner(
            ScriptedRunner::new().on(["jj", "status"], Reply::ok("Working copy changes:\n")),
        );
        assert!(
            jj.status_text(Path::new("."))
                .await
                .expect("status_text")
                .contains("Working copy changes")
        );
    }

    #[tokio::test]
    async fn run_args_forwards_str_slices() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(["jj", "root"], Reply::ok("/r\n")));
        assert_eq!(jj.run_args(&["root"]).await.unwrap(), "/r");
    }

    #[tokio::test]
    async fn bookmark_move_appends_allow_backwards() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.bookmark_move(
            Path::new("/r"),
            BookmarkMove::new(bn("main"), rv("@")).allow_backwards(),
        )
        .await
        .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            [
                "bookmark",
                "move",
                "exact:main",
                "--to",
                "@",
                "--allow-backwards",
                "--color",
                "never"
            ]
        );
    }

    // The default spec omits `--allow-backwards`.
    #[tokio::test]
    async fn bookmark_move_default_omits_allow_backwards() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.bookmark_move(Path::new("/r"), BookmarkMove::new(bn("main"), rv("@")))
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            [
                "bookmark",
                "move",
                "exact:main",
                "--to",
                "@",
                "--color",
                "never"
            ]
        );
    }

    // `squash_into` builds `squash --into <rev>`; the spec's setter appends
    // `--use-destination-message` (after the forced `--color never`, which
    // `cmd_in` adds before the trailing setter — order is functionally irrelevant
    // to jj).
    #[tokio::test]
    async fn squash_into_builds_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.squash_into(Path::new("/r"), SquashInto::new(rv("@-")))
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["squash", "--into", "@-", "--color", "never"]
        );

        let flagged = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&flagged);
        jj.squash_into(
            Path::new("/r"),
            SquashInto::new(rv("@-")).use_destination_message(),
        )
        .await
        .unwrap();
        assert_eq!(
            flagged.only_call().args_str(),
            [
                "squash",
                "--into",
                "@-",
                "--color",
                "never",
                "--use-destination-message"
            ]
        );
    }

    #[tokio::test]
    async fn new_merge_appends_parents() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.new_merge(Path::new("/r"), "m", vec![rv("p1"), rv("p2")])
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["new", "-m", "m", "p1", "p2", "--color", "never"]
        );
    }

    #[tokio::test]
    async fn is_conflicted_reads_template_flag() {
        let yes = Jj::with_runner(ScriptedRunner::new().on(["jj", "log"], Reply::ok("1\n")));
        assert!(yes.is_conflicted(Path::new("."), &rv("@")).await.unwrap());
        let no = Jj::with_runner(ScriptedRunner::new().on(["jj", "log"], Reply::ok("0\n")));
        assert!(!no.is_conflicted(Path::new("."), &rv("@")).await.unwrap());
    }

    #[tokio::test]
    async fn commit_count_counts_template_lines() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(["jj", "log"], Reply::ok("a\nb\nc\n")));
        assert_eq!(jj.commit_count(Path::new("."), &rv("::@")).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn reachable_bookmarks_queries_heads_revset() {
        let rec = RecordingRunner::replying(Reply::ok("main\tabc123\n"));
        let jj = Jj::with_runner(&rec);
        let got = jj.reachable_bookmarks(Path::new(".")).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "main");
        let args = rec.only_call().args_str();
        assert_eq!(
            &args[..4],
            &["log", "-r", "heads(::@ & bookmarks())", "--no-graph"]
        );
    }

    #[tokio::test]
    async fn resolve_list_distinguishes_no_conflicts_from_errors() {
        // The benign "no conflicts" non-zero exit → empty list.
        let none = Jj::with_runner(ScriptedRunner::new().on(
            ["jj", "resolve"],
            Reply::fail(2, "Error: No conflicts found at this revision"),
        ));
        assert!(
            none.resolve_list(Path::new("."), &rv("@"))
                .await
                .unwrap()
                .is_empty()
        );
        // A real failure (e.g. bad revset) must surface, not read as "no conflicts".
        let bad = Jj::with_runner(ScriptedRunner::new().on(
            ["jj", "resolve"],
            Reply::fail(1, "Error: Revision `bogus` doesn't exist"),
        ));
        assert!(bad.resolve_list(Path::new("."), &rv("bogus")).await.is_err());
        // Success with conflicts → parsed paths.
        let some = Jj::with_runner(
            ScriptedRunner::new().on(["jj", "resolve"], Reply::ok("a.rs    2-sided conflict\n")),
        );
        assert_eq!(
            some.resolve_list(Path::new("."), &rv("@")).await.unwrap(),
            ["a.rs"]
        );
    }

    #[tokio::test]
    async fn current_bookmark_takes_first_or_none() {
        let some = Jj::with_runner(ScriptedRunner::new().on(["jj", "log"], Reply::ok("main\n")));
        assert_eq!(
            some.current_bookmark(Path::new("."))
                .await
                .unwrap()
                .as_deref(),
            Some("main")
        );
        let none = Jj::with_runner(ScriptedRunner::new().on(["jj", "log"], Reply::ok("\n")));
        assert!(
            none.current_bookmark(Path::new("."))
                .await
                .unwrap()
                .is_none()
        );
    }

    // Hermetic: real log() arg-building + template parsing against canned output.
    #[tokio::test]
    async fn current_change_parses_scripted_output() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(
            ["jj", "log"],
            Reply::ok("kztuxlro\t38e00654\tfalse\thello jj\n"),
        ));
        let change = jj
            .current_change(Path::new("."))
            .await
            .expect("current_change");
        assert_eq!(change.change_id, "kztuxlro");
        assert!(!change.empty);
        assert_eq!(change.description, "hello jj");
    }

    // With a bookmark, the run must build `git push -b exact:<name>` (the `exact:`
    // prefix disables jj's glob so a `*` can't push every bookmark — H1). Only that
    // command is scripted (no fallback), so a regression that dropped the flag or
    // the `exact:` prefix would match no rule and error.
    #[tokio::test]
    async fn git_push_appends_bookmark_flag() {
        let jj = Jj::with_runner(
            ScriptedRunner::new().on(["jj", "git", "push", "-b", "exact:feature"], Reply::ok("")),
        );
        jj.git_push(Path::new("."), Some(bn("feature")))
            .await
            .expect("should build `git push -b exact:feature`");
    }

    // Without a bookmark, the run is a bare `git push`.
    #[tokio::test]
    async fn git_push_without_bookmark_is_bare() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(["jj", "git", "push"], Reply::ok("")));
        jj.git_push(Path::new("."), None).await.expect("bare push");
    }

    // H1: `bookmark delete` and `git fetch -b` pass the name through `exact:` so a
    // `*` can't mass-delete/fetch. (The other exact: methods are covered by
    // git_push/bookmark_move/bookmark_track/git_fetch_from tests.)
    #[tokio::test]
    async fn bookmark_delete_and_fetch_branch_use_exact() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.bookmark_delete(Path::new("."), &bn("foo")).await.unwrap();
        assert_eq!(
            &rec.only_call().args_str()[..3],
            &["bookmark", "delete", "exact:foo"]
        );

        let rec2 = RecordingRunner::replying(Reply::ok(""));
        let jj2 = Jj::with_runner(&rec2);
        jj2.git_fetch_branch(Path::new("."), &bn("foo")).await.unwrap();
        assert_eq!(
            &rec2.only_call().args_str()[..6],
            &["git", "fetch", "--remote", "origin", "-b", "exact:foo"]
        );
        // M28: pinned C locale so a localized transient marker still classifies.
        assert!(
            rec2.only_call().envs.iter().any(|(k, v)| {
                k.to_str() == Some("LC_ALL") && v.as_deref().and_then(|s| s.to_str()) == Some("C")
            }),
            "git_fetch_branch must pin LC_ALL=C"
        );
    }

    // `git_fetch` retries a transient (network) failure up to FETCH_ATTEMPTS times.
    #[tokio::test]
    async fn git_fetch_retries_transient_failures() {
        let rec = RecordingRunner::replying(Reply::fail(1, "Error: Could not resolve host: x"));
        let jj = Jj::with_runner(&rec);
        assert!(jj.git_fetch(Path::new(".")).await.is_err());
        assert_eq!(rec.calls().len(), FETCH_ATTEMPTS as usize);
        // M28: the fetch runs under LC_ALL=C, so a localized libc/gai transient marker
        // ("Temporary failure in name resolution") still classifies as retryable.
        assert!(
            rec.calls()[0].envs.iter().any(|(k, v)| {
                k.to_str() == Some("LC_ALL") && v.as_deref().and_then(|s| s.to_str()) == Some("C")
            }),
            "git fetch must pin LC_ALL=C"
        );
    }

    // Opt-in lock-contention retry mirrors `vcs-git`: a mutation that fails on jj's
    // working-copy lock is retried and succeeds; off by default. (Zero backoff → no
    // sleep in the test.)
    #[tokio::test]
    async fn with_retry_retries_lock_contention_on_a_mutation() {
        let rec = RecordingRunner::new(ScriptedRunner::new().on_sequence(
            ["jj", "abandon"],
            [
                Reply::fail(1, "Error: Failed to lock working copy"),
                Reply::ok(""),
            ],
        ));
        let jj = Jj::with_runner(&rec).with_retry(RetryPolicy::none().attempts(3));
        jj.abandon(Path::new("."), &rv("@-"))
            .await
            .expect("retried past the lock");
        assert_eq!(rec.calls().len(), 2, "one retry after the lock failure");

        // Off by default.
        let rec = RecordingRunner::new(ScriptedRunner::new().on_sequence(
            ["jj", "abandon"],
            [
                Reply::fail(1, "Error: Failed to lock working copy"),
                Reply::ok(""),
            ],
        ));
        let jj = Jj::with_runner(&rec);
        assert!(jj.abandon(Path::new("."), &rv("@-")).await.is_err());
        assert_eq!(rec.calls().len(), 1, "no retry without with_retry");
    }

    // `git_fetch_from` names the remote and shares `git_fetch`'s transient retry.
    #[tokio::test]
    async fn git_fetch_from_builds_args_and_retries() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.git_fetch_from(Path::new("."), "upstream")
            .await
            .expect("git_fetch_from");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "git",
                "fetch",
                "--remote",
                "exact:upstream",
                "--color",
                "never"
            ]
        );
        // M28: pinned C locale so a localized transient marker still classifies.
        assert!(
            rec.only_call().envs.iter().any(|(k, v)| {
                k.to_str() == Some("LC_ALL") && v.as_deref().and_then(|s| s.to_str()) == Some("C")
            }),
            "git_fetch_from must pin LC_ALL=C"
        );

        let failing = RecordingRunner::replying(Reply::fail(1, "Error: Connection timed out"));
        let jj = Jj::with_runner(&failing);
        assert!(jj.git_fetch_from(Path::new("."), "upstream").await.is_err());
        assert_eq!(failing.calls().len(), FETCH_ATTEMPTS as usize);
    }

    // `transaction` captures the op head and restores it when the closure errors —
    // and the original (closure) error is what surfaces.
    #[tokio::test]
    async fn transaction_restores_op_head_on_error() {
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["jj", "op", "log"], Reply::ok("abc123\n"))
                .on(["jj", "op", "restore"], Reply::ok(""))
                .on(["jj", "describe"], Reply::fail(1, "boom")),
        );
        let jj = Jj::with_runner(&rec);
        let res = jj
            .transaction(
                Path::new("/r"),
                |tx| async move { tx.describe("wip").await },
            )
            .await;
        let err = res.expect_err("closure error must surface");
        assert!(matches!(err, Error::Exit { .. }));
        let calls = rec.calls();
        assert_eq!(calls.len(), 3, "op head, mutation, restore: {calls:?}");
        assert_eq!(calls[0].args_str()[..2], ["op", "log"]);
        assert_eq!(calls[1].args_str()[0], "describe");
        assert_eq!(calls[2].args_str()[..3], ["op", "restore", "abc123"]);
    }

    // A successful transaction must NOT restore (that would undo the work).
    #[tokio::test]
    async fn transaction_keeps_changes_on_success() {
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["jj", "op", "log"], Reply::ok("abc123\n"))
                .on(["jj", "describe"], Reply::ok("")),
        );
        let jj = Jj::with_runner(&rec);
        jj.transaction(
            Path::new("/r"),
            |tx| async move { tx.describe("wip").await },
        )
        .await
        .expect("transaction");
        let calls = rec.calls();
        assert_eq!(calls.len(), 2);
        assert!(
            calls.iter().all(|c| c.args_str()[..2] != ["op", "restore"]),
            "no restore on success: {calls:?}"
        );
    }

    // The bound view forwards `transaction` with `dir` pre-bound.
    #[tokio::test]
    async fn bound_view_forwards_transaction() {
        let dir = Path::new("/repo");
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["jj", "op", "log"], Reply::ok("op9\n"))
                .on(["jj", "new"], Reply::ok("")),
        );
        let jj = Jj::with_runner(&rec);
        jj.at(dir)
            .transaction(|tx| async move { tx.new_change("x").await })
            .await
            .expect("transaction");
        assert_eq!(rec.calls()[1].cwd.as_deref(), Some(dir));
    }

    // The injection barrier now has two tiers:
    //  1. bookmark names and revsets are validated NEWTYPES, so a flag-like or
    //     malformed value is rejected at *construction* — it can never reach an
    //     argv slot (migration test below); and
    //  2. the remaining bare-positional `&str` inputs that are not
    //     bookmarks/revsets (operation ids, workspace names, URLs) keep the
    //     internal guard, refused before anything spawns.

    // Tier 1 — the newtypes reject the flag-like / malformed values the typed ops
    // would otherwise have received, as a classifiable invalid-input error.
    #[test]
    fn validated_bookmark_and_revset_newtypes_reject_bad_values() {
        for bad in ["", "-evil", "--all", "-bad", "--config=x", "-r"] {
            let b = BookmarkName::new(bad).expect_err("bookmark name must be rejected");
            assert!(vcs_cli_support::is_invalid_input(&b), "bookmark {bad:?}");
            let r = RevsetExpr::new(bad).expect_err("revset must be rejected");
            assert!(vcs_cli_support::is_invalid_input(&r), "revset {bad:?}");
        }
        // Legitimate values construct fine.
        assert!(BookmarkName::new("feature/x").is_ok());
        assert!(RevsetExpr::new("heads(::@ & bookmarks())").is_ok());
    }

    // Tier 2 — the ops that still take a bare `&str` (operation ids, workspace
    // names, URLs) refuse a flag-like value BEFORE anything spawns.
    #[tokio::test]
    async fn str_positionals_are_rejected_before_spawning() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        let dir = Path::new("/r");

        assert!(jj.op_restore(dir, "--help").await.is_err());
        assert!(jj.workspace_forget(dir, "-evil").await.is_err());
        assert!(
            jj.git_clone("-evil", dir, GitClone::separate())
                .await
                .is_err()
        );

        assert!(
            rec.calls().is_empty(),
            "nothing may spawn: {:?}",
            rec.calls()
        );
    }

    // A legitimate revset still flows through the typed path unchanged.
    #[tokio::test]
    async fn typed_edit_passes_through() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.edit(Path::new("/r"), &rv("abc123")).await.expect("edit");
        assert_eq!(
            rec.only_call().args_str(),
            ["edit", "abc123", "--color", "never"]
        );
    }

    #[test]
    fn revset_expr_validates() {
        assert!(RevsetExpr::new("heads(::@ & bookmarks())").is_ok());
        assert_eq!(RevsetExpr::new("@-").unwrap().as_str(), "@-");
        assert!(RevsetExpr::new("-evil").is_err());
        assert!(RevsetExpr::new("").is_err());
    }

    // capabilities parses jj's version line (incl. dev-build suffixes) and
    // gates precisely on the validated 0.38 floor.
    #[tokio::test]
    async fn capabilities_parse_and_gate_versions() {
        let jj = Jj::with_runner(
            ScriptedRunner::new().on(["jj", "--version"], Reply::ok("jj 0.38.0\n")),
        );
        let caps = jj.capabilities().await.expect("capabilities");
        assert!(caps.is_supported());
        caps.ensure_supported().expect("supported");

        // A dev-build suffix parses; an older release fails the precise gate.
        let dev = Jj::with_runner(
            ScriptedRunner::new().on(["jj", "--version"], Reply::ok("jj 0.39.0-dev+abc123\n")),
        );
        assert!(dev.capabilities().await.unwrap().is_supported());

        let old = Jj::with_runner(
            ScriptedRunner::new().on(["jj", "--version"], Reply::ok("jj 0.35.0\n")),
        );
        let caps = old.capabilities().await.expect("capabilities");
        assert!(!caps.is_supported());
        let err = caps.ensure_supported().expect_err("unsupported");
        // The message must name both the floor and the found version.
        let Error::Spawn { source, .. } = &err else {
            panic!("expected Spawn, got {err:?}");
        };
        let message = source.to_string();
        assert!(message.contains("0.38.0"), "names the floor: {message}");
        assert!(
            message.contains("0.35.0"),
            "names the found version: {message}"
        );

        let garbage =
            Jj::with_runner(ScriptedRunner::new().on(["jj", "--version"], Reply::ok("nope")));
        assert!(matches!(
            garbage.capabilities().await.unwrap_err(),
            Error::Parse { .. }
        ));
    }

    // git_clone is dir-less; the colocate flag is ALWAYS explicit (jj's default
    // varies by version/config) and `--color never` still lands at the very end.
    #[tokio::test]
    async fn git_clone_builds_dirless_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.git_clone("https://x/r.git", Path::new("/dest"), GitClone::colocated())
            .await
            .expect("clone");
        let call = rec.only_call();
        assert_eq!(
            call.args_str(),
            [
                "git",
                "clone",
                "https://x/r.git",
                "/dest",
                "--colocate",
                "--color",
                "never"
            ]
        );
        assert_eq!(call.cwd, None, "clone runs without a working directory");

        let plain = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&plain);
        jj.git_clone("u", Path::new("/d"), GitClone::separate())
            .await
            .unwrap();
        let call = plain.only_call();
        assert!(call.has_flag("--no-colocate"), "explicit either way");
        assert!(!call.has_flag("--colocate"));
    }

    // R7 (mirrors vcs-git): a failed `git_clone` cleans a `dest` it could have created
    // (absent/empty) so a retry isn't blocked, but never a non-empty pre-existing dir
    // (the caller's data). Scripted-fail clone + real temp dirs.
    #[tokio::test]
    async fn git_clone_failure_cleans_only_a_dest_it_could_have_created() {
        use vcs_testkit::TempDir;
        let tmp = TempDir::new("r7-jj-clone");
        let jj = Jj::with_runner(ScriptedRunner::new().on(
            ["jj", "git", "clone"],
            Reply::fail(1, "Error: fetch failed"),
        ));

        // A non-empty caller dir must survive.
        let occupied = tmp.path().join("occupied");
        std::fs::create_dir(&occupied).unwrap();
        std::fs::write(occupied.join("keep.txt"), b"caller data").unwrap();
        assert!(
            jj.git_clone("https://x/r", &occupied, GitClone::separate())
                .await
                .is_err()
        );
        assert!(
            occupied.join("keep.txt").exists(),
            "a non-empty caller dir must survive a failed jj clone"
        );

        // An empty dest we could have populated is removed on failure.
        let empty = tmp.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert!(
            jj.git_clone("https://x/r", &empty, GitClone::separate())
                .await
                .is_err()
        );
        assert!(
            !empty.exists(),
            "an empty dest is cleaned so a retry isn't blocked"
        );
    }

    #[tokio::test]
    async fn absorb_and_split_build_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.absorb(Path::new("/r"), None, &[]).await.unwrap();
        jj.absorb(Path::new("/r"), Some(rv("@-")),
            &[JjFileset::path("src/a.rs")],
        )
        .await
        .unwrap();
        jj.split_paths(Path::new("/r"), &[JjFileset::path("b.rs")], "split out b")
            .await
            .unwrap();
        jj.duplicate(Path::new("/r"), &rv("@-")).await.unwrap();
        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), ["absorb", "--color", "never"]);
        assert_eq!(
            calls[1].args_str(),
            [
                "absorb",
                "--from",
                "@-",
                "root-file:\"src/a.rs\"",
                "--color",
                "never"
            ]
        );
        assert_eq!(
            calls[2].args_str(),
            [
                "split",
                "-m",
                "split out b",
                "root-file:\"b.rs\"",
                "--color",
                "never"
            ]
        );
        assert_eq!(calls[3].args_str(), ["duplicate", "@-", "--color", "never"]);
    }

    // An empty split would open jj's interactive diff editor and hang headless —
    // it must be refused BEFORE any process spawns.
    #[tokio::test]
    async fn split_paths_refuses_empty_filesets_without_spawning() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        let err = jj
            .split_paths(Path::new("/r"), &[], "msg")
            .await
            .expect_err("empty filesets must be refused");
        assert!(matches!(err, Error::Spawn { .. }), "got {err:?}");
        assert!(rec.calls().is_empty(), "nothing may spawn");
    }

    // M7: an empty fileset slice must NOT degrade to a bare `jj commit` (which would
    // commit the whole working copy) — it's refused before any spawn.
    #[tokio::test]
    async fn commit_paths_refuses_empty_filesets_without_spawning() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        let err = jj
            .commit_paths(Path::new("/r"), &[], "msg")
            .await
            .expect_err("empty filesets must be refused");
        assert!(matches!(err, Error::Spawn { .. }), "got {err:?}");
        assert!(rec.calls().is_empty(), "nothing may spawn");
    }

    #[tokio::test]
    async fn log_paths_builds_revset_template_and_filesets() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.log_paths(Path::new("."), &rv("main..@"),
            5,
            &[JjFileset::path("x|y.rs"), JjFileset::path("z.rs")],
        )
        .await
        .expect("log_paths");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "log",
                "-r",
                "main..@",
                "-n5",
                "--no-graph",
                "-T",
                parse::CHANGE_TEMPLATE,
                "root-file:\"x|y.rs\"",
                "root-file:\"z.rs\"",
                "--color",
                "never"
            ]
        );
    }

    // An empty fileset slice must NOT degrade to a bare `jj log -r <revset>`
    // (unrestricted history) — it's refused before any spawn, mirroring
    // `commit_paths_refuses_empty_filesets_without_spawning`.
    #[tokio::test]
    async fn log_paths_refuses_empty_filesets_without_spawning() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        let err = jj
            .log_paths(Path::new("."), &rv("@"), 5, &[])
            .await
            .expect_err("empty filesets must be refused");
        assert!(matches!(err, Error::Spawn { .. }), "got {err:?}");
        assert!(rec.calls().is_empty(), "nothing may spawn");
    }

    #[tokio::test]
    async fn op_log_parses_template_rows() {
        let rec = RecordingRunner::new(ScriptedRunner::new().on(
            ["jj", "op", "log"],
            Reply::ok("abc\tu@h\t2026-06-05T10:00:00+0200\tnew empty commit\n"),
        ));
        let jj = Jj::with_runner(&rec);
        let ops = jj.op_log(Path::new("."), 5).await.expect("op_log");
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].id, "abc");
        assert_eq!(ops[0].description, "new empty commit");
        let args = rec.only_call().args_str();
        assert_eq!(&args[..5], &["op", "log", "--no-graph", "--limit", "5"]);
    }

    // evolog must use the commit-context template (bare `change_id` doesn't
    // exist there) but flows through the same Change parser.
    #[tokio::test]
    async fn evolog_uses_commit_context_template() {
        let rec = RecordingRunner::new(
            ScriptedRunner::new().on(["jj", "evolog"], Reply::ok("kz\t38\tfalse\twip\n")),
        );
        let jj = Jj::with_runner(&rec);
        let rows = jj.evolog(Path::new("."), &rv("@"), 10).await.expect("evolog");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].description, "wip");
        let args = rec.only_call().args_str();
        assert_eq!(
            &args[..6],
            &["evolog", "-r", "@", "--no-graph", "--limit", "10"]
        );
        let template = &args[7];
        assert!(
            template.contains("commit.change_id()"),
            "commit-context form required, got {template}"
        );
    }

    #[tokio::test]
    async fn file_annotate_and_show_build_args() {
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(
                    ["jj", "file", "annotate"],
                    Reply::ok("kz\tline one\nkz\tline two"),
                )
                .on(["jj", "file", "show"], Reply::ok("content\n")),
        );
        let jj = Jj::with_runner(&rec);
        let lines = jj
            .file_annotate(Path::new("."), "src/a.rs", Some(rv("@-")))
            .await
            .expect("annotate");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].change_id, "kz");
        assert_eq!(lines[1].line, 2);
        // H7: the file's trailing newline is preserved verbatim, not trimmed.
        assert_eq!(
            jj.file_show(Path::new("."), &rv("@-"), "src/a.rs")
                .await
                .unwrap(),
            "content\n"
        );
        let calls = rec.calls();
        // The path follows a `--` separator (a leading-`-` filename stays safe);
        // `--color never` must precede `--`, not trail it.
        assert_eq!(
            calls[0].args_str(),
            [
                "file",
                "annotate",
                "-r",
                "@-",
                "-T",
                parse::ANNOTATE_TEMPLATE,
                "--color",
                "never",
                "--",
                "src/a.rs"
            ]
        );
        // file_show wraps the path as an exact-path fileset (metacharacters in
        // the name must stay literal); annotate takes a PLAIN path — quoting
        // it would break jj's path lookup.
        assert_eq!(
            calls[1].args_str(),
            [
                "file",
                "show",
                "-r",
                "@-",
                "root-file:\"src/a.rs\"",
                "--color",
                "never"
            ]
        );
    }

    // `description` is a fixed template query: first match only, raw description.
    #[tokio::test]
    async fn description_builds_single_commit_template_query() {
        let rec = RecordingRunner::replying(Reply::ok("feat: parser\n\nbody\n"));
        let jj = Jj::with_runner(&rec);
        let text = jj
            .description(Path::new("."), &rv("abc123"))
            .await
            .expect("description");
        assert_eq!(text, "feat: parser\n\nbody");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "log",
                "-r",
                "abc123",
                "--no-graph",
                "--limit",
                "1",
                "-T",
                "description",
                "--color",
                "never"
            ]
        );
    }

    // H7: content verbs return jj's output byte-for-byte — the round-trip-corrupting
    // cases are multiple trailing newlines, a missing final newline, and a diff whose
    // last hunk ends in a blank context line.
    #[tokio::test]
    async fn content_verbs_preserve_exact_trailing_bytes() {
        for raw in ["a\nb\n\n", "no-final-newline", "trailing   \n"] {
            let rec = RecordingRunner::replying(Reply::ok(raw));
            let jj = Jj::with_runner(&rec);
            assert_eq!(
                jj.file_show(Path::new("."), &rv("@"), "f.txt")
                    .await
                    .expect("file_show"),
                raw
            );
        }
        let diff = "diff --git a/f b/f\n@@ -1,2 +1,2 @@\n-x\n+y\n \n";
        let rec = RecordingRunner::replying(Reply::ok(diff));
        let jj = Jj::with_runner(&rec);
        assert_eq!(
            jj.diff_text(Path::new("."), DiffSpec::Rev("@".into()))
                .await
                .expect("diff_text"),
            diff
        );
    }

    // `diff_text` for the working copy must build `diff -r @ --git`.
    #[tokio::test]
    async fn diff_text_builds_working_copy_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.diff_text(Path::new("."), DiffSpec::WorkingTree)
            .await
            .expect("diff_text");
        assert_eq!(
            rec.only_call().args_str(),
            ["diff", "-r", "@", "--git", "--color", "never"]
        );
    }

    // Every repo-scoped command forces `--color never` so a user's
    // `ui.color = "always"` config can't wrap parsed output in ANSI escapes.
    #[tokio::test]
    async fn commands_force_color_off() {
        let rec = RecordingRunner::replying(Reply::ok("x\n"));
        let jj = Jj::with_runner(&rec);
        jj.status_text(Path::new(".")).await.expect("status_text");
        let args = rec.only_call().args_str();
        let pos = args.iter().position(|a| a == "--color");
        assert_eq!(
            pos.map(|p| args.get(p + 1).map(String::as_str)),
            Some(Some("never"))
        );
    }

    // Hermetic: real diff() arg-building (`Rev`) + the ported parser against
    // canned git-format output.
    #[tokio::test]
    async fn diff_parses_scripted_output() {
        let out = "diff --git a/m b/m\n--- a/m\n+++ b/m\n@@ -1 +1 @@\n-a\n+b\n";
        let jj = Jj::with_runner(ScriptedRunner::new().on(["jj", "diff"], Reply::ok(out)));
        let files = jj
            .diff(Path::new("."), DiffSpec::Rev("@-".into()))
            .await
            .expect("diff");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "m");
        assert_eq!(files[0].change, ChangeKind::Modified);
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn consumer_mocks_the_interface() {
        let mut mock = MockJjApi::new();
        mock.expect_describe().returning(|_, _| Ok(()));
        assert!(mock.describe(Path::new("."), "msg").await.is_ok());
    }
}

// Long-form how-to guides, rendered from this crate's docs/*.md on docs.rs.
#[doc = include_str!("../docs/jj.md")]
#[allow(rustdoc::broken_intra_doc_links)]
pub mod guide {}
