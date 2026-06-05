//! Typed results from `gh … --json` and the deserialization helpers. Parsing is
//! pure, so these tests are hermetic and run on CI.

use processkit::{Error, Result};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::BINARY;

/// A pull request (`gh pr list/view --json number,title,state,headRefName,baseRefName,url`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct PullRequest {
    /// PR number.
    pub number: u64,
    /// PR title.
    pub title: String,
    /// State, e.g. `"OPEN"`, `"MERGED"`, `"CLOSED"`.
    pub state: String,
    /// Source (head) branch name.
    #[serde(rename = "headRefName", default)]
    pub head_ref_name: String,
    /// Target (base) branch name.
    #[serde(rename = "baseRefName", default)]
    pub base_ref_name: String,
    /// Web URL.
    #[serde(default)]
    pub url: String,
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
    /// Issue body (markdown); empty from `issue_list`, which doesn't fetch it.
    #[serde(default)]
    pub body: String,
    /// Web URL; empty from `issue_list`, which doesn't fetch it.
    #[serde(default)]
    pub url: String,
}

/// A GitHub Actions workflow run (`gh run list/view --json …`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct WorkflowRun {
    /// The run id (`databaseId`) — the `<run-id>` other `gh run` commands take.
    #[serde(rename = "databaseId")]
    pub database_id: u64,
    /// Workflow name as shown in the runs list.
    #[serde(default)]
    pub name: String,
    /// The run's display title (usually the commit subject).
    #[serde(rename = "displayTitle", default)]
    pub display_title: String,
    /// Lifecycle status, e.g. `"queued"`, `"in_progress"`, `"completed"`.
    #[serde(default)]
    pub status: String,
    /// Outcome, e.g. `"success"`, `"failure"`, `"cancelled"`, `"skipped"` —
    /// gh reports an **empty string** until the run completes (not `null`).
    #[serde(default)]
    pub conclusion: String,
    /// Name of the workflow that produced the run.
    #[serde(rename = "workflowName", default)]
    pub workflow_name: String,
    /// Branch the run was triggered for.
    #[serde(rename = "headBranch", default)]
    pub head_branch: String,
    /// Triggering event, e.g. `"push"`, `"workflow_dispatch"`.
    #[serde(default)]
    pub event: String,
    /// Web URL.
    #[serde(default)]
    pub url: String,
    /// Creation timestamp (ISO 8601).
    #[serde(rename = "createdAt", default)]
    pub created_at: String,
}

/// One check on a PR (`gh pr checks --json …`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct CheckRun {
    /// Check name.
    pub name: String,
    /// Raw state, e.g. `"SUCCESS"`, `"FAILURE"`, `"IN_PROGRESS"`.
    #[serde(default)]
    pub state: String,
    /// gh's categorisation of `state`: one of `"pass"`, `"fail"`, `"pending"`,
    /// `"skipping"`, `"cancel"` — the field to branch on.
    #[serde(default)]
    pub bucket: String,
    /// Workflow the check belongs to (empty for non-Actions checks).
    #[serde(default)]
    pub workflow: String,
    /// Web link to the check's details.
    #[serde(default)]
    pub link: String,
    /// Start timestamp (ISO 8601), empty until started.
    #[serde(rename = "startedAt", default)]
    pub started_at: String,
    /// Completion timestamp (ISO 8601), empty until completed.
    #[serde(rename = "completedAt", default)]
    pub completed_at: String,
}

/// A release (`gh release list/view --json …`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Release {
    /// The release's tag.
    #[serde(rename = "tagName")]
    pub tag_name: String,
    /// Release title (may be empty).
    #[serde(default)]
    pub name: String,
    /// Release notes (markdown); empty from `release_list`, which doesn't
    /// fetch it.
    #[serde(default)]
    pub body: String,
    /// Web URL; empty from `release_list`, which doesn't fetch it.
    #[serde(default)]
    pub url: String,
    /// Publication timestamp (ISO 8601); empty for a draft.
    #[serde(rename = "publishedAt", default)]
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
pub struct Repo {
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
// flatten into the public `Repo`.
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

/// Deserialize `gh --json` output into `T`, mapping parse errors to
/// [`Error::Parse`].
pub(crate) fn from_json<T: DeserializeOwned>(json: &str) -> Result<T> {
    serde_json::from_str(json).map_err(|e| Error::Parse {
        program: BINARY.to_string(),
        message: e.to_string(),
    })
}

/// Parse `gh repo view --json …` output, flattening the nested objects.
pub(crate) fn parse_repo(json: &str) -> Result<Repo> {
    let raw: RepoJson = from_json(json)?;
    Ok(Repo {
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

#[derive(Deserialize)]
struct ReviewJson {
    #[serde(default)]
    author: Option<AuthorJson>,
    #[serde(default)]
    state: String,
    #[serde(default)]
    body: String,
    #[serde(rename = "submittedAt", default)]
    submitted_at: String,
}

#[derive(Deserialize)]
struct CommentJson {
    #[serde(default)]
    author: Option<AuthorJson>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    url: String,
    #[serde(rename = "createdAt", default)]
    created_at: String,
}

#[derive(Deserialize)]
struct AuthorJson {
    #[serde(default)]
    login: String,
}

/// Parse `gh pr view --json reviews,comments` output, flattening the nested
/// author objects (a deleted account's `null` author becomes an empty login).
pub(crate) fn parse_feedback(json: &str) -> Result<PrFeedback> {
    let raw: FeedbackJson = from_json(json)?;
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

    #[test]
    fn parses_pr_list() {
        let json = r#"[
            {"number": 12, "title": "Add feature", "state": "OPEN",
             "headRefName": "feat/x", "baseRefName": "main", "url": "https://gh/pr/12"}
        ]"#;
        let prs: Vec<PullRequest> = from_json(json).expect("parse prs");
        assert_eq!(prs.len(), 1);
        assert_eq!(
            prs[0],
            PullRequest {
                number: 12,
                title: "Add feature".into(),
                state: "OPEN".into(),
                head_ref_name: "feat/x".into(),
                base_ref_name: "main".into(),
                url: "https://gh/pr/12".into(),
            }
        );
    }

    #[test]
    fn parses_issue_list() {
        let json = r#"[{"number": 3, "title": "Docs", "state": "OPEN"}]"#;
        let issues: Vec<Issue> = from_json(json).expect("parse issues");
        assert_eq!(issues[0].number, 3);
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
        match from_json::<Vec<Issue>>("not json").unwrap_err() {
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
        let runs: Vec<WorkflowRun> = from_json(json).expect("parse runs");
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
        let checks: Vec<CheckRun> = from_json(json).expect("parse checks");
        let buckets: Vec<&str> = checks.iter().map(|c| c.bucket.as_str()).collect();
        assert_eq!(buckets, ["pass", "fail", "pending", "skipping", "cancel"]);
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
        let releases: Vec<Release> = from_json(list).expect("parse list");
        assert!(releases[0].is_latest);
        assert_eq!(releases[0].tag_name, "vcs-git-v0.4.0");
        assert_eq!(releases[0].body, "", "list doesn't fetch the body");

        let view = r#"{"tagName": "vcs-git-v0.4.0", "name": "vcs-git v0.4.0",
            "body": "Added\n- stuff", "url": "https://gh/releases/1",
            "publishedAt": "2026-06-04T12:00:00Z",
            "isDraft": false, "isPrerelease": false}"#;
        let release: Release = from_json(view).expect("parse view");
        assert!(!release.is_latest, "view has no isLatest → default false");
        assert_eq!(release.body, "Added\n- stuff");
        assert_eq!(release.url, "https://gh/releases/1");
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
        let issues: Vec<Issue> = from_json(list).expect("parse list");
        assert_eq!(issues[0].body, "");
        assert_eq!(issues[0].url, "");

        let view = r#"{"number": 3, "title": "Docs", "state": "OPEN",
            "body": "Write them.", "url": "https://gh/issues/3"}"#;
        let issue: Issue = from_json(view).expect("parse view");
        assert_eq!(issue.body, "Write them.");
        assert_eq!(issue.url, "https://gh/issues/3");
    }
}
