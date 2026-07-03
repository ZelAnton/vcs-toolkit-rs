# vcs-git — Git CLI guide

**What you can do:** status & branches, stage/commit/checkout, diff & log,
merge/rebase/reset, worktrees, tags, blame, clone, config, cherry-pick/revert,
parse & resolve conflict markers, and a hardened (hooks-off) mode for untrusted
repos. This guide is the full reference — every command by theme, with examples.

Typed, repo-scoped, **async** commands over the `git` binary, behind a mockable
interface. Every method runs `git` inside an OS job (via [`processkit`]) so a
subprocess is never orphaned, returns the structured `processkit::Error`, and
honours an optional timeout. Consumers code against the [`GitApi`] trait and swap
in a fake in tests.

Caller-supplied names, revisions, ranges, remotes, and URLs that land in a bare
positional argv slot are guarded automatically — a value that is empty or begins
with `-` is rejected with an `Error::Spawn` *before* anything spawns, so it can't
be smuggled in as a flag.

[`processkit`]: https://crates.io/crates/processkit

## Construction & configuration

```rust,ignore
# use std::time::Duration;
use vcs_git::Git;

let git = Git::new();                                         // real, job-backed runner
let git = Git::new().default_timeout(Duration::from_secs(30)); // every cmd → Error::Timeout past 30s
```

- `Git::new()` — the production client over the real job-backed runner.
- `Git::with_runner(runner)` — inject a fake `ProcessRunner` (e.g.
  `processkit::testing::ScriptedRunner`) for hermetic tests; see [Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/).
- `default_timeout(Duration)` — builder; arms a per-command timeout.

`new`, `with_runner`, and `default_timeout` all come from the
`processkit::cli_client!` macro that defines `Git`.

### Hardening (`Git::hardened()`)

Running `git` inside an untrusted checkout executes that repository's hooks and
honours its config — arbitrary code execution by default. `Git::hardened()`
(equivalently `Git::new().harden()`) applies a containment profile to **every**
command the client runs:

- **Disables hooks** — `core.hooksPath` is pinned to `/dev/null` via git's
  env-based config (`GIT_CONFIG_COUNT`/`KEY_n`/`VALUE_n`, which overrides even the
  *repo-local* `.git/config`) — and `core.fsmonitor` is forced `false` (a
  config-driven daemon launch).
- **Neutralizes `core.sshCommand`** — pinned empty (the config-key twin of the
  scrubbed `GIT_SSH_COMMAND`), so a repo-local override can't run an arbitrary
  program for the SSH transport. The default `ssh` (ambient `~/.ssh/config`/agent)
  still works.
- **Scrubs inherited `GIT_*` redirectors** so a poisoned parent environment
  can't point commands at another repo: `GIT_DIR`, `GIT_WORK_TREE`,
  `GIT_INDEX_FILE`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`,
  `GIT_NAMESPACE`, `GIT_CEILING_DIRECTORIES`, `GIT_CONFIG_PARAMETERS`,
  `GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM`.
- **Scrubs inherited command hooks** that make git spawn an arbitrary program
  from the environment: `GIT_SSH_COMMAND`/`GIT_SSH`, `GIT_ASKPASS`,
  `GIT_EXTERNAL_DIFF`, `GIT_PAGER`, `GIT_EDITOR`/`GIT_SEQUENCE_EDITOR`. The
  opt-in `with_credentials` auth seam still works (it injects a
  `credential.helper` / token env, not these variables).
- **Skips system config** (`GIT_CONFIG_NOSYSTEM=1`) and keeps terminal prompts
  off everywhere (`GIT_TERMINAL_PROMPT=0`).

**Residual repo-local-config vectors (not neutralized).** A few repo-local
`.git/config` / `.gitattributes` keys still run an arbitrary program and are *not*
pinned: `filter.<drv>.clean`/`smudge` (run on any working-tree materialization —
checkout, `stash pop`, `worktree add`) and `diff.<drv>.textconv`/`diff.external`
(run when a diff is produced; `diff_text` defends with `--no-ext-diff`, other diff/
blame reads do not). For a **fully untrusted** repo, don't materialize its working
tree or run diffs through a hardened client without an OS-level sandbox — `harden()`
is hardening, not a sandbox.

It does **not** sandbox the git binary or vet the repo's *content*. `harden()` is
chainable on any runner — `Git::with_runner(rec).harden()` works in tests — but
`Git::hardened()` is the shorthand for the common case. See
[Security & hardening](https://docs.rs/vcs-git/latest/vcs_git/guide/security/).

```rust,ignore
use vcs_git::Git;

let git = Git::hardened();   // drive a repo you didn't create — hooks/config neutered
```

### The cwd-bound view (`GitAt`)

Most `GitApi` methods take a leading `dir: &Path`. When you drive one directory
repeatedly, bind it once with `git.at(&path)` — the returned `GitAt` drops that
argument:

```rust,ignore
# use std::path::Path;
# use vcs_git::Git;
# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
let git = Git::new();
let at = git.at(repo);            // GitAt — Copy, borrows the client + path
let branch = at.current_branch().await?;   // == git.current_branch(repo)
at.commit("feat: thing").await?;           // == git.commit(repo, "…")
# Ok(()) }
```

`GitAt` is `Copy` for every runner (it holds only two references). The dir-taking
`GitApi` methods stay on `Git` so one client can drive many directories — e.g.
linked worktrees. Through the facade, `vcs_core::Repo::git_at` yields the same
handle.

### Inherent `run_args` / `run_raw_args`

The object-safe `GitApi` trait can't take `&[&str]`, so two inherent helpers do —
no `Vec<String>` allocation:

```rust,ignore
# use vcs_git::Git;
# async fn demo(git: &Git) -> Result<(), processkit::Error> {
let out = git.run_args(&["status", "-s"]).await?;   // String, errors on non-zero exit
let res = git.run_raw_args(&["rev-parse", "HEAD"]).await?; // ProcessResult<String>, never errors on exit
# let _ = (out, res); Ok(()) }
```

## Status & staging

Working-tree inspection and the index.

```rust,ignore
async fn status(&self, dir: &Path) -> Result<Vec<StatusEntry>>;
async fn status_text(&self, dir: &Path) -> Result<String>;
async fn status_tracked(&self, dir: &Path) -> Result<Vec<StatusEntry>>;
async fn branch_status(&self, dir: &Path) -> Result<BranchStatus>;
async fn add(&self, dir: &Path, paths: &[PathBuf]) -> Result<()>;
async fn staged_is_empty(&self, dir: &Path) -> Result<bool>;
async fn conflicted_files(&self, dir: &Path) -> Result<Vec<String>>;
```

- **`status`** — `git status --porcelain=v1 -z`, parsed. Renames carry both paths.
- **`status_text`** — the raw porcelain text (`--porcelain=v1`), unparsed.
- **`status_tracked`** — `status` ignoring untracked files (`--untracked-files=no`);
  "is the *tracked* tree dirty", staged or not.
- **`branch_status`** — a combined branch + working-tree snapshot in **one** spawn
  (`status --porcelain=v2 --branch -z`): HEAD, branch, upstream, ahead/behind, and
  change counts ([`BranchStatus`](#branchstatus)) — the cheap primitive behind the
  facade's [`Repo::snapshot`](https://docs.rs/vcs-core/latest/vcs_core/guide/). Use it for a prompt/status-bar
  line without N round-trips.
- **`add`** — `git add -- <paths>` (the `--` keeps a path from being read as a flag).
- **`staged_is_empty`** — `git diff --cached --quiet`, exit-code mapped: `true` =
  nothing staged.
- **`conflicted_files`** — `git diff --name-only --diff-filter=U -z`; repo-relative
  paths with `/` separators, empty when there are none.

```rust,ignore
# use std::path::{Path, PathBuf};
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
git.add(repo, &[PathBuf::from("src/lib.rs")]).await?;       // `git add -- src/lib.rs`

for entry in git.status(repo).await? {                       // Vec<StatusEntry>
    match entry.old_path {
        Some(from) => println!("rename {from} -> {}", entry.path),
        None => println!("{} {}", entry.code, entry.path),
    }
}

if !git.staged_is_empty(repo).await? {                       // bool
    println!("index has staged changes");
}
for path in git.conflicted_files(repo).await? {             // Vec<String>
    println!("conflict: {path}");
}
# Ok(()) }
```

## Commits & log

```rust,ignore
async fn log(&self, dir: &Path, revspec: &str, max: usize) -> Result<Vec<Commit>>;
async fn commit(&self, dir: &Path, message: &str) -> Result<()>;
async fn commit_paths(&self, dir: &Path, spec: CommitPaths) -> Result<()>;
async fn last_commit_message(&self, dir: &Path) -> Result<String>;
async fn rev_list_count(&self, dir: &Path, range: &str) -> Result<usize>;
```

- **`log`** — up to `max` commits reachable from `revspec`, newest first. Pass
  `"HEAD"` for the current branch, or a range like `main..HEAD`. One signature
  mirrors `JjApi::log`'s revset argument.
- **`commit`** — `git commit -m <message>` of the staged index.
- **`commit_paths`** — commit exactly the spec's paths' working-tree content,
  ignoring the index (`commit [--amend] -m <message> --only -- <paths>`); built
  through [`CommitPaths`](#commitpaths).
- **`last_commit_message`** — the full last message (`log -1 --format=%B`), e.g. to
  pre-fill an amend.
- **`rev_list_count`** — how many commits a `range` spans (`rev-list --count
  <range>`), e.g. how far ahead of the upstream you are — cheaper than fetching
  and counting `log`.

```rust,ignore
# use std::path::Path;
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
git.commit(repo, "feat: tidy lib").await?;
for c in git.log(repo, "HEAD", 5).await? {                  // Vec<Commit>, newest first
    println!("{} {} — {} <{}>", c.short_hash, c.subject, c.author, c.date);
}
let ahead = git.log(repo, "origin/main..HEAD", 50).await?;  // Vec<Commit>
let n = git.rev_list_count(repo, "origin/main..HEAD").await?;    // usize — # commits ahead
let _ = (ahead, n);
# Ok(()) }
```

A `commit` that finds nothing to record fails; classify it with
[`is_nothing_to_commit`](#error-classification) rather than treating it as a real
error.

## Branches

```rust,ignore
async fn branches(&self, dir: &Path) -> Result<Vec<Branch>>;
async fn create_branch(&self, dir: &Path, name: &str) -> Result<()>;
async fn branch_exists(&self, dir: &Path, name: &str) -> Result<bool>;
async fn delete_branch(&self, dir: &Path, name: &str, force: bool) -> Result<()>;
async fn rename_branch(&self, dir: &Path, old: &str, new: &str) -> Result<()>;
async fn is_merged(&self, dir: &Path, spec: MergeCheck) -> Result<bool>; // MergeCheck::branch(b).into_base(base)
async fn set_upstream(&self, dir: &Path, branch: &str, upstream: &str) -> Result<()>;
async fn current_branch(&self, dir: &Path) -> Result<Option<String>>;
```

- **`branches`** — local branches (`git branch`), current one flagged.
- **`create_branch`** — `git branch <name>`, without switching to it.
- **`branch_exists`** — `show-ref --verify --quiet refs/heads/<name>`, exit-code mapped.
- **`delete_branch`** — `branch -d`, or `-D` when `force`.
- **`rename_branch`** — `branch -m <old> <new>`.
- **`is_merged`** — whether the [`MergeCheck`]'s `branch` is fully merged into its
  `base` (`branch --merged <base>`). Built as
  `MergeCheck::branch("feature").into_base("main")` — naming the two same-typed refs
  across two steps so a swap (which would *invert* the answer) can't compile silently.
- **`set_upstream`** — `branch --set-upstream-to=<upstream> <branch>`.
- **`current_branch`** — `symbolic-ref --quiet --short HEAD` → `Some("main")` for a
  normal **or unborn** branch, `None` on a detached HEAD. (Mirrors jj's `Option`
  bookmark shape.)

```rust,ignore
# use std::path::Path;
# use vcs_git::{Git, GitApi, MergeCheck};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
if !git.branch_exists(repo, "feature").await? {            // bool
    git.create_branch(repo, "feature").await?;
}
git.set_upstream(repo, "feature", "origin/feature").await?;
if git.is_merged(repo, MergeCheck::branch("feature").into_base("main")).await? { // bool
    git.delete_branch(repo, "feature", false).await?;      // `branch -d feature`
}
for b in git.branches(repo).await? {                       // Vec<Branch>
    println!("{}{}", if b.current { "* " } else { "  " }, b.name);
}
# Ok(()) }
```

## Revisions

```rust,ignore
async fn rev_parse(&self, dir: &Path, rev: &str) -> Result<String>;
async fn rev_parse_short(&self, dir: &Path, rev: &str) -> Result<String>;
async fn resolve_commit(&self, dir: &Path, rev: &str) -> Result<String>;
async fn is_unborn(&self, dir: &Path) -> Result<bool>;
async fn checkout(&self, dir: &Path, reference: &str) -> Result<()>;
async fn checkout_detach(&self, dir: &Path, commit: &str) -> Result<()>;
```

- **`rev_parse`** — resolve a revision to its full hash (`rev-parse <rev>`).
- **`rev_parse_short`** — the abbreviated hash (`rev-parse --short <rev>`), e.g. to
  label a detached HEAD.
- **`resolve_commit`** — resolve to a commit hash, peeling annotated tags
  (`rev-parse --verify <rev>^{commit}`).
- **`is_unborn`** — whether `HEAD` is unborn — a fresh repo with no commits
  (`rev-parse --verify -q HEAD`, exit-code mapped).
- **`checkout`** — switch to a branch or revision (`git checkout <reference>`).
- **`checkout_detach`** — check out a commit as a detached HEAD (`checkout --detach
  <commit>`).

```rust,ignore
# use std::path::Path;
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
if git.is_unborn(repo).await? {                            // bool
    println!("no commits yet");
}
let hash = git.rev_parse(repo, "HEAD").await?;             // String — full 40-hex sha
let short = git.rev_parse_short(repo, "HEAD").await?;      // String — abbreviated
let _ = (hash, short);
git.checkout(repo, "main").await?;
# Ok(()) }
```

To carry uncommitted changes across a switch, see the composed inherent helper
[`switch_with_stash`](#composed-inherent-helpers).

## Worktrees

```rust,ignore
async fn worktree_list(&self, dir: &Path) -> Result<Vec<Worktree>>;
async fn worktree_add(&self, dir: &Path, spec: WorktreeAdd) -> Result<()>;
async fn worktree_remove(&self, dir: &Path, path: &Path, force: bool) -> Result<()>;
async fn worktree_move(&self, dir: &Path, from: &Path, to: &Path) -> Result<()>;
async fn worktree_prune(&self, dir: &Path) -> Result<()>;
```

- **`worktree_list`** — `worktree list --porcelain`, parsed into `Vec<Worktree>`.
- **`worktree_add`** — `worktree add [-b <branch>] [--no-checkout] <path> [<commitish>]`;
  built through [`WorktreeAdd`](#worktreeadd).
- **`worktree_remove`** — `worktree remove [--force] <path>`.
- **`worktree_move`** — `worktree move <from> <to>`.
- **`worktree_prune`** — `worktree prune`, dropping stale admin entries.

```rust,ignore
# use std::path::Path;
# use vcs_git::{Git, GitApi, WorktreeAdd};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
git.worktree_add(repo, WorktreeAdd::create_branch("/tmp/feature", "feature", "HEAD"))
    .await?;                                                 // `worktree add -b feature /tmp/feature HEAD`

for wt in git.worktree_list(repo).await? {                  // Vec<Worktree>
    println!("{} -> {:?}", wt.path.display(), wt.branch);
}

git.worktree_remove(repo, Path::new("/tmp/feature"), false).await?;
# Ok(()) }
```

For a synchronous best-effort removal in a `Drop` guard, see
[`blocking::worktree_remove`](#blocking-helpers).

## Diff

```rust,ignore
async fn diff(&self, dir: &Path, spec: DiffSpec) -> Result<Vec<FileDiff>>;
async fn diff_text(&self, dir: &Path, spec: DiffSpec) -> Result<String>;
async fn diff_is_empty(&self, dir: &Path) -> Result<bool>;
async fn diff_range_is_empty(&self, dir: &Path, range: &str) -> Result<bool>;
async fn diff_stat(&self, dir: &Path, range: &str) -> Result<DiffStat>;
```

- **`diff`** — parsed per-file unified diff for `spec`, layered on `diff_text`.
- **`diff_text`** — raw git-format unified diff for `spec` (`diff <spec> --no-color
  --no-ext-diff -M`) — stable machine output. On an unborn repo,
  `DiffSpec::WorkingTree` diffs against the empty tree rather than failing.
- **`diff_is_empty`** — `git diff --quiet`, exit-code mapped: are there unstaged
  modifications to **tracked** files? Untracked files are not counted — not a full
  "is the working tree clean?" check; use `status` for that.
- **`diff_range_is_empty`** — `diff --quiet <range>`, exit-code mapped.
- **`diff_stat`** — aggregate `DiffStat` for a range (`diff --shortstat <range>`).

[`DiffSpec`](#diffspec) selects what is compared: `WorkingTree` (vs HEAD) or
`Rev(String)` (a revision or range).

```rust,ignore
# use std::path::Path;
# use vcs_git::{Git, GitApi, DiffSpec};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
if !git.diff_is_empty(repo).await? {
    println!("working tree has unstaged tracked changes");
}
for file in git.diff(repo, DiffSpec::WorkingTree).await? {  // Vec<FileDiff>
    println!("{:?} {}", file.change, file.path);
}
let raw = git.diff_text(repo, DiffSpec::Rev("main..HEAD".into())).await?; // String
let stat = git.diff_stat(repo, "main..HEAD").await?;        // DiffStat
println!("{} files, +{} -{}", stat.files_changed, stat.insertions, stat.deletions);
let _ = raw;
# Ok(()) }
```

## Blame

```rust,ignore
async fn blame(&self, dir: &Path, path: &str, rev: Option<String>) -> Result<Vec<BlameLine>>;
```

Per-line authorship of `path` (`blame --line-porcelain [<rev>] -- <path>`); `None`
blames the working tree's HEAD.

```rust,ignore
# use std::path::Path;
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
for line in git.blame(repo, "src/lib.rs", None).await? {    // Vec<BlameLine>
    println!("{} {} {}", &line.commit[..8], line.author, line.content);
}
# Ok(()) }
```

## Remotes & upstream

```rust,ignore
async fn remote_url(&self, dir: &Path, remote: &str) -> Result<String>;
async fn remote_add(&self, dir: &Path, name: &str, url: &str) -> Result<()>;
async fn remote_set_url(&self, dir: &Path, name: &str, url: &str) -> Result<()>;
async fn remote_branches(&self, dir: &Path, remote: &str) -> Result<Vec<String>>;
async fn remote_branch_exists(&self, dir: &Path, name: &str) -> Result<bool>;
async fn remote_head_branch(&self, dir: &Path) -> Result<Option<String>>;
async fn upstream(&self, dir: &Path) -> Result<Option<String>>;
```

- **`remote_url`** — a remote's URL (`remote get-url <remote>`).
- **`remote_add`** / **`remote_set_url`** — `remote add` / `remote set-url`.
- **`remote_branches`** — branch names on `remote`, without fetching (`ls-remote
  --heads <remote>`), with `GIT_TERMINAL_PROMPT=0`.
- **`remote_branch_exists`** — whether `origin` has `name` without fetching, querying
  the fully-qualified ref so `foo` can't tail-match `bar/foo`. Runs prompt-off with
  a 10s timeout; an unreachable remote reads as `false`, not an error.
- **`remote_head_branch`** — `origin`'s default branch (short name) from
  `symbolic-ref refs/remotes/origin/HEAD`; `None` when unset.
- **`upstream`** — the current branch's upstream, e.g. `Some("origin/main")`; `None`
  when unset.

```rust,ignore
# use std::path::Path;
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
if let Some(up) = git.upstream(repo).await? {              // Option<String>
    println!("tracking {up}");
}
if let Some(default) = git.remote_head_branch(repo).await? { // Option<String>
    println!("origin default: {default}");
}
let exists = git.remote_branch_exists(repo, "main").await?;  // bool — best-effort
let _ = exists;
# Ok(()) }
```

## Fetch / push / merge

```rust,ignore
async fn fetch(&self, dir: &Path) -> Result<()>;
async fn fetch_from(&self, dir: &Path, remote: &str) -> Result<()>;
async fn fetch_branch(&self, dir: &Path, branch: &str) -> Result<()>;
async fn push(&self, dir: &Path, spec: GitPush) -> Result<()>;
async fn merge_squash(&self, dir: &Path, branch: &str) -> Result<()>;
async fn merge_commit(&self, dir: &Path, spec: MergeCommit) -> Result<()>;
async fn merge_no_commit(&self, dir: &Path, spec: MergeNoCommit) -> Result<()>;
async fn merge_abort(&self, dir: &Path) -> Result<()>;
async fn merge_continue(&self, dir: &Path) -> Result<()>;
async fn reset_merge(&self, dir: &Path) -> Result<()>;
async fn reset_hard(&self, dir: &Path, rev: &str) -> Result<()>;
```

- **`fetch`** — `fetch --quiet` from the default remote, prompt-off, retried on
  transient failures (3 attempts, 500 ms backoff).
- **`fetch_from`** — fetch from a *named* remote; same containment and retry.
- **`fetch_branch`** — fetch one branch into its remote-tracking ref
  (`fetch --quiet origin refs/heads/<b>:refs/remotes/origin/<b>`); same retry.
- **`push`** — `push [-u] <remote> <refspec>`, prompt-off; built through
  [`GitPush`](#gitpush).
- **`merge_squash`** — stage a branch's changes without committing (`merge --squash`).
- **`merge_commit`** — `merge [--no-ff] [-m <msg> | --no-edit] <branch>`; with no
  message it takes the default merge message non-interactively. Built through
  [`MergeCommit`](#mergecommit).
- **`merge_no_commit`** — merge without committing, for a dry run (`merge --no-commit
  [--squash | --no-ff] <branch>`). Built through [`MergeNoCommit`](#mergenocommit).
- **`merge_abort`** — `merge --abort`.
- **`merge_continue`** — finish a merge after resolving conflicts (`commit --no-edit`,
  editor suppressed).
- **`reset_merge`** — clear merge state, squash-safe (`reset --merge`).
- **`reset_hard`** — move `HEAD` and the working tree to `rev`, discarding all
  staged and unstaged changes (`reset --hard <rev>`) — destructive; there is no
  undo for uncommitted work.

```rust,ignore
# use std::path::Path;
# use vcs_git::{Git, GitApi, GitPush, MergeCommit, is_merge_conflict};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
git.fetch(repo).await?;                                      // retried on transient failure
git.push(repo, GitPush::branch("feature").set_upstream()).await?; // `push -u origin feature`

match git.merge_commit(repo, MergeCommit::branch("feature").no_ff()).await {  // --no-ff, default message
    Ok(()) => {}
    Err(e) if is_merge_conflict(&e) => {
        // resolve conflicts, then:
        git.merge_continue(repo).await?;
    }
    Err(e) => return Err(e),
}
# Ok(()) }
```

## Rebase & sequencer

```rust,ignore
async fn rebase(&self, dir: &Path, onto: &str) -> Result<()>;
async fn rebase_abort(&self, dir: &Path) -> Result<()>;
async fn rebase_continue(&self, dir: &Path) -> Result<()>;
async fn rebase_skip(&self, dir: &Path) -> Result<()>;
async fn cherry_pick(&self, dir: &Path, rev: &str) -> Result<()>;
async fn revert(&self, dir: &Path, rev: &str) -> Result<()>;
```

Every command here suppresses the editor (`GIT_EDITOR=true`,
`GIT_SEQUENCE_EDITOR=true`) so it never hangs a headless caller.

- **`rebase`** — rebase the current branch onto `onto` (`rebase <onto>`).
- **`rebase_abort`** / **`rebase_continue`** — `rebase --abort` / `--continue`.
- **`rebase_skip`** — `rebase --skip`; mainly for the `apply` backend's "nothing to
  commit" stop (the default `merge` backend auto-drops emptied patches on `--continue`).
- **`cherry_pick`** — apply a commit onto the current branch (`cherry-pick <rev>`); a
  conflict surfaces as an error classifiable by `is_merge_conflict`.
- **`revert`** — revert a commit with the default message (`revert --no-edit <rev>`).

```rust,ignore
# use std::path::Path;
# use vcs_git::{Git, GitApi, is_merge_conflict};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
match git.cherry_pick(repo, "abc123").await {
    Ok(()) => {}
    Err(e) if is_merge_conflict(&e) => {
        // resolve, then continue or abort
        git.rebase_abort(repo).await.ok();
    }
    Err(e) => return Err(e),
}
# Ok(()) }
```

## Stash

```rust,ignore
async fn stash_push(&self, dir: &Path, include_untracked: bool) -> Result<()>;
async fn stash_pop(&self, dir: &Path) -> Result<()>;
```

- **`stash_push`** — `stash push` (`--include-untracked` when asked), e.g. to save
  state before a copy-on-write restore.
- **`stash_pop`** — restore the most recent stash and drop it (`stash pop`).

```rust,ignore
# use std::path::Path;
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
git.stash_push(repo, true).await?;   // include untracked
// … do work on a clean tree …
git.stash_pop(repo).await?;
# Ok(()) }
```

## In-progress state

```rust,ignore
async fn is_rebase_in_progress(&self, dir: &Path) -> Result<bool>;
async fn is_merge_in_progress(&self, dir: &Path) -> Result<bool>;
```

- **`is_rebase_in_progress`** — `true` when a `rebase-merge`/`rebase-apply` dir exists
  under the git dir.
- **`is_merge_in_progress`** — `true` when `MERGE_HEAD` exists under the git dir.

```rust,ignore
# use std::path::Path;
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
if git.is_rebase_in_progress(repo).await? || git.is_merge_in_progress(repo).await? {
    println!("repo is mid-operation");
}
# Ok(()) }
```

## Clone / tags / config / show

```rust,ignore
async fn clone_repo(&self, url: &str, dest: &Path, spec: CloneSpec) -> Result<()>;
async fn tag_create(&self, dir: &Path, name: &str, rev: Option<String>) -> Result<()>;
async fn tag_create_annotated(&self, dir: &Path, spec: AnnotatedTag) -> Result<()>;
async fn tag_list(&self, dir: &Path) -> Result<Vec<String>>;
async fn tag_delete(&self, dir: &Path, name: &str) -> Result<()>;
async fn show_file(&self, dir: &Path, rev: &str, path: &str) -> Result<String>;
async fn config_get(&self, dir: &Path, key: &str) -> Result<Option<String>>;
async fn config_set(&self, dir: &Path, key: &str, value: &str) -> Result<()>;
```

- **`clone_repo`** — `git clone <url> <dest>` plus [`CloneSpec`](#clonespec) flags.
  Runs without a working directory — pass an **absolute** `dest`. Prompt-off.
- **`tag_create`** — a lightweight tag at `rev` (`tag <name> [<rev>]`; `None` = HEAD).
- **`tag_create_annotated`** — `tag -a <name> -m <message> [<rev>]`; built through
  [`AnnotatedTag`](#annotatedtag).
- **`tag_list`** — tag names in git's default ordering (`tag --list`).
- **`tag_delete`** — `tag -d <name>`.
- **`show_file`** — a file's content at a revision (`show <rev>:<path>`). `path` is
  repo-relative; backslashes are normalised to `/`. Decoded **lossily** — binary
  files come back mangled rather than erroring.
- **`config_get`** — a config key's value, or `None` when unset (`config --get <key>`).
  A multi-valued key errors; read those via `run`.
- **`config_set`** — set a key in the repo's local config (`config <key> <value>`).

```rust,ignore
# use std::path::Path;
# use vcs_git::{Git, GitApi, AnnotatedTag, CloneSpec};
# async fn demo(git: &Git) -> Result<(), processkit::Error> {
git.clone_repo(
    "https://example.com/repo.git",
    Path::new("/abs/dest"),
    CloneSpec::new().branch("main").depth(1),
).await?;                                                    // shallow, single branch

let repo = Path::new("/abs/dest");
git.tag_create_annotated(repo, AnnotatedTag::new("v1.0.0", "first release")).await?;
if let Some(name) = git.config_get(repo, "user.name").await? { // Option<String>
    println!("user.name = {name}");
}
let readme = git.show_file(repo, "HEAD", "README.md").await?;  // String (lossy)
let _ = readme;
# Ok(()) }
```

## Discovery

```rust,ignore
async fn version(&self) -> Result<String>;
async fn capabilities(&self) -> Result<GitCapabilities>;
async fn common_dir(&self, dir: &Path) -> Result<PathBuf>;
async fn git_dir(&self, dir: &Path) -> Result<PathBuf>;
async fn init(&self, dir: &Path) -> Result<()>;
```

- **`version`** — `git --version` text.
- **`capabilities`** — the parsed version as [`GitCapabilities`](#gitcapabilities). A
  value type — probe once and keep it; an unrecognisable version string is an
  `Error::Parse`.
- **`common_dir`** — the repository's common git directory (`rev-parse
  --git-common-dir`), stable across linked worktrees.
- **`git_dir`** — this worktree's git directory (`rev-parse --git-dir`).
- **`init`** — initialise a repository (`git init`).

```rust,ignore
# use std::path::Path;
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
let caps = git.capabilities().await?;                       // GitCapabilities
caps.ensure_supported()?;                                    // clear error if git < 2
println!("git {}", caps.version);
let common = git.common_dir(repo).await?;                    // PathBuf
let _ = common;
# Ok(()) }
```

## Raw escape hatches

```rust,ignore
async fn run(&self, args: &[String]) -> Result<String>;
async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>>;
```

- **`run`** — `git <args>` in the current directory, returning trimmed stdout
  (errors on a non-zero exit). For unmodelled commands.
- **`run_raw`** — like `run` but never errors on a non-zero exit — returns the
  captured `ProcessResult`.

These are **not** flag-guarded — the caller owns the argv. The inherent
`run_args` / `run_raw_args` take `&[&str]` to skip the `Vec<String>` allocation.

```rust,ignore
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git) -> Result<(), processkit::Error> {
let out = git.run(&["describe".into(), "--tags".into()]).await?; // String
let res = git.run_raw(&["status".into(), "-s".into()]).await?;   // ProcessResult<String>
println!("exited {:?}", res.code());
let _ = out;
# Ok(()) }
```

### Composed inherent helpers

These live on `Git` (and `GitAt`), not the object-safe `GitApi` trait — they are
multi-step operations, not 1:1 CLI verbs, so mock their underlying calls instead.

- **`switch_with_stash(dir, branch)`** — switch to `branch`, carrying uncommitted
  changes (tracked *and* untracked) across via the stash: `stash push -u` →
  `checkout` → `stash pop`. A clean tree skips the stash round-trip. On a failed
  checkout the stash is popped back to restore the original state; on a conflicting
  pop the target branch stays checked out with the stash entry preserved.

```rust,ignore
# use std::path::Path;
# use vcs_git::Git;
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
git.switch_with_stash(repo, "feature").await?;  // dirty tree comes along
# Ok(()) }
```

### Blocking helpers

```text
pub fn blocking::worktree_remove(dir: &Path, path: &Path, force: bool) -> std::io::Result<()>;
```

A synchronous, best-effort `git worktree remove [--force] <path>` for contexts that
cannot `.await` — chiefly a `Drop` guard. It shells out through `std::process`
directly (no async, no job-containment), so reserve it for short-lived cleanup.

## Result types

All result structs/enums are `#[non_exhaustive]` (except `GitVersion`) — match with
a trailing `..` and construct via the crate, not struct literals.

The diff types (`ChangeKind`, `DiffLine`, `Hunk`, `FileDiff`, `DiffStat`,
`parse_diff`) and `GitVersion` actually live in the shared
[`vcs-diff`](https://crates.io/crates/vcs-diff) crate — `git diff` and
`jj diff --git` are byte-identical for ASCII paths (they differ only in
non-ASCII filename rendering, which the shared parser decodes), so `vcs-git`
and `vcs-jj` share one parser.
They're re-exported here, so `vcs_git::FileDiff` etc. still resolve (`GitVersion`
is an alias of `vcs_diff::Version`).

### `StatusEntry`

One entry from `git status --porcelain=v1 -z`.

| field | type | meaning |
|-------|------|---------|
| `code` | `String` | two-character status code, e.g. `" M"`, `"??"`, `"A "`, `"R "` |
| `path` | `String` | the path (the *new* path for a rename/copy); raw, unquoted |
| `old_path` | `Option<String>` | the original path for a rename/copy; `None` otherwise |

### `BranchStatus`

The combined snapshot from `branch_status` (`status --porcelain=v2 --branch -z`).
`is_dirty()` returns whether there's any change (tracked or untracked).

| field | type | meaning |
|-------|------|---------|
| `head` | `Option<String>` | HEAD commit's full oid; `None` on an unborn repo (truncate for display) |
| `branch` | `Option<String>` | current branch; `None` when detached |
| `upstream` | `Option<String>` | upstream tracking branch; `None` when unset |
| `ahead` | `Option<usize>` | commits ahead of upstream; `None` with no upstream |
| `behind` | `Option<usize>` | commits behind upstream; `None` with no upstream |
| `tracked_changes` | `usize` | changed tracked entries (`1`/`2`/`u` records) |
| `untracked` | `usize` | untracked files (`?` records) |
| `conflicts` | `usize` | unmerged entries (`u` records; also in `tracked_changes`) |

### `Commit`

A commit parsed from a `\x1f`-delimited `git log` line.

| field | type | meaning |
|-------|------|---------|
| `hash` | `String` | full commit hash (`%H`) |
| `short_hash` | `String` | abbreviated hash (`%h`) |
| `author` | `String` | author name (`%an`) |
| `date` | `String` | author date, strict ISO-8601 (`%aI`) |
| `subject` | `String` | subject line (`%s`) |

### `Branch`

| field | type | meaning |
|-------|------|---------|
| `name` | `String` | branch name |
| `current` | `bool` | whether this is the checked-out branch (the `*` marker) |

### `Worktree`

| field | type | meaning |
|-------|------|---------|
| `path` | `PathBuf` | absolute path to the worktree |
| `branch` | `Option<String>` | short branch name (`refs/heads/` stripped); `None` when detached or bare |
| `head` | `Option<String>` | the checked-out commit; `None` for a bare entry |
| `bare` | `bool` | the main worktree of a bare repository |
| `detached` | `bool` | checked out at a detached HEAD |
| `locked` | `bool` | locked against pruning |

### `DiffStat`

`Copy`. Aggregate counts from `git diff --shortstat`.

| field | type | meaning |
|-------|------|---------|
| `files_changed` | `usize` | number of files changed |
| `insertions` | `usize` | lines added |
| `deletions` | `usize` | lines removed |

### `FileDiff`

One file's entry in a parsed git-format unified diff.

| field | type | meaning |
|-------|------|---------|
| `change` | `ChangeKind` | how the file changed |
| `path` | `String` | the file's path (the *new* path for a rename), `/`-normalised |
| `old_path` | `Option<String>` | the original path for a rename, `/`-normalised; `None` otherwise |
| `hunks` | `Vec<Hunk>` | the `@@` hunks; empty for a binary file or a pure rename |
| `raw` | `String` | the verbatim `diff --git …` block, for callers that display raw text |

#### `ChangeKind`

`Copy` enum: `Added`, `Modified`, `Deleted`, `Renamed`.

#### `Hunk`

A single `@@ … @@` hunk within a `FileDiff`.

| field | type | meaning |
|-------|------|---------|
| `old_start` | `usize` | start line in the old file |
| `old_lines` | `usize` | line count in the old file (defaults to 1 when the `,count` is omitted) |
| `new_start` | `usize` | start line in the new file |
| `new_lines` | `usize` | line count in the new file (defaults to 1 when omitted) |
| `section` | `String` | text after the closing `@@` (the function/section heading); empty when none |
| `lines` | `Vec<DiffLine>` | the hunk body, one entry per line |

#### `DiffLine`

Enum, one variant per line role; the stored text excludes the leading marker:
`Context(String)` (` `), `Added(String)` (`+`), `Removed(String)` (`-`).

### `BlameLine`

One line of `git blame --line-porcelain` output.

| field | type | meaning |
|-------|------|---------|
| `commit` | `String` | full hash of the commit that last changed the line |
| `orig_line` | `u32` | line number in that commit's version (1-based) |
| `final_line` | `u32` | line number in the blamed version (1-based) |
| `author` | `String` | author name of that commit |
| `author_time` | `i64` | author timestamp as a unix epoch (seconds) |
| `author_tz` | `String` | author timezone offset, e.g. `+0200` |
| `content` | `String` | the line's content (no trailing newline) |

### `GitVersion`

`Copy`, and `Ord` (so versions compare directly). **Not** `#[non_exhaustive]`.

| field | type | meaning |
|-------|------|---------|
| `major` | `u64` | major component (`2` in `2.54.0`) |
| `minor` | `u64` | minor component |
| `patch` | `u64` | patch component (`0` when the binary reports only `major.minor`) |

Displays as `major.minor.patch`.

### `GitCapabilities`

`Copy`. What the installed `git` supports, probed via `capabilities()`.

| field | type | meaning |
|-------|------|---------|
| `version` | `GitVersion` | the binary's parsed version |

Methods: `is_supported(&self) -> bool` (major ≥ 2) and `ensure_supported(&self) ->
Result<()>` (a clear "needs git ≥ 2" error otherwise).

## Config & builder types

### `DiffSpec`

An enum selecting what `diff` / `diff_text` compares — a re-export of
`vcs_diff::DiffSpec`, deliberately exhaustive (not `#[non_exhaustive]`):

- `DiffSpec::WorkingTree` — all tracked working-tree changes vs the last commit
  (`git diff HEAD`), staged or not, excluding untracked files.
- `DiffSpec::Rev(String)` — a specific revision or range, e.g. `main..HEAD` or
  `HEAD~1` (`git diff <rev>`).

### `MergeCheck`

The spec `is_merged` takes — "is `branch` fully merged into `base`?". A two-step
type-state builder (`#[non_exhaustive]`), so the two same-typed refs are named across
separate steps and can't be silently transposed (a swap would *invert* the answer):

```rust,ignore
pub fn branch(name: impl Into<String>) -> MergeCheckPartial; // entry: names the branch tested
pub fn into_base(self, base: impl Into<String>) -> MergeCheck; // on MergeCheckPartial: names the base
// fields: pub branch: String, pub base: String
```

- Built as `MergeCheck::branch("feature").into_base("main")`; `is_merged` then runs
  `git branch --merged <base>` and reports whether `<branch>` appears.

### `WorktreeAdd`

Options for `worktree_add`. `#[non_exhaustive]` — build it through the constructors,
not a struct literal.

```rust,ignore
pub fn checkout(path: impl Into<PathBuf>, commitish: impl Into<String>) -> Self;
pub fn create_branch(path: impl Into<PathBuf>, name: impl Into<String>, commitish: impl Into<String>) -> Self;
pub fn no_checkout(self) -> Self;   // chainable: register without populating files (--no-checkout)
```

- **`checkout`** — a worktree at `path` checking out an existing `commitish`:
  `worktree add <path> <commitish>`.
- **`create_branch`** — create a new branch `name` based on `commitish`:
  `worktree add -b <name> <path> <commitish>`.
- **`no_checkout`** — register the worktree without populating its files
  (`--no-checkout`), for a caller (e.g. a copy-on-write clone) that fills the working
  tree itself.

Fields: `path: PathBuf`, `new_branch: Option<String>`, `commitish: Option<String>`,
`no_checkout: bool`.

```rust,ignore
# use vcs_git::WorktreeAdd;
let a = WorktreeAdd::checkout("/wt", "main");                       // existing branch
let b = WorktreeAdd::create_branch("/wt", "feature", "HEAD");       // new branch off HEAD
let c = WorktreeAdd::checkout("/wt", "main").no_checkout();         // skeleton only
# let _ = (a, b, c);
```

### `GitPush`

Options for `push`. `#[non_exhaustive]` — build it through the constructors.

```rust,ignore
pub fn branch(name: impl Into<String>) -> Self;                          // push origin <name>
pub fn refspec(local: impl AsRef<str>, remote_branch: impl AsRef<str>) -> Self; // push origin <local>:<remote_branch>
pub fn remote(self, remote: impl Into<String>) -> Self;                  // chainable: non-default remote
pub fn set_upstream(self) -> Self;                                       // chainable: record upstream (-u)
```

Fields: `remote: String` (defaults to `origin`), `refspec: String`,
`set_upstream: bool`.

```rust,ignore
# use vcs_git::GitPush;
let p = GitPush::branch("feature").set_upstream();           // push -u origin feature
let q = GitPush::refspec("local", "remote_branch").remote("upstream");
# let _ = (p, q);
```

### `CloneSpec`

Options for `clone_repo`. `#[non_exhaustive]`, `Default` — build through `new` and
the chained setters.

```rust,ignore
pub fn new() -> Self;                          // a plain full clone of the default branch
pub fn branch(self, branch: impl Into<String>) -> Self; // --branch
pub fn depth(self, depth: u32) -> Self;        // --depth (see local-path caveat below)
pub fn bare(self) -> Self;                     // --bare
```

Fields: `branch: Option<String>`, `depth: Option<u32>`, `bare: bool`.

`depth` is silently ignored by git for a plain local-path source (it warns and
clones fully); use a `file://` URL to shallow-clone locally.

```rust,ignore
# use vcs_git::CloneSpec;
let spec = CloneSpec::new().branch("main").depth(1);
let bare = CloneSpec::new().bare();
# let _ = (spec, bare);
```

### `CommitPaths`

Options for `commit_paths`. `#[non_exhaustive]` — build through `new` and the
chained setter.

```rust,ignore
pub fn new(paths: impl IntoIterator<Item = impl Into<PathBuf>>, message: impl Into<String>) -> Self;
pub fn amend(self) -> Self;                    // chainable: amend the previous commit (--amend)
```

Fields: `paths: Vec<PathBuf>` (`--only -- <paths>`), `message: String` (`-m`),
`amend: bool` (`--amend`).

```rust,ignore
# use vcs_git::CommitPaths;
let c = CommitPaths::new(["src/lib.rs"], "feat: thing");
let a = CommitPaths::new(["src/lib.rs"], "feat: thing").amend();
# let _ = (c, a);
```

### `MergeCommit`

Options for `merge_commit`. `#[non_exhaustive]` — build through `branch` and the
chained setters.

```rust,ignore
pub fn branch(name: impl Into<String>) -> Self; // merge --no-edit <name> (default message)
pub fn no_ff(self) -> Self;                     // chainable: always create a merge commit (--no-ff)
pub fn message(self, m: impl Into<String>) -> Self; // chainable: merge message (-m)
```

Fields: `branch: String`, `no_ff: bool` (`--no-ff`), `message: Option<String>`
(`-m`; `None` takes the default message non-interactively via `--no-edit`).

```rust,ignore
# use vcs_git::MergeCommit;
let m = MergeCommit::branch("feature").no_ff();             // --no-ff, default message
let n = MergeCommit::branch("feature").message("merge it"); // -m "merge it"
# let _ = (m, n);
```

### `MergeNoCommit`

Options for `merge_no_commit`. `#[non_exhaustive]` — build through `branch` and the
chained setters.

```rust,ignore
pub fn branch(name: impl Into<String>) -> Self; // merge --no-commit <name>
pub fn squash(self) -> Self;                    // chainable: stage the squashed result (--squash)
pub fn no_ff(self) -> Self;                     // chainable: record a real abortable merge (--no-ff)
```

Fields: `branch: String`, `squash: bool` (`--squash`; takes precedence over
`no_ff`), `no_ff: bool` (`--no-ff`). With `no_ff` (and not `squash`) git records
`MERGE_HEAD`, so the merge is abortable via `merge_abort`; with `squash` no
`MERGE_HEAD` is recorded — undo via `reset_merge` / `reset_hard`.

```rust,ignore
# use vcs_git::MergeNoCommit;
let dry = MergeNoCommit::branch("feature").no_ff();   // abortable dry-run merge
let sq = MergeNoCommit::branch("feature").squash();   // stage squashed, no MERGE_HEAD
# let _ = (dry, sq);
```

### `AnnotatedTag`

Options for `tag_create_annotated`. `#[non_exhaustive]` — build through `new` and
the chained setter.

```rust,ignore
pub fn new(name: impl Into<String>, message: impl Into<String>) -> Self; // tag -a <name> -m <message> at HEAD
pub fn rev(self, r: impl Into<String>) -> Self;  // chainable: tag <rev> instead of HEAD
```

Fields: `name: String`, `message: String` (`-m`), `rev: Option<String>` (`<rev>`;
`None` tags `HEAD`).

```rust,ignore
# use vcs_git::AnnotatedTag;
let t = AnnotatedTag::new("v1.0.0", "first release");
let u = AnnotatedTag::new("v1.0.0", "first release").rev("abc123");
# let _ = (t, u);
```

## Validating newtypes

Optional up-front validation for callers that accept names/revisions from untrusted
input (UIs, bots, agents) and want to fail early with a clear error at the input
boundary. They are **not** required wrappers — the dir-taking methods stay `&str`
and apply the same flag-injection guard internally on every call, regardless of
whether you used these.

### `RefName`

A pre-validated reference name (branch/tag/remote), following the load-bearing core
of `git check-ref-format`. Rejects a name that is:

- empty,
- has a leading `-` or `.`,
- contains `..`,
- contains a control character or space, or any of `~ ^ : ? * [ \`,
- ends with `/` or `.lock`.

```rust,ignore
pub fn new(name: impl Into<String>) -> Result<Self>;
pub fn as_str(&self) -> &str;
```

### `RevSpec`

A pre-validated revision/range expression (`HEAD~2`, `main..feature`). Deliberately
*minimal* — git's revision grammar is too rich to validate here — it only
guarantees the expression is non-empty and cannot be parsed as a flag (no leading
`-`), matching the internal guard.

```rust,ignore
pub fn new(rev: impl Into<String>) -> Result<Self>;
pub fn as_str(&self) -> &str;
```

```rust,ignore
# use vcs_git::{RefName, RevSpec};
# fn demo() -> Result<(), processkit::Error> {
let name = RefName::new("feature/login")?;   // Ok
let rev = RevSpec::new("main..HEAD")?;        // Ok
assert!(RefName::new("-evil").is_err());      // leading '-'
assert!(RefName::new("bad..name").is_err());  // contains '..'
let _ = (name, rev);
# Ok(()) }
```

Both implement `Display` and yield the validated string via `as_str()`.

## Error classification

git writes load-bearing diagnostics to *either* stream on failure, so these free
functions probe both `stdout` and `stderr` of an `Error::Exit` — call them instead
of re-implementing the string-scraping yourself.

```rust,ignore
pub fn is_merge_conflict(err: &Error) -> bool;        // a merge/cherry-pick stopped on conflicts
pub fn is_nothing_to_commit(err: &Error) -> bool;     // a commit found a clean tree
pub fn is_transient_fetch_error(err: &Error) -> bool; // DNS / dropped connection — retryable
```

`is_transient_fetch_error` deliberately does **not** treat a processkit-level
`Error::Timeout` as retryable: a timeout already spent the caller's full deadline, so
retrying it would multiply the wall-clock (a fetch is tried up to 3×). Raise the
timeout rather than have it silently tripled. See [Process model & errors](https://docs.rs/vcs-core/latest/vcs_core/guide/process_model/) for the `Error` shape.

## See also

- [Conflict resolution](https://docs.rs/vcs-git/latest/vcs_git/guide/conflicts/) — `vcs_git::conflict`: parse conflict markers
  into structured regions and resolve a chosen side.
- [Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/) — the `mock` feature's `MockGitApi` and the
  `ScriptedRunner` / `RecordingRunner` seams.
- [Security & hardening](https://docs.rs/vcs-git/latest/vcs_git/guide/security/) — `Git::hardened()` and the injection guards.
- [Process model & errors](https://docs.rs/vcs-core/latest/vcs_core/guide/process_model/) — job containment, timeouts, and the
  structured `Error`.
- [the crate docs](https://docs.rs/vcs-git).
