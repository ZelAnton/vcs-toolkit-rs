//! Jujutsu-backed implementations of the facade operations.
//!
//! jj's model differs from git's: workspaces are *named*, not path-addressed, and
//! `jj workspace list` carries no path — so worktree lookups resolve a name by
//! matching `jj workspace root --name <n>` against the requested path. The
//! copy-on-write / op-log-rollback creation flow stays in the consumer; the
//! facade only does the plain `jj workspace add` path.

use std::path::{Path, PathBuf};

use processkit::ProcessRunner;
use vcs_jj::{ChangedPath, Jj, JjApi, JjFileset, WorkspaceAdd};

use crate::dto::{ChangeKind, CreateOutcome, DiffStat, FileChange, OperationState, WorktreeInfo};
use crate::error::{Error, Result};

pub(crate) async fn current_branch<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
) -> Result<Option<String>> {
    Ok(jj.current_bookmark(dir).await?)
}

pub(crate) async fn trunk<R: ProcessRunner>(jj: &Jj<R>, dir: &Path) -> Result<Option<String>> {
    Ok(jj.trunk(dir).await?)
}

pub(crate) async fn local_branches<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
) -> Result<Vec<String>> {
    Ok(jj
        .bookmarks(dir)
        .await?
        .into_iter()
        .map(|b| b.name)
        .collect())
}

pub(crate) async fn branch_exists<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    name: &str,
) -> Result<bool> {
    // jj has no direct existence probe; scan the local bookmarks.
    Ok(jj.bookmarks(dir).await?.iter().any(|b| b.name == name))
}

pub(crate) async fn has_uncommitted_changes<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
) -> Result<bool> {
    Ok(!jj.current_change(dir).await?.empty)
}

pub(crate) async fn delete_branch<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    name: &str,
) -> Result<()> {
    jj.bookmark_delete(dir, name).await?;
    Ok(())
}

pub(crate) async fn rename_branch<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    old: &str,
    new: &str,
) -> Result<()> {
    jj.bookmark_rename(dir, old, new).await?;
    Ok(())
}

pub(crate) async fn changed_files<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
) -> Result<Vec<FileChange>> {
    let entries = jj.status(dir).await?;
    Ok(entries.into_iter().map(file_change_from_summary).collect())
}

pub(crate) async fn diff_stat<R: ProcessRunner>(jj: &Jj<R>, dir: &Path) -> Result<DiffStat> {
    let stat = jj.diff_stat(dir, "@").await?;
    Ok(DiffStat {
        files_changed: stat.files_changed,
        insertions: stat.insertions,
        deletions: stat.deletions,
    })
}

pub(crate) async fn commit_paths<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    paths: &[String],
    message: &str,
) -> Result<()> {
    let filesets: Vec<JjFileset> = paths.iter().map(JjFileset::path).collect();
    jj.commit_paths(dir, &filesets, message).await?;
    Ok(())
}

pub(crate) async fn fetch<R: ProcessRunner>(jj: &Jj<R>, dir: &Path) -> Result<()> {
    jj.git_fetch(dir).await?;
    Ok(())
}

pub(crate) async fn fetch_remote_branch<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    branch: &str,
) -> Result<()> {
    jj.git_fetch_branch(dir, branch).await?;
    Ok(())
}

pub(crate) async fn checkout<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    reference: &str,
) -> Result<()> {
    // jj has no "switch branch"; moving `@` to the bookmark/revision is the
    // equivalent of a git checkout.
    jj.edit(dir, reference).await?;
    Ok(())
}

pub(crate) async fn rebase<R: ProcessRunner>(jj: &Jj<R>, dir: &Path, onto: &str) -> Result<()> {
    jj.rebase(dir, onto).await?;
    Ok(())
}

pub(crate) async fn in_progress_state<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
) -> Result<OperationState> {
    // jj operations are atomic — there is no paused merge/rebase. A conflict is
    // recorded on the working-copy change instead.
    if jj.has_workingcopy_conflict(dir).await? {
        Ok(OperationState::Conflict)
    } else {
        Ok(OperationState::Clear)
    }
}

pub(crate) async fn list_worktrees<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
) -> Result<Vec<WorktreeInfo>> {
    // jj's `Workspace` carries no path, so resolve each via `workspace root`.
    let workspaces = jj.workspace_list(dir).await?;
    let mut out = Vec::new();
    for ws in workspaces {
        let Ok(root) = jj.workspace_root(dir, Some(ws.name.clone())).await else {
            continue; // No useful entry without a path.
        };
        out.push(WorktreeInfo {
            path: root,
            branch: ws.bookmarks.into_iter().next(),
            commit: (!ws.commit.is_empty()).then_some(ws.commit),
            is_bare: false,
        });
    }
    Ok(out)
}

pub(crate) async fn create_worktree<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    path: &Path,
    branch: &str,
    base: &str,
) -> Result<CreateOutcome> {
    let ws_name = workspace_name_for(branch);
    jj.workspace_add(dir, WorkspaceAdd::new(ws_name.clone(), base, path))
        .await?;
    // `workspace add -r <base>` puts a fresh empty change on the new workspace's
    // `@`; `<ws_name>@` resolves to it regardless of the cwd. Anchor the bookmark
    // there so the worktree carries the requested branch.
    let revset = format!("{ws_name}@");
    jj.bookmark_create(dir, branch, &revset).await?;
    Ok(CreateOutcome::Plain)
}

pub(crate) async fn remove_worktree<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    path: &Path,
    _force: bool,
) -> Result<()> {
    let name = workspace_name_for_path(jj, dir, path).await?;
    // Delete the on-disk dir first: an orphan dir jj has forgotten is worse than
    // a still-attached workspace.
    if path.exists() {
        std::fs::remove_dir_all(path)?;
    }
    // Best-effort: jj happily forgets an already-deleted workspace dir.
    let _ = jj.workspace_forget(dir, &name).await;
    Ok(())
}

/// Derive a jj workspace name from a branch name. jj workspace names must be
/// valid identifiers, so substitute path/whitespace characters with `_`.
/// Deterministic so a later lookup can reconstruct it.
fn workspace_name_for(branch: &str) -> String {
    branch
        .chars()
        .map(|c| match c {
            '/' | '\\' | '.' | ':' | ' ' | '\t' | '\n' | '\r' => '_',
            other => other,
        })
        .collect()
}

/// Find the workspace name whose `jj workspace root` matches `path`. Uses jj's
/// recorded name rather than a re-derived guess, so a branch containing `/`
/// resolves correctly.
async fn workspace_name_for_path<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    path: &Path,
) -> Result<String> {
    let target = normalize_for_compare(path);
    for ws in jj.workspace_list(dir).await? {
        let Ok(root) = jj.workspace_root(dir, Some(ws.name.clone())).await else {
            continue;
        };
        if normalize_for_compare(&root) == target || root == path {
            return Ok(ws.name);
        }
    }
    Err(Error::WorktreeNotFound(path.to_path_buf()))
}

/// Normalise a path for comparison against jj's `workspace root` output:
/// canonicalize (resolve symlinks / macOS case) and strip the Windows verbatim
/// prefix (`\\?\…`, which `canonicalize` adds but jj never emits).
fn normalize_for_compare(p: &Path) -> PathBuf {
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

/// Project a `jj diff --summary` entry into a [`FileChange`]. For a rename/copy
/// the parser supplies the original path; otherwise `old_path` is `None`.
fn file_change_from_summary(entry: ChangedPath) -> FileChange {
    FileChange {
        kind: change_kind_from_status(entry.status),
        path: entry.path,
        old_path: entry.old_path,
    }
}

/// Map a `jj diff --summary` status letter to a [`ChangeKind`].
fn change_kind_from_status(status: char) -> ChangeKind {
    match status {
        'A' | 'C' => ChangeKind::Added,
        'D' => ChangeKind::Deleted,
        'R' => ChangeKind::Renamed,
        _ => ChangeKind::Modified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_name_substitutes_invalid_chars() {
        assert_eq!(workspace_name_for("feature/x.y"), "feature_x_y");
        assert_eq!(workspace_name_for("plain"), "plain");
    }

    #[test]
    fn summary_status_maps_to_change_kind() {
        assert_eq!(change_kind_from_status('M'), ChangeKind::Modified);
        assert_eq!(change_kind_from_status('A'), ChangeKind::Added);
        assert_eq!(change_kind_from_status('C'), ChangeKind::Added);
        assert_eq!(change_kind_from_status('D'), ChangeKind::Deleted);
        assert_eq!(change_kind_from_status('R'), ChangeKind::Renamed);
    }
}
