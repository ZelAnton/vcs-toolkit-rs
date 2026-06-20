//! Backend-agnostic data types the facade returns, generalising the per-tool
//! shapes of `vcs-git` and `vcs-jj` into one set a consumer can use without
//! knowing which backend is in play.

use std::path::PathBuf;

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
    pub path: String,
    /// The original path for a rename, populated by **both** backends (git's
    /// `R old -> new` status; jj's `{old => new}` diff-summary form); `None`
    /// for non-renames.
    pub old_path: Option<String>,
    /// How the file changed.
    pub kind: ChangeKind,
}

/// Aggregate insertion/deletion counts for the working copy — the shared
/// [`vcs_diff::DiffStat`], returned by the backends directly (no remapping).
pub use vcs_diff::DiffStat;

/// One attached worktree (git) / workspace (jj).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct WorktreeInfo {
    /// Filesystem path of the worktree's working copy.
    pub path: PathBuf,
    /// The branch (git) or first bookmark (jj) on it; `None` when detached/none.
    pub branch: Option<String>,
    /// The checked-out commit; `None` when unavailable (e.g. a bare git entry).
    pub commit: Option<String>,
    /// A bare git worktree entry (always `false` for jj).
    pub is_bare: bool,
}

/// Whether the working copy is mid-operation, unified across the backends'
/// different models: git exposes an in-progress merge or rebase as on-disk state
/// (`MERGE_HEAD` / a `rebase-*` dir), while jj has no multi-step operations — it
/// records a conflict directly on the working-copy change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum OperationState {
    /// No operation in progress and no conflict.
    Clear,
    /// A git merge is in progress (`MERGE_HEAD` present).
    Merge,
    /// A git rebase is in progress (a `rebase-merge`/`rebase-apply` dir present).
    Rebase,
    /// The working copy has an unresolved conflict (chiefly jj, which records
    /// conflicts on the change rather than pausing an operation).
    Conflict,
}

/// Upstream tracking for the current branch: the upstream ref and how far the
/// branch is ahead/behind it. Only meaningful as a whole — git reports the three
/// together or not at all — so [`RepoSnapshot`] carries it as one
/// `Option<UpstreamTracking>` rather than three coupled `Option`s.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct UpstreamTracking {
    /// The upstream tracking branch, e.g. `"origin/main"`.
    pub branch: String,
    /// Commits the local branch is ahead of the upstream.
    pub ahead: usize,
    /// Commits the local branch is behind the upstream.
    pub behind: usize,
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
    /// display.
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
    /// the same contract as [`conflicted_files`](crate::Repo::conflicted_files)).
    Conflicts(Vec<String>),
}

impl MergeProbe {
    /// Whether the probe found no conflicts.
    pub fn is_clean(&self) -> bool {
        matches!(self, MergeProbe::Clean)
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
                ahead: 1,
                behind: 0,
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
        assert_eq!(v["path"], "a.rs");
        assert_eq!(v["kind"], "Added");
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
}
