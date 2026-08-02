//! Synchronous, best-effort helpers for Drop and other non-async contexts.

use std::io;
use std::path::Path;
use std::process::Command;

/// The repository redirectors normally removed by the async client's
/// `managed_client!` profile. This direct `std::process` cleanup path cannot
/// inherit them: a hook-provided `GIT_DIR` could otherwise force destructive
/// worktree removal against another repository.
const REPO_REDIRECTORS: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
];

fn scrub_repo_redirectors(command: &mut Command) {
    for name in REPO_REDIRECTORS {
        command.env_remove(name);
    }
}

/// Remove a worktree synchronously (`git worktree remove [--force] <path>`);
/// see [`WorktreeRemove`](super::WorktreeRemove).
pub fn worktree_remove(dir: &Path, spec: super::WorktreeRemove) -> std::io::Result<()> {
    // Guard before spawning, matching the async twin's `reject_flag_like_path`
    // — a leading-`-` path would otherwise be misparsed as a flag by `git
    // worktree remove`. This helper has no async runtime to reuse the async
    // client's guard through, so it re-derives an equivalent `io::Error`
    // locally, keeping this function's `std::io::Result<()>` signature.
    super::reject_flag_like_path("worktree path", &spec.path)
        .map_err(|err| io::Error::other(err.to_string()))?;
    let mut cmd = Command::new(super::BINARY);
    cmd.current_dir(dir).args(["worktree", "remove"]);
    scrub_repo_redirectors(&mut cmd);
    if spec.force {
        cmd.arg("--force");
    }
    cmd.arg(&spec.path);
    let output = cmd.output()?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(std::io::Error::other(format!(
            "`git worktree remove` exited with {}: {}",
            output.status,
            stderr.trim(),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocking_cleanup_scrubs_every_repo_redirector() {
        let mut command = Command::new("git");
        scrub_repo_redirectors(&mut command);
        let removed: std::collections::BTreeSet<_> = command
            .get_envs()
            .filter_map(|(name, value)| value.is_none().then_some(name.to_str().unwrap()))
            .collect();
        assert_eq!(
            removed,
            REPO_REDIRECTORS.iter().copied().collect(),
            "every redirector must be removed regardless of Command's ordering"
        );
    }

    // A flag-shaped path is refused before `cmd.output()` spawns anything — the
    // guard's own message (not a `git`-produced "exited with"/spawn-failure
    // message) proves rejection happened up front, not via a real `git` run.
    #[test]
    fn worktree_remove_rejects_flag_like_path_before_spawn() {
        let err = worktree_remove(
            Path::new("/repo"),
            super::super::WorktreeRemove::new("--force"),
        )
        .expect_err("a flag-like worktree path must be refused");
        let message = err.to_string();
        assert!(
            message.contains("would be parsed as a flag"),
            "expected the guard's message, got: {message}"
        );
    }

    #[test]
    fn worktree_remove_rejects_empty_path_before_spawn() {
        let err = worktree_remove(Path::new("/repo"), super::super::WorktreeRemove::new("  "))
            .expect_err("an empty worktree path must be refused");
        let message = err.to_string();
        assert!(
            message.contains("would be parsed as a flag"),
            "expected the guard's message, got: {message}"
        );
    }

    // A non-UTF-8 path (valid on Unix) must not panic anywhere in the guard —
    // it may pass through to `cmd.output()` (which then fails to find a `git`
    // worktree at a nonsense path) or be refused, but never abort the check.
    #[cfg(unix)]
    #[test]
    fn worktree_remove_does_not_panic_on_non_utf8_path() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let bytes = vec![0xFFu8, 0xFEu8, b'x'];
        let path = std::path::PathBuf::from(OsString::from_vec(bytes));
        // Must return, not panic — the outcome itself (Ok/Err from a missing
        // `git`/repo) is not the point of this test.
        let _ = worktree_remove(Path::new("/repo"), super::super::WorktreeRemove::new(path));
    }

    // This raw `Command` helper has no ProcessRunner seam. Keep the end-to-end
    // assertion ignored, like the crate's other real-git tests, while comparing
    // against the exact stderr emitted by the installed binary.
    #[test]
    #[ignore = "requires the git binary"]
    fn worktree_remove_failure_includes_captured_stderr() {
        let temp = vcs_testkit::TempDir::new("blocking-worktree-remove-failure");
        let dir = temp.path();
        let path = "missing-worktree";
        let mut command = Command::new(super::super::BINARY);
        command.current_dir(dir).args(["worktree", "remove", path]);
        scrub_repo_redirectors(&mut command);
        let expected = command.output().expect("run git directly");
        assert!(
            !expected.status.success(),
            "the direct git command must fail for this diagnostic test"
        );
        let expected_stderr = String::from_utf8_lossy(&expected.stderr).trim().to_owned();
        assert!(
            !expected_stderr.is_empty(),
            "the failing git command must emit stderr"
        );

        let err = worktree_remove(dir, super::super::WorktreeRemove::new(path))
            .expect_err("remove must fail");
        assert!(
            err.to_string().contains(&expected_stderr),
            "error must retain git stderr; expected {expected_stderr:?}, got: {err}"
        );
    }
}
