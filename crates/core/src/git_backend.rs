//! Git-backed implementations of the facade operations: thin calls to the
//! `vcs-git` client plus pure mappers from its types into the facade DTOs.

use std::path::Path;

use processkit::ProcessRunner;
use vcs_git::{Git, GitApi, StatusEntry, WorktreeAdd};

use crate::dto::{
    ChangeKind, CreateOutcome, DiffStat, FileChange, MergeProbe, OperationState, WorktreeInfo,
};
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
    // Working tree vs the last commit. On an unborn repo `HEAD` doesn't resolve
    // (`git diff HEAD` errors), so stat against the empty tree — a fresh repo's
    // working copy then reports its files as additions instead of hard-failing,
    // matching `changed_files()` (status-based) and `git.diff_text(WorkingTree)`.
    // `git.diff_stat` already returns the shared `vcs_diff::DiffStat` — no remap.
    let range = if git.is_unborn(dir).await? {
        vcs_git::EMPTY_TREE
    } else {
        "HEAD"
    };
    git.diff_stat(dir, range).await.map_err(Into::into)
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

pub(crate) async fn try_merge<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    source: &str,
) -> Result<MergeProbe> {
    // `--no-ff` so even a fast-forwardable merge stages a real (abortable) merge
    // instead of moving HEAD; `--no-commit` so nothing is committed either way.
    let merged = git.merge_no_commit(dir, source, false, true).await;
    match merged {
        Ok(()) => {
            // "Already up to date." exits 0 *without* MERGE_HEAD — `merge
            // --abort` would then fail, so only abort an actually-started merge.
            if git.is_merge_in_progress(dir).await? {
                git.merge_abort(dir).await?;
            }
            Ok(MergeProbe::Clean)
        }
        Err(err) if vcs_git::is_merge_conflict(&err) => {
            // Collect the conflicted paths BEFORE aborting — `merge --abort`
            // clears the unmerged index entries this reads.
            let files = git.conflicted_files(dir).await?;
            // A failed abort breaks the guaranteed-rollback contract → propagate
            // rather than return a `Conflicts` that lies about the tree state.
            git.merge_abort(dir).await?;
            Ok(MergeProbe::Conflicts(files))
        }
        Err(err) => {
            // E.g. a dirty-tree refusal or an unknown ref — the merge usually
            // never started, but clean up if it did.
            if git.is_merge_in_progress(dir).await? {
                git.merge_abort(dir).await?;
            }
            Err(err.into())
        }
    }
}

pub(crate) async fn abort_in_progress<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
) -> Result<OperationState> {
    match in_progress_state(git, dir).await? {
        OperationState::Merge => git.merge_abort(dir).await?,
        OperationState::Rebase => git.rebase_abort(dir).await?,
        _ => {}
    }
    // Recompute rather than assume `Clear` — the return is the *post-call* state.
    in_progress_state(git, dir).await
}

pub(crate) async fn continue_in_progress<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
) -> Result<OperationState> {
    // git refuses to continue while unmerged paths remain; report instead of
    // tripping over the hard error.
    if !git.conflicted_files(dir).await?.is_empty() {
        return Ok(OperationState::Conflict);
    }
    match in_progress_state(git, dir).await? {
        OperationState::Merge => git.merge_continue(dir).await?,
        OperationState::Rebase => {
            // `rebase --continue` exits non-zero when it stops on the NEXT
            // patch's conflict — that's the `Conflict` outcome, not an error.
            if let Err(err) = git.rebase_continue(dir).await {
                if !git.conflicted_files(dir).await?.is_empty() {
                    return Ok(OperationState::Conflict);
                }
                return Err(err.into());
            }
        }
        _ => {}
    }
    // Belt and braces: report any unresolved paths the continue left behind.
    if !git.conflicted_files(dir).await?.is_empty() {
        return Ok(OperationState::Conflict);
    }
    in_progress_state(git, dir).await
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
/// others; an untracked (`??`) or copied (`C`) entry counts as added (a copy is a
/// new file — `parse_porcelain` even records its source as `old_path`, like a
/// rename); unmerged states (`UU`/`AA`/`DD`/…) fold into their underlying kind —
/// use [`conflicted_files`](crate::Repo::conflicted_files) for the conflict signal.
fn change_kind_from_code(code: &str) -> ChangeKind {
    if code.contains('R') {
        ChangeKind::Renamed
    } else if code.contains('D') {
        ChangeKind::Deleted
    } else if code.contains('A') || code.contains('?') || code.contains('C') {
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
        // A copy (only emitted with copy detection on) is a new file, not a modify.
        assert_eq!(change_kind_from_code("C "), ChangeKind::Added);
    }
}
