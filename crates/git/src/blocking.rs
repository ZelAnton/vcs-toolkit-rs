//! Synchronous, best-effort helpers for Drop and other non-async contexts.

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
    let mut cmd = Command::new(super::BINARY);
    cmd.current_dir(dir).args(["worktree", "remove"]);
    scrub_repo_redirectors(&mut cmd);
    if spec.force {
        cmd.arg("--force");
    }
    cmd.arg(&spec.path);
    let status = cmd.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "`git worktree remove` exited with {status}"
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
}
