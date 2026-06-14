# vcs-jj — automate Jujutsu from Rust

Part of the [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.

**What you can do:** working-copy status & the change log, describe/new change,
bookmarks, the operation log (restore/undo), workspaces, squash/split/absorb/
duplicate/abandon, diff & template queries, git sync (fetch/push/clone/import),
parse & resolve jj's native conflict markers, and transactions that roll the op log
back on error — all as typed, repo-scoped `async` methods over the `jj` binary,
behind a mockable interface.

**How it works:** each call runs the real `jj` (its exact behaviour and config) and
parses the templated output into typed values. Commands run inside an OS job (an
OS-level container that kills the whole process tree if your program exits, via
[`processkit`]) so no `jj` subprocess is ever orphaned; calls return the structured
`Error` and honour an optional timeout.

[`processkit`]: https://crates.io/crates/processkit

> 📖 **Full guide:** [on docs.rs](https://docs.rs/vcs-jj/latest/vcs_jj/guide/)
> — every command by theme, result types, builder/newtype APIs, and worked examples.

Every method is `async`, so call it from a tokio runtime:

```rust
use std::path::Path;
use vcs_jj::{Jj, JjApi};

let jj = Jj::new();
let head = jj.current_change(Path::new(".")).await?; // Change
jj.describe(Path::new("."), "feat: new thing").await?; // set @ description
```

### A change workflow

```rust
use std::path::Path;
use vcs_jj::{Jj, JjApi};

# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
    let jj = Jj::new();

    // Describe the working-copy change, then start a fresh one on top.
    jj.describe(repo, "feat: parser").await?;
    jj.new_change(repo, "wip: follow-up").await?;

    let head = jj.current_change(repo).await?; // Change { change_id, commit_id, empty, description }
    println!("@ = {} ({})", head.change_id, head.description);

    // Everything reachable from @, newest first.
    for c in jj.log(repo, "::@", 10).await? {
        println!(
            "{} {}{}",
            c.change_id,
            if c.empty { "(empty) " } else { "" },
            c.description
        );
    }
# Ok(()) }
```

### Bookmarks and syncing the git remote

```rust
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
    let jj = Jj::new();

    jj.git_fetch(repo).await?; // `jj git fetch`
    jj.bookmark_set(repo, "main", "@").await?; // point `main` at @
    for b in jj.bookmarks(repo).await? {
        println!("{} -> {}", b.name, b.target);
    }
    jj.git_push(repo, Some("main".to_string())).await?; // `jj git push -b main`
# Ok(()) }
```

### Workspaces

Manage workspaces (jj's worktrees) with structured results:

```rust
use vcs_jj::{Jj, JjApi, WorkspaceAdd};
use std::path::Path;

# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
let jj = Jj::new();

jj.workspace_add(repo, WorkspaceAdd::new("feature", "@", "/tmp/feature"))
    .await?;

for ws in jj.workspace_list(repo).await? {            // Vec<Workspace>
    println!("{} @ {} {:?}", ws.name, ws.commit, ws.bookmarks);
}

jj.workspace_forget(repo, "feature").await?;
# Ok(()) }
```

### Timeouts

```rust
# use vcs_jj::Jj;
use std::time::Duration;
let jj = Jj::new().default_timeout(Duration::from_secs(10));
// every command now fails with `processkit::Error::Timeout` if it outruns 10s
# let _ = jj;
```

Consumers depend on the `JjApi` trait and substitute a fake in tests — enable
the `mock` feature for a `mockall`-generated `MockJjApi`, or inject a fake
process runner with `Jj::with_runner(processkit::testing::ScriptedRunner::new()…)`:

```rust
use processkit::testing::{Reply, ScriptedRunner};
use std::path::Path;
use vcs_jj::{Jj, JjApi};

# async fn demo() {
    let jj = Jj::with_runner(
        ScriptedRunner::new().on(["jj", "log"], Reply::ok("kztuxlro\t38e00654\tfalse\thello\n")),
    );
    assert_eq!(
        jj.current_change(Path::new(".")).await.unwrap().description,
        "hello"
    );
# }
```

Requires the `jj` binary on `PATH`.

## License

MIT
