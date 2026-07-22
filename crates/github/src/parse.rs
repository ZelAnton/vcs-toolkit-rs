//! Typed results from `gh … --json` and the deserialization helpers. Parsing is
//! pure, so these tests are hermetic and run on CI.

use processkit::Result;
use serde::Deserialize;

use crate::BINARY;

/// Parse `gh --version` output (`gh version 2.40.1 (2024-01-05)`) into the shared
/// [`vcs_diff::Version`]: the first dotted-numeric token wins, so gh's `(date)` and
/// the release-URL trailer on the next line are ignored. `None` when the banner
/// carries no version token. Reuses the same tolerant parser `vcs-git`/`vcs-jj`
/// gate on, so the three CLIs share one version-parsing contract.
pub(crate) fn parse_gh_version(raw: &str) -> Option<vcs_diff::Version> {
    vcs_diff::parse_dotted_version(raw)
}

/// A pull request
/// (`gh pr list/view --json number,title,state,isDraft,headRefName,baseRefName,url`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct PullRequest {
    /// PR number.
    pub number: u64,
    /// PR title.
    pub title: String,
    /// State, e.g. `"OPEN"`, `"MERGED"`, `"CLOSED"`.
    pub state: String,
    /// Whether the PR is a draft (`gh --json isDraft`).
    #[serde(rename = "isDraft", default)]
    pub is_draft: bool,
    /// Source (head) branch name.
    #[serde(
        rename = "headRefName",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub head_ref_name: String,
    /// Target (base) branch name.
    #[serde(
        rename = "baseRefName",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub base_ref_name: String,
    /// Web URL.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub url: String,
    /// Labels attached to the PR (gh `--json labels`, flattened from
    /// `[{"name": "bug", ...}]` to plain names).
    #[serde(default, deserialize_with = "labels_to_names")]
    pub labels: Vec<String>,
    /// Logins of assigned users (gh `--json assignees`, flattened from
    /// `[{"login": "octocat", ...}]` to plain logins).
    #[serde(default, deserialize_with = "assignees_to_logins")]
    pub assignees: Vec<String>,
    /// Author's login (gh `--json author`, flattened from `{"login": …}`; a
    /// deleted account's `null` author becomes an empty string, matching the
    /// existing PR feedback author flatten).
    #[serde(default, deserialize_with = "author_login")]
    pub author: String,
    /// Creation timestamp (RFC 3339) (gh `--json createdAt`).
    #[serde(
        rename = "createdAt",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub created_at: String,
    /// Last-update timestamp (RFC 3339) (gh `--json updatedAt`).
    #[serde(
        rename = "updatedAt",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub updated_at: String,
    /// Milestone title, or `None` when no milestone is attached (gh `--json
    /// milestone`, flattened from `{"title": …}`; a `null` milestone becomes
    /// `None`).
    #[serde(default, deserialize_with = "milestone_to_title")]
    pub milestone: Option<String>,
}

/// An issue (`gh issue list --json number,title,state`;
/// `gh issue view` additionally fills `body`/`url`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Issue {
    /// Issue number.
    pub number: u64,
    /// Issue title.
    pub title: String,
    /// State, e.g. `"OPEN"`, `"CLOSED"`.
    pub state: String,
    /// Issue body (markdown). Fetched by both `issue_list` and `issue_view`.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub body: String,
    /// Web URL. Fetched by both `issue_list` and `issue_view`.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub url: String,
    /// Labels attached to the issue (gh `--json labels`, flattened from
    /// `[{"name": "bug", ...}]` to plain names).
    #[serde(default, deserialize_with = "labels_to_names")]
    pub labels: Vec<String>,
    /// Logins of assigned users (gh `--json assignees`, flattened from
    /// `[{"login": "octocat", ...}]` to plain logins).
    #[serde(default, deserialize_with = "assignees_to_logins")]
    pub assignees: Vec<String>,
    /// Author's login (gh `--json author`, flattened from `{"login": …}`; a
    /// deleted account's `null` author becomes an empty string, matching the
    /// existing PR feedback author flatten).
    #[serde(default, deserialize_with = "author_login")]
    pub author: String,
    /// Creation timestamp (RFC 3339) (gh `--json createdAt`).
    #[serde(
        rename = "createdAt",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub created_at: String,
    /// Last-update timestamp (RFC 3339) (gh `--json updatedAt`).
    #[serde(
        rename = "updatedAt",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub updated_at: String,
    /// Milestone title, or `None` when no milestone is attached (gh `--json
    /// milestone`, flattened from `{"title": …}`; a `null` milestone becomes
    /// `None`).
    #[serde(default, deserialize_with = "milestone_to_title")]
    pub milestone: Option<String>,
}

// gh emits both `labels` and `assignees` as arrays of objects (`[{"name": …}]`,
// `[{"login": …}]`), not plain strings — flatten each into a `Vec<String>`.
// `Option<Vec<_>>` (not a bare `Vec<_>`) so a present JSON `null` — like the
// other optional fields in this file — degrades to an empty list rather than
// failing the whole parse.
#[derive(Deserialize)]
struct LabelJson {
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    name: String,
}

#[derive(Deserialize)]
struct AssigneeJson {
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    login: String,
}

fn labels_to_names<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<Vec<LabelJson>>::deserialize(deserializer)?.unwrap_or_default();
    Ok(raw.into_iter().map(|l| l.name).collect())
}

fn assignees_to_logins<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<Vec<AssigneeJson>>::deserialize(deserializer)?.unwrap_or_default();
    Ok(raw.into_iter().map(|a| a.login).collect())
}

// gh nests a PR/issue/release `author` as `{"login": …}` (and reports `null` for
// a deleted account) — the same shape `AuthorJson` (below) flattens for PR
// feedback; reused here so an author's `null` uniformly becomes an empty login.
fn author_login<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<AuthorJson>::deserialize(deserializer)?;
    Ok(raw.map(|a| a.login).unwrap_or_default())
}

fn author_login_opt<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<AuthorJson>::deserialize(deserializer)?;
    Ok(raw.map(|a| a.login))
}

// gh nests `milestone` as `{"title": …}`, `null` when none is attached.
#[derive(Deserialize)]
struct MilestoneJson {
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    title: String,
}

fn milestone_to_title<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<MilestoneJson>::deserialize(deserializer)?;
    Ok(raw.map(|m| m.title))
}

/// A GitHub Actions workflow run (`gh run list/view --json …`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct WorkflowRun {
    /// The run id (`databaseId`) — the `<run-id>` other `gh run` commands take.
    #[serde(rename = "databaseId")]
    pub database_id: u64,
    /// Workflow name as shown in the runs list.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub name: String,
    /// The run's display title (usually the commit subject).
    #[serde(
        rename = "displayTitle",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub display_title: String,
    /// Lifecycle status, e.g. `"queued"`, `"in_progress"`, `"completed"`.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub status: String,
    /// Outcome, e.g. `"success"`, `"failure"`, `"cancelled"`, `"skipped"` —
    /// gh reports an **empty string** until the run completes (not `null`).
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub conclusion: String,
    /// Name of the workflow that produced the run.
    #[serde(
        rename = "workflowName",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub workflow_name: String,
    /// Branch the run was triggered for.
    #[serde(
        rename = "headBranch",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub head_branch: String,
    /// Triggering event, e.g. `"push"`, `"workflow_dispatch"`.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub event: String,
    /// Web URL.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub url: String,
    /// Creation timestamp (ISO 8601).
    #[serde(
        rename = "createdAt",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub created_at: String,
}

/// gh's coarse categorisation of a [`CheckRun`]'s state — the field to branch on
/// when deciding whether CI passed. `gh` derives it from the raw `state`; this is
/// the typed form of its `pass`/`fail`/`pending`/`skipping`/`cancel` strings.
///
/// `#[non_exhaustive]` with an [`Unknown`](CheckBucket::Unknown) catch-all: a
/// bucket name a future `gh` introduces (or a missing field) deserialises to
/// `Unknown` rather than failing the parse, so the wrapper never breaks on an
/// unmodelled value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CheckBucket {
    /// The check succeeded.
    Pass,
    /// The check failed.
    Fail,
    /// The check is queued or still running.
    Pending,
    /// The check was skipped (e.g. a conditional job that didn't run).
    Skipping,
    /// The check was cancelled.
    Cancel,
    /// A bucket `gh` reported that this version doesn't model, or an absent field.
    #[default]
    #[serde(other)]
    Unknown,
}

impl CheckBucket {
    /// Whether this bucket means the check failed or was cancelled — the states
    /// that should fail an aggregate CI verdict.
    pub fn is_failing(self) -> bool {
        matches!(self, CheckBucket::Fail | CheckBucket::Cancel)
    }

    /// Whether this bucket means the check is still in flight (queued/running).
    pub fn is_pending(self) -> bool {
        matches!(self, CheckBucket::Pending)
    }

    /// Whether this bucket means the check completed successfully.
    pub fn is_passing(self) -> bool {
        matches!(self, CheckBucket::Pass)
    }

    /// Whether this is the [`Unknown`](CheckBucket::Unknown) catch-all — a bucket a
    /// future `gh` introduced (or a missing field) that this version doesn't model.
    /// Distinct from [`Skipping`](CheckBucket::Skipping): a skip is a deliberate,
    /// terminal no-op, whereas an unknown bucket is *unclassified* and should be
    /// treated conservatively (as "not known to be done") by an aggregator.
    pub fn is_unknown(self) -> bool {
        matches!(self, CheckBucket::Unknown)
    }
}

/// One check on a PR (`gh pr checks --json …`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct CheckRun {
    /// Check name.
    pub name: String,
    /// Raw state, e.g. `"SUCCESS"`, `"FAILURE"`, `"IN_PROGRESS"`.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub state: String,
    /// gh's categorisation of `state` — the field to branch on. See [`CheckBucket`].
    #[serde(default)]
    pub bucket: CheckBucket,
    /// Workflow the check belongs to (empty for non-Actions checks).
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub workflow: String,
    /// Web link to the check's details.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub link: String,
    /// Start timestamp (ISO 8601), empty until started.
    #[serde(
        rename = "startedAt",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub started_at: String,
    /// Completion timestamp (ISO 8601), empty until completed.
    #[serde(
        rename = "completedAt",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub completed_at: String,
}

/// A release (`gh release list/view --json …`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Release {
    /// The release's tag.
    #[serde(rename = "tagName")]
    pub tag_name: String,
    /// Release title (may be empty/null).
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub name: String,
    /// Release notes (markdown). `None` from `release_list`, which doesn't request
    /// the field (only `release_view` does) — so an absent value reads as the
    /// honest "not fetched", not a false empty string. A present JSON `null` (a
    /// release genuinely without notes) likewise reads as `None`.
    #[serde(default)]
    pub body: Option<String>,
    /// Web URL. `None` from `release_list`, which doesn't request the field (only
    /// `release_view` does) — so an absent value reads as the honest "not fetched",
    /// not a false empty string. A present JSON `null` likewise reads as `None`.
    #[serde(default)]
    pub url: Option<String>,
    /// Publication timestamp (ISO 8601); empty/null for a draft.
    #[serde(
        rename = "publishedAt",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub published_at: String,
    /// `true` for an unpublished draft.
    #[serde(rename = "isDraft", default)]
    pub is_draft: bool,
    /// `true` for a prerelease.
    #[serde(rename = "isPrerelease", default)]
    pub is_prerelease: bool,
    /// `true` for the latest release. Only `release_list` reports this field;
    /// from `release_view` it defaults to `false`.
    #[serde(rename = "isLatest", default)]
    pub is_latest: bool,
    /// Release author's login. `None` from `release_list`, which doesn't request
    /// the field (only `release_view` does) — so an absent value reads as the
    /// honest "not fetched", not a false empty string. A present author object
    /// with no login for a deleted or anonymized account becomes `Some("")`.
    #[serde(default, deserialize_with = "author_login_opt")]
    pub author: Option<String>,
}

/// A submitted PR review (from `gh pr view --json reviews`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Review {
    /// Reviewer login.
    pub author: String,
    /// Review state: `"APPROVED"`, `"CHANGES_REQUESTED"`, `"COMMENTED"`,
    /// `"DISMISSED"` or `"PENDING"`.
    pub state: String,
    /// Review body (may be empty).
    pub body: String,
    /// Submission timestamp (ISO 8601).
    pub submitted_at: String,
}

/// A PR conversation comment (from `gh pr view --json comments`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Comment {
    /// Commenter login.
    pub author: String,
    /// Comment body.
    pub body: String,
    /// Web URL of the comment.
    pub url: String,
    /// Creation timestamp (ISO 8601).
    pub created_at: String,
}

/// The review/comment feedback on a PR (`gh pr view --json reviews,comments`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PrFeedback {
    /// Submitted reviews, oldest first (gh's order).
    pub reviews: Vec<Review>,
    /// Conversation comments, oldest first (gh's order).
    pub comments: Vec<Comment>,
}

/// A repository (`gh repo view --json name,owner,description,url,isPrivate,defaultBranchRef`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RepoView {
    /// Repository name.
    pub name: String,
    /// Owner login.
    pub owner: String,
    /// Description, `None` when GitHub returns `null`.
    pub description: Option<String>,
    /// Web URL.
    pub url: String,
    /// `true` for a private repository.
    pub is_private: bool,
    /// Default branch name (empty for an empty repository).
    pub default_branch: String,
}

// gh nests `owner` and `defaultBranchRef` as objects; deserialize into this and
// flatten into the public `RepoView`.
#[derive(Deserialize)]
struct RepoJson {
    name: String,
    owner: OwnerJson,
    #[serde(default)]
    description: Option<String>,
    url: String,
    #[serde(rename = "isPrivate")]
    is_private: bool,
    #[serde(rename = "defaultBranchRef", default)]
    default_branch_ref: Option<BranchRefJson>,
}

#[derive(Deserialize)]
struct OwnerJson {
    login: String,
}

#[derive(Deserialize)]
struct BranchRefJson {
    name: String,
}

/// Parse `gh repo view --json …` output, flattening the nested objects.
pub(crate) fn parse_repo(json: &str) -> Result<RepoView> {
    let raw: RepoJson = vcs_cli_support::json::from_json(BINARY, json)?;
    Ok(RepoView {
        name: raw.name,
        owner: raw.owner.login,
        description: raw.description,
        url: raw.url,
        is_private: raw.is_private,
        default_branch: raw.default_branch_ref.map(|b| b.name).unwrap_or_default(),
    })
}

// gh nests the author as `{"login": …}` (and reports `null` for a deleted
// account); deserialize into these and flatten into the public types.
#[derive(Deserialize)]
struct FeedbackJson {
    #[serde(default)]
    reviews: Vec<ReviewJson>,
    #[serde(default)]
    comments: Vec<CommentJson>,
}

// Optional string fields use `null_to_empty` (not bare `default`) so a present
// JSON `null` maps to "" like an absent key — uniform with the rest of this
// crate's `gh --json` DTOs, robust to whatever `gh` emits for an empty value.
#[derive(Deserialize)]
struct ReviewJson {
    #[serde(default)]
    author: Option<AuthorJson>,
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    state: String,
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    body: String,
    #[serde(
        rename = "submittedAt",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    submitted_at: String,
}

#[derive(Deserialize)]
struct CommentJson {
    #[serde(default)]
    author: Option<AuthorJson>,
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    body: String,
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    url: String,
    #[serde(
        rename = "createdAt",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    created_at: String,
}

#[derive(Deserialize)]
struct AuthorJson {
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    login: String,
}

/// Parse `gh pr view --json reviews,comments` output, flattening the nested
/// author objects (a deleted account's `null` author becomes an empty login).
pub(crate) fn parse_feedback(json: &str) -> Result<PrFeedback> {
    let raw: FeedbackJson = vcs_cli_support::json::from_json(BINARY, json)?;
    Ok(PrFeedback {
        reviews: raw
            .reviews
            .into_iter()
            .map(|r| Review {
                author: r.author.map(|a| a.login).unwrap_or_default(),
                state: r.state,
                body: r.body,
                submitted_at: r.submitted_at,
            })
            .collect(),
        comments: raw
            .comments
            .into_iter()
            .map(|c| Comment {
                author: c.author.map(|a| a.login).unwrap_or_default(),
                body: c.body,
                url: c.url,
                created_at: c.created_at,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::Error;

    #[test]
    fn parses_pr_list() {
        let json = r#"[
            {"number": 12, "title": "Add feature", "state": "OPEN", "isDraft": true,
             "headRefName": "feat/x", "baseRefName": "main", "url": "https://gh/pr/12"}
        ]"#;
        let prs: Vec<PullRequest> =
            vcs_cli_support::json::from_json(BINARY, json).expect("parse prs");
        assert_eq!(prs.len(), 1);
        assert_eq!(
            prs[0],
            PullRequest {
                number: 12,
                title: "Add feature".into(),
                state: "OPEN".into(),
                is_draft: true,
                head_ref_name: "feat/x".into(),
                base_ref_name: "main".into(),
                url: "https://gh/pr/12".into(),
                labels: Vec::new(),
                assignees: Vec::new(),
                author: String::new(),
                created_at: String::new(),
                updated_at: String::new(),
                milestone: None,
            }
        );
    }

    // Positive case: gh's `--json labels,assignees` shape (`[{"name": …}]`,
    // `[{"login": …}]`) flattens to plain `Vec<String>`.
    #[test]
    fn pr_parses_labels_and_assignees() {
        let json = r#"{"number": 12, "title": "Add feature", "state": "OPEN", "isDraft": false,
            "headRefName": "feat/x", "baseRefName": "main", "url": "https://gh/pr/12",
            "labels": [{"name": "bug", "color": "f00"}, {"name": "priority-1"}],
            "assignees": [{"login": "octocat", "id": 1}, {"login": "hubot"}]}"#;
        let pr: PullRequest =
            vcs_cli_support::json::from_json(BINARY, json).expect("parse pr with labels/assignees");
        assert_eq!(pr.labels, vec!["bug".to_string(), "priority-1".to_string()]);
        assert_eq!(
            pr.assignees,
            vec!["octocat".to_string(), "hubot".to_string()]
        );
    }

    // Negative case: an empty `labels`/`assignees` array parses to an empty
    // `Vec`, not a panic or parse error. And when the keys are absent entirely
    // (e.g. an older canned fixture), `#[serde(default)]` fills the same empty
    // `Vec`.
    #[test]
    fn pr_without_labels_or_assignees_parses_to_empty_vecs() {
        let json = r#"{"number": 13, "title": "t", "state": "OPEN", "isDraft": false,
            "headRefName": "h", "baseRefName": "main", "url": "u",
            "labels": [], "assignees": []}"#;
        let pr: PullRequest =
            vcs_cli_support::json::from_json(BINARY, json).expect("PR with empty labels/assignees");
        assert!(pr.labels.is_empty());
        assert!(pr.assignees.is_empty());

        let pr_no_keys: PullRequest = vcs_cli_support::json::from_json(
            BINARY,
            r#"{"number": 14, "title": "t", "state": "OPEN",
                "headRefName": "h", "baseRefName": "main", "url": "u"}"#,
        )
        .expect("PR without labels/assignees keys");
        assert!(pr_no_keys.labels.is_empty());
        assert!(pr_no_keys.assignees.is_empty());
    }

    // Positive case: gh's `--json author,createdAt,updatedAt,milestone` shape
    // (`{"login": …}`/`{"title": …}` nested objects) flattens to plain strings.
    #[test]
    fn pr_parses_author_timestamps_and_milestone() {
        let json = r#"{"number": 12, "title": "Add feature", "state": "OPEN", "isDraft": false,
            "headRefName": "feat/x", "baseRefName": "main", "url": "https://gh/pr/12",
            "author": {"login": "octocat", "id": 1},
            "createdAt": "2026-07-01T00:00:00Z", "updatedAt": "2026-07-02T00:00:00Z",
            "milestone": {"title": "v1.0"}}"#;
        let pr: PullRequest = vcs_cli_support::json::from_json(BINARY, json)
            .expect("parse pr with author/timestamps/milestone");
        assert_eq!(pr.author, "octocat");
        assert_eq!(pr.created_at, "2026-07-01T00:00:00Z");
        assert_eq!(pr.updated_at, "2026-07-02T00:00:00Z");
        assert_eq!(pr.milestone.as_deref(), Some("v1.0"));
    }

    // Negative case: a `null` author (deleted account) flattens to an empty
    // login, and a `null` milestone (none attached) flattens to `None` — neither
    // fails the parse.
    #[test]
    fn pr_null_author_and_milestone_parse_tolerantly() {
        let json = r#"{"number": 13, "title": "t", "state": "OPEN", "isDraft": false,
            "headRefName": "h", "baseRefName": "main", "url": "u",
            "author": null, "milestone": null}"#;
        let pr: PullRequest =
            vcs_cli_support::json::from_json(BINARY, json).expect("PR with null author/milestone");
        assert_eq!(pr.author, "", "deleted account → empty login");
        assert_eq!(pr.milestone, None, "no milestone attached → None");

        // Absent keys entirely (an older canned fixture) default the same way.
        let pr_no_keys: PullRequest = vcs_cli_support::json::from_json(
            BINARY,
            r#"{"number": 14, "title": "t", "state": "OPEN",
                "headRefName": "h", "baseRefName": "main", "url": "u"}"#,
        )
        .expect("PR without author/timestamps/milestone keys");
        assert_eq!(pr_no_keys.author, "");
        assert_eq!(pr_no_keys.created_at, "");
        assert_eq!(pr_no_keys.updated_at, "");
        assert_eq!(pr_no_keys.milestone, None);
    }

    // `#[serde(default)]` robustness: a payload that omits `isDraft` deserializes
    // to `false` rather than failing the whole parse. (When we request `--json
    // isDraft`, gh emits the key or hard-errors on an unknown field — it never
    // silently omits it — so this guards our own tolerance, not a real gh quirk.)
    #[test]
    fn pr_without_is_draft_defaults_false() {
        let pr: PullRequest = vcs_cli_support::json::from_json(
            BINARY,
            r#"{"number": 4, "title": "t", "state": "OPEN",
                "headRefName": "h", "baseRefName": "main", "url": "u"}"#,
        )
        .expect("PR without isDraft");
        assert!(!pr.is_draft);
    }

    #[test]
    fn parses_issue_list() {
        let json = r#"[{"number": 3, "title": "Docs", "state": "OPEN"}]"#;
        let issues: Vec<Issue> =
            vcs_cli_support::json::from_json(BINARY, json).expect("parse issues");
        assert_eq!(issues[0].number, 3);
    }

    // Positive case for issues, mirroring `pr_parses_author_timestamps_and_milestone`.
    #[test]
    fn issue_parses_author_timestamps_and_milestone() {
        let json = r#"{"number": 3, "title": "Docs", "state": "OPEN",
            "author": {"login": "andyfeller"},
            "createdAt": "2026-07-01T00:00:00Z", "updatedAt": "2026-07-02T00:00:00Z",
            "milestone": {"title": "v1.0"}}"#;
        let issue: Issue = vcs_cli_support::json::from_json(BINARY, json)
            .expect("parse issue with author/timestamps/milestone");
        assert_eq!(issue.author, "andyfeller");
        assert_eq!(issue.created_at, "2026-07-01T00:00:00Z");
        assert_eq!(issue.updated_at, "2026-07-02T00:00:00Z");
        assert_eq!(issue.milestone.as_deref(), Some("v1.0"));
    }

    // Negative case for issues: a `null` author/milestone parses tolerantly.
    #[test]
    fn issue_null_author_and_milestone_parse_tolerantly() {
        let json = r#"{"number": 4, "title": "t", "state": "OPEN",
            "author": null, "milestone": null}"#;
        let issue: Issue = vcs_cli_support::json::from_json(BINARY, json)
            .expect("issue with null author/milestone");
        assert_eq!(issue.author, "");
        assert_eq!(issue.milestone, None);
    }

    // gh emits a *present* `null` (not an absent key) for some optional strings —
    // notably `headRefName`/`baseRefName` on a PR whose head branch was deleted, and
    // a null `body`. `#[serde(default)]` alone rejects a present null; `null_to_empty`
    // must turn it into an empty string rather than failing the whole parse.
    #[test]
    fn null_optional_fields_parse_to_empty() {
        let pr: PullRequest = vcs_cli_support::json::from_json(
            BINARY,
            r#"{"number": 1, "title": "t", "state": "CLOSED",
                "headRefName": null, "baseRefName": null, "url": null}"#,
        )
        .expect("PR with null head/base/url (deleted-branch PR)");
        assert_eq!(pr.head_ref_name, "");
        assert_eq!(pr.base_ref_name, "");
        assert_eq!(pr.url, "");

        let issue: Issue = vcs_cli_support::json::from_json(
            BINARY,
            r#"{"number": 2, "title": "t", "state": "OPEN", "body": null, "url": null}"#,
        )
        .expect("issue with null body/url");
        assert_eq!(issue.body, "");
        assert_eq!(issue.url, "");

        let release: Release = vcs_cli_support::json::from_json(
            BINARY,
            r#"{"tagName": "v1", "name": null, "body": null, "url": null, "publishedAt": null,
                "author": {}}"#,
        )
        .expect("release with null name/body/url/publishedAt/author");
        assert_eq!(release.name, "");
        // `body`/`url` are `Option`: a present `null` reads as `None`, not "".
        assert_eq!(release.body, None);
        assert_eq!(release.url, None);
        assert_eq!(
            release.author,
            Some("".to_string()),
            "deleted account → empty login"
        );
    }

    #[test]
    fn parses_repo_flattening_nested_objects() {
        let json = r#"{
            "name": "vcs-toolkit-rs",
            "owner": {"login": "ZelAnton"},
            "description": null,
            "url": "https://gh/repo",
            "isPrivate": false,
            "defaultBranchRef": {"name": "main"}
        }"#;
        let repo = parse_repo(json).expect("parse repo");
        assert_eq!(repo.name, "vcs-toolkit-rs");
        assert_eq!(repo.owner, "ZelAnton");
        assert_eq!(repo.description, None);
        assert_eq!(repo.default_branch, "main");
        assert!(!repo.is_private);
    }

    #[test]
    fn empty_repo_has_blank_default_branch() {
        let json = r#"{"name":"e","owner":{"login":"o"},"url":"u","isPrivate":true,"defaultBranchRef":null}"#;
        let repo = parse_repo(json).expect("parse repo");
        assert_eq!(repo.default_branch, "");
        assert!(repo.is_private);
    }

    #[test]
    fn malformed_json_is_a_parse_error() {
        match vcs_cli_support::json::from_json::<Vec<Issue>>(BINARY, "not json").unwrap_err() {
            Error::Parse { .. } => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    // gh reports `"conclusion": ""` (an empty string, NOT null) while a run is
    // in progress — the DTO must accept that shape, not demand an Option.
    #[test]
    fn parses_run_list_with_blank_in_progress_conclusion() {
        let json = r#"[
            {"databaseId": 27023111945, "name": "CI", "displayTitle": "fix: x",
             "status": "in_progress", "conclusion": "", "workflowName": "CI",
             "headBranch": "main", "event": "push",
             "url": "https://gh/runs/27023111945",
             "createdAt": "2026-06-05T10:00:00Z"}
        ]"#;
        let runs: Vec<WorkflowRun> =
            vcs_cli_support::json::from_json(BINARY, json).expect("parse runs");
        assert_eq!(runs[0].database_id, 27023111945);
        assert_eq!(runs[0].status, "in_progress");
        assert_eq!(runs[0].conclusion, "");
        assert_eq!(runs[0].workflow_name, "CI");
    }

    #[test]
    fn parses_check_runs_across_buckets() {
        let json = r#"[
            {"name": "build", "state": "SUCCESS", "bucket": "pass",
             "workflow": "CI", "link": "https://gh/c/1",
             "startedAt": "2026-06-05T10:00:00Z", "completedAt": "2026-06-05T10:05:00Z"},
            {"name": "lint", "state": "FAILURE", "bucket": "fail",
             "workflow": "CI", "link": "", "startedAt": "", "completedAt": ""},
            {"name": "deploy", "state": "IN_PROGRESS", "bucket": "pending",
             "workflow": "CD", "link": "", "startedAt": "", "completedAt": ""},
            {"name": "docs", "state": "SKIPPED", "bucket": "skipping",
             "workflow": "", "link": "", "startedAt": "", "completedAt": ""},
            {"name": "bench", "state": "CANCELLED", "bucket": "cancel",
             "workflow": "", "link": "", "startedAt": "", "completedAt": ""}
        ]"#;
        let checks: Vec<CheckRun> =
            vcs_cli_support::json::from_json(BINARY, json).expect("parse checks");
        let buckets: Vec<CheckBucket> = checks.iter().map(|c| c.bucket).collect();
        assert_eq!(
            buckets,
            [
                CheckBucket::Pass,
                CheckBucket::Fail,
                CheckBucket::Pending,
                CheckBucket::Skipping,
                CheckBucket::Cancel,
            ]
        );
        // An unrecognised bucket deserialises to the forward-compatible catch-all.
        let exotic: CheckRun =
            serde_json::from_str(r#"{"name":"x","bucket":"teleport"}"#).expect("parse");
        assert_eq!(exotic.bucket, CheckBucket::Unknown);
        assert_eq!(checks[0].name, "build");
    }

    // `release list` carries isLatest; `release view` does NOT have that field
    // (it must default to false) but fills body/url.
    #[test]
    fn parses_release_list_and_view_shapes() {
        let list = r#"[
            {"tagName": "vcs-git-v0.4.0", "name": "vcs-git v0.4.0",
             "isLatest": true, "isDraft": false, "isPrerelease": false,
             "publishedAt": "2026-06-04T12:00:00Z"}
        ]"#;
        let releases: Vec<Release> =
            vcs_cli_support::json::from_json(BINARY, list).expect("parse list");
        assert!(releases[0].is_latest);
        assert_eq!(releases[0].tag_name, "vcs-git-v0.4.0");
        assert_eq!(
            releases[0].body, None,
            "list doesn't request the body → None"
        );
        assert_eq!(releases[0].url, None, "list doesn't request the url → None");
        assert_eq!(releases[0].author, None);

        let view = r#"{"tagName": "vcs-git-v0.4.0", "name": "vcs-git v0.4.0",
            "body": "Added\n- stuff", "url": "https://gh/releases/1",
            "publishedAt": "2026-06-04T12:00:00Z",
            "isDraft": false, "isPrerelease": false, "author": {"login": "ZelAnton"}}"#;
        let release: Release = vcs_cli_support::json::from_json(BINARY, view).expect("parse view");
        assert!(!release.is_latest, "view has no isLatest → default false");
        assert_eq!(release.body.as_deref(), Some("Added\n- stuff"));
        assert_eq!(release.url.as_deref(), Some("https://gh/releases/1"));
        assert_eq!(release.author, Some("ZelAnton".to_string()));
    }

    #[test]
    fn parses_feedback_flattening_nested_authors() {
        let json = r#"{
            "reviews": [
                {"author": {"login": "steiza"}, "state": "APPROVED",
                 "body": "LGTM", "submittedAt": "2026-06-01T00:00:00Z"},
                {"author": null, "state": "COMMENTED", "body": "ghost",
                 "submittedAt": ""}
            ],
            "comments": [
                {"author": {"login": "andyfeller"}, "body": "nice",
                 "url": "https://gh/c/9", "createdAt": "2026-06-02T00:00:00Z"}
            ]
        }"#;
        let feedback = parse_feedback(json).expect("parse feedback");
        assert_eq!(feedback.reviews.len(), 2);
        assert_eq!(feedback.reviews[0].author, "steiza");
        assert_eq!(feedback.reviews[0].state, "APPROVED");
        assert_eq!(feedback.reviews[1].author, "", "deleted account → empty");
        assert_eq!(feedback.comments[0].author, "andyfeller");
        assert_eq!(feedback.comments[0].url, "https://gh/c/9");
    }

    // The Issue extension must stay backward-compatible with `issue list`
    // JSON (no body/url requested) while `issue view` fills both.
    #[test]
    fn issue_parses_with_and_without_view_fields() {
        let list = r#"[{"number": 3, "title": "Docs", "state": "OPEN"}]"#;
        let issues: Vec<Issue> =
            vcs_cli_support::json::from_json(BINARY, list).expect("parse list");
        assert_eq!(issues[0].body, "");
        assert_eq!(issues[0].url, "");

        let view = r#"{"number": 3, "title": "Docs", "state": "OPEN",
            "body": "Write them.", "url": "https://gh/issues/3"}"#;
        let issue: Issue = vcs_cli_support::json::from_json(BINARY, view).expect("parse view");
        assert_eq!(issue.body, "Write them.");
        assert_eq!(issue.url, "https://gh/issues/3");
        assert!(issue.labels.is_empty());
        assert!(issue.assignees.is_empty());
    }

    // Positive case for issues, mirroring `pr_parses_labels_and_assignees`.
    #[test]
    fn issue_parses_labels_and_assignees() {
        let json = r#"{"number": 3, "title": "Docs", "state": "OPEN",
            "body": "b", "url": "https://gh/issues/3",
            "labels": [{"name": "docs"}, {"name": "good-first-issue"}],
            "assignees": [{"login": "andyfeller"}]}"#;
        let issue: Issue = vcs_cli_support::json::from_json(BINARY, json)
            .expect("parse issue with labels/assignees");
        assert_eq!(
            issue.labels,
            vec!["docs".to_string(), "good-first-issue".to_string()]
        );
        assert_eq!(issue.assignees, vec!["andyfeller".to_string()]);
    }

    // Negative case for issues: empty arrays parse to empty `Vec`s, not an error.
    #[test]
    fn issue_without_labels_or_assignees_parses_to_empty_vecs() {
        let json = r#"{"number": 4, "title": "t", "state": "CLOSED",
            "labels": [], "assignees": []}"#;
        let issue: Issue = vcs_cli_support::json::from_json(BINARY, json)
            .expect("issue with empty labels/assignees");
        assert!(issue.labels.is_empty());
        assert!(issue.assignees.is_empty());
    }
}
