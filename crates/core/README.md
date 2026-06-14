# vcs-core — one repository API for git and jj

[![crates.io](https://img.shields.io/crates/v/vcs-core.svg)](https://crates.io/crates/vcs-core) [![docs.rs](https://img.shields.io/docsrs/vcs-core)](https://docs.rs/vcs-core) [![downloads](https://img.shields.io/crates/d/vcs-core.svg)](https://crates.io/crates/vcs-core)

Part of the [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.

**What you can do:** hold one `Repo` handle that auto-detects whether a directory is
a git or a jj checkout, then run whatever *both* tools support — current branch, a
batched status snapshot, changed files & diff, commit paths, fetch/push/checkout/
rebase, a conflict-probe merge, in-progress merge/rebase state, and worktrees — all
returning plain result types that don't mention the backend.

**How it works:** it drives the `vcs-git` / `vcs-jj` clients under the hood and
exposes only the shared operations. Tool-specific power (a full merge, jj's
`op restore`, range/revset queries) stays on the raw client, reachable via
`Repo::git()` / `Repo::jj()`.

> 📖 **Full guide:** [on docs.rs](https://docs.rs/vcs-core/latest/vcs_core/guide/)
> — detection, the unified facade surface, the DTOs, and when to drop to the raw client.

## What it gives you

- **`detect(dir) -> Option<Located>`** — walk up from `dir` to find a `.git`/`.jj`
  repository. A `.jj` directory wins over `.git` (colocated repos are driven
  through jj). Pure filesystem probing, no subprocess.
- **`Repo`** — a cwd-bound handle. Open it once, then call the common surface
  without threading a directory through every call:

```rust,no_run
use vcs_core::Repo;

# fn main() -> vcs_core::Result<()> {
let repo = Repo::open(".")?;
println!("backend: {}", repo.kind().as_str());
# Ok(())
# }
```

## Common surface

`current_branch`, `trunk`, `changed_files`, `diff_stat`, `commit_paths`,
`fetch`, `push`, `list_worktrees`, `create_worktree`, `remove_worktree` — each
returning plain result types (`FileChange`, `DiffStat`, `WorktreeInfo`,
`RepoSnapshot` with its bundled `tracking`, …) that don't mention git or jj.
Re-anchor a handle to a sibling directory with `repo.at(other_dir)`.

The error type wraps `processkit::Error` (re-exported as `vcs_core::processkit`, so
you can match it without a direct dependency) and carries intent classifiers —
`is_merge_conflict` / `is_nothing_to_commit` / `is_transient_fetch_error` /
`is_transient` / `is_not_found`.

## Testing

`Repo` is generic over `processkit::ProcessRunner`. Build one from an explicit
client (`Repo::from_git` / `Repo::from_jj`) with a `ScriptedRunner` to test
dispatch hermetically, exactly as the underlying crates do.
