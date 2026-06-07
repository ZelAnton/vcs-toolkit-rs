# vcs-gitea

Automate **Gitea** (and Forgejo) from Rust through the `tea` CLI and process
execution. Part of the [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs)
workspace.

Typed, **async** commands over the Gitea CLI (`tea`) that deserialize
`tea … --output json` (tea's print-table for lists, a typed object for the issue
detail view — **not** the Gitea REST shape) into structs, behind a **mockable
interface**. Commands run inside an OS job (via [`processkit`]) so no
`tea` subprocess is ever orphaned, return the structured `Error`, and honour an
optional timeout. The [`vcs-forge`](https://crates.io/crates/vcs-forge) facade
unifies this with `vcs-github` and `vcs-gitlab`.

[`processkit`]: https://crates.io/crates/processkit

> 📖 **Full guide:** [docs/gitea.md](https://github.com/ZelAnton/vcs-toolkit-rs/blob/main/docs/gitea.md)

`tea`'s surface is narrower than `gh`/`glab`: it has **no** single-PR view (this
crate synthesizes `pr_view` by listing and filtering), **no** current-repo view,
**no** draft toggle, and **no** PR-checks command — so those operations are absent
here (the facade reports them as `Unsupported` for the Gitea backend).

Inside an async context (every method is `async`):

```rust
use std::path::Path;
use vcs_gitea::{Gitea, GiteaApi};

let tea = Gitea::new();
let prs = tea.pr_list(Path::new(".")).await?; // Vec<PullRequest>
let authed = tea.auth_status().await?; // bool — true when a login is configured
```

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
process runner with `Gitea::with_runner(processkit::ScriptedRunner::new()…)`:

```rust
use processkit::{Reply, ScriptedRunner};
use std::path::Path;
use vcs_gitea::{Gitea, GiteaApi};

# async fn demo() {
    let json = r#"[{"number":7,"title":"Add X","state":"open"}]"#;
    let tea = Gitea::with_runner(ScriptedRunner::new().on(["pr", "list"], Reply::ok(json)));
    assert_eq!(tea.pr_list(Path::new(".")).await.unwrap()[0].number, 7);
# }
```

Requires the `tea` binary on `PATH` (configured via `tea login add`).

## License

MIT
