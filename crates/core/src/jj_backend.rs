//! Jujutsu-backed implementations of the facade operations.
//!
//! jj's model differs from git's: workspaces are *named*, not path-addressed, and
//! `jj workspace list` carries no path — so worktree lookups resolve a name by
//! matching `jj workspace root --name <n>` against the requested path. The
//! copy-on-write / op-log-rollback creation flow stays in the consumer; the
//! facade only does the plain `jj workspace add` path.

use std::path::{Path, PathBuf};

use processkit::ProcessRunner;
use vcs_jj::{
    BookmarkName, ChangedPath, Jj, JjApi, JjFileset, OutputBudget, RevsetExpr, Rollback,
    WorkspaceAdd,
};

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

/// Whether a snapshot/branch query lets jj snapshot the working copy.
///
/// An ordinary jj query snapshots the working copy first — taking the lock,
/// importing bare edits into a fresh `@`, and **recording a new operation** — so
/// it mutates the very state it reads. [`Observe::ReadOnly`] runs the same query
/// through vcs-jj's `--ignore-working-copy` variants: it reports the state of the
/// last recorded operation without a lock, a new operation, or moving `@` — the
/// mode an *observer* (a repo watcher / prompt) needs. The trade-off is that a
/// bare working-tree edit jj has not yet snapshotted is invisible to a
/// [`ReadOnly`](Observe::ReadOnly) read (it reflects the last operation, not
/// unsaved edits); a caller that must see such edits uses [`Live`](Observe::Live)
/// and accepts the recorded operation.
#[derive(Clone, Copy)]
enum Observe {
    /// jj's default: snapshot the working copy (records an operation, may move `@`).
    Live,
    /// `--ignore-working-copy`: read the last recorded operation's state, recording
    /// no operation and never moving `@`.
    ReadOnly,
}

/// The nearest bookmark reachable from `@`, letting `observe` decide whether jj
/// may snapshot the working copy first (see [`Observe`]).
async fn current_branch_with<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    observe: Observe,
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
    let bookmarks = match observe {
        Observe::Live => jj.reachable_bookmarks(dir).await?,
        Observe::ReadOnly => jj.reachable_bookmarks_ignoring_working_copy(dir).await?,
    };
    Ok(bookmarks.into_iter().map(|b| b.name).min())
}

pub(crate) async fn current_branch<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
) -> Result<Option<String>> {
    current_branch_with(jj, dir, Observe::Live).await
}

pub(crate) async fn trunk<R: ProcessRunner>(jj: &Jj<R>, dir: &Path) -> Result<Option<String>> {
    Ok(jj.trunk(dir).await?)
}

async fn local_branches_with<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    observe: Observe,
) -> Result<Vec<String>> {
    let bookmarks = match observe {
        Observe::Live => jj.bookmarks(dir).await?,
        Observe::ReadOnly => jj.bookmarks_ignoring_working_copy(dir).await?,
    };
    Ok(bookmarks.into_iter().map(|b| b.name).collect())
}

pub(crate) async fn local_branches<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
) -> Result<Vec<String>> {
    local_branches_with(jj, dir, Observe::Live).await
}

/// [`local_branches`] as a **read-only** query: passes `--ignore-working-copy`,
/// so listing the bookmarks records no jj operation and never moves `@`. Built
/// for an observer (the repo watcher) that must not mutate the state it reads.
pub(crate) async fn local_branches_readonly<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
) -> Result<Vec<String>> {
    local_branches_with(jj, dir, Observe::ReadOnly).await
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
) -> Result<Vec<PathBuf>> {
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

pub(crate) async fn show_file_within<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    revset: &str,
    path: &str,
    budget: OutputBudget,
) -> Result<String> {
    Ok(jj
        .file_show_within(dir, &rev(revset)?, path, budget)
        .await?)
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
    snapshot_with(jj, dir, Observe::Live).await
}

/// [`snapshot`] as a **read-only** query: every underlying jj command passes
/// `--ignore-working-copy`, so the batched read records **no** jj operation and
/// never moves `@`. It reports the state of the last recorded operation, so a
/// bare working-tree edit jj has not yet snapshotted is not reflected — the
/// deliberate contract for an observer (the repo watcher) that must not perturb
/// the working copy it is reporting on. Callers that must observe such edits use
/// [`snapshot`] and accept the recorded operation.
pub(crate) async fn snapshot_readonly<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
) -> Result<RepoSnapshot> {
    snapshot_with(jj, dir, Observe::ReadOnly).await
}

/// Shared core of [`snapshot`] / [`snapshot_readonly`], selecting whether jj may
/// snapshot the working copy before each underlying query (see [`Observe`]). Under
/// [`Observe::ReadOnly`] every spawn is a `--ignore-working-copy` read, so the
/// three fields (`@`'s head/empty/conflict, the branch, the change count) all
/// reflect the **same** last-recorded operation — a coherent read-only snapshot,
/// not a mix of snapshotted and non-snapshotted views.
async fn snapshot_with<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    observe: Observe,
) -> Result<RepoSnapshot> {
    // Spawn 1: head/empty/conflict for `@`. Spawn 2: `branch` via
    // `current_branch` (the nearest reachable bookmark). Spawn 3, only when
    // dirty: the change count.
    let row = match observe {
        Observe::Live => {
            jj.template_query(dir, &rev("@")?, SNAPSHOT_TEMPLATE, Some(1))
                .await?
        }
        Observe::ReadOnly => {
            jj.template_query_ignoring_working_copy(dir, &rev("@")?, SNAPSHOT_TEMPLATE, Some(1))
                .await?
        }
    };
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
    let branch = current_branch_with(jj, dir, observe).await?;
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
    // conflicted case, so the count reflects the conflicted files too). Under
    // `ReadOnly` this counts the last recorded operation's changes without
    // snapshotting, matching the `empty`/`conflict` flags read above.
    let change_count = if dirty {
        match observe {
            Observe::Live => jj.status(dir).await?.len(),
            Observe::ReadOnly => jj.status_ignoring_working_copy(dir).await?.len(),
        }
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
    paths: &[PathBuf],
    message: &str,
) -> Result<()> {
    // jj's fileset language is text (`root-file:"<path>"`), so a path is rendered
    // through `to_string_lossy` here — jj itself does not accept a non-UTF-8 fileset
    // token, so this is jj's own limit, not a lossy step we introduce. The
    // byte-faithful cross-backend round-trip (status→commit) is exercised on git.
    let filesets: Vec<JjFileset> = paths
        .iter()
        .map(|p| JjFileset::path(p.to_string_lossy()))
        .collect();
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
    // Always roll back — also when the merge or the probe errored. Uses the shared
    // concurrency-safe protocol (`Jj::rollback_to`): the cleanup survives a cancelled
    // operation and refuses to clobber a concurrent process's work, rather than the
    // bare `op_restore` this used to run (T-036 — one rollback protocol, not two).
    let rollback = jj.rollback_to(dir, &pre_op).await;
    match (merged, probe) {
        (Ok(()), Ok(conflicts)) => {
            // The probe is only trustworthy if the rollback actually happened —
            // a `Clean`/`Conflicts` with the probe commit still present (a failed or
            // divergence-refused rollback) lies.
            rollback_result(rollback)?;
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
            rollback_result(rollback)?;
            Err(err.into())
        }
        // The merge itself failed — that's the root cause; a secondary
        // restore/probe failure must not mask it.
        (Err(err), _) => Err(err.into()),
    }
}

/// Turn a [`Rollback`] outcome into a facade [`Result`]: a completed (or unneeded)
/// rollback is `Ok`; a **failed** restore or a **divergence-refused** one is a
/// [`Error::Rollback`], carrying the structured outcome so the caller can tell them
/// apart. Used by [`try_merge`] to decide whether its probe result is trustworthy.
fn rollback_result(rollback: Rollback) -> Result<()> {
    match rollback {
        Rollback::Restored | Rollback::NotAttempted => Ok(()),
        diverged_or_failed => Err(Error::Rollback(diverged_or_failed)),
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
            // `ws.commit` is the workspace commit's **full** id (WORKSPACE_TEMPLATE
            // renders `commit_id`, not `.short()`), the same identity `snapshot`
            // reports as `head` — so `WorktreeInfo.commit` can be compared against
            // `RepoSnapshot.head` without a short-prefix collision (T-041).
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
    // Validate every argument the post-`workspace add` step will need — and build
    // the revset `workspace add` itself doesn't require — *before* the mutation
    // below. `workspace add -r <base>` puts a fresh empty change on the new
    // workspace's `@`; `<ws_name>@` resolves to it regardless of the cwd, so the
    // revset is computable purely from `ws_name` without touching the repo.
    // Doing this first means a rejected `branch` (or revset) returns via the `?`s
    // below without ever calling `workspace_add`, so no half-made worktree is
    // created that would need a rollback. Once `workspace_add` has run, every
    // later failure path must go through `rollback_failed_create` instead of an
    // early `?`, or it would leak the workspace/directory it already created.
    let bookmark_name = BookmarkName::new(branch)?;
    let anchor = rev(&format!("{ws_name}@"))?;
    jj.workspace_add(dir, WorkspaceAdd::new(ws_name.clone(), rev(base)?, path))
        .await?;
    // Anchor the bookmark on the fresh workspace's `@` so the worktree carries the
    // requested branch.
    if let Err(e) = jj.bookmark_create(dir, &bookmark_name, &anchor).await {
        // The two steps aren't atomic: `workspace add` already created the workspace
        // and (unless it pre-existed) its on-disk dir, but the bookmark didn't land.
        // Roll back the half-made worktree — but report any cleanup residue rather
        // than swallowing it, so a leaked dir or a dangling registration is visible
        // and the caller can finish (and safely re-run) the cleanup.
        return Err(rollback_failed_create(jj, dir, &ws_name, &abs_path, preexisting, e).await);
    }
    Ok(CreateOutcome::Plain)
}

/// Roll back a [`create_worktree`] whose `bookmark create` step failed, **reporting**
/// any cleanup residue instead of swallowing it. Removes the workspace directory
/// (only when `create_worktree` created it — a `preexisting` dir is the caller's data
/// and is left untouched, never `remove_dir_all`'d) and forgets the workspace, then
/// composes the outcome: the `bookmark create` failure is the root `cause`; a
/// directory that could not be removed or a registration that could not be forgotten
/// is appended as still-to-clean state (so a partial rollback is diagnosable and can
/// be re-run). A fully clean rollback surfaces the original `cause` unchanged.
async fn rollback_failed_create<R: ProcessRunner>(
    jj: &Jj<R>,
    dir: &Path,
    ws_name: &str,
    abs_path: &Path,
    preexisting: bool,
    cause: processkit::Error,
) -> Error {
    let mut residue: Vec<String> = Vec::new();
    // Only remove a dir WE created — a pre-existing one is not ours to delete
    // (removing it would be silent data loss on an unrelated failure).
    if !preexisting
        && abs_path.exists()
        && let Err(e) = std::fs::remove_dir_all(abs_path)
    {
        residue.push(format!(
            "the workspace directory {} could not be removed ({e})",
            abs_path.display()
        ));
    }
    if let Err(e) = jj.workspace_forget(dir, ws_name).await {
        residue.push(format!(
            "the workspace `{ws_name}` could not be forgotten ({e})"
        ));
    }
    if residue.is_empty() {
        // A clean rollback: the bookmark-step failure is the whole story, surfaced
        // with its original classification.
        return Error::Vcs(cause);
    }
    Error::Io(std::io::Error::other(format!(
        "creating the worktree failed at `bookmark create` ({cause}), and the rollback \
         could not fully clean up: {}. Finish the cleanup manually and retry.",
        residue.join("; ")
    )))
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
        std::fs::remove_dir_all(&abs_path).map_err(|e| {
            // Report what remains so the failure is diagnosable and the cleanup is
            // repeatable: the directory is still on disk AND the workspace is still
            // registered (its name was resolved above). Once the directory is free, a
            // retry re-resolves the same name and finishes the removal + forget.
            Error::Io(std::io::Error::new(
                e.kind(),
                format!(
                    "failed to remove the worktree directory {} ({e}); the jj workspace \
                     `{name}` is still registered — free the directory and retry",
                    abs_path.display()
                ),
            ))
        })?;
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
    // Registered workspaces whose root couldn't be resolved via `workspace root
    // --name`: a no-match must NOT be reported as a clean `WorktreeNotFound` when we
    // merely failed to place a registered workspace — `path` may well be that one.
    let mut unresolved: Vec<String> = Vec::new();
    for (ws, root) in workspaces.into_iter().zip(roots) {
        match root {
            Ok(root) => {
                // Shared with `vcs_jj::blocking::workspace_name_for_path` (the
                // Drop-path resolver) so both sides answer "does this path
                // resolve to a workspace" identically (T-080).
                if vcs_jj::workspace_root_matches(&root, path) {
                    return Ok(ws.name);
                }
            }
            Err(_) => unresolved.push(ws.name),
        }
    }
    if unresolved.is_empty() {
        // Every registered workspace resolved and none matched: a genuine miss.
        Err(Error::WorktreeNotFound(path.to_path_buf()))
    } else {
        // Some registered workspaces did not resolve, so `path`'s absence can't be
        // proven — surface a DISTINCT, diagnosable error (not a clean
        // `WorktreeNotFound`, so `is_resource_not_found` stays false) that names the
        // unresolved workspaces, rather than misreporting a real one as "not found".
        Err(Error::Io(std::io::Error::other(format!(
            "could not resolve the worktree at {}: {} registered workspace(s) did not \
             resolve via `jj workspace root --name` ({}); the path may belong to one of \
             them — resolve or `jj workspace forget` it manually",
            path.display(),
            unresolved.len(),
            unresolved.join(", "),
        ))))
    }
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

    // The async resolver delegates to `vcs_jj::workspace_root_matches`, which also
    // backs the blocking Drop-path resolver. Pin both UNC spellings here at the async
    // `workspace_name_for_path` boundary so either side continues to resolve the
    // registered workspace when canonicalisation cannot access the UNC root.
    #[cfg(windows)]
    #[tokio::test]
    async fn workspace_name_for_path_matches_verbatim_unc_in_both_directions() {
        use processkit::testing::{Reply, ScriptedRunner};
        use vcs_jj::Jj;

        for (root, path) in [
            (
                r"\\?\UNC\server\share\workspace",
                r"\\server\share\workspace",
            ),
            (
                r"\\server\share\workspace",
                r"\\?\UNC\server\share\workspace",
            ),
        ] {
            let jj = Jj::with_runner(
                ScriptedRunner::new()
                    .on(
                        ["jj", "workspace", "list"],
                        Reply::ok("\"workspace\"\tc0ffee\t\n"),
                    )
                    .on(
                        [
                            "jj",
                            "--ignore-working-copy",
                            "workspace",
                            "root",
                            "--name",
                            "workspace",
                        ],
                        Reply::ok(format!("{root}\n")),
                    ),
            );

            assert_eq!(
                workspace_name_for_path(&jj, Path::new(r"C:\repo"), Path::new(path))
                    .await
                    .expect("the matching workspace must resolve"),
                "workspace"
            );
        }
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

    // T-064: an invalid `branch` must be rejected *before* `workspace add` ever
    // runs, so no half-made worktree is left for `rollback_failed_create` to clean
    // up in the first place. A whitespace-only branch is the reproducer:
    // `workspace_name_for("  ")` substitutes both spaces and yields the valid
    // workspace name `__`, so `jj workspace add` would happily succeed — but
    // `BookmarkName::new("  ")` rejects the value (empty after trim). Scripted
    // with only a `fallback` reply (no rule for `workspace add`/`bookmark
    // create`), wrapped in a `RecordingRunner` so the test asserts on *zero calls*
    // reaching the process seam at all, not just that `workspace add`'s specific
    // rule went unmatched.
    #[tokio::test]
    async fn create_worktree_rejects_invalid_branch_before_workspace_add() {
        use processkit::testing::{RecordingRunner, Reply, ScriptedRunner};
        use vcs_jj::Jj;
        use vcs_testkit::TempDir;

        let tmp = TempDir::new("t064-validate-before-mutate");
        let repo = tmp.path();
        let wt = repo.join("wt");

        let recorder = RecordingRunner::new(
            ScriptedRunner::new().fallback(Reply::fail(1, "no rule should ever match")),
        );
        let jj = Jj::with_runner(&recorder);

        let result = create_worktree(&jj, repo, &wt, "   ", "@").await;

        assert!(result.is_err(), "a whitespace-only branch must be rejected");
        assert!(
            result.unwrap_err().is_invalid_input(),
            "the rejection must classify as invalid input"
        );
        assert!(
            !wt.exists(),
            "no workspace directory must be created for a rejected branch"
        );
        assert!(
            recorder.calls().is_empty(),
            "no process call must run before the branch is validated"
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

    // R2: the rollback must NOT swallow a secondary cleanup error. `workspace add`
    // created the dir (which the rollback removes fine), but `workspace forget`
    // fails — the returned error must report the dangling registration rather than
    // hide it behind the bookmark-step cause.
    #[tokio::test]
    async fn create_worktree_rollback_surfaces_forget_failure() {
        use processkit::testing::{Reply, ScriptedRunner};
        use vcs_jj::Jj;
        use vcs_testkit::TempDir;

        let tmp = TempDir::new("r2-rollback-forget");
        let repo = tmp.path();
        let wt = repo.join("wt");

        let jj = Jj::with_runner(AddCreatesDir {
            dir: wt.clone(),
            inner: ScriptedRunner::new()
                .on(["jj", "workspace", "add"], Reply::ok(""))
                .on(
                    ["jj", "bookmark", "create"],
                    Reply::fail(1, "bookmark already exists\n"),
                )
                .on(
                    ["jj", "workspace", "forget"],
                    Reply::fail(1, "cannot forget workspace\n"),
                ),
        });

        let err = create_worktree(&jj, repo, &wt, "feature", "@")
            .await
            .expect_err("the bookmark-step failure must propagate");
        let msg = err.to_string();
        assert!(
            msg.contains("could not be forgotten") && msg.contains("feature"),
            "the swallowed forget failure must be reported: {msg}"
        );
        assert!(
            !wt.exists(),
            "the dir removal still ran (only the forget failed)"
        );
    }

    // A `ScriptedRunner` whose mocked `workspace add` drops a *file* where the
    // worktree directory should be, so the rollback's `remove_dir_all` fails
    // deterministically and cross-platform — a hermetic stand-in for a Windows-like
    // "the directory can't be removed" outcome (a locked/undeletable dir), letting a
    // test assert the removal failure is REPORTED rather than swallowed.
    struct AddCreatesFile {
        inner: processkit::testing::ScriptedRunner,
        path: std::path::PathBuf,
    }

    #[async_trait::async_trait]
    impl processkit::ProcessRunner for AddCreatesFile {
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
                if let Some(parent) = self.path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&self.path, b"not a dir");
            }
            self.inner.output_string(command).await
        }
    }

    // R2 (Windows-like removal failure): the rollback surfaces a dir-removal failure
    // instead of swallowing it. `workspace add` leaves a *file* at the worktree path,
    // so `remove_dir_all` errors; the returned error must report the leaked path.
    #[tokio::test]
    async fn create_worktree_rollback_surfaces_dir_removal_failure() {
        use processkit::testing::{Reply, ScriptedRunner};
        use vcs_jj::Jj;
        use vcs_testkit::TempDir;

        let tmp = TempDir::new("r2-rollback-rmdir");
        let repo = tmp.path();
        let wt = repo.join("wt");
        assert!(
            !wt.exists(),
            "must not pre-exist (so it counts as ours to remove)"
        );

        let jj = Jj::with_runner(AddCreatesFile {
            path: wt.clone(),
            inner: ScriptedRunner::new()
                .on(["jj", "workspace", "add"], Reply::ok(""))
                .on(
                    ["jj", "bookmark", "create"],
                    Reply::fail(1, "bookmark already exists\n"),
                )
                .on(["jj", "workspace", "forget"], Reply::ok("")),
        });

        let err = create_worktree(&jj, repo, &wt, "feature", "@")
            .await
            .expect_err("the bookmark-step failure must propagate");
        let msg = err.to_string();
        assert!(
            msg.contains("could not be removed") && msg.contains("wt"),
            "the swallowed dir-removal failure must be reported: {msg}"
        );
    }

    // T-038: the read-only snapshot must pass `--ignore-working-copy` on **every**
    // spawn (the `@` template query, the reachable-bookmark query, and — when the
    // change reads dirty — the change-count query), so an observer never snapshots
    // the jj working copy (records an operation / moves `@`). The row's middle
    // field is `0` (empty=false ⇒ dirty), which forces the third (status) spawn to
    // fire, so all three are exercised. Driven by a `RecordingRunner` so the argv
    // is asserted hermetically.
    #[tokio::test]
    async fn snapshot_readonly_ignores_working_copy_on_every_spawn() {
        use processkit::testing::{RecordingRunner, Reply};
        use vcs_jj::Jj;

        let rec = RecordingRunner::replying(Reply::ok("abc123\t0\t0\n"));
        let jj = Jj::with_runner(&rec);
        snapshot_readonly(&jj, Path::new("/repo"))
            .await
            .expect("read-only snapshot");

        let calls = rec.calls();
        assert!(
            calls.len() >= 3,
            "template + reachable bookmarks + change-count spawns, got {}",
            calls.len()
        );
        for c in &calls {
            assert!(
                c.args_str().iter().any(|a| a == "--ignore-working-copy"),
                "every read-only snapshot spawn must ignore the working copy: {:?}",
                c.args_str()
            );
        }
    }

    // The complement: the default `snapshot` snapshots the working copy (jj's
    // normal behaviour), so it must NOT carry the read-only flag on any spawn.
    #[tokio::test]
    async fn snapshot_default_does_not_ignore_working_copy() {
        use processkit::testing::{RecordingRunner, Reply};
        use vcs_jj::Jj;

        let rec = RecordingRunner::replying(Reply::ok("abc123\t0\t0\n"));
        let jj = Jj::with_runner(&rec);
        snapshot(&jj, Path::new("/repo"))
            .await
            .expect("default snapshot");

        for c in &rec.calls() {
            assert!(
                !c.args_str().iter().any(|a| a == "--ignore-working-copy"),
                "the default snapshot must let jj snapshot the working copy: {:?}",
                c.args_str()
            );
        }
    }

    // T-041 (tombstone, end-to-end): a locally-deleted bookmark still tracked on a
    // remote must not appear in `local_branches` or `branch_exists`. The scripted
    // `bookmark list` output is what jj renders for that state — a `present=0`
    // local row plus a `present=1` remote-tracking row — so only the live local
    // bookmark survives, and the deleted one never looks alive.
    #[tokio::test]
    async fn tombstone_bookmark_is_not_a_live_local_branch() {
        use processkit::testing::{Reply, ScriptedRunner};
        use vcs_jj::Jj;

        let jj = Jj::with_runner(ScriptedRunner::new().on(
            ["jj", "bookmark", "list"],
            Reply::ok(concat!(
                "1\t\t\"main\"\tabc123\n",         // live local
                "0\t\t\"gone\"\t\n",               // deleted local tombstone
                "1\torigin\t\"gone\"\tdeadbeef\n", // its remote-tracking row
            )),
        ));

        let names = local_branches(&jj, Path::new("/repo"))
            .await
            .expect("local_branches");
        assert_eq!(
            names,
            vec!["main".to_string()],
            "the tombstone must not appear as a local branch"
        );
        assert!(
            branch_exists(&jj, Path::new("/repo"), "main")
                .await
                .expect("branch_exists main")
        );
        assert!(
            !branch_exists(&jj, Path::new("/repo"), "gone")
                .await
                .expect("branch_exists gone"),
            "a deleted bookmark must not report as an existing branch"
        );
    }

    // T-041: `RepoSnapshot.head` and `WorktreeInfo.commit` must carry the SAME
    // full commit id for the same commit, so a consumer can compare them to tell
    // whether a worktree sits on the snapshotted commit. Driven hermetically: `@`
    // and the `default` workspace both point at one 40-hex oid, and the two facade
    // fields must come back equal and full-length — not one full, one truncated.
    #[tokio::test]
    async fn snapshot_head_and_worktree_commit_share_the_full_id() {
        use processkit::testing::{Reply, ScriptedRunner};
        use vcs_jj::Jj;

        const FULL: &str = "abcdef0123456789abcdef0123456789abcdef01";

        let jj = Jj::with_runner(
            ScriptedRunner::new()
                // snapshot spawn 1: `@` head/empty/conflict — empty=1 ⇒ clean, so
                // no change-count spawn follows.
                .on(
                    ["jj", "log", "-r", "@"],
                    Reply::ok(format!("{FULL}\t1\t0\n")),
                )
                // snapshot spawn 2: branch = nearest reachable bookmark.
                .on(
                    ["jj", "log", "-r", "heads(::@ & bookmarks())"],
                    Reply::ok(format!("\"main\"\t{FULL}\n")),
                )
                // list_worktrees: the default workspace points at the same commit.
                .on(
                    ["jj", "workspace", "list"],
                    Reply::ok(format!("\"default\"\t{FULL}\t\"main\"\n")),
                )
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
                ),
        );

        let snap = snapshot(&jj, Path::new("/repo")).await.expect("snapshot");
        let worktrees = list_worktrees(&jj, Path::new("/repo"))
            .await
            .expect("worktrees");

        let head = snap.head.expect("snapshot head present");
        assert_eq!(
            head.len(),
            40,
            "head must be the full oid, not a short prefix"
        );
        let wt_commit = worktrees[0].commit.as_deref().expect("worktree commit");
        assert_eq!(
            head, wt_commit,
            "snapshot head and worktree commit must be the same full id"
        );
    }

    // `local_branches_readonly` lists bookmarks read-only (no operation recorded).
    #[tokio::test]
    async fn local_branches_readonly_ignores_working_copy() {
        use processkit::testing::{RecordingRunner, Reply};
        use vcs_jj::Jj;

        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        local_branches_readonly(&jj, Path::new("/repo"))
            .await
            .expect("read-only branches");

        let calls = rec.calls();
        assert_eq!(calls.len(), 1, "a single `bookmark list` spawn");
        assert!(
            calls[0]
                .args_str()
                .iter()
                .any(|a| a == "--ignore-working-copy"),
            "read-only branch listing must ignore the working copy: {:?}",
            calls[0].args_str()
        );
    }
}
