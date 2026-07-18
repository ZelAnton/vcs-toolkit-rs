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
//! # Non-UTF-8 paths (fail-closed policy)
//!
//! Path-bearing results — [`repo_status`](VcsMcpServer::repo_status)'s
//! `FileChange.path`, [`repo_conflicts`](VcsMcpServer::repo_conflicts)'s list,
//! [`repo_info`](VcsMcpServer::repo_info)'s `root`/`cwd` — carry
//! the facade's [`PathBuf`](std::path::PathBuf) or [`Path`], which the toolkit reads
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

use std::path::Path;
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::schemars;
use rmcp::{ErrorData, ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;
use vcs_core::{BranchDelete, Repo, VcsRepo};
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

/// Rebase the current line onto a revision.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RebaseParams {
    /// The branch, bookmark, or revision to rebase onto.
    pub onto: String,
}

/// Start new work on top of a revision without modifying it.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NewChildParams {
    /// The branch, bookmark, or revision to start the child work from.
    pub reference: String,
}

/// Create a local branch/bookmark at the current head.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateBranchParams {
    /// The local branch (git) / bookmark (jj) name to create.
    pub name: String,
}

/// Delete a local branch/bookmark.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteBranchParams {
    /// The local branch (git) / bookmark (jj) to delete.
    pub name: String,
    /// Delete an unmerged git branch (`git branch -D`). jj ignores this flag.
    #[serde(default)]
    pub force: bool,
}

/// Rename a local branch/bookmark.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RenameBranchParams {
    /// The existing local branch (git) / bookmark (jj) name.
    pub old: String,
    /// The replacement local branch (git) / bookmark (jj) name.
    pub new: String,
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

/// List recent history.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LogParams {
    /// The revspec (git) / revset (jj) to list history from, e.g. `"HEAD"` (git) or
    /// `"@"` (jj).
    pub revspec_or_revset: String,
    /// Maximum number of commits to return.
    pub max: usize,
}

/// Read a file's content at a revision.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ShowFileParams {
    /// The revspec (git) / revset (jj) to read the file from, e.g. `"HEAD"` (git)
    /// or `"@-"` (jj).
    pub rev: String,
    /// Repo-relative path of the file to read.
    pub path: String,
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
    /// Enable auto-merge — merge once requirements/checks are met. **GitHub only**;
    /// GitLab/Gitea reject it as unsupported (`invalid_params`) rather than merging
    /// immediately anyway. Defaults to `false`.
    #[serde(default)]
    pub auto: bool,
    /// Delete the source branch after merging. **GitHub only**; GitLab/Gitea reject
    /// it as unsupported (`invalid_params`) rather than silently leaving the branch.
    /// Defaults to `false`.
    #[serde(default)]
    pub delete_branch: bool,
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

/// Submit a "request changes" review on a pull/merge request.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrRequestChangesParams {
    /// The PR/MR number.
    pub number: u64,
    /// The review body / reason. Required — a request-changes review needs a
    /// reason; an empty (or whitespace-only) body is rejected up front.
    pub body: String,
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
    /// calls sequentially), so most of them aren't gated by this (`forge_pr_create`,
    /// `forge_issue_create`, `forge_pr_close`, `forge_pr_mark_ready`,
    /// `forge_pr_comment`, `forge_pr_edit`). The exceptions are `forge_pr_checkout`
    /// (fetches and switches the local checkout) and `forge_pr_merge` (which can
    /// delete the local branch and switch the checkout via `delete_branch`) —
    /// these *locally* mutate the working copy just like `repo_*` tools do, so
    /// they take this same lock too, closing the local repo-state race, the one
    /// R1 targets.
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
///
/// **Non-UTF-8 path policy (fail-closed).** Path-bearing DTOs carry a
/// [`PathBuf`](std::path::PathBuf), which serialises to a JSON string only when it
/// is valid UTF-8. A path whose bytes are not valid UTF-8 (possible on Unix) makes
/// serialisation fail, and this returns an **explicit error** rather than emitting
/// the path with `U+FFFD` substitution — so an agent never receives a
/// silently-corrupted path it would feed back into a mutating tool. The ordinary
/// UTF-8 case is unaffected (a plain JSON string). See the crate-level
/// *Non-UTF-8 paths* section.
fn ok_json<T: serde::Serialize>(value: &T) -> Result<CallToolResult, ErrorData> {
    let json = serde_json::to_string_pretty(value).map_err(|e| {
        ErrorData::internal_error(
            format!(
                "failed to serialise the result to JSON: {e} (a filesystem path that is \
                 not valid UTF-8 cannot be represented as a JSON string; it is refused \
                 rather than emitted with U+FFFD substitution)"
            ),
            None,
        )
    })?;
    Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
}

/// [`repo_info`](VcsMcpServer::repo_info)'s JSON shape. `root`/`cwd` are
/// borrowed [`Path`]s — not `to_string_lossy` strings — so that a non-UTF-8
/// root/cwd (legal on Unix) fails serialization in [`ok_json`] the same way
/// every other path-bearing DTO in this crate does, instead of silently
/// substituting `U+FFFD`. See the crate-level *Non-UTF-8 paths* section.
///
/// Deliberately **not** built with `serde_json::json!{}`: that macro resolves
/// a non-literal field to `serde_json::to_value(&expr).unwrap()`, which would
/// **panic** rather than surface a graceful error on a serialization failure
/// (i.e. exactly the non-UTF-8 case this type exists to handle). Passing a
/// concrete `Serialize` struct straight to [`ok_json`] instead runs
/// `serde_json::to_string_pretty`, whose `Err` is already handled there.
#[derive(serde::Serialize)]
struct RepoInfo<'a> {
    backend: &'static str,
    root: &'a Path,
    cwd: &'a Path,
    forge: Option<&'static str>,
}

/// Map a `vcs-core` error into an MCP error. The facade reports a refused
/// *input* (e.g. `commit_paths` with an empty path set) as an
/// `InvalidInput` io error — that's the client's call to fix, so surface it as
/// an invalid-params error rather than an internal one.
fn core_err(e: vcs_core::Error) -> ErrorData {
    // A bad-argument failure — a facade precondition (`Error::Io`/`InvalidInput`)
    // OR the boundary refusal of a flag-like/malformed ref/revision (which the
    // facade now raises as `Error::Vcs` carrying an `InvalidInput` spawn source
    // when it converts a tool string into a validated newtype) — is a client-facing
    // invalid-request, not an internal error. `is_invalid_input` classifies both.
    if e.is_invalid_input() {
        ErrorData::invalid_params(e.to_string(), None)
    } else {
        ErrorData::internal_error(e.to_string(), None)
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

    // NOTE (T-068): NOT `read_only_hint = true`. On jj this tool runs a plain,
    // working-copy-**snapshotting** jj command that records an op-log operation, so
    // asserting `readOnlyHint` ("does not modify its environment") would violate the
    // MCP contract. It is classified as the honest, backend-agnostic truth instead —
    // non-destructive + idempotent (the op-log snapshot is append-only/recoverable
    // and changes no tracked content, refs, or bookmarks) — the same classification
    // `repo_try_merge` uses. See the `jj_snapshotting_read_tools_*` tests and the
    // Safety model note in `docs/mcp.md`. It stays callable without a write gate (an
    // op-log snapshot is not a content/ref mutation).
    #[tool(
        description = "A batched snapshot of the repo state: branch, upstream, ahead/behind, HEAD, dirtiness, change count, conflict, and operation state. Read query; on jj it snapshots the working copy (records a reversible op-log operation), so it is annotated non-destructive rather than readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_snapshot(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.snapshot().await.map_err(core_err)?)
    }

    // T-068: `read_only_hint = true` is correct here (unlike the other repo_* reads).
    // `repo_info` spawns NO backend command at all — it reads the backend kind and the
    // root/cwd paths the facade cached at construction, and the forge kind — so it can
    // never snapshot a jj working copy or record an op-log operation. The read-only
    // guarantee holds on both backends. Pinned by `truly_read_only_tools_keep_read_only_hint`.
    #[tool(
        description = "Which backend (git/jj), the repository root, the working directory, and the configured forge (if any).",
        annotations(read_only_hint = true)
    )]
    pub async fn repo_info(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&RepoInfo {
            backend: self.repo.kind().as_str(),
            root: self.repo.root(),
            cwd: self.repo.cwd(),
            forge: self.forge.as_ref().map(|f| f.kind().as_str()),
        })
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate).
    #[tool(
        description = "The working-copy changes (added/modified/deleted/renamed paths). Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_status(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.changed_files().await.map_err(core_err)?)
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate).
    #[tool(
        description = "Aggregate insertion/deletion/file counts for the working copy. Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_diff_stat(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.diff_stat().await.map_err(core_err)?)
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate).
    #[tool(
        description = "The full parsed working-copy diff (per-file hunks/lines) — same scope as repo_diff_stat: git working tree vs HEAD (excludes untracked files), jj @ vs its parent (includes newly-added files). Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_diff(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.diff().await.map_err(core_err)?)
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate). A plain `jj log`
    // snapshots the working copy first, exactly like the other repo_* reads.
    #[tool(
        description = "Recent history: up to `max` commits reachable from `revspec_or_revset` (a git revspec, e.g. \"HEAD\", or a jj revset, e.g. \"@\"), most-recent-first. `author`/`date` are null on jj — its typed log doesn't currently surface authorship or a timestamp. Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_log(
        &self,
        Parameters(p): Parameters<LogParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(
            &self
                .repo
                .log(&p.revspec_or_revset, p.max)
                .await
                .map_err(core_err)?,
        )
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate). A plain `jj file show`
    // snapshots the working copy first, exactly like the other repo_* reads.
    #[tool(
        description = "The content of a file at a revision (a git revspec, e.g. \"HEAD\", or a jj revset, e.g. \"@-\"). Returns the file's bytes verbatim (including any trailing newline). Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_show_file(
        &self,
        Parameters(p): Parameters<ShowFileParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(
            &self
                .repo
                .show_file(&p.rev, &p.path)
                .await
                .map_err(core_err)?,
        )
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate).
    #[tool(
        description = "Local branch (git) / bookmark (jj) names. Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_branches(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.local_branches().await.map_err(core_err)?)
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate).
    #[tool(
        description = "The current branch/bookmark (null when detached/unset). Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_current_branch(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.current_branch().await.map_err(core_err)?)
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate). `jj resolve --list`
    // snapshots the working copy first, exactly like the other repo_* reads.
    #[tool(
        description = "Paths with unresolved merge conflicts (repo-relative, '/'-separated). Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    pub async fn repo_conflicts(&self) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.repo.conflicted_files().await.map_err(core_err)?)
    }

    // T-068: jj-snapshotting read tool — see `repo_snapshot`'s note (non-destructive,
    // NOT readOnlyHint; still callable without a write gate). `jj workspace list`
    // snapshots the working copy first (the per-workspace `workspace root` probes it
    // fans out are already `--ignore-working-copy`, but the top-level list is not).
    #[tool(
        description = "Attached worktrees (git) / workspaces (jj). Read query; on jj it snapshots the working copy (reversible op-log op) — annotated non-destructive, not readOnlyHint.",
        annotations(destructive_hint = false, idempotent_hint = true)
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
        // The facade takes `PathBuf`s (lossless for a non-UTF-8 path on Unix); a tool
        // input arrives as JSON `String`s (always UTF-8), so the conversion is exact.
        let paths: Vec<std::path::PathBuf> = p.paths.iter().map(std::path::PathBuf::from).collect();
        self.repo
            .commit_paths(&paths, &p.message)
            .await
            .map_err(core_err)?;
        ok_json(&serde_json::json!({ "committed_paths": paths.len() }))
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
        description = "Rebase the current line onto a branch, bookmark, or revision. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_rebase(
        &self,
        Parameters(p): Parameters<RebaseParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_rebase").await?;
        ok_json(&self.repo.rebase(&p.onto).await.map_err(core_err)?)
    }

    #[tool(
        description = "Abort the in-progress repository operation, if any. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_abort_in_progress(&self) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_abort_in_progress").await?;
        let operation_state = self.repo.abort_in_progress().await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "operation_state": operation_state }))
    }

    #[tool(
        description = "Continue the in-progress repository operation after resolving conflicts. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_continue_in_progress(&self) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_continue_in_progress").await?;
        let operation_state = self.repo.continue_in_progress().await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "operation_state": operation_state }))
    }

    #[tool(
        description = "Start new work on top of a branch, bookmark, or revision. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_new_child(
        &self,
        Parameters(p): Parameters<NewChildParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_new_child").await?;
        ok_json(&self.repo.new_child(&p.reference).await.map_err(core_err)?)
    }

    #[tool(
        description = "Create a local branch or bookmark at the current head, without switching the working copy (git branch <name> / jj bookmark create <name> -r @). Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_create_branch(
        &self,
        Parameters(p): Parameters<CreateBranchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_create_branch").await?;
        self.repo.create_branch(&p.name).await.map_err(core_err)?;
        ok_json(&serde_json::json!({ "created_branch": p.name }))
    }

    #[tool(
        description = "Delete a local branch or bookmark. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_delete_branch(
        &self,
        Parameters(p): Parameters<DeleteBranchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_delete_branch").await?;
        let spec = if p.force {
            BranchDelete::new(p.name).force()
        } else {
            BranchDelete::new(p.name)
        };
        ok_json(&self.repo.delete_branch(spec).await.map_err(core_err)?)
    }

    #[tool(
        description = "Rename a local branch or bookmark. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn repo_rename_branch(
        &self,
        Parameters(p): Parameters<RenameBranchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _write = self.begin_repo_write("repo_rename_branch").await?;
        ok_json(
            &self
                .repo
                .rename_branch(&p.old, &p.new)
                .await
                .map_err(core_err)?,
        )
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
        description = "The PR/MR's diff, one file entry per changed file (Unsupported on Gitea).",
        annotations(read_only_hint = true)
    )]
    pub async fn forge_pr_diff(
        &self,
        Parameters(p): Parameters<PrNumberParams>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_json(&self.forge()?.pr_diff(p.number).await.map_err(forge_err)?)
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
        description = "Merge a pull/merge request with a strategy (merge|squash|rebase). Optional `auto` (merge once requirements are met) and `delete_branch` are GitHub-only — GitLab/Gitea reject them as unsupported rather than merging without them. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_merge(
        &self,
        Parameters(p): Parameters<PrMergeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        // With `delete_branch`, `gh pr merge --delete-branch` deletes the local
        // branch and switches the checkout to the default branch — a local
        // working-copy mutation that races `repo_*` mutations the same way they
        // race each other. Take the lock unconditionally (rather than only when
        // `delete_branch` is set) to keep this simple and avoid any race in a
        // conditional-lock branch (see the `write_lock` field comment).
        let _write = self.begin_repo_write("forge_pr_merge").await?;
        let mut merge = vcs_forge::PrMerge::new(p.strategy.into());
        if p.auto {
            merge = merge.auto();
        }
        if p.delete_branch {
            merge = merge.delete_branch();
        }
        self.forge()?
            .pr_merge(p.number, merge)
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
        description = "Submit an approving review on a pull/merge request (gh pr review --approve / glab mr approve / tea pr approve). Supported on all three forges. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_approve(
        &self,
        Parameters(p): Parameters<PrNumberParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_approve")?;
        // A remote review action (no local working-copy mutation), so `require_write`
        // rather than the repo write lock — uniform with `forge_pr_comment`.
        self.forge()?
            .pr_approve(p.number)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "approved": p.number }))
    }

    #[tool(
        description = "Submit a \"request changes\" review with a required body/reason (gh pr review --request-changes --body / tea pr reject). `Unsupported` on GitLab, whose review model is approve/revoke with no request-changes action. An empty body is rejected up front as invalid params. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_request_changes(
        &self,
        Parameters(p): Parameters<PrRequestChangesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        self.require_write("forge_pr_request_changes")?;
        // No MCP-layer argv guard on `body` (see `forge_pr_comment`): GitHub puts it
        // in a flag-VALUE slot (`--body`), and the Gitea wrapper guards its bare
        // positional itself. The facade also rejects an empty body — and reports
        // GitLab `Unsupported` — before any spawn, surfaced here as invalid params.
        self.forge()?
            .pr_request_changes(p.number, &p.body)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "requested_changes": p.number }))
    }

    #[tool(
        description = "Check out a pull/merge request's branch into the local working copy (gh pr checkout / glab mr checkout / tea pr checkout). Mutates the working copy — the head/source branch is fetched and switched to. Requires write access (--allow-write, or --allow-tools naming this tool).",
        annotations(destructive_hint = true)
    )]
    pub async fn forge_pr_checkout(
        &self,
        Parameters(p): Parameters<PrNumberParams>,
    ) -> Result<CallToolResult, ErrorData> {
        // Unlike most forge tools, this one locally mutates the working copy (the
        // head/source branch is fetched and switched to), so it races `repo_*`
        // mutations the same way they race each other — gate it through the same
        // per-repo write lock (see the `write_lock` field comment).
        let _write = self.begin_repo_write("forge_pr_checkout").await?;
        self.forge()?
            .pr_checkout(p.number)
            .await
            .map_err(forge_err)?;
        ok_json(&serde_json::json!({ "checked_out": p.number }))
    }

    #[tool(
        description = "The forge's identity and flat capability map (read-only). Returns `{ kind, capabilities: { pr_create, pr_comment, pr_edit, pr_checks, pr_merge, pr_approve, pr_request_changes, issue_create, version, supported, authed } }` for the configured forge. `version` is the installed CLI's `{major,minor,patch}` (or null if unknown/unrecognisable) and `supported` whether it meets the CLI's declared version floor; the per-op flags are the intersection of \"the CLI ships the command\", `supported`, and `authed`. `pr_request_changes` is always false for GitLab (its review model is approve/revoke). Note: for GitLab, `authed` is best-effort (`glab auth status` can report authed when it is not); a real API call is the sure test.",
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

    // `repo_log` is a read tool (no write gate) that surfaces the facade's
    // unified `Commit` DTO as JSON, author/date included on git.
    #[tokio::test]
    async fn repo_log_returns_commit_json() {
        let server = git_server(
            ScriptedRunner::new().on(
                ["git", "log"],
                Reply::ok(
                    "deadbeef\u{1f}dead\u{1f}Jane\u{1f}2026-05-31T10:00:00+00:00\u{1f}Fix bug\0",
                ),
            ),
            WriteGate::None,
        );
        let out = server
            .repo_log(Parameters(LogParams {
                revspec_or_revset: "HEAD".into(),
                max: 10,
            }))
            .await
            .expect("repo_log ok");
        let json = result_json(&out);
        assert!(json.contains("deadbeef"), "{json}");
        assert!(json.contains("Fix bug"), "{json}");
        assert!(json.contains("Jane"), "{json}");
    }

    // `repo_show_file` is a read tool (no write gate) that surfaces the facade's
    // file content verbatim.
    #[tokio::test]
    async fn repo_show_file_returns_content() {
        let server = git_server(
            ScriptedRunner::new().on(["git", "show"], Reply::ok("fn main() {}\n")),
            WriteGate::None,
        );
        let out = server
            .repo_show_file(Parameters(ShowFileParams {
                rev: "HEAD".into(),
                path: "src/main.rs".into(),
            }))
            .await
            .expect("repo_show_file ok");
        let json = result_json(&out);
        assert!(json.contains("fn main"), "{json}");
    }

    // T-049: the MCP server INHERITS the output budget of the client its `Repo` was
    // built over — a `repo_show_file` whose content exceeds the budget surfaces as a
    // tool error (the wrapped `OutputTooLarge`), never a silently truncated file. A
    // budget below the ceiling returns the content in full.
    #[tokio::test]
    async fn repo_show_file_honours_inherited_output_budget() {
        let big = "x".repeat(200_000);
        // Over budget → the tool errors instead of returning a clipped file.
        let budgeted = Git::with_runner(ScriptedRunner::new().on(["git", "show"], Reply::ok(&big)))
            .default_output_budget(vcs_core::OutputBudget::bytes(64 * 1024));
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git("/repo", "/repo", budgeted));
        let server = VcsMcpServer::from_handles(repo, None, WriteGate::None);
        let err = server
            .repo_show_file(Parameters(ShowFileParams {
                rev: "HEAD".into(),
                path: "big.bin".into(),
            }))
            .await
            .expect_err("over-budget show_file must error, not truncate");
        assert!(
            format!("{err:?}").to_lowercase().contains("ceiling")
                || format!("{err:?}").to_lowercase().contains("too large")
                || format!("{err:?}").to_lowercase().contains("exceeded"),
            "error should name the output ceiling: {err:?}"
        );

        // Under the same budget a small file still reads in full.
        let small = Git::with_runner(
            ScriptedRunner::new().on(["git", "show"], Reply::ok("fn main() {}\n")),
        )
        .default_output_budget(vcs_core::OutputBudget::bytes(64 * 1024));
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git("/repo", "/repo", small));
        let server = VcsMcpServer::from_handles(repo, None, WriteGate::None);
        let out = server
            .repo_show_file(Parameters(ShowFileParams {
                rev: "HEAD".into(),
                path: "src/main.rs".into(),
            }))
            .await
            .expect("under-budget show_file ok");
        assert!(result_json(&out).contains("fn main"));
    }

    // `repo_diff` is a read tool (no write gate) that surfaces the facade's full
    // parsed working-copy diff as JSON.
    #[tokio::test]
    async fn repo_diff_returns_parsed_diff() {
        let out_text = "diff --git a/m b/m\n--- a/m\n+++ b/m\n@@ -1 +1 @@\n-a\n+b\n";
        let server = git_server(
            ScriptedRunner::new()
                .on(["git", "rev-parse"], Reply::ok("deadbeef\n")) // HEAD resolves
                .on(["git", "diff"], Reply::ok(out_text)),
            WriteGate::None,
        );
        let out = server.repo_diff().await.expect("repo_diff ok");
        let json = result_json(&out);
        assert!(json.contains("\\\"m\\\""), "{json}");
        assert!(json.contains("Modified"), "{json}");
    }

    // T-049/T-068: `repo_diff` INHERITS the output budget of the client its `Repo`
    // was built over, exactly like `repo_show_file` — an over-budget diff surfaces
    // as a tool error (the wrapped `OutputTooLarge`), never a silently truncated
    // diff. A budget below the ceiling returns the diff in full.
    #[tokio::test]
    async fn repo_diff_honours_inherited_output_budget() {
        let big = "diff --git a/m b/m\n".to_string() + &"+x\n".repeat(100_000);
        // Over budget → the tool errors instead of returning a clipped diff.
        let budgeted = Git::with_runner(
            ScriptedRunner::new()
                .on(["git", "rev-parse"], Reply::ok("deadbeef\n"))
                .on(["git", "diff"], Reply::ok(&big)),
        )
        .default_output_budget(vcs_core::OutputBudget::bytes(64 * 1024));
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git("/repo", "/repo", budgeted));
        let server = VcsMcpServer::from_handles(repo, None, WriteGate::None);
        let err = server
            .repo_diff()
            .await
            .expect_err("over-budget diff must error, not truncate");
        assert!(
            format!("{err:?}").to_lowercase().contains("ceiling")
                || format!("{err:?}").to_lowercase().contains("too large")
                || format!("{err:?}").to_lowercase().contains("exceeded"),
            "error should name the output ceiling: {err:?}"
        );

        // Under the same budget a small diff still reads in full.
        let small_text = "diff --git a/m b/m\n--- a/m\n+++ b/m\n@@ -1 +1 @@\n-a\n+b\n";
        let small = Git::with_runner(
            ScriptedRunner::new()
                .on(["git", "rev-parse"], Reply::ok("deadbeef\n"))
                .on(["git", "diff"], Reply::ok(small_text)),
        )
        .default_output_budget(vcs_core::OutputBudget::bytes(64 * 1024));
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git("/repo", "/repo", small));
        let server = VcsMcpServer::from_handles(repo, None, WriteGate::None);
        let out = server.repo_diff().await.expect("under-budget diff ok");
        assert!(result_json(&out).contains("Modified"));
    }

    // `repo_info` is a plain UTF-8 round trip in the ordinary case: `backend`,
    // `root`, `cwd`, `forge` all surface as JSON strings (the regression below
    // covers the non-UTF-8 fail-closed case).
    #[tokio::test]
    async fn repo_info_returns_utf8_paths() {
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo/sub",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let server = VcsMcpServer::from_handles(repo, None, WriteGate::None);
        let out = server.repo_info().await.expect("repo_info ok");
        let json = result_json(&out);
        assert!(json.contains("backend"), "{json}");
        assert!(json.contains("git"), "{json}");
        assert!(json.contains("/repo"), "{json}");
        assert!(json.contains("/repo/sub"), "{json}");
        assert!(json.contains("forge"), "{json}");
    }

    // T-062: `repo_info`'s `root`/`cwd` used to serialise through
    // `to_string_lossy`, silently emitting `U+FFFD` for a non-UTF-8 root/cwd
    // (legal on Unix). They now go through the same fail-closed path as every
    // other path-bearing DTO in this crate (see `ok_json`'s doc comment): a
    // non-UTF-8 root/cwd must fail the call instead of returning corrupted JSON.
    #[cfg(unix)]
    #[tokio::test]
    async fn repo_info_rejects_non_utf8_root_instead_of_lossy_substituting() {
        let bad = std::path::PathBuf::from(vcs_testkit::non_utf8_filename());
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            bad.clone(),
            bad,
            Git::with_runner(ScriptedRunner::new()),
        ));
        let server = VcsMcpServer::from_handles(repo, None, WriteGate::None);
        let err = server
            .repo_info()
            .await
            .expect_err("a non-UTF-8 root/cwd must be refused, not lossy-substituted");
        assert!(
            format!("{err:?}").to_lowercase().contains("utf-8"),
            "error should name the UTF-8 refusal: {err:?}"
        );
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

    // A flag-like ref/revision tool parameter is rejected the moment the facade
    // converts it into the validated newtype (`RefName`/`RevSpec`) — surfacing as
    // INVALID_PARAMS (a classifiable client mistake) *before* any git process
    // spawns, rather than an opaque internal error. The runner has no `git log`
    // scripted, so had the value NOT been refused pre-spawn the command would have
    // surfaced as an internal error instead — the INVALID_PARAMS code is the proof
    // the rejection happened at the boundary.
    #[tokio::test]
    async fn flag_like_revspec_surfaces_as_invalid_params() {
        let server = git_server(ScriptedRunner::new(), WriteGate::None);
        let err = server
            .repo_log(Parameters(LogParams {
                revspec_or_revset: "--upload-pack=/bin/evil".into(),
                max: 10,
            }))
            .await
            .expect_err("a flag-like revspec must be refused");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
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

    // `forge_pr_diff` is read-only (works with no write access) and returns the
    // parsed per-file diff as JSON.
    #[tokio::test]
    async fn forge_pr_diff_routes_and_returns_parsed_diff() {
        let diff = "diff --git a/notes.txt b/notes.txt\n--- a/notes.txt\n+++ b/notes.txt\n@@ -1 +1 @@\n-a\n+b\n";
        let gh = vcs_forge::vcs_github::GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "pr", "diff"], Reply::ok(diff)),
        );
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

        let out = server
            .forge_pr_diff(Parameters(PrNumberParams { number: 7 }))
            .await
            .expect("pr_diff ok");
        // `result_json` serialises the whole `CallToolResult`, so the tool's own
        // JSON text comes back escaped inside it — match unquoted substrings.
        let json = result_json(&out);
        assert!(json.contains("notes.txt"), "{json}");
        assert!(json.contains("Modified"), "{json}");
    }

    // T-049: `forge_pr_diff` inherits the output budget of the forge client the
    // server was built over — an over-budget PR diff surfaces as a tool error
    // (the wrapped `OutputTooLarge`), never a truncated diff.
    #[tokio::test]
    async fn forge_pr_diff_honours_inherited_output_budget() {
        let big = "diff --git a/m b/m\n".to_string() + &"+line\n".repeat(20_000);
        let gh = vcs_forge::vcs_github::GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "pr", "diff"], Reply::ok(&big)),
        )
        .default_output_budget(vcs_core::OutputBudget::bytes(64 * 1024));
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);
        let err = server
            .forge_pr_diff(Parameters(PrNumberParams { number: 7 }))
            .await
            .expect_err("over-budget pr_diff must error, not truncate");
        assert!(
            format!("{err:?}").to_lowercase().contains("ceiling")
                || format!("{err:?}").to_lowercase().contains("too large")
                || format!("{err:?}").to_lowercase().contains("exceeded"),
            "error should name the output ceiling: {err:?}"
        );
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

    // Same treatment for `forge_pr_diff` (tea has no diff command).
    #[tokio::test]
    async fn forge_pr_diff_unsupported_maps_to_invalid_params() {
        let tea = vcs_forge::vcs_gitea::Gitea::with_runner(ScriptedRunner::new());
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_gitea("/repo", tea));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);

        let err = server
            .forge_pr_diff(Parameters(PrNumberParams { number: 1 }))
            .await
            .expect_err("unsupported on gitea");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(err.message.contains("pr_diff"), "{}", err.message);
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

    // `forge_pr_approve` is write-gated: refused under `WriteGate::None`, routed to
    // `gh pr review --approve` when allowed (the runner rule matches only
    // `["gh","pr","review"]`, so reaching the reply proves the routing).
    #[tokio::test]
    async fn forge_pr_approve_gates_and_routes() {
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github(
            "/repo",
            vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new()),
        ));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);
        let err = server
            .forge_pr_approve(Parameters(PrNumberParams { number: 7 }))
            .await
            .expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

        let gh = vcs_forge::vcs_github::GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "pr", "review"], Reply::ok("")),
        );
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
        let out = server
            .forge_pr_approve(Parameters(PrNumberParams { number: 7 }))
            .await
            .expect("approve ok");
        assert!(
            result_json(&out).contains("approved"),
            "{}",
            result_json(&out)
        );
    }

    // `forge_pr_request_changes` is write-gated and routes to `gh pr review
    // --request-changes`; on GitLab it maps to the facade's `Unsupported`
    // (invalid_params), and an empty body is rejected up front — both without a spawn.
    #[tokio::test]
    async fn forge_pr_request_changes_gates_routes_and_unsupported_on_gitlab() {
        // Gated under WriteGate::None.
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github(
            "/repo",
            vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new()),
        ));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);
        let err = server
            .forge_pr_request_changes(Parameters(PrRequestChangesParams {
                number: 7,
                body: "please fix".into(),
            }))
            .await
            .expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

        // Allowed on GitHub: routes to `gh pr review`.
        let gh = vcs_forge::vcs_github::GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "pr", "review"], Reply::ok("")),
        );
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
        let out = server
            .forge_pr_request_changes(Parameters(PrRequestChangesParams {
                number: 7,
                body: "please fix".into(),
            }))
            .await
            .expect("request-changes ok");
        assert!(
            result_json(&out).contains("requested_changes"),
            "{}",
            result_json(&out)
        );

        // GitLab: Unsupported → invalid_params, without spawning (no runner rule).
        let glab = vcs_forge::vcs_gitlab::GitLab::with_runner(ScriptedRunner::new());
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_gitlab("/repo", glab));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
        let err = server
            .forge_pr_request_changes(Parameters(PrRequestChangesParams {
                number: 7,
                body: "please fix".into(),
            }))
            .await
            .expect_err("unsupported on gitlab");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("pr_request_changes"),
            "{}",
            err.message
        );

        // An empty body is rejected up front (invalid_params), also without a spawn.
        let gh = vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new());
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
        let err = server
            .forge_pr_request_changes(Parameters(PrRequestChangesParams {
                number: 7,
                body: "   ".into(),
            }))
            .await
            .expect_err("empty body rejected");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }

    // `forge_pr_checkout` is write-gated like the other forge mutations: refused
    // under `WriteGate::None`, but routed to `gh pr checkout <n>` when allowed.
    #[tokio::test]
    async fn forge_pr_checkout_gates_and_routes() {
        // Gated: refused before any spawn.
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github(
            "/repo",
            vcs_forge::vcs_github::GitHub::with_runner(ScriptedRunner::new()),
        ));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::None);
        let err = server
            .forge_pr_checkout(Parameters(PrNumberParams { number: 7 }))
            .await
            .expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

        // Allowed: routes to `gh pr checkout` and reports the checked-out number.
        let gh = vcs_forge::vcs_github::GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "pr", "checkout"], Reply::ok("")),
        );
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);
        let out = server
            .forge_pr_checkout(Parameters(PrNumberParams { number: 7 }))
            .await
            .expect("checkout ok");
        assert!(
            result_json(&out).contains("checked_out"),
            "{}",
            result_json(&out)
        );
    }

    // `forge_pr_merge` is write-gated; when allowed it maps the strategy plus the
    // GitHub-only `auto`/`delete_branch` params onto gh's own flags. The runner
    // rule matches only `["gh", "pr", "merge"]`, so reaching the reply proves the
    // whole spec was routed to the wrapper.
    #[tokio::test]
    async fn forge_pr_merge_routes_strategy_and_github_options() {
        let gh = vcs_forge::vcs_github::GitHub::with_runner(
            ScriptedRunner::new().on(["gh", "pr", "merge"], Reply::ok("")),
        );
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

        let out = server
            .forge_pr_merge(Parameters(PrMergeParams {
                number: 7,
                strategy: MergeStrategyArg::Squash,
                auto: true,
                delete_branch: true,
            }))
            .await
            .expect("merge ok");
        assert!(
            result_json(&out).contains("merged"),
            "{}",
            result_json(&out)
        );
    }

    // The GitHub-only `auto`/`delete_branch` merge options are rejected as
    // `invalid_params` on GitLab/Gitea — the facade's `Unsupported` (bubbled from
    // the wrapper) is a client-fixable request, not an internal error — and nothing
    // spawns (the runner has no rule).
    #[tokio::test]
    async fn forge_pr_merge_unsupported_options_map_to_invalid_params() {
        let tea = vcs_forge::vcs_gitea::Gitea::with_runner(ScriptedRunner::new());
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_gitea("/repo", tea));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

        let err = server
            .forge_pr_merge(Parameters(PrMergeParams {
                number: 7,
                strategy: MergeStrategyArg::Merge,
                auto: true,
                delete_branch: false,
            }))
            .await
            .expect_err("auto is unsupported on gitea");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    }

    // T-058: `forge_pr_checkout` and `forge_pr_merge` locally mutate the working
    // copy (checkout/switch), so — unlike the other forge tools — they must go
    // through `begin_repo_write` and actually hold the same per-repo `write_lock`
    // as `repo_*` mutations, not just call the gate-only `require_write`. Prove it
    // by holding the lock ourselves first: the tool call must then block (time out)
    // rather than run past the lock acquisition, and must succeed once the lock is
    // released.
    #[tokio::test]
    async fn forge_pr_checkout_and_forge_pr_merge_hold_the_repo_write_lock() {
        let gh = vcs_forge::vcs_github::GitHub::with_runner(
            ScriptedRunner::new()
                .on(["gh", "pr", "checkout"], Reply::ok(""))
                .on(["gh", "pr", "merge"], Reply::ok("")),
        );
        let repo: Arc<dyn VcsRepo> = Arc::new(Repo::from_git(
            "/repo",
            "/repo",
            Git::with_runner(ScriptedRunner::new()),
        ));
        let forge: Arc<dyn ForgeApi> = Arc::new(Forge::from_github("/repo", gh));
        let server = VcsMcpServer::from_handles(repo, Some(forge), WriteGate::All);

        // Hold the write lock ourselves (simulating a concurrent repo_* mutation
        // in flight), then attempt both forge tools — both must block on the same
        // lock rather than run through immediately.
        let outer_guard = server
            .write_lock
            .clone()
            .try_lock_owned()
            .expect("uncontended at test start");

        let checkout_timed_out = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            server.forge_pr_checkout(Parameters(PrNumberParams { number: 7 })),
        )
        .await
        .is_err();
        assert!(
            checkout_timed_out,
            "forge_pr_checkout must block while the repo write lock is held elsewhere"
        );

        let merge_timed_out = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            server.forge_pr_merge(Parameters(PrMergeParams {
                number: 7,
                strategy: MergeStrategyArg::Merge,
                auto: false,
                delete_branch: false,
            })),
        )
        .await
        .is_err();
        assert!(
            merge_timed_out,
            "forge_pr_merge must block while the repo write lock is held elsewhere"
        );

        // Release the lock: both calls now go through and route to the wrapper.
        drop(outer_guard);

        let out = server
            .forge_pr_checkout(Parameters(PrNumberParams { number: 7 }))
            .await
            .expect("checkout ok once the lock is free");
        assert!(
            result_json(&out).contains("checked_out"),
            "{}",
            result_json(&out)
        );

        let out = server
            .forge_pr_merge(Parameters(PrMergeParams {
                number: 7,
                strategy: MergeStrategyArg::Merge,
                auto: false,
                delete_branch: false,
            }))
            .await
            .expect("merge ok once the lock is free");
        assert!(
            result_json(&out).contains("merged"),
            "{}",
            result_json(&out)
        );
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
            .and_then(|c| c.as_text())
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
            .and_then(|c| c.as_text())
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
    // GitHub handle on a modern `gh`. `capabilities()` probes the CLI version
    // (`gh --version`, scripted to a modern banner above the 2.0 floor) and auth
    // (`auth status`, exit 0); every static cap is `true` post-fork, and the map
    // now also carries `version`/`supported`.
    #[tokio::test]
    async fn forge_info_with_authed_github_reports_all_true() {
        let gh = vcs_forge::vcs_github::GitHub::with_runner(
            ScriptedRunner::new()
                .on(
                    ["gh", "--version"],
                    Reply::ok("gh version 2.40.1 (2024-01-05)\n"),
                )
                .on(["gh", "auth", "status"], Reply::ok("")),
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
            .and_then(|c| c.as_text())
            .map(|t| t.text.clone())
            .expect("text content");
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(value["kind"], "github");
        assert_eq!(value["capabilities"]["authed"], true);
        assert_eq!(value["capabilities"]["supported"], true);
        // `version` serialises as the structured `{major,minor,patch}` shape of
        // `vcs_diff::Version` (its derived `Serialize`).
        assert_eq!(
            value["capabilities"]["version"],
            serde_json::json!({ "major": 2, "minor": 40, "patch": 1 })
        );
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

        // The review-action tools are destructive (they change a PR/MR's review state).
        let tool = VcsMcpServer::forge_pr_approve_tool_attr();
        let a = tool.annotations.expect("annotations present");
        assert_eq!(a.destructive_hint, Some(true));
        assert_eq!(a.read_only_hint, None);

        let tool = VcsMcpServer::forge_pr_request_changes_tool_attr();
        let a = tool.annotations.expect("annotations present");
        assert_eq!(a.destructive_hint, Some(true));
        assert_eq!(a.read_only_hint, None);

        // `forge_pr_checkout` mutates the working copy — destructive, not read-only.
        let tool = VcsMcpServer::forge_pr_checkout_tool_attr();
        let a = tool.annotations.expect("annotations present");
        assert_eq!(a.destructive_hint, Some(true));
        assert_eq!(a.read_only_hint, None);
    }

    // The macro-generated tool definitions carry the right MCP annotations: a
    // genuinely read-only tool (`repo_info` — no backend spawn) is read-only, a
    // mutation tool is destructive. (`repo_snapshot` used to be the read example
    // here, but T-068 reclassified it — it snapshots the jj working copy — so the
    // read example is now `repo_info`, the one repo_* read that never spawns.)
    #[test]
    fn tool_annotations_mark_read_vs_destructive() {
        let read = VcsMcpServer::repo_info_tool_attr();
        assert_eq!(read.annotations.unwrap().read_only_hint, Some(true));
        let write = VcsMcpServer::repo_commit_tool_attr();
        assert_eq!(write.annotations.unwrap().destructive_hint, Some(true));
    }

    // T-068 (variant C — strict MCP compliance). Every `repo_*` read tool that, on
    // the jj backend, dispatches to a plain (working-copy-**snapshotting**) jj
    // command records an op-log operation — so it must NOT assert `readOnlyHint`
    // ("does not modify its environment"), which would break the MCP contract. The
    // honest, backend-agnostic classification is non-destructive + idempotent (the
    // op-log snapshot is append-only/recoverable and changes no tracked content,
    // refs, or bookmarks; on git these tools are read-only, a strict subset). This
    // list is the *verified* set (checked against `vcs-jj`'s command construction and
    // `jj_backend.rs`), which is broader than the ticket's initial sketch: `repo_log`,
    // `repo_show_file`, and `repo_conflicts` snapshot too (`jj log` / `jj file show` /
    // `jj resolve --list` are all default-snapshotting), and are included here for
    // consistency. `repo_worktrees` snapshots via its top-level `jj workspace list`
    // (its per-workspace `workspace root` fan-out is already `--ignore-working-copy`).
    // Pinning all three annotation fields makes an accidental re-classification (or a
    // silent `read_only_hint = true` creeping back) fail the build.
    #[test]
    fn jj_snapshotting_read_tools_are_not_read_only_but_non_destructive() {
        let tools = [
            ("repo_snapshot", VcsMcpServer::repo_snapshot_tool_attr()),
            ("repo_status", VcsMcpServer::repo_status_tool_attr()),
            ("repo_diff_stat", VcsMcpServer::repo_diff_stat_tool_attr()),
            ("repo_diff", VcsMcpServer::repo_diff_tool_attr()),
            ("repo_log", VcsMcpServer::repo_log_tool_attr()),
            ("repo_show_file", VcsMcpServer::repo_show_file_tool_attr()),
            ("repo_branches", VcsMcpServer::repo_branches_tool_attr()),
            (
                "repo_current_branch",
                VcsMcpServer::repo_current_branch_tool_attr(),
            ),
            ("repo_conflicts", VcsMcpServer::repo_conflicts_tool_attr()),
            ("repo_worktrees", VcsMcpServer::repo_worktrees_tool_attr()),
        ];
        for (name, tool) in tools {
            let a = tool
                .annotations
                .unwrap_or_else(|| panic!("{name} must carry annotations"));
            assert_eq!(
                a.read_only_hint, None,
                "{name} must NOT assert readOnlyHint — on jj it snapshots the working \
                 copy (records an op-log operation), so the read-only claim is false"
            );
            assert_eq!(
                a.destructive_hint,
                Some(false),
                "{name} is non-destructive (the jj op-log snapshot is append-only and \
                 recoverable; no tracked content/refs/bookmarks change)"
            );
            assert_eq!(
                a.idempotent_hint,
                Some(true),
                "{name} is idempotent (a re-run with no interim filesystem edit records \
                 no further op-log operation)"
            );
        }
    }

    // T-068: the complement. The genuinely backend-agnostic read-only tools KEEP
    // `readOnlyHint = true`. `repo_info` makes no backend spawn at all (cached
    // kind/root/cwd + forge kind); every `forge_*` read tool drives the forge CLI, not
    // the jj working copy — so neither can snapshot, and the read-only claim holds on
    // both backends. This is the consistency half of the fix: only the tools that
    // *actually* reach a snapshotting jj command were reclassified, not the whole read
    // surface.
    #[test]
    fn truly_read_only_tools_keep_read_only_hint() {
        let tools = [
            ("repo_info", VcsMcpServer::repo_info_tool_attr()),
            (
                "forge_auth_status",
                VcsMcpServer::forge_auth_status_tool_attr(),
            ),
            ("forge_repo_view", VcsMcpServer::forge_repo_view_tool_attr()),
            ("forge_pr_list", VcsMcpServer::forge_pr_list_tool_attr()),
            ("forge_pr_view", VcsMcpServer::forge_pr_view_tool_attr()),
            ("forge_pr_checks", VcsMcpServer::forge_pr_checks_tool_attr()),
            ("forge_pr_diff", VcsMcpServer::forge_pr_diff_tool_attr()),
            (
                "forge_issue_list",
                VcsMcpServer::forge_issue_list_tool_attr(),
            ),
            (
                "forge_issue_view",
                VcsMcpServer::forge_issue_view_tool_attr(),
            ),
            (
                "forge_release_list",
                VcsMcpServer::forge_release_list_tool_attr(),
            ),
            (
                "forge_release_view",
                VcsMcpServer::forge_release_view_tool_attr(),
            ),
            ("forge_info", VcsMcpServer::forge_info_tool_attr()),
        ];
        for (name, tool) in tools {
            let a = tool
                .annotations
                .unwrap_or_else(|| panic!("{name} must carry annotations"));
            assert_eq!(
                a.read_only_hint,
                Some(true),
                "{name} is genuinely read-only on both backends and must keep readOnlyHint"
            );
        }
    }

    // T-068: reclassifying the jj-snapshotting reads must NOT change their
    // availability — they stay ordinary read tools, callable in the default
    // read-only mode. An op-log snapshot mutates neither tracked content nor refs, so
    // (unlike `repo_try_merge`, which materializes working-tree content that can run
    // untrusted filter/textconv drivers) it needs no `--allow-write`; none of these
    // names may leak into `WRITE_TOOLS`. Two of them are also exercised end-to-end
    // under `WriteGate::None` to prove they run without a gate.
    #[tokio::test]
    async fn reclassified_reads_stay_ungated_and_callable() {
        for name in [
            "repo_snapshot",
            "repo_status",
            "repo_diff_stat",
            "repo_diff",
            "repo_log",
            "repo_show_file",
            "repo_branches",
            "repo_current_branch",
            "repo_conflicts",
            "repo_worktrees",
        ] {
            assert!(
                !WRITE_TOOLS.contains(&name),
                "{name} is a read tool — it must not be write-gated"
            );
        }

        // End-to-end: they run under the default read-only gate (no --allow-write).
        let server = git_server(
            ScriptedRunner::new()
                .on(["git", "status"], Reply::ok(" M a.rs\0"))
                .on(["git", "symbolic-ref"], Reply::ok("main\n")),
            WriteGate::None,
        );
        server.repo_status().await.expect("repo_status ungated");
        server
            .repo_current_branch()
            .await
            .expect("repo_current_branch ungated");
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
        assert!(names.contains(&"forge_pr_approve"), "{names:?}");
        assert!(names.contains(&"forge_pr_request_changes"), "{names:?}");
        assert!(names.contains(&"forge_pr_checkout"), "{names:?}");
        assert!(names.contains(&"forge_info"), "{names:?}");

        let result = client
            .call_tool(CallToolRequestParams::new("repo_current_branch"))
            .await
            .expect("call repo_current_branch");
        let text = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.as_str())
            .expect("text content");
        assert!(text.contains("main"), "{text}");

        let _ = client.cancel().await;
        server_handle.abort();
    }

    #[tokio::test]
    async fn repo_rebase_is_gated_and_rebases() {
        let server = git_server(ScriptedRunner::new(), WriteGate::None);
        let err = server
            .repo_rebase(Parameters(RebaseParams {
                onto: "main".into(),
            }))
            .await
            .expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

        let server = git_server(
            ScriptedRunner::new().on(["git", "rebase"], Reply::ok("")),
            WriteGate::All,
        );
        let out = server
            .repo_rebase(Parameters(RebaseParams {
                onto: "main".into(),
            }))
            .await
            .expect("rebase ok");
        assert!(!result_json(&out).is_empty());
    }

    #[tokio::test]
    async fn repo_abort_in_progress_is_gated() {
        let server = git_server(ScriptedRunner::new(), WriteGate::None);
        let err = server.repo_abort_in_progress().await.expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

        let server = git_server(
            ScriptedRunner::new().on(["git", "rev-parse"], Reply::ok("/repo/.git\n")),
            WriteGate::All,
        );
        let out = server.repo_abort_in_progress().await.expect("abort ok");
        assert!(result_json(&out).contains("operation_state"));
    }

    #[tokio::test]
    async fn repo_continue_in_progress_is_gated() {
        let server = git_server(ScriptedRunner::new(), WriteGate::None);
        let err = server.repo_continue_in_progress().await.expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

        let server = git_server(
            ScriptedRunner::new()
                .on(["git", "diff"], Reply::ok(""))
                .on(["git", "rev-parse"], Reply::ok("/repo/.git\n")),
            WriteGate::All,
        );
        let out = server
            .repo_continue_in_progress()
            .await
            .expect("continue ok");
        assert!(result_json(&out).contains("operation_state"));
    }

    #[tokio::test]
    async fn repo_new_child_is_gated_and_creates() {
        let server = git_server(ScriptedRunner::new(), WriteGate::None);
        let err = server
            .repo_new_child(Parameters(NewChildParams {
                reference: "main".into(),
            }))
            .await
            .expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

        let server = git_server(
            ScriptedRunner::new().on(["git", "checkout"], Reply::ok("")),
            WriteGate::All,
        );
        let out = server
            .repo_new_child(Parameters(NewChildParams {
                reference: "main".into(),
            }))
            .await
            .expect("new child ok");
        assert!(!result_json(&out).is_empty());
    }

    #[tokio::test]
    async fn repo_create_branch_is_gated() {
        let server = git_server(ScriptedRunner::new(), WriteGate::None);
        let err = server
            .repo_create_branch(Parameters(CreateBranchParams {
                name: "feature".into(),
            }))
            .await
            .expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

        let server = git_server(
            ScriptedRunner::new().on(["git", "branch"], Reply::ok("")),
            WriteGate::All,
        );
        let out = server
            .repo_create_branch(Parameters(CreateBranchParams {
                name: "feature".into(),
            }))
            .await
            .expect("create branch ok");
        assert!(
            result_json(&out).contains("created_branch"),
            "{}",
            result_json(&out)
        );
    }

    #[tokio::test]
    async fn repo_delete_branch_is_gated() {
        let server = git_server(ScriptedRunner::new(), WriteGate::None);
        let err = server
            .repo_delete_branch(Parameters(DeleteBranchParams {
                name: "feature".into(),
                force: false,
            }))
            .await
            .expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

        let server = git_server(
            ScriptedRunner::new().on(["git", "branch"], Reply::ok("")),
            WriteGate::All,
        );
        let out = server
            .repo_delete_branch(Parameters(DeleteBranchParams {
                name: "feature".into(),
                force: false,
            }))
            .await
            .expect("delete branch ok");
        assert!(!result_json(&out).is_empty());
    }

    #[tokio::test]
    async fn repo_rename_branch_is_gated() {
        let server = git_server(ScriptedRunner::new(), WriteGate::None);
        let err = server
            .repo_rename_branch(Parameters(RenameBranchParams {
                old: "old".into(),
                new: "new".into(),
            }))
            .await
            .expect_err("gated");
        assert!(format!("{err:?}").contains("allow-write"), "{err:?}");

        let server = git_server(
            ScriptedRunner::new().on(["git", "branch"], Reply::ok("")),
            WriteGate::All,
        );
        let out = server
            .repo_rename_branch(Parameters(RenameBranchParams {
                old: "old".into(),
                new: "new".into(),
            }))
            .await
            .expect("rename branch ok");
        assert!(!result_json(&out).is_empty());
    }
}

// Long-form how-to guides, rendered from this crate's docs/*.md on docs.rs.
#[doc = include_str!("../docs/mcp.md")]
#[allow(rustdoc::broken_intra_doc_links)]
pub mod guide {}
