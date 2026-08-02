//! Synchronous, best-effort helpers for Drop and other non-async contexts.

use std::io;
use std::path::Path;
use std::process::Command;

/// Forget a workspace synchronously (`jj workspace forget <name>`). Guards
/// `name` before spawning, matching the async twin's
/// `JjApi::workspace_forget` (`reject_flag_like("workspace name", name)`) —
/// this helper has no async runtime to reuse that guard through, so it
/// re-derives an equivalent `io::Error` locally, keeping this function's
/// `std::io::Result<()>` signature.
pub fn workspace_forget(dir: &Path, name: &str) -> std::io::Result<()> {
    super::reject_flag_like("workspace name", name)
        .map_err(|err| io::Error::other(err.to_string()))?;
    let status = Command::new(super::BINARY)
        .current_dir(dir)
        .args(["workspace", "forget", name])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "`jj workspace forget` exited with {status}"
        )))
    }
}

/// Resolve the workspace *name* whose root matches `path`, synchronously —
/// for `Drop`, which can't `.await` the typed `workspace_list`/`workspace_root`.
/// Lists workspaces (`workspace list -T name`), then matches each
/// `workspace root --name <n>` against `path` (canonicalised, Windows
/// verbatim-prefix stripped).
///
/// The three outcomes are kept **distinct** so a `Drop` caller no longer has to
/// treat "the probe failed" as "no such workspace" — the old `Option` return
/// folded both into `None`, silently skipping cleanup that a real failure should
/// have surfaced (and hiding a workspace that *is* registered but couldn't be
/// placed):
/// - `Ok(Some(name))` — a registered workspace's root matched `path`.
/// - `Ok(None)` — jj listed the workspaces cleanly and none matched `path`: a
///   genuine miss, so the caller safely skips the forget (nothing to clean up).
/// - `Err(_)` — the probe itself could not answer: `jj` was missing / failed to
///   spawn, `workspace list` exited non-zero, or one or more *registered*
///   workspaces did not resolve via `workspace root --name` (so `path`'s absence
///   can't be proven). The caller can report it instead of silently doing nothing.
pub fn workspace_name_for_path(dir: &Path, path: &Path) -> io::Result<Option<String>> {
    let out = Command::new(super::BINARY)
        .current_dir(dir)
        // `--ignore-working-copy`: this is a **read-only** probe run from a Drop
        // guard, so it must NOT snapshot the working copy — a plain `workspace
        // list` takes the working-copy lock and writes a snapshot op (M10),
        // mutating the very repo being cleaned up and failing (→ leak) under lock
        // contention. The workspace list/root are static metadata, unaffected.
        // `--color never`: this raw probe bypasses `cmd_in`, so pin it here too
        // — `ui.color = "always"` would otherwise wrap names in ANSI escapes
        // and break the name->root match below (leaking the workspace on Drop).
        .args([
            "--ignore-working-copy",
            "workspace",
            "list",
            "-T",
            super::parse::WORKSPACE_NAME_TEMPLATE,
            "--color",
            "never",
        ])
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "`jj workspace list` exited with {} while resolving the workspace at {}",
            out.status,
            path.display(),
        )));
    }
    // Registered workspaces whose root did not resolve via `workspace root
    // --name` — remembered so a no-match doesn't silently hide a workspace we
    // merely failed to place (it may be the very one at `path`).
    let mut unresolved: Vec<String> = Vec::new();
    for name in super::parse::parse_workspace_names(&String::from_utf8_lossy(&out.stdout)) {
        let root = Command::new(super::BINARY)
            .current_dir(dir)
            .args([
                "--ignore-working-copy",
                "workspace",
                "root",
                "--name",
                &name,
                "--color",
                "never",
            ])
            .output();
        match root {
            Ok(r) if r.status.success() => {
                let p = super::parse::workspace_root_from_bytes(&r.stdout);
                if super::workspace_root_matches(&p, path) {
                    return Ok(Some(name));
                }
            }
            _ => unresolved.push(name),
        }
    }
    if unresolved.is_empty() {
        Ok(None)
    } else {
        Err(io::Error::other(format!(
            "could not resolve the workspace at {}: {} registered workspace(s) did not \
                 resolve via `jj workspace root --name` ({}); resolve or `jj workspace forget` \
                 them manually",
            path.display(),
            unresolved.len(),
            unresolved.join(", "),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A flag-shaped `name` is refused before `Command::status()` spawns
    // anything — the guard's own message (not a `jj`-produced "exited
    // with"/spawn-failure message) proves rejection happened up front, not
    // via a real `jj` run.
    #[test]
    fn workspace_forget_rejects_flag_like_name_before_spawn() {
        let err = workspace_forget(Path::new("/repo"), "--force")
            .expect_err("a flag-like workspace name must be refused");
        let message = err.to_string();
        assert!(
            message.contains("would be parsed as a flag"),
            "expected the guard's message, got: {message}"
        );
    }

    #[test]
    fn workspace_forget_rejects_empty_name_before_spawn() {
        let err = workspace_forget(Path::new("/repo"), "  ")
            .expect_err("an empty workspace name must be refused");
        let message = err.to_string();
        assert!(
            message.contains("would be parsed as a flag"),
            "expected the guard's message, got: {message}"
        );
    }
}
