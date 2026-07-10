# vcs-core — backend-agnostic facade guide

`vcs-core` lifts one layer that every downstream tool kept re-implementing:
*detect whether a directory is git or jj, then dispatch the operations both tools
share behind a single interface.* It returns backend-agnostic DTOs, so a caller
codes against "the repository" rather than against `git` or `jj` specifically.

It is deliberately a thin common surface — not a replacement for the underlying
clients. Rich, tool-specific operations (a full `merge`, jj's `op restore`,
range/revset-scoped queries) stay on [`vcs-git`](https://docs.rs/vcs-git/latest/vcs_git/guide/) / [`vcs-jj`](https://docs.rs/vcs-jj/latest/vcs_jj/guide/) and
are reachable through escape hatches on the handle. Reach for the facade when the
code must work on both backends; drop to the raw client the moment you need
power only one of them offers.

Examples use `vcs_core::Result<()>` and hidden `# ` setup lines. The `no_run`
ones compile against this crate's API (but don't spawn `git`/`jj`); a few
signature snippets are marked `ignore`.

```rust,no_run
use vcs_core::Repo;
# fn run() -> vcs_core::Result<()> {
let repo = Repo::discover(".")?;
println!("backend: {}", repo.kind().as_str()); // "git" / "jj"
# Ok(()) }
```

## Detection

```rust,ignore
pub fn discover(start: &Path) -> Option<Located>
```

`discover` walks up from `start` to the filesystem root, returning the first
repository it finds. A **valid** `.jj` **wins over `.git`** — colocated repos are
driven through jj, since that's the tool actually managing the working copy. Both
markers are **validated**, not merely present: a `.jj` must contain its `repo` store
(a directory in a main workspace / colocated repo, a file pointer in a secondary
workspace), and a `.git` may be a directory *or* a gitlink file (a linked worktree or
submodule) whose content starts with `gitdir:`. So a stray/empty `.jj` (a leftover
`mkdir .jj`) or a file merely named `.git` doesn't register as a repository or shadow
a real one higher up. Pure filesystem probing — no subprocess is ever spawned.

`start` is walked via `Path::parent`, so pass an **absolute** path to search
ancestors. A relative path like `"."` has no ancestor chain — only its own
directory is checked. (`Repo::discover` absolutises for you; `discover` does not.)

```rust,ignore
#[non_exhaustive]
pub struct Located {
    pub kind: BackendKind, // Git / Jj
    pub root: PathBuf,     // the directory holding .git/.jj — the worktree root
}
```

`BackendKind` is `Git` or `Jj`, with `as_str(self) -> &'static str` returning
`"git"` / `"jj"`.

```rust,ignore
# use std::path::Path;
# use vcs_core::{discover, BackendKind};
# fn run() {
if let Some(loc) = discover(Path::new("/abs/path/to/checkout/src")) {
    match loc.kind {
        BackendKind::Jj  => println!("jj at {}", loc.root.display()),
        BackendKind::Git => println!("git at {}", loc.root.display()),
    }
}
# }
```

## Opening a repo

```rust,ignore
impl Repo<JobRunner> {
    pub fn discover(dir: impl AsRef<Path>) -> Result<Self>;
    pub fn open(dir: impl AsRef<Path>) -> Result<Self>;
}
```

`Repo::discover` detects the repository at or above `dir` and opens a handle
**bound to `dir`**, using the real job-backed process runner. It errors with
`Error::NotARepository(dir)` when no `.git`/`.jj` is found from the start dir up
to the filesystem root, or with `Error::BareRepository(path)` when the walk
instead reaches a bare git repository (`git init --bare`) before finding a
`.jj`/`.git` marker.

`Repo::open` is the strict counterpart — mirroring the discover-vs-open split in
gitoxide (`gix::discover`/`gix::open`) and libgit2
(`git_repository_discover`/`git_repository_open`). It checks **only** `dir`
itself, with no ancestor walk: `dir` must directly hold the `.jj`/`.git` marker,
or it errors with `Error::NotARepository(dir)` even if a repository exists
somewhere above `dir`.

For tests or a pre-configured client, build a handle from an explicit client —
these are generic over the `ProcessRunner` so you can inject a fake:

```rust,ignore
pub fn from_git(root: impl Into<PathBuf>, cwd: impl Into<PathBuf>, client: Git<R>) -> Self;
pub fn from_jj (root: impl Into<PathBuf>, cwd: impl Into<PathBuf>, client: Jj<R>)  -> Self;
```

### Properties and re-anchoring

```rust,ignore
pub fn kind(&self) -> BackendKind;  // which backend drives this handle
pub fn root(&self) -> &Path;        // the repo root detected at open time
pub fn cwd(&self)  -> &Path;        // the directory operations run against
pub fn at(&self, dir: impl Into<PathBuf>) -> Self; // sibling handle bound elsewhere
```

`at` returns a sibling handle bound to `dir`, sharing this handle's client (the
backend is held behind an `Arc`) and root. It's cheap — no client rebuild, no
re-detection — so threading work across worktrees doesn't mean re-opening:

```rust,no_run
# async fn f(repo: vcs_core::Repo, wt: &std::path::Path) -> vcs_core::Result<()> {
let wt = repo.at(wt);                 // owns the re-anchored handle
let dirty = wt.has_uncommitted_changes().await?;
# Ok(()) }
```

## Escape hatches to the underlying client

The facade only covers what both tools share. For anything else — tool-specific
operations, range queries, jj transactions — drop to the typed client:

```rust,ignore
pub fn git(&self) -> Option<&Git<R>>;       // None when jj-backed
pub fn jj(&self)  -> Option<&Jj<R>>;        // None when git-backed
pub fn git_at(&self) -> Option<GitAt<'_, R>>; // bound to self.cwd(); None when jj-backed
pub fn jj_at(&self)  -> Option<JjAt<'_, R>>;  // bound to self.cwd(); None when git-backed
```

`git()`/`jj()` hand out a borrow of the raw client (whose methods still take a
`dir` argument). `git_at()`/`jj_at()` hand out the cwd-bound view (`GitAt` /
`JjAt`) whose methods omit `dir` — the dir-free counterpart. The bound view
borrows `self`, so to work in another worktree **bind the re-anchored handle
first** — the view can't outlive a temporary `at`:

```rust,no_run
# async fn f(repo: vcs_core::Repo, wt: &std::path::Path) -> vcs_core::Result<()> {
let wt = repo.at(wt);          // owns the re-anchored handle
let git = wt.git_at().unwrap();
git.fetch().await?;
# Ok(()) }
```

`vcs_core` re-exports `vcs_git` and `vcs_jj`, so a consumer depending only on
`vcs-core` still reaches the raw client types (`GitApi`, `JjApi`, `WorktreeAdd`,
`JjFileset`, …) without adding those crates as separate dependencies.

## Status & files

```rust,ignore
pub async fn current_branch(&self) -> Result<Option<String>>;
pub async fn trunk(&self)          -> Result<Option<String>>;
pub async fn local_branches(&self) -> Result<Vec<String>>;
pub async fn branch_exists(&self, name: &str) -> Result<bool>;
pub async fn conflicted_files(&self) -> Result<Vec<PathBuf>>;
pub async fn changed_files(&self)    -> Result<Vec<FileChange>>;
pub async fn diff_stat(&self)        -> Result<DiffStat>;
pub async fn snapshot(&self)         -> Result<RepoSnapshot>;
```

`current_branch` is the current branch (git) or, on jj, the nearest bookmark
reachable from `@` (`heads(::@ & bookmarks())`) — so it stays set across a
`jj describe`/`jj new`/`jj commit`, which leave the bookmark on the described
parent; `None` when detached / no bookmark on or above `@`.

`trunk` resolves in order: the backend's own notion (git's `origin/HEAD`, jj's
`trunk()` revset), then a fallback to a local `main`, then `master`; `None` when
none resolve.

`local_branches` lists local branch (git) / bookmark (jj) names. `branch_exists`
checks one by name.

`conflicted_files` returns paths with unresolved merge conflicts in the working
copy — **repo-relative, `/` separators** (git `diff --diff-filter=U` / jj
`resolve --list -r @`). Empty when there are none.

`changed_files` is the working-copy change set (git `status` / jj
`diff -r @ --summary`), as `Vec<FileChange>`. `diff_stat` is the aggregate
insertion/deletion counts.

> **Lossless paths.** `changed_files`/`conflicted_files` carry each path as a
> `PathBuf` (not a `String`): a filename whose bytes are not valid UTF-8 — legal on
> Unix — is preserved byte-for-byte, so it can be fed straight back into
> `commit_paths` (which also takes `PathBuf`) and address the *same* file. A
> `String::from_utf8_lossy` would substitute `U+FFFD` and silently retarget the
> path. Use `.display()` for human output; keep the `PathBuf` for a round-trip.

`snapshot` is the **batched** state query for a prompt/status-bar/TUI refresh —
branch, upstream, ahead/behind, HEAD, dirtiness, change count, and operation
state in a **small fixed** number of spawns rather than a call per field
([`RepoSnapshot`](#reposnapshot)). git issues one `status --porcelain=v2 --branch`
plus the cheap in-progress probe; jj issues a `log -r @` template (HEAD/empty/
conflict), a `reachable_bookmarks` query for `branch`, and a change count only
when dirty. Note the asymmetry: `tracking` (the upstream ref plus ahead/behind,
bundled into one [`UpstreamTracking`](#reposnapshot)) is always `None` on jj (no
git-style upstream tracking).

> **Backend nuance — untracked files.** `diff_stat` counts the git working tree
> against `HEAD` (`git diff`, which **excludes untracked files**), but on jj it
> counts the `@` change against its parent (which **includes** newly-added
> files). So a brand-new file shows in `changed_files` but *not* in `diff_stat`
> on git, whereas on jj it shows in both.

```rust,no_run
# async fn f(repo: vcs_core::Repo) -> vcs_core::Result<()> {
for c in repo.changed_files().await? {
    // `path`/`old_path` are `PathBuf` (lossless for a non-UTF-8 name on Unix);
    // use `.display()` for lossy display, or the bytes for an exact round-trip.
    match c.old_path {
        Some(from) => println!("rename {} -> {}", from.display(), c.path.display()),
        None       => println!("{:?} {}", c.kind, c.path.display()),
    }
}
let stat = repo.diff_stat().await?;
println!("{} files, +{} -{}", stat.files_changed, stat.insertions, stat.deletions);
# Ok(()) }
```

## Uncommitted state

```rust,ignore
pub async fn has_uncommitted_changes(&self) -> Result<bool>;
pub async fn has_tracked_changes(&self)     -> Result<bool>;
```

`has_uncommitted_changes` — whether the working copy has *any* uncommitted
change (git: a non-empty `status`; jj: a non-empty working-copy change `@`).

`has_tracked_changes` — whether *tracked* files have uncommitted changes. git
ignores untracked files here (`status --untracked-files=no`); jj auto-tracks new
files, so it has no untracked concept and this is identical to
`has_uncommitted_changes` on jj.

## Branch mutations

```rust,ignore
pub async fn delete_branch(&self, spec: BranchDelete) -> Result<()>; // BranchDelete::new(name)[.force()]
pub async fn rename_branch(&self, old: &str, new: &str)     -> Result<()>;
```

`delete_branch` deletes a local branch (git) / bookmark (jj). **`force` applies
to git only** (`branch -D` vs `-d`); jj has no force and ignores the flag.

`rename_branch` renames a local branch (git) / bookmark (jj).

## Commits & paths

```rust,ignore
pub async fn commit_paths(&self, paths: &[PathBuf], message: &str) -> Result<()>;
```

Commit exactly `paths` with `message` (git `commit --only`, jj
`commit <filesets>`). **Paths are repo-relative.**

## Remotes

```rust,ignore
pub async fn fetch(&self)                          -> Result<()>;
pub async fn fetch_from(&self, remote: &str)       -> Result<()>;
pub async fn fetch_branch(&self, branch: &str) -> Result<()>;
pub async fn push(&self, branch: &str)             -> Result<()>;
```

- `fetch` — from the default remote (git `fetch` / jj `git fetch`).
- `fetch_from` — from a *named* remote (git `fetch <remote>` / jj
  `git fetch --remote <remote>`).
- `fetch_branch` — a single branch/bookmark from `origin` into its
  remote-tracking ref (git `fetch_branch` / jj `git fetch -b`).
- `push` — an **existing** local branch/bookmark to `origin` (git
  `push -u origin <branch>` / jj `git push -b <branch>`). The backends honestly
  differ: git pushes the *ref* and records the upstream (`-u`, idempotent); jj
  pushes the *bookmark's state* — including a remote deletion if the bookmark
  was deleted locally. Renamed refspecs (`local:remote`) and non-`origin`
  remotes are git-only — use the escape hatch (`vcs_git::GitPush`).

Transient network failures are retried by the underlying client; for retrying a
higher-level flow, classify with `Error::is_transient_fetch_error`.

## Checkout / rebase

```rust,ignore
pub async fn checkout(&self, reference: &str) -> Result<()>;
pub async fn rebase(&self, onto: &str)        -> Result<()>;
```

`checkout` switches the working copy to `reference` — and this is where the two
tools genuinely differ, not just in verb (**git `checkout`, jj `edit`**) but in
*consequence*. ⚠ On **git**, your next commit *appends* on top of `reference`. On
**jj**, `edit` makes `reference`'s commit **itself** the working-copy change, so a
following `commit_paths` (or any edit) **rewrites that commit in place** — a silent
amend of a possibly-pushed commit, not a new commit on top. Backend-agnostic "start
fresh work on top of `main`" code must therefore not rely on `checkout` alone: for
git-like append-on-top on both backends, start a new child change explicitly (on jj,
`jj new <reference>` through the raw [`Repo::jj`] client — a first-class `new_child`
facade primitive is planned; on git, `checkout` already appends).

`rebase` rebases the current line onto `onto`, and the backends **diverge** on
non-linear layouts (a documented least-common-denominator): git (`rebase <onto>` =
`merge-base(HEAD,onto)..HEAD`) moves only `HEAD`'s ancestor line, while jj (`rebase
-d <onto>` = default `-b @` = `(onto..@)::`) also moves everything stacked on `@`
and any sibling off an *intermediate* commit. They agree on a linear `HEAD`/`@`; a
sibling sharing only the fork point is moved by neither. `onto` is a branch/bookmark
name or revision the backend understands.

## Merge probe & operation state

```rust,ignore
pub async fn try_merge(&self, source: &str)    -> Result<MergeProbe>;
pub async fn in_progress_state(&self)          -> Result<OperationState>;
pub async fn continue_in_progress(&self)       -> Result<OperationState>;
pub async fn abort_in_progress(&self)          -> Result<OperationState>;
```

`try_merge` probes whether merging `source` into the current work would conflict,
**without leaving any trace** — the probe is rolled back before returning,
whatever the outcome (git: `merge --no-commit --no-ff` then `merge --abort`; jj:
a merge change probed and rolled back through the concurrency-safe
`Jj::rollback_to`). It only *reports* what a real merge would do.

- git requires a clean-enough working tree: a dirty-tree refusal propagates as a
  plain error, **not** as `MergeProbe::Conflicts`.
- A failing *rollback* **propagates as an error** rather than returning a result
  that would misdescribe the on-disk state. On the jj backend this includes a
  rollback **refused** because a concurrent jj process advanced the operation log
  during the trial merge (reverting would clobber that work): it surfaces as
  `Error::Rollback` rather than a stale `MergeProbe::Clean`/`Conflicts`.

```rust,ignore
# use vcs_core::MergeProbe;
# async fn f(repo: vcs_core::Repo) -> vcs_core::Result<()> {
match repo.try_merge("feature").await? {
    MergeProbe::Clean            => println!("merges cleanly"),
    MergeProbe::Conflicts(paths) => println!("would conflict in {paths:?}"),
}
# Ok(()) }
```

The remaining three deal with operation state, and this is the sharpest
git-vs-jj asymmetry the facade has to paper over. git models an in-progress merge,
rebase, `am`, cherry-pick, revert, or bisect as *paused on-disk state* (`MERGE_HEAD`,
a `rebase-*` dir, `CHERRY_PICK_HEAD`, `REVERT_HEAD`, `BISECT_LOG`); jj has no
paused multi-step operations at all — it records a conflict directly on the
working-copy change.

`in_progress_state` reports whether the working copy is mid-operation. On git it
returns `Merge`/`Rebase`/`ApplyMailbox` (a `git am`)/`CherryPick`/`Revert`/`Bisect`
and **never `Conflict`** — a git conflict *is* that paused state, and the conflict
itself surfaces on the failed op (via `Error::is_merge_conflict`) or via
`continue_in_progress`. The sequencer states are kept distinct because each is
driven by its own git command: a cherry-pick/revert conflict writes
`CHERRY_PICK_HEAD`/`REVERT_HEAD` (never `MERGE_HEAD`), so dispatching `merge --abort`
on one would be wrong. On jj, which has no paused op, it reports `Conflict` directly.

`continue_in_progress` continues after conflict resolution (git: `commit --no-edit`
for a merge / `rebase --continue` / `cherry-pick --continue` / `revert --continue`;
jj: a **no-op** — resolving the files *is* the continuation). It returns the fresh
*post-call* state:
- `Conflict` when unresolved paths still block continuing (and **here git
  *does* report `Conflict`**, unlike `in_progress_state`), or when a continued
  rebase/cherry-pick/revert stops on the next commit's conflict.
- `Clear` when the operation finished.
- A `Bisect` has no continue step, so it returns `Error::Unsupported`
  (`is_unsupported()`) instead of a misleading success — end it with
  `abort_in_progress`, or mark commits with `git bisect good`/`bad` directly.

`abort_in_progress` aborts the in-progress operation, if any, dispatching the
state's own git command (`merge --abort` / `rebase --abort` / `am --abort` /
`cherry-pick --abort` / `revert --abort` / `bisect reset`; jj: a **no-op** —
nothing is ever paused; roll back explicitly via the jj client's `transaction` /
`op_restore`). It returns the fresh *post-call* state — `Clear` when nothing was,
or remains, in progress.

## Worktrees / workspaces

```rust,ignore
pub async fn list_worktrees(&self)  -> Result<Vec<WorktreeInfo>>;
pub async fn create_worktree(&self, spec: WorktreeCreate) -> Result<CreateOutcome>; // WorktreeCreate::new(path, branch).base(base)
pub async fn remove_worktree(&self, spec: WorktreeRemove) -> Result<()>; // WorktreeRemove::new(path)[.force()]
pub fn cleanup_worktree_blocking(&self, path: &Path) -> Result<()>;
```

`list_worktrees` lists attached worktrees (git) / workspaces (jj).

`create_worktree` creates a worktree/workspace at `path` on a **new** `branch`
based on `base`. It always reports `CreateOutcome::Plain` — a copy-on-write
strategy stays in the consumer. `branch` must not already exist. **The jj path is
two steps** (`workspace add`, then `bookmark create`) and is not atomic, but a
failed bookmark step **rolls back**: the workspace directory is removed only when
`workspace add` created it (a pre-existing directory the caller already had is
left intact), then the workspace is forgotten. The rollback no longer discards its
own residue — if it can't remove that directory or can't `forget` the workspace,
the call fails with a composite `Error::Io` that **names** what still needs
cleaning up (and is safe to re-run) instead of hiding it. A **clean** rollback
instead surfaces the original bookmark-step error unchanged (keeping its
`Error::Vcs` classification) — so a failed call never silently leaks a half-made
worktree.

`remove_worktree` removes the worktree/workspace at `path`. For jj this resolves
the workspace name by matching `path`, deletes the directory, then forgets it; a
directory that can't be deleted is **reported** — an `Error::Io` naming the jj
workspace still registered, so the retry is obvious, with the `forget` deferred to
that retry rather than orphaning the directory. Path resolution has two failure
shapes: a `path` that matches none of the **resolvable** workspaces — every
registered workspace resolved and none matched — returns `Error::WorktreeNotFound`
(`is_resource_not_found() == true`); but when some registered workspace can **not**
be resolved via `jj workspace root --name`, the path's absence can't be proven, so
a **distinct** diagnosable `Error::Io` (naming the unresolved workspaces;
`is_resource_not_found()` stays `false`) is returned instead of a misleading
`WorktreeNotFound`. (Contrast `cleanup_worktree_blocking` below, where a genuine
no-match is an `Ok` no-op.)
`force` mirrors git's `worktree remove`: with `force = false` a worktree that
still has **uncommitted changes** is refused rather than deleted (on jj the
changes are snapshotted into the op log first, so a refusal keeps them
recoverable); pass `force = true` to remove it anyway. The repository's **main**
workspace is always refused — deleting its directory would destroy the repo.

`cleanup_worktree_blocking` is the **synchronous** counterpart — for a context
that cannot `.await`, chiefly a `Drop` guard. It force-removes the worktree at
`path` (git: `worktree remove --force`; jj: resolve the workspace name by
`path`, delete the directory, then `workspace forget`). It stays short-lived and
**shells out directly with no job-containment**, unlike the async methods, but it
no longer swallows failures. A jj `path` that genuinely matches no workspace is
still an `Ok` no-op; a **probe** failure, though — the `workspace list`, or a
registered workspace that won't resolve — is now surfaced as `Err` rather than
folded into a silent no-op, and a `remove_dir_all` failure is likewise surfaced,
with the `forget` **skipped** on a failed removal so a surviving directory isn't
orphaned by a workspace record that outlived it. Like the async method it still
**refuses the main workspace** (a repo-wipe is never intended).

```rust,no_run
# use std::path::Path;
# use vcs_core::{CreateOutcome, WorktreeCreate, WorktreeRemove};
# async fn f(repo: vcs_core::Repo) -> vcs_core::Result<()> {
let out = repo
    .create_worktree(WorktreeCreate::new(Path::new("/tmp/feat"), "feature").base("main"))
    .await?;
assert_eq!(out, CreateOutcome::Plain);
repo.remove_worktree(WorktreeRemove::new("/tmp/feat")).await?; // add `.force()` to remove a dirty one
# Ok(()) }
```

## DTOs

All facade DTOs are `#[non_exhaustive]` — match with a `..` rest pattern, and don't
rely on field-init syntax from outside the crate. They normally come *from* the facade,
but the **return DTOs also have public builder constructors** (`RepoSnapshot::new()`,
`WorktreeInfo::new(path)`, `FileChange::new(path, kind)`, `UpstreamTracking::new(branch)`
— each with chained setters) so a custom `VcsRepo` backend or a test double can build one
to return despite the `#[non_exhaustive]`.

### `BackendKind`

`Git` | `Jj`. `as_str(self) -> &'static str` → `"git"` / `"jj"`.

### `ChangeKind`

`Added` (added or untracked) | `Modified` | `Deleted` | `Renamed` (see
`FileChange::old_path`).

### `FileChange`

```rust,ignore
#[non_exhaustive]
pub struct FileChange {
    pub path: PathBuf,             // the path (the *new* path for a rename); lossless, non-UTF-8-safe
    pub old_path: Option<PathBuf>, // original path for a rename (both backends); None for non-renames
    pub kind: ChangeKind,
}
```

### `DiffStat`

```rust,ignore
#[non_exhaustive]
pub struct DiffStat {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}
```

`Default` derives to all-zero.

### `WorktreeInfo`

```rust,ignore
#[non_exhaustive]
pub struct WorktreeInfo {
    pub path: PathBuf,             // working copy of the worktree
    pub branch: Option<String>,   // branch (git) / first bookmark (jj); None when detached/none
    pub commit: Option<String>,   // checked-out commit; None when unavailable (e.g. a bare git entry)
    pub is_bare: bool,            // a bare git worktree entry (always false for jj)
}
```

### `OperationState`

Unifies the backends' different models of "mid-operation":

| Variant    | When it arises |
| ---------- | -------------- |
| `Clear`    | No operation in progress and no conflict. |
| `Merge`    | A git merge is in progress (`MERGE_HEAD` present). git only. |
| `Rebase`   | A git rebase is in progress (a `rebase-merge` dir, or a `rebase-apply` dir *not* left by `git am`). git only. |
| `ApplyMailbox` | A git `am` (mailbox patch apply) is in progress (`rebase-apply/applying`). Distinct from `Rebase` because it aborts with `am --abort`. git only. |
| `CherryPick` | A git cherry-pick is in progress (`CHERRY_PICK_HEAD` present). Aborts with `cherry-pick --abort`, continues with `cherry-pick --continue`. A cherry-pick conflict writes `CHERRY_PICK_HEAD`, *not* `MERGE_HEAD`, so it's never read as a `Merge`. git only. |
| `Revert`   | A git revert is in progress (`REVERT_HEAD` present). Aborts with `revert --abort`, continues with `revert --continue`. git only. |
| `Bisect`   | A git bisect session is in progress (`BISECT_LOG` present). Aborts with `bisect reset`; it has *no* continue step (bisect advances by `git bisect good`/`bad`), so `continue_in_progress` returns `Error::Unsupported`. git only. |
| `Conflict` | The working copy has an unresolved conflict — chiefly jj, which records conflicts on the change rather than pausing an operation. On git this surfaces from `continue_in_progress`, not `in_progress_state`. |

### `RepoSnapshot`

The batched state from [`snapshot`](#status--files). `#[non_exhaustive]`.

```rust,ignore
#[non_exhaustive]
pub struct RepoSnapshot {
    pub head: Option<String>,      // working-copy commit's FULL oid (both backends); None on an unborn git repo; truncate for display
    pub branch: Option<String>,    // current branch (git) / nearest reachable bookmark (jj); None when detached/no bookmark on or above @
    pub tracking: Option<UpstreamTracking>, // upstream ref + ahead/behind, bundled; Some only with an upstream, ALWAYS None on jj
    pub dirty: bool,               // any uncommitted change (tracked or untracked)
    pub change_count: usize,       // number of changed paths
    pub conflicted: bool,          // an unresolved conflict is present
    pub operation: OperationState, // in-progress operation / conflict state
}

#[non_exhaustive]
pub struct UpstreamTracking {  // RepoSnapshot::tracking; Some only when an upstream is set
    pub branch: String,          // the upstream ref, e.g. "origin/main"
    pub ahead: Option<usize>,    // commits ahead; None if the upstream is set-but-gone (uncountable)
    pub behind: Option<usize>,   // commits behind; None if uncountable
}
```

### `MergeProbe`

`Clean` | `Conflicts(Vec<String>)` — the conflicting paths, repo-relative with
`/` separators, the same contract as `conflicted_files`. `is_clean(&self) ->
bool` returns whether the probe found no conflicts. The probe is always rolled
back before it returns; this type only *reports* what a real merge would do.

### `CreateOutcome`

`Plain` (the tool materialised the working copy itself) | `CowCloned` (a
copy-on-write clone populated the working copy). The facade **always** reports
`Plain`; the `CowCloned` variant exists so a consumer that layers a
copy-on-write strategy on top can reuse this type rather than inventing its own.

### `Error`

The facade error wraps `processkit::Error` and adds detection failures:
`NotARepository(PathBuf)`, `BareRepository(PathBuf)` (a bare `git init --bare`
repository — valid git, but no working tree for this facade to drive),
`WorktreeNotFound(PathBuf)`, `Io(io::Error)`, `Vcs(processkit::Error)`. Classifiers let a caller branch without matching on
internals: `is_merge_conflict()`, `is_nothing_to_commit()`,
`is_transient_fetch_error()`, `is_transient()` (a transient io/spawn hiccup — narrower
than the fetch classifier), and `is_not_found()` (the `git`/`jj` binary isn't
installed) — named to match the wrapper classifiers, one name per concept
workspace-wide. `processkit` is re-exported (`vcs_core::processkit`), so you can
match `Vcs(vcs_core::processkit::Error::…)` without a direct `processkit` dependency.
`Result<T>` is `std::result::Result<T, Error>`. See
[Process model & errors](https://docs.rs/vcs-core/latest/vcs_core/guide/process_model/).

### The `VcsRepo` trait

```rust,ignore
#[async_trait::async_trait]
pub trait VcsRepo: Send + Sync { /* … */ }
```

The backend-agnostic common surface of `Repo`, as an **object-safe** trait — so a
consumer can hold a `Box<dyn VcsRepo>` / `&dyn VcsRepo` and code against the
operations *without* naming the `ProcessRunner` generic or wrapping `Repo`
themselves. Every method mirrors the like-named inherent method on `Repo`; the
trait adds nothing but the abstraction boundary. Tool-specific operations stay
off it — reach those through the concrete `Repo` and its bound handles.

For hermetic tests, build a `Repo` over a fake runner with `from_git` /
`from_jj` rather than mocking this trait.

```rust,no_run
# async fn report(repo: &dyn vcs_core::VcsRepo) -> vcs_core::Result<()> {
println!("{} on {:?}", repo.kind().as_str(), repo.current_branch().await?);
# Ok(()) }
```

## When to use the facade vs the raw client

Use the **facade** (`Repo` / `VcsRepo`) for code that must run on both backends:
status, diffs, fetch/push, partial commits, worktree lifecycle, conflict probing.
You get one code path and backend-agnostic DTOs.

Drop to the **raw client** — `repo.git()` / `repo.jj()` (or the bound
`git_at()` / `jj_at()`) — the moment you need power only one tool offers: a full
`merge`, jj's `op restore` / transactions, range/revset-scoped queries (git's
`a..b` and jj's revsets aren't interchangeable, so they're deliberately *not* on
the common surface). The handle hands out a borrow; the consumer decides, per
call, whether to go through the facade or straight to the tool.

A quick router:

| You need… | Use |
|---|---|
| Backend-portable state/lifecycle (status, snapshot, branches, commit-paths, fetch/push, worktrees, conflict probe) | the facade method |
| An op with no faithful analogue on the other backend (full merge, rebase `--onto`, jj transactions / `op restore`, stash, tags, range diffs, revsets) | the raw client: `repo.git()?` / `repo.jj()?` |
| The same, dir-free at the handle's cwd | the bound view: `repo.git_at()?` / `repo.jj_at()?` |
| Options the facade's LCD drops (push refspecs/remotes via `GitPush`, amend via `CommitPaths`, `--no-ff` via `MergeCommit`…) | the raw client with its spec/builder |
| A flag/subcommand the wrapper doesn't model at all | the wrapper's raw `run(dir, args)` |

### The three call shapes

Every git/jj operation is reachable in three equivalent shapes — pick by how
much context the call site already holds:

1. **Dir-threading** (`Git::new().fetch(dir)`) — the wrapper client itself;
   right when one client serves many repos, or `dir` varies per call.
2. **At-view** (`repo.git_at()?.fetch()`) — dir-free, bound to the handle's
   cwd; right inside facade-holding code that needs one tool-specific call.
3. **Facade** (`repo.fetch()`) — backend-portable; right whenever the operation
   is on the common surface (prefer it there — you get jj support for free).

## See also

- [vcs-git guide](https://docs.rs/vcs-git/latest/vcs_git/guide/)
- [vcs-jj guide](https://docs.rs/vcs-jj/latest/vcs_jj/guide/)
- [Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/)
- [Process model & errors](https://docs.rs/vcs-core/latest/vcs_core/guide/process_model/)
- [crate docs](https://docs.rs/vcs-core)
