# vcs-github — automate GitHub from Rust

[![crates.io](https://img.shields.io/crates/v/vcs-github.svg)](https://crates.io/crates/vcs-github) [![docs.rs](https://img.shields.io/docsrs/vcs-github)](https://docs.rs/vcs-github) [![downloads](https://img.shields.io/crates/d/vcs-github.svg)](https://crates.io/crates/vcs-github)

Part of the [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.

**What you can do:** check auth, view the repo, run the full pull-request lifecycle
(list/view/create/merge/mark-ready/close, review/comment, CI checks, feedback),
manage issues and releases, and list/view/watch GitHub Actions runs — all as typed
`async` methods over the `gh` CLI, behind a mockable interface.

**How it works:** each call runs the real `gh` (its own auth and host resolution)
and deserializes its `--json` output into structs — nothing scrapes human-readable
text. Commands run inside an OS job (an OS-level container that kills the whole
process tree if your program exits, via [`processkit`]) so no `gh` subprocess is
ever orphaned; calls return the structured `Error` and honour an optional timeout.

**Credentials:** `gh`'s ambient login by default; to supply a token per operation
(CI, vault, multi-account), the one-liner is `GitHub::new().with_token(tok)` (or
`.with_env_token("MY_TOKEN")`); for full control attach a `CredentialProvider` with
`.with_credentials(...)`. Either way the token is injected as `GH_TOKEN`, kept out
of `argv`.

[`processkit`]: https://crates.io/crates/processkit

> 📖 **Full guide:** [on docs.rs](https://docs.rs/vcs-github/latest/vcs_github/guide/)
> — every command by theme, result types, config types, and worked examples.

Every method is `async`, so call it from a tokio runtime:

```rust
use std::path::Path;
use vcs_github::{GitHub, GitHubApi};

let gh = GitHub::new();
let prs = gh.pr_list(Path::new(".")).await?; // Vec<PullRequest>
let authed = gh.auth_status().await?; // bool — true when `gh auth status` exits 0
```

### Inspect the repo and open a PR

```rust
use std::path::Path;
use vcs_github::{GitHub, GitHubApi, PrCreate};

# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
    let gh = GitHub::new();

    let r = gh.repo_view(repo).await?; // Repo { owner, name, default_branch, is_private, … }
    println!("{}/{} (default: {})", r.owner, r.name, r.default_branch);

    // Any PRs (open/closed/merged) merging this branch into main? (title + URL)
    for pr in gh.pr_list_for_branch(repo, "feat/streaming", "main").await? {
        println!("#{} [{}] {} — {}", pr.number, pr.state, pr.title, pr.url);
    }

    // Open a PR with `PrCreate`. `head`/`base` are optional — omit them for the
    // current branch / repo default. Returns the URL.
    let url = gh
        .pr_create(
            repo,
            PrCreate::new("Add streaming", "Implements …")
                .head("feat/streaming")
                .base("main"),
        )
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
process runner with `GitHub::with_runner(processkit::testing::ScriptedRunner::new()…)`:

```rust
use processkit::testing::{Reply, ScriptedRunner};
use std::path::Path;
use vcs_github::{GitHub, GitHubApi};

# async fn demo() {
    let json = r#"[{"number":7,"title":"Add X","state":"OPEN"}]"#;
    let gh = GitHub::with_runner(ScriptedRunner::new().on(["gh", "pr", "list"], Reply::ok(json)));
    assert_eq!(gh.pr_list(Path::new(".")).await.unwrap()[0].number, 7);
# }
```

Requires the `gh` binary on `PATH` (authenticated via `gh auth login`).

## License

MIT
