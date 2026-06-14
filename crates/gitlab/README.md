# vcs-gitlab — automate GitLab from Rust

Part of the [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.

**What you can do:** check auth, view the project, run the lean merge-request
lifecycle (list/view/create/merge/mark-ready/close), read CI/pipeline status, and
manage issues and releases — all as typed `async` methods over the `glab` CLI,
behind a mockable interface.

**How it works:** each call runs the real `glab` (its own host config and
credentials), asks for `--output json`, and deserializes the result into structs.
Commands run inside an OS job (an OS-level container that kills the whole process
tree if your program exits, via [`processkit`]) so no `glab` subprocess is ever
orphaned; calls return the structured `Error` and honour an optional timeout. The
[`vcs-forge`](https://crates.io/crates/vcs-forge) facade unifies this with
`vcs-github` and `vcs-gitea`.

[`processkit`]: https://crates.io/crates/processkit

> 📖 **Full guide:** [on docs.rs](https://docs.rs/vcs-gitlab/latest/vcs_gitlab/guide/)

Every method is `async`, so call it from a tokio runtime:

```rust
use std::path::Path;
use vcs_gitlab::{GitLab, GitLabApi};

let glab = GitLab::new();
let mrs = glab.mr_list(Path::new(".")).await?; // Vec<MergeRequest>
let authed = glab.auth_status().await?; // bool — true when `glab auth status` exits 0
```

### Inspect the project and open a merge request

```rust
use std::path::Path;
use vcs_gitlab::{GitLab, GitLabApi, MrCreate};

# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
    let glab = GitLab::new();

    let p = glab.repo_view(repo).await?; // Project { path_with_namespace, default_branch, … }
    println!("{} (default: {})", p.path_with_namespace, p.default_branch);

    for mr in glab.mr_list(repo).await? {
        println!("!{} [{}] {} — {}", mr.iid, mr.state, mr.title, mr.web_url);
    }

    // Open an MR from an explicit source into an explicit target (both optional —
    // omit `.source(…)` for the current branch, `.target(…)` for the project default).
    let url = glab
        .mr_create(
            repo,
            MrCreate::new("Add streaming", "Implements …")
                .source("feat/streaming")
                .target("main"),
        )
        .await?;
    println!("opened {url}");
# Ok(()) }
```

Consumers depend on the `GitLabApi` trait and substitute a fake in tests — enable
the `mock` feature for a `mockall`-generated `MockGitLabApi`, or inject a fake
process runner with `GitLab::with_runner(processkit::testing::ScriptedRunner::new()…)`:

```rust
use processkit::testing::{Reply, ScriptedRunner};
use std::path::Path;
use vcs_gitlab::{GitLab, GitLabApi};

# async fn demo() {
    let json = r#"[{"iid":7,"title":"Add X","state":"opened"}]"#;
    let glab = GitLab::with_runner(ScriptedRunner::new().on(["glab", "mr", "list"], Reply::ok(json)));
    assert_eq!(glab.mr_list(Path::new(".")).await.unwrap()[0].iid, 7);
# }
```

Requires the `glab` binary on `PATH` (authenticated via `glab auth login`).

## License

MIT
