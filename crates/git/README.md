# vcs-git — automate Git from Rust

[![crates.io](https://img.shields.io/crates/v/vcs-git.svg)](https://crates.io/crates/vcs-git) [![docs.rs](https://img.shields.io/docsrs/vcs-git)](https://docs.rs/vcs-git) [![downloads](https://img.shields.io/crates/d/vcs-git.svg)](https://crates.io/crates/vcs-git)

Part of the [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.

**What you can do:** status & branches, stage/commit/checkout, diff & log,
merge/rebase/reset, worktrees, tags, blame, clone, config, cherry-pick/revert,
parse & resolve conflict markers, and a hardened (hooks-off) mode for untrusted
repos — all as typed, repo-scoped `async` methods over the `git` binary, behind a
mockable interface.

**How it works:** each call runs the real `git` (its exact behaviour, config and
credentials) and parses the output into typed values. Commands run inside an OS job
(an OS-level container that kills the whole process tree if your program exits, via
[`processkit`]) so no `git` subprocess is ever orphaned; calls return the
structured `Error` and honour an optional timeout.

**Credentials:** ambient git credential helpers / SSH agent by default; to supply a
token per operation, the one-liner is `Git::new().with_token(tok)` (or
`.with_env_token("MY_TOKEN")`); for full control attach a `CredentialProvider` with
`.with_credentials(...)`. For HTTPS remotes the secret is fed via an inline
`credential.helper`, kept out of `argv` (SSH remotes use the ambient agent).

[`processkit`]: https://crates.io/crates/processkit

> 📖 **Full guide:** [on docs.rs](https://docs.rs/vcs-git/latest/vcs_git/guide/)
> — every command by theme, result types, builder/newtype APIs, and worked examples.

Every method is `async`, so call it from a tokio runtime:

```rust
use std::path::Path;
use vcs_git::{Git, GitApi};

let git = Git::new();
let repo = Path::new(".");

let branch = git.current_branch(repo).await?; // Option<String>, e.g. Some("main")
let status = git.status(repo).await?; // Vec<StatusEntry>
let log = git.log(repo, "HEAD", 10).await?; // Vec<Commit>, newest first
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

    for c in git.log(repo, "HEAD", 5).await? {
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
        match entry.old_path {
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
process runner with `Git::with_runner(processkit::testing::ScriptedRunner::new()…)`:

```rust
use processkit::testing::{Reply, ScriptedRunner};
use std::path::Path;
use vcs_git::{Git, GitApi};

# async fn demo() {
    let git = Git::with_runner(ScriptedRunner::new().on(["git", "symbolic-ref"], Reply::ok("feature\n")));
    assert_eq!(git.current_branch(Path::new(".")).await.unwrap(), Some("feature".to_string()));
# }
```

Requires the `git` binary on `PATH`.

## License

MIT
