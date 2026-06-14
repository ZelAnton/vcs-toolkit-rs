# vcs-watch

[![crates.io](https://img.shields.io/crates/v/vcs-watch.svg)](https://crates.io/crates/vcs-watch) [![docs.rs](https://img.shields.io/docsrs/vcs-watch)](https://docs.rs/vcs-watch) [![downloads](https://img.shields.io/crates/d/vcs-watch.svg)](https://crates.io/crates/vcs-watch)

Filesystem-watch a **git or jj** repository and stream **typed state-change
events**. Part of the [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs)
workspace.

Built on [`vcs-core`](https://crates.io/crates/vcs-core): on each filesystem
change `vcs-watch` debounces the burst, **re-queries** the repo's batched snapshot,
and **diffs** it against the previous state — so noise (git's ref temp-renames,
`index.lock`, reflog churn) coalesces into one re-check instead of being mis-read
as events. The foundation for prompts, status bars, TUIs, and daemons.

> 📖 **Full guide:** [on docs.rs](https://docs.rs/vcs-watch/latest/vcs_watch/guide/)

```rust
use vcs_core::Repo;
use vcs_watch::{RepoWatcher, RepoEvent};

# async fn demo() -> vcs_watch::Result<()> {
let repo = Repo::open(".")?;
let mut watcher = RepoWatcher::watch(repo).await?;
while let Some(change) = watcher.recv().await {
    for event in &change.events {
        println!("{event:?}");          // HeadMoved / BranchCreated / WorkingCopyChanged / …
    }
    // change.snapshot is the fresh full RepoSnapshot — render a status line from it.
}
# Ok(()) }
```

Each settled change is a `RepoChange { snapshot, events }` — the new full state
*and* the typed deltas. Configure the scope and timing with the builder:

```rust
# use std::time::Duration;
# use vcs_core::Repo;
# use vcs_watch::RepoWatcher;
# async fn demo(repo: Repo) -> vcs_watch::Result<()> {
let watcher = RepoWatcher::builder(repo)
    .working_tree(true)                    // also catch bare unstaged edits
    .debounce(Duration::from_millis(150))
    .build()
    .await?;
# let _ = watcher; Ok(()) }
```

**Runtime:** unlike the rest of the toolkit (which hides tokio behind
`processkit`), `vcs-watch` uses **tokio at runtime** — build and await it inside a
tokio runtime. It watches `.git`/`.jj` (jj wins when colocated) by default;
`working_tree(true)` adds the working tree.

## License

MIT
