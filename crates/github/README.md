# vcs-github

Automate **GitHub** from Rust through the `gh` CLI and process execution. Part of
the [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.

Typed, **async** commands over the GitHub CLI (`gh`) that deserialize
`gh … --json` output into structs, behind a **mockable interface**. Commands run
inside an OS job (via [`processkit`]) so no `gh` subprocess is ever orphaned,
return the structured `Error`, and honour an optional timeout.

[`processkit`]: https://crates.io/crates/processkit

Inside an async context (every method is `async`):

```rust
use vcs_github::{GitHub, GitHubApi};
use std::path::Path;

let gh = GitHub::new();
let prs = gh.pr_list(Path::new(".")).await?;   // Vec<PullRequest>
let authed = gh.auth_status().await?;          // bool — true when `gh auth status` exits 0
```

### Inspect the repo and open a PR

```rust
use vcs_github::{GitHub, GitHubApi};
use std::path::Path;

# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
let gh = GitHub::new();

let r = gh.repo_view(repo).await?;             // Repo { owner, name, default_branch, is_private, … }
println!("{}/{} (default: {})", r.owner, r.name, r.default_branch);

// Open a PR against an explicit base; returns the new PR's URL.
let url = gh
    .pr_create(repo, "Add streaming", "Implements …", Some("main".to_string()))
    .await?;
println!("opened {url}");

for issue in gh.issue_list(repo).await? {
    println!("#{} [{}] {}", issue.number, issue.state, issue.title);
}
# Ok(()) }
```

### `auth_status` and timeouts

`auth_status` reports the bool from `gh auth status`'s exit code, but a spawn
failure or a timeout still surfaces as a `processkit::Error` rather than a
silent `false`:

```rust
# use vcs_github::{GitHub, GitHubApi};
use std::time::Duration;
# async fn demo() -> Result<(), processkit::Error> {
let gh = GitHub::new().default_timeout(Duration::from_secs(5));
match gh.auth_status().await {
    Ok(true) => println!("authenticated"),
    Ok(false) => println!("not logged in (run `gh auth login`)"),
    Err(processkit::Error::Timeout { .. }) => eprintln!("gh timed out"),
    Err(e) => eprintln!("{e}"),
}
# Ok(()) }
```

Consumers depend on the `GitHubApi` trait and substitute a fake in tests — enable
the `mock` feature for a `mockall`-generated `MockGitHubApi`, or inject a fake
process runner with `GitHub::with_runner(processkit::ScriptedRunner::new()…)`:

```rust
use vcs_github::{GitHub, GitHubApi};
use processkit::{Reply, ScriptedRunner};
use std::path::Path;

# async fn demo() {
let json = r#"[{"number":7,"title":"Add X","state":"OPEN"}]"#;
let gh = GitHub::with_runner(ScriptedRunner::new().on(["pr", "list"], Reply::ok(json)));
assert_eq!(gh.pr_list(Path::new(".")).await.unwrap()[0].number, 7);
# }
```

Requires the `gh` binary on `PATH` (authenticated via `gh auth login`).

## License

MIT
