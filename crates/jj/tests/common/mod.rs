//! Shared helpers for `vcs-jj` integration tests.
#![allow(dead_code)] // not every test binary uses every helper

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique temporary directory, removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "vcs-jj-test-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Create a populated **bare** git repository under `dir` and return its path —
/// a local clone/fetch source for integration tests (no network). Seeded with
/// one commit on `main` containing `seed.txt`.
pub fn bare_remote(dir: &Path) -> PathBuf {
    let git = |cwd: &Path, args: &[&str]| {
        let status = std::process::Command::new("git")
            .current_dir(cwd)
            .args(args)
            .status()
            .expect("git command");
        assert!(status.success(), "git {args:?} failed");
    };
    let work = dir.join("seed-work");
    let bare = dir.join("remote.git");
    std::fs::create_dir_all(&work).expect("create work dir");
    std::fs::create_dir_all(&bare).expect("create bare dir");
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["config", "user.name", "Test"]);
    git(&work, &["config", "user.email", "test@example.com"]);
    git(&work, &["config", "commit.gpgsign", "false"]);
    std::fs::write(work.join("seed.txt"), "seed\n").expect("write seed");
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-qm", "seed"]);
    git(&bare, &["init", "-q", "--bare", "-b", "main"]);
    git(
        &work,
        &["push", "-q", bare.to_str().expect("utf8 path"), "main:main"],
    );
    bare
}
