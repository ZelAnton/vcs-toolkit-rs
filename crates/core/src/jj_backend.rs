//! Jujutsu-backed implementations of the facade operations.
//!
//! jj's model differs from git's: workspaces are *named*, not path-addressed, and
//! `jj workspace list` carries no path — so worktree lookups resolve a name by
//! matching `jj workspace root --name <n>` against the requested path. The
//! copy-on-write / op-log-rollback creation flow stays in the consumer; the
//! facade only does the plain `jj workspace add` path.

use std::path::{Path, PathBuf};

use processkit::ProcessRunner;
use vcs_jj::{BookmarkName, ChangedPath, Jj, JjApi, JjFileset, RevsetExpr, WorkspaceAdd};

use crate::dto::{
    ChangeKind, Commit, CreateOutcome, DiffStat, FileChange, MergeProbe, OperationState,
    RepoSnapshot, WorktreeInfo,
};
use crate::error::{Error, Result};

/// Validate a facade revset string into a [`RevsetExpr`] at the boundary, mapping
/// a rejected value to a classifiable input-validation error. The internal `"@"`
/// callers pass a fixed literal (never fails); user-supplied revsets are checked.
fn rev(s: &str) -> Result<RevsetExpr> {
    Ok(RevsetExpr::new(s)?)
}

pub(crate) async fn current_branch<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
) -> Result<Option<String>> {
    // jj has no "current branch" in the git sense: after `jj describe` /
    // `jj new` / `jj commit` the bookmark stays on the described parent while
    // the new working-copy change carries none, so a strict "bookmark on `@`"
    // probe returns `None` right after a commit. Report the nearest bookmark
    // reachable from `@` instead (revset `heads(::@ & bookmarks())`), which
    // keeps the answer non-empty across a commit — git's "I'm still on my
    // branch" reporting. The strict "does `@` itself carry a bookmark" question
    // (e.g. to decide whether `jj git push` would push `@`) stays on
    // `vcs_jj::JjApi::current_bookmark`.
    //
    // Tie-break: `heads(::@ & bookmarks())` can yield several equally-near
    // bookmarks — a merge of two bookmarked lines (one head each), or one commit
    // carrying several. Pick the lexicographically-smallest name so the answer is
    // deterministic instead of dependent on jj's row order.
    Ok(jj
        .reachable_bookmarks(dir)
        .await?
        .into_iter()
        .map(|b| b.name)
        .min())
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
    if !jj.current_change(dir).await?.empty {
        return Ok(true);
    }
    // A **conflicted** change is uncommitted state (it needs resolution) even when jj
    // marks it `empty` — so `has_uncommitted_changes` agrees with `snapshot().dirty`,
    // which already treats `conflict ⇒ dirty` (M18). Only probed when `@` is empty, so
    // the common non-empty case stays a single query.
    Ok(jj.is_conflicted(dir, &rev("@")?).await?)
}

pub(crate) async fn conflicted_files<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
) -> Result<Vec<String>> {
    Ok(jj.resolve_list(dir, &rev("@")?).await?)
}

pub(crate) async fn delete_branch<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    name: &str,
) -> Result<()> {
    jj.bookmark_delete(dir, &BookmarkName::new(name)?).await?;
    Ok(())
}

pub(crate) async fn rename_branch<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    old: &str,
    new: &str,
) -> Result<()> {
    jj.bookmark_rename(dir, &BookmarkName::new(old)?, &BookmarkName::new(new)?)
        .await?;
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
    // `jj.diff_stat` already returns the shared `vcs_diff::DiffStat` — no remap.
    jj.diff_stat(dir, &rev("@")?).await.map_err(Into::into)
}

pub(crate) async fn log<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    revset: &str,
    max: usize,
) -> Result<Vec<Commit>> {
    // `JjApi::log`'s typed `Change` carries no author/timestamp (its template
    // renders only change-id/commit-id/empty/description) — so, unlike git,
    // `author`/`date` stay `None` here rather than being guessed (see the
    // `Commit` type docs).
    Ok(jj
        .log(dir, &rev(revset)?, max)
        .await?
        .into_iter()
        .map(|c| Commit::new(c.commit_id, c.description))
        .collect())
}

pub(crate) async fn show_file<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    revset: &str,
    path: &str,
) -> Result<String> {
    Ok(jj.file_show(dir, &rev(revset)?, path).await?)
}

/// One `jj log -r @` template carrying the working-copy-only fields the
/// snapshot needs except the change count: the full commit id (`head` is the
/// full oid on both backends — truncate for display; a short id would make a
/// fixed-width truncation panic), the `empty` flag (→ dirty), and the
/// `conflict` flag — all bare keywords valid in the `jj log` commit context.
/// The branch comes from [`current_branch`] (the nearest reachable bookmark),
/// not `@`'s own bookmarks, so the snapshot's `branch` can't disagree with
/// `Repo::current_branch` after a commit.
const SNAPSHOT_TEMPLATE: &str = "commit_id ++ \"\\t\" ++ \
    if(empty, \"1\", \"0\") ++ \"\\t\" ++ if(conflict, \"1\", \"0\")";

pub(crate) async fn snapshot<R: ProcessRunner>(jj: &Jj<R>, dir: &Path) -> Result<RepoSnapshot> {
    // Spawn 1: head/empty/conflict for `@`. Spawn 2: `branch` via
    // `current_branch` (the nearest reachable bookmark). Spawn 3, only when
    // dirty: the change count.
    let row = jj
        .template_query(dir, &rev("@")?, SNAPSHOT_TEMPLATE, Some(1))
        .await?;
    let line = row.trim_end_matches(['\r', '\n']);
    let fields: Vec<&str> = line.split('\t').collect();
    // SNAPSHOT_TEMPLATE renders exactly three tab-separated fields: commit_id,
    // the empty-flag, and the conflict-flag. A different arity means the
    // template / jj contract drifted — debug-assert it (so tests and debug
    // builds catch a template edit) and read each field by position so a
    // release build still returns a *coherent* snapshot rather than one whose
    // `dirty` flag flips on a truncated row.
    debug_assert_eq!(
        fields.len(),
        3,
        "jj snapshot template arity drift (expected 3 tab fields): {line:?}"
    );
    let head = fields
        .first()
        .copied()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let branch = current_branch(jj, dir).await?;
    // Read the flags as explicit values: `conflict == "1"` ⇒ conflicted, and
    // `empty == "0"` ⇒ a non-empty change ⇒ dirty (so a missing/garbled field falls
    // to clean, not a contradictory "dirty with 0 changes"). A **conflicted** change
    // is also dirty even when jj marks it `empty`: the conflict is uncommitted state
    // needing resolution — exactly as git reports conflict markers as unstaged
    // changes — so cross-backend `dirty` stays consistent (no `conflicted: true`
    // alongside `dirty: false`).
    let conflicted = fields.get(2) == Some(&"1");
    let dirty = fields.get(1) == Some(&"0") || conflicted;
    // jj has no paused merge/rebase; a conflict is recorded on the change itself.
    let operation = if conflicted {
        OperationState::Conflict
    } else {
        OperationState::Clear
    };
    // 2nd spawn only when there's something to count (dirty now includes the
    // conflicted case, so the count reflects the conflicted files too).
    let change_count = if dirty {
        jj.status(dir).await?.len()
    } else {
        0
    };
    Ok(RepoSnapshot {
        head,
        branch,
        // jj has no git-style upstream tracking.
        tracking: None,
        dirty,
        change_count,
        conflicted,
        operation,
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

pub(crate) async fn fetch_from<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    remote: &str,
) -> Result<()> {
    jj.git_fetch_from(dir, remote).await?;
    Ok(())
}

pub(crate) async fn fetch_branch<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    branch: &str,
) -> Result<()> {
    jj.git_fetch_branch(dir, &BookmarkName::new(branch)?)
        .await?;
    Ok(())
}

pub(crate) async fn push<R: ProcessRunner>(jj: &Jj<R>, dir: &Path, branch: &str) -> Result<()> {
    // jj pushes *bookmark state* (`git push -b <name>`); jj configures the
    // tracking relationship itself, so there is no `-u` analogue to mirror.
    // The bookmark rides the `-b` flag-VALUE slot, so it is deliberately not
    // guarded (the documented convention — same as `rebase`/`fetch_from`'s jj
    // paths): jj consumes the token as a name and errors on a nonexistent
    // bookmark. Only the git path guards, because there the branch lands in a
    // *bare positional* refspec slot where a `--flag` would be parsed as one.
    jj.git_push(dir, Some(BookmarkName::new(branch)?)).await?;
    Ok(())
}

pub(crate) async fn checkout<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    reference: &str,
) -> Result<()> {
    // jj has no "switch branch"; moving `@` to the bookmark/revision is the
    // equivalent of a git checkout.
    jj.edit(dir, &rev(reference)?).await?;
    Ok(())
}

pub(crate) async fn new_child<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    reference: &str,
) -> Result<()> {
    jj.new_child(dir, &rev(reference)?).await?;
    Ok(())
}

pub(crate) async fn rebase<R: ProcessRunner>(jj: &Jj<R>, dir: &Path, onto: &str) -> Result<()> {
    jj.rebase(dir, &rev(onto)?).await?;
    Ok(())
}

pub(crate) async fn try_merge<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    source: &str,
) -> Result<MergeProbe> {
    // Capture the rollback point BEFORE any mutation.
    let pre_op = jj.op_head(dir).await?;
    // Materialise the merge as a new working-copy change; jj records conflicts
    // on the commit instead of failing, so a 0 exit does NOT mean "clean".
    // Validate the revsets at the boundary before the mutation; `@` is a fixed
    // literal and `source` is the caller's, so an invalid `source` is a
    // classifiable input error rather than a spawn failure.
    let wc = rev("@")?;
    let merged = jj
        .new_merge(
            dir,
            "vcs-core try_merge probe (rolled back)",
            vec![wc.clone(), rev(source)?],
        )
        .await;
    // Probe the outcome before restoring (the probe target disappears after).
    // If `new_merge` itself failed, a failing probe must not mask that error.
    let probe = async {
        if jj.is_conflicted(dir, &wc).await? {
            Ok::<_, vcs_jj::Error>(Some(jj.resolve_list(dir, &wc).await?))
        } else {
            Ok(None)
        }
    }
    .await;
    // Always roll back — also when the merge or the probe errored.
    let restored = jj.op_restore(dir, &pre_op).await;
    match (merged, probe) {
        (Ok(()), Ok(conflicts)) => {
            // The probe is only trustworthy if the rollback actually happened —
            // a `Clean`/`Conflicts` with the probe commit still present lies.
            restored?;
            Ok(match conflicts {
                Some(files) => MergeProbe::Conflicts(files),
                None => MergeProbe::Clean,
            })
        }
        // The merge succeeded but the probe errored. Surface a *failed* rollback
        // first — it means the probe change is still in the working copy, the
        // condition the caller must act on (mirrors the git path's abort-failure
        // propagation); otherwise surface the probe error.
        (Ok(()), Err(err)) => {
            restored?;
            Err(err.into())
        }
        // The merge itself failed — that's the root cause; a secondary
        // restore/probe failure must not mask it.
        (Err(err), _) => Err(err.into()),
    }
}

pub(crate) async fn abort_in_progress<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
) -> Result<OperationState> {
    // jj has no paused operations to abort — a conflict lives on the change
    // itself. Roll back explicitly via `Jj::transaction` / `op_restore` instead;
    // this only reports the current state.
    in_progress_state(jj, dir).await
}

pub(crate) async fn continue_in_progress<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
) -> Result<OperationState> {
    // jj has nothing to continue — resolving the conflicted files *is* the
    // continuation. This only reports the current state.
    in_progress_state(jj, dir).await
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
    // jj's `Workspace` carries no path, so resolve each via `workspace root` —
    // batched in one bounded fan-out rather than awaited one workspace at a time.
    let workspaces = jj.workspace_list(dir).await?;
    let names: Vec<String> = workspaces.iter().map(|ws| ws.name.clone()).collect();
    let roots = jj.workspace_roots(dir, &names).await;
    // `workspace_roots` returns exactly one result per name (it's a fan-out over
    // `names`), so the `zip` below is 1:1. Pin that invariant: if it ever drifted to
    // returning only the *successful* rows, `zip` would silently drop the tail
    // workspaces from the listing rather than erroring.
    debug_assert_eq!(
        names.len(),
        roots.len(),
        "workspace_roots must return one result per name"
    );
    let mut out = Vec::new();
    for (ws, root) in workspaces.into_iter().zip(roots) {
        let Ok(root) = root else {
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
    // `jj workspace add` runs with cwd = `dir` and resolves a *relative* `path`
    // against `dir`. Resolve it the same way for our own filesystem ops, so a `Repo`
    // bound to a directory != the process cwd (e.g. `vcs-mcp --repo /elsewhere`)
    // probes/deletes the location jj actually used, not one under the process cwd.
    // `dir.join(path)` returns `path` unchanged when it's already absolute.
    let abs_path = dir.join(path);
    // Whether the destination existed *before* we touched it. `jj workspace add`
    // creates the directory itself, so a pre-existing path is not ours to delete:
    // the rollback below must not `remove_dir_all` a directory the caller already
    // had (that would be silent data loss on an unrelated failure).
    let preexisting = abs_path.exists();
    jj.workspace_add(dir, WorkspaceAdd::new(ws_name.clone(), rev(base)?, path))
        .await?;
    // `workspace add -r <base>` puts a fresh empty change on the new workspace's
    // `@`; `<ws_name>@` resolves to it regardless of the cwd. Anchor the bookmark
    // there so the worktree carries the requested branch.
    let revset = format!("{ws_name}@");
    if let Err(e) = jj
        .bookmark_create(dir, &BookmarkName::new(branch)?, &rev(&revset)?)
        .await
    {
        // The two steps aren't atomic: `workspace add` already created the
        // workspace and its on-disk dir, but the bookmark didn't land. Roll back
        // so a failed call doesn't leak a half-made worktree — mirror
        // `remove_worktree` (delete the dir first, then forget the workspace
        // best-effort), then surface the original error. Only remove the dir if we
        // created it (it didn't exist before `workspace add`).
        if !preexisting && abs_path.exists() {
            let _ = std::fs::remove_dir_all(&abs_path);
        }
        let _ = jj.workspace_forget(dir, &ws_name).await;
        return Err(e.into());
    }
    Ok(CreateOutcome::Plain)
}

/// jj's initial workspace — its directory is the repository's main working copy,
/// so it must never be deleted by a worktree-removal call.
const DEFAULT_WORKSPACE: &str = "default";

pub(crate) async fn remove_worktree<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    path: &Path,
    force: bool,
) -> Result<()> {
    // Resolve `path` against `dir` (jj's cwd) so the workspace lookup and the dir
    // removal target the location jj used, even when the process cwd differs.
    let abs_path = dir.join(path);
    let name = workspace_name_for_path(jj, dir, &abs_path).await?;

    // Never remove the repository's **main** workspace: its directory *is* the
    // main working copy, so `remove_dir_all` on it wipes the whole checkout
    // (`.jj`/`.git` and every file). git refuses to remove the main worktree; jj
    // has no such guard and we delete the directory ourselves, so guard it here.
    // Two signals, because either alone is bypassable:
    //   - the name is `default` (the initial workspace's name); but
    //     `jj workspace rename` can move the main workspace off that name, so
    //   - the workspace directory owns the object store: a *main* workspace's
    //     `.jj/repo` is a directory (the store), a *secondary* workspace's is a
    //     file (a pointer to the store) — verified on jj 0.42, and stable across
    //     a rename. If either holds, this is the repository, not a stray worktree.
    if name == DEFAULT_WORKSPACE || abs_path.join(".jj").join("repo").is_dir() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to remove the repository's main workspace (its directory is \
             the main working copy and owns the object store)",
        )));
    }

    // Honor `force` like git's `worktree remove`: unless forced, refuse a
    // workspace that still has uncommitted changes. Querying `current_change`
    // there snapshots the working copy first (jj only records it when a command
    // runs in that workspace), so a refusal leaves the edits captured in jj's op
    // log rather than only on disk — and the check sees edits made since the last
    // jj command ran there, exactly the state git's `worktree remove` refuses on.
    // (Skip when the directory is already gone: nothing to lose, just re-forget.)
    if !force && abs_path.exists() && !jj.current_change(&abs_path).await?.empty {
        return Err(Error::Io(std::io::Error::other(
            "worktree has uncommitted changes; pass force = true to remove it \
             (the changes are snapshotted in jj's op log and recoverable)",
        )));
    }

    // Delete the on-disk dir first: an orphan dir jj has forgotten is worse than
    // a still-attached workspace. (This is a blocking `remove_dir_all` on the
    // async worker; vcs-core is deliberately runtime-agnostic — no tokio — so a
    // multi-GB worktree delete can briefly stall the caller's task. Offloading
    // it would couple the facade to one runtime, a worse trade for a library
    // meant to run under any executor; see docs/audit-2026-07.md P2.)
    if abs_path.exists() {
        std::fs::remove_dir_all(&abs_path)?;
    }
    // Then forget the workspace. jj happily forgets an already-deleted workspace
    // dir, so this normally succeeds; we *surface* a failure rather than swallow it
    // (name resolution above already proved the workspace is registered, so an
    // error here is a real dangling-registration the caller should see and can
    // retry — the dir is gone, so a retry skips straight back to this forget).
    jj.workspace_forget(dir, &name).await?;
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
    let workspaces = jj.workspace_list(dir).await?;
    let names: Vec<String> = workspaces.iter().map(|ws| ws.name.clone()).collect();
    let roots = jj.workspace_roots(dir, &names).await;
    // One result per name (see `list_worktrees`): a `zip` mismatch would silently
    // skip a workspace and wrongly report `WorktreeNotFound` for a real one.
    debug_assert_eq!(
        names.len(),
        roots.len(),
        "workspace_roots must return one result per name"
    );
    for (ws, root) in workspaces.into_iter().zip(roots) {
        let Ok(root) = root else {
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

    // A `ScriptedRunner` that also performs `jj workspace add`'s real side effect —
    // creating the destination directory — so a hermetic test can exercise the
    // rollback's "we created the dir, so clean it up" branch faithfully: the dir
    // does **not** exist when `create_worktree` is entered (matching the real flow,
    // where `workspace add` is what creates it), and only appears once the mocked
    // `workspace add` "runs".
    struct AddCreatesDir {
        inner: processkit::testing::ScriptedRunner,
        dir: std::path::PathBuf,
    }

    #[async_trait::async_trait]
    impl processkit::ProcessRunner for AddCreatesDir {
        async fn output_string(
            &self,
            command: &processkit::Command,
        ) -> processkit::Result<processkit::ProcessResult<String>> {
            let args: Vec<String> = command
                .arguments()
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            if args.iter().any(|a| a == "workspace") && args.iter().any(|a| a == "add") {
                let _ = std::fs::create_dir_all(&self.dir);
            }
            self.inner.output_string(command).await
        }
    }

    // R1: `create_worktree` is two non-atomic steps (`workspace add` then
    // `bookmark create`). If the bookmark step fails, the workspace dir that
    // `workspace add` just created must be cleaned up rather than leaked. Driven
    // hermetically with `AddCreatesDir` so the dir is born from the mocked
    // `workspace add` (not pre-created): `bookmark create` fails, and we assert the
    // error propagates and the dir is gone.
    #[tokio::test]
    async fn create_worktree_rolls_back_when_bookmark_step_fails() {
        use processkit::testing::{Reply, ScriptedRunner};
        use vcs_jj::Jj;
        use vcs_testkit::TempDir;

        let tmp = TempDir::new("r1-worktree-rollback");
        let repo = tmp.path();
        let wt = repo.join("wt");
        assert!(!wt.exists(), "the worktree dir must not pre-exist");

        let jj = Jj::with_runner(AddCreatesDir {
            dir: wt.clone(),
            inner: ScriptedRunner::new()
                .on(["jj", "workspace", "add"], Reply::ok(""))
                .on(
                    ["jj", "bookmark", "create"],
                    Reply::fail(1, "bookmark already exists\n"),
                )
                .on(["jj", "workspace", "forget"], Reply::ok("")),
        });

        let result = create_worktree(&jj, repo, &wt, "feature", "@").await;

        assert!(result.is_err(), "the bookmark-step failure must propagate");
        assert!(
            !wt.exists(),
            "the worktree dir that `workspace add` created must be cleaned up on rollback"
        );
    }

    // The rollback's complement: a directory that already existed when
    // `create_worktree` was entered is **not** deleted on a bookmark-step failure
    // (it isn't ours to remove). Here the dir pre-exists, so `AddCreatesDir` isn't
    // needed — a plain `ScriptedRunner` suffices.
    #[tokio::test]
    async fn create_worktree_rollback_spares_preexisting_dir() {
        use processkit::testing::{Reply, ScriptedRunner};
        use vcs_jj::Jj;
        use vcs_testkit::TempDir;

        let tmp = TempDir::new("r1-worktree-spare");
        let repo = tmp.path();
        let wt = repo.join("existing");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join("keep.txt"), b"mine").unwrap(); // sentinel

        let jj = Jj::with_runner(
            ScriptedRunner::new()
                .on(["jj", "workspace", "add"], Reply::ok(""))
                .on(
                    ["jj", "bookmark", "create"],
                    Reply::fail(1, "bookmark already exists\n"),
                )
                .on(["jj", "workspace", "forget"], Reply::ok("")),
        );

        let result = create_worktree(&jj, repo, &wt, "feature", "@").await;

        assert!(result.is_err(), "the bookmark-step failure must propagate");
        assert!(
            wt.join("keep.txt").exists(),
            "a pre-existing directory must survive the rollback untouched"
        );
    }

    // A **relative** worktree path is resolved against `dir` (jj's cwd), NOT the
    // process cwd. jj's mocked `workspace add` creates `dir/<rel>`; the rollback
    // must remove exactly that, even though the relative path resolved against the
    // process cwd would point somewhere else entirely.
    #[tokio::test]
    async fn create_worktree_resolves_relative_path_against_dir() {
        use processkit::testing::{Reply, ScriptedRunner};
        use std::path::Path;
        use vcs_jj::Jj;
        use vcs_testkit::TempDir;

        let tmp = TempDir::new("r1-worktree-relpath");
        let repo = tmp.path(); // an absolute repo dir, almost certainly != the process cwd
        let rel = Path::new("rel-wt");
        let resolved = repo.join(rel); // where jj actually creates it
        assert!(!resolved.exists());

        let jj = Jj::with_runner(AddCreatesDir {
            dir: resolved.clone(), // the mocked `workspace add` creates dir/<rel>
            inner: ScriptedRunner::new()
                .on(["jj", "workspace", "add"], Reply::ok(""))
                .on(
                    ["jj", "bookmark", "create"],
                    Reply::fail(1, "bookmark already exists\n"),
                )
                .on(["jj", "workspace", "forget"], Reply::ok("")),
        });

        let result = create_worktree(&jj, repo, rel, "feature", "@").await;

        assert!(result.is_err(), "the bookmark-step failure must propagate");
        assert!(
            !resolved.exists(),
            "the rollback must remove dir/<rel>, the location jj created"
        );
    }
}
