//! Typed results from `glab … --output json` and the deserialization helpers.
//! Parsing is pure (over GitLab's REST JSON, which `glab` emits verbatim), so
//! these tests are hermetic and run on CI.

use processkit::Result;
use serde::Deserialize;

use crate::BINARY;

/// Parse `glab --version` output (`glab 1.36.0` / `glab version 1.36.0 (…)`) into
/// the shared [`vcs_diff::Version`]: the first dotted-numeric token wins, so any
/// build/commit trailer is ignored. `None` when the banner carries no version
/// token. Reuses the same tolerant parser `vcs-git`/`vcs-jj` gate on, so the three
/// CLIs share one version-parsing contract.
pub(crate) fn parse_glab_version(raw: &str) -> Option<vcs_diff::Version> {
    vcs_diff::parse_dotted_version(raw)
}

/// A merge request (`glab mr list/view --output json`). The fields are GitLab's
/// REST `MergeRequest` object, which `glab` passes through unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct MergeRequest {
    /// The **project-scoped** id (`iid`) — the `<id>` other `glab mr` commands
    /// take. (GitLab's global `id` is deliberately not surfaced.)
    pub iid: u64,
    /// MR title.
    pub title: String,
    /// State, e.g. `"opened"`, `"closed"`, `"merged"`, `"locked"` (GitLab's
    /// lower-case spelling — note it is `"opened"`, not `"open"`).
    pub state: String,
    /// Source (head) branch name.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub source_branch: String,
    /// Target (base) branch name.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub target_branch: String,
    /// Web URL.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub web_url: String,
    /// Whether the MR is a draft (GitLab's `draft`; the deprecated
    /// `work_in_progress` is not read).
    #[serde(default)]
    pub draft: bool,
    /// Labels attached to the MR. GitLab's REST API reports these as plain
    /// label-name strings (not objects), unlike GitHub's `[{"name": …}]`.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Usernames of assigned users, flattened from GitLab's REST `assignees`
    /// array of User objects (`[{"username": …}, ...]`) to plain usernames.
    #[serde(default, deserialize_with = "users_to_usernames")]
    pub assignees: Vec<String>,
    /// Author's username, flattened from GitLab's REST `author` User object
    /// (`{"username": …}`) to a plain string.
    #[serde(default, deserialize_with = "author_username")]
    pub author: String,
    /// Creation timestamp (RFC 3339) (GitLab REST `created_at`).
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub created_at: String,
    /// Last-update timestamp (RFC 3339) (GitLab REST `updated_at`).
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub updated_at: String,
    /// Milestone title, or `None` when no milestone is attached (GitLab REST
    /// `milestone.title`; `null` → `None`).
    #[serde(default, deserialize_with = "milestone_to_title")]
    pub milestone: Option<String>,
}

/// A project, returned as `RepoView` (`glab repo view --output json`) — the
/// fields are GitLab's REST `Project` object.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct RepoView {
    /// Project name (the last path segment's display name).
    pub name: String,
    /// Full namespace path, e.g. `"group/subgroup/repo"`.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub path_with_namespace: String,
    /// Default branch name (empty/null for an empty project).
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub default_branch: String,
    /// Web URL.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub web_url: String,
    /// Visibility, e.g. `"public"`, `"internal"`, `"private"`. `None` when glab
    /// omits the field — a consumer must treat an absent visibility as *unknown*,
    /// not as private (see [`ForgeRepo::private`](../../vcs_forge/struct.ForgeRepo.html)).
    #[serde(default)]
    pub visibility: Option<String>,
}

/// An issue (`glab issue list/view --output json`). The fields are GitLab's
/// REST `Issue` object, which `glab` passes through unchanged. Mirrors
/// [`MergeRequest`]'s shape (project-scoped `iid`, tolerant optional fields).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Issue {
    /// The **project-scoped** id (`iid`) — the `<id>` other `glab issue`
    /// commands take. (GitLab's global `id` is deliberately not surfaced.)
    /// Surfaced through the public field name `number` for cross-forge
    /// consistency with [`vcs-github`](https://crates.io/crates/vcs-github)'s
    /// `Issue`.
    #[serde(rename = "iid")]
    pub number: u64,
    /// Issue title.
    pub title: String,
    /// State, e.g. `"opened"`, `"closed"` (GitLab's lower-case spelling — note
    /// it is `"opened"`, not `"open"`).
    pub state: String,
    /// Issue body (GitLab's `description`, markdown). `glab issue list` does
    /// include it, but it can be absent/null, so it is tolerant.
    #[serde(
        rename = "description",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub body: String,
    /// Web URL.
    #[serde(
        rename = "web_url",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub url: String,
    /// Labels attached to the issue. GitLab's REST API reports these as plain
    /// label-name strings (not objects), unlike GitHub's `[{"name": …}]`.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Usernames of assigned users, flattened from GitLab's REST `assignees`
    /// array of User objects (`[{"username": …}, ...]`) to plain usernames.
    #[serde(default, deserialize_with = "users_to_usernames")]
    pub assignees: Vec<String>,
    /// Author's username, flattened from GitLab's REST `author` User object
    /// (`{"username": …}`) to a plain string.
    #[serde(default, deserialize_with = "author_username")]
    pub author: String,
    /// Creation timestamp (RFC 3339) (GitLab REST `created_at`).
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub created_at: String,
    /// Last-update timestamp (RFC 3339) (GitLab REST `updated_at`).
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub updated_at: String,
    /// Milestone title, or `None` when no milestone is attached (GitLab REST
    /// `milestone.title`; `null` → `None`).
    #[serde(default, deserialize_with = "milestone_to_title")]
    pub milestone: Option<String>,
}

// GitLab's REST `assignees` is an array of User objects (`{"username": …, "id":
// …, "name": …, ...}`), unlike `labels`, which is already a plain array of
// strings — flatten just the username. `Option<Vec<_>>` (not a bare `Vec<_>`)
// so a present JSON `null` degrades to an empty list rather than failing the
// whole parse, matching this file's other tolerant optional fields.
#[derive(Deserialize)]
struct UserJson {
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    username: String,
}

fn users_to_usernames<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<Vec<UserJson>>::deserialize(deserializer)?.unwrap_or_default();
    Ok(raw.into_iter().map(|u| u.username).collect())
}

// GitLab's REST `author` is a single User object (`{"username": …, "id": …,
// ...}`), unlike `assignees` (an array of the same shape) — flatten just the
// username. A present `null` (an anonymised/deleted account) degrades to an
// empty username rather than failing the whole parse.
fn author_username<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<UserJson>::deserialize(deserializer)?;
    Ok(raw.map(|u| u.username).unwrap_or_default())
}

// GitLab's REST `milestone` is a Milestone object (`{"title": …, "id": …,
// ...}`), `null` when none is attached.
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

/// A release (`glab release list/view --output json`) — GitLab's REST
/// `Release` object, which `glab` passes through unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Release {
    /// The Git tag the release is attached to (the `<tag>`
    /// [`release_view`](crate::GitLabApi::release_view) takes).
    pub tag_name: String,
    /// Release title (may be empty/absent/null — GitLab defaults it to the tag).
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub name: String,
    /// Web URL of the release page. GitLab carries it as `_links.self` (there
    /// is no top-level `web_url` on a release), so it is pulled off that nested
    /// object; empty when absent.
    #[serde(rename = "_links", default, deserialize_with = "self_link")]
    pub url: String,
    /// Publication timestamp (GitLab's `released_at`, ISO 8601); empty when
    /// absent/null (e.g. an upcoming/unpublished release).
    #[serde(
        rename = "released_at",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub published_at: String,
    /// Release notes (GitLab's `description`, markdown); empty when absent/null.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub description: String,
    /// Author's username, flattened from GitLab's REST `author` User object
    /// (`{"username": …}`) to a plain string.
    #[serde(default, deserialize_with = "author_username")]
    pub author: String,
}

/// Deserialize a `Release`'s `url` from GitLab's `_links.self`. The links object
/// can be absent or have a null/missing `self`; any of those yield an empty
/// string rather than erroring (matching the tolerant `#[serde(default)]` style).
fn self_link<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct Links {
        #[serde(rename = "self", default)]
        self_url: String,
    }
    let links = Option::<Links>::deserialize(deserializer)?;
    Ok(links.map(|l| l.self_url).unwrap_or_default())
}

/// The coarse CI/pipeline outcome for an MR (`glab mr view … --output json`'s
/// `head_pipeline.status`), bucketed into the four states a caller acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CiStatus {
    /// The pipeline succeeded (`success`).
    Passing,
    /// The pipeline failed or was canceled (`failed`/`canceled`).
    Failing,
    /// The pipeline is still going (`running`/`pending`/`created`/…) **or is
    /// blocked awaiting action** (`manual`/`scheduled`/`waiting_for_resource`).
    /// The blocked states are bucketed here conservatively ("not known to be
    /// done"), so a poller that loops until this is no longer `Pending` should
    /// bound its wait — a `manual` pipeline stays blocked until someone triggers
    /// it and would otherwise be polled forever.
    Pending,
    /// No pipeline ran (none attached, or `skipped`).
    None,
}

impl CiStatus {
    /// Bucket a raw GitLab pipeline `status` string. Unknown values — and the
    /// blocked-awaiting-action states `manual`/`scheduled` — read as
    /// [`Pending`](CiStatus::Pending) (conservative — "not known to be done";
    /// see the variant docs on bounding a poller's wait).
    pub(crate) fn from_gitlab(status: &str) -> Self {
        match status {
            "success" => CiStatus::Passing,
            "failed" | "canceled" | "cancelled" => CiStatus::Failing,
            "skipped" | "" => CiStatus::None,
            "running"
            | "pending"
            | "created"
            | "preparing"
            | "scheduled"
            | "waiting_for_resource"
            | "manual" => CiStatus::Pending,
            _ => CiStatus::Pending,
        }
    }
}

// The MR JSON carries the pipeline as a nested object; deserialize just the
// status off it. `head_pipeline` is the current one; `pipeline` is the older
// alias — accept either.
#[derive(Deserialize)]
struct MrPipelineJson {
    #[serde(default)]
    head_pipeline: Option<PipelineJson>,
    #[serde(default)]
    pipeline: Option<PipelineJson>,
}

#[derive(Deserialize)]
struct PipelineJson {
    #[serde(default)]
    status: String,
}

/// Parse the CI/pipeline status out of `glab mr view <id> --output json` —
/// `head_pipeline.status` (falling back to the deprecated `pipeline.status`);
/// no pipeline at all is [`CiStatus::None`].
pub(crate) fn parse_ci_status(json: &str) -> Result<CiStatus> {
    let raw: MrPipelineJson = vcs_cli_support::json::from_json(BINARY, json)?;
    let status = raw
        .head_pipeline
        .or(raw.pipeline)
        .map(|p| p.status)
        .unwrap_or_default();
    Ok(CiStatus::from_gitlab(&status))
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::Error;

    #[test]
    fn parses_mr_list() {
        let json = r#"[
            {"iid": 12, "title": "Add feature", "state": "opened",
             "source_branch": "feat/x", "target_branch": "main",
             "web_url": "https://gl/mr/12", "draft": false}
        ]"#;
        let mrs: Vec<MergeRequest> =
            vcs_cli_support::json::from_json(BINARY, json).expect("parse mrs");
        assert_eq!(mrs.len(), 1);
        assert_eq!(
            mrs[0],
            MergeRequest {
                iid: 12,
                title: "Add feature".into(),
                state: "opened".into(),
                source_branch: "feat/x".into(),
                target_branch: "main".into(),
                web_url: "https://gl/mr/12".into(),
                draft: false,
                labels: Vec::new(),
                assignees: Vec::new(),
                author: String::new(),
                created_at: String::new(),
                updated_at: String::new(),
                milestone: None,
            }
        );
    }

    // Positive case: GitLab's `labels` are already plain strings, and
    // `assignees` is an array of User objects flattened to plain usernames.
    #[test]
    fn mr_parses_labels_and_assignees() {
        let json = r#"{"iid": 12, "title": "Add feature", "state": "opened",
            "source_branch": "feat/x", "target_branch": "main",
            "web_url": "https://gl/mr/12", "draft": false,
            "labels": ["bug", "priority::1"],
            "assignees": [{"username": "steiza", "id": 1}, {"username": "andyfeller"}]}"#;
        let mr: MergeRequest =
            vcs_cli_support::json::from_json(BINARY, json).expect("parse mr with labels/assignees");
        assert_eq!(
            mr.labels,
            vec!["bug".to_string(), "priority::1".to_string()]
        );
        assert_eq!(
            mr.assignees,
            vec!["steiza".to_string(), "andyfeller".to_string()]
        );
    }

    // Negative case: empty arrays parse to empty `Vec`s, not an error, and an
    // absent key defaults the same way.
    #[test]
    fn mr_without_labels_or_assignees_parses_to_empty_vecs() {
        let json = r#"{"iid": 13, "title": "t", "state": "opened",
            "labels": [], "assignees": []}"#;
        let mr: MergeRequest =
            vcs_cli_support::json::from_json(BINARY, json).expect("mr with empty labels/assignees");
        assert!(mr.labels.is_empty());
        assert!(mr.assignees.is_empty());

        let mr_no_keys: MergeRequest = vcs_cli_support::json::from_json(
            BINARY,
            r#"{"iid": 14, "title": "t", "state": "opened"}"#,
        )
        .expect("mr without labels/assignees keys");
        assert!(mr_no_keys.labels.is_empty());
        assert!(mr_no_keys.assignees.is_empty());
    }

    // Positive case: GitLab's `author` is a single User object flattened to a
    // plain username; `milestone` is a Milestone object flattened to its title.
    #[test]
    fn mr_parses_author_timestamps_and_milestone() {
        let json = r#"{"iid": 12, "title": "Add feature", "state": "opened",
            "author": {"username": "steiza", "id": 1},
            "created_at": "2026-07-01T00:00:00Z", "updated_at": "2026-07-02T00:00:00Z",
            "milestone": {"title": "v1.0"}}"#;
        let mr: MergeRequest = vcs_cli_support::json::from_json(BINARY, json)
            .expect("parse mr with author/timestamps/milestone");
        assert_eq!(mr.author, "steiza");
        assert_eq!(mr.created_at, "2026-07-01T00:00:00Z");
        assert_eq!(mr.updated_at, "2026-07-02T00:00:00Z");
        assert_eq!(mr.milestone.as_deref(), Some("v1.0"));
    }

    // Negative case: a `null` author (anonymised account) flattens to an empty
    // username, and a `null` milestone (none attached) flattens to `None`.
    #[test]
    fn mr_null_author_and_milestone_parse_tolerantly() {
        let json = r#"{"iid": 13, "title": "t", "state": "opened",
            "author": null, "milestone": null}"#;
        let mr: MergeRequest =
            vcs_cli_support::json::from_json(BINARY, json).expect("mr with null author/milestone");
        assert_eq!(mr.author, "", "anonymised account → empty username");
        assert_eq!(mr.milestone, None, "no milestone attached → None");

        let mr_no_keys: MergeRequest = vcs_cli_support::json::from_json(
            BINARY,
            r#"{"iid": 14, "title": "t", "state": "opened"}"#,
        )
        .expect("mr without author/timestamps/milestone keys");
        assert_eq!(mr_no_keys.author, "");
        assert_eq!(mr_no_keys.created_at, "");
        assert_eq!(mr_no_keys.updated_at, "");
        assert_eq!(mr_no_keys.milestone, None);
    }

    // glab/GitLab omit fields that don't apply; the DTO must tolerate a minimal
    // object (only the required `iid`/`title`/`state`).
    #[test]
    fn mr_tolerates_missing_optional_fields() {
        let json = r#"{"iid": 5, "title": "wip", "state": "opened", "draft": true}"#;
        let mr: MergeRequest = vcs_cli_support::json::from_json(BINARY, json).expect("parse mr");
        assert_eq!(mr.source_branch, "");
        assert_eq!(mr.web_url, "");
        assert!(mr.draft);
    }

    #[test]
    fn parses_issue_list() {
        // Field shapes from the GitLab Issues API: iid/title/state/description/web_url.
        let json = r#"[
            {"iid": 1, "title": "Fix bug", "state": "opened",
             "description": "the body", "web_url": "https://gl/i/1"}
        ]"#;
        let issues: Vec<Issue> =
            vcs_cli_support::json::from_json(BINARY, json).expect("parse issues");
        assert_eq!(issues.len(), 1);
        assert_eq!(
            issues[0],
            Issue {
                number: 1,
                title: "Fix bug".into(),
                state: "opened".into(),
                body: "the body".into(),
                url: "https://gl/i/1".into(),
                labels: Vec::new(),
                assignees: Vec::new(),
                author: String::new(),
                created_at: String::new(),
                updated_at: String::new(),
                milestone: None,
            }
        );
    }

    // Positive case for issues, mirroring `mr_parses_labels_and_assignees`.
    #[test]
    fn issue_parses_labels_and_assignees() {
        let json = r#"{"iid": 1, "title": "Fix bug", "state": "opened",
            "description": "the body", "web_url": "https://gl/i/1",
            "labels": ["bug", "confirmed"],
            "assignees": [{"username": "steiza"}]}"#;
        let issue: Issue = vcs_cli_support::json::from_json(BINARY, json)
            .expect("parse issue with labels/assignees");
        assert_eq!(
            issue.labels,
            vec!["bug".to_string(), "confirmed".to_string()]
        );
        assert_eq!(issue.assignees, vec!["steiza".to_string()]);
    }

    // Negative case for issues: empty arrays parse to empty `Vec`s, not an error.
    #[test]
    fn issue_without_labels_or_assignees_parses_to_empty_vecs() {
        let json = r#"{"iid": 2, "title": "t", "state": "closed",
            "labels": [], "assignees": []}"#;
        let issue: Issue = vcs_cli_support::json::from_json(BINARY, json)
            .expect("issue with empty labels/assignees");
        assert!(issue.labels.is_empty());
        assert!(issue.assignees.is_empty());
    }

    // Positive case for issues, mirroring `mr_parses_author_timestamps_and_milestone`.
    #[test]
    fn issue_parses_author_timestamps_and_milestone() {
        let json = r#"{"iid": 1, "title": "Fix bug", "state": "opened",
            "author": {"username": "steiza"},
            "created_at": "2026-07-01T00:00:00Z", "updated_at": "2026-07-02T00:00:00Z",
            "milestone": {"title": "v1.0"}}"#;
        let issue: Issue = vcs_cli_support::json::from_json(BINARY, json)
            .expect("parse issue with author/timestamps/milestone");
        assert_eq!(issue.author, "steiza");
        assert_eq!(issue.created_at, "2026-07-01T00:00:00Z");
        assert_eq!(issue.updated_at, "2026-07-02T00:00:00Z");
        assert_eq!(issue.milestone.as_deref(), Some("v1.0"));
    }

    // Negative case for issues: a `null` author/milestone parses tolerantly.
    #[test]
    fn issue_null_author_and_milestone_parse_tolerantly() {
        let json = r#"{"iid": 2, "title": "t", "state": "closed",
            "author": null, "milestone": null}"#;
        let issue: Issue = vcs_cli_support::json::from_json(BINARY, json)
            .expect("issue with null author/milestone");
        assert_eq!(issue.author, "");
        assert_eq!(issue.milestone, None);
    }

    // glab/GitLab can omit description/web_url; the DTO must tolerate a minimal
    // object (only the required `iid`/`title`/`state`).
    #[test]
    fn issue_tolerates_missing_optional_fields() {
        let json = r#"{"iid": 9, "title": "wip", "state": "closed"}"#;
        let issue: Issue = vcs_cli_support::json::from_json(BINARY, json).expect("parse issue");
        assert_eq!(issue.body, "");
        assert_eq!(issue.url, "");
    }

    // GitLab's REST API sends a *present* `null` (not an absent key) for an empty
    // optional field — an issue/MR with no `description`, a project with no
    // `default_branch`. `#[serde(default)]` alone rejects a present null; the
    // `null_to_empty` deserializer must turn it into an empty string instead of
    // failing the whole parse. These are the single most common real shapes.
    #[test]
    fn null_optional_fields_parse_to_empty() {
        let issue: Issue = vcs_cli_support::json::from_json(
            BINARY,
            r#"{"iid": 9, "title": "t", "state": "closed", "description": null, "web_url": null}"#,
        )
        .expect("issue with null description/web_url");
        assert_eq!(issue.body, "");
        assert_eq!(issue.url, "");

        let mr: MergeRequest = vcs_cli_support::json::from_json(
            BINARY,
            r#"{"iid": 3, "title": "t", "state": "opened",
                "source_branch": null, "target_branch": null, "web_url": null}"#,
        )
        .expect("mr with null branches/url");
        assert_eq!(mr.source_branch, "");
        assert_eq!(mr.target_branch, "");

        let project: RepoView = vcs_cli_support::json::from_json(BINARY,
            r#"{"name": "p", "path_with_namespace": null, "default_branch": null, "web_url": null}"#,
        )
        .expect("project with null default_branch");
        assert_eq!(project.default_branch, "");

        let release: Release = vcs_cli_support::json::from_json(
            BINARY,
            r#"{"tag_name": "v1", "name": null, "released_at": null, "description": null,
                "author": null}"#,
        )
        .expect("release with null name/date/description/author");
        assert_eq!(release.name, "");
        assert_eq!(release.published_at, "");
        assert_eq!(release.description, "");
        assert_eq!(release.author, "", "anonymised account → empty username");
    }

    #[test]
    fn parses_release_view() {
        // Field shapes from the GitLab Releases API: tag_name/name/released_at,
        // and the URL nested under `_links.self` (no top-level web_url).
        let json = r#"{
            "tag_name": "v1.0", "name": "Release 1.0",
            "released_at": "2026-01-02T03:04:05.000Z",
            "description": "the notes",
            "_links": {"self": "https://gl/-/releases/v1.0"},
            "author": {"username": "zelanton"}
        }"#;
        let rel: Release = vcs_cli_support::json::from_json(BINARY, json).expect("parse release");
        assert_eq!(
            rel,
            Release {
                tag_name: "v1.0".into(),
                name: "Release 1.0".into(),
                url: "https://gl/-/releases/v1.0".into(),
                published_at: "2026-01-02T03:04:05.000Z".into(),
                description: "the notes".into(),
                author: "zelanton".into(),
            }
        );
    }

    // A release with no `_links` and no `released_at` (e.g. an upcoming release)
    // must deserialize with empty url/published_at, not error.
    #[test]
    fn release_tolerates_missing_links_and_date() {
        let json = r#"{"tag_name": "v2.0"}"#;
        let rel: Release = vcs_cli_support::json::from_json(BINARY, json).expect("parse release");
        assert_eq!(rel.name, "");
        assert_eq!(rel.url, "");
        assert_eq!(rel.published_at, "");
    }

    #[test]
    fn parses_project_view() {
        let json = r#"{
            "name": "cli", "path_with_namespace": "gitlab-org/cli",
            "default_branch": "main", "web_url": "https://gl/p",
            "visibility": "public"
        }"#;
        let p: RepoView = vcs_cli_support::json::from_json(BINARY, json).expect("parse project");
        assert_eq!(p.name, "cli");
        assert_eq!(p.path_with_namespace, "gitlab-org/cli");
        assert_eq!(p.default_branch, "main");
        assert_eq!(p.visibility.as_deref(), Some("public"));
    }

    // glab omits `visibility` for some responses; it must deserialize to `None`
    // (unknown), never a default that a consumer could mistake for private.
    #[test]
    fn project_tolerates_missing_visibility() {
        let json = r#"{"name":"cli","path_with_namespace":"o/cli","default_branch":"main"}"#;
        let p: RepoView = vcs_cli_support::json::from_json(BINARY, json).expect("parse project");
        assert_eq!(p.visibility, None);
    }

    #[test]
    fn malformed_json_is_a_parse_error() {
        match vcs_cli_support::json::from_json::<Vec<MergeRequest>>(BINARY, "not json").unwrap_err()
        {
            Error::Parse { .. } => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn ci_status_buckets_pipeline_states() {
        assert_eq!(CiStatus::from_gitlab("success"), CiStatus::Passing);
        assert_eq!(CiStatus::from_gitlab("failed"), CiStatus::Failing);
        assert_eq!(CiStatus::from_gitlab("canceled"), CiStatus::Failing);
        assert_eq!(CiStatus::from_gitlab("running"), CiStatus::Pending);
        assert_eq!(CiStatus::from_gitlab("manual"), CiStatus::Pending);
        assert_eq!(CiStatus::from_gitlab("skipped"), CiStatus::None);
        assert_eq!(CiStatus::from_gitlab(""), CiStatus::None);
        // Unknown future states read as Pending, not a panic.
        assert_eq!(CiStatus::from_gitlab("brand_new"), CiStatus::Pending);
    }

    #[test]
    fn parse_ci_status_reads_head_pipeline_then_falls_back() {
        // head_pipeline wins.
        let json =
            r#"{"iid":1,"head_pipeline":{"status":"success"},"pipeline":{"status":"failed"}}"#;
        assert_eq!(parse_ci_status(json).unwrap(), CiStatus::Passing);
        // Falls back to the deprecated `pipeline` when there's no head_pipeline.
        let json = r#"{"iid":1,"pipeline":{"status":"failed"}}"#;
        assert_eq!(parse_ci_status(json).unwrap(), CiStatus::Failing);
        // No pipeline at all → None.
        let json = r#"{"iid":1}"#;
        assert_eq!(parse_ci_status(json).unwrap(), CiStatus::None);
    }
}
