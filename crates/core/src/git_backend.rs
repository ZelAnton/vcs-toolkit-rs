//! Git-backed implementations of the facade operations: thin calls to the
//! `vcs-git` client plus pure mappers from its types into the facade DTOs.

use std::path::Path;

use processkit::ProcessRunner;
use vcs_git::{Git, GitApi, StatusEntry, WorktreeAdd};

use crate::dto::{ChangeKind, CreateOutcome, DiffStat, FileChange, OperationState, WorktreeInfo};
use crate::error::Result;

pub(crate) async fn current_branch<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
) -> Result<Option<String>> {
    // `current_branch` returns the literal "HEAD" when detached; surface that as
    // "no named branch" (`None`) so it mirrors jj's `Option` bookmark.
    let branch = git.current_branch(dir).await?;
    Ok((branch != "HEAD").then_some(branch))
}

pub(crate) async fn trunk<R: ProcessRunner>(git: &Git<R>, dir: &Path) -> Result<Option<String>> {
    Ok(git.remote_head_branch(dir).await?)
}

pub(crate) async fn local_branches<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
) -> Result<Vec<String>> {
    Ok(git
        .branches(dir)
        .await?
        .into_iter()
        .map(|b| b.name)
        .collect())
}

pub(crate) async fn branch_exists<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    name: &str,
) -> Result<bool> {
    Ok(git.branch_exists(dir, name).await?)
}

pub(crate) async fn has_uncommitted_changes<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
) -> Result<bool> {
    Ok(!git.status(dir).await?.is_empty())
}

pub(crate) async fn has_tracked_changes<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
) -> Result<bool> {
    Ok(!git.status_tracked(dir).await?.is_empty())
}

pub(crate) async fn conflicted_files<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
) -> Result<Vec<String>> {
    Ok(git.conflicted_files(dir).await?)
}

pub(crate) async fn delete_branch<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    name: &str,
    force: bool,
) -> Result<()> {
    git.delete_branch(dir, name, force).await?;
    Ok(())
}

pub(crate) async fn rename_branch<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    old: &str,
    new: &str,
) -> Result<()> {
    git.rename_branch(dir, old, new).await?;
    Ok(())
}

pub(crate) async fn changed_files<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
) -> Result<Vec<FileChange>> {
    let entries = git.status(dir).await?;
    Ok(entries.into_iter().map(file_change_from_status).collect())
}

pub(crate) async fn diff_stat<R: ProcessRunner>(git: &Git<R>, dir: &Path) -> Result<DiffStat> {
    // Working tree vs the last commit.
    let stat = git.diff_stat(dir, "HEAD").await?;
    Ok(DiffStat {
        files_changed: stat.files_changed,
        insertions: stat.insertions,
        deletions: stat.deletions,
    })
}

pub(crate) async fn commit_paths<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    paths: &[String],
    message: &str,
) -> Result<()> {
    let pathbufs: Vec<std::path::PathBuf> = paths.iter().map(Into::into).collect();
    git.commit_paths(dir, &pathbufs, message, false).await?;
    Ok(())
}

pub(crate) async fn fetch<R: ProcessRunner>(git: &Git<R>, dir: &Path) -> Result<()> {
    git.fetch(dir).await?;
    Ok(())
}

pub(crate) async fn fetch_from<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    remote: &str,
) -> Result<()> {
    git.fetch_from(dir, remote).await?;
    Ok(())
}

pub(crate) async fn fetch_remote_branch<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    branch: &str,
) -> Result<()> {
    git.fetch_remote_branch(dir, branch).await?;
    Ok(())
}

pub(crate) async fn checkout<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    reference: &str,
) -> Result<()> {
    git.checkout(dir, reference).await?;
    Ok(())
}

pub(crate) async fn rebase<R: ProcessRunner>(git: &Git<R>, dir: &Path, onto: &str) -> Result<()> {
    git.rebase(dir, onto).await?;
    Ok(())
}

pub(crate) async fn in_progress_state<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
) -> Result<OperationState> {
    // git surfaces an interrupted operation as on-disk state; a merge and a rebase
    // can't both be live, so report whichever is present.
    if git.is_merge_in_progress(dir).await? {
        Ok(OperationState::Merge)
    } else if git.is_rebase_in_progress(dir).await? {
        Ok(OperationState::Rebase)
    } else {
        Ok(OperationState::Clear)
    }
}

pub(crate) async fn list_worktrees<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
) -> Result<Vec<WorktreeInfo>> {
    let worktrees = git.worktree_list(dir).await?;
    Ok(worktrees
        .into_iter()
        .map(|w| WorktreeInfo {
            path: w.path,
            branch: w.branch,
            commit: w.head,
            is_bare: w.bare,
        })
        .collect())
}

pub(crate) async fn create_worktree<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    path: &Path,
    branch: &str,
    base: &str,
) -> Result<CreateOutcome> {
    git.worktree_add(dir, WorktreeAdd::create_branch(path, branch, base))
        .await?;
    Ok(CreateOutcome::Plain)
}

pub(crate) async fn remove_worktree<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    path: &Path,
    force: bool,
) -> Result<()> {
    git.worktree_remove(dir, path, force).await?;
    Ok(())
}

/// Project a `git status --porcelain` entry into a [`FileChange`].
fn file_change_from_status(entry: StatusEntry) -> FileChange {
    FileChange {
        kind: change_kind_from_code(&entry.code),
        path: entry.path,
        old_path: entry.orig_path,
    }
}

/// Map a porcelain `XY` status code to a [`ChangeKind`]. Rename wins over the
/// others; an untracked (`??`) entry counts as added.
fn change_kind_from_code(code: &str) -> ChangeKind {
    if code.contains('R') {
        ChangeKind::Renamed
    } else if code.contains('D') {
        ChangeKind::Deleted
    } else if code.contains('A') || code.contains('?') {
        ChangeKind::Added
    } else {
        ChangeKind::Modified
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_code_maps_to_change_kind() {
        assert_eq!(change_kind_from_code(" M"), ChangeKind::Modified);
        assert_eq!(change_kind_from_code("??"), ChangeKind::Added);
        assert_eq!(change_kind_from_code("A "), ChangeKind::Added);
        assert_eq!(change_kind_from_code(" D"), ChangeKind::Deleted);
        assert_eq!(change_kind_from_code("R "), ChangeKind::Renamed);
    }
}
