# vcs-forge — one forge API for GitHub, GitLab and Gitea

[![crates.io](https://img.shields.io/crates/v/vcs-forge.svg)](https://crates.io/crates/vcs-forge) [![docs.rs](https://img.shields.io/docsrs/vcs-forge)](https://docs.rs/vcs-forge) [![downloads](https://img.shields.io/crates/d/vcs-forge.svg)](https://crates.io/crates/vcs-forge)

Part of the [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.

**What you can do:** hold one `Forge` handle and automate all three forges through one
API — check auth, view the repo/project, the PR/MR lifecycle (list/view/create/merge/
close, plus mark-ready and CI checks on GitHub/GitLab), issues (list/view/create), and
releases (list/view) — all returning plain result types (`ForgePr`, `ForgeIssue`,
`ForgeRelease`, `ForgeRepo`, `CiStatus`) that don't mention which forge produced them.
(Gitea's `tea` is narrower — a few operations are GitHub/GitLab-only; see "Coverage
differs per CLI" below.)

**How it works:** it sends each operation to whichever CLI (`gh`/`glab`/`tea`) backs
the handle. It's the forge analogue of how
[`vcs-core`](https://crates.io/crates/vcs-core) sits over git and jj.

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
(`Forge::from_github("/repo", GitHub::with_runner(runner))`).

## License

MIT
