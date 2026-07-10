//! Git-backed implementations of the facade operations: thin calls to the
//! `vcs-git` client plus pure mappers from its types into the facade DTOs.

use std::path::Path;

use processkit::ProcessRunner;
use vcs_git::{CheckoutTarget, Git, GitApi, GitPush, RefName, RevSpec, StatusEntry, WorktreeAdd};

use crate::dto::{
    ChangeKind, Commit, CreateOutcome, DiffStat, FileChange, MergeProbe, OperationState,
    RepoSnapshot, UpstreamTracking, WorktreeInfo,
};
use crate::error::{Error, Result};

pub(crate) async fn current_branch<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
) -> Result<Option<String>> {
    // `GitApi::current_branch` already maps a detached HEAD to `None`, mirroring
    // jj's `Option` bookmark — forward it directly.
    Ok(git.current_branch(dir).await?)
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
    Ok(git.branch_exists(dir, &RefName::new(name)?).await?)
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
    let mut spec = vcs_git::BranchDelete::new(RefName::new(name)?);
    if force {
        spec = spec.force();
    }
    git.delete_branch(dir, spec).await?;
    Ok(())
}

pub(crate) async fn rename_branch<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    old: &str,
    new: &str,
) -> Result<()> {
    git.rename_branch(dir, &RefName::new(old)?, &RefName::new(new)?)
        .await?;
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
    // The empty tree's id depends on the repo's object format, so ask git for the
    // format-correct one (`empty_tree_oid`) rather than a hard-coded SHA-1 value,
    // which doesn't exist in a SHA-256 repo. `git.diff_stat` already returns the
    // shared `vcs_diff::DiffStat` — no remap.
    let range: String = if git.is_unborn(dir).await? {
        git.empty_tree_oid(dir).await?
    } else {
        "HEAD".to_string()
    };
    // `range` here is always `HEAD` or a resolved empty-tree oid (no flag-like or
    // pathspec input), so the conversion never fails; it goes through the newtype
    // for a uniform boundary.
    git.diff_stat(dir, &RevSpec::new(&range)?)
        .await
        .map_err(Into::into)
}

pub(crate) async fn log<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    revspec: &str,
    max: usize,
) -> Result<Vec<Commit>> {
    Ok(git
        .log(dir, &RevSpec::new(revspec)?, max)
        .await?
        .into_iter()
        .map(|c| Commit::new(c.hash, c.subject).author(c.author).date(c.date))
        .collect())
}

pub(crate) async fn show_file<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    rev: &str,
    path: &str,
) -> Result<String> {
    Ok(git.show_file(dir, &RevSpec::new(rev)?, path).await?)
}

pub(crate) async fn snapshot<R: ProcessRunner>(git: &Git<R>, dir: &Path) -> Result<RepoSnapshot> {
    // 1 spawn: branch + upstream + ahead/behind + change counts (porcelain v2).
    let bs = git.branch_status(dir).await?;
    // 1 spawn: resolve the git dir, then a filesystem probe for an interrupted
    // merge/rebase/am/cherry-pick/revert/bisect (porcelain v2 doesn't report it). A
    // git conflict is part of that paused state, so `operation` is one of the git
    // sequencer states here (matching `in_progress_state`); the unresolved-files
    // signal is `conflicted`. Resolving the git dir once and reading the markers
    // inline keeps `snapshot` to a single extra spawn (it's the watcher's hot path),
    // rather than the per-marker `rev-parse` that `in_progress_state` delegates to.
    // Mirrors the client's private `resolved_git_dir` (relative `--git-dir` → join `dir`).
    let raw = git.git_dir(dir).await?;
    let git_dir = if raw.is_absolute() {
        raw
    } else {
        dir.join(raw)
    };
    // Same precedence as `in_progress_state`: `git am` and an apply-backend rebase
    // share `rebase-apply/`, but am marks it `applying` — check that first so an am
    // reads `ApplyMailbox`, not `Rebase` (M20). Cherry-pick/revert key off their own
    // head file (a conflict there writes `CHERRY_PICK_HEAD`/`REVERT_HEAD`, never
    // `MERGE_HEAD`); bisect keys off `BISECT_LOG`.
    let rebase_apply = git_dir.join("rebase-apply");
    let operation = if git_dir.join("MERGE_HEAD").exists() {
        OperationState::Merge
    } else if rebase_apply.join("applying").exists() {
        OperationState::ApplyMailbox
    } else if git_dir.join("rebase-merge").exists() || rebase_apply.exists() {
        OperationState::Rebase
    } else if git_dir.join("CHERRY_PICK_HEAD").exists() {
        OperationState::CherryPick
    } else if git_dir.join("REVERT_HEAD").exists() {
        OperationState::Revert
    } else if git_dir.join("BISECT_LOG").exists() {
        OperationState::Bisect
    } else {
        OperationState::Clear
    };
    // Derive before moving the String fields out of `bs`.
    let dirty = bs.is_dirty();
    let change_count = bs.tracked_changes + bs.untracked;
    let conflicted = bs.conflicts > 0;
    // Upstream and its ahead/behind counts are separate signals: git names the
    // upstream branch whenever one is configured, but reports the counts only when
    // that upstream ref actually resolves. So carry the counts as `Option` — a set-
    // but-gone upstream keeps `branch` with `ahead`/`behind: None` (uncountable),
    // instead of a `unwrap_or(0)` fabricating a false "in sync" (M17).
    let tracking = bs.upstream.map(|branch| UpstreamTracking {
        branch,
        ahead: bs.ahead,
        behind: bs.behind,
    });
    Ok(RepoSnapshot {
        head: bs.head,
        branch: bs.branch,
        tracking,
        dirty,
        change_count,
        conflicted,
        operation,
    })
}

pub(crate) async fn commit_paths<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    paths: &[String],
    message: &str,
) -> Result<()> {
    git.commit_paths(
        dir,
        vcs_git::CommitPaths::new(paths.iter().map(String::as_str), message),
    )
    .await?;
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

pub(crate) async fn fetch_branch<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    branch: &str,
) -> Result<()> {
    git.fetch_branch(dir, &RefName::new(branch)?).await?;
    Ok(())
}

pub(crate) async fn push<R: ProcessRunner>(git: &Git<R>, dir: &Path, branch: &str) -> Result<()> {
    // `-u` so the first facade push also records the upstream — the facade has
    // no separate set-upstream step, and `-u` on later pushes is idempotent.
    git.push(dir, GitPush::branch(RefName::new(branch)?).set_upstream())
        .await?;
    Ok(())
}

pub(crate) async fn checkout<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    reference: &str,
) -> Result<()> {
    git.checkout(dir, &checkout_target(reference)?).await?;
    Ok(())
}

/// Map a facade checkout string to a validated [`CheckoutTarget`] at the boundary:
/// git's `-` "previous branch" shortcut is its own variant (a safe fixed literal),
/// everything else is a validated [`RevSpec`] — so a flag-like value is refused
/// here with a classifiable input-validation error rather than reaching argv.
fn checkout_target(reference: &str) -> Result<CheckoutTarget> {
    if reference == "-" {
        Ok(CheckoutTarget::Previous)
    } else {
        Ok(CheckoutTarget::Ref(RevSpec::new(reference)?))
    }
}

pub(crate) async fn new_child<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    reference: &str,
) -> Result<()> {
    checkout(git, dir, reference).await
}

pub(crate) async fn rebase<R: ProcessRunner>(git: &Git<R>, dir: &Path, onto: &str) -> Result<()> {
    git.rebase(dir, &RevSpec::new(onto)?).await?;
    Ok(())
}

pub(crate) async fn try_merge<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    source: &str,
) -> Result<MergeProbe> {
    // `--no-ff` so even a fast-forwardable merge stages a real (abortable) merge
    // instead of moving HEAD; `--no-commit` so nothing is committed either way.
    let merged = git
        .merge_no_commit(
            dir,
            vcs_git::MergeNoCommit::branch(RevSpec::new(source)?).no_ff(),
        )
        .await;
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
            // clears the unmerged index entries this reads. Don't `?` the read
            // yet: abort first regardless, so a transient read failure can't
            // leave the probe merge staged in the working tree (matching the jj
            // path, which always restores).
            let files = git.conflicted_files(dir).await;
            // A failed abort breaks the guaranteed-rollback contract → propagate
            // rather than return a `Conflicts` that lies about the tree state.
            git.merge_abort(dir).await?;
            Ok(MergeProbe::Conflicts(files?))
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
    // Each state aborts with its OWN git command — dispatching the wrong one on a
    // real repository is exactly what the distinct states guard against. `Clear`
    // and `Conflict` have nothing to abort (a git conflict IS the paused state, so
    // it never reaches here from `in_progress_state`), so they no-op honestly.
    match in_progress_state(git, dir).await? {
        OperationState::Merge => git.merge_abort(dir).await?,
        OperationState::Rebase => git.rebase_abort(dir).await?,
        OperationState::ApplyMailbox => git.am_abort(dir).await?,
        OperationState::CherryPick => git.cherry_pick_abort(dir).await?,
        OperationState::Revert => git.revert_abort(dir).await?,
        OperationState::Bisect => git.bisect_reset(dir).await?,
        OperationState::Clear | OperationState::Conflict => {}
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
    // Merge finishes with a plain commit; the sequencer states (rebase, cherry-pick,
    // revert) each have a `--continue` that can stop AGAIN on the next commit's
    // conflict (exit non-zero) — that's the `Conflict` outcome, not an error. Bisect
    // has no continue step, so it is refused **explicitly** rather than pretending to
    // succeed while still mid-bisect. `am --continue` isn't wired here (unchanged).
    match in_progress_state(git, dir).await? {
        OperationState::Merge => git.merge_continue(dir).await?,
        state @ (OperationState::Rebase | OperationState::CherryPick | OperationState::Revert) => {
            let continued = match state {
                OperationState::CherryPick => git.cherry_pick_continue(dir).await,
                OperationState::Revert => git.revert_continue(dir).await,
                _ => git.rebase_continue(dir).await,
            };
            if let Err(err) = continued {
                if !git.conflicted_files(dir).await?.is_empty() {
                    return Ok(OperationState::Conflict);
                }
                return Err(err.into());
            }
        }
        OperationState::Bisect => {
            return Err(Error::Unsupported(
                "a git bisect has no continue step — mark commits with `git bisect \
                 good`/`bad`, or end it with abort_in_progress (`bisect reset`)"
                    .to_string(),
            ));
        }
        OperationState::ApplyMailbox | OperationState::Clear | OperationState::Conflict => {}
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
    // git surfaces an interrupted operation as on-disk state; at most one of these is
    // live, so report whichever is present. The precedence mirrors git's own
    // `wt_status_get_state` (merge → am/rebase → cherry-pick → revert, bisect
    // independent) and is safe because the markers are mutually exclusive in
    // practice: a cherry-pick/revert conflict writes `CHERRY_PICK_HEAD`/`REVERT_HEAD`
    // (never `MERGE_HEAD`), and a rebase that internally cherry-picks does NOT set
    // `CHERRY_PICK_HEAD`. `git am` is checked distinctly from rebase (both use
    // `rebase-apply/`, but am marks it `applying`) so an am isn't mis-aborted with
    // `rebase --abort` (M20). Keep this in step with the `snapshot` probe below.
    if git.is_merge_in_progress(dir).await? {
        Ok(OperationState::Merge)
    } else if git.is_am_in_progress(dir).await? {
        Ok(OperationState::ApplyMailbox)
    } else if git.is_rebase_in_progress(dir).await? {
        Ok(OperationState::Rebase)
    } else if git.is_cherry_pick_in_progress(dir).await? {
        Ok(OperationState::CherryPick)
    } else if git.is_revert_in_progress(dir).await? {
        Ok(OperationState::Revert)
    } else if git.is_bisect_in_progress(dir).await? {
        Ok(OperationState::Bisect)
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
    git.worktree_add(
        dir,
        WorktreeAdd::create_branch(path, RefName::new(branch)?, RevSpec::new(base)?),
    )
    .await?;
    Ok(CreateOutcome::Plain)
}

pub(crate) async fn remove_worktree<R: ProcessRunner>(
    git: &Git<R>,
    dir: &Path,
    path: &Path,
    force: bool,
) -> Result<()> {
    let mut spec = vcs_git::WorktreeRemove::new(path);
    if force {
        spec = spec.force();
    }
    git.worktree_remove(dir, spec).await?;
    Ok(())
}

/// Project a `git status --porcelain` entry into a [`FileChange`].
fn file_change_from_status(entry: StatusEntry) -> FileChange {
    FileChange {
        kind: change_kind_from_code(&entry.code),
        path: entry.path,
        old_path: entry.old_path,
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
