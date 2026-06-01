# vcs-git

Automate the **Git** CLI from Rust through process execution. Part of the
[vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.

Typed, repo-scoped, **async** commands over the `git` binary, behind a
**mockable interface**. Commands run inside an OS job (via [`processkit`]) so no
`git` subprocess is ever orphaned, return the structured `Error`, and honour an
optional timeout.

[`processkit`]: https://crates.io/crates/processkit

Inside an async context (every method is `async`):

```rust
use std::path::Path;
use vcs_git::{Git, GitApi};

let git = Git::new();
let repo = Path::new(".");

let branch = git.current_branch(repo).await?; // String, e.g. "main"
let status = git.status(repo).await?; // Vec<StatusEntry>
let log = git.log(repo, 10).await?; // Vec<Commit>, newest first
```

### Stage, commit, inspect

```rust
use std::path::{Path, PathBuf};
use vcs_git::{Git, GitApi};

# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
    let git = Git::new();

    git.add(repo, &[PathBuf::from("src/lib.rs")]).await?; // `git add -- src/lib.rs`
    git.commit(repo, "feat: tidy lib").await?; // `git commit -m …`

    // `diff_is_empty` is the exit-code answer of `git diff --quiet`:
    if !git.diff_is_empty(repo).await? {
        println!("working tree still has unstaged changes");
    }

    for c in git.log(repo, 5).await? {
        println!("{} {} — {} <{}>", c.short_hash, c.subject, c.author, c.date);
    }
# Ok(()) }
```

### Renames come back structured

`status` runs `git status --porcelain=v1 -z`, so a rename carries both paths:

```rust
# use std::path::Path;
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git, repo: &Path) -> Result<(), processkit::Error> {
    for entry in git.status(repo).await? {
        match entry.orig_path {
            Some(from) => println!("rename {from} -> {}", entry.path),
            None => println!("{} {}", entry.code, entry.path),
        }
    }
# Ok(()) }
```

### Worktrees

Manage linked worktrees with structured results:

```rust
use vcs_git::{Git, GitApi, WorktreeAdd};
use std::path::Path;

# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
let git = Git::new();

// Create a worktree on a new branch based on HEAD.
git.worktree_add(repo, WorktreeAdd::create_branch("/tmp/feature", "feature", "HEAD"))
    .await?;

for wt in git.worktree_list(repo).await? {            // Vec<Worktree>
    println!("{} -> {:?}", wt.path.display(), wt.branch);
}

git.worktree_remove(repo, Path::new("/tmp/feature"), false).await?;
# Ok(()) }
```

### Distinguish failures structurally

```rust
# use std::path::Path;
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git, repo: &Path) {
    match git.checkout(repo, "nope").await {
        Ok(()) => {}
        Err(processkit::Error::Exit { code, stderr, .. }) => {
            eprintln!("git exited {code}: {stderr}")
        }
        Err(processkit::Error::Timeout { .. }) => eprintln!("timed out"),
        Err(e) => eprintln!("{e}"),
    }
# }
```

Consumers depend on the `GitApi` trait and substitute a fake in tests — enable
the `mock` feature for a `mockall`-generated `MockGitApi`, or inject a fake
process runner with `Git::with_runner(processkit::ScriptedRunner::new()…)`:

```rust
use processkit::{Reply, ScriptedRunner};
use std::path::Path;
use vcs_git::{Git, GitApi};

# async fn demo() {
    let git = Git::with_runner(ScriptedRunner::new().on(["rev-parse"], Reply::ok("feature\n")));
    assert_eq!(git.current_branch(Path::new(".")).await.unwrap(), "feature");
# }
```

Requires the `git` binary on `PATH`.

## License

MIT
