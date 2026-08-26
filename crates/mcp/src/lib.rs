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
//!   (`Arc`). [`with_output_budget`](VcsMcpServer::with_output_budget) bounds the
//!   two conflict tools' direct working-copy read, the one content path no client
//!   [`OutputBudget`] can reach (it spawns no command).
//! - **[`WriteGate`]** — the server's write policy: [`None`](WriteGate::None)
//!   (read-only, the default), [`All`](WriteGate::All) (`--allow-write`), or
//!   [`Set`](WriteGate::Set) (a per-tool allowlist). [`allows`](WriteGate::allows)
//!   answers whether a named mutating tool may run.
//! - **Tools** are the `#[tool]` methods on [`VcsMcpServer`]: the `repo_*` group
//!   ([`repo_snapshot`](VcsMcpServer::repo_snapshot),
//!   [`repo_commit`](VcsMcpServer::repo_commit), …) over the `Repo` facade, and
//!   the `forge_*` group ([`forge_pr_list`](VcsMcpServer::forge_pr_list),
//!   [`forge_pr_for_branch`](VcsMcpServer::forge_pr_for_branch),
//!   [`forge_pr_create`](VcsMcpServer::forge_pr_create), …) over the `Forge` one.
//! - **Parameter structs** — one `Deserialize` + `JsonSchema` struct per
//!   tool-with-arguments ([`CommitParams`], [`PrListParams`], [`IssueListParams`],
//!   [`PrCreateParams`], [`LabelsParams`], [`MergeStrategyArg`], …); their schema
//!   is the tool's advertised input schema.
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
//! # Non-UTF-8 paths (fail-closed policy)
//!
//! Path-bearing results — [`repo_status`](VcsMcpServer::repo_status)'s
//! `FileChange.path`, [`repo_conflicts`](VcsMcpServer::repo_conflicts)'s list,
//! [`repo_info`](VcsMcpServer::repo_info)'s `root`/`cwd` — carry
//! the facade's [`PathBuf`](std::path::PathBuf) or [`Path`](std::path::Path), which the toolkit reads
//! **losslessly** from the backend (a filename need not be valid UTF-8 on Unix).
//! JSON strings, however, are UTF-8. A path that is not valid UTF-8 is therefore
//! **refused with an explicit serialization error** rather than emitted with
//! `U+FFFD` replacement characters: an agent must never be handed a
//! silently-corrupted path it would feed back into a mutating tool
//! ([`repo_commit`](VcsMcpServer::repo_commit)) and so address the wrong file. The
//! ordinary UTF-8 case is unchanged — a plain JSON string. (Tool *inputs* are JSON
//! strings too, so a non-UTF-8 path cannot be named over MCP in the first place;
//! the lossless round-trip is a property of the Rust [`vcs_core::Repo`] API.)
//!
//! # In-depth guide
//!
//! Beyond this page, this crate ships a full how-to guide — rendered on docs.rs
//! from `docs/` — covering the CLI flags, the full tool catalogue, forge
//! auto-detection, and the binary's hardening/timeout safety model. See the
//! [`guide`] module.

use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData, ServerHandler, tool_handler};
use vcs_core::processkit::ProcessRunner;
use vcs_core::{BackendKind, OutputBudget, Repo, VcsRepo};
use vcs_forge::{Forge, ForgeApi, ForgeKind};

mod conflicts;
mod forge_tools;
mod outcome_tools;
mod output;
mod params;
mod repo_tools;
mod write_gate;

pub use params::*;
pub use write_gate::*;

/// Discovery guidance sent to every MCP client. The first 512 characters are a
/// complete decision rule so clients that truncate instructions still receive
/// the typed-tool, preflight, write-gate, and raw-fallback contract.
pub const SERVER_INSTRUCTIONS: &str = "Prefer typed outcome_* tools for inspect, changes, checked commit, publish, and exact-revision CI. Start with outcome_inspect as preflight. Mutating tools are advertised only when the write gate enables them; never bypass that gate. Use lower-level typed repo_* or forge_* tools for operations with no outcome tool. Use a raw CLI only as a last-resort fallback when no typed tool is advertised or a typed result explicitly reports unsupported; preserve the same preflight and write policy.";

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
    /// calls sequentially), so most of them aren't gated by this (`forge_pr_create`,
    /// `forge_issue_create`, `forge_pr_mark_ready`, `forge_pr_comment`,
    /// `forge_pr_edit`). The exceptions are `forge_pr_checkout` (fetches and
    /// switches the local checkout), `forge_pr_merge`, and `forge_pr_close` (the
    /// latter two can delete the local branch and switch the checkout via
    /// `delete_branch`) — these *locally* mutate the working copy just like
    /// `repo_*` tools do, so they take this same lock too, closing the local
    /// repo-state race that R1 targets.
    write_lock: Arc<tokio::sync::Mutex<()>>,
    /// The content ceiling for the tools that read a file **directly** from the
    /// working copy — `repo_conflict_regions` and the read inside
    /// `repo_resolve_conflict`.
    ///
    /// Every other content-returning tool (`repo_show_file`, `repo_diff`,
    /// `forge_pr_diff`) reads through a backend subprocess and is bounded by the
    /// [`OutputBudget`] the caller configured on the *client* the `Repo`/`Forge`
    /// was built over. The conflict tools spawn nothing for their read — conflict
    /// markers live only in the working copy — so no client budget can reach them,
    /// and without this field an agent could point them at a multi-gigabyte file
    /// and buffer it whole into the server. Same knob, same unit, same fail-loud
    /// behaviour ("refuse", never "truncate"), applied at the filesystem instead
    /// of at a pipe.
    ///
    /// Defaults to [`OutputBudget::unlimited`] — the type's own default and the
    /// same "a cap is a caller choice" stance the CLI clients take. The binary
    /// sets it from `--max-output-bytes` (10 MiB by default); a library embedder
    /// sets it with [`with_output_budget`](Self::with_output_budget).
    content_budget: OutputBudget,
}

impl VcsMcpServer {
    /// Build a server bound to `repo`, with an optional `forge` (PR/MR tools), and
    /// a [`WriteGate`] controlling which mutating tools are callable.
    ///
    /// Generic over the clients' [`ProcessRunner`] so a caller can inject a
    /// non-default runner — for example a command-logging decorator
    /// ([`vcs_cli_support::logging::LoggingRunner`], as the `--log-commands` binary
    /// flag does) built over a `Box<dyn ProcessRunner>` — without this crate naming
    /// the runner type. The handles are erased to `dyn VcsRepo`/`dyn ForgeApi`
    /// immediately, so the server itself stays runner-agnostic.
    pub fn new<R: ProcessRunner + 'static>(
        repo: Repo<R>,
        forge: Option<Forge<R>>,
        writes: WriteGate,
    ) -> Self {
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
        let backend = repo.kind();
        let forge_kind = forge.as_deref().map(ForgeApi::kind);
        let mut tool_router =
            Self::repo_tool_router() + Self::forge_tool_router() + Self::outcome_tool_router();

        // Capability discovery is executable truth, not a catalogue of calls
        // that will predictably be rejected. Inherent methods remain available
        // for Rust compatibility; only the advertised MCP router is filtered.
        for name in WRITE_TOOLS {
            if !writes.allows(name) {
                tool_router.remove_route(name);
            }
        }
        if backend == BackendKind::Git {
            for name in ["repo_op_log", "repo_undo"] {
                tool_router.remove_route(name);
            }
        }
        if forge_kind.is_none() || forge_kind == Some(ForgeKind::Unknown) {
            let forge_names = tool_router
                .list_all()
                .into_iter()
                .map(|tool| tool.name.into_owned())
                .filter(|name| name.starts_with("forge_"))
                .collect::<Vec<_>>();
            for name in forge_names {
                tool_router.remove_route(&name);
            }
        }
        if forge_kind != Some(ForgeKind::GitHub) {
            for name in outcome_tools::OUTCOME_FORGE_TOOLS {
                tool_router.remove_route(name);
            }
        }
        match forge_kind {
            Some(ForgeKind::Gitea) => {
                for name in [
                    "forge_repo_view",
                    "forge_pr_for_branch",
                    "forge_pr_add_labels",
                    "forge_pr_remove_labels",
                    "forge_pr_edit",
                    "forge_pr_mark_ready",
                    "forge_pr_checks",
                    "forge_pr_diff",
                    "forge_issue_add_labels",
                    "forge_issue_remove_labels",
                    "forge_release_view",
                ] {
                    tool_router.remove_route(name);
                }
            }
            Some(ForgeKind::GitLab) => tool_router.remove_route("forge_pr_request_changes"),
            _ => {}
        }
        Self {
            repo,
            forge,
            writes,
            // The tool surface is split across two `#[tool_router]` impl blocks —
            // `repo_tools` and `forge_tools` — each generating its own named router;
            // rmcp's `ToolRouter: Add` combines them into the single router this
            // server dispatches on (the repo tools register first, then the forge
            // tools, preserving the original registration order).
            tool_router,
            write_lock: Arc::new(tokio::sync::Mutex::new(())),
            content_budget: OutputBudget::unlimited(),
        }
    }

    /// Bound how much a **direct working-copy read** may buffer: the ceiling the
    /// conflict tools (`repo_conflict_regions`, and the read inside
    /// `repo_resolve_conflict`) apply to the file they read.
    ///
    /// Those two are the only tools that touch the filesystem without a backend
    /// subprocess, so the [`OutputBudget`] set on the git/jj client — the one that
    /// bounds `repo_show_file`/`repo_diff` — cannot reach them; set it here as
    /// well to bound them the same way. The binary passes its `--max-output-bytes`
    /// value to both. Over-budget is refused (an error naming the ceiling), never
    /// truncated, and the check is made from the file's size *before* its content
    /// is buffered. [`OutputBudget::unlimited`] (the default) removes the ceiling.
    ///
    /// ```no_run
    /// # use vcs_core::{OutputBudget, Repo};
    /// # use vcs_mcp::{VcsMcpServer, WriteGate};
    /// # fn build() -> Result<(), Box<dyn std::error::Error>> {
    /// let repo = Repo::discover(".")?;
    /// let server = VcsMcpServer::new(repo, None, WriteGate::None)
    ///     .with_output_budget(OutputBudget::bytes(10 * 1024 * 1024));
    /// # Ok(()) }
    /// ```
    #[must_use]
    pub fn with_output_budget(mut self, budget: OutputBudget) -> Self {
        self.content_budget = budget;
        self
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

#[tool_handler(router = self.tool_router)]
impl ServerHandler for VcsMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            // Identify as vcs-mcp on the wire. `ServerInfo::new` defaults the
            // server_info to `Implementation::from_build_env()`, whose `env!` is
            // expanded in *rmcp's* crate — so without this it advertises "rmcp".
            .with_server_info(Implementation::new("vcs-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(SERVER_INSTRUCTIONS)
    }
}

// Long-form how-to guides, rendered from this crate's docs/*.md on docs.rs.
#[doc = include_str!("../docs/mcp.md")]
#[allow(rustdoc::broken_intra_doc_links)]
pub mod guide {}

#[cfg(test)]
mod concurrency_tests;
#[cfg(test)]
mod tests;
