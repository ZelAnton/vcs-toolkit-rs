//! `vcs-testkit` — test fixtures for git/jj automation.
//!
//! Throwaway repositories for integration tests: a unique self-cleaning
//! [`TempDir`], a configured [`GitSandbox`] / [`JjSandbox`] to build scenarios
//! in, and a seeded [`BareRemote`] to clone/fetch/push against. Everything is
//! **synchronous** (test setup needs no runtime) and shells out to the real
//! `git` / `jj` binaries on `PATH` — gate tests that use it behind
//! `#[ignore = "requires the git binary"]` so hermetic CI stays green.
//!
//! **Every helper panics on failure.** These are test fixtures: a broken
//! fixture should fail the test loudly at the call site, not thread `Result`s
//! through scenario-building code.
//!
//! ```no_run
//! use vcs_testkit::{BareRemote, GitSandbox};
//!
//! let repo = GitSandbox::init("my-test");
//! repo.commit_file("a.txt", "one\n", "first");
//! repo.branch("feature");
//!
//! let remote = BareRemote::seeded("my-remote");
//! repo.git(&["remote", "add", "origin", remote.url().as_str()]);
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique temporary directory, removed on drop.
///
/// Unique without a temp-dir crate: process id + a process-wide monotonic
/// counter, so parallel tests never collide.
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

/// Run a binary in `cwd`, panicking (with the command line in the message) on
/// a spawn failure or non-zero exit. The fixture contract: fail loudly.
fn run(binary: &str, cwd: &Path, args: &[&str]) {
    let status = Command::new(binary)
        .current_dir(cwd)
        .args(args)
        .status()
        .unwrap_or_else(|e| panic!("failed to run `{binary} {args:?}`: {e}"));
    assert!(status.success(), "`{binary} {args:?}` exited with {status}");
}

/// Like [`run`] but capturing trimmed stdout.
fn run_capture(binary: &str, cwd: &Path, args: &[&str]) -> String {
    let out = Command::new(binary)
        .current_dir(cwd)
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
/// prompts), `core.autocrlf=false` (no CRLF rewriting under content
/// assertions on Windows).
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
    pub fn init(tag: &str) -> Self {
        let dir = TempDir::new(tag);
        run("git", dir.path(), &["init", "-q", "-b", "main"]);
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
        run("git", &work, &["init", "-q", "-b", "main"]);
        configure_identity(&work);
        std::fs::write(work.join("seed.txt"), "seed\n").expect("write seed");
        run("git", &work, &["add", "-A"]);
        run("git", &work, &["commit", "-qm", "seed"]);
        run("git", &bare, &["init", "-q", "--bare", "-b", "main"]);
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
}
