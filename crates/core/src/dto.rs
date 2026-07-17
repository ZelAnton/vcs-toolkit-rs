//! Backend-agnostic data types the facade returns — plus the option **specs** it
//! accepts — generalising the per-tool shapes of `vcs-git` and `vcs-jj` into one set
//! a consumer can use without knowing which backend is in play.

use std::path::PathBuf;

/// Options for [`Repo::remove_worktree`](crate::Repo::remove_worktree).
///
/// `#[non_exhaustive]`, so build it through [`WorktreeRemove::new`] and the chained
/// [`force`](WorktreeRemove::force) setter rather than a struct literal — a bare
/// `bool` at the call site (`remove_worktree(path, true)`) doesn't say what `true`
/// means, and this leaves room to add options without a breaking signature change.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WorktreeRemove {
    /// The attached worktree (git) / secondary workspace (jj) path to remove.
    pub path: PathBuf,
    /// Remove even when the worktree has uncommitted changes — git `worktree remove
    /// --force`; on jj, the snapshot-and-refuse-if-dirty guard is bypassed. The
    /// repository's **main** worktree/workspace is refused regardless of this flag.
    pub force: bool,
}

impl WorktreeRemove {
    /// Remove the worktree/workspace at `path`; not forced (refuses a dirty one).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            force: false,
        }
    }

    /// Remove even when the worktree has uncommitted changes.
    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }
}

/// Partial [`WorktreeCreate`] — carries the path and new-branch name; chain
/// [`base`](WorktreeCreatePartial::base) to name the ref it forks from.
#[derive(Debug, Clone)]
pub struct WorktreeCreatePartial {
    path: PathBuf,
    branch: String,
}

impl WorktreeCreatePartial {
    /// The ref the new worktree/workspace forks from — a branch, tag, or commit
    /// (git `HEAD`; jj `@` / a change id). Required and explicit: it has no default
    /// because the sentinel for "current" differs by backend.
    pub fn base(self, base: impl Into<String>) -> WorktreeCreate {
        WorktreeCreate {
            path: self.path,
            branch: self.branch,
            base: base.into(),
        }
    }
}

/// Options for [`Repo::create_worktree`](crate::Repo::create_worktree).
///
/// Built as `WorktreeCreate::new(path, "feature").base("main")` — the new-branch name
/// and the fork-point `base` (both plain strings that a swap would silently accept,
/// creating a branch *named* like the base) are named across **two** builder steps, so
/// they can't be transposed. `#[non_exhaustive]`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WorktreeCreate {
    /// Where the new attached worktree (git) / secondary workspace (jj) is created.
    pub path: PathBuf,
    /// The new branch (git) / bookmark (jj) to create at the worktree.
    pub branch: String,
    /// The ref the new branch forks from (git `HEAD`, jj `@`, a branch/tag/commit).
    pub base: String,
}

impl WorktreeCreate {
    /// Name the worktree `path` and the new `branch` to create there; chain
    /// [`base`](WorktreeCreatePartial::base) to name the fork point.
    ///
    // A type-state builder entry: `new` returns the partial (not `Self`) so `base`
    // is mandatory — the recognised builder exception to `new_ret_no_self`.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(path: impl Into<PathBuf>, branch: impl Into<String>) -> WorktreeCreatePartial {
        WorktreeCreatePartial {
            path: path.into(),
            branch: branch.into(),
        }
    }
}

/// Options for [`Repo::delete_branch`](crate::Repo::delete_branch).
///
/// `#[non_exhaustive]`, so build it through [`BranchDelete::new`] and the chained
/// [`force`](BranchDelete::force) setter rather than a struct literal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BranchDelete {
    /// The local branch (git) / bookmark (jj) name to delete.
    pub name: String,
    /// Delete even if not fully merged — git `branch -D` vs `-d`. **git only**: jj has
    /// no force flag for `bookmark delete` and ignores it.
    pub force: bool,
}

impl BranchDelete {
    /// Delete branch/bookmark `name`; not forced (git refuses an unmerged branch).
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            force: false,
        }
    }

    /// Delete even if not fully merged (git only).
    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }
}

/// Which version-control tool backs a [`Repo`](crate::Repo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum BackendKind {
    /// A plain Git repository.
    Git,
    /// A Jujutsu repository (possibly colocated with Git).
    Jj,
}

impl BackendKind {
    /// The tool's short name (`"git"` / `"jj"`).
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Git => "git",
            BackendKind::Jj => "jj",
        }
    }
}

/// How a file changed in the working copy — the shared [`vcs_diff::ChangeKind`]
/// (one type across the wrappers and the facade, no remapping). The status-code
/// mappers in the backends turn git's `XY` codes / jj's letters into it.
pub use vcs_diff::ChangeKind;

/// One changed path in the working copy, unified across `git status` /
/// `jj diff --summary`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FileChange {
    /// The path (the *new* path for a rename).
    ///
    /// A [`PathBuf`] (not a `String`) so a filename whose bytes are not valid UTF-8
    /// — legal on Unix — is carried **losslessly** from `status`/`diff` and can be
    /// fed straight back into [`Repo::commit_paths`](crate::Repo::commit_paths) /
    /// the backend `add`. A `String` filled via `String::from_utf8_lossy` would
    /// substitute `U+FFFD` and address a different file. See the crate's
    /// serde-policy note for how a non-UTF-8 path is emitted as JSON.
    pub path: PathBuf,
    /// The original path for a rename, populated by **both** backends (git's
    /// `R old -> new` status; jj's `{old => new}` diff-summary form); `None`
    /// for non-renames.
    pub old_path: Option<PathBuf>,
    /// How the file changed.
    pub kind: ChangeKind,
}

impl FileChange {
    /// A change to `path` of the given `kind`, with no original path. Chain the
    /// `old_path` setter for a rename or copy. Lets an external `VcsRepo` impl or a
    /// test build one despite the `#[non_exhaustive]`.
    pub fn new(path: impl Into<PathBuf>, kind: ChangeKind) -> Self {
        Self {
            path: path.into(),
            old_path: None,
            kind,
        }
    }

    /// Record the original path — a rename's or copy's source (sets the `old_path`
    /// field, which both a rename and a copy populate).
    pub fn old_path(mut self, old: impl Into<PathBuf>) -> Self {
        self.old_path = Some(old.into());
        self
    }
}

/// Aggregate insertion/deletion counts for the working copy — the shared
/// [`vcs_diff::DiffStat`], returned by the backends directly (no remapping).
pub use vcs_diff::DiffStat;

/// One file's full parsed diff (hunks and lines) — the shared
/// [`vcs_diff::FileDiff`], returned by [`Repo::diff`](crate::Repo::diff) directly
/// (no remapping); the same type `GitApi::diff`/`JjApi::diff` already return.
pub use vcs_diff::FileDiff;

/// One attached worktree (git) / workspace (jj).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct WorktreeInfo {
    /// Filesystem path of the worktree's working copy.
    pub path: PathBuf,
    /// The branch (git) or first bookmark (jj) on it; `None` when detached/none.
    pub branch: Option<String>,
    /// The checked-out commit's **full** object id (git `HEAD` oid / jj `@` commit
    /// id) on both backends — the same identity a [`RepoSnapshot::head`] carries, so
    /// the two can be compared directly to tell whether a worktree sits on the
    /// snapshotted commit. Not a display-truncated prefix (which could collide);
    /// truncate for display. `None` when unavailable (e.g. a bare git entry).
    pub commit: Option<String>,
    /// A bare git worktree entry (always `false` for jj).
    pub is_bare: bool,
}

impl WorktreeInfo {
    /// A worktree at `path` with no branch/commit and not bare; chain the setters.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            branch: None,
            commit: None,
            is_bare: false,
        }
    }

    /// Set the branch (git) / first bookmark (jj) on the worktree.
    pub fn branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Set the checked-out commit.
    pub fn commit(mut self, commit: impl Into<String>) -> Self {
        self.commit = Some(commit.into());
        self
    }

    /// Mark it a bare git worktree entry.
    pub fn bare(mut self) -> Self {
        self.is_bare = true;
        self
    }
}

/// Whether the working copy is mid-operation, unified across the backends'
/// different models: git exposes an in-progress merge, rebase, `am`, cherry-pick,
/// revert, or bisect as on-disk state (`MERGE_HEAD` / a `rebase-*` dir /
/// `CHERRY_PICK_HEAD` / `REVERT_HEAD` / `BISECT_LOG`), while jj has no multi-step
/// operations — it records a conflict directly on the working-copy change.
///
/// The sequencer states are kept **distinct** because each aborts (and, where it
/// makes sense, continues) with its *own* git command — dispatching the wrong one
/// on a user's real repository is exactly what this type exists to prevent. See
/// [`Repo::abort_in_progress`](crate::Repo::abort_in_progress) /
/// [`continue_in_progress`](crate::Repo::continue_in_progress).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum OperationState {
    /// No operation in progress and no conflict.
    Clear,
    /// A git merge is in progress (`MERGE_HEAD` present).
    Merge,
    /// A git rebase is in progress (a `rebase-merge` dir, or a `rebase-apply` dir
    /// **not** left by `git am` — see [`ApplyMailbox`](OperationState::ApplyMailbox)).
    Rebase,
    /// A git `am` (mailbox patch apply) is in progress. Distinct from `Rebase`
    /// because it aborts/continues with `am --abort` / `am --continue`, not the
    /// `rebase --*` twins (M20). Like the other sequencer states it has a real
    /// `--continue`, so [`continue_in_progress`](crate::Repo::continue_in_progress)
    /// drives it forward (reporting `Conflict` if the next patch stops) rather than
    /// treating it as nothing to do.
    ApplyMailbox,
    /// A git cherry-pick is in progress (`CHERRY_PICK_HEAD` present). Distinct
    /// from `Merge`: it aborts/continues with `cherry-pick --abort` /
    /// `cherry-pick --continue` (a cherry-pick conflict writes `CHERRY_PICK_HEAD`,
    /// **not** `MERGE_HEAD`). git only.
    CherryPick,
    /// A git revert is in progress (`REVERT_HEAD` present). Aborts/continues with
    /// `revert --abort` / `revert --continue`. git only.
    Revert,
    /// A git bisect session is in progress (`BISECT_LOG` present). Aborts with
    /// `bisect reset`; it has **no** `--continue` step (bisect advances by marking
    /// commits good/bad), so
    /// [`continue_in_progress`](crate::Repo::continue_in_progress) reports it as
    /// unsupported rather than silently doing nothing. git only.
    Bisect,
    /// The working copy has an unresolved conflict (chiefly jj, which records
    /// conflicts on the change rather than pausing an operation).
    Conflict,
}

/// Upstream tracking for the current branch: the upstream ref and how far the
/// branch is ahead/behind it. [`RepoSnapshot`] carries it as one
/// `Option<UpstreamTracking>` — `None` when no upstream is configured at all.
///
/// The ahead/behind counts are themselves `Option`: git reports them only when the
/// upstream ref actually **resolves**, so a branch whose upstream is *set but gone*
/// (deleted on the remote, or not yet fetched) yields `Some(UpstreamTracking { branch,
/// ahead: None, behind: None })` — "tracking configured but uncountable", distinct
/// from the in-sync `Some(0)`/`Some(0)` that a `unwrap_or(0)` used to fabricate (M17).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct UpstreamTracking {
    /// The upstream tracking branch, e.g. `"origin/main"`.
    pub branch: String,
    /// Commits the local branch is ahead of the upstream; `None` when the upstream is
    /// set but git couldn't count against it (gone remote / not fetched).
    pub ahead: Option<usize>,
    /// Commits the local branch is behind the upstream; `None` when uncountable (see
    /// [`ahead`](UpstreamTracking::ahead)).
    pub behind: Option<usize>,
}

impl UpstreamTracking {
    /// Tracking `branch` (e.g. `"origin/main"`) with **uncounted** ahead/behind
    /// (both `None`); chain [`ahead`](UpstreamTracking::ahead) /
    /// [`behind`](UpstreamTracking::behind) to set the counts.
    pub fn new(branch: impl Into<String>) -> Self {
        Self {
            branch: branch.into(),
            ahead: None,
            behind: None,
        }
    }

    /// Set the ahead count.
    pub fn ahead(mut self, n: usize) -> Self {
        self.ahead = Some(n);
        self
    }

    /// Set the behind count.
    pub fn behind(mut self, n: usize) -> Self {
        self.behind = Some(n);
        self
    }
}

/// A one-shot snapshot of the common repository state — branch, upstream
/// tracking, ahead/behind, dirtiness, and operation state — gathered in a
/// **small fixed** number of process spawns instead of a call per field. The
/// data a prompt, status line, or TUI refresh needs. See
/// [`Repo::snapshot`](crate::Repo::snapshot).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct RepoSnapshot {
    /// The working-copy commit's **full** object id (git `HEAD` oid / jj `@`
    /// commit id) on both backends; `None` on an unborn git repo. Truncate for
    /// display. Carries the full id (not a short prefix) so it can be
    /// cross-referenced against a [`WorktreeInfo::commit`] or a git oid without a
    /// short-prefix collision.
    pub head: Option<String>,
    /// Current branch (git) / bookmark (jj). On jj this is the nearest bookmark
    /// reachable from `@` (`heads(::@ & bookmarks())`), so it stays set across a
    /// `jj describe`/`jj new`/`jj commit`; `None` when detached / no bookmark on
    /// or above `@`. Matches [`Repo::current_branch`](crate::Repo::current_branch)
    /// by construction.
    pub branch: Option<String>,
    /// Upstream tracking and how far the branch is ahead/behind it, as one unit —
    /// `Some` only when an upstream is configured, `None` otherwise (and **always
    /// `None` on jj**, which has no git-style upstream tracking). Bundling the
    /// three together makes the "all-or-nothing" relationship unrepresentable as a
    /// half-populated state. See [`UpstreamTracking`].
    pub tracking: Option<UpstreamTracking>,
    /// Whether the working copy has any uncommitted change (tracked or untracked).
    pub dirty: bool,
    /// Number of changed paths (tracked + untracked on git; the `@` change's
    /// files on jj).
    pub change_count: usize,
    /// Whether the working copy has an unresolved conflict.
    pub conflicted: bool,
    /// In-progress operation / conflict state (see [`OperationState`]).
    pub operation: OperationState,
}

impl RepoSnapshot {
    /// A clean snapshot: detached (no `head`/`branch`), no upstream tracking, not
    /// dirty or conflicted, change count 0, [`OperationState::Clear`]. Chain the
    /// setters to fill it — for a test double or a custom `VcsRepo` backend that must
    /// return a `RepoSnapshot` (the struct is `#[non_exhaustive]`, so it can't be
    /// built with a literal outside this crate).
    pub fn new() -> Self {
        Self {
            head: None,
            branch: None,
            tracking: None,
            dirty: false,
            change_count: 0,
            conflicted: false,
            operation: OperationState::Clear,
        }
    }

    /// Set the working-copy commit's object id.
    pub fn head(mut self, head: impl Into<String>) -> Self {
        self.head = Some(head.into());
        self
    }

    /// Set the current branch (git) / bookmark (jj).
    pub fn branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Set the upstream tracking (see [`UpstreamTracking`]).
    pub fn tracking(mut self, tracking: UpstreamTracking) -> Self {
        self.tracking = Some(tracking);
        self
    }

    /// Mark the working copy dirty and record how many paths changed (a real snapshot
    /// has `change_count >= 1` when dirty — the two fields move together, so this
    /// setter couples them). A clean copy is the [`new`](RepoSnapshot::new) default.
    pub fn dirty(mut self, change_count: usize) -> Self {
        self.dirty = true;
        self.change_count = change_count;
        self
    }

    /// Mark the working copy as having an unresolved conflict.
    pub fn conflicted(mut self) -> Self {
        self.conflicted = true;
        self
    }

    /// Set the in-progress operation / conflict state.
    pub fn operation(mut self, operation: OperationState) -> Self {
        self.operation = operation;
        self
    }
}

impl Default for RepoSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

/// The outcome of a [`try_merge`](crate::Repo::try_merge) probe. The probe
/// itself is rolled back before it returns, whatever the outcome — this only
/// *reports* what a real merge would do.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
// Adjacently tagged so the JSON is a *type-stable object* for both outcomes —
// `{"outcome":"Clean"}` and `{"outcome":"Conflicts","files":[…]}` — rather than
// serde's default externally-tagged shape, which would emit a bare string
// `"Clean"` for one variant and an object for the other (a polymorphic result an
// agent consumer can't branch on uniformly).
#[cfg_attr(feature = "serde", serde(tag = "outcome", content = "files"))]
#[non_exhaustive]
pub enum MergeProbe {
    /// The merge would apply without conflicts.
    Clean,
    /// The merge would conflict in these paths (repo-relative, `/` separators —
    /// the same contract and [`PathBuf`] type as
    /// [`conflicted_files`](crate::Repo::conflicted_files), so a non-UTF-8 path is
    /// carried losslessly).
    Conflicts(Vec<PathBuf>),
}

impl MergeProbe {
    /// Whether the probe found no conflicts.
    pub fn is_clean(&self) -> bool {
        matches!(self, MergeProbe::Clean)
    }
}

/// One commit/change from the repository history — the honest least common
/// denominator between git's typed `git log` (`vcs_git::parse::Commit`, which
/// carries hash/short-hash/author/date/subject) and jj's typed `jj log`
/// (`vcs_jj::parse::Change`, which carries change-id/commit-id/empty/description).
/// See [`Repo::log`](crate::Repo::log).
///
/// `author`/`date` are `Some` only on git: jj's typed log doesn't currently
/// surface authorship or a timestamp (its template renders only the id/empty/
/// description columns), so this DTO leaves them `None` on jj rather than
/// fabricating a value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Commit {
    /// The commit's identifying hash: git's full object id (`%H`) / jj's
    /// (already-short) commit id.
    pub id: String,
    /// Commit message: git's subject line (`%s`) / jj's first description line.
    pub description: String,
    /// Author name (git `%an`); `None` on jj (see the type docs).
    pub author: Option<String>,
    /// Author date, strict ISO-8601 on git (`%aI`); `None` on jj (see the type
    /// docs).
    pub date: Option<String>,
}

impl Commit {
    /// A commit `id` with `description`, no author/date (jj's typed-log shape);
    /// chain [`author`](Commit::author) / [`date`](Commit::date) to add them
    /// (git's shape). Lets an external `VcsRepo` impl or a test build one despite
    /// the `#[non_exhaustive]`.
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            author: None,
            date: None,
        }
    }

    /// Set the author name.
    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    /// Set the author date.
    pub fn date(mut self, date: impl Into<String>) -> Self {
        self.date = Some(date.into());
        self
    }
}

/// How a worktree was materialised. The facade always reports
/// [`Plain`](CreateOutcome::Plain); the [`CowCloned`](CreateOutcome::CowCloned)
/// variant exists so a consumer that layers a copy-on-write strategy on top can
/// reuse this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CreateOutcome {
    /// The tool materialised the working copy itself.
    Plain,
    /// A copy-on-write clone populated the working copy (consumer-supplied).
    CowCloned,
}

// The optional `serde` feature derives `Serialize` on the facade DTOs.
#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use super::*;

    #[test]
    fn snapshot_and_file_change_serialize_to_clean_json() {
        let snap = RepoSnapshot {
            head: Some("abc".into()),
            branch: Some("main".into()),
            tracking: Some(UpstreamTracking {
                branch: "origin/main".into(),
                ahead: Some(1),
                behind: Some(0),
            }),
            dirty: true,
            change_count: 2,
            conflicted: false,
            operation: OperationState::Merge,
        };
        let v = serde_json::to_value(&snap).unwrap();
        assert_eq!(v["branch"], "main");
        assert_eq!(v["operation"], "Merge"); // enum → variant name
        assert_eq!(v["change_count"], 2);
        // Tracking serialises as one nested object (or null), not three fields.
        assert_eq!(v["tracking"]["branch"], "origin/main");
        assert_eq!(v["tracking"]["ahead"], 1);

        let fc = FileChange {
            path: "a.rs".into(),
            old_path: None,
            kind: ChangeKind::Added, // re-exported vcs_diff type, Serialize via vcs-diff/serde
        };
        let v = serde_json::to_value(fc).unwrap();
        // A `PathBuf` field serialises as a plain JSON string for a UTF-8 path.
        assert_eq!(v["path"], "a.rs");
        assert_eq!(v["kind"], "Added");
    }

    // Every `OperationState` variant, including the sequencer additions, serialises
    // to its bare variant name (the JSON a `snapshot`/MCP consumer branches on).
    #[test]
    fn operation_state_variants_serialize_to_their_names() {
        for (state, name) in [
            (OperationState::Clear, "Clear"),
            (OperationState::Merge, "Merge"),
            (OperationState::Rebase, "Rebase"),
            (OperationState::ApplyMailbox, "ApplyMailbox"),
            (OperationState::CherryPick, "CherryPick"),
            (OperationState::Revert, "Revert"),
            (OperationState::Bisect, "Bisect"),
            (OperationState::Conflict, "Conflict"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), name);
        }
    }

    // `MergeProbe` is adjacently tagged: BOTH outcomes are objects with an
    // `outcome` discriminant — a stable shape a tool consumer can branch on,
    // never a bare string for one case and an object for the other.
    #[test]
    fn merge_probe_serializes_to_a_type_stable_object() {
        let clean = serde_json::to_value(MergeProbe::Clean).unwrap();
        assert_eq!(clean["outcome"], "Clean");
        assert!(clean.get("files").is_none(), "{clean}");

        let conflicts =
            serde_json::to_value(MergeProbe::Conflicts(vec!["a.rs".into(), "b.rs".into()]))
                .unwrap();
        assert_eq!(conflicts["outcome"], "Conflicts");
        assert_eq!(conflicts["files"][0], "a.rs");
        assert_eq!(conflicts["files"][1], "b.rs");
    }

    #[test]
    fn commit_serializes_with_null_author_date_on_the_jj_shape() {
        let jj_shaped = Commit::new("abc123", "first line");
        let v = serde_json::to_value(&jj_shaped).unwrap();
        assert_eq!(v["id"], "abc123");
        assert_eq!(v["description"], "first line");
        assert!(v["author"].is_null());
        assert!(v["date"].is_null());
    }
}

#[cfg(test)]
mod ctor_tests {
    use super::*;

    // A4: the public builder constructors let an external `VcsRepo` impl / test
    // build the `#[non_exhaustive]` return DTOs, and land the fields where expected.
    #[test]
    fn dto_constructors_populate_fields() {
        let jj_shaped = Commit::new("abc123", "first line");
        assert_eq!(jj_shaped.id, "abc123");
        assert_eq!(jj_shaped.description, "first line");
        assert_eq!(jj_shaped.author, None);
        assert_eq!(jj_shaped.date, None);

        let git_shaped = Commit::new("deadbeef", "subject")
            .author("Jane")
            .date("2026-05-31");
        assert_eq!(git_shaped.author.as_deref(), Some("Jane"));
        assert_eq!(git_shaped.date.as_deref(), Some("2026-05-31"));

        let fc = FileChange::new("new.rs", ChangeKind::Modified).old_path("old.rs");
        assert_eq!(fc.path, PathBuf::from("new.rs"));
        assert_eq!(fc.old_path.as_deref(), Some(std::path::Path::new("old.rs")));
        assert_eq!(fc.kind, ChangeKind::Modified);

        let wt = WorktreeInfo::new("/wt")
            .branch("feature")
            .commit("abc123")
            .bare();
        assert_eq!(wt.path, PathBuf::from("/wt"));
        assert_eq!(wt.branch.as_deref(), Some("feature"));
        assert_eq!(wt.commit.as_deref(), Some("abc123"));
        assert!(wt.is_bare);

        let up = UpstreamTracking::new("origin/main").ahead(2).behind(3);
        assert_eq!(up.branch, "origin/main");
        assert_eq!(up.ahead, Some(2));
        assert_eq!(up.behind, Some(3));
        // Uncounted by default.
        assert_eq!(UpstreamTracking::new("origin/x").ahead, None);

        let snap = RepoSnapshot::new()
            .head("deadbeef")
            .branch("main")
            .tracking(up)
            .dirty(4)
            .conflicted()
            .operation(OperationState::Merge);
        assert_eq!(snap.head.as_deref(), Some("deadbeef"));
        assert_eq!(snap.branch.as_deref(), Some("main"));
        assert_eq!(snap.tracking.as_ref().unwrap().branch, "origin/main");
        assert_eq!(snap.tracking.as_ref().unwrap().ahead, Some(2));
        assert!(snap.dirty);
        assert_eq!(snap.change_count, 4);
        assert!(snap.conflicted);
        assert_eq!(snap.operation, OperationState::Merge);

        // A default snapshot is clean.
        let clean = RepoSnapshot::default();
        assert!(!clean.dirty && !clean.conflicted && clean.head.is_none());
        assert_eq!(clean.operation, OperationState::Clear);
        assert_eq!(clean.change_count, 0);
    }
}
