#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-mcp` — a [Model Context Protocol](https://modelcontextprotocol.io)
//! server that exposes the toolkit's typed git/jj + forge operations as
//! agent-callable **tools**.
//!
//! An agent harness (Claude Code, an IDE assistant, any MCP client) drives a
//! repository — and its forge — through structured, schema-validated calls
//! instead of raw shell. Each tool wraps one operation on the [`vcs_core::Repo`]
//! (git/jj) or [`vcs_forge::Forge`] (GitHub/GitLab/Gitea) facade and returns its
//! DTO as JSON. Built on the [`rmcp`] SDK; the `vcs-mcp` binary serves over
//! stdio. It is the workspace's first binary crate — a thin binary over a
//! hermetically-testable library.
//!
//! # The surface
//!
//! - **[`VcsMcpServer`]** — the server: an `rmcp` [`ServerHandler`] bound to one
//!   repository and (optionally) its forge. Build it with
//!   [`new`](VcsMcpServer::new), then `serve` it over an `rmcp` transport. Held
//!   as object-safe trait handles, so it's runner-agnostic and `Clone` is cheap
//!   (`Arc`).
//! - **[`WriteGate`]** — the server's write policy: [`None`](WriteGate::None)
//!   (read-only, the default), [`All`](WriteGate::All) (`--allow-write`), or
//!   [`Set`](WriteGate::Set) (a per-tool allowlist). [`allows`](WriteGate::allows)
//!   answers whether a named mutating tool may run.
//! - **Tools** are the `#[tool]` methods on [`VcsMcpServer`]: the `repo_*` group
//!   ([`repo_snapshot`](VcsMcpServer::repo_snapshot),
//!   [`repo_commit`](VcsMcpServer::repo_commit), …) over the `Repo` facade, and
//!   the `forge_*` group ([`forge_pr_list`](VcsMcpServer::forge_pr_list),
//!   [`forge_pr_create`](VcsMcpServer::forge_pr_create), …) over the `Forge` one.
//! - **Parameter structs** — one `Deserialize` + `JsonSchema` struct per
//!   tool-with-arguments ([`CommitParams`], [`PrCreateParams`],
//!   [`MergeStrategyArg`], …); their schema is the tool's advertised input schema.
//!
//! # Tools & the write gate
//!
//! Read tools are **always available**; mutating tools are **gated**. A gated tool
//! rejects the call — naming itself, before spawning anything — unless the
//! [`WriteGate`] covers it. Most are annotated `destructiveHint`; `repo_try_merge`
//! is gated too (it spawns a real, content-materializing trial merge) but rolls
//! back, so it's annotated non-destructive/idempotent. `--allow-write` enables
//! every gated tool; `--allow-tools repo_commit,forge_pr_create` enables only the
//! named ones; read tools are unaffected either way. Tool names are the method
//! names (e.g. `"repo_commit"`).
//! This is the crate's core safety property: a default server is read-only, and a
//! client can surface a confirmation prompt off the `destructiveHint`.
//!
//! # Recipes
//!
//! Build a [`VcsMcpServer`] and serve it over a transport (the binary uses stdio):
//!
//! ```no_run
//! # use vcs_core::Repo;
//! # use vcs_mcp::{VcsMcpServer, WriteGate};
//! # use rmcp::{ServiceExt, transport::stdio};
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let repo = Repo::discover(".")?;
//! let server = VcsMcpServer::new(repo, None, WriteGate::None); // read-only
//! server.serve(stdio()).await?.waiting().await?;
//! # Ok(()) }
//! ```
//!
//! Or point an MCP client at the installed binary — read-only over one repo, or
//! with mutations enabled and a forge forced:
//!
//! ```text
//! vcs-mcp --repo /path/to/repo
//! vcs-mcp --repo /path/to/repo --forge github --allow-tools repo_commit,repo_push
//! ```
//!
//! When `--forge` is omitted the forge is auto-detected from the `origin` remote;
//! a pure-jj repo with no recognised remote resolves to no forge (the `repo_*`
//! tools still work, the `forge_*` tools return a clear error).
//!
//! # In-depth guide
//!
//! Beyond this page, this crate ships a full how-to guide — rendered on docs.rs
//! from `docs/` — covering the CLI flags, the full tool catalogue, forge
//! auto-detection, and the binary's hardening/timeout safety model. See the
//! [`guide`] module.

use std::path::Path;
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo};
use rmcp::schemars;
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;
use vcs_core::{Repo, VcsRepo};
use vcs_forge::{Forge, ForgeApi};

// --- Tool parameter structs (Deserialize + JsonSchema → the MCP input schema) --

/// Switch the working copy to a branch/bookmark/revision.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CheckoutParams {
    /// The branch, bookmark, or revision to switch to (git checkout / jj edit).
    pub reference: String,
}

/// Commit exactly these paths.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CommitParams {
    /// Repo-relative paths to commit (and nothing else).
    pub paths: Vec<String>,
    /// The commit message.
    pub message: String,
}

/// Push a branch/bookmark to `origin`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PushParams {
    /// The existing local branch (git) / bookmark (jj) to push.
    pub branch: String,
}

/// Probe a merge.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TryMergeParams {
    /// The branch/revision to probe merging into the current work.
    pub source: String,
}

/// Create a worktree/workspace.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateWorktreeParams {
    /// Filesystem path for the new worktree/workspace.
    pub path: String,
    /// The new branch/bookmark to create on it.
    pub branch: String,
    /// The base revision to start it from.
    pub base: String,
}

/// Remove a worktree/workspace.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RemoveWorktreeParams {
    /// Filesystem path of the worktree/workspace to remove.
    pub path: String,
    /// Force removal even when the worktree has uncommitted changes. Without it,
    /// a worktree with local changes is refused on **both** git and jj. The
    /// repository's main worktree/workspace is always refused (deleting it would
    /// destroy the repo), regardless of this flag.
    #[serde(default)]
    pub force: bool,
}

/// A pull/merge request by number.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrNumberParams {
    /// The PR/MR number (GitLab uses the project-scoped `iid`).
    pub number: u64,
}

/// Open a pull/merge request.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrCreateParams {
    /// Title.
    pub title: String,
    /// Body / description.
    pub body: String,
    /// Source/head branch; omit for the current branch.
    #[serde(default)]
    pub source: Option<String>,
    /// Target/base branch; omit for the repo default.
    #[serde(default)]
    pub target: Option<String>,
}

/// Merge a pull/merge request.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrMergeParams {
    /// The PR/MR number.
    pub number: u64,
    /// Merge strategy.
    pub strategy: MergeStrategyArg,
}

/// Close a pull/merge request.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrCloseParams {
    /// The PR/MR number.
    pub number: u64,
    /// Also delete the source branch (GitHub only).
    #[serde(default)]
    pub delete_branch: bool,
}

/// Post a comment to an existing pull/merge request.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrCommentParams {
    /// The PR/MR number.
    pub number: u64,
    /// The markdown comment body.
    pub body: String,
}

/// Edit a pull/merge request's title and/or body.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrEditParams {
    /// The PR/MR number.
    pub number: u64,
    /// The new title; omit (or null) to leave the title alone.
    #[serde(default)]
    pub title: Option<String>,
    /// The new body / description; omit (or null) to leave the body alone.
    /// At least one of `title` or `body` must be set — the facade rejects
    /// both-absent with an `invalid_params` error before any spawn.
    #[serde(default)]
    pub body: Option<String>,
}

/// An issue by number.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IssueNumberParams {
    /// The issue number (GitLab uses the project-scoped `iid`).
    pub number: u64,
}

/// Open an issue.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IssueCreateParams {
    /// Title.
    pub title: String,
    /// Body / description.
    pub body: String,
}

/// A release by tag.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReleaseTagParams {
    /// The release's Git tag.
    pub tag: String,
}

/// How [`forge_pr_merge`](VcsMcpServer::forge_pr_merge) merges.
#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MergeStrategyArg {
    /// A merge commit.
    Merge,
    /// Squash into one commit.
    Squash,
    /// Rebase onto the target.
    Rebase,
}

impl From<MergeStrategyArg> for vcs_forge::MergeStrategy {
    fn from(s: MergeStrategyArg) -> Self {
        match s {
            MergeStrategyArg::Merge => vcs_forge::MergeStrategy::Merge,
            MergeStrategyArg::Squash => vcs_forge::MergeStrategy::Squash,
            MergeStrategyArg::Rebase => vcs_forge::MergeStrategy::Rebase,
        }
    }
}

// --- The server --------------------------------------------------------------

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

/// An MCP server over a single repository (and, optionally, its forge). Held as
/// object-safe trait handles, so it's runner-agnostic; clone is cheap (`Arc`).
/// Construct with [`new`](Self::new).
#[derive(Clone)]
pub struct VcsMcpServer {
    repo: Arc<dyn VcsRepo>,
    forge: Option<Arc<dyn ForgeApi>>,
    writes: WriteGate,
    tool_router: ToolRouter<Self>,
    /// Serializes the **repo**-mutating tools. rmcp dispatches a task per request,
    /// so without this two concurrent mutations (e.g. `repo_try_merge`'s materialize-
    /// then-rollback racing a `repo_commit`) could interleave and lose one's work,
    /// or collide on the repo lock. Forge tools are *predominantly* remote calls to
    /// a server that serializes on its side (and MCP clients typically issue tool
    /// calls sequentially), so they aren't gated by this — this closes the local
    /// repo-state race, the one R1 targets.
    write_lock: Arc<tokio::sync::Mutex<()>>,
}

impl VcsMcpServer {
    /// Build a server bound to `repo`, with an optional `forge` (PR/MR tools), and
    /// a [`WriteGate`] controlling which mutating tools are callable.
    pub fn new(repo: Repo, forge: Option<Forge>, writes: WriteGate) -> Self {
        Self::from_handles(
            Arc::new(repo),
            forge.map(|f| Arc::new(f) as Arc<dyn ForgeApi>),
            writes,
        )
    }

    /// Build from already-erased handles — the seam tests use to inject a `Repo`
    /// over a fake `ProcessRunner`.
    fn from_handles(
        repo: Arc<dyn VcsRepo>,
        forge: Option<Arc<dyn ForgeApi>>,
        writes: WriteGate,
    ) -> Self {
        Self {
            repo,
            forge,
            writes,
            tool_router: Self::tool_router(),
            write_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Reject the mutating tool `tool` when the write gate doesn't cover it.
    fn require_write(&self, tool: &str) -> Result<(), ErrorData> {
        debug_assert!(
            WRITE_TOOLS.contains(&tool),
            "write-gated tool {tool:?} is missing from WRITE_TOOLS — keep them in sync"
        );
        if self.writes.allows(tool) {
            Ok(())
        } else {
            Err(ErrorData::invalid_params(
                format!(
                    "write tool {tool:?} is disabled; restart the server with --allow-write \
                     (all mutations) or --allow-tools naming it"
                ),
                None,
            ))
        }
    }

    /// Gate a **repo**-mutating tool: check the write gate, then acquire the
    /// per-repo write lock. Hold the returned guard for the tool's duration so
    /// concurrent repo mutations run one at a time (rmcp dispatches a task per
    /// request). Returns the gate error without taking the lock when disabled.
    async fn begin_repo_write(
        &self,
        tool: &str,
    ) -> Result<tokio::sync::MutexGuard<'_, ()>, ErrorData> {
        self.require_write(tool)?;
        Ok(self.write_lock.lock().await)
    }

    /// The configured forge, or a clear error when none was resolved.
    fn forge(&self) -> Result<&dyn ForgeApi, ErrorData> {
        self.forge.as_deref().ok_or_else(|| {
            ErrorData::invalid_params(
                "no forge is configured for this repository (pass --forge github|gitlab|gitea)"
                    .to_string(),
                None,
            )
        })
    }
}

/// Encode a serializable value as a JSON text result.
fn ok_json<T: serde::Serialize>(value: &T) -> Result<CallToolResult, ErrorData> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

/// Map a `vcs-core` error into an MCP error. The facade reports a refused
/// *input* (e.g. `commit_paths` with an empty path set) as an
/// `InvalidInput` io error — that's the client's call to fix, so surface it as
/// an invalid-params error rather than an internal one.
fn core_err(e: vcs_core::Error) -> ErrorData {
    match &e {
        vcs_core::Error::Io(io) if io.kind() == std::io::ErrorKind::InvalidInput => {
            ErrorData::invalid_params(e.to_string(), None)
        }
        _ => ErrorData::internal_error(e.to_string(), None),
    }
}

/// Map a `vcs-forge` error into an MCP error — an `Unsupported` op or an
/// `InvalidInput` (the facade's pre-spawn refusal path) is a client-facing
/// invalid-request; a forge/network failure is internal.
fn forge_err(e: vcs_forge::Error) -> ErrorData {
    if e.is_unsupported() || matches!(e, vcs_forge::Error::InvalidInput(_)) {
        ErrorData::invalid_params(e.to_string(), None)
    } else {
        ErrorData::internal_error(e.to_string(), None)
    }
}

#[tool_router]
impl VcsMcpServer {
    // --- repo: read --------------------------------------------------------

    #[tool(
        description = "A batched snapshot of the repo state: branch, upstream, ahead/behind, HEAD, dirtiness, change count, conflict, and operation state.",
        annotations(read_only_hint = true)
    )]
    pub async fn repo_snapshot(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.snapshot().await.map_err(core_err)?)
    }

    #[tool(
        description = "Which backend (git/jj), the repository root, the working directory, and the configured forge (if any).",
        annotations(read_only_hint = true)
    )]
    pub async fn repo_info(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&serde_json::json!({
            "backend": self.repo.kind().as_str(),
            "root": self.repo.root().to_string_lossy(),
            "cwd": self.repo.cwd().to_string_lossy(),
            "forge": self.forge.as_ref().map(|f| f.kind().as_str()),
        }))
    }

    #[tool(
        description = "The working-copy changes (added/modified/deleted/renamed paths).",
        annotations(read_only_hint = true)
    )]
    pub async fn repo_status(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.changed_files().await.map_err(core_err)?)
    }

    #[tool(
        description = "Aggregate insertion/deletion/file counts for the working copy.",
        annotations(read_only_hint = true)
    )]
    pub async fn repo_diff_stat(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.diff_stat().await.map_err(core_err)?)
    }

    #[tool(
        description = "Local branch (git) / bookmark (jj) names.",
        annotations(read_only_hint = true)
    )]
    pub async fn repo_branches(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.local_branches().await.map_err(core_err)?)
    }

    #[tool(
        description = "The current branch/bookmark (null when detached/unset).",
        annotations(read_only_hint = true)
    )]
    pub async fn repo_current_branch(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.current_branch().await.map_err(core_err)?)
    }

    #[tool(
        description = "Paths with unresolved merge conflicts (repo-relative, '/'-separated).",
        annotations(read_only_hint = true)
    )]
    pub async fn repo_conflicts(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.conflicted_files().await.map_err(core_err)?)
    }

    #[tool(
        description = "Attached worktrees (git) / workspaces (jj).",
        annotations(read_only_hint = true)
    )]
    pub async fn repo_worktrees(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.list_worktrees().await.map_err(core_err)?)
    }

    // NOTE: the absence of `read_only_hint = true` here is DELIBERATE — do not add
    // it back. `try_merge` materializes a real (rolled-back) trial merge, so it is
    // write-gated below; marking it read-only would re-expose it in the default
    // read-only mode and reopen the untrusted-repo filter/textconv code-exec path.
    #[tool(
        description = "Probe whether merging `source` into the current work would conflict, WITHOUT leaving a trace (the probe is always rolled back). It spawns a REAL trial merge that materializes working-tree content, so — like checkout — it is write-gated: on an untrusted repository that materialization can run repo-local `filter`/`textconv` drivers, which the hardened client does not sandbox. Enable it with `--allow-write` or `--allow-tools repo_try_merge`.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_try_merge(
        &self,
        Parameters(p): Parameters<TryMergeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_try_merge").await?;
        ok_json(&self.repo.try_merge(&p.source).await.map_err(core_err)?)
    }

    // --- repo: mutations (gated) ------------------------------------------

    #[tool(
        description = "Commit exactly the given paths with a message (git commit --only / jj commit <filesets>). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_commit(
        &self,
        Parameters(p): Parameters<CommitParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_commit").await?;
        self.repo
            .commit_paths(&p.paths, &p.message)
            .await
            .map_err(core_err)?;
        ok_json(&serde_json::json!({ "committed_paths": p.paths.len() }))
    }

    #[tool(
        description = "Switch the working copy to a branch/bookmark/revision (git checkout / jj edit). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_checkout(
        &self,
        Parameters(p): Parameters<CheckoutParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_checkout").await?;
        self.repo.checkout(&p.reference).await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "checked_out": p.reference }))
    }

    #[tool(
        description = "Fetch from the default remote (git fetch / jj git fetch). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_fetch(&self) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_fetch").await?;
        self.repo.fetch().await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "fetched": true }))
    }

    #[tool(
        description = "Push an existing branch/bookmark to origin (git push -u origin <branch> / jj git push -b <branch>). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_push(
        &self,
        Parameters(p): Parameters<PushParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_push").await?;
        self.repo.push(&p.branch).await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "pushed": p.branch }))
    }

    #[tool(
        description = "Create a worktree/workspace at `path` on a new `branch` from `base`. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_create_worktree(
        &self,
        Parameters(p): Parameters<CreateWorktreeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_create_worktree").await?;
        let spec = vcs_core::WorktreeCreate::new(p.path, p.branch).base(p.base);
        let outcome = self.repo.create_worktree(spec).await.map_err(core_err)?;
        ok_json(&outcome)
    }

    #[tool(
        description = "Remove the worktree/workspace at `path`. Without `force`, a worktree with uncommitted changes is refused; the repository's main worktree/workspace is always refused. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_remove_worktree(
        &self,
        Parameters(p): Parameters<RemoveWorktreeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_remove_worktree").await?;
        let mut spec = vcs_core::WorktreeRemove::new(Path::new(&p.path));
        if p.force {
            spec = spec.force();
        }
        self.repo.remove_worktree(spec).await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "removed": p.path }))
    }

    // --- forge: read -------------------------------------------------------

    #[tool(
        description = "Whether the forge CLI reports an authenticated session.",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_auth_status(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.auth_status().await.map_err(forge_err)?)
    }

    #[tool(
        description = "The repository/project on the configured forge (Unsupported on Gitea).",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_repo_view(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.repo_view().await.map_err(forge_err)?)
    }

    #[tool(
        description = "Open pull/merge requests on the configured forge (up to 100; ~50 on Gitea).",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_pr_list(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.pr_list().await.map_err(forge_err)?)
    }

    #[tool(
        description = "A single pull/merge request by number.",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_pr_view(
        &self,
        Parameters(p): Parameters<PrNumberParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.pr_view(p.number).await.map_err(forge_err)?)
    }

    #[tool(
        description = "The PR/MR's coarse CI status (Unsupported on Gitea).",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_pr_checks(
        &self,
        Parameters(p): Parameters<PrNumberParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.pr_checks(p.number).await.map_err(forge_err)?)
    }

    #[tool(
        description = "Open issues on the configured forge (up to 100; ~50 on Gitea).",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_issue_list(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.issue_list().await.map_err(forge_err)?)
    }

    #[tool(
        description = "A single issue by number, with body and URL filled.",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_issue_view(
        &self,
        Parameters(p): Parameters<IssueNumberParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(
            &self
                .forge()?
                .issue_view(p.number)
                .await
                .map_err(forge_err)?,
        )
    }

    #[tool(
        description = "Releases on the configured forge, newest first (up to 100; ~50 on Gitea).",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_release_list(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.release_list().await.map_err(forge_err)?)
    }

    #[tool(
        description = "A single release by tag (Unsupported on Gitea — filter forge_release_list instead).",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_release_view(
        &self,
        Parameters(p): Parameters<ReleaseTagParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(
            &self
                .forge()?
                .release_view(&p.tag)
                .await
                .map_err(forge_err)?,
        )
    }

    // --- forge: mutations (gated) -----------------------------------------

    #[tool(
        description = "Open an issue, returning the CLI's output (the URL on success). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_issue_create(
        &self,
        Parameters(p): Parameters<IssueCreateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_issue_create")?;
        // No MCP-layer argv guard on `title`/`body`: every backend passes both in
        // a flag-VALUE slot (`--title`/`--body`/`--description`), so a leading `-`
        // is safe — uniform with `forge_pr_comment`/`forge_pr_edit` (T-013). Any
        // genuine bare-positional slot is guarded in its own wrapper.
        let out = self
            .forge()?
            .issue_create(vcs_forge::IssueCreate::new(p.title, p.body))
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "output": out }))
    }

    #[tool(
        description = "Open a pull/merge request, returning the CLI's output (the URL on success). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_create(
        &self,
        Parameters(p): Parameters<PrCreateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_create")?;
        // No MCP-layer argv guard on `title`/`body`: both ride in flag-VALUE slots
        // on every backend (`--title`/`--body`/`--description`), so a leading `-`
        // is safe — uniform with `forge_pr_comment`/`forge_pr_edit` (T-013).
        let mut spec = vcs_forge::PrCreate::new(p.title, p.body);
        if let Some(source) = p.source {
            spec = spec.source(source);
        }
        if let Some(target) = p.target {
            spec = spec.target(target);
        }
        let out = self.forge()?.pr_create(spec).await.map_err(forge_err)?;
        ok_json(&serde_json::json!({ "output": out }))
    }

    #[tool(
        description = "Merge a pull/merge request with a strategy (merge|squash|rebase). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_merge(
        &self,
        Parameters(p): Parameters<PrMergeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_merge")?;
        self.forge()?
            .pr_merge(p.number, p.strategy.into())
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "merged": p.number }))
    }

    #[tool(
        description = "Close a pull/merge request without merging. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_close(
        &self,
        Parameters(p): Parameters<PrCloseParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_close")?;
        let mut spec = vcs_forge::PrClose::new(p.number);
        if p.delete_branch {
            spec = spec.delete_branch();
        }
        self.forge()?.pr_close(spec).await.map_err(forge_err)?;
        ok_json(&serde_json::json!({ "closed": p.number }))
    }

    #[tool(
        description = "Mark a draft pull/merge request as ready for review. Requires write access (--allow-write, or --allow-tools naming this tool). `Unsupported` on Gitea (`tea` has no ready command).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_mark_ready(
        &self,
        Parameters(p): Parameters<PrNumberParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_mark_ready")?;
        self.forge()?
            .pr_mark_ready(p.number)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "ready": p.number }))
    }

    #[tool(
        description = "Post a comment to an existing pull/merge request, returning the CLI's output. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_comment(
        &self,
        Parameters(p): Parameters<PrCommentParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_comment")?;
        // No MCP-layer argv guard on `body`: argv-injection safety is a
        // wrapper-layer concern, and only the wrapper knows which argv slot a
        // value lands in. GitHub (`gh pr comment --body <body>`) and GitLab
        // (`glab mr note -m <body>`) put the body in a flag-VALUE slot, where a
        // leading `-` is safe and typical for Markdown (a `- item` bullet list or
        // `---` rule); Gitea's `tea comment <n> <body>` is the one bare positional,
        // and the Gitea wrapper already guards it with `reject_flag_like`. A blanket
        // leading-`-` refusal here wrongly rejected legitimate Markdown on
        // GitHub/GitLab (T-013). (An empty body is still rejected by the facade
        // itself, before any spawn.)
        let out = self
            .forge()?
            .pr_comment(p.number, &p.body)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "output": out }))
    }

    #[tool(
        description = "Edit a pull/merge request's title and/or body. At least one of `title` or `body` must be set; both absent is rejected up front as an invalid-params error. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_edit(
        &self,
        Parameters(p): Parameters<PrEditParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_edit")?;
        // No MCP-layer argv guard on `title`/`body` (see `forge_pr_comment`): every
        // backend passes both in a flag-VALUE slot (`gh`/`tea` `--title`/`--body`/
        // `--description`, `glab mr update --title`/`--description`), so a leading
        // `-` is safe here — refusing it wrongly rejected legitimate Markdown
        // titles/bodies (T-013). The facade still rejects both-`None` with
        // `InvalidInput` before spawning — a backstop the MCP tool surfaces as
        // `invalid_params`.
        let mut edit = vcs_forge::PrEdit::new();
        if let Some(title) = p.title {
            edit = edit.title(title);
        }
        if let Some(body) = p.body {
            edit = edit.body(body);
        }
        self.forge()?
            .pr_edit(p.number, edit)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "edited": p.number }))
    }

    #[tool(
        description = "The forge's identity and flat capability map (read-only). Returns `{ kind, capabilities: { pr_create, pr_comment, pr_edit, pr_checks, pr_merge, issue_create, authed } }` for the configured forge. Note: for GitLab, `authed` is best-effort (`glab auth status` can report authed when it is not); a real API call is the sure test.",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_info(&self) -> Result<CallToolResult, ErrorData> {
        let forge = self.forge()?;
        let kind = forge.kind();
        let capabilities = forge.capabilities().await.map_err(forge_err)?;
        ok_json(&serde_json::json!({
            "kind": kind.as_str(),
            "capabilities": capabilities,
        }))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for VcsMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            // Identify as vcs-mcp on the wire. `ServerInfo::new` defaults the
            // server_info to `Implementation::from_build_env()`, whose `env!` is
            // expanded in *rmcp's* crate — so without this it advertises "rmcp".
            .with_server_info(Implementation::new("vcs-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Drive a git/jj repository (and its forge) through typed tools. Read tools \
                 (repo_*/forge_* queries) are always available; mutating tools require the server \
                 to have been started with --allow-write (all mutations) or --allow-tools \
                 name,... (a per-tool allowlist), and reject calls otherwise.",
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::testing::{Reply, ScriptedRunner};
    use vcs_core::vcs_git::Git;

    /// A git-backed server over a scripted runner — no real binary, no forge.
    fn git_server(runner: ScriptedRunner, writes: WriteGate) -> VcsMcpServer {
        let repo: Arc<dyn VcsRepo> =
            Arc::new(Repo::from_git("/repo", "/repo", Git::with_runner(runner)));
        VcsMcpServer::from_handles(repo, None, writes)
    }

    /// The JSON of a successful tool result (serialised wire form).
    fn result_json(r: &CallToolResult) -> String {
        serde_json::to_string(r).expect("CallToolResult serialises")
    }

    // A read tool calls the facade and returns its DTO as JSON.
    #[tokio::test]
    async fn read_tool_returns_dto_json() {
        let server = git_server(
            ScriptedRunner::new().on(["git", "symbolic-ref"], Reply::ok("main\n")),
            WriteGate::None,
        );
        let out = server.repo_current_branch().await.expect("tool ok");
        assert!(result_json(&out).contains("main"), "{}", result_json(&out));
    }

    // R1: `begin_repo_write` checks the gate and, when allowed, *holds* the per-repo
    // write lock for the caller's duration — so concurrent repo mutations serialize.
    // A disabled write returns the gate error without taking the lock.
    #[tokio::test]
    async fn begin_repo_write_gates_then_holds_the_lock() {
        let server = git_server(ScriptedRunner::new(), WriteGate::All);
        let guard = server
            .begin_repo_write("repo_commit")
            .await
            .expect("allowed → guard");
        assert!(
            server.write_lock.try_lock().is_err(),
            "the write lock is held while a guard is outstanding"
        );
        drop(guard);
        assert!(
            server.write_lock.try_lock().is_ok(),
            "the lock is released once the guard drops"
        );

        // Read-only server: the gate rejects before any lock is taken.
        let ro = git_server(ScriptedRunner::new(), WriteGate::None);
        assert!(
            ro.begin_repo_write("repo_commit").await.is_err(),
            "a gated write is rejected"
        );
        assert!(
            ro.write_lock.try_lock().is_ok(),
            "no lock is taken on the rejected path"
        );
    }

    // Read tools work even when writes are disabled (the default).
    #[tokio::test]
    async fn read_tool_works_in_readonly_mode() {
        let server = git_server(
            ScriptedRunner::new().on(["git", "status"], Reply::ok(" M a.rs\0")),
            WriteGate::None,
        );
        let out = server.repo_status().await.expect("status ok");
        assert!(result_json(&out).contains("a.rs"));
    }

    // A mutation tool is gated when writes are disabled — it errors WITHOUT
    // reaching the runner. The scripted runner has NO `checkout` rule, so if the
    // gate failed and the tool spawned, the call would error differently than the
    // gate's `--allow-write` message.
    #[tokio::test]
    async fn mutation_is_gated_without_allow_write() {
        let server = git_server(ScriptedRunner::new(), WriteGate::None);
        let err = server
            .repo_checkout(Parameters(CheckoutParams {
                reference: "feat".into(),
            }))
            .await
            .expect_err("gated");
        assert!(
            format!("{err:?}").contains("allow-write"),
            "error should mention --allow-write: {err:?}"
        );
    }

    // `repo_try_merge` is write-gated: it spawns a real trial merge that
    // materializes working-tree content (which on an untrusted repo can run
    // repo-local filter/textconv drivers), so it must NOT be callable in the default
    // read-only mode — unlike the genuinely read-only tools.
    #[tokio::test]
    async fn try_merge_is_write_gated() {
        let server = git_server(ScriptedRunner::new(), WriteGate::None);
        let err = server
            .repo_try_merge(Parameters(TryMergeParams {
                source: "feat".into(),
            }))
            .await
            .expect_err("try_merge must be gated in read-only mode");
        assert!(
            format!("{err:?}").contains("allow-write"),
            "error should mention --allow-write: {err:?}"
        );
    }

    // With writes enabled, the same tool reaches the runner and returns success.
    #[tokio::test]
    async fn mutation_reaches_runner_with_allow_write() {
        let server = git_server(
            ScriptedRunner::new().on(["git", "checkout"], Reply::ok("")),
            WriteGate::All,
        );
        let out = server
            .repo_checkout(Parameters(CheckoutParams {
                reference: "feat".into(),
            }))
            .await
            .expect("checkout ok");
        assert!(result_json(&out).contains("feat"));
    }

    // repo_push is a gated mutation: blocked read-only, and with writes enabled
    // it drives the facade's `push -u origin <branch>` (only ["push"] is
    // scripted, so a different argv shape would error).
    #[tokio::test]
    async fn repo_push_is_gated_and_pushes_branch() {
        let server = git_server(ScriptedRunner::new(), WriteGate::None);
        let err = server
            .repo_push(Parameters(PushParams {
                branch: "feature".into(),
            }))
            .await
            .expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

        let server = git_server(
            ScriptedRunner::new().on(["git", "push"], Reply::ok("")),
            WriteGate::All,
        );
        let out = server
            .repo_push(Parameters(PushParams {
                branch: "feature".into(),
            }))
            .await
            .expect("push ok");
        assert!(result_json(&out).contains("feature"));
    }

    // A Set gate admits exactly the named mutations: the listed tool runs, an
    // unlisted one is rejected (naming itself), and read tools stay available.
    #[tokio::test]
    async fn allow_tools_set_gates_per_tool() {
        let gate = WriteGate::Set(
            ["repo_checkout".to_string()]
                .into_iter()
                .collect::<std::collections::HashSet<_>>(),
        );
        let server = git_server(
            ScriptedRunner::new()
                .on(["git", "checkout"], Reply::ok(""))
                .on(["git", "symbolic-ref"], Reply::ok("main\n")),
            gate,
        );

        // Listed mutation runs.
        server
            .repo_checkout(Parameters(CheckoutParams {
                reference: "feat".into(),
            }))
            .await
            .expect("listed tool allowed");

        // Unlisted mutation is rejected, naming the tool.
        let err = server.repo_fetch().await.expect_err("unlisted gated");
        assert!(format!("{err:?}").contains("repo_fetch"), "{err:?}");

        // Read tools are unaffected by the allowlist.
        server.repo_current_branch().await.expect("read tool ok");
    }

    // The facade's refused-input errors (here: an empty `paths` set, which the
    // facade rejects up front) surface as INVALID_PARAMS — the client's mistake
    // to fix — not as an internal server error.
    #[tokio::test]
    async fn refused_input_surfaces_as_invalid_params() {
        let server = git_server(ScriptedRunner::new(), WriteGate::All);
        let err = server
            .repo_commit(Parameters(CommitParams {
                paths: vec![],
                message: "msg".into(),
            }))
            .await
            .expect_err("empty paths refused");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("at least one path"),
            "unexpected message: {}",
            err.message
        );
    }

    // Forge tools report a clear error when no forge was configured.
    #[tokio::test]
    async fn forge_tools_error_without_a_forge() {
        let server = git_server(ScriptedRunner::new(), WriteGate::All);
        let err = server.forge_pr_list().await.expect_err("no forge");
        assert!(
            format!("{err:?}").contains("no forge"),
            "should mention no forge: {err:?}"
        );
    }

    // The forge issue tools route to the forge handle: the read tool works in
    // read-only mode and returns the unified DTO JSON; the create tool is gated.
    #[tokio::test]
    async fn forge_issue_tools_route_and_gate() {
        let json = r#"[{"number":3,"title":"Bug","state":"OPEN"}]"#;
        let gh = vcs_forge::vcs_github::GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "issue", "list"], Reply::ok(json)),
        );
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

        let out = server.forge_issue_list().await.expect("issue list ok");
        assert!(result_json(&out).contains("Bug"));

        let err = server
            .forge_issue_create(Parameters(IssueCreateParams {
                title: "t".into(),
                body: "b".into(),
            }))
            .await
            .expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");
    }

    // A forge op the backend can't do (tea has no single-release view) surfaces
    // as INVALID_PARAMS — the client's "this forge can't do that" — without
    // spawning anything (the runner has no rules, so a spawn would error
    // differently).
    #[tokio::test]
    async fn forge_release_view_unsupported_maps_to_invalid_params() {
        let tea = vcs_forge::vcs_gitea::Gitea::with_runner(ScriptedRunner::new());
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_gitea("/repo", tea));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

        let err = server
            .forge_release_view(Parameters(ReleaseTagParams { tag: "v1".into() }))
            .await
            .expect_err("unsupported on gitea");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("release_view"), "{}", err.message);
    }

    // The two new mutating tools (`forge_pr_comment`, `forge_pr_edit`) are
    // gated like the existing `forge_pr_create` / `forge_pr_close`: the
    // runner has no `pr comment` / `pr edit` rule, so a leak-through would
    // error differently than the gate's `--allow-write` message.
    #[tokio::test]
    async fn forge_pr_comment_is_gated() {
        let gh = vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new());
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

        let err = server
            .forge_pr_comment(Parameters(PrCommentParams {
                number: 7,
                body: "hi".into(),
            }))
            .await
            .expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");
    }

    #[tokio::test]
    async fn forge_pr_edit_is_gated() {
        let gh = vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new());
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

        let err = server
            .forge_pr_edit(Parameters(PrEditParams {
                number: 7,
                title: Some("T".into()),
                body: None,
            }))
            .await
            .expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");
    }

    #[tokio::test]
    async fn forge_pr_mark_ready_is_gated() {
        let gh = vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new());
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

        let err = server
            .forge_pr_mark_ready(Parameters(PrNumberParams { number: 7 }))
            .await
            .expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");
    }

    // T-013: on GitHub a `body` that begins with `-` is a legitimate Markdown
    // value (a `- item` bullet list, or a `---` rule), not a flag — `gh pr comment
    // --body <body>` puts it in a flag-VALUE slot. The MCP layer must NOT reject it
    // (the old blanket `guard_argv_field` did). The runner rule matches only
    // `["gh", "pr", "comment"]`, so reaching the reply proves the body was passed
    // through to the wrapper rather than refused up front.
    #[tokio::test]
    async fn forge_pr_comment_github_allows_leading_dash_body() {
        for body in ["- item one\n- item two", "---"] {
            let gh = vcs_forge::vcs_github::GitHub::with_runner(
                ScriptedRunner::new().on(["gh", "pr", "comment"], Reply::ok("https://gh/pr/7#c1")),
            );
            let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
                "/repo",
                "/repo",
                Git::with_runner(ScriptedRunner::new()),
            ));
            let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
            let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

            let out = server
                .forge_pr_comment(Parameters(PrCommentParams {
                    number: 7,
                    body: body.into(),
                }))
                .await
                .unwrap_or_else(|e| panic!("leading-`-` body {body:?} must pass on GitHub: {e:?}"));
            assert!(
                result_json(&out).contains("https://gh/pr/7#c1"),
                "{}",
                result_json(&out)
            );
        }
    }

    // T-013: the same on GitLab — `glab mr note <id> -m <body>` is a flag-VALUE
    // slot, so a leading `-` is safe and must pass.
    #[tokio::test]
    async fn forge_pr_comment_gitlab_allows_leading_dash_body() {
        let gl = vcs_forge::vcs_gitlab::GitLab::with_runner(
            ScriptedRunner::new().on(["glab", "mr", "note"], Reply::ok("https://gl/mr/7#note1")),
        );
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_gitlab("/repo", gl));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

        let out = server
            .forge_pr_comment(Parameters(PrCommentParams {
                number: 7,
                body: "- a bullet".into(),
            }))
            .await
            .expect("leading-`-` body must pass on GitLab");
        assert!(
            result_json(&out).contains("https://gl/mr/7#note1"),
            "{}",
            result_json(&out)
        );
    }

    // T-013 regression: Gitea's `tea comment <n> <body>` takes the body as a bare
    // POSITIONAL, so a flag-like body IS dangerous there and stays rejected — by
    // the Gitea wrapper's own `reject_flag_like`, reached through the MCP tool. The
    // runner has a `["tea", "comment"]` rule, so a leak-through would SUCCEED
    // (returning the reply) instead of erroring — this pins that it does not.
    #[tokio::test]
    async fn forge_pr_comment_gitea_rejects_flag_like_body() {
        let tea = vcs_forge::vcs_gitea::Gitea::with_runner(
            ScriptedRunner::new().on(["tea", "comment"], Reply::ok("https://gitea/pr/7#c1")),
        );
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_gitea("/repo", tea));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

        let err = server
            .forge_pr_comment(Parameters(PrCommentParams {
                number: 7,
                body: "-evil".into(),
            }))
            .await
            .expect_err("flag-like body must stay rejected on Gitea's positional slot");
        assert!(err.message.contains("flag"), "{}", err.message);
    }

    // T-013: `forge_pr_edit` also passes leading-`-` `title`/`body` through — both
    // ride in flag-VALUE slots (`gh pr edit --title <t> --body <b>`), so a Markdown
    // bullet title or a `---` body is legitimate and must not be refused.
    #[tokio::test]
    async fn forge_pr_edit_allows_leading_dash_title_and_body() {
        let gh = vcs_forge::vcs_github::GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "pr", "edit"], Reply::ok("")),
        );
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

        let out = server
            .forge_pr_edit(Parameters(PrEditParams {
                number: 7,
                title: Some("- a bullet title".into()),
                body: Some("---".into()),
            }))
            .await
            .expect("leading-`-` title/body must pass on GitHub");
        let text = out
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .expect("text content");
        let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(value["edited"], 7, "{text}");
    }

    // `forge_pr_edit` rejects both-`None` with an invalid-params error BEFORE
    // reaching the wrapper — the facade's `InvalidInput` shape surfaces as
    // `invalid_params` (per the updated `forge_err` mapping).
    #[tokio::test]
    async fn forge_pr_edit_both_none_is_invalid_params() {
        let gh = vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new());
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

        let err = server
            .forge_pr_edit(Parameters(PrEditParams {
                number: 7,
                title: None,
                body: None,
            }))
            .await
            .expect_err("both-None rejected");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("title"), "{}", err.message);
    }

    // `Some("")` is a real value (clears the field). The MCP tool passes it
    // through to the wrapper, and the wrapper's argv carries `--title ""`
    // literally. This test pins the round-trip end to end: the
    // `ScriptedRunner::on(["pr", "edit"], …)` rule matches **only** an argv
    // whose first two elements are exactly `["pr", "edit"]` (a different
    // command, or a different argv shape, would fall through and the call
    // would error). Combined with the response shape check, the round-trip
    // is fully verified.
    #[tokio::test]
    async fn forge_pr_edit_some_empty_string_passes_through() {
        let gh = vcs_forge::vcs_github::GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "pr", "edit"], Reply::ok("")),
        );
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

        let out = server
            .forge_pr_edit(Parameters(PrEditParams {
                number: 7,
                title: Some("".into()),
                body: None,
            }))
            .await
            .expect("empty title accepted");
        // `ok_json` uses `to_string_pretty`; pull the inner text and check
        // the `edited` field is present (number == 7).
        let text = out
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .expect("text content");
        let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(value["edited"], 7, "{text}");
    }

    // `forge_info` is read-only: a no-forge server errors with the same
    // "no forge is configured" message every other forge tool uses (per the
    // Q6 override).
    #[tokio::test]
    async fn forge_info_without_a_forge_errors() {
        let server = git_server(ScriptedRunner::new(), WriteGate::None);
        let err = server.forge_info().await.expect_err("no forge");
        assert!(format!("{err:?}").contains("no forge"), "{err:?}");
    }

    // `forge_info` returns the kind string + capability map for an authed
    // GitHub handle. The auth probe is a single `auth status` call (mocked
    // to exit 0); every static cap is `true` post-fork.
    #[tokio::test]
    async fn forge_info_with_authed_github_reports_all_true() {
        let gh = vcs_forge::vcs_github::GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "auth", "status"], Reply::ok("")),
        );
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

        let out = server.forge_info().await.expect("forge_info ok");
        // Extract the inner text content (the JSON value) — `result_json`
        // re-serialises the whole `CallToolResult` with the `content`
        // envelope, so assertions on the inner JSON need the inner text.
        let text = out
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .expect("text content");
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(value["kind"], "github");
        assert_eq!(value["capabilities"]["authed"], true);
        assert_eq!(value["capabilities"]["pr_create"], true);
        assert_eq!(value["capabilities"]["pr_comment"], true);
        assert_eq!(value["capabilities"]["pr_edit"], true);
        assert_eq!(value["capabilities"]["pr_checks"], true);
        assert_eq!(value["capabilities"]["pr_merge"], true);
        assert_eq!(value["capabilities"]["issue_create"], true);
    }

    // The `forge_info` tool is read-only — its annotation is `readOnlyHint`,
    // not `destructiveHint`. Pinned here alongside the existing
    // `tool_annotations_mark_read_vs_destructive` test.
    #[test]
    fn tool_annotations_mark_forge_info_as_read_only() {
        let tool = VcsMcpServer::forge_info_tool_attr();
        let a = tool.annotations.expect("annotations present");
        assert_eq!(a.read_only_hint, Some(true));
        assert_eq!(a.destructive_hint, None);

        let tool = VcsMcpServer::forge_pr_comment_tool_attr();
        let a = tool.annotations.expect("annotations present");
        assert_eq!(a.destructive_hint, Some(true));
        assert_eq!(a.read_only_hint, None);

        let tool = VcsMcpServer::forge_pr_edit_tool_attr();
        let a = tool.annotations.expect("annotations present");
        assert_eq!(a.destructive_hint, Some(true));
        assert_eq!(a.read_only_hint, None);
    }

    // The macro-generated tool definitions carry the right MCP annotations: read
    // tools are read-only, mutation tools are destructive.
    #[test]
    fn tool_annotations_mark_read_vs_destructive() {
        let read = VcsMcpServer::repo_snapshot_tool_attr();
        assert_eq!(read.annotations.unwrap().read_only_hint, Some(true));
        let write = VcsMcpServer::repo_commit_tool_attr();
        assert_eq!(write.annotations.unwrap().destructive_hint, Some(true));
    }

    // The server identifies itself as `vcs-mcp` on the wire, not rmcp's default
    // build-env identity (which would say "rmcp").
    #[test]
    fn server_info_identifies_as_vcs_mcp() {
        let server = git_server(ScriptedRunner::new(), WriteGate::None);
        let info = server.get_info();
        assert_eq!(info.server_info.name, "vcs-mcp");
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
    }

    /// A no-op MCP client handler for the in-process round-trip.
    #[derive(Clone, Default)]
    struct TestClient;
    impl rmcp::ClientHandler for TestClient {
        fn get_info(&self) -> rmcp::model::ClientInfo {
            rmcp::model::ClientInfo::default()
        }
    }

    // End-to-end through rmcp: an in-process client lists the tools and calls a
    // read tool over an in-memory transport — proving the #[tool_router]/
    // #[tool_handler] wiring routes calls, not just that the methods compile.
    #[tokio::test]
    async fn in_process_client_lists_and_calls_tools() {
        use rmcp::ServiceExt;
        use rmcp::model::CallToolRequestParams;

        let server = git_server(
            ScriptedRunner::new().on(["git", "symbolic-ref"], Reply::ok("main\n")),
            WriteGate::None,
        );
        let (server_t, client_t) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            if let Ok(running) = server.serve(server_t).await {
                let _ = running.waiting().await;
            }
        });

        let client = TestClient.serve(client_t).await.expect("client connects");

        let tools = client.list_all_tools().await.expect("list_tools");
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(names.contains(&"repo_snapshot"), "{names:?}");
        assert!(names.contains(&"repo_commit"), "{names:?}");
        assert!(names.contains(&"forge_pr_list"), "{names:?}");
        assert!(names.contains(&"forge_pr_comment"), "{names:?}");
        assert!(names.contains(&"forge_pr_edit"), "{names:?}");
        assert!(names.contains(&"forge_info"), "{names:?}");

        let result = client
            .call_tool(CallToolRequestParams::new("repo_current_branch"))
            .await
            .expect("call repo_current_branch");
        let text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.as_str())
            .expect("text content");
        assert!(text.contains("main"), "{text}");

        let _ = client.cancel().await;
        server_handle.abort();
    }
}

// Long-form how-to guides, rendered from this crate's docs/*.md on docs.rs.
#[doc = include_str!("../docs/mcp.md")]
#[allow(rustdoc::broken_intra_doc_links)]
pub mod guide {}
