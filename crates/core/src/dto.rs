//! Backend-agnostic data types the facade returns, generalising the per-tool
//! shapes of `vcs-git` and `vcs-jj` into one set a consumer can use without
//! knowing which backend is in play.

use std::path::PathBuf;

/// Which version-control tool backs a [`Repo`](crate::Repo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// How a file changed in the working copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChangeKind {
    /// Added or untracked.
    Added,
    /// Contents modified.
    Modified,
    /// Removed.
    Deleted,
    /// Renamed (see [`FileChange::old_path`]).
    Renamed,
}

/// One changed path in the working copy, unified across `git status` /
/// `jj diff --summary`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileChange {
    /// The path (the *new* path for a rename).
    pub path: String,
    /// The original path for a rename; `None` otherwise (jj never supplies it).
    pub old_path: Option<String>,
    /// How the file changed.
    pub kind: ChangeKind,
}

/// Aggregate insertion/deletion counts for the working copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct DiffStat {
    /// Number of files changed.
    pub files_changed: usize,
    /// Lines inserted.
    pub insertions: usize,
    /// Lines deleted.
    pub deletions: usize,
}

/// One attached worktree (git) / workspace (jj).
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// How a worktree was materialised. The facade always reports
/// [`Plain`](CreateOutcome::Plain); the [`CowCloned`](CreateOutcome::CowCloned)
/// variant exists so a consumer that layers a copy-on-write strategy on top can
/// reuse this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CreateOutcome {
    /// The tool materialised the working copy itself.
    Plain,
    /// A copy-on-write clone populated the working copy (consumer-supplied).
    CowCloned,
}
