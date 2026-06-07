//! Typed results from `tea … --output json` and the deserialization helpers.
//!
//! **`tea --output json` is NOT the Gitea REST shape.** It has two distinct
//! paths (verified against tea's source — `modules/print/table.go` for the table,
//! `cmd/issues.go` for the issue-detail `buildIssueData`):
//!
//! - **List** commands (`pr/issues/releases list`) serialize tea's print-table:
//!   a JSON **array of string-maps** whose keys are column headers run through
//!   tea's `toSnakeCase`, and whose **values are all JSON strings** — never typed
//!   numbers/bools, never `html_url`, never nested `head.ref`/`base.ref`. We
//!   select the columns we need with `--fields` where the command supports it.
//!   `toSnakeCase` is quirky: its `(.)([A-Z][a-z]+)` rule inserts a stray `_`
//!   before each capitalised run, so the fixed `releases` headers (`Tag-Name`,
//!   `Published At`, `Tar/Zip URL`) become the literal keys `"tag-_name"`,
//!   `"published _at"`, `"tar/_zip url"` (spaces/slashes preserved). Lowercase
//!   single-word `--fields` headers (`index`, `head`, …) snake-case to themselves.
//! - **Detail** views (`issues <n>`) bypass the table and marshal a hand-written
//!   **typed** struct (real numbers, mixed-case keys), a single object.
//!
//! So the internal list DTOs are string-typed (`From` parses `index` → `u64`),
//! the issue-detail DTO is typed, and the public structs are the flattened
//! result either way. Parsing is pure, so the unit tests are hermetic — but the
//! fixtures must encode tea's *table* shape, not the REST shape; the definitive
//! check is the `#[ignore]` real-`tea` tests in `tests/cli.rs`.

use processkit::{Error, Result};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::BINARY;

/// A pull request (`tea pr list --output json`), flattened from tea's table
/// columns (`index`/`title`/`state`/`head`/`base`/`url`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PullRequest {
    /// PR number (tea's `index` column).
    pub number: u64,
    /// PR title.
    pub title: String,
    /// State, e.g. `"open"`, `"closed"`, `"merged"` — tea folds the merge flag
    /// into this column (a merged PR reads `"merged"`, not `"closed"`).
    pub state: String,
    /// Whether the PR has been merged — derived from `state == "merged"` (tea has
    /// no separate merged column).
    pub merged: bool,
    /// Source (head) branch name (tea's `head` column, a flat branch name).
    pub head_branch: String,
    /// Target (base) branch name (tea's `base` column, a flat branch name).
    pub base_branch: String,
    /// Web URL (tea's `url` column).
    pub url: String,
}

// A row of `tea pr list --output json` — every value is a JSON string. `index`
// has no `default`: a row always carries it, so a missing id is a real parse
// failure, not a silent `0` that `pr_view` could then "find".
#[derive(Deserialize)]
struct PrJson {
    index: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    head: String,
    #[serde(default)]
    base: String,
    #[serde(default)]
    url: String,
}

impl TryFrom<PrJson> for PullRequest {
    type Error = Error;

    fn try_from(raw: PrJson) -> Result<Self> {
        Ok(PullRequest {
            number: parse_index(&raw.index)?,
            title: raw.title,
            // tea's `state` column already folds in the merge flag.
            merged: raw.state.eq_ignore_ascii_case("merged"),
            state: raw.state,
            head_branch: raw.head,
            base_branch: raw.base,
            url: raw.url,
        })
    }
}

/// An issue (`tea issues list --output json` / `tea issues <index> --output
/// json`). The two tea paths differ — the **list** is a string-table row, the
/// **detail** view a typed object — but both flatten into this struct.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Issue {
    /// Issue number (tea's `index`).
    pub number: u64,
    /// Issue title.
    pub title: String,
    /// State, e.g. `"open"`, `"closed"`.
    pub state: String,
    /// Issue body / description.
    pub body: String,
    /// Web URL (tea's `url`).
    pub url: String,
}

// A row of `tea issues list --output json` — all-string values, `index` column.
// We pass `--fields index,title,state,body,url`, so all are present, but keep
// `default` on the optionals to tolerate a future column trim.
#[derive(Deserialize)]
struct IssueListJson {
    index: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    url: String,
}

impl TryFrom<IssueListJson> for Issue {
    type Error = Error;

    fn try_from(raw: IssueListJson) -> Result<Self> {
        Ok(Issue {
            number: parse_index(&raw.index)?,
            title: raw.title,
            state: raw.state,
            body: raw.body,
            url: raw.url,
        })
    }
}

// The single-issue **detail** view (`tea issues <n> --output json`) is a typed
// object built by tea's `buildIssueData` (`cmd/issues.go`): `index` is a
// real number, keys are `index`/`title`/`state`/`body`/`url`. No `default` on
// `index`: a missing id is a real parse failure.
#[derive(Deserialize)]
struct IssueDetailJson {
    index: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    url: String,
}

impl From<IssueDetailJson> for Issue {
    fn from(raw: IssueDetailJson) -> Self {
        Issue {
            number: raw.index,
            title: raw.title,
            state: raw.state,
            body: raw.body,
            url: raw.url,
        }
    }
}

/// A release (`tea releases list --output json`), flattened from tea's fixed
/// release-table columns. **`tea releases` exposes no web-page URL** (only a
/// combined tar/zip download URL, which we deliberately don't surface), so
/// [`url`](Release::url) is always empty for Gitea — see the field doc.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Release {
    /// Git tag the release points at (tea's `Tag-Name` column).
    pub tag: String,
    /// Release title (tea's `Title` column).
    pub title: String,
    /// Publish timestamp, e.g. `"2023-07-26T13:02:36Z"` (tea's `Published At`
    /// column); empty for an unpublished draft.
    pub published_at: String,
    /// Whether the release is a draft (derived from tea's `Status` column).
    pub draft: bool,
    /// Whether the release is a pre-release (derived from tea's `Status` column).
    pub prerelease: bool,
    /// **Always empty for Gitea.** `tea releases list` has no release-page URL
    /// column (only a tar/zip download URL, intentionally not surfaced here).
    pub url: String,
}

// A row of `tea releases list --output json`: all-string values, fixed columns.
// `releases list` has no `--fields` flag. The keys are tea's Title-Case headers
// (`Tag-Name`/`Published At`/`Status`/`Tar/Zip URL`) run through tea's
// `toSnakeCase`, whose `(.)([A-Z][a-z]+)` rule inserts a stray `_` before each
// capitalised run — so the literal keys are `tag-_name`, `published _at`,
// `status`, `tar/_zip url` (verified against tea's `modules/print/table.go`).
#[derive(Deserialize)]
struct ReleaseJson {
    // No `default`: a row always carries the tag column, so a missing tag is a
    // real parse failure rather than a silent empty string.
    #[serde(rename = "tag-_name")]
    tag_name: String,
    #[serde(default)]
    title: String,
    #[serde(rename = "published _at", default)]
    published_at: String,
    // tea collapses draft/prerelease/released into one `Status` column.
    #[serde(default)]
    status: String,
}

impl From<ReleaseJson> for Release {
    fn from(raw: ReleaseJson) -> Self {
        Release {
            tag: raw.tag_name,
            title: raw.title,
            published_at: raw.published_at,
            draft: raw.status.eq_ignore_ascii_case("draft"),
            prerelease: raw.status.eq_ignore_ascii_case("prerelease"),
            // tea's release table carries no web-page URL column.
            url: String::new(),
        }
    }
}

/// Parse a tea table cell holding an issue/PR index (always a JSON **string**,
/// e.g. `"4"`) into a `u64`, mapping a non-numeric value to [`Error::Parse`].
fn parse_index(value: &str) -> Result<u64> {
    value.trim().parse().map_err(|_| Error::Parse {
        program: BINARY.to_string(),
        message: format!("expected a numeric index, got {value:?}"),
    })
}

/// Deserialize `tea … --output json` output into `T`, mapping parse errors to
/// [`Error::Parse`].
pub(crate) fn from_json<T: DeserializeOwned>(json: &str) -> Result<T> {
    serde_json::from_str(json).map_err(|e| Error::Parse {
        program: BINARY.to_string(),
        message: e.to_string(),
    })
}

/// Parse `tea pr list --output json` into the flattened [`PullRequest`]s.
pub(crate) fn parse_pr_list(json: &str) -> Result<Vec<PullRequest>> {
    let raw: Vec<PrJson> = from_json(json)?;
    raw.into_iter().map(PullRequest::try_from).collect()
}

/// Parse `tea issues list --output json` into the flattened [`Issue`]s.
pub(crate) fn parse_issue_list(json: &str) -> Result<Vec<Issue>> {
    let raw: Vec<IssueListJson> = from_json(json)?;
    raw.into_iter().map(Issue::try_from).collect()
}

/// Parse `tea issues <index> --output json` into a single [`Issue`]. Unlike the
/// list, the single-issue view yields one **typed** object, not an array.
pub(crate) fn parse_issue(json: &str) -> Result<Issue> {
    let raw: IssueDetailJson = from_json(json)?;
    Ok(Issue::from(raw))
}

/// Parse `tea releases list --output json` into the flattened [`Release`]s.
pub(crate) fn parse_release_list(json: &str) -> Result<Vec<Release>> {
    let raw: Vec<ReleaseJson> = from_json(json)?;
    Ok(raw.into_iter().map(Release::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // `tea pr list --output json` is a table: all-string values, `index` column,
    // flat `head`/`base`, `url` column. (We pass `--fields index,title,state,
    // head,base,url`.)
    #[test]
    fn parses_pr_list_table_row() {
        let json = r#"[
            {"index": "7", "title": "Add X", "state": "open",
             "head": "feat/x", "base": "main", "url": "https://gitea/pr/7"}
        ]"#;
        let prs = parse_pr_list(json).expect("parse prs");
        assert_eq!(prs.len(), 1);
        assert_eq!(
            prs[0],
            PullRequest {
                number: 7,
                title: "Add X".into(),
                state: "open".into(),
                merged: false,
                head_branch: "feat/x".into(),
                base_branch: "main".into(),
                url: "https://gitea/pr/7".into(),
            }
        );
    }

    // tea folds the merge flag into the `state` column: a merged PR reads
    // `state="merged"`, from which `merged` is derived.
    #[test]
    fn pr_state_merged_derives_the_flag() {
        let json = r#"[{"index": "9", "title": "done", "state": "merged",
                        "head": "f", "base": "main", "url": "u"}]"#;
        let prs = parse_pr_list(json).expect("parse prs");
        assert_eq!(prs[0].number, 9);
        assert!(prs[0].merged);
        assert_eq!(prs[0].state, "merged");
    }

    // A non-numeric `index` string is a real parse failure, not a silent `0`
    // that `pr_view` could then "find".
    #[test]
    fn pr_non_numeric_index_is_a_parse_error() {
        match parse_pr_list(r#"[{"index": "x", "title": "t", "state": "open"}]"#).unwrap_err() {
            Error::Parse { .. } => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn malformed_json_is_a_parse_error() {
        match parse_pr_list("not json").unwrap_err() {
            Error::Parse { .. } => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    // `tea issues list --output json` is a table — all-string values, `index`
    // column. We request `--fields index,title,state,body,url`.
    #[test]
    fn parses_issue_list_table_row() {
        let json = r#"[
            {"index": "12", "title": "Bug", "state": "open", "body": "broken",
             "url": "https://gitea/issues/12"}
        ]"#;
        let issues = parse_issue_list(json).expect("parse issues");
        assert_eq!(issues.len(), 1);
        assert_eq!(
            issues[0],
            Issue {
                number: 12,
                title: "Bug".into(),
                state: "open".into(),
                body: "broken".into(),
                url: "https://gitea/issues/12".into(),
            }
        );
    }

    // A column trim (body/url absent) must still parse via the field defaults.
    #[test]
    fn issue_list_tolerates_trimmed_columns() {
        let json = r#"[{"index": "4", "title": "wip", "state": "open"}]"#;
        let issues = parse_issue_list(json).expect("parse issues");
        assert_eq!(issues[0].number, 4);
        assert_eq!(issues[0].body, "");
        assert_eq!(issues[0].url, "");
    }

    // The single-issue **detail** view (`tea issues <index> --output json`) is a
    // typed object: `index` is a real JSON number, not a string.
    #[test]
    fn parses_single_issue_detail_object() {
        let json = r#"{"index": 7, "title": "One", "state": "closed", "body": "b",
                       "url": "https://gitea/issues/7"}"#;
        let issue = parse_issue(json).expect("parse issue");
        assert_eq!(issue.number, 7);
        assert_eq!(issue.title, "One");
        assert_eq!(issue.state, "closed");
        assert_eq!(issue.url, "https://gitea/issues/7");
    }

    // `tea releases list --output json` is a fixed table: all-string values,
    // tea's `toSnakeCase`d header keys (`tag-_name`, `published _at`, `status`,
    // `tar/_zip url` — note the stray `_` tea's snake-caser inserts), and NO
    // release-page URL column.
    #[test]
    fn parses_release_list_table_row() {
        let json = r#"[
            {"tag-_name": "0.1", "title": "First", "status": "released",
             "published _at": "2023-07-26T13:02:36Z",
             "tar/_zip url": "https://gitea/0.1.tar.gz\nhttps://gitea/0.1.zip"}
        ]"#;
        let releases = parse_release_list(json).expect("parse releases");
        assert_eq!(releases.len(), 1);
        assert_eq!(
            releases[0],
            Release {
                tag: "0.1".into(),
                title: "First".into(),
                published_at: "2023-07-26T13:02:36Z".into(),
                draft: false,
                prerelease: false,
                url: String::new(), // tea exposes no release-page URL
            }
        );
    }

    // A draft release: tea's `status` column is "draft", and `published _at` is
    // empty (zero time). The status string drives the `draft` flag.
    #[test]
    fn release_status_drives_draft_flag() {
        let json = r#"[{"tag-_name": "v2", "title": "Two", "status": "draft",
                        "published _at": ""}]"#;
        let releases = parse_release_list(json).expect("parse releases");
        assert_eq!(releases[0].tag, "v2");
        assert!(releases[0].draft);
        assert_eq!(releases[0].published_at, "");
        assert!(!releases[0].prerelease);
    }

    // A prerelease: `status` = "prerelease" sets the prerelease flag only.
    #[test]
    fn release_status_drives_prerelease_flag() {
        let json = r#"[{"tag-_name": "v3-rc1", "title": "RC", "status": "prerelease",
                        "published _at": "2026-01-02T03:04:05Z"}]"#;
        let releases = parse_release_list(json).expect("parse releases");
        assert!(releases[0].prerelease);
        assert!(!releases[0].draft);
    }

    // A release row without the tag column is a real parse failure, not a silent
    // empty tag.
    #[test]
    fn release_missing_tag_is_a_parse_error() {
        match parse_release_list(r#"[{"title": "no tag"}]"#).unwrap_err() {
            Error::Parse { .. } => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    // auth_status counts the logins array; an empty array means "not logged in".
    #[test]
    fn login_array_counts() {
        let some: Vec<serde_json::Value> =
            from_json(r#"[{"name":"gitea"}]"#).expect("parse logins");
        assert!(!some.is_empty());
        let none: Vec<serde_json::Value> = from_json("[]").expect("parse empty");
        assert!(none.is_empty());
    }
}
