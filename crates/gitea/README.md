# vcs-gitea — automate Gitea (and Forgejo) from Rust

[![crates.io](https://img.shields.io/crates/v/vcs-gitea.svg)](https://crates.io/crates/vcs-gitea) [![docs.rs](https://img.shields.io/docsrs/vcs-gitea)](https://docs.rs/vcs-gitea) [![downloads](https://img.shields.io/crates/d/vcs-gitea.svg)](https://crates.io/crates/vcs-gitea)

Part of the [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.

**What you can do:** check auth, run the lean pull-request lifecycle (list/view/
create/merge/close), manage issues (list/view/create), and list releases — all as
typed `async` methods over the `tea` CLI, behind a mockable interface.

**How it works:** each call runs the real `tea` (its own auth, config and instance
handling), asks for `--output json`, and deserializes the result into structs.
Commands run inside an OS job (an OS-level container that kills the whole process
tree if your program exits, via [`processkit`]) so no `tea` subprocess is ever
orphaned; calls return the structured `Error` and honour an optional timeout. The
[`vcs-forge`](https://crates.io/crates/vcs-forge) facade unifies this with
`vcs-github` and `vcs-gitlab`.

[`processkit`]: https://crates.io/crates/processkit

> 📖 **Full guide:** [on docs.rs](https://docs.rs/vcs-gitea/latest/vcs_gitea/guide/)

Every method is `async`, so call it from a tokio runtime:

```rust
use std::path::Path;
use vcs_gitea::{Gitea, GiteaApi};

let tea = Gitea::new();
let prs = tea.pr_list(Path::new(".")).await?; // Vec<PullRequest>
let authed = tea.auth_status().await?; // bool — true when a login is configured
```

> **Narrower than `gh`/`glab`.** `tea` has no single-PR view (this crate synthesizes
> `pr_view` by listing and filtering), no current-repo view, no draft toggle, and no
> PR-checks command — those operations are absent here (the `vcs-forge` facade reports
> them `Unsupported` for the Gitea backend). Its `--output json` is tea's print-table
> for lists (a typed object only for the issue detail view), **not** the Gitea REST
> shape.

### Open and merge a pull request

```rust
use std::path::Path;
use vcs_gitea::{Gitea, GiteaApi, MergeStrategy, PrCreate};

# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
    let tea = Gitea::new();

    for pr in tea.pr_list(repo).await? {
        println!("#{} [{}] {} — {}", pr.number, pr.state, pr.title, pr.url);
    }

    let out = tea
        .pr_create(
            repo,
            PrCreate::new("Add streaming", "Implements …")
                .head("feat/streaming")
                .base("main"),
        )
        .await?;
    println!("{out}");

    tea.pr_merge(repo, 7, MergeStrategy::Squash).await?;
# Ok(()) }
```

Consumers depend on the `GiteaApi` trait and substitute a fake in tests — enable
the `mock` feature for a `mockall`-generated `MockGiteaApi`, or inject a fake
process runner with `Gitea::with_runner(processkit::testing::ScriptedRunner::new()…)`:

```rust
use processkit::testing::{Reply, ScriptedRunner};
use std::path::Path;
use vcs_gitea::{Gitea, GiteaApi};

# async fn demo() {
    let json = r#"[{"number":7,"title":"Add X","state":"open"}]"#;
    let tea = Gitea::with_runner(ScriptedRunner::new().on(["tea", "pr", "list"], Reply::ok(json)));
    assert_eq!(tea.pr_list(Path::new(".")).await.unwrap()[0].number, 7);
# }
```

Requires the `tea` binary on `PATH` (configured via `tea login add`).

## License

MIT
