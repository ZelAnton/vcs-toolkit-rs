# Cookbook

Task-oriented, end-to-end recipes that compose the wrappers into the jobs people
actually reach for — deeper than the README snippets, lighter than the per-crate
guides ([git](https://docs.rs/vcs-git/latest/vcs_git/guide/) / [jj](https://docs.rs/vcs-jj/latest/vcs_jj/guide/) / [github](https://docs.rs/vcs-github/latest/vcs_github/guide/) / [gitlab](https://docs.rs/vcs-gitlab/latest/vcs_gitlab/guide/) /
[gitea](https://docs.rs/vcs-gitea/latest/vcs_gitea/guide/) / [core](https://docs.rs/vcs-core/latest/vcs_core/guide/) / [forge](https://docs.rs/vcs-forge/latest/vcs_forge/guide/)), which document the full
surface each recipe draws on.

## A prompt / status-bar line in one or two spawns

A shell prompt or TUI refreshes constantly, so the cost is per-field spawns.
[`Repo::snapshot`](https://docs.rs/vcs-core/latest/vcs_core/guide/) batches the common state into **one** `git status
--porcelain=v2 --branch` (plus an in-progress probe) — or, on jj, one `log -r @`
template plus a change count — and hands back a [`RepoSnapshot`](https://docs.rs/vcs-core/latest/vcs_core/guide/).

```rust,ignore
# use vcs_core::{Repo, OperationState};
# async fn demo(repo: &Repo) -> vcs_core::Result<()> {
let s = repo.snapshot().await?;                 // RepoSnapshot — one/two spawns

let branch = s.branch.as_deref().unwrap_or("(detached)");
let mut line = branch.to_string();
if let (Some(a), Some(b)) = (s.ahead, s.behind) {
    line.push_str(&format!(" ↑{a}↓{b}"));       // upstream tracking — git only
}
if s.dirty {
    line.push_str(" *");                        // uncommitted changes
}
if s.conflicted || s.operation != OperationState::Clear {
    line.push_str(" ⚠");                        // mid-merge/rebase or conflicted
}
println!("{line}");                             // e.g. `main ↑1↓0 *`
# Ok(()) }
```

Notes: `upstream`/`ahead`/`behind` are **always `None` on jj** (no git-style
upstream tracking) — the `↑↓` segment simply won't render there. If you're git-only
and want the raw one-spawn primitive without the facade, call
[`GitApi::branch_status`](https://docs.rs/vcs-git/latest/vcs_git/guide/) directly — it returns a [`BranchStatus`](https://docs.rs/vcs-git/latest/vcs_git/guide/)
with the same fields plus `is_dirty()`.

## Keep a status line live

Don't poll `snapshot()` on a timer — let [`vcs-watch`](https://docs.rs/vcs-watch/latest/vcs_watch/guide/) push a fresh one
whenever the repo actually changes. It filesystem-watches `.git`/`.jj`, debounces,
re-queries, and hands you the new [`RepoSnapshot`] plus the typed deltas.

```rust,ignore
# use vcs_core::Repo;
# use vcs_watch::RepoWatcher;
# async fn demo() -> vcs_watch::Result<()> {
let repo = Repo::open(".")?;
let mut watcher = RepoWatcher::watch(repo).await?;     // tokio runtime required
render(watcher.current());                             // initial paint
while let Some(change) = watcher.recv().await {
    render(&change.snapshot);                           // repaint with the fresh state
    let _ = &change.events;                             // …or react to specific deltas
}
# Ok(()) }
# fn render(_s: &vcs_watch::RepoSnapshot) {}
```

Notes: each `change` carries both the full `snapshot` (repaint) and `events`
(`HeadMoved`/`BranchCreated`/`WorkingCopyChanged`/… — react). A bare unstaged edit
is caught only once staged unless you opt into `RepoWatcher::builder(repo)
.working_tree(true)`. Dropping the watcher stops it. `RepoSnapshot` is re-exported
from `vcs-watch`, so depending on it alone suffices.

## Open a PR and wait for CI

Push a branch, open the PR, then block on its workflow run and branch on the
outcome. `gh run watch` blocks for the whole run, so drive it from a client with a
generous (or no) timeout — see [github.md](https://docs.rs/vcs-github/latest/vcs_github/guide/).

```rust,ignore
# use std::path::Path;
# use vcs_github::{GitHub, GitHubApi, PrCreate};
# async fn demo(gh: &GitHub, repo: &Path) -> Result<(), processkit::Error> {
if !gh.auth_status().await? {                                   // bool
    return Ok(()); // not logged in — `gh auth login` first
}
let spec = PrCreate::new("Add the thing", "Body.").head("feat/x").base("main");
let url = gh.pr_create(repo, spec).await?;                      // String — PR url
println!("opened {url}");

// The newest run on the head branch carries the id `run_watch` needs.
let runs = gh.run_list(repo, 1, Some("feat/x".into())).await?; // Vec<WorkflowRun>, newest first
if let Some(run) = runs.first() {
    let done = gh.run_watch(repo, run.database_id).await?;      // blocks, then re-reads → WorkflowRun
    match done.conclusion.as_str() {                           // "" until complete; "success"/"failure"/…
        "success" => println!("CI green"),
        other => println!("CI {other}: {}", done.url),
    }
}
# Ok(()) }
```

Notes: `run_watch` deliberately omits `--exit-status`, so the outcome travels in
`WorkflowRun.conclusion` (a failed run can't be told from a cancelled one by exit
code). `PrCreate`'s `.head()`/`.base()` are optional — omitted means the current
branch / repo default. `run_list`'s `limit` is a `u64`. **Targeting GitLab or
Gitea instead of GitHub?** Use the [`vcs-forge`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/) facade — one
`Forge::pr_create`/`pr_merge`/`pr_checks` lifecycle across all three forges, with
unified DTOs (it picks the binary; `gh`-specific bits like `run_watch` stay on
`vcs-github`).

## Cancel a long-running watch / fetch

`run_watch` blocks for the whole CI run; a `fetch`/`clone`/`push` over a dead
network can hang for its full timeout. Cancellation is always available (no
feature flag): a client built with `default_cancel_on(token)` carries that token
into *every* command it runs, so one `token.cancel()` kills all of its in-flight
calls — no new API, no per-call plumbing.

```rust,ignore
// `CancellationToken` and `Error` are both
// re-exported by each wrapper, so a consumer needn't depend on `processkit` directly.
use vcs_github::{CancellationToken, Error, GitHub, GitHubApi};

let token = CancellationToken::new();
// Scope the cancellation to a CLIENT, not a call — clients are cheap; give each
// cancellable scope its own (child) token.
let gh = GitHub::new().default_cancel_on(token.child_token());

// A controller (timeout, Ctrl-C handler, "stop" button) cancels out-of-band:
tokio::spawn(async move {
    shutdown_signal().await;
    token.cancel();                          // every in-flight gh call dies (kill-on-close tree)
});

match gh.run_watch(repo, run_id).await {     // long block — interruptible now
    Err(e) if matches!(e, Error::Cancelled { .. }) => println!("watch cancelled"),
    other => { other?; }
}
```

A per-command `cancel_on` on a built command **replaces** the client default
(explicit beats default, like `timeout`); derive both from one `child_token()` if
you need two cancel sources. `Error::Cancelled` is **terminal** — the fetch-retry
treats it as non-transient and will not replay a cancelled run. Through the facades,
build the wrapped client the same way (`GitHub::new().default_cancel_on(t)`) and
hand it to `Forge::for_github(cwd, client)` / `Repo::from_git(root, cwd, client)`.

**Cancellation is "stop now", not "stop and clean up".** A fired token kills
*every* command the client still runs — **including any cleanup the toolkit itself
issues**. A multi-step facade operation that is cancelled mid-flight can therefore
be left part-done: [`Repo::try_merge`](crate::Repo::try_merge) probes a throwaway merge and
rolls it back with `op_restore` (jj) / `merge --abort` (git), but that rollback runs
on the same client, so a token that fired during the probe also cancels the rollback
— the probe change may remain. Likewise [`Jj::transaction`]'s op-log rollback runs
on `Err`, and a cancellation *is* an `Err`, but the `op_restore` it would run is
itself cancelled. If you need a guaranteed-clean state after cancelling, re-probe
(`Repo::in_progress_state` / `Jj::op_head`) and reset with a **fresh, un-cancelled
client** rather than assuming the interrupted call tidied up after itself.

## Stash-safe branch switch

Carry a dirty working tree across a checkout without losing it.
[`switch_with_stash`](https://docs.rs/vcs-git/latest/vcs_git/guide/) is an inherent helper on `Git` (not the `GitApi`
trait): it does `stash push -u` → `checkout` → `stash pop`, popping back to restore
the original branch if the checkout fails.

```rust,ignore
# use std::path::Path;
# use vcs_git::Git;
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
git.switch_with_stash(repo, "feature").await?;   // tracked + untracked changes come along
# Ok(()) }
```

Notes: a clean tree skips the stash round-trip entirely. On a conflicting pop the
target branch stays checked out with the stash entry preserved — inspect with
`git stash list` rather than assuming the pop landed. Being a composed helper, it
lives off the object-safe trait; in tests, script its underlying `stash`/`checkout`
calls rather than mocking one method.

## Programmatic conflict resolution

Resolve every conflict in a file to one side, without a text editor. The
[`conflict`](https://docs.rs/vcs-git/latest/vcs_git/guide/conflicts/) modules are **pure parsers over a file's content** — the
client fetches the bytes, the module reasons about them. Pair
[`conflicted_files`](https://docs.rs/vcs-git/latest/vcs_git/guide/) (or jj's [`resolve_list`](https://docs.rs/vcs-jj/latest/vcs_jj/guide/)) to find the paths
with [`show_file`](https://docs.rs/vcs-git/latest/vcs_git/guide/) / [`file_show`](https://docs.rs/vcs-jj/latest/vcs_jj/guide/) (or just reading the worktree
file) to get the bytes.

```rust,ignore
# use std::path::Path;
# use vcs_git::{Git, GitApi};
# use vcs_git::conflict::{has_conflict_markers, parse_conflicts, resolve, ResolutionSide};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
for path in git.conflicted_files(repo).await? {           // Vec<String>, `/`-separated
    let content = std::fs::read_to_string(repo.join(&path))?;
    if !has_conflict_markers(&content) {
        continue;                                         // cheap pre-check
    }
    let segments = parse_conflicts(&content)?;            // Vec<ConflictSegment>
    let resolved = resolve(&segments, ResolutionSide::Ours)?; // keep our side everywhere
    std::fs::write(repo.join(&path), resolved)?;
}
// then `git.add(...)` + `git.merge_continue(repo)` / `rebase_continue`.
# Ok(()) }
```

The jj side mirrors this with [`JjResolution`](https://docs.rs/vcs-git/latest/vcs_git/guide/conflicts/) and 0-based side
indices (jj conflicts can have more than two sides):

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# use vcs_jj::conflict::{parse_conflicts, resolve, JjResolution};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
for path in jj.resolve_list(repo, "@").await? {           // Vec<String> — conflicts on `@`
    let content = jj.file_show(repo, "@", &path).await?;  // String (lossy)
    let segments = parse_conflicts(&content)?;            // Vec<JjConflictSegment>
    let resolved = resolve(&segments, JjResolution::Side(0))?; // the first side
    std::fs::write(repo.join(&path), resolved)?;
}
# Ok(()) }
```

Notes: `ResolutionSide::Base` (git) / `JjResolution::Base` errors when the conflict
records no base (git's 2-way `merge` style has none). `render(parse(x)?)` is a
byte-exact round-trip, so resolving only rewrites the regions you chose. A
jj file materialized with the `git` marker style parses with `vcs_git::conflict`,
not `vcs_jj::conflict` (the module steers you there on a mismatch).

## Detect the backend and dispatch

Write one code path that works on git *or* jj. [`detect`](https://docs.rs/vcs-core/latest/vcs_core/guide/) probes the
filesystem (jj wins when colocated), and [`Repo::open`](https://docs.rs/vcs-core/latest/vcs_core/guide/) opens a handle
bound to a directory; the common methods dispatch to whichever tool is present.

```rust,ignore
# use vcs_core::{detect, Repo, BackendKind};
# use std::path::Path;
# async fn demo(start: &Path) -> vcs_core::Result<()> {
if detect(start).is_none() {                       // Option<Located> — no spawn
    return Ok(()); // not a repo
}
let repo = Repo::open(start)?;
println!("backend: {}", repo.kind().as_str());     // "git" / "jj"

for change in repo.changed_files().await? { /* FileChange */ let _ = change; }
let branch = repo.current_branch().await?;         // Option<String>
let conflicts = repo.conflicted_files().await?;    // Vec<String>
let _ = (branch, conflicts);

// Drop to the raw client for tool-specific ops off the common surface:
match repo.kind() {
    BackendKind::Git => { let _g = repo.git().unwrap();  /* git-only verbs */ }
    BackendKind::Jj  => { let _j = repo.jj().unwrap();   /* jj-only verbs  */ }
}
# Ok(()) }
```

Notes: `repo.git()` / `repo.jj()` return `Option` — `Some` only for the matching
backend. For a dir-free view bound to the handle's cwd, use `repo.git_at()` /
`repo.jj_at()`. A consumer that wants to avoid naming the runner generic can hold a
`&dyn VcsRepo` instead.

## jj transaction with op-log rollback

jj's operation log makes a multi-step mutation atomically reversible — something
git can't faithfully offer. [`Jj::transaction`](https://docs.rs/vcs-jj/latest/vcs_jj/guide/) captures the current op
head, runs your closure against a bound [`JjAt`](https://docs.rs/vcs-jj/latest/vcs_jj/guide/), and restores the op head on
`Err`.

```rust,ignore
# use std::path::Path;
# use vcs_jj::Jj;
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
jj.transaction(repo, |tx| async move {        // tx: JjAt, dir pre-bound
    tx.describe("wip: refactor").await?;
    tx.new_change("next").await               // an Err here rolls back the describe
})
.await?;
# Ok(()) }
```

Notes: `transaction` is inherent (the generic closure can't live on the object-safe
trait). Rollback runs on `Err` only — **not** on panic or a dropped future (no async
`Drop`); convert panics to `Err` inside the closure if you need that. If the restore
itself fails, the closure's original error is returned and the repo may be left
mid-transaction — re-probe `op_head` to detect it.

## Test a consumer hermetically

Depend on the interface, never the concrete client, then pick the cheapest seam (see
[testing.md](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/)). Stub a whole method with the `mock` feature, or feed
canned process output through the real argv-building and parsing with a
`ScriptedRunner` — and assert the exact argv with a `RecordingRunner`.

```rust,ignore
# use std::path::Path;
use vcs_git::{Git, GitApi};
use processkit::testing::{RecordingRunner, Reply, ScriptedRunner};

// 1. Code against the trait — the mock implements it too.
async fn on_main(git: &dyn GitApi) -> bool {
    git.current_branch(Path::new(".")).await.unwrap() == "main"
}

# async fn demo() -> Result<(), processkit::Error> {
// 2. Feed canned output through the real command wiring (no binary, no repo).
let git = Git::with_runner(ScriptedRunner::new().on(["git", "rev-parse"], Reply::ok("main\n")));
assert!(on_main(&git).await);

// 3. Record to assert the exact argv that was built.
let rec = RecordingRunner::replying(Reply::ok(""));
let git = Git::with_runner(&rec);
git.create_branch(Path::new("/repo"), "feature").await?;
assert_eq!(rec.only_call().args_str(), ["branch", "feature"]);
# Ok(()) }
```

Notes: the `mock` feature (`MockGitApi` / `MockJjApi` / `MockGitHubApi`) lives in
`[dev-dependencies]` only — it never ships in release builds. To test the
`vcs-core` facade's dispatch, build a `Repo` over a fake runner with
`Repo::from_git("/repo", "/repo", Git::with_runner(runner))` / `Repo::from_jj(…)`.

## Drop to a raw command

Every client carries an escape hatch for an unmodelled command. `run` returns
trimmed stdout and errors on a non-zero exit; `run_raw` never errors on exit — it
hands back the captured `ProcessResult` so you read the code yourself.

```rust,ignore
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git) -> Result<(), processkit::Error> {
let described = git.run(&["describe".into(), "--tags".into()]).await?;  // String
let res = git.run_raw(&["rev-parse".into(), "HEAD".into()]).await?;     // ProcessResult<String>
println!("exited {:?}", res.code());
let _ = described;

// Inherent `run_args` / `run_raw_args` take &[&str] — no Vec<String> allocation:
let short = git.run_args(&["rev-parse", "--short", "HEAD"]).await?;     // String
let _ = short;
# Ok(()) }
```

Notes: `run` / `run_raw` are the same on `Jj` and `GitHub` (`gh.run(&["api",
"user".into()])`, etc.). These are **not** flag-guarded — you own the argv, so a
caller-supplied value with a leading `-` is passed through verbatim. Through the
facade, reach them via `repo.git()?.run(…)` / `repo.jj()?.run(…)`.

## See also

- [vcs-git guide](https://docs.rs/vcs-git/latest/vcs_git/guide/) — the full git surface.
- [vcs-jj guide](https://docs.rs/vcs-jj/latest/vcs_jj/guide/) — the full jj surface, including the operation log.
- [vcs-github guide](https://docs.rs/vcs-github/latest/vcs_github/guide/) — the full `gh` surface.
- [vcs-core guide](https://docs.rs/vcs-core/latest/vcs_core/guide/) — the backend-agnostic facade and DTOs.
- [Conflict resolution](https://docs.rs/vcs-git/latest/vcs_git/guide/conflicts/) — the `vcs_git::conflict` / `vcs_jj::conflict`
  marker models in depth.
- [Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/) — the three test seams and the dry-run harness.
