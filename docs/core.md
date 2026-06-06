# vcs-core — backend-agnostic facade guide

`vcs-core` lifts one layer that every downstream tool kept re-implementing:
*detect whether a directory is git or jj, then dispatch the operations both tools
share behind a single interface.* It returns backend-agnostic DTOs, so a caller
codes against "the repository" rather than against `git` or `jj` specifically.

It is deliberately a thin common surface — not a replacement for the underlying
clients. Rich, tool-specific operations (a full `merge`, jj's `op restore`,
range/revset-scoped queries) stay on [`vcs-git`](git.md) / [`vcs-jj`](jj.md) and
are reachable through escape hatches on the handle. Reach for the facade when the
code must work on both backends; drop to the raw client the moment you need
power only one of them offers.

Examples use `vcs_core::Result<()>` and hidden `# ` setup lines; they are
illustrative, not compiled.

```rust,no_run
use vcs_core::Repo;
# fn run() -> vcs_core::Result<()> {
let repo = Repo::open(".")?;
println!("backend: {}", repo.kind().as_str()); // "git" / "jj"
# Ok(()) }
```

## Detection

```rust
pub fn detect(start: &Path) -> Option<Located>
```

`detect` walks up from `start` to the filesystem root, returning the first
repository it finds. A `.jj` directory **wins over `.git`** — colocated repos are
driven through jj, since that's the tool actually managing the working copy.
`.git` may be a directory *or* a gitlink file (a linked worktree or submodule),
so the git probe is `.exists()`, not `.is_dir()`. Pure filesystem probing — no
subprocess is ever spawned.

`start` is walked via `Path::parent`, so pass an **absolute** path to search
ancestors. A relative path like `"."` has no ancestor chain — only its own
directory is checked. (`Repo::open` absolutises for you; `detect` does not.)

```rust
#[non_exhaustive]
pub struct Located {
    pub kind: BackendKind, // Git / Jj
    pub root: PathBuf,     // the directory holding .git/.jj — the worktree root
}
```

`BackendKind` is `Git` or `Jj`, with `as_str(self) -> &'static str` returning
`"git"` / `"jj"`.

```rust,no_run
# use std::path::Path;
# use vcs_core::{detect, BackendKind};
# fn run() {
if let Some(loc) = detect(Path::new("/abs/path/to/checkout/src")) {
    match loc.kind {
        BackendKind::Jj  => println!("jj at {}", loc.root.display()),
        BackendKind::Git => println!("git at {}", loc.root.display()),
    }
}
# }
```

## Opening a repo

```rust
impl Repo<JobRunner> {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self>;
}
```

`Repo::open` detects the repository at or above `dir` and opens a handle **bound
to `dir`**, using the real job-backed process runner. It errors with
`Error::NotARepository(dir)` when no `.git`/`.jj` is found from the start dir up
to the filesystem root.

For tests or a pre-configured client, build a handle from an explicit client —
these are generic over the `ProcessRunner` so you can inject a fake:

```rust
pub fn from_git(root: impl Into<PathBuf>, cwd: impl Into<PathBuf>, client: Git<R>) -> Self;
pub fn from_jj (root: impl Into<PathBuf>, cwd: impl Into<PathBuf>, client: Jj<R>)  -> Self;
```

### Properties and re-anchoring

```rust
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

```rust
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

```rust
pub async fn current_branch(&self) -> Result<Option<String>>;
pub async fn trunk(&self)          -> Result<Option<String>>;
pub async fn local_branches(&self) -> Result<Vec<String>>;
pub async fn branch_exists(&self, name: &str) -> Result<bool>;
pub async fn conflicted_files(&self) -> Result<Vec<String>>;
pub async fn changed_files(&self)    -> Result<Vec<FileChange>>;
pub async fn diff_stat(&self)        -> Result<DiffStat>;
```

`current_branch` is the current branch (git) or bookmark (jj); `None` when
detached / no bookmark on the working copy.

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

> **Backend nuance — untracked files.** `diff_stat` counts the git working tree
> against `HEAD` (`git diff`, which **excludes untracked files**), but on jj it
> counts the `@` change against its parent (which **includes** newly-added
> files). So a brand-new file shows in `changed_files` but *not* in `diff_stat`
> on git, whereas on jj it shows in both.

```rust,no_run
# async fn f(repo: vcs_core::Repo) -> vcs_core::Result<()> {
for c in repo.changed_files().await? {
    match c.old_path {
        Some(from) => println!("rename {from} -> {}", c.path),
        None       => println!("{:?} {}", c.kind, c.path),
    }
}
let stat = repo.diff_stat().await?;
println!("{} files, +{} -{}", stat.files_changed, stat.insertions, stat.deletions);
# Ok(()) }
```

## Uncommitted state

```rust
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

```rust
pub async fn delete_branch(&self, name: &str, force: bool) -> Result<()>;
pub async fn rename_branch(&self, old: &str, new: &str)     -> Result<()>;
```

`delete_branch` deletes a local branch (git) / bookmark (jj). **`force` applies
to git only** (`branch -D` vs `-d`); jj has no force and ignores the flag.

`rename_branch` renames a local branch (git) / bookmark (jj).

## Commits & paths

```rust
pub async fn commit_paths(&self, paths: &[String], message: &str) -> Result<()>;
```

Commit exactly `paths` with `message` (git `commit --only`, jj
`commit <filesets>`). **Paths are repo-relative.**

## Remotes

```rust
pub async fn fetch(&self)                          -> Result<()>;
pub async fn fetch_from(&self, remote: &str)       -> Result<()>;
pub async fn fetch_remote_branch(&self, branch: &str) -> Result<()>;
```

- `fetch` — from the default remote (git `fetch` / jj `git fetch`).
- `fetch_from` — from a *named* remote (git `fetch <remote>` / jj
  `git fetch --remote <remote>`).
- `fetch_remote_branch` — a single branch/bookmark from `origin` into its
  remote-tracking ref (git `fetch_remote_branch` / jj `git fetch -b`).

Transient network failures are retried by the underlying client; for retrying a
higher-level flow, classify with `Error::is_transient_fetch`.

## Checkout / rebase

```rust
pub async fn checkout(&self, reference: &str) -> Result<()>;
pub async fn rebase(&self, onto: &str)        -> Result<()>;
```

`checkout` switches the working copy to `reference` — and this is where the two
tools genuinely differ in verb: **git `checkout`, jj `edit`.** The facade
dispatches to whichever the backend uses.

`rebase` rebases the current work onto `onto` (git `rebase` / jj `rebase -d`).
`onto` is a branch/bookmark name or revision the backend understands.

## Merge probe & operation state

```rust
pub async fn try_merge(&self, source: &str)    -> Result<MergeProbe>;
pub async fn in_progress_state(&self)          -> Result<OperationState>;
pub async fn continue_in_progress(&self)       -> Result<OperationState>;
pub async fn abort_in_progress(&self)          -> Result<OperationState>;
```

`try_merge` probes whether merging `source` into the current work would conflict,
**without leaving any trace** — the probe is rolled back before returning,
whatever the outcome (git: `merge --no-commit --no-ff` then `merge --abort`; jj:
a merge change probed and undone via `op restore`). It only *reports* what a real
merge would do.

- git requires a clean-enough working tree: a dirty-tree refusal propagates as a
  plain error, **not** as `MergeProbe::Conflicts`.
- A failing *rollback* **propagates as an error** rather than returning a result
  that would misdescribe the on-disk state.

```rust,no_run
# use vcs_core::MergeProbe;
# async fn f(repo: vcs_core::Repo) -> vcs_core::Result<()> {
match repo.try_merge("feature").await? {
    MergeProbe::Clean            => println!("merges cleanly"),
    MergeProbe::Conflicts(paths) => println!("would conflict in {paths:?}"),
}
# Ok(()) }
```

The remaining three deal with operation state, and this is the sharpest
git-vs-jj asymmetry the facade has to paper over. git models an in-progress merge
or rebase as *paused on-disk state* (`MERGE_HEAD`, a `rebase-*` dir); jj has no
paused multi-step operations at all — it records a conflict directly on the
working-copy change.

`in_progress_state` reports whether the working copy is mid-operation. On git it
returns `Merge`/`Rebase` and **never `Conflict`** — a git conflict *is* that
paused state, and the conflict itself surfaces on the failed op (via
`Error::is_conflict`) or via `continue_in_progress`. On jj, which has no paused
op, it reports `Conflict` directly.

`continue_in_progress` continues after conflict resolution (git:
`commit --no-edit` for a merge / `rebase --continue`; jj: a **no-op** —
resolving the files *is* the continuation). It returns the fresh *post-call*
state:
- `Conflict` when unresolved paths still block continuing (and **here git
  *does* report `Conflict`**, unlike `in_progress_state`), or when a continued
  rebase stops on the next patch's conflict.
- `Clear` when the operation finished.

`abort_in_progress` aborts the in-progress operation, if any (git:
`merge --abort` / `rebase --abort`; jj: a **no-op** — nothing is ever paused;
roll back explicitly via the jj client's `transaction` / `op_restore`). It
returns the fresh *post-call* state — `Clear` when nothing was, or remains, in
progress.

## Worktrees / workspaces

```rust
pub async fn list_worktrees(&self)  -> Result<Vec<WorktreeInfo>>;
pub async fn create_worktree(&self, path: &Path, branch: &str, base: &str) -> Result<CreateOutcome>;
pub async fn remove_worktree(&self, path: &Path, force: bool) -> Result<()>;
pub fn cleanup_worktree_blocking(&self, path: &Path) -> Result<()>;
```

`list_worktrees` lists attached worktrees (git) / workspaces (jj).

`create_worktree` creates a worktree/workspace at `path` on a **new** `branch`
based on `base`. It always reports `CreateOutcome::Plain` — a copy-on-write
strategy stays in the consumer. `branch` must not already exist. **The jj path is
not atomic**: it's two steps (`workspace add`, then `bookmark create`), and if
the bookmark step fails the freshly-added workspace is left in place for the
caller to clean up. A consumer needing resume-existing or rollback semantics
should drive the underlying client via `jj()` / `git()`.

`remove_worktree` removes the worktree/workspace at `path`. For jj this resolves
the workspace name by matching `path`, deletes the directory, then forgets it.

`cleanup_worktree_blocking` is the **synchronous** counterpart — for a context
that cannot `.await`, chiefly a `Drop` guard. It force-removes the worktree at
`path` (git: `worktree remove --force`; jj: resolve the workspace name by
`path`, delete the directory, then `workspace forget`). It is best-effort and
short-lived: it **shells out directly with no job-containment**, unlike the async
methods. A jj `path` that matches no workspace is a no-op (`Ok`).

```rust,no_run
# use std::path::Path;
# use vcs_core::CreateOutcome;
# async fn f(repo: vcs_core::Repo) -> vcs_core::Result<()> {
let out = repo.create_worktree(Path::new("/tmp/feat"), "feature", "main").await?;
assert_eq!(out, CreateOutcome::Plain);
repo.remove_worktree(Path::new("/tmp/feat"), false).await?;
# Ok(()) }
```

## DTOs

All facade DTOs are `#[non_exhaustive]` — construct via the facade, match with a
`..` rest pattern, and don't rely on field-init syntax from outside the crate.

### `BackendKind`

`Git` | `Jj`. `as_str(self) -> &'static str` → `"git"` / `"jj"`.

### `ChangeKind`

`Added` (added or untracked) | `Modified` | `Deleted` | `Renamed` (see
`FileChange::old_path`).

### `FileChange`

```rust
#[non_exhaustive]
pub struct FileChange {
    pub path: String,             // the path (the *new* path for a rename)
    pub old_path: Option<String>, // original path for a rename; jj never supplies it
    pub kind: ChangeKind,
}
```

### `DiffStat`

```rust
#[non_exhaustive]
pub struct DiffStat {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}
```

`Default` derives to all-zero.

### `WorktreeInfo`

```rust
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
| `Rebase`   | A git rebase is in progress (a `rebase-merge`/`rebase-apply` dir present). git only. |
| `Conflict` | The working copy has an unresolved conflict — chiefly jj, which records conflicts on the change rather than pausing an operation. On git this surfaces from `continue_in_progress`, not `in_progress_state`. |

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
`NotARepository(PathBuf)`, `WorktreeNotFound(PathBuf)`, `Io(io::Error)`,
`Vcs(processkit::Error)`. Classifiers let a caller branch without matching on
internals: `is_conflict()`, `is_nothing_to_commit()`, `is_transient_fetch()`.
`Result<T>` is `std::result::Result<T, Error>`. See
[Process model & errors](process-model.md).

### The `VcsRepo` trait

```rust
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
status, diffs, fetch, partial commits, worktree lifecycle, conflict probing. You
get one code path and backend-agnostic DTOs.

Drop to the **raw client** — `repo.git()` / `repo.jj()` (or the bound
`git_at()` / `jj_at()`) — the moment you need power only one tool offers: a full
`merge`, jj's `op restore` / transactions, range/revset-scoped queries (git's
`a..b` and jj's revsets aren't interchangeable, so they're deliberately *not* on
the common surface). The handle hands out a borrow; the consumer decides, per
call, whether to go through the facade or straight to the tool.

## See also

- [vcs-git guide](git.md)
- [vcs-jj guide](jj.md)
- [Testing & mocking](testing.md)
- [Process model & errors](process-model.md)
- [crate README](../crates/core/README.md)
