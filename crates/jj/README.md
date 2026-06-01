# vcs-jj

Automate the **Jujutsu** (`jj`) CLI from Rust through process execution. Part of
the [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.

Typed, repo-scoped, **async** commands over the `jj` binary, behind a **mockable
interface**. Commands run inside an OS job (via [`processkit`]) so no `jj`
subprocess is ever orphaned, return the structured `Error`, and honour an
optional timeout.

[`processkit`]: https://crates.io/crates/processkit

Inside an async context (every method is `async`):

```rust
use vcs_jj::{Jj, JjApi};
use std::path::Path;

let jj = Jj::new();
let head = jj.current_change(Path::new(".")).await?;   // Change
jj.describe(Path::new("."), "feat: new thing").await?; // set @ description
```

### A change workflow

```rust
use vcs_jj::{Jj, JjApi};
use std::path::Path;

# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
let jj = Jj::new();

// Describe the working-copy change, then start a fresh one on top.
jj.describe(repo, "feat: parser").await?;
jj.new_change(repo, "wip: follow-up").await?;

let head = jj.current_change(repo).await?;             // Change { change_id, commit_id, empty, description }
println!("@ = {} ({})", head.change_id, head.description);

// Everything reachable from @, newest first.
for c in jj.log(repo, "::@", 10).await? {
    println!("{} {}{}", c.change_id, if c.empty { "(empty) " } else { "" }, c.description);
}
# Ok(()) }
```

### Bookmarks and syncing the git remote

```rust
# use vcs_jj::{Jj, JjApi};
# use std::path::Path;
# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
let jj = Jj::new();

jj.git_fetch(repo).await?;                             // `jj git fetch`
jj.bookmark_set(repo, "main", "@").await?;             // point `main` at @
for b in jj.bookmarks(repo).await? {
    println!("{} -> {}", b.name, b.target);
}
jj.git_push(repo, Some("main".to_string())).await?;    // `jj git push -b main`
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
process runner with `Jj::with_runner(processkit::ScriptedRunner::new()…)`:

```rust
use vcs_jj::{Jj, JjApi};
use processkit::{Reply, ScriptedRunner};
use std::path::Path;

# async fn demo() {
let jj = Jj::with_runner(
    ScriptedRunner::new().on(["log"], Reply::ok("kztuxlro\t38e00654\tfalse\thello\n")),
);
assert_eq!(jj.current_change(Path::new(".")).await.unwrap().description, "hello");
# }
```

Requires the `jj` binary on `PATH`.

## License

MIT
