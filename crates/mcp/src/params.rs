//! Tool parameter structs: one `serde::Deserialize` + `schemars::JsonSchema`
//! struct per tool-with-arguments. Each struct's derived schema is the tool's
//! advertised MCP input schema. These are re-exported from the crate root, so
//! their public paths (`vcs_mcp::CommitParams`, …) are unchanged.

use rmcp::schemars;
use serde::Deserialize;

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

/// Attribute each line of a file.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AnnotateParams {
    /// Repo-relative path of the file to annotate.
    pub path: String,
    /// Optional git revspec / jj revset. Omit for git `HEAD` / jj `@`.
    #[serde(default)]
    pub rev: Option<String>,
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

/// Post a comment to an existing issue.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IssueCommentParams {
    /// The issue number (GitLab uses the project-scoped `iid`).
    pub number: u64,
    /// The markdown comment body.
    pub body: String,
}

/// A release by tag.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReleaseTagParams {
    /// The release's Git tag.
    pub tag: String,
}

/// Create a release.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReleaseCreateParams {
    /// The release's Git tag. GitHub creates the tag from the default branch if it
    /// doesn't exist yet; GitLab/Gitea expect it to exist.
    pub tag: String,
    /// The release title; omit for the forge's default (commonly the tag).
    #[serde(default)]
    pub title: Option<String>,
    /// The release notes / description (markdown); omit to leave it unset.
    #[serde(default)]
    pub notes: Option<String>,
    /// Save as a draft instead of publishing. **GitHub/Gitea only** — GitLab rejects
    /// it as unsupported (`invalid_params`) rather than ignoring it. Defaults to
    /// `false`.
    #[serde(default)]
    pub draft: bool,
    /// Mark as a prerelease. **GitHub/Gitea only** — GitLab rejects it as
    /// unsupported (`invalid_params`). Defaults to `false`.
    #[serde(default)]
    pub prerelease: bool,
}

/// How [`forge_pr_merge`](crate::VcsMcpServer::forge_pr_merge) merges.
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
