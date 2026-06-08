# vcs-testkit

Test fixtures for git/jj automation: throwaway repositories for integration
tests.

> 📖 **Full guide:** [on docs.rs](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/)
> — every fixture with examples, plus the
> [Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/) guide.

- **`TempDir`** — a unique, self-cleaning temp directory (pid + counter, no
  temp-dir dependency).
- **`GitSandbox`** — a configured git repo on branch `main` (deterministic
  identity, signing off, `autocrlf=false`) with scenario helpers:
  `commit_file`, `branch`, `checkout`, `rev_parse`, and a raw `git(&[…])`
  escape hatch.
- **`BareRemote`** — a populated bare repository to clone/fetch/push against
  locally (no network).
- **`JjSandbox`** — a jj (git-backed) workspace with repo-scoped identity:
  `describe`, `new_change`, `bookmark`, raw `jj(&[…])`.
- **`configure_identity`** — just the deterministic config, for tests whose
  *subject* is repository initialisation itself.

Everything is synchronous (`std::process`) and **panics on failure** — these
are fixtures; a broken fixture should fail the test loudly at the call site.
The real `git` / `jj` binaries must be on `PATH`, so gate tests behind
`#[ignore = "requires the git binary"]` to keep hermetic CI green.

```rust
use vcs_testkit::{BareRemote, GitSandbox};

let repo = GitSandbox::init("my-test");
repo.commit_file("a.txt", "one\n", "first");
repo.branch("feature");

let remote = BareRemote::seeded("my-remote");
repo.git(&["remote", "add", "origin", remote.url().as_str()]);
```

Part of [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs); used as a
**dev-dependency** by the `vcs-git` / `vcs-jj` / `vcs-core` test suites and by
downstream consumers' integration tests. Depends on nothing (std only).
