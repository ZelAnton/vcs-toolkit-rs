#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-testkit` — throwaway git/jj sandboxes (and a bare remote) for
//! integration tests.
//!
//! Hands a `#[test]` a real repository to drive: a unique self-cleaning
//! [`TempDir`], a configured [`GitSandbox`] / [`JjSandbox`] to build scenarios
//! in, and a seeded [`BareRemote`] to clone/fetch/push against. It is
//! **dependency-free** (not even the wrapper crates, so it can be a
//! dev-dependency of any of them without a cycle), **synchronous** (test setup
//! needs no runtime — it shells out with `std::process::Command`, not the async
//! client under test), and **panics on failure** (a broken fixture should fail
//! loudly at the call site, not thread `Result`s through scenario code).
//!
//! Built for `#[test]` / `#[ignore]` integration tests that need a *real* repo:
//! the helpers run the actual `git` / `jj` on `PATH`, so gate any test that
//! touches one behind `#[ignore = "requires the git binary"]` — a hermetic CI
//! with no binaries installed then stays green, and `cargo test -- --ignored`
//! runs them locally. Every sandbox is isolated from the host's VCS config (no
//! system/global config, no `init.templateDir` hook leakage, a deterministic
//! identity even on the commit `jj git init` creates) — see `command`.
//!
//! # The surface
//!
//! - **[`TempDir`]** — a unique temporary directory, removed on drop.
//!   Uniqueness without a temp-dir crate: pid + a process-wide monotonic
//!   counter, so parallel tests in a run never collide. Every fixture owns one.
//! - **[`GitSandbox`]** — a throwaway **git** repo on branch `main` with a
//!   deterministic identity. Build scenarios through the convenience steps
//!   ([`commit_file`](GitSandbox::commit_file), [`branch`](GitSandbox::branch),
//!   [`checkout`](GitSandbox::checkout), [`rev_parse`](GitSandbox::rev_parse))
//!   plus the raw [`git`](GitSandbox::git) escape hatch for anything unmodelled.
//! - **[`JjSandbox`]** — the same shape for a **jj** (git-backed) workspace:
//!   [`describe`](JjSandbox::describe), [`new_change`](JjSandbox::new_change),
//!   [`bookmark`](JjSandbox::bookmark), and the raw [`jj`](JjSandbox::jj) hatch.
//! - **[`BareRemote`]** — a populated **bare** git repo, a local
//!   clone/fetch/push source with no network. [`BareRemote::seeded`] gives one
//!   commit on `main` containing `seed.txt`; [`url`](BareRemote::url) yields a
//!   string remote URL.
//! - **[`configure_identity`]** — stamp a git repo with a deterministic
//!   identity and byte-stable behaviour (`user.*`, `commit.gpgsign=false`,
//!   `core.autocrlf=false`). Standalone, for tests whose *subject* is `init`.
//! - **Raw steps [`git`] / [`jj`]** — run one command in any `dir`, panicking
//!   on failure: for scenario steps in directories no sandbox owns (linked
//!   worktrees, fresh clones, repos the code under test initialised).
//!
//! # Recipes
//!
//! These are sync — no async wrapper, no `Result` (fixtures panic). They are
//! `no_run`: they really create temp dirs and shell out to `git`/`jj`, so they
//! compile here but only run under a binary-equipped `#[test]`.
//!
//! Build a git scenario — write + stage + commit is one step:
//!
//! ```no_run
//! use vcs_testkit::GitSandbox;
//! # fn demo() {
//! let repo = GitSandbox::init("scenario");
//! repo.commit_file("a.txt", "one\n", "first");   // write + add -A + commit
//! repo.branch("feature");
//! repo.checkout("feature");
//! repo.commit_file("sub/b.txt", "two\n", "second");
//!
//! let head = repo.rev_parse("HEAD");
//! assert_eq!(head.len(), 40);
//! assert_ne!(head, repo.rev_parse("main"));       // feature has diverged
//! # }
//! ```
//!
//! Seed a bare remote and fetch from it — drop to raw `git` for the remote wiring:
//!
//! ```no_run
//! use vcs_testkit::{BareRemote, GitSandbox};
//! # fn demo() {
//! let repo = GitSandbox::init("local");
//! repo.commit_file("a.txt", "one\n", "first");
//!
//! let remote = BareRemote::seeded("origin");
//! repo.git(&["remote", "add", "origin", remote.url().as_str()]);
//! repo.git(&["fetch", "-q", "origin"]);
//! assert_eq!(repo.rev_parse("origin/main").len(), 40); // seed commit fetched
//! # }
//! ```
//!
//! # In-depth guide
//!
//! Beyond this page, this crate ships a full how-to guide — rendered on docs.rs
//! from `docs/`. See the [`guide`] module (and its cross-cutting
//! [`testing`](crate::guide::testing) sub-guide on the trait / mock / runner
//! seams that let most tests skip real binaries entirely).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique temporary directory, removed on drop.
///
/// Unique without a temp-dir crate: process id + a process-wide monotonic
/// counter, so parallel tests within a run never collide. The name is kept
/// deliberately short — jj's `op_store` paths are deep, and a long prefix here
/// can tip a nested `.jj/repo/op_store/operations/<id>` path over Windows'
/// `MAX_PATH` (260) limit.
pub struct TempDir(PathBuf);

impl TempDir {
    /// Create `%TEMP%/vcs-testkit-<tag>-<pid>-<n>`. Panics when the directory
    /// cannot be created.
    pub fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "vcs-testkit-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    /// The directory's path.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Best-effort: a leaked temp dir must not fail the test run.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build an isolated [`Command`] for `binary` in `cwd`.
///
/// **Every** git/jj invocation the testkit makes routes through here so the
/// sandbox is hermetic — it must not inherit the host user's VCS config. A
/// host-global `init.templateDir` / `core.hooksPath` (git) or `[user]` block
/// (jj) would otherwise leak in: a templateDir hook gets copied into the
/// sandbox's `.git/hooks` and *executes* during sandbox commits, and a host
/// jj identity stamps the init-created working-copy commit.
///
/// The redirect-config env vars point at a guaranteed-nonexistent path; git
/// and jj both treat a missing config file as empty, so no temp file is
/// needed and the free [`git`]/[`jj`] helpers (which own no sandbox dir) get
/// the same isolation as the sandbox methods.
fn command(binary: &str, cwd: &Path) -> Command {
    // A path that cannot exist: a child of *this* binary's own path (a file,
    // so it can have no children). Resolved per call to stay self-contained.
    let nonexistent = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("vcs-testkit-no-such"))
        .join("vcs-testkit-nonexistent-config");
    let mut cmd = Command::new(binary);
    cmd.current_dir(cwd);
    match binary {
        "git" => {
            // Ignore system config; redirect global/system config at a
            // nonexistent file (defeats a host-set GIT_CONFIG_GLOBAL too);
            // and never block on a credential prompt. Scrub any inherited
            // GIT_DIR-style vars that would otherwise point git elsewhere.
            cmd.env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", &nonexistent)
                .env("GIT_CONFIG_SYSTEM", &nonexistent)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env_remove("GIT_CONFIG_PARAMETERS")
                .env_remove("GIT_CONFIG")
                .env_remove("GIT_DIR")
                .env_remove("GIT_COMMON_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env_remove("GIT_OBJECT_DIRECTORY")
                .env_remove("GIT_NAMESPACE");
        }
        "jj" => {
            // Read config exclusively from a nonexistent file (no host
            // config), and stamp a deterministic identity on *every* commit
            // — including the working-copy commit `jj git init` creates,
            // which a later `config set --repo user.*` cannot retroactively
            // re-author.
            cmd.env("JJ_CONFIG", &nonexistent)
                .env("JJ_USER", "test")
                .env("JJ_EMAIL", "test@example.com");
        }
        _ => {}
    }
    cmd
}

/// Run a binary in `cwd`, panicking (with the command line in the message) on
/// a spawn failure or non-zero exit. The fixture contract: fail loudly.
fn run(binary: &str, cwd: &Path, args: &[&str]) {
    let status = command(binary, cwd)
        .args(args)
        .status()
        .unwrap_or_else(|e| panic!("failed to run `{binary} {args:?}`: {e}"));
    assert!(status.success(), "`{binary} {args:?}` exited with {status}");
}

/// Like [`run`] but capturing trimmed stdout.
fn run_capture(binary: &str, cwd: &Path, args: &[&str]) -> String {
    let out = command(binary, cwd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run `{binary} {args:?}`: {e}"));
    assert!(
        out.status.success(),
        "`{binary} {args:?}` exited with {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

/// Run `git <args>` in `dir`, panicking on failure — for scenario steps in
/// directories not owned by a [`GitSandbox`] (linked worktrees, fresh clones,
/// repos initialised by the code under test).
pub fn git(dir: &Path, args: &[&str]) {
    run("git", dir, args);
}

/// Run `jj <args>` in `dir`, panicking on failure (see [`git`]).
pub fn jj(dir: &Path, args: &[&str]) {
    run("jj", dir, args);
}

/// Give the git repository at `dir` a deterministic identity and byte-stable
/// behaviour: `user.name`/`user.email`, `commit.gpgsign=false` (no keychain
/// prompts), and `core.autocrlf=false` (no CRLF rewriting under content
/// assertions on Windows).
///
/// Deliberately does NOT touch `core.hooksPath`: host-config hook leakage is
/// neutralised at the source instead — `command`'s env redirect keeps a host
/// global/system config (a `core.hooksPath` or `init.templateDir`) out of
/// every testkit-run git, and `--template=` on `init` keeps template hooks
/// from being copied into `.git/hooks`. Disabling hooks in the repo's *local*
/// config would also disable hooks a test itself installs on purpose (e.g.
/// the hardened-profile suppression test).
///
/// Standalone (not folded into [`GitSandbox::init`] only) for tests whose
/// *subject* is repository initialisation itself — they run their own `init`
/// and only need the identity applied afterwards.
pub fn configure_identity(dir: &Path) {
    for (key, val) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        run("git", dir, &["config", key, val]);
    }
}

/// A throwaway **git** repository: owns its [`TempDir`], initialised on
/// branch `main` with a deterministic identity (see [`configure_identity`]).
///
/// Scenario-building goes through the raw [`git`](GitSandbox::git) escape
/// hatch plus the convenience methods — the sandbox deliberately does not
/// depend on the typed wrapper crates, so it can be a dev-dependency of any
/// of them.
pub struct GitSandbox {
    dir: TempDir,
}

impl GitSandbox {
    /// Create and initialise a repository (`git init -b main` — git ≥ 2.28,
    /// comfortably below the wrappers' documented floor).
    ///
    /// `--template=` (empty) makes the new repo skip *any* init template,
    /// so a host-global `init.templateDir` cannot seed hooks into
    /// `.git/hooks` — the version-portable complement to the config
    /// isolation in `command`.
    pub fn init(tag: &str) -> Self {
        let dir = TempDir::new(tag);
        run(
            "git",
            dir.path(),
            &["init", "-q", "-b", "main", "--template="],
        );
        configure_identity(dir.path());
        GitSandbox { dir }
    }

    /// The repository's working-tree path.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Run `git <args>` in the repository, panicking on failure.
    pub fn git(&self, args: &[&str]) {
        run("git", self.path(), args);
    }

    /// Write `content` to the repo-relative `path` (creating parent dirs).
    pub fn write(&self, path: &str, content: &str) {
        let full = self.path().join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("create parent dirs");
        }
        std::fs::write(full, content).expect("write file");
    }

    /// Stage everything (`git add -A`).
    pub fn add_all(&self) {
        self.git(&["add", "-A"]);
    }

    /// Commit the staged changes (`git commit -qm <message>`).
    pub fn commit(&self, message: &str) {
        self.git(&["commit", "-qm", message]);
    }

    /// Write + stage + commit one file — the everyday scenario step.
    pub fn commit_file(&self, path: &str, content: &str, message: &str) {
        self.write(path, content);
        self.add_all();
        self.commit(message);
    }

    /// Create a branch at HEAD without switching (`git branch <name>`).
    pub fn branch(&self, name: &str) {
        self.git(&["branch", "-q", name]);
    }

    /// Switch to a branch (`git checkout <name>`).
    pub fn checkout(&self, name: &str) {
        self.git(&["checkout", "-q", name]);
    }

    /// Resolve a revision to a full hash (`git rev-parse <rev>`).
    pub fn rev_parse(&self, rev: &str) -> String {
        run_capture("git", self.path(), &["rev-parse", rev])
    }
}

/// A populated **bare** git repository — a local clone/fetch/push source for
/// integration tests (no network). Seeded with one commit on `main`
/// containing `seed.txt`.
pub struct BareRemote {
    dir: TempDir,
    bare: PathBuf,
}

impl BareRemote {
    /// Build the seeded bare repository.
    pub fn seeded(tag: &str) -> Self {
        let dir = TempDir::new(tag);
        let work = dir.path().join("seed-work");
        let bare = dir.path().join("remote.git");
        std::fs::create_dir_all(&work).expect("create work dir");
        std::fs::create_dir_all(&bare).expect("create bare dir");
        run("git", &work, &["init", "-q", "-b", "main", "--template="]);
        configure_identity(&work);
        std::fs::write(work.join("seed.txt"), "seed\n").expect("write seed");
        run("git", &work, &["add", "-A"]);
        run("git", &work, &["commit", "-qm", "seed"]);
        run(
            "git",
            &bare,
            &["init", "-q", "--bare", "-b", "main", "--template="],
        );
        run(
            "git",
            &work,
            &["push", "-q", bare.to_str().expect("utf8 path"), "main:main"],
        );
        BareRemote { dir, bare }
    }

    /// The bare repository's path (use as a local remote URL).
    pub fn path(&self) -> &Path {
        &self.bare
    }

    /// The path as a `String` — convenient for argv slices.
    pub fn url(&self) -> String {
        self.bare.to_str().expect("utf8 path").to_string()
    }

    /// The owning temp dir (kept alive as long as the remote is used).
    pub fn temp_dir(&self) -> &Path {
        self.dir.path()
    }
}

/// A throwaway **jj** repository (git-backed) with a repo-scoped identity.
pub struct JjSandbox {
    dir: TempDir,
}

impl JjSandbox {
    /// Create and initialise the repository (`jj git init` + repo-scoped
    /// `user.name`/`user.email`).
    ///
    /// The identity is supplied to *every* jj invocation as `JJ_USER` /
    /// `JJ_EMAIL` env (see `command`), so the working-copy commit that
    /// `jj git init` creates is authored deterministically — a later
    /// `config set --repo user.*` only affects *future* commits and so cannot
    /// fix the init commit on its own. The repo-scoped config is kept anyway
    /// as belt-and-braces for any tool path that reads config over the env.
    pub fn init(tag: &str) -> Self {
        let dir = TempDir::new(tag);
        run("jj", dir.path(), &["git", "init"]);
        run(
            "jj",
            dir.path(),
            &["config", "set", "--repo", "user.name", "Test"],
        );
        run(
            "jj",
            dir.path(),
            &["config", "set", "--repo", "user.email", "test@example.com"],
        );
        JjSandbox { dir }
    }

    /// The workspace root path.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Run `jj <args>` in the workspace, panicking on failure.
    pub fn jj(&self, args: &[&str]) {
        run("jj", self.path(), args);
    }

    /// Run `jj <args>` in the workspace and capture trimmed stdout (panics on
    /// failure) — for reading state in assertions (op ids, the `@` commit id).
    /// Uses the same config-isolated environment as [`jj`](JjSandbox::jj).
    pub fn jj_capture(&self, args: &[&str]) -> String {
        run_capture("jj", self.path(), args)
    }

    /// The current operation id (`jj op log … --ignore-working-copy`). Capture it
    /// before and after a series of *read-only* queries to assert none recorded a
    /// new operation (a mutating `jj` snapshot would advance it).
    ///
    /// **Read-only measurement:** `--ignore-working-copy` is essential here — a
    /// plain `jj op log` would itself snapshot any pending working-tree edit and
    /// record an operation, so the measurement would perturb the very thing it is
    /// asserting about.
    pub fn op_head(&self) -> String {
        self.jj_capture(&[
            "op",
            "log",
            "--no-graph",
            "-n1",
            "-T",
            "id.short()",
            "--ignore-working-copy",
        ])
    }

    /// The working-copy commit id of `@` (`jj log -r @ … --ignore-working-copy`) —
    /// to assert a read-only query did not move `@` (jj rewrites `@` when it
    /// snapshots an unsnapshotted working-tree edit). Read-only for the same reason
    /// as [`op_head`](JjSandbox::op_head).
    pub fn at_commit(&self) -> String {
        self.jj_capture(&[
            "log",
            "-r",
            "@",
            "--no-graph",
            "-T",
            "commit_id",
            "--ignore-working-copy",
        ])
    }

    /// Write `content` to the workspace-relative `path` (creating parents).
    pub fn write(&self, path: &str, content: &str) {
        let full = self.path().join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("create parent dirs");
        }
        std::fs::write(full, content).expect("write file");
    }

    /// Describe the working-copy change (`jj describe -m <message>`).
    pub fn describe(&self, message: &str) {
        self.jj(&["describe", "-m", message]);
    }

    /// Start a new change on top (`jj new -m <message>`).
    pub fn new_change(&self, message: &str) {
        self.jj(&["new", "-m", message]);
    }

    /// Create a bookmark at `@` (`jj bookmark create <name> -r @`).
    pub fn bookmark(&self, name: &str) {
        self.jj(&["bookmark", "create", name, "-r", "@"]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hermetic: uniqueness and cleanup need no binaries.
    #[test]
    fn temp_dirs_are_unique_and_removed_on_drop() {
        let a = TempDir::new("unique");
        let b = TempDir::new("unique");
        assert_ne!(a.path(), b.path());
        assert!(a.path().exists() && b.path().exists());
        let kept = a.path().to_path_buf();
        drop(a);
        assert!(!kept.exists(), "removed on drop");
    }

    // Real-binary round-trips; ignored so hermetic CI stays green.
    #[test]
    #[ignore = "requires the git binary"]
    fn git_sandbox_builds_scenarios() {
        let repo = GitSandbox::init("sandbox");
        repo.commit_file("a.txt", "one\n", "first");
        repo.branch("feature");
        repo.checkout("feature");
        repo.commit_file("sub/b.txt", "two\n", "second");
        let head = repo.rev_parse("HEAD");
        assert_eq!(head.len(), 40);
        assert_ne!(head, repo.rev_parse("main"));

        let remote = BareRemote::seeded("remote");
        repo.git(&["remote", "add", "origin", remote.url().as_str()]);
        repo.git(&["fetch", "-q", "origin"]);
        assert_eq!(
            run_capture("git", repo.path(), &["show", "origin/main:seed.txt"]),
            "seed"
        );
    }

    // Isolation: `--template=` plus the config env keep a host-global
    // `init.templateDir` from seeding hooks, so the sandbox's `.git/hooks`
    // holds no live hook. (A real host hook firing is what the reviewer hit;
    // here we assert the precondition — no enabled hook files — which holds
    // regardless of the host's config.)
    #[test]
    #[ignore = "requires the git binary"]
    fn git_sandbox_has_no_leaked_hooks() {
        let repo = GitSandbox::init("hooks");
        repo.commit_file("a.txt", "one\n", "first");
        let hooks = repo.path().join(".git").join("hooks");
        let enabled: Vec<_> = std::fs::read_dir(&hooks)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            // git ships `*.sample` hooks (inert); only non-sample files run.
            .filter(|name| !name.ends_with(".sample"))
            .collect();
        assert!(
            enabled.is_empty(),
            "sandbox should have no live hooks, found {enabled:?}"
        );
        // Note `core.hooksPath` is deliberately NOT pinned in the local config —
        // a test may install its own hook on purpose (see `configure_identity`);
        // the isolation lives in `command`'s env + `--template=` instead.
    }

    #[test]
    #[ignore = "requires the jj binary"]
    fn jj_sandbox_builds_scenarios() {
        let repo = JjSandbox::init("sandbox");
        repo.write("a.txt", "one\n");
        repo.describe("base");
        repo.bookmark("mark");
        repo.new_change("next");
        // The described change and the bookmark are visible to jj.
        let out = run_capture(
            "jj",
            repo.path(),
            &[
                "log",
                "-r",
                "::@",
                "--no-graph",
                "-T",
                "description.first_line() ++ \"\\n\"",
                "--color",
                "never",
            ],
        );
        assert!(out.contains("base"), "got {out:?}");
    }

    // Isolation: the working-copy commit `jj git init` creates is authored
    // deterministically from the `JJ_USER`/`JJ_EMAIL` env, *not* from the
    // host's jj config (which `config set --repo` could not retroactively
    // re-author). `root()+` is the first non-root commit — the init commit.
    #[test]
    #[ignore = "requires the jj binary"]
    fn jj_sandbox_init_commit_has_deterministic_author() {
        let repo = JjSandbox::init("identity");
        let email = run_capture(
            "jj",
            repo.path(),
            &[
                "log",
                "-r",
                "root()+",
                "--no-graph",
                "-T",
                "author.email()",
                "--color",
                "never",
            ],
        );
        assert_eq!(email, "test@example.com", "init commit author.email");
    }
}

// Long-form how-to guides, rendered from this crate's docs/*.md on docs.rs.
#[doc = include_str!("../docs/testkit.md")]
#[allow(rustdoc::broken_intra_doc_links)]
pub mod guide {
    #[doc = include_str!("../docs/testing.md")]
    #[allow(rustdoc::broken_intra_doc_links)]
    pub mod testing {}
}
