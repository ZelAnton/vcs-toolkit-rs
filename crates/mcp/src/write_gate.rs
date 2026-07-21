//! The server's write policy: the [`WriteGate`] enum and the canonical
//! [`WRITE_TOOLS`] registry of mutating tool names. Both are re-exported from the
//! crate root (`vcs_mcp::WriteGate`, `vcs_mcp::WRITE_TOOLS`), so their public
//! paths are unchanged.

/// The canonical names of every **mutating** (write-gated) tool, in registration
/// order. The single source of truth for which names `--allow-tools` accepts: a
/// front-end can validate its allowlist against this set and reject a typo up
/// front (a misspelled name would otherwise be silently inert — it never matches a
/// real tool, so the intended write would stay disabled). `require_write`
/// debug-asserts every gated tool is listed here, so the two can't drift.
pub const WRITE_TOOLS: &[&str] = &[
    "repo_try_merge",
    "repo_commit",
    "repo_checkout",
    "repo_rebase",
    "repo_abort_in_progress",
    "repo_continue_in_progress",
    "repo_new_child",
    "repo_create_branch",
    "repo_delete_branch",
    "repo_rename_branch",
    "repo_fetch",
    "repo_push",
    "repo_create_worktree",
    "repo_remove_worktree",
    "forge_issue_create",
    "forge_pr_create",
    "forge_pr_merge",
    "forge_pr_close",
    "forge_pr_mark_ready",
    "forge_pr_comment",
    "forge_pr_edit",
    "forge_pr_approve",
    "forge_pr_request_changes",
    "forge_pr_checkout",
    "forge_release_create",
    "forge_release_delete",
];

/// Which mutating tools are callable — the server's write policy.
///
/// Read tools are always available; every mutating tool checks this gate by its
/// own tool name before doing anything.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WriteGate {
    /// No mutating tool is callable (the default).
    #[default]
    None,
    /// Every mutating tool is callable (`--allow-write`).
    All,
    /// Only the named mutating tools are callable (`--allow-tools a,b,c`).
    /// Tool names are the method names (e.g. `"repo_commit"`, the [`WRITE_TOOLS`]
    /// set); read tools are unaffected (always available). At the gate an unknown
    /// name simply never matches; the `vcs-mcp` binary additionally rejects an
    /// unknown `--allow-tools` name up front rather than building an inert entry.
    Set(std::collections::HashSet<String>),
}

impl WriteGate {
    /// Whether the mutating tool `name` may run under this gate.
    pub fn allows(&self, name: &str) -> bool {
        match self {
            WriteGate::All => true,
            WriteGate::None => false,
            WriteGate::Set(tools) => tools.contains(name),
        }
    }
}
