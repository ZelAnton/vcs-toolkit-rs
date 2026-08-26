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
/// (`gh pr list/view --json number,title,state,isDraft,headRefName,headRefOid,
/// baseRefName,isCrossRepository,url`).
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
    /// Exact object id at the PR head. Empty only when an older/scripted response
    /// omitted the requested `headRefOid`; callers that need identity proof must
    /// fail closed on that value.
    #[serde(
        rename = "headRefOid",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub head_ref_oid: String,
    /// Target (base) branch name.
    #[serde(
        rename = "baseRefName",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub base_ref_name: String,
    /// Whether the PR head comes from another repository. `None` means the
    /// requested field was absent, not "same repository".
    #[serde(rename = "isCrossRepository", default)]
    pub is_cross_repository: Option<bool>,
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
    /// Exact commit SHA the workflow ran for (`headSha`). An empty value means
    /// the CLI/backend did not provide revision evidence and must never match an
    /// exact-revision CI query.
    #[serde(
        rename = "headSha",
        default,
        deserialize_with = "vcs_cli_support::json::null_to_empty"
    )]
    pub head_sha: String,
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

/// A GitHub Actions workflow definition (`gh workflow list --json …`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Workflow {
    /// The workflow's repository-scoped database id.
    pub id: u64,
    /// Display name from the workflow file.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub name: String,
    /// Repository-relative workflow file path (normally `.github/workflows/*.yml`).
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub path: String,
    /// GitHub state, e.g. `"active"`, `"disabled_manually"`, or
    /// `"disabled_inactivity"`.
    #[serde(default, deserialize_with = "vcs_cli_support::json::null_to_empty")]
    pub state: String,
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

/// One account `gh auth status` reports as logged in: the host it is logged in
/// to, its login, and whether `gh` marks it **active** for that host.
///
/// A machine can hold several `gh` logins for one host (a personal and a work
/// account) but runs commands as exactly one of them — the active one. That
/// distinction is what makes an auth probe honest: a session existing says
/// nothing about *which* identity a call will run under.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AuthAccount {
    /// The host this account is logged in to, verbatim from `gh` (e.g.
    /// `github.com`, or a GitHub Enterprise Server hostname).
    pub host: String,
    /// The account login.
    pub login: String,
    /// Whether `gh` marks this account as the **active** one for its host — the
    /// identity commands actually run as. `None` when the report says nothing
    /// either way: a `gh` older than 2.40 has no multi-account concept and prints
    /// no active marker at all. Read [`GitHubAuth::active`] rather than this field
    /// to answer "which account is in use" — it resolves the `None` case and
    /// refuses to guess when the report is ambiguous.
    pub active: Option<bool>,
}

/// What one `gh auth status` run says: whether a session exists at all, and the
/// accounts `gh` reports as logged in. Returned by
/// [`auth_info`](crate::GitHubApi::auth_info).
///
/// **Fail-soft.** `gh auth status` has no `--json`, so the accounts are read from
/// its human-readable output. A format this parser doesn't model degrades to an
/// **empty** [`accounts`](Self::accounts) — "unknown", never an error — so a `gh`
/// upgrade that reshapes the text can't break the probe. Distinguish the two empty
/// cases with [`authed`](Self::authed): `authed == false` means *no session*, while
/// `authed == true` with no accounts means the output wasn't recognised (see
/// [`is_unknown`](Self::is_unknown)).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct GitHubAuth {
    /// Whether `gh auth status` exited zero — the same coarse "some session
    /// exists" answer [`auth_status`](crate::GitHubApi::auth_status) returns, from
    /// the same run that produced [`accounts`](Self::accounts).
    pub authed: bool,
    /// Every account `gh` reported as logged in, in gh's own order. Empty when
    /// there is no session **or** when the output format wasn't recognised.
    pub accounts: Vec<AuthAccount>,
}

impl GitHubAuth {
    /// The account commands run as, when that is unambiguous — `None` when it
    /// isn't (which is itself the honest answer, not a failure).
    ///
    /// Resolution, in order:
    ///
    /// - exactly one account marked active → that account (the ordinary case,
    ///   including a machine with several logins on one host);
    /// - a lone account the report says **nothing** about → that account. A `gh`
    ///   older than 2.40 has no multi-account concept and prints no active marker,
    ///   so its single login *is* the identity in use;
    /// - anything else → `None`: several hosts each contribute their own active
    ///   account (which one applies depends on the repository's host, which this
    ///   probe does not resolve); or the report marks accounts active and none of
    ///   the ones recognised here is the active one — notably when the active
    ///   entry is a **failed** login (an invalid `GH_TOKEN` in the environment
    ///   outranks a working keyring account, and gh reports it as failed rather
    ///   than as a logged-in account), where naming the surviving account would be
    ///   exactly the wrong answer; or nothing was recognised at all. Read
    ///   [`accounts`](Self::accounts) — each entry carries its host and active
    ///   flag — to see the full picture.
    pub fn active(&self) -> Option<&AuthAccount> {
        let mut marked = self
            .accounts
            .iter()
            .filter(|account| account.active == Some(true));
        match (marked.next(), marked.next()) {
            // Exactly one account is flagged active — the unambiguous answer.
            (Some(only), None) => Some(only),
            // Several hosts, each with its own active account: ambiguous here.
            (Some(_), Some(_)) => None,
            // Nothing recognised is flagged active. A lone login the report is
            // *silent* about (a pre-2.40 `gh`) is the identity in use; a lone
            // login the report explicitly did not flag is not — the active entry
            // is then something this parser skipped.
            (None, _) => match self.accounts.as_slice() {
                [only] if only.active.is_none() => Some(only),
                _ => None,
            },
        }
    }

    /// Whether `gh` reported a session but **no** account line was recognised —
    /// i.e. the output is in a format this parser doesn't model (a future `gh`,
    /// or a locale/format this crate hasn't seen). The honest "unknown", as
    /// opposed to the "no session" that [`authed`](Self::authed) `== false` means.
    pub fn is_unknown(&self) -> bool {
        self.authed && self.accounts.is_empty()
    }
}

/// The marker that opens an account line in `gh auth status` output — the same
/// wording in the pre-2.40 single-account form (`Logged in to github.com as
/// octocat (keyring)`) and the multi-account one gh 2.40+ prints (`Logged in to
/// github.com account octocat (keyring)`). A *failed* login is worded differently
/// (`Failed to log in to …`), so it never matches: an account whose token gh
/// rejects is not reported as logged in.
const LOGGED_IN: &str = "Logged in to ";

/// The marker that opens a **failed** login entry (`X Failed to log in to
/// github.com account work (keyring)`, `X Failed to log in to github.com using
/// token (GH_TOKEN)`). Not an account — but gh prints the entry its own
/// `- Active account:` detail line, which therefore describes an entry this
/// parser skips and must not qualify a neighbouring one.
const FAILED_LOGIN: &str = "Failed to log in to ";

/// The per-account detail line (gh 2.40+) that names the active account.
const ACTIVE_ACCOUNT: &str = "Active account:";

/// The widest prefix accepted before a marker on its line: gh indents the line
/// and prefixes a one-character status glyph (`✓`, `✗`, `X`) or a `-` bullet.
/// Bounding it keeps a marker that merely *appears* inside some other sentence
/// from being read as an account line.
const MAX_MARKER_PREFIX_CHARS: usize = 2;

/// The text after `marker` on `line`, when the marker opens the line (modulo
/// indentation and gh's one-character status glyph / `-` bullet). `None` when the
/// line doesn't carry the marker in that position.
fn after_marker<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let at = line.find(marker)?;
    let prefix = line[..at].trim();
    (prefix.chars().count() <= MAX_MARKER_PREFIX_CHARS).then(|| line[at + marker.len()..].trim())
}

/// Parse one `Logged in to <host> account|as <login> (<source>)` line's tail
/// (everything after the [`LOGGED_IN`] marker) into an account. `None` when the
/// shape isn't the one gh prints — the fail-soft path: an unrecognised line is
/// skipped, never an error.
fn parse_account_line(rest: &str) -> Option<AuthAccount> {
    let mut fields = rest.split_whitespace();
    let host = fields.next()?;
    // `account` is the gh >= 2.40 wording, `as` the pre-2.40 one. Anything else
    // is a sentence this parser doesn't model.
    if !matches!(fields.next()?, "account" | "as") {
        return None;
    }
    let login = fields.next()?;
    // gh appends the credential source in parentheses (`(keyring)`); a login that
    // *starts* one means the login itself was missing from the line.
    if login.starts_with('(') {
        return None;
    }
    Some(AuthAccount {
        host: host.to_string(),
        login: login.to_string(),
        // Not "inactive": nothing has been said yet. Only an `Active account:`
        // line below turns this into a `Some`.
        active: None,
    })
}

/// Parse the accounts out of `gh auth status` output (the command has no
/// `--json`, so this reads its human-readable text).
///
/// **Never fails.** Lines that don't match the shapes gh prints are skipped, so an
/// unrecognised format yields an empty list — "unknown" rather than an error, which
/// is what keeps a `gh` upgrade from breaking the probe (see [`GitHubAuth`]).
///
/// Handles both wordings gh has shipped — pre-2.40 `Logged in to <host> as
/// <login>` and the multi-account `Logged in to <host> account <login>` with its
/// `- Active account: true|false` detail line — and feeds on stdout **and** stderr
/// concatenated, because `gh` printed this report on stderr before it moved to
/// stdout.
///
/// The report is a sequence of **entries**, each a header line plus its indented
/// detail lines, and an entry this parser skips (a rejected login) still prints
/// its own `Active account:` line. A detail line is therefore bound to the entry
/// it sits under — not to "the last account parsed" — so a skipped entry's flag
/// reaches neither the account above it nor the one below, whichever order gh
/// prints them in.
pub(crate) fn parse_auth_accounts(raw: &str) -> Vec<AuthAccount> {
    let mut accounts: Vec<AuthAccount> = Vec::new();
    // The account the detail lines being read belong to: the index of the entry
    // whose header was recognised as a login, or `None` while inside an entry
    // that was not one.
    let mut current: Option<usize> = None;
    for line in raw.lines() {
        if let Some(rest) = after_marker(line, LOGGED_IN) {
            current = match parse_account_line(rest) {
                Some(account) => {
                    accounts.push(account);
                    Some(accounts.len() - 1)
                }
                // A `Logged in to …` line in a shape this parser doesn't model:
                // the entry opened, but no account was recorded for it, so its
                // detail lines have nothing to qualify.
                None => None,
            };
            continue;
        }
        if after_marker(line, FAILED_LOGIN).is_some() {
            // A rejected login opens an entry of its own — deliberately not an
            // account (see `LOGGED_IN`). Detach, so the `Active account:` line
            // gh prints under it stays with it.
            current = None;
            continue;
        }
        // gh prints `Active account:` once per entry, so the first value wins:
        // a second one under the same account came from a header this parser
        // didn't recognise, and overwriting would let it dictate the flag. A
        // value that isn't a plain bool is likewise left as "not said".
        if let Some(value) = after_marker(line, ACTIVE_ACCOUNT)
            && let Some(account) = current.and_then(|at| accounts.get_mut(at))
            && account.active.is_none()
            && let Ok(active) = value.trim().parse::<bool>()
        {
            account.active = Some(active);
        }
    }
    accounts
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
    use processkit::ErrorReason;

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
                head_ref_oid: String::new(),
                base_ref_name: "main".into(),
                is_cross_repository: None,
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

    #[test]
    fn pr_parses_exact_head_identity_and_repository_relation() {
        let json = r#"{"number":12,"title":"Add feature","state":"OPEN","isDraft":false,
            "headRefName":"feat/x","headRefOid":"0123456789abcdef","baseRefName":"main",
            "isCrossRepository":false,"url":"https://gh/pr/12"}"#;
        let pr: PullRequest =
            vcs_cli_support::json::from_json(BINARY, json).expect("parse exact PR identity");
        assert_eq!(pr.head_ref_oid, "0123456789abcdef");
        assert_eq!(pr.is_cross_repository, Some(false));
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

    // The typical gh >= 2.40 report for one login (captured from a real `gh auth
    // status`): the active account is recognised, and the masked token line is
    // *not* mistaken for account data.
    #[test]
    fn parses_auth_status_single_account() {
        let raw = "github.com\n  \u{2713} Logged in to github.com account octocat (keyring)\n  \
                   - Active account: true\n  - Git operations protocol: ssh\n  \
                   - Token: gho_************************************\n  \
                   - Token scopes: 'gist', 'read:org', 'repo', 'workflow'\n";
        let accounts = parse_auth_accounts(raw);
        assert_eq!(
            accounts,
            vec![AuthAccount {
                host: "github.com".into(),
                login: "octocat".into(),
                active: Some(true),
            }]
        );
        let auth = GitHubAuth {
            authed: true,
            accounts,
        };
        assert_eq!(auth.active().map(|a| a.login.as_str()), Some("octocat"));
        assert!(!auth.is_unknown(), "a parsed account is not `unknown`");
    }

    // Several logins on one host — the exact situation that makes a bare
    // "authenticated: true" misleading. Every account is listed, and only the one
    // gh marks active is reported as the identity in use.
    #[test]
    fn parses_auth_status_multiple_accounts_on_one_host() {
        let raw = "github.com\n  \u{2713} Logged in to github.com account work-acct (keyring)\n  \
                   - Active account: false\n  - Git operations protocol: https\n  \
                   - Token: gho_************************************\n\n  \
                   \u{2713} Logged in to github.com account personal (keyring)\n  \
                   - Active account: true\n  - Git operations protocol: https\n  \
                   - Token: gho_************************************\n";
        let auth = GitHubAuth {
            authed: true,
            accounts: parse_auth_accounts(raw),
        };
        assert_eq!(
            auth.accounts
                .iter()
                .map(|a| (a.login.as_str(), a.active))
                .collect::<Vec<_>>(),
            [("work-acct", Some(false)), ("personal", Some(true))],
            "both logins listed, with gh's own active flag"
        );
        assert_eq!(auth.active().map(|a| a.login.as_str()), Some("personal"));
    }

    // A pre-2.40 `gh` says "as <login>" and prints no active marker at all. Its
    // single login *is* the identity in use, so `active()` resolves it.
    #[test]
    fn parses_pre_multi_account_auth_status_wording() {
        let raw = "github.com\n  \u{2713} Logged in to github.com as octocat (keyring)\n  \
                   \u{2713} Git operations for github.com configured to use https protocol.\n  \
                   \u{2713} Token: *******************\n  \
                   \u{2713} Token scopes: gist, read:org, repo\n";
        let auth = GitHubAuth {
            authed: true,
            accounts: parse_auth_accounts(raw),
        };
        assert_eq!(
            auth.accounts,
            vec![AuthAccount {
                host: "github.com".into(),
                login: "octocat".into(),
                active: None,
            }],
            "this gh says nothing about active accounts — `None`, not `false`"
        );
        assert_eq!(
            auth.active().map(|a| a.login.as_str()),
            Some("octocat"),
            "a lone login is the identity in use"
        );
    }

    // Two hosts, each with its own active account: which one applies depends on
    // the repository's host, which this probe does not resolve — so `active()` is
    // honestly `None` while both accounts stay listed with their hosts.
    #[test]
    fn active_account_is_unresolved_across_two_hosts() {
        let raw = "github.com\n  \u{2713} Logged in to github.com account octocat (keyring)\n  \
                   - Active account: true\n\nghe.example.com\n  \
                   \u{2713} Logged in to ghe.example.com account acme-bot (keyring)\n  \
                   - Active account: true\n";
        let auth = GitHubAuth {
            authed: true,
            accounts: parse_auth_accounts(raw),
        };
        assert_eq!(
            auth.accounts
                .iter()
                .map(|a| (a.host.as_str(), a.login.as_str()))
                .collect::<Vec<_>>(),
            [("github.com", "octocat"), ("ghe.example.com", "acme-bot")]
        );
        assert!(
            auth.active().is_none(),
            "two hosts, two active accounts — ambiguous, not a guess"
        );
        assert!(!auth.is_unknown(), "accounts were recognised");
    }

    // The fail-soft contract: output this parser doesn't model — a future `gh`
    // that switches to JSON, a translated build, an unrelated message — yields an
    // empty list, never a panic or an `Err`. `is_unknown` reports that honestly
    // instead of implying "no accounts".
    #[test]
    fn unrecognised_auth_status_output_is_unknown_not_an_error() {
        for raw in [
            // A hypothetical future JSON report.
            r#"{"hosts":[{"host":"github.com","accounts":[{"login":"octocat","active":true}]}]}"#,
            // Reworded/translated output.
            "github.com\n  \u{2713} Angemeldet bei github.com als octocat (keyring)\n",
            // The marker inside a sentence, not opening an account line.
            "note: the string \"Logged in to github.com account octocat\" is not a report\n",
            // A line that starts right but has no login.
            "  \u{2713} Logged in to github.com account (keyring)\n",
            // Truncated mid-word, and a stray active marker with no account.
            "  \u{2713} Logged in to\n  - Active account: true\n",
            "",
        ] {
            let auth = GitHubAuth {
                authed: true,
                accounts: parse_auth_accounts(raw),
            };
            assert!(
                auth.accounts.is_empty(),
                "unrecognised output must parse to no accounts: {raw:?}"
            );
            assert!(auth.active().is_none(), "nothing to be active: {raw:?}");
            assert!(
                auth.is_unknown(),
                "a session with no parsed account: {raw:?}"
            );
        }

        // With no session at all, the same empty list means "not logged in" —
        // `is_unknown` must not claim the format was unrecognised.
        let none = GitHubAuth::default();
        assert!(!none.authed);
        assert!(!none.is_unknown());
    }

    // gh's "not logged in" message parses to nothing (and, paired with the
    // non-zero exit the caller folds into `authed`, reads as "no session").
    #[test]
    fn logged_out_auth_status_has_no_accounts() {
        let raw = "You are not logged into any GitHub hosts. To log in, run: gh auth login\n";
        assert!(parse_auth_accounts(raw).is_empty());
    }

    // A login gh reports as *broken* is not a usable identity: the failure wording
    // differs, so it is not listed as logged in.
    #[test]
    fn failed_login_is_not_reported_as_logged_in() {
        let raw = "github.com\n  X Failed to log in to github.com account stale-acct (keyring)\n  \
                   - Active account: true\n  - The token in keyring is invalid.\n";
        assert!(
            parse_auth_accounts(raw).is_empty(),
            "a rejected token is not a logged-in account"
        );
    }

    // Regression, captured verbatim from a real `gh auth status` run with an
    // invalid `GH_TOKEN` in the environment: the **active** entry is the failed
    // env token, and the surviving keyring account is explicitly NOT active. The
    // skipped entry's `Active account: true` line must not be misattributed to the
    // account below it, and the lone surviving login must not be reported as the
    // identity in use — gh is not using it.
    #[test]
    fn active_marker_of_a_failed_entry_is_not_inherited() {
        let raw = "github.com\n  X Failed to log in to github.com using token (GH_TOKEN)\n  \
                   - Active account: true\n  - The token in GH_TOKEN is invalid.\n\n  \
                   \u{2713} Logged in to github.com account octocat (keyring)\n  \
                   - Active account: false\n  - Git operations protocol: ssh\n";
        let auth = GitHubAuth {
            // gh exits non-zero when any configured entry fails, even with a
            // working account alongside it.
            authed: false,
            accounts: parse_auth_accounts(raw),
        };
        assert_eq!(
            auth.accounts,
            vec![AuthAccount {
                host: "github.com".into(),
                login: "octocat".into(),
                active: Some(false),
            }],
            "the failed entry's active marker stays with the failed entry"
        );
        assert!(
            auth.active().is_none(),
            "the only recognised login is explicitly not the active one — \
             naming it would point at the wrong identity"
        );
    }

    // The mirror image of the regression above: the failed entry comes *after*
    // the working one. gh prints an `Active account:` line under the entry it
    // describes, so the failed entry's `false` must not overwrite the flag of the
    // account above it — that would turn "this is the identity in use" into an
    // explicit, and wrong, negative.
    #[test]
    fn failed_entry_after_a_login_does_not_clear_its_active_flag() {
        let raw = "github.com\n  \u{2713} Logged in to github.com account personal (keyring)\n  \
                   - Active account: true\n  - Git operations protocol: ssh\n\n  \
                   X Failed to log in to github.com account work (keyring)\n  \
                   - Active account: false\n  - The token in keyring is invalid.\n";
        let auth = GitHubAuth {
            // gh exits non-zero when any configured entry fails.
            authed: false,
            accounts: parse_auth_accounts(raw),
        };
        assert_eq!(
            auth.accounts,
            vec![AuthAccount {
                host: "github.com".into(),
                login: "personal".into(),
                active: Some(true),
            }],
            "the failed entry's active marker stays with the failed entry"
        );
        assert_eq!(
            auth.active().map(|a| a.login.as_str()),
            Some("personal"),
            "the account gh marks active is still the identity in use"
        );
    }

    // The other order of the same pair: a failed entry marked ACTIVE arrives
    // after a working login gh marked inactive. Inheriting that `true` would name
    // the wrong identity — the surviving account is precisely the one gh is *not*
    // running as.
    #[test]
    fn active_marker_of_a_failed_entry_below_is_not_inherited() {
        let raw = "github.com\n  \u{2713} Logged in to github.com account work-acct (keyring)\n  \
                   - Active account: false\n  - Git operations protocol: ssh\n\n  \
                   X Failed to log in to github.com using token (GH_TOKEN)\n  \
                   - Active account: true\n  - The token in GH_TOKEN is invalid.\n";
        let auth = GitHubAuth {
            authed: false,
            accounts: parse_auth_accounts(raw),
        };
        assert_eq!(
            auth.accounts,
            vec![AuthAccount {
                host: "github.com".into(),
                login: "work-acct".into(),
                active: Some(false),
            }],
            "the surviving login keeps gh's own `false`"
        );
        assert!(
            auth.active().is_none(),
            "the active entry is one this parser skipped — naming the survivor \
             would point at the wrong identity"
        );
    }

    #[test]
    fn malformed_json_is_a_parse_error() {
        match vcs_cli_support::json::from_json::<Vec<Issue>>(BINARY, "not json")
            .unwrap_err()
            .into_reason()
        {
            ErrorReason::Parse { .. } => {}
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
    fn parses_workflow_inventory() {
        let json = r#"[
            {"id": 17, "name": "CI", "path": ".github/workflows/ci.yml",
             "state": "active"},
            {"id": 18, "name": null, "path": null, "state": null}
        ]"#;
        let workflows: Vec<Workflow> =
            vcs_cli_support::json::from_json(BINARY, json).expect("parse workflows");
        assert_eq!(workflows[0].id, 17);
        assert_eq!(workflows[0].name, "CI");
        assert_eq!(workflows[0].path, ".github/workflows/ci.yml");
        assert_eq!(workflows[0].state, "active");
        assert_eq!(workflows[1].name, "");
        assert_eq!(workflows[1].path, "");
        assert_eq!(workflows[1].state, "");
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
