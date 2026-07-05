# vcs-watch — repo-event stream

`vcs-watch` filesystem-watches a git or jj repository and streams **typed
state-change events** — the foundation for prompts, status bars, TUIs, and
daemons. It's built on [`vcs-core`](https://docs.rs/vcs-core/latest/vcs_core/guide/): on each filesystem change it
**re-queries** the repo's batched [`snapshot`](https://docs.rs/vcs-core/latest/vcs_core/guide/), **diffs** it
against the previous state, and emits the deltas.

```rust,ignore
use vcs_core::Repo;
use vcs_watch::{RepoWatcher, RepoEvent};

# async fn demo() -> vcs_watch::Result<()> {
let repo = Repo::discover(".")?;
let mut watcher = RepoWatcher::watch(repo).await?;
while let Some(change) = watcher.recv().await {
    for event in &change.events {
        match event {
            RepoEvent::HeadMoved { to, .. }      => println!("head → {to:?}"),
            RepoEvent::BranchCreated { name, .. } => println!("+branch {name}"),
            RepoEvent::WorkingCopyChanged { dirty, .. } => println!("dirty={dirty}"),
            other => println!("{other:?}"),
        }
    }
    // `change.snapshot` is the fresh full state — render a status line from it.
}
# Ok(()) }
```

## Why re-query + diff (not raw events)

Interpreting raw filesystem events is a trap: git writes refs through a temp-file
rename, churns `index.lock`, and appends to `.git/logs/` constantly. `vcs-watch`
treats **any** event as "something changed — re-check", coalesces the burst, takes
one fresh [`RepoSnapshot`](https://docs.rs/vcs-core/latest/vcs_core/guide/) (+ the branch list), and diffs.
Noise that doesn't change observable state produces **no** event. This also means
a stray event can't desync the consumer — every emission carries the true current
state.

## Events

[`RepoEvent`] (`#[non_exhaustive]`), derived by diffing two snapshots:

| Event | Fires when |
|---|---|
| `HeadMoved { from, to }` | the working-copy commit id changed (commit, checkout, reset, jj op) |
| `BranchSwitched { from, to }` | the *current* branch/bookmark changed (or detached → `None`) |
| `BranchCreated { name }` / `BranchDeleted { name }` | a local branch/bookmark appeared / was removed |
| `WorkingCopyChanged { dirty, change_count }` | dirtiness or the changed-path *count* moved |
| `UpstreamChanged { upstream }` | the upstream tracking branch changed (git only) |
| `AheadBehindChanged { ahead, behind }` | ahead/behind vs upstream changed (git only) |
| `OperationChanged { from, to }` | a git merge/rebase started or finished (**git only**) |
| `ConflictChanged { conflicted }` | the unresolved-conflict flag toggled (both backends) |

Two semantics worth knowing:

- **Conflicts → `ConflictChanged`, on both backends.** `OperationChanged` covers
  only git's merge/rebase/am lifecycle (`Clear`/`Merge`/`Rebase`/`ApplyMailbox`); it
  never fires on jj. `vcs-core` derives jj's `operation` and `conflicted` from the same bit, so a
  jj conflict appearing would otherwise double-signal — the redundant
  `OperationChanged` is suppressed, and `ConflictChanged` is the one true conflict
  event everywhere. (A git merge that *has* conflicts is two distinct facts and
  fires both `OperationChanged` and `ConflictChanged`.)
- **`WorkingCopyChanged` is dirty-flag + path *count*, not file identity.**
  Swapping *which* file is edited while the count stays the same emits **nothing**
  (the status-line count is unchanged anyway). A consumer that needs the file set
  reads `change.snapshot` / calls `Repo::changed_files()`.

Each settled change arrives as a [`RepoChange`] `{ snapshot: RepoSnapshot, events:
Vec<RepoEvent> }` — `events` is never empty, and the events come in a stable order
(head, branch switch, created, deleted, working copy, upstream, ahead/behind,
operation, conflict; created/deleted names sorted).

## Building the watcher

```rust,ignore
# use std::time::Duration;
# use vcs_core::Repo;
# use vcs_watch::RepoWatcher;
# async fn demo(repo: Repo) -> vcs_watch::Result<()> {
let watcher = RepoWatcher::builder(repo)
    .working_tree(true)                       // also watch the working tree
    .debounce(Duration::from_millis(150))     // quiet window (default 250 ms)
    .max_wait(Duration::from_secs(2))         // re-query ceiling (default 1 s)
    .requery_timeout(Some(Duration::from_secs(10))) // per-query deadline (default 30 s)
    .build()
    .await?;
# let _ = watcher; Ok(()) }
```

Two **orthogonal** timing knobs are easy to confuse: `max_wait` bounds how long
a continuous event stream may *defer* a re-query (cadence under load);
`requery_timeout` bounds how long one re-query may *run* — a wedged command
(say, a held `index.lock` on a client with no timeout of its own) is killed and
skipped as transient instead of stalling the watch forever
(`requery_timeout(None)` disables it). `requery_timeout` **also bounds the startup
baseline**: if capturing it exceeds the deadline, `build()` returns a *transient*
`Io` `TimedOut` (`err.is_transient()`) instead of hanging — so a wedged repo can't
stall startup any more than it can stall the loop.

- **`recv().await -> Option<RepoChange>`** — the next settled change; `None` once
  the watcher is dropped. `current() -> &RepoSnapshot` is the last known state —
  the build-time baseline, advanced **only when you pull a change** (via `recv`
  or the stream — it is as fresh as your last pull, not a live view).
- **`stats()`** — lock-free health counters: re-queries run, changes emitted,
  skips (transient failures + deadline overruns) and what the last skip failed
  on. A climbing `skipped` with flat `requeries` means the repository is wedged —
  poll it from a health check instead of inferring health from event silence.
- **The `stream` feature** adds `impl futures_core::Stream for RepoWatcher`, so
  the watcher drops into `tokio::select!`/stream combinators directly. `recv()`
  and the stream pull from the **same** channel — an item is delivered to
  whichever is polled first, never duplicated — and both advance `current()`.
- **Drop stops everything** — dropping the `RepoWatcher` ends the OS watch and the
  background task.

### Watch scope — state dir vs working tree

By default the watcher monitors only the **state directory** (`.git`/`.jj`):
HEAD, refs, the index, packed-refs, merge/rebase markers, the jj op log. This is
cheap and robust, and catches structural changes plus anything that touches the
index (staging, commit) or a jj snapshot. A **bare unstaged edit** (`vim file`
with no `git add`) doesn't touch the state dir, so it's seen only once it's staged
— unless you opt into **`working_tree(true)`**, which also watches the working
tree recursively and fires `WorkingCopyChanged` immediately. The trade-off:
working-tree watching is `.gitignore`-unaware (it also watches `target/` etc.) and
heavier on a large repo.

## Backends, colocation, worktrees

The backend (and which dir to watch) comes from `vcs-core`'s pure `discover`: `.jj`
for jj, `.git` for git, and **jj wins when colocated** — so a colocated repo is
watched via `.jj` (jj drives; its op-log change is the signal). A linked
worktree's `.git` is a gitlink *file*; the watcher resolves it to that worktree's
private git directory (HEAD/index) **and** — via its `commondir` file — to the
shared main `.git`, where branch refs (`refs/heads/*`, `packed-refs`) actually
live. Both are watched, so `BranchCreated`/`BranchDeleted` made from any checkout
are observed from a watched worktree too.

## Semantics & limits

- **Transient re-query failures are skipped, not surfaced.** A snapshot taken
  while an operation holds `index.lock` may fail (or overrun `requery_timeout`);
  the watcher skips that re-check and the next event re-queries the settled
  state. Setup failures (the watch can't start) surface from `build()`. Skips
  are **counted** — `stats()` reports them (with the failure kind) — and the
  `tracing` feature adds a debug line on each.
- **Runtime.** Unlike the rest of the toolkit, `vcs-watch` uses **tokio at
  runtime** (the watch task + debounce timer). Build/await it inside a tokio
  runtime.

## See also

- [vcs-core guide](https://docs.rs/vcs-core/latest/vcs_core/guide/) — the `Repo`/`RepoSnapshot` it re-queries.
- [Cookbook](https://docs.rs/vcs-core/latest/vcs_core/guide/cookbook/) — a live status-line recipe.
- [crate docs](https://docs.rs/vcs-watch) — quickstart.
