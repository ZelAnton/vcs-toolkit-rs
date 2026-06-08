# vcs-gitlab

Automate **GitLab** from Rust through the `glab` CLI and process execution. Part
of the [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.

Typed, **async** commands over the GitLab CLI (`glab`) that deserialize
`glab … --output json` (GitLab's REST JSON) into structs, behind a **mockable
interface**. Commands run inside an OS job (via [`processkit`]) so no `glab`
subprocess is ever orphaned, return the structured `Error`, and honour an
optional timeout. The surface is the **lean merge-request lifecycle**; the
[`vcs-forge`](https://crates.io/crates/vcs-forge) facade unifies this with
`vcs-github` and `vcs-gitea`.

[`processkit`]: https://crates.io/crates/processkit

> 📖 **Full guide:** [on docs.rs](https://docs.rs/vcs-gitlab/latest/vcs_gitlab/guide/)

Inside an async context (every method is `async`):

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
process runner with `GitLab::with_runner(processkit::ScriptedRunner::new()…)`:

```rust
use processkit::{Reply, ScriptedRunner};
use std::path::Path;
use vcs_gitlab::{GitLab, GitLabApi};

# async fn demo() {
    let json = r#"[{"iid":7,"title":"Add X","state":"opened"}]"#;
    let glab = GitLab::with_runner(ScriptedRunner::new().on(["mr", "list"], Reply::ok(json)));
    assert_eq!(glab.mr_list(Path::new(".")).await.unwrap()[0].iid, 7);
# }
```

Requires the `glab` binary on `PATH` (authenticated via `glab auth login`).

## License

MIT
