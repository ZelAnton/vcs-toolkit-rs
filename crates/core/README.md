# vcs-core

A unified facade over [`vcs-git`](https://crates.io/crates/vcs-git) and
[`vcs-jj`](https://crates.io/crates/vcs-jj): repository detection plus a
backend-agnostic handle for the operations both tools share.

It exists to lift the "detect git-vs-jj and dispatch behind one interface" layer
that downstream tools kept re-implementing. Rich, tool-specific operations stay
on the underlying `vcs-git` / `vcs-jj` clients, reachable through the
`Repo::git()` / `Repo::jj()` escape hatches.

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
returning backend-agnostic DTOs (`FileChange`, `DiffStat`, `WorktreeInfo`, …).
Re-anchor a handle to a sibling directory with `repo.at(other_dir)`.

## Testing

`Repo` is generic over `processkit::ProcessRunner`. Build one from an explicit
client (`Repo::from_git` / `Repo::from_jj`) with a `ScriptedRunner` to test
dispatch hermetically, exactly as the underlying crates do.
