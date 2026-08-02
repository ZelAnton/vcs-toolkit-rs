//! Canonical **forge-CLI output fixtures** — hermetic `gh` / `glab` / `tea`
//! payloads for consumer tests, without a live forge and without copy-pasted
//! fragile JSON strings.
//!
//! A consumer testing its own logic on top of
//! [`vcs-forge`](https://crates.io/crates/vcs-forge) /
//! [`vcs-github`](https://crates.io/crates/vcs-github) / … normally drives the
//! wrapper client through a scripted [`ProcessRunner`] and has to supply the
//! stdout the real CLI would have printed. Reverse-engineering that stdout is
//! the expensive part: the three CLIs agree on nothing. `gh` emits a compact
//! JSON object with **alphabetically ordered** keys and nested
//! `author`/`labels`/`milestone` objects; `glab` passes GitLab's **REST** JSON
//! through verbatim (flat `labels` strings, `iid`, `_links.self`); `tea` emits
//! neither — a **quoted DSV table**, in one of two wire dialects, whose columns
//! are positional. These builders emit each of those shapes so a test never has
//! to.
//!
//! Every builder starts from a complete, plausible default row and exposes
//! setters for the fields a scenario actually cares about, so a test states only
//! what it is testing:
//!
//! ```
//! use vcs_testkit::forge_fixtures::GhPr;
//!
//! let stdout = GhPr::list(&[
//!     GhPr::new(12, "Add feature").head("feat/x"),
//!     GhPr::new(13, "Draft work").draft(true),
//! ]);
//! assert!(stdout.starts_with('['));
//! ```
//!
//! Feed the result to a scripted runner as the CLI's stdout — the argv prefix
//! each fixture answers is named in its own docs:
//!
//! ```rust,ignore
//! let runner = ScriptedRunner::new()
//!     .on(["gh", "pr", "list"], Reply::ok(GhPr::list(&[GhPr::new(12, "Add feature")])));
//! let prs = GitHub::with_runner(runner).pr_list(dir).await?;
//! ```
//!
//! # These shapes cannot silently rot
//!
//! Every fixture here is pinned by `crates/testkit/tests/forge_fixtures.rs`,
//! which feeds it to the **real** parser of the matching wrapper crate
//! (`vcs_github` / `vcs_gitlab` / `vcs_gitea`, dev-dependencies of this crate —
//! never runtime ones) through that crate's actual client method. A fixture that
//! drifted from the parser it models fails that test instead of quietly
//! misleading every consumer that trusted it.
//!
//! [`ProcessRunner`]: https://docs.rs/processkit/latest/processkit/trait.ProcessRunner.html

// ---------------------------------------------------------------------------
// JSON writing (hand-rolled: this crate has no dependencies, not even serde)
// ---------------------------------------------------------------------------

/// Encode `value` as a JSON string literal, surrounding quotes included.
fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Every other control character has no short escape; JSON requires
            // it be written as a `\u` sequence rather than emitted literally.
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A compact JSON object from `(key, already-encoded value)` pairs, in the given
/// order — the CLIs emit no whitespace, so neither do we.
fn json_object(fields: &[(&str, String)]) -> String {
    let body: Vec<String> = fields
        .iter()
        .map(|(key, value)| format!("{}:{value}", json_string(key)))
        .collect();
    format!("{{{}}}", body.join(","))
}

/// A compact JSON array from already-encoded values.
fn json_array(values: &[String]) -> String {
    format!("[{}]", values.join(","))
}

/// A compact JSON array of strings.
fn json_string_array(values: &[String]) -> String {
    let encoded: Vec<String> = values.iter().map(|v| json_string(v)).collect();
    json_array(&encoded)
}

/// A JSON array of one-key objects — the shape both forges use for nested user /
/// label lists (`[{"login": …}]`, `[{"username": …}]`).
fn json_object_array(key: &str, values: &[String]) -> String {
    let encoded: Vec<String> = values
        .iter()
        .map(|v| json_object(&[(key, json_string(v))]))
        .collect();
    json_array(&encoded)
}

/// `{"<key>": "<value>"}`, or JSON `null` when there is no value — the shape
/// `gh`/`glab` use for an optional nested object (`author`, `milestone`).
fn json_nested_or_null(key: &str, value: Option<&str>) -> String {
    match value {
        Some(v) => json_object(&[(key, json_string(v))]),
        None => "null".to_string(),
    }
}

fn json_bool(value: bool) -> String {
    if value { "true" } else { "false" }.to_string()
}

/// Render every builder in `items` through `entry` and join them into the JSON
/// array a `… list` command prints.
fn json_list<T>(items: &[T], entry: impl Fn(&T) -> String) -> String {
    let encoded: Vec<String> = items.iter().map(entry).collect();
    json_array(&encoded)
}

// ---------------------------------------------------------------------------
// GitHub (`gh … --json <fields>`)
// ---------------------------------------------------------------------------

/// A pull request as `gh pr view --json …` / `gh pr list --json …` print it.
///
/// `gh` requests one field list for both subcommands, so the object shape is
/// identical for [`view`](GhPr::view) and [`list`](GhPr::list) — unlike
/// [`GhRelease`], whose two subcommands genuinely diverge.
///
/// Answers the argv prefixes `["gh", "pr", "view"]` and `["gh", "pr", "list"]`.
#[derive(Debug, Clone)]
pub struct GhPr {
    number: u64,
    title: String,
    state: String,
    draft: bool,
    head: String,
    base: String,
    url: String,
    labels: Vec<String>,
    assignees: Vec<String>,
    author: String,
    created_at: String,
    updated_at: String,
    milestone: Option<String>,
}

impl GhPr {
    /// An open, non-draft PR `number` titled `title`, from `feature` into
    /// `main`, authored by `octocat`, with no labels, assignees or milestone.
    pub fn new(number: u64, title: &str) -> Self {
        Self {
            number,
            title: title.to_string(),
            state: "OPEN".to_string(),
            draft: false,
            head: "feature".to_string(),
            base: "main".to_string(),
            url: format!("https://github.com/octocat/hello-world/pull/{number}"),
            labels: Vec::new(),
            assignees: Vec::new(),
            author: "octocat".to_string(),
            created_at: CREATED_AT.to_string(),
            updated_at: UPDATED_AT.to_string(),
            milestone: None,
        }
    }

    /// `gh`'s upper-case PR state: `"OPEN"`, `"CLOSED"` or `"MERGED"` (note this
    /// is **not** GitLab's `"opened"` nor Gitea's `"open"`).
    pub fn state(mut self, state: &str) -> Self {
        self.state = state.to_string();
        self
    }

    /// Whether the PR is a draft (`isDraft`).
    pub fn draft(mut self, draft: bool) -> Self {
        self.draft = draft;
        self
    }

    /// Source branch (`headRefName`).
    pub fn head(mut self, branch: &str) -> Self {
        self.head = branch.to_string();
        self
    }

    /// Target branch (`baseRefName`).
    pub fn base(mut self, branch: &str) -> Self {
        self.base = branch.to_string();
        self
    }

    /// Web URL.
    pub fn url(mut self, url: &str) -> Self {
        self.url = url.to_string();
        self
    }

    /// Label names — `gh` nests each as `{"name": …}`; pass the plain names.
    pub fn labels(mut self, labels: &[&str]) -> Self {
        self.labels = to_owned(labels);
        self
    }

    /// Assignee logins — `gh` nests each as `{"login": …}`; pass plain logins.
    pub fn assignees(mut self, logins: &[&str]) -> Self {
        self.assignees = to_owned(logins);
        self
    }

    /// Author login. An empty login models `gh`'s `"author": null` (a deleted
    /// account).
    pub fn author(mut self, login: &str) -> Self {
        self.author = login.to_string();
        self
    }

    /// Creation and last-update timestamps (`createdAt` / `updatedAt`).
    pub fn timestamps(mut self, created_at: &str, updated_at: &str) -> Self {
        self.created_at = created_at.to_string();
        self.updated_at = updated_at.to_string();
        self
    }

    /// Attach a milestone by title (the default is `gh`'s `"milestone": null`).
    pub fn milestone(mut self, title: &str) -> Self {
        self.milestone = Some(title.to_string());
        self
    }

    /// The single JSON object `gh pr view --json …` prints.
    pub fn view(&self) -> String {
        // `gh` serialises the requested fields in alphabetical key order, not in
        // the order they were asked for (confirmed against the live-recorded
        // cassettes in crates/github/tests/cassettes/).
        json_object(&[
            ("assignees", json_object_array("login", &self.assignees)),
            ("author", gh_author(&self.author)),
            ("baseRefName", json_string(&self.base)),
            ("createdAt", json_string(&self.created_at)),
            ("headRefName", json_string(&self.head)),
            ("isDraft", json_bool(self.draft)),
            ("labels", json_object_array("name", &self.labels)),
            (
                "milestone",
                json_nested_or_null("title", self.milestone.as_deref()),
            ),
            ("number", self.number.to_string()),
            ("state", json_string(&self.state)),
            ("title", json_string(&self.title)),
            ("updatedAt", json_string(&self.updated_at)),
            ("url", json_string(&self.url)),
        ])
    }

    /// The JSON array `gh pr list --json …` prints. An empty slice yields `[]`,
    /// which is exactly what `gh` prints for a repository with no PRs.
    pub fn list(prs: &[GhPr]) -> String {
        json_list(prs, GhPr::view)
    }
}

/// An issue as `gh issue view --json …` / `gh issue list --json …` print it.
///
/// Both subcommands request the same field list, so one shape serves
/// [`view`](GhIssue::view) and [`list`](GhIssue::list).
///
/// Answers the argv prefixes `["gh", "issue", "view"]` and
/// `["gh", "issue", "list"]`.
#[derive(Debug, Clone)]
pub struct GhIssue {
    number: u64,
    title: String,
    state: String,
    body: String,
    url: String,
    labels: Vec<String>,
    assignees: Vec<String>,
    author: String,
    created_at: String,
    updated_at: String,
    milestone: Option<String>,
}

impl GhIssue {
    /// An open issue `number` titled `title`, with an empty body, authored by
    /// `octocat`, with no labels, assignees or milestone.
    pub fn new(number: u64, title: &str) -> Self {
        Self {
            number,
            title: title.to_string(),
            state: "OPEN".to_string(),
            body: String::new(),
            url: format!("https://github.com/octocat/hello-world/issues/{number}"),
            labels: Vec::new(),
            assignees: Vec::new(),
            author: "octocat".to_string(),
            created_at: CREATED_AT.to_string(),
            updated_at: UPDATED_AT.to_string(),
            milestone: None,
        }
    }

    /// `gh`'s upper-case issue state: `"OPEN"` or `"CLOSED"`.
    pub fn state(mut self, state: &str) -> Self {
        self.state = state.to_string();
        self
    }

    /// Issue body (markdown). Newlines are escaped for you.
    pub fn body(mut self, body: &str) -> Self {
        self.body = body.to_string();
        self
    }

    /// Web URL.
    pub fn url(mut self, url: &str) -> Self {
        self.url = url.to_string();
        self
    }

    /// Label names — `gh` nests each as `{"name": …}`; pass the plain names.
    pub fn labels(mut self, labels: &[&str]) -> Self {
        self.labels = to_owned(labels);
        self
    }

    /// Assignee logins — `gh` nests each as `{"login": …}`; pass plain logins.
    pub fn assignees(mut self, logins: &[&str]) -> Self {
        self.assignees = to_owned(logins);
        self
    }

    /// Author login. An empty login models `gh`'s `"author": null` (a deleted
    /// account).
    pub fn author(mut self, login: &str) -> Self {
        self.author = login.to_string();
        self
    }

    /// Creation and last-update timestamps (`createdAt` / `updatedAt`).
    pub fn timestamps(mut self, created_at: &str, updated_at: &str) -> Self {
        self.created_at = created_at.to_string();
        self.updated_at = updated_at.to_string();
        self
    }

    /// Attach a milestone by title (the default is `gh`'s `"milestone": null`).
    pub fn milestone(mut self, title: &str) -> Self {
        self.milestone = Some(title.to_string());
        self
    }

    /// The single JSON object `gh issue view --json …` prints.
    pub fn view(&self) -> String {
        json_object(&[
            ("assignees", json_object_array("login", &self.assignees)),
            ("author", gh_author(&self.author)),
            ("body", json_string(&self.body)),
            ("createdAt", json_string(&self.created_at)),
            ("labels", json_object_array("name", &self.labels)),
            (
                "milestone",
                json_nested_or_null("title", self.milestone.as_deref()),
            ),
            ("number", self.number.to_string()),
            ("state", json_string(&self.state)),
            ("title", json_string(&self.title)),
            ("updatedAt", json_string(&self.updated_at)),
            ("url", json_string(&self.url)),
        ])
    }

    /// The JSON array `gh issue list --json …` prints.
    pub fn list(issues: &[GhIssue]) -> String {
        json_list(issues, GhIssue::view)
    }
}

/// A release as `gh release view --json …` / `gh release list --json …` print
/// it.
///
/// **The two subcommands do not print the same fields**, and this builder keeps
/// them apart on purpose: `release list` reports `isLatest` but no
/// `body`/`url`/`author`, while `release view` reports those three and no
/// `isLatest`. A fixture that mixed them would let a consumer's test pass
/// against a payload the real `gh` never emits.
///
/// Answers the argv prefixes `["gh", "release", "view"]` and
/// `["gh", "release", "list"]`.
#[derive(Debug, Clone)]
pub struct GhRelease {
    tag: String,
    name: String,
    body: String,
    url: String,
    published_at: String,
    draft: bool,
    prerelease: bool,
    latest: bool,
    author: String,
}

impl GhRelease {
    /// A published, non-draft, non-prerelease release of `tag`, titled `tag`,
    /// not marked latest, authored by `octocat`.
    pub fn new(tag: &str) -> Self {
        Self {
            tag: tag.to_string(),
            name: tag.to_string(),
            body: String::new(),
            url: format!("https://github.com/octocat/hello-world/releases/tag/{tag}"),
            published_at: PUBLISHED_AT.to_string(),
            draft: false,
            prerelease: false,
            latest: false,
            author: "octocat".to_string(),
        }
    }

    /// Release title (`name`).
    pub fn name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    /// Release notes (`body`) — **`release view` only**.
    pub fn body(mut self, body: &str) -> Self {
        self.body = body.to_string();
        self
    }

    /// Web URL — **`release view` only**.
    pub fn url(mut self, url: &str) -> Self {
        self.url = url.to_string();
        self
    }

    /// Publication timestamp (`publishedAt`); empty for an unpublished draft.
    pub fn published_at(mut self, timestamp: &str) -> Self {
        self.published_at = timestamp.to_string();
        self
    }

    /// Whether the release is an unpublished draft (`isDraft`).
    pub fn draft(mut self, draft: bool) -> Self {
        self.draft = draft;
        self
    }

    /// Whether the release is a pre-release (`isPrerelease`).
    pub fn prerelease(mut self, prerelease: bool) -> Self {
        self.prerelease = prerelease;
        self
    }

    /// Whether the release is the latest one (`isLatest`) — **`release list`
    /// only**.
    pub fn latest(mut self, latest: bool) -> Self {
        self.latest = latest;
        self
    }

    /// Author login — **`release view` only**. An empty login models a deleted
    /// or anonymised account (`gh` still prints an author object, with no
    /// `login`).
    pub fn author(mut self, login: &str) -> Self {
        self.author = login.to_string();
        self
    }

    /// The single JSON object `gh release view --json …` prints: `body`, `url`
    /// and `author`, but **no** `isLatest`.
    pub fn view(&self) -> String {
        json_object(&[
            (
                "author",
                json_object(&[("login", json_string(&self.author))]),
            ),
            ("body", json_string(&self.body)),
            ("isDraft", json_bool(self.draft)),
            ("isPrerelease", json_bool(self.prerelease)),
            ("name", json_string(&self.name)),
            ("publishedAt", json_string(&self.published_at)),
            ("tagName", json_string(&self.tag)),
            ("url", json_string(&self.url)),
        ])
    }

    /// One entry of `gh release list --json …`: `isLatest`, but **no** `body`,
    /// `url` or `author`.
    fn list_entry(&self) -> String {
        json_object(&[
            ("isDraft", json_bool(self.draft)),
            ("isLatest", json_bool(self.latest)),
            ("isPrerelease", json_bool(self.prerelease)),
            ("name", json_string(&self.name)),
            ("publishedAt", json_string(&self.published_at)),
            ("tagName", json_string(&self.tag)),
        ])
    }

    /// The JSON array `gh release list --json …` prints — entries carry the
    /// narrower list field set described on [`GhRelease`], not
    /// [`view`](GhRelease::view)'s.
    pub fn list(releases: &[GhRelease]) -> String {
        json_list(releases, GhRelease::list_entry)
    }
}

/// `gh`'s nested author object, or the `null` it prints for a deleted account
/// (which this builder models as an empty login).
fn gh_author(login: &str) -> String {
    if login.is_empty() {
        "null".to_string()
    } else {
        json_object(&[("login", json_string(login))])
    }
}

// ---------------------------------------------------------------------------
// GitLab (`glab … --output json`, GitLab REST passed through verbatim)
// ---------------------------------------------------------------------------

/// A merge request as `glab mr view --output json` / `glab mr list --output
/// json` print it — GitLab's REST `MergeRequest` object, which `glab` forwards
/// unchanged.
///
/// Only the fields `vcs-gitlab`'s parser reads are emitted (a real payload
/// carries dozens more, all ignored): `iid`, `title`, `state`, `source_branch`,
/// `target_branch`, `web_url`, `draft`, `labels`, `assignees`, `author`,
/// `created_at`, `updated_at`, `milestone`.
///
/// Answers the argv prefixes `["glab", "mr", "view"]` and
/// `["glab", "mr", "list"]`.
#[derive(Debug, Clone)]
pub struct GlabMr {
    iid: u64,
    title: String,
    state: String,
    source_branch: String,
    target_branch: String,
    web_url: String,
    draft: bool,
    labels: Vec<String>,
    assignees: Vec<String>,
    author: String,
    created_at: String,
    updated_at: String,
    milestone: Option<String>,
}

impl GlabMr {
    /// An opened, non-draft MR `iid` titled `title`, from `feature` into `main`,
    /// authored by `octocat`, with no labels, assignees or milestone.
    pub fn new(iid: u64, title: &str) -> Self {
        Self {
            iid,
            title: title.to_string(),
            state: "opened".to_string(),
            source_branch: "feature".to_string(),
            target_branch: "main".to_string(),
            web_url: format!("{GITLAB_PROJECT}/-/merge_requests/{iid}"),
            draft: false,
            labels: Vec::new(),
            assignees: Vec::new(),
            author: "octocat".to_string(),
            created_at: GITLAB_CREATED_AT.to_string(),
            updated_at: GITLAB_UPDATED_AT.to_string(),
            milestone: None,
        }
    }

    /// GitLab's lower-case MR state: `"opened"` (note: **not** `"open"`),
    /// `"closed"`, `"merged"` or `"locked"`.
    pub fn state(mut self, state: &str) -> Self {
        self.state = state.to_string();
        self
    }

    /// Whether the MR is a draft (GitLab's `draft`; the deprecated
    /// `work_in_progress` twin is not emitted, because the parser ignores it).
    pub fn draft(mut self, draft: bool) -> Self {
        self.draft = draft;
        self
    }

    /// Source branch (`source_branch`).
    pub fn source_branch(mut self, branch: &str) -> Self {
        self.source_branch = branch.to_string();
        self
    }

    /// Target branch (`target_branch`).
    pub fn target_branch(mut self, branch: &str) -> Self {
        self.target_branch = branch.to_string();
        self
    }

    /// Web URL (`web_url`).
    pub fn web_url(mut self, url: &str) -> Self {
        self.web_url = url.to_string();
        self
    }

    /// Label names. GitLab REST reports these as **plain strings**, unlike
    /// GitHub's `[{"name": …}]` — the builder emits the flat form.
    pub fn labels(mut self, labels: &[&str]) -> Self {
        self.labels = to_owned(labels);
        self
    }

    /// Assignee usernames — GitLab nests each as a User object; pass plain
    /// usernames.
    pub fn assignees(mut self, usernames: &[&str]) -> Self {
        self.assignees = to_owned(usernames);
        self
    }

    /// Author username. An empty username models GitLab's `"author": null` (an
    /// anonymised account).
    pub fn author(mut self, username: &str) -> Self {
        self.author = username.to_string();
        self
    }

    /// Creation and last-update timestamps (`created_at` / `updated_at`).
    pub fn timestamps(mut self, created_at: &str, updated_at: &str) -> Self {
        self.created_at = created_at.to_string();
        self.updated_at = updated_at.to_string();
        self
    }

    /// Attach a milestone by title (the default is GitLab's `"milestone":
    /// null`).
    pub fn milestone(mut self, title: &str) -> Self {
        self.milestone = Some(title.to_string());
        self
    }

    /// The single JSON object `glab mr view --output json` prints.
    pub fn view(&self) -> String {
        json_object(&[
            ("iid", self.iid.to_string()),
            ("title", json_string(&self.title)),
            ("state", json_string(&self.state)),
            ("created_at", json_string(&self.created_at)),
            ("updated_at", json_string(&self.updated_at)),
            ("target_branch", json_string(&self.target_branch)),
            ("source_branch", json_string(&self.source_branch)),
            ("author", glab_user(&self.author)),
            ("assignees", json_object_array("username", &self.assignees)),
            ("labels", json_string_array(&self.labels)),
            ("draft", json_bool(self.draft)),
            (
                "milestone",
                json_nested_or_null("title", self.milestone.as_deref()),
            ),
            ("web_url", json_string(&self.web_url)),
        ])
    }

    /// The JSON array `glab mr list --output json` prints.
    pub fn list(mrs: &[GlabMr]) -> String {
        json_list(mrs, GlabMr::view)
    }
}

/// An issue as `glab issue view --output json` / `glab issue list --output json`
/// print it — GitLab's REST `Issue` object.
///
/// Note the REST spellings the parser maps from: the number is `iid`, the body
/// is `description`, and the URL is `web_url`.
///
/// Answers the argv prefixes `["glab", "issue", "view"]` and
/// `["glab", "issue", "list"]`.
#[derive(Debug, Clone)]
pub struct GlabIssue {
    iid: u64,
    title: String,
    state: String,
    description: String,
    web_url: String,
    labels: Vec<String>,
    assignees: Vec<String>,
    author: String,
    created_at: String,
    updated_at: String,
    milestone: Option<String>,
}

impl GlabIssue {
    /// An opened issue `iid` titled `title`, with an empty description, authored
    /// by `octocat`, with no labels, assignees or milestone.
    pub fn new(iid: u64, title: &str) -> Self {
        Self {
            iid,
            title: title.to_string(),
            state: "opened".to_string(),
            description: String::new(),
            web_url: format!("{GITLAB_PROJECT}/-/issues/{iid}"),
            labels: Vec::new(),
            assignees: Vec::new(),
            author: "octocat".to_string(),
            created_at: GITLAB_CREATED_AT.to_string(),
            updated_at: GITLAB_UPDATED_AT.to_string(),
            milestone: None,
        }
    }

    /// GitLab's lower-case issue state: `"opened"` (note: **not** `"open"`) or
    /// `"closed"`.
    pub fn state(mut self, state: &str) -> Self {
        self.state = state.to_string();
        self
    }

    /// Issue body — GitLab's `description`. Newlines are escaped for you.
    pub fn body(mut self, body: &str) -> Self {
        self.description = body.to_string();
        self
    }

    /// Web URL (`web_url`).
    pub fn web_url(mut self, url: &str) -> Self {
        self.web_url = url.to_string();
        self
    }

    /// Label names. GitLab REST reports these as **plain strings**, unlike
    /// GitHub's `[{"name": …}]`.
    pub fn labels(mut self, labels: &[&str]) -> Self {
        self.labels = to_owned(labels);
        self
    }

    /// Assignee usernames — GitLab nests each as a User object; pass plain
    /// usernames.
    pub fn assignees(mut self, usernames: &[&str]) -> Self {
        self.assignees = to_owned(usernames);
        self
    }

    /// Author username. An empty username models GitLab's `"author": null`.
    pub fn author(mut self, username: &str) -> Self {
        self.author = username.to_string();
        self
    }

    /// Creation and last-update timestamps (`created_at` / `updated_at`).
    pub fn timestamps(mut self, created_at: &str, updated_at: &str) -> Self {
        self.created_at = created_at.to_string();
        self.updated_at = updated_at.to_string();
        self
    }

    /// Attach a milestone by title (the default is GitLab's `"milestone":
    /// null`).
    pub fn milestone(mut self, title: &str) -> Self {
        self.milestone = Some(title.to_string());
        self
    }

    /// The single JSON object `glab issue view --output json` prints.
    pub fn view(&self) -> String {
        json_object(&[
            ("iid", self.iid.to_string()),
            ("title", json_string(&self.title)),
            ("state", json_string(&self.state)),
            ("description", json_string(&self.description)),
            ("created_at", json_string(&self.created_at)),
            ("updated_at", json_string(&self.updated_at)),
            ("author", glab_user(&self.author)),
            ("assignees", json_object_array("username", &self.assignees)),
            ("labels", json_string_array(&self.labels)),
            (
                "milestone",
                json_nested_or_null("title", self.milestone.as_deref()),
            ),
            ("web_url", json_string(&self.web_url)),
        ])
    }

    /// The JSON array `glab issue list --output json` prints.
    pub fn list(issues: &[GlabIssue]) -> String {
        json_list(issues, GlabIssue::view)
    }
}

/// A release as `glab release view --output json` / `glab release list --output
/// json` print it — GitLab's REST `Release` object.
///
/// Two GitLab-specific shapes the parser depends on: the release page URL lives
/// in the nested `_links.self` (a release has **no** top-level `web_url`), and
/// the notes are `description`, not `body`. GitLab models neither drafts nor
/// pre-releases, so — unlike [`GhRelease`] — there are no such flags here.
///
/// Answers the argv prefixes `["glab", "release", "view"]` and
/// `["glab", "release", "list"]`.
#[derive(Debug, Clone)]
pub struct GlabRelease {
    tag: String,
    name: String,
    url: String,
    released_at: String,
    description: String,
    author: String,
}

impl GlabRelease {
    /// A release of `tag`, titled `tag`, with empty notes, authored by
    /// `octocat`.
    pub fn new(tag: &str) -> Self {
        Self {
            tag: tag.to_string(),
            name: tag.to_string(),
            url: format!("{GITLAB_PROJECT}/-/releases/{tag}"),
            released_at: GITLAB_RELEASED_AT.to_string(),
            description: String::new(),
            author: "octocat".to_string(),
        }
    }

    /// Release title (`name`).
    pub fn name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    /// Release page URL — emitted as the nested `_links.self`.
    pub fn url(mut self, url: &str) -> Self {
        self.url = url.to_string();
        self
    }

    /// Publication timestamp — GitLab's `released_at`.
    pub fn released_at(mut self, timestamp: &str) -> Self {
        self.released_at = timestamp.to_string();
        self
    }

    /// Release notes — GitLab's `description`.
    pub fn description(mut self, description: &str) -> Self {
        self.description = description.to_string();
        self
    }

    /// Author username. An empty username models GitLab's `"author": null`.
    pub fn author(mut self, username: &str) -> Self {
        self.author = username.to_string();
        self
    }

    /// The single JSON object `glab release view --output json` prints.
    pub fn view(&self) -> String {
        json_object(&[
            ("tag_name", json_string(&self.tag)),
            ("name", json_string(&self.name)),
            ("description", json_string(&self.description)),
            ("released_at", json_string(&self.released_at)),
            ("author", glab_user(&self.author)),
            ("_links", json_object(&[("self", json_string(&self.url))])),
        ])
    }

    /// The JSON array `glab release list --output json` prints.
    pub fn list(releases: &[GlabRelease]) -> String {
        json_list(releases, GlabRelease::view)
    }
}

/// GitLab's nested User object, or the `null` it reports for an anonymised
/// account (which this builder models as an empty username).
fn glab_user(username: &str) -> String {
    if username.is_empty() {
        "null".to_string()
    } else {
        json_object(&[("username", json_string(username))])
    }
}

// ---------------------------------------------------------------------------
// Gitea (`tea … --output csv`, a quoted DSV table in one of two dialects)
// ---------------------------------------------------------------------------

/// Which wire dialect of `tea`'s `--output csv` to emit.
///
/// `tea`'s DSV writer was reimplemented mid-line, and both shapes are in the
/// wild across the `tea` versions `vcs-gitea` supports — a consumer's parsing
/// should be exercised against **both** (loop [`TeaDsv::ALL`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TeaDsv {
    /// `tea` 0.9.x–0.13.x: every field is wrapped in `"` and the fields are
    /// joined by the three-character sequence `","`, with **no escaping at all**
    /// — so a value containing a `"` has no representable form (building one
    /// panics; use [`Rfc4180`](TeaDsv::Rfc4180)).
    Naive,
    /// `tea` 0.14.x and later: proper RFC 4180 via Go's `encoding/csv` — only
    /// fields that need it are quoted, and an embedded `"` is doubled.
    Rfc4180,
}

impl TeaDsv {
    /// Every dialect a supported `tea` emits. Loop this so a fixture-backed test
    /// covers both wire shapes rather than whichever one the author had
    /// installed.
    pub const ALL: &'static [TeaDsv] = &[TeaDsv::Naive, TeaDsv::Rfc4180];
}

// Both dialects here end a record with a bare `\n` — what Go's writers produce
// (`UseCRLF` is left off on the `encoding/csv` path). `vcs-gitea`'s reader also
// accepts `\r\n` records, so a fixture is not the place to exercise that; the
// dialects differ in *quoting*, which is what actually changes how a value must
// be read back.

/// A pull request row of `tea pr list --output csv`.
///
/// `tea` has **no single-PR view** — `vcs-gitea` synthesizes one by paging this
/// same list — so there is no `view()` here: script a one-row [`list`] for a
/// `pr_view` too.
///
/// The columns are the `--fields index,title,state,head,base,url` this crate's
/// wrapper pins, in that order (`tea`'s default column set omits
/// `head`/`base`/`url` entirely).
///
/// Answers the argv prefix `["tea", "pr", "list"]`.
///
/// [`list`]: TeaPr::list
#[derive(Debug, Clone)]
pub struct TeaPr {
    index: u64,
    title: String,
    state: String,
    head: String,
    base: String,
    url: String,
}

impl TeaPr {
    /// An open PR `index` titled `title`, from `feature` into `main`.
    pub fn new(index: u64, title: &str) -> Self {
        Self {
            index,
            title: title.to_string(),
            state: "open".to_string(),
            head: "feature".to_string(),
            base: "main".to_string(),
            url: format!("https://gitea.example.com/octocat/hello-world/pulls/{index}"),
        }
    }

    /// Gitea's lower-case PR state: `"open"`, `"closed"` — or `"merged"`, which
    /// `tea` folds into this one column (there is no separate merged flag, and
    /// `vcs-gitea` derives `merged` from exactly this value).
    pub fn state(mut self, state: &str) -> Self {
        self.state = state.to_string();
        self
    }

    /// Source branch. For a **fork** PR `tea` renders `owner:branch` here —
    /// pass that spelling to exercise the wrapper's owner-stripping.
    pub fn head(mut self, branch: &str) -> Self {
        self.head = branch.to_string();
        self
    }

    /// Target branch.
    pub fn base(mut self, branch: &str) -> Self {
        self.base = branch.to_string();
        self
    }

    /// Web URL.
    pub fn url(mut self, url: &str) -> Self {
        self.url = url.to_string();
        self
    }

    /// The DSV table `tea pr list --output csv` prints in `dialect`. An empty
    /// slice yields the **header-only** table `tea` prints for an empty list
    /// (never an empty string) — which is also what a past-the-end page looks
    /// like to `pr_view`'s pagination.
    pub fn list(dialect: TeaDsv, prs: &[TeaPr]) -> String {
        let rows: Vec<Vec<String>> = prs
            .iter()
            .map(|pr| {
                vec![
                    pr.index.to_string(),
                    pr.title.clone(),
                    pr.state.clone(),
                    pr.head.clone(),
                    pr.base.clone(),
                    pr.url.clone(),
                ]
            })
            .collect();
        render_dsv(
            dialect,
            &["index", "title", "state", "head", "base", "url"],
            &rows,
        )
    }
}

/// An issue row of `tea issues list --output csv` (note `tea`'s plural
/// subcommand).
///
/// As with [`TeaPr`], there is no single-issue view: `tea issues <n>` renders
/// **Markdown** and ignores `--output` entirely, so `vcs-gitea` pages this list
/// instead. Script a one-row [`list`] for an `issue_view` too.
///
/// The columns are the `--fields index,title,state,body,url` the wrapper pins.
///
/// Answers the argv prefix `["tea", "issues", "list"]`.
///
/// [`list`]: TeaIssue::list
#[derive(Debug, Clone)]
pub struct TeaIssue {
    index: u64,
    title: String,
    state: String,
    body: String,
    url: String,
}

impl TeaIssue {
    /// An open issue `index` titled `title`, with an empty body.
    pub fn new(index: u64, title: &str) -> Self {
        Self {
            index,
            title: title.to_string(),
            state: "open".to_string(),
            body: String::new(),
            url: format!("https://gitea.example.com/octocat/hello-world/issues/{index}"),
        }
    }

    /// Gitea's lower-case issue state: `"open"` or `"closed"`.
    pub fn state(mut self, state: &str) -> Self {
        self.state = state.to_string();
        self
    }

    /// Issue body. A multi-line body is quoted and spans physical lines in both
    /// dialects — exactly as `tea` emits it.
    pub fn body(mut self, body: &str) -> Self {
        self.body = body.to_string();
        self
    }

    /// Web URL.
    pub fn url(mut self, url: &str) -> Self {
        self.url = url.to_string();
        self
    }

    /// The DSV table `tea issues list --output csv` prints in `dialect`. An
    /// empty slice yields the header-only table.
    pub fn list(dialect: TeaDsv, issues: &[TeaIssue]) -> String {
        let rows: Vec<Vec<String>> = issues
            .iter()
            .map(|issue| {
                vec![
                    issue.index.to_string(),
                    issue.title.clone(),
                    issue.state.clone(),
                    issue.body.clone(),
                    issue.url.clone(),
                ]
            })
            .collect();
        render_dsv(dialect, &["index", "title", "state", "body", "url"], &rows)
    }
}

/// A release row of `tea releases list --output csv`.
///
/// `tea releases list` has **no `--fields` flag**, so its columns are
/// tea-intrinsic and fixed: `Tag-Name`, `Title`, `Published At`, `Status`,
/// `Tar URL`. `Status` is the single column carrying both the draft and the
/// pre-release marker (`released` / `draft` / `prerelease`), and there is **no
/// release-page URL column at all** — only a tarball URL.
///
/// Answers the argv prefix `["tea", "releases", "list"]`.
#[derive(Debug, Clone)]
pub struct TeaRelease {
    tag: String,
    title: String,
    published_at: String,
    status: String,
    tar_url: String,
}

impl TeaRelease {
    /// A published (`Status` = `released`) release of `tag`, titled `tag`.
    pub fn new(tag: &str) -> Self {
        Self {
            tag: tag.to_string(),
            title: tag.to_string(),
            published_at: PUBLISHED_AT.to_string(),
            status: "released".to_string(),
            tar_url: format!("https://gitea.example.com/octocat/hello-world/archive/{tag}.tar.gz"),
        }
    }

    /// Release title (`Title`).
    pub fn title(mut self, title: &str) -> Self {
        self.title = title.to_string();
        self
    }

    /// Publication timestamp (`Published At`); `tea` leaves this **empty** for
    /// an unpublished draft.
    pub fn published_at(mut self, timestamp: &str) -> Self {
        self.published_at = timestamp.to_string();
        self
    }

    /// Mark the release a draft: `Status` becomes `draft` and `Published At` is
    /// cleared, as `tea` renders an unpublished release.
    pub fn draft(mut self) -> Self {
        self.status = "draft".to_string();
        self.published_at = String::new();
        self
    }

    /// Mark the release a pre-release: `Status` becomes `prerelease`.
    pub fn prerelease(mut self) -> Self {
        self.status = "prerelease".to_string();
        self
    }

    /// Tarball URL (`Tar URL`) — the only URL `tea` reports for a release.
    pub fn tar_url(mut self, url: &str) -> Self {
        self.tar_url = url.to_string();
        self
    }

    /// The DSV table `tea releases list --output csv` prints in `dialect`. An
    /// empty slice yields the header-only table.
    pub fn list(dialect: TeaDsv, releases: &[TeaRelease]) -> String {
        let rows: Vec<Vec<String>> = releases
            .iter()
            .map(|release| {
                vec![
                    release.tag.clone(),
                    release.title.clone(),
                    release.published_at.clone(),
                    release.status.clone(),
                    release.tar_url.clone(),
                ]
            })
            .collect();
        render_dsv(
            dialect,
            &["Tag-Name", "Title", "Published At", "Status", "Tar URL"],
            &rows,
        )
    }
}

/// Render a header plus data rows as `tea`'s `--output csv` table in `dialect`.
/// `tea` prints the header even for an empty list, and terminates every record
/// with a bare `\n` (Go's `fmt.Fprintln` on the naive path, and `encoding/csv`
/// with `UseCRLF` left off on the RFC-4180 path).
fn render_dsv(dialect: TeaDsv, header: &[&str], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    let header: Vec<String> = header.iter().map(|h| (*h).to_string()).collect();
    push_dsv_record(&mut out, dialect, &header);
    for row in rows {
        push_dsv_record(&mut out, dialect, row);
    }
    out
}

fn push_dsv_record(out: &mut String, dialect: TeaDsv, fields: &[String]) {
    let rendered: Vec<String> = fields
        .iter()
        .map(|field| dsv_field(dialect, field))
        .collect();
    out.push_str(&rendered.join(","));
    out.push('\n');
}

/// One DSV cell in `dialect`.
///
/// # Panics
/// In [`TeaDsv::Naive`], if `value` contains a `"`: that dialect escapes
/// nothing, so no faithful wire form exists (a real `tea` 0.9.x would emit
/// corrupt output there — a fixture must not pretend otherwise).
fn dsv_field(dialect: TeaDsv, value: &str) -> String {
    match dialect {
        TeaDsv::Naive => {
            assert!(
                !value.contains('"'),
                "tea's naive DSV dialect (0.9.x-0.13.x) wraps every field in `\"` and escapes \
                 nothing, so a value containing a double quote has no faithful wire form: \
                 {value:?} — build this table with TeaDsv::Rfc4180 (tea 0.14+) instead"
            );
            format!("\"{value}\"")
        }
        TeaDsv::Rfc4180 => {
            if rfc4180_needs_quotes(value) {
                format!("\"{}\"", value.replace('"', "\"\""))
            } else {
                value.to_string()
            }
        }
    }
}

/// Whether Go's `encoding/csv` writer — the one `tea` 0.14+ uses — would quote
/// this field: when it holds the delimiter, a quote or a line break, when it
/// starts with whitespace, or for the `\.` sentinel (`fieldNeedsQuotes`).
fn rfc4180_needs_quotes(field: &str) -> bool {
    if field.is_empty() {
        return false;
    }
    if field == "\\." {
        return true;
    }
    if field.contains([',', '"', '\r', '\n']) {
        return true;
    }
    field.starts_with(char::is_whitespace)
}

// ---------------------------------------------------------------------------
// Shared defaults
// ---------------------------------------------------------------------------

/// Default creation timestamp for `gh`-shaped fixtures (RFC 3339, as `gh` emits).
const CREATED_AT: &str = "2026-01-01T00:00:00Z";
/// Default last-update timestamp for `gh`-shaped fixtures.
const UPDATED_AT: &str = "2026-01-02T00:00:00Z";
/// Default publication timestamp for `gh`/`tea` release fixtures.
const PUBLISHED_AT: &str = "2026-01-03T00:00:00Z";
/// Default project base URL for `glab` fixtures.
const GITLAB_PROJECT: &str = "https://gitlab.example.com/octocat/hello-world";
/// Default creation timestamp for `glab` fixtures — GitLab REST stamps carry
/// milliseconds, unlike `gh`'s.
const GITLAB_CREATED_AT: &str = "2026-01-01T00:00:00.000Z";
/// Default last-update timestamp for `glab` fixtures.
const GITLAB_UPDATED_AT: &str = "2026-01-02T00:00:00.000Z";
/// Default `released_at` for `glab` release fixtures.
const GITLAB_RELEASED_AT: &str = "2026-01-03T00:00:00.000Z";

fn to_owned(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The JSON writer is hand-rolled (this crate has no serde), so the escaping
    // it produces is a contract of its own: a quote, a backslash, a newline and
    // a bare control character must all come out as valid JSON escapes.
    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_string("plain"), "\"plain\"");
        assert_eq!(json_string("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(json_string("a\\b"), "\"a\\\\b\"");
        assert_eq!(json_string("line1\nline2"), "\"line1\\nline2\"");
        assert_eq!(json_string("bell\u{7}"), "\"bell\\u0007\"");
    }

    // `gh` prints the requested fields in alphabetical key order, not in the
    // order `--json` asked for them.
    #[test]
    fn gh_pr_keys_are_alphabetical() {
        let json = GhPr::new(12, "Add feature").view();
        assert!(
            json.starts_with(
                "{\"assignees\":[],\"author\":{\"login\":\"octocat\"},\"baseRefName\":\"main\","
            ),
            "{json}"
        );
        assert!(json.ends_with("\"url\":\"https://github.com/octocat/hello-world/pull/12\"}"));
    }

    // An empty author login is `gh`'s `null` author (a deleted account), and an
    // unset milestone is a present `null` — not an absent key.
    #[test]
    fn gh_pr_models_null_author_and_milestone() {
        let json = GhPr::new(1, "t").author("").view();
        assert!(json.contains("\"author\":null"), "{json}");
        assert!(json.contains("\"milestone\":null"), "{json}");
    }

    // `gh`'s nested label/assignee objects vs GitLab's flat label strings — the
    // single most copy-paste-prone difference between the two JSON forges.
    #[test]
    fn label_shapes_differ_between_gh_and_glab() {
        let gh = GhPr::new(1, "t")
            .labels(&["bug"])
            .assignees(&["hubot"])
            .view();
        assert!(gh.contains("\"labels\":[{\"name\":\"bug\"}]"), "{gh}");
        assert!(gh.contains("\"assignees\":[{\"login\":\"hubot\"}]"), "{gh}");

        let glab = GlabMr::new(1, "t")
            .labels(&["bug"])
            .assignees(&["hubot"])
            .view();
        assert!(glab.contains("\"labels\":[\"bug\"]"), "{glab}");
        assert!(
            glab.contains("\"assignees\":[{\"username\":\"hubot\"}]"),
            "{glab}"
        );
    }

    // `gh release list` and `gh release view` request genuinely different field
    // sets; the builder must not blur them (see the type docs).
    #[test]
    fn gh_release_list_and_view_field_sets_diverge() {
        let release = GhRelease::new("v1.0.0").latest(true).body("notes");

        let view = release.view();
        assert!(view.contains("\"body\":\"notes\""), "{view}");
        assert!(view.contains("\"author\":"), "{view}");
        assert!(view.contains("\"url\":"), "{view}");
        assert!(!view.contains("isLatest"), "view has no isLatest: {view}");

        let list = GhRelease::list(&[release]);
        assert!(list.contains("\"isLatest\":true"), "{list}");
        assert!(!list.contains("\"body\""), "list has no body: {list}");
        assert!(!list.contains("\"author\""), "list has no author: {list}");
        assert!(!list.contains("\"url\""), "list has no url: {list}");
    }

    // A GitLab release carries its page URL only as the nested `_links.self`.
    #[test]
    fn glab_release_url_is_nested_under_links_self() {
        let json = GlabRelease::new("v1.0.0")
            .url("https://gl/releases/v1")
            .view();
        assert!(
            json.contains("\"_links\":{\"self\":\"https://gl/releases/v1\"}"),
            "{json}"
        );
        assert!(!json.contains("\"web_url\""), "{json}");
    }

    // An empty list is `[]` for the JSON forges but a header-only table for tea
    // — the shape `pr_view`'s pagination reads as "walked past the end".
    #[test]
    fn empty_lists_keep_each_cli_shape() {
        assert_eq!(GhPr::list(&[]), "[]");
        assert_eq!(GlabMr::list(&[]), "[]");
        for dialect in TeaDsv::ALL {
            let table = TeaPr::list(*dialect, &[]);
            assert_eq!(table.lines().count(), 1, "header only: {table:?}");
            assert!(table.ends_with('\n'), "{table:?}");
        }
    }

    // The two tea dialects differ exactly in quoting: naive quotes everything,
    // RFC 4180 quotes only what needs it.
    #[test]
    fn tea_dialects_quote_differently() {
        let prs = [TeaPr::new(7, "Add X").url("u")];

        let naive = TeaPr::list(TeaDsv::Naive, &prs);
        assert_eq!(
            naive,
            "\"index\",\"title\",\"state\",\"head\",\"base\",\"url\"\n\
             \"7\",\"Add X\",\"open\",\"feature\",\"main\",\"u\"\n"
        );

        let rfc = TeaPr::list(TeaDsv::Rfc4180, &prs);
        assert_eq!(
            rfc,
            "index,title,state,head,base,url\n7,Add X,open,feature,main,u\n"
        );
    }

    // RFC 4180 quotes a cell holding the delimiter, a quote, or a line break,
    // and doubles an embedded quote (Go's `encoding/csv` rules).
    #[test]
    fn rfc4180_quotes_only_what_needs_it() {
        assert_eq!(dsv_field(TeaDsv::Rfc4180, "plain"), "plain");
        assert_eq!(dsv_field(TeaDsv::Rfc4180, ""), "");
        assert_eq!(dsv_field(TeaDsv::Rfc4180, "a,b"), "\"a,b\"");
        assert_eq!(
            dsv_field(TeaDsv::Rfc4180, "say \"hi\""),
            "\"say \"\"hi\"\"\""
        );
        assert_eq!(dsv_field(TeaDsv::Rfc4180, "l1\nl2"), "\"l1\nl2\"");
        assert_eq!(dsv_field(TeaDsv::Rfc4180, " lead"), "\" lead\"");
    }

    // The naive dialect cannot escape a quote, so a fixture that would need one
    // fails loudly at the call site instead of emitting output no `tea` prints.
    #[test]
    #[should_panic(expected = "no faithful wire form")]
    fn naive_dialect_rejects_an_unrepresentable_quote() {
        TeaIssue::list(TeaDsv::Naive, &[TeaIssue::new(1, "say \"hi\"")]);
    }

    // A multi-line body is a quoted field spanning physical lines in both
    // dialects — the shape the wrapper's DSV reader is built to rejoin.
    #[test]
    fn tea_multiline_body_stays_one_quoted_field() {
        for dialect in TeaDsv::ALL {
            let table = TeaIssue::list(*dialect, &[TeaIssue::new(12, "Bug").body("l1\nl2")]);
            assert!(table.contains("\"l1\nl2\""), "{dialect:?}: {table:?}");
        }
    }

    // tea's release table is a fixed five-column shape with no `--fields` pin,
    // and a draft carries an empty `Published At`.
    #[test]
    fn tea_release_table_is_the_fixed_five_column_shape() {
        let table = TeaRelease::list(TeaDsv::Rfc4180, &[TeaRelease::new("v2").draft()]);
        let mut lines = table.lines();
        assert_eq!(
            lines.next().unwrap(),
            "Tag-Name,Title,Published At,Status,Tar URL"
        );
        let row: Vec<&str> = lines.next().unwrap().split(',').collect();
        assert_eq!(row.len(), 5, "{table:?}");
        assert_eq!(row[2], "", "a draft has no publication timestamp");
        assert_eq!(row[3], "draft");
    }
}
