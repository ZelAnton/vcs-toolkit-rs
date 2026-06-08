# vcs-forge

A backend-agnostic **forge facade** over `vcs-github`, `vcs-gitlab`, and
`vcs-gitea` — the `gh`/`glab`/`tea` analogue of how
[`vcs-core`](https://crates.io/crates/vcs-core) sits over git and jj. Part of the
[vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.

A [`Forge`] handle dispatches the **common forge operations** — auth, repo/project
view, the PR/MR lifecycle (list / view / create / merge / mark-ready / close,
plus CI status), issues (list / view / create), and releases (list / view) — to
whichever CLI backs it, returning forge-agnostic DTOs ([`ForgePr`],
[`ForgeIssue`], [`ForgeRelease`], [`ForgeRepo`], [`CiStatus`]).

> 📖 **Full guide:** [on docs.rs](https://docs.rs/vcs-forge/latest/vcs_forge/guide/)

A forge has **no filesystem marker** (it's the remote host), so a `Forge` is
constructed explicitly — optionally guided by `ForgeKind::from_remote_url` on a
remote URL you already hold:

```rust
use vcs_forge::{Forge, ForgeApi, ForgeKind, MergeStrategy};

# async fn demo() -> vcs_forge::Result<()> {
    // Explicit, or sniffed from a remote URL:
    let forge = match ForgeKind::from_remote_url("git@gitlab.com:o/r.git") {
        Some(ForgeKind::GitLab) => Forge::gitlab("."),
        Some(ForgeKind::Gitea)  => Forge::gitea("."),
        _                       => Forge::github("."),
    };

    for pr in forge.pr_list().await? {
        println!("#{} [{:?}] {} — {}", pr.number, pr.state, pr.title, pr.url);
    }
    forge.pr_merge(7, MergeStrategy::Squash).await?;
# Ok(()) }
```

## Coverage differs per CLI

Gitea's `tea` has no current-repo view, draft toggle, checks command, or
single-release view, so `repo_view`, `pr_mark_ready`, `pr_checks`, and
`release_view` return `Error::Unsupported` for the Gitea backend
(`err.is_unsupported()`). GitHub and GitLab support the full lean surface.

Consumers can code against the object-safe `ForgeApi` trait (`&dyn ForgeApi`), and
build a `Forge` over a fake runner for hermetic tests
(`Forge::for_github("/repo", GitHub::with_runner(runner))`).

## License

MIT
