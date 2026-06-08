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
//! Read tools are **always available**; mutating tools are **gated**. Every
//! mutation is registered and annotated `destructiveHint`, but rejects the call
//! — naming itself, before spawning anything — unless the [`WriteGate`] covers
//! it. `--allow-write` enables every mutation; `--allow-tools
//! repo_commit,forge_pr_create` enables only the named ones; read tools are
//! unaffected either way. Tool names are the method names (e.g. `"repo_commit"`).
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
//! let repo = Repo::open(".")?;
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
    /// Force removal even with local changes (git only).
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
    /// Tool names are the method names (e.g. `"repo_commit"`); read tools are
    /// unaffected (always available) and unknown names simply never match.
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
        }
    }

    /// Reject the mutating tool `tool` when the write gate doesn't cover it.
    fn require_write(&self, tool: &str) -> Result<(), ErrorData> {
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

/// Map a `vcs-forge` error into an MCP error — an `Unsupported` op is a
/// client-facing invalid-request; a forge/network failure is internal.
fn forge_err(e: vcs_forge::Error) -> ErrorData {
    if e.is_unsupported() {
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

    #[tool(
        description = "Probe whether merging `source` into the current work would conflict, WITHOUT leaving a trace (the probe is always rolled back). Read-only, but it spawns a real trial merge.",
        annotations(read_only_hint = true)
    )]
    pub async fn repo_try_merge(
        &self,
        Parameters(p): Parameters<TryMergeParams>,
    ) -> Result<CallToolResult, ErrorData> {
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
        self.require_write("repo_commit")?;
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
        self.require_write("repo_checkout")?;
        self.repo.checkout(&p.reference).await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "checked_out": p.reference }))
    }

    #[tool(
        description = "Fetch from the default remote (git fetch / jj git fetch). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_fetch(&self) -> Result<CallToolResult, ErrorData> {
        self.require_write("repo_fetch")?;
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
        self.require_write("repo_push")?;
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
        self.require_write("repo_create_worktree")?;
        let outcome = self
            .repo
            .create_worktree(Path::new(&p.path), &p.branch, &p.base)
            .await
            .map_err(core_err)?;
        ok_json(&outcome)
    }

    #[tool(
        description = "Remove the worktree/workspace at `path`. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_remove_worktree(
        &self,
        Parameters(p): Parameters<RemoveWorktreeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("repo_remove_worktree")?;
        self.repo
            .remove_worktree(Path::new(&p.path), p.force)
            .await
            .map_err(core_err)?;
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
        description = "Open pull/merge requests on the configured forge.",
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
        description = "Open issues on the configured forge (up to 100).",
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
        description = "Releases on the configured forge, newest first (up to 100).",
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
        let out = self
            .forge()?
            .issue_create(&p.title, &p.body)
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
        self.forge()?
            .pr_close(p.number, p.delete_branch)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "closed": p.number }))
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
    use processkit::{Reply, ScriptedRunner};
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
            ScriptedRunner::new().on(["rev-parse"], Reply::ok("main\n")),
            WriteGate::None,
        );
        let out = server.repo_current_branch().await.expect("tool ok");
        assert!(result_json(&out).contains("main"), "{}", result_json(&out));
    }

    // Read tools work even when writes are disabled (the default).
    #[tokio::test]
    async fn read_tool_works_in_readonly_mode() {
        let server = git_server(
            ScriptedRunner::new().on(["status"], Reply::ok(" M a.rs\0")),
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

    // With writes enabled, the same tool reaches the runner and returns success.
    #[tokio::test]
    async fn mutation_reaches_runner_with_allow_write() {
        let server = git_server(
            ScriptedRunner::new().on(["checkout"], Reply::ok("")),
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
            ScriptedRunner::new().on(["push"], Reply::ok("")),
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
                .on(["checkout"], Reply::ok(""))
                .on(["rev-parse"], Reply::ok("main\n")),
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
            ScriptedRunner::new().on(["issue", "list"], Reply::ok(json)),
        );
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::for_github("/repo", gh));
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
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::for_gitea("/repo", tea));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

        let err = server
            .forge_release_view(Parameters(ReleaseTagParams { tag: "v1".into() }))
            .await
            .expect_err("unsupported on gitea");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("release_view"), "{}", err.message);
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
            ScriptedRunner::new().on(["rev-parse"], Reply::ok("main\n")),
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
