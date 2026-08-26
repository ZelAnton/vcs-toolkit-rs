//! Tool parameter structs: one `serde::Deserialize` + `schemars::JsonSchema`
//! struct per tool-with-arguments. Each struct's derived schema is the tool's
//! advertised MCP input schema. These are re-exported from the crate root, so
//! their public paths (`vcs_mcp::CommitParams`, …) are unchanged.

use rmcp::schemars;
use serde::Deserialize;

/// Detail requested from the outcome-oriented changes service.
#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OutcomeChangesModeArg {
    /// Changed paths, kinds, counts, and aggregate line statistics.
    Summary,
    /// Summary plus structured file hunks and lines.
    Full,
}

/// Parameters for the outcome-oriented repository changes workflow.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OutcomeChangesParams {
    /// Detail level. Defaults to `summary`.
    #[serde(default)]
    pub mode: Option<OutcomeChangesModeArg>,
}

/// Checked exact-path commit request.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OutcomeCommitParams {
    /// Revision that must still be current at preflight and after selection.
    pub expected_revision: String,
    /// Non-empty commit message.
    pub message: String,
    /// Exact repo-relative leaf paths to commit, preserving every unrelated change.
    pub paths: Vec<String>,
}

/// Checked exact-revision publish request.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OutcomePublishParams {
    /// Revision that must be current and must be the revision published.
    pub expected_revision: String,
    /// Expected remote state before push: a revision or the literal `absent`.
    pub expected_remote_revision: String,
    /// Configured remote name.
    pub remote: String,
    /// Local source branch/bookmark.
    pub source: String,
    /// Pull/merge request target branch.
    pub target: String,
    /// Expected forge (`github`, `gitlab`, or `gitea`).
    pub forge: String,
    /// Expected active forge account.
    pub expected_account: String,
    /// Pull/merge request title.
    pub title: String,
    /// Pull/merge request body; may be empty.
    pub body: String,
}

/// Exact-revision CI status request.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OutcomeCiStatusParams {
    /// Expected forge (`github`, `gitlab`, or `gitea`).
    pub forge: String,
    /// Published source branch/bookmark.
    pub source: String,
    /// Exact revision whose runs may be reported.
    pub expected_revision: String,
}

/// Exact-revision CI wait request.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OutcomeCiWaitParams {
    /// Expected forge (`github`, `gitlab`, or `gitea`).
    pub forge: String,
    /// Published source branch/bookmark.
    pub source: String,
    /// Exact revision whose runs may be reported.
    pub expected_revision: String,
    /// Total deadline in seconds. Defaults to 1800.
    #[serde(default)]
    pub wait_seconds: Option<u64>,
    /// Poll interval in seconds. Defaults to 10.
    #[serde(default)]
    pub poll_seconds: Option<u64>,
}

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

/// List recent repository operation-log entries (jj only).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OpLogParams {
    /// Maximum number of operations to return, newest first.
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

/// Read a conflicted file's parsed conflict regions.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConflictRegionsParams {
    /// Repo-relative path of the conflicted file, `/`-separated. Read from the
    /// **working copy** (that is where conflict markers are materialized on both
    /// backends), so it must stay inside the repository: an absolute path, or one
    /// containing a `..` component, is refused.
    pub path: String,
}

/// Replace a conflicted file's regions with one chosen side.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ResolveConflictParams {
    /// Repo-relative path of the conflicted file, `/`-separated. Same containment
    /// rules as [`ConflictRegionsParams::path`]; additionally the path must be one
    /// the backend currently reports as conflicted.
    pub path: String,
    /// Which side to keep in every conflict region of the file.
    pub side: ConflictSideArg,
    /// The 0-based side index, **required with (and only with)**
    /// `side = "side"` — jj's n-way form. File order, so `0` is the first side.
    #[serde(default)]
    pub index: Option<usize>,
}

/// Which side [`repo_resolve_conflict`](crate::VcsMcpServer::repo_resolve_conflict)
/// keeps. The two backends have genuinely different domains, so not every value is
/// valid everywhere — a value the current backend (or the file's actual conflict
/// shape) cannot honour is refused *before* anything is written, never silently
/// approximated:
///
/// | value | git | jj |
/// |---|---|---|
/// | `ours` | the `<<<<<<<` side | the first side (`Side(0)`) |
/// | `theirs` | the `>>>>>>>` side | the second side — **only** when every region is 2-sided |
/// | `base` | the `\|\|\|\|\|\|\|` base (diff3/zdiff3 only) | the recorded base |
/// | `side` | refused — git's sides are named | `Side(index)`, any arity |
// `Serialize` too (unlike the other param enums): the resolve tool echoes the
// chosen side back in its result, so the agent's own transcript records which
// side it destroyed the other of — `rename_all` keeps that echo spelled exactly
// as the input was.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ConflictSideArg {
    /// Keep "our" side — git's `<<<<<<<` side, jj's first side.
    Ours,
    /// Keep the merge base. Refused when the region records none (git's 2-way
    /// `merge` conflict style records no base).
    Base,
    /// Keep "their" side — git's `>>>>>>>` side, jj's second side. On jj this is
    /// refused for a conflict with more than two sides, where "theirs" is
    /// ambiguous; use `side` with an explicit `index` there.
    Theirs,
    /// Keep the `index`-th side (0-based, file order). **jj only** — git's three
    /// sides are named, so `ours`/`base`/`theirs` address them exactly.
    Side,
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

/// Filters for listing pull/merge requests. Omit either field for the facade
/// default (`open`, limit 100).
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct PrListParams {
    /// State filter.
    #[serde(default)]
    pub state: Option<PrListStateArg>,
    /// Maximum number requested from the forge CLI.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Pull/merge-request state accepted by [`PrListParams`].
#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PrListStateArg {
    /// Open pull/merge requests.
    Open,
    /// Closed, unmerged pull/merge requests.
    Closed,
    /// Merged pull/merge requests (unsupported by Gitea's `tea` CLI).
    Merged,
    /// Pull/merge requests in every state.
    All,
}

impl From<PrListParams> for vcs_forge::PrList {
    fn from(p: PrListParams) -> Self {
        let mut spec = vcs_forge::PrList::new();
        if let Some(state) = p.state {
            spec = spec.state(match state {
                PrListStateArg::Open => vcs_forge::PrListState::Open,
                PrListStateArg::Closed => vcs_forge::PrListState::Closed,
                PrListStateArg::Merged => vcs_forge::PrListState::Merged,
                PrListStateArg::All => vcs_forge::PrListState::All,
            });
        }
        if let Some(limit) = p.limit {
            spec = spec.limit(limit);
        }
        spec
    }
}

/// Filters for listing issues. Omit either field for the facade default (`open`,
/// limit 100).
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct IssueListParams {
    /// State filter.
    #[serde(default)]
    pub state: Option<IssueListStateArg>,
    /// Maximum number requested from the forge CLI.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Issue state accepted by [`IssueListParams`].
#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum IssueListStateArg {
    /// Open issues.
    Open,
    /// Closed issues.
    Closed,
    /// Issues in every state.
    All,
}

impl From<IssueListParams> for vcs_forge::IssueList {
    fn from(p: IssueListParams) -> Self {
        let mut spec = vcs_forge::IssueList::new();
        if let Some(state) = p.state {
            spec = spec.state(match state {
                IssueListStateArg::Open => vcs_forge::IssueListState::Open,
                IssueListStateArg::Closed => vcs_forge::IssueListState::Closed,
                IssueListStateArg::All => vcs_forge::IssueListState::All,
            });
        }
        if let Some(limit) = p.limit {
            spec = spec.limit(limit);
        }
        spec
    }
}

/// A pull/merge request by number.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrNumberParams {
    /// The PR/MR number (GitLab uses the project-scoped `iid`).
    pub number: u64,
}

/// Pull/merge requests whose source branch matches this branch.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrForBranchParams {
    /// Source/head branch to query, independent of the target branch and state.
    pub source_branch: String,
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
    /// Labels to apply while creating the PR/MR.
    #[serde(default)]
    pub labels: Vec<String>,
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

/// Edit a GitHub pull request or GitLab merge request's title and/or body.
/// Gitea is unsupported because `tea` has no `pr edit` subcommand.
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
    /// Labels to apply while creating the issue.
    #[serde(default)]
    pub labels: Vec<String>,
}

/// Add or remove labels on an existing PR/MR or issue.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LabelsParams {
    /// The PR/MR or issue number (GitLab uses the project-scoped `iid`).
    pub number: u64,
    /// One or more non-empty label names.
    pub labels: Vec<String>,
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
