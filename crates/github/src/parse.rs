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

/// An issue (`gh issue list --json number,title,state`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Issue {
    /// Issue number.
    pub number: u64,
    /// Issue title.
    pub title: String,
    /// State, e.g. `"OPEN"`, `"CLOSED"`.
    pub state: String,
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
}
