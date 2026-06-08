# vcs-testkit — test fixtures guide

Throwaway repositories for integration tests. `vcs-testkit` gives you a
self-cleaning [`TempDir`](#tempdir), a configured [`GitSandbox`](#gitsandbox) /
[`JjSandbox`](#jjsandbox) to build scenarios in, and a seeded
[`BareRemote`](#bareremote) to clone/fetch/push against — the same fixtures this
workspace's own ignored tests run on.

Three properties shape every helper, and they are deliberate:

- **Synchronous.** Test setup needs no runtime — fixtures shell out with
  `std::process::Command`, not the async `processkit` client under test, so they
  stay usable from any `#[test]` regardless of how the subject is wired.
- **Panics on failure.** A fixture is not the thing under test; a broken fixture
  should fail the test *loudly at the call site*, not thread `Result`s through
  scenario-building code. Every method `unwrap`s/`assert`s internally.
- **Needs real binaries.** Helpers run the real `git` / `jj` on `PATH`. Gate any
  test that touches them behind `#[ignore = "requires the git binary"]` so a
  hermetic CI (no binaries installed) stays green; run them locally with
  `cargo test -- --ignored`.

`vcs-testkit` depends on nothing — not even the wrapper crates — so it can be a
dev-dependency of any of them without a coupling cycle. Scenario-building goes
through each sandbox's raw escape hatch (`git`/`jj`) plus a few convenience
methods.

```toml
# Cargo.toml — a path dev-dependency, stripped on publish.
[dev-dependencies]
vcs-testkit = { path = "../testkit" }
```

---

## `TempDir`

A unique temporary directory, removed on drop. Uniqueness without a temp-dir
crate: process id + a process-wide monotonic counter, so parallel tests never
collide.

- `TempDir::new(tag)` — create `%TEMP%/vcs-testkit-<tag>-<pid>-<n>`. Panics when
  the directory cannot be created.
- `path()` — the directory's path (`&Path`).
- `Drop` — best-effort `remove_dir_all` on the way out; a leaked temp dir must
  not fail the run, so the cleanup error is swallowed.

```rust,ignore
use vcs_testkit::TempDir;

let tmp = TempDir::new("scratch");
std::fs::write(tmp.path().join("note.txt"), "hi").unwrap();
let kept = tmp.path().to_path_buf();
drop(tmp);                       // directory tree removed here
assert!(!kept.exists());
```

---

## `GitSandbox`

A throwaway **git** repository: owns its [`TempDir`], initialised on branch
`main` (`git init -b main`) with a deterministic identity (see
[`configure_identity`](#standalone-functions)).

- `GitSandbox::init(tag)` — create and initialise the repository.
- `path()` — the working-tree path (`&Path`).
- `git(&[..])` — run `git <args>` in the repo, panicking on failure (the escape
  hatch for anything the convenience methods don't cover).
- `write(path, content)` — write `content` to the repo-relative `path`, creating
  parent dirs.
- `add_all()` — stage everything (`git add -A`).
- `commit(msg)` — commit the staged changes (`git commit -qm <msg>`).
- `commit_file(path, content, msg)` — write + stage + commit one file, the
  everyday scenario step.
- `branch(name)` — create a branch at HEAD without switching (`git branch`).
- `checkout(name)` — switch to a branch (`git checkout`).
- `rev_parse(rev)` — resolve a revision to its full 40-char hash (`String`).

```rust,ignore
use vcs_testkit::GitSandbox;

let repo = GitSandbox::init("scenario");
repo.commit_file("a.txt", "one\n", "first");   // write + add -A + commit
repo.branch("feature");
repo.checkout("feature");
repo.commit_file("sub/b.txt", "two\n", "second");

let head = repo.rev_parse("HEAD");
assert_eq!(head.len(), 40);
assert_ne!(head, repo.rev_parse("main"));       // feature has diverged

// Drop to raw git for anything not modelled:
repo.git(&["tag", "v1"]);
```

---

## `BareRemote`

A populated **bare** git repository — a local clone/fetch/push source for
integration tests, no network. Seeded with one commit on `main` containing
`seed.txt`.

- `BareRemote::seeded(tag)` — build the seeded bare repository.
- `path()` — the bare repo's path (`&Path`); use it as a local remote URL.
- `url()` — the path as an owned `String`, convenient for argv slices.
- `temp_dir()` — the owning temp dir (`&Path`), kept alive as long as the remote
  is in use.

```rust,ignore
use vcs_testkit::{BareRemote, GitSandbox};

let remote = BareRemote::seeded("origin");
let repo = GitSandbox::init("clone-target");
repo.git(&["remote", "add", "origin", remote.url().as_str()]);
repo.git(&["fetch", "-q", "origin"]);
// `seed.txt` from the seed commit is now reachable as origin/main.
```

---

## `JjSandbox`

A throwaway **jj** repository (git-backed) with a repo-scoped identity
(`jj git init` + `user.name`/`user.email` set `--repo`).

- `JjSandbox::init(tag)` — create and initialise the workspace.
- `path()` — the workspace root path (`&Path`).
- `jj(&[..])` — run `jj <args>` in the workspace, panicking on failure.
- `write(path, content)` — write to the workspace-relative `path`, creating
  parents.
- `describe(msg)` — describe the working-copy change (`jj describe -m <msg>`).
- `new_change(msg)` — start a new change on top (`jj new -m <msg>`).
- `bookmark(name)` — create a bookmark at `@` (`jj bookmark create <name> -r @`).

```rust,ignore
use vcs_testkit::JjSandbox;

let repo = JjSandbox::init("jj-scenario");
repo.write("a.txt", "one\n");
repo.describe("base");          // describe the working-copy change
repo.bookmark("mark");          // bookmark at @
repo.new_change("next");        // start a fresh change on top
repo.jj(&["log", "--no-graph"]); // raw escape hatch
```

---

## Standalone functions

For scenario steps in directories *not owned by a sandbox* — linked worktrees,
fresh clones, or repos initialised by the code under test.

- `git(dir, &[..])` — run `git <args>` in `dir`, panicking on failure.
- `jj(dir, &[..])` — the same for `jj`.
- `configure_identity(dir)` — give a git repo at `dir` a deterministic identity
  and byte-stable behaviour: `user.name`/`user.email`, `commit.gpgsign=false`
  (no keychain prompts), `core.autocrlf=false` (no CRLF rewriting under content
  assertions on Windows). Standalone — not folded only into `GitSandbox::init` —
  for tests whose *subject* is repository initialisation itself: they run their
  own `init` and only need the identity applied afterwards.

```rust,ignore
use std::path::Path;
use vcs_testkit::{configure_identity, git, TempDir};

// Test whose subject is init itself: do the init, then make it deterministic.
let tmp = TempDir::new("custom-init");
git(tmp.path(), &["init", "-q", "-b", "trunk"]);
configure_identity(tmp.path());
git(tmp.path(), &["commit", "--allow-empty", "-qm", "root"]);
```

---

## Worked end-to-end scenario

Sandbox + bare remote, exercising a push then a fetch round-trip — the shape of
the crate's own ignored integration test.

```rust,ignore
use vcs_testkit::{BareRemote, GitSandbox};

#[test]
#[ignore = "requires the git binary"]   // gated — hermetic CI has no git
fn push_then_fetch_round_trip() {
    // Local repo with two commits on a feature branch.
    let repo = GitSandbox::init("local");
    repo.commit_file("a.txt", "one\n", "first");
    repo.branch("feature");
    repo.checkout("feature");
    repo.commit_file("b.txt", "two\n", "second");

    // A seeded bare repo to push at.
    let remote = BareRemote::seeded("remote");
    repo.git(&["remote", "add", "origin", remote.url().as_str()]);
    repo.git(&["push", "-q", "origin", "feature"]);

    // Fetch brings the seeded main back; the pushed feature is on the remote.
    repo.git(&["fetch", "-q", "origin"]);
    let remote_main = repo.rev_parse("origin/main");
    assert_eq!(remote_main.len(), 40);
    // Both fixtures' temp dirs clean themselves up when they drop here.
}
```

---

See also: [Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/) for the trait / mock / runner seams
that let most tests skip real binaries entirely, and the
[crate docs](https://docs.rs/vcs-testkit).
