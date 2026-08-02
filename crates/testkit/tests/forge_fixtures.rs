//! Pins every [`vcs_testkit::forge_fixtures`] builder against the **real**
//! parser of the wrapper crate it models.
//!
//! Each fixture is handed to a [`ScriptedRunner`] as the CLI's stdout and read
//! back through the actual `GitHubApi` / `GitLabApi` / `GiteaApi` method a
//! consumer would call — never through a re-implemented copy of the parsing. So
//! a fixture cannot quietly drift from the shape its wrapper expects: if `gh`,
//! `glab` or `tea` changes an output contract and the wrapper follows, the
//! fixture that still claims the old shape fails here.
//!
//! Parsing alone would still miss a *widened* request: a wrapper that starts
//! asking `gh` for one more `--json` field parses an old fixture happily (the
//! new field is simply absent), while the real CLI would have printed it. So the
//! "field-set pins" at the bottom also capture the argv each wrapper builds and
//! require the fixture to answer **every** field it requested.
//!
//! The three wrapper crates are `[dev-dependencies]` of `vcs-testkit` — the
//! published library keeps its "no dependencies at all" property, so all three
//! (and every other workspace crate) can keep dev-depending on `vcs-testkit`
//! themselves.

use std::path::Path;

use processkit::testing::{RecordingRunner, Reply, ScriptedRunner};
use vcs_testkit::forge_fixtures::{
    GhIssue, GhPr, GhRelease, GlabIssue, GlabMr, GlabRelease, TeaDsv, TeaIssue, TeaPr, TeaRelease,
};

/// The directory the wrapper clients are pointed at. Nothing ever spawns, so it
/// need not exist.
fn dir() -> &'static Path {
    Path::new("/repo")
}

fn gh(argv: [&str; 3], stdout: String) -> vcs_github::GitHub<ScriptedRunner> {
    vcs_github::GitHub::with_runner(ScriptedRunner::new().on(argv, Reply::ok(stdout)))
}

fn glab(argv: [&str; 3], stdout: String) -> vcs_gitlab::GitLab<ScriptedRunner> {
    vcs_gitlab::GitLab::with_runner(ScriptedRunner::new().on(argv, Reply::ok(stdout)))
}

fn tea(argv: [&str; 3], stdout: String) -> vcs_gitea::Gitea<ScriptedRunner> {
    vcs_gitea::Gitea::with_runner(ScriptedRunner::new().on(argv, Reply::ok(stdout)))
}

// ---------------------------------------------------------------------------
// GitHub — `gh … --json <fields>`
// ---------------------------------------------------------------------------

#[tokio::test]
async fn gh_pr_view_fixture_parses_every_field() {
    use vcs_github::GitHubApi;

    let fixture = GhPr::new(12, "Add feature")
        .state("MERGED")
        .draft(true)
        .head("feat/x")
        .base("release/1")
        .url("https://github.com/octocat/hello-world/pull/12")
        .labels(&["bug", "priority-1"])
        .assignees(&["octocat", "hubot"])
        .author("steiza")
        .timestamps("2026-07-01T00:00:00Z", "2026-07-02T00:00:00Z")
        .milestone("v1.0");

    let pr = gh(["gh", "pr", "view"], fixture.view())
        .pr_view(dir(), 12)
        .await
        .expect("the fixture must parse with vcs-github's own parser");

    assert_eq!(pr.number, 12);
    assert_eq!(pr.title, "Add feature");
    assert_eq!(pr.state, "MERGED");
    assert!(pr.is_draft);
    assert_eq!(pr.head_ref_name, "feat/x");
    assert_eq!(pr.base_ref_name, "release/1");
    assert_eq!(pr.url, "https://github.com/octocat/hello-world/pull/12");
    assert_eq!(pr.labels, ["bug", "priority-1"]);
    assert_eq!(pr.assignees, ["octocat", "hubot"]);
    assert_eq!(pr.author, "steiza");
    assert_eq!(pr.created_at, "2026-07-01T00:00:00Z");
    assert_eq!(pr.updated_at, "2026-07-02T00:00:00Z");
    assert_eq!(pr.milestone.as_deref(), Some("v1.0"));
}

// The defaults alone must be a valid payload — the point of a canonical fixture
// is that a test states only what it cares about.
#[tokio::test]
async fn gh_pr_defaults_and_list_parse() {
    use vcs_github::GitHubApi;

    let prs = gh(
        ["gh", "pr", "list"],
        GhPr::list(&[GhPr::new(1, "One"), GhPr::new(2, "Two").draft(true)]),
    )
    .pr_list(dir())
    .await
    .expect("list fixture parses");

    assert_eq!(prs.len(), 2);
    assert_eq!(prs[0].number, 1);
    assert_eq!(prs[0].state, "OPEN");
    assert_eq!(prs[0].base_ref_name, "main");
    assert!(prs[0].milestone.is_none());
    assert!(prs[1].is_draft);

    let empty = gh(["gh", "pr", "list"], GhPr::list(&[]))
        .pr_list(dir())
        .await
        .expect("empty list fixture parses");
    assert!(empty.is_empty());
}

// A deleted account is `"author": null` on the wire; the wrapper flattens it to
// an empty login rather than failing the parse.
#[tokio::test]
async fn gh_pr_null_author_fixture_parses_to_an_empty_login() {
    use vcs_github::GitHubApi;

    let pr = gh(
        ["gh", "pr", "view"],
        GhPr::new(3, "Ghosted").author("").view(),
    )
    .pr_view(dir(), 3)
    .await
    .expect("null-author fixture parses");
    assert_eq!(pr.author, "");
}

#[tokio::test]
async fn gh_issue_view_and_list_fixtures_parse() {
    use vcs_github::GitHubApi;

    let fixture = GhIssue::new(3, "Docs")
        .state("CLOSED")
        .body("Write them.\n\n- and them")
        .url("https://github.com/octocat/hello-world/issues/3")
        .labels(&["docs"])
        .assignees(&["andyfeller"])
        .author("andyfeller")
        .milestone("v1.0");

    let issue = gh(["gh", "issue", "view"], fixture.view())
        .issue_view(dir(), 3)
        .await
        .expect("issue view fixture parses");
    assert_eq!(issue.number, 3);
    assert_eq!(issue.state, "CLOSED");
    assert_eq!(issue.body, "Write them.\n\n- and them");
    assert_eq!(issue.labels, ["docs"]);
    assert_eq!(issue.assignees, ["andyfeller"]);
    assert_eq!(issue.milestone.as_deref(), Some("v1.0"));

    let issues = gh(
        ["gh", "issue", "list"],
        GhIssue::list(&[GhIssue::new(4, "Four")]),
    )
    .issue_list(dir())
    .await
    .expect("issue list fixture parses");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].number, 4);
    assert_eq!(issues[0].state, "OPEN");
}

// `gh release list` and `gh release view` carry genuinely different field sets;
// the wrapper models the absent ones as `None` ("not fetched"), so the two
// fixture shapes must land on opposite sides of that distinction.
#[tokio::test]
async fn gh_release_fixtures_keep_the_list_view_field_split() {
    use vcs_github::GitHubApi;

    let fixture = GhRelease::new("vcs-testkit-v0.8.0")
        .name("vcs-testkit v0.8.0")
        .body("### Added\n- forge fixtures")
        .url("https://github.com/octocat/hello-world/releases/tag/v0.8.0")
        .published_at("2026-07-19T13:14:42Z")
        .latest(true)
        .author("github-actions[bot]");

    let viewed = gh(["gh", "release", "view"], fixture.view())
        .release_view(dir(), "vcs-testkit-v0.8.0")
        .await
        .expect("release view fixture parses");
    assert_eq!(viewed.tag_name, "vcs-testkit-v0.8.0");
    assert_eq!(viewed.body.as_deref(), Some("### Added\n- forge fixtures"));
    assert_eq!(
        viewed.url.as_deref(),
        Some("https://github.com/octocat/hello-world/releases/tag/v0.8.0")
    );
    assert_eq!(viewed.author.as_deref(), Some("github-actions[bot]"));
    assert!(
        !viewed.is_latest,
        "`gh release view` has no isLatest field to report"
    );

    let listed = gh(["gh", "release", "list"], GhRelease::list(&[fixture]))
        .release_list(dir())
        .await
        .expect("release list fixture parses");
    assert_eq!(listed.len(), 1);
    assert!(listed[0].is_latest);
    assert_eq!(listed[0].body, None, "list does not fetch the body");
    assert_eq!(listed[0].url, None, "list does not fetch the url");
    assert_eq!(listed[0].author, None, "list does not fetch the author");
}

// ---------------------------------------------------------------------------
// GitLab — `glab … --output json` (GitLab REST, passed through verbatim)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn glab_mr_view_fixture_parses_every_field() {
    use vcs_gitlab::GitLabApi;

    let fixture = GlabMr::new(12, "Add feature")
        .state("merged")
        .draft(true)
        .source_branch("feat/x")
        .target_branch("release/1")
        .web_url("https://gitlab.example.com/octocat/hello-world/-/merge_requests/12")
        .labels(&["bug", "priority-1"])
        .assignees(&["octocat", "hubot"])
        .author("steiza")
        .timestamps("2026-07-01T00:00:00.000Z", "2026-07-02T00:00:00.000Z")
        .milestone("v1.0");

    let mr = glab(["glab", "mr", "view"], fixture.view())
        .mr_view(dir(), 12)
        .await
        .expect("the fixture must parse with vcs-gitlab's own parser");

    assert_eq!(mr.iid, 12);
    assert_eq!(mr.title, "Add feature");
    assert_eq!(mr.state, "merged");
    assert!(mr.draft);
    assert_eq!(mr.source_branch, "feat/x");
    assert_eq!(mr.target_branch, "release/1");
    assert_eq!(
        mr.web_url,
        "https://gitlab.example.com/octocat/hello-world/-/merge_requests/12"
    );
    assert_eq!(mr.labels, ["bug", "priority-1"]);
    assert_eq!(mr.assignees, ["octocat", "hubot"]);
    assert_eq!(mr.author, "steiza");
    assert_eq!(mr.created_at, "2026-07-01T00:00:00.000Z");
    assert_eq!(mr.updated_at, "2026-07-02T00:00:00.000Z");
    assert_eq!(mr.milestone.as_deref(), Some("v1.0"));
}

#[tokio::test]
async fn glab_mr_defaults_and_list_parse() {
    use vcs_gitlab::GitLabApi;

    let mrs = glab(
        ["glab", "mr", "list"],
        GlabMr::list(&[GlabMr::new(1, "One"), GlabMr::new(2, "Two")]),
    )
    .mr_list(dir())
    .await
    .expect("list fixture parses");

    assert_eq!(mrs.len(), 2);
    assert_eq!(
        mrs[0].state, "opened",
        "GitLab spells the open state `opened`"
    );
    assert_eq!(mrs[0].target_branch, "main");
    assert!(mrs[0].milestone.is_none());

    let empty = glab(["glab", "mr", "list"], GlabMr::list(&[]))
        .mr_list(dir())
        .await
        .expect("empty list fixture parses");
    assert!(empty.is_empty());
}

#[tokio::test]
async fn glab_issue_view_and_list_fixtures_parse() {
    use vcs_gitlab::GitLabApi;

    let fixture = GlabIssue::new(7, "Docs")
        .state("closed")
        .body("Write them.\n\n- and them")
        .web_url("https://gitlab.example.com/octocat/hello-world/-/issues/7")
        .labels(&["docs"])
        .assignees(&["andyfeller"])
        .author("andyfeller")
        .milestone("v1.0");

    let issue = glab(["glab", "issue", "view"], fixture.view())
        .issue_view(dir(), 7)
        .await
        .expect("issue view fixture parses");
    // The wrapper surfaces GitLab's `iid` as `number`, `description` as `body`
    // and `web_url` as `url` — the fixture must use the REST spellings.
    assert_eq!(issue.number, 7);
    assert_eq!(issue.state, "closed");
    assert_eq!(issue.body, "Write them.\n\n- and them");
    assert_eq!(
        issue.url,
        "https://gitlab.example.com/octocat/hello-world/-/issues/7"
    );
    assert_eq!(issue.labels, ["docs"]);
    assert_eq!(issue.assignees, ["andyfeller"]);
    assert_eq!(issue.milestone.as_deref(), Some("v1.0"));

    let issues = glab(
        ["glab", "issue", "list"],
        GlabIssue::list(&[GlabIssue::new(8, "Eight")]),
    )
    .issue_list(dir())
    .await
    .expect("issue list fixture parses");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].number, 8);
    assert_eq!(issues[0].state, "opened");
}

// A GitLab release has no top-level web URL — the wrapper reads it out of the
// nested `_links.self`, which is exactly where the fixture puts it.
#[tokio::test]
async fn glab_release_fixture_parses_including_the_nested_self_link() {
    use vcs_gitlab::GitLabApi;

    let fixture = GlabRelease::new("v1.0.0")
        .name("One")
        .description("### Added\n- things")
        .released_at("2026-07-19T13:14:42.000Z")
        .url("https://gitlab.example.com/octocat/hello-world/-/releases/v1.0.0")
        .author("steiza");

    let release = glab(["glab", "release", "view"], fixture.view())
        .release_view(dir(), "v1.0.0")
        .await
        .expect("release view fixture parses");
    assert_eq!(release.tag_name, "v1.0.0");
    assert_eq!(release.name, "One");
    assert_eq!(release.description, "### Added\n- things");
    assert_eq!(release.published_at, "2026-07-19T13:14:42.000Z");
    assert_eq!(
        release.url,
        "https://gitlab.example.com/octocat/hello-world/-/releases/v1.0.0"
    );
    assert_eq!(release.author, "steiza");

    let releases = glab(
        ["glab", "release", "list"],
        GlabRelease::list(&[GlabRelease::new("v0.9.0")]),
    )
    .release_list(dir())
    .await
    .expect("release list fixture parses");
    assert_eq!(releases.len(), 1);
    assert_eq!(releases[0].tag_name, "v0.9.0");
}

// ---------------------------------------------------------------------------
// Gitea — `tea … --output csv` (a quoted DSV table, two wire dialects)
// ---------------------------------------------------------------------------

// Both dialects must parse identically through the wrapper's single reader —
// that is the whole point of offering both.
#[tokio::test]
async fn tea_pr_list_fixture_parses_in_both_dsv_dialects() {
    use vcs_gitea::GiteaApi;

    for dialect in TeaDsv::ALL {
        let fixture = TeaPr::list(
            *dialect,
            &[
                TeaPr::new(7, "Add X").head("feat/x").url("https://gitea/7"),
                TeaPr::new(9, "Done").state("merged"),
            ],
        );
        let prs = tea(["tea", "pr", "list"], fixture)
            .pr_list(dir())
            .await
            .unwrap_or_else(|e| panic!("{dialect:?} fixture must parse with vcs-gitea: {e}"));

        assert_eq!(prs.len(), 2, "{dialect:?}");
        assert_eq!(prs[0].number, 7, "{dialect:?}");
        assert_eq!(prs[0].title, "Add X", "{dialect:?}");
        assert_eq!(prs[0].state, "open", "{dialect:?}");
        assert!(!prs[0].merged, "{dialect:?}");
        assert_eq!(prs[0].head_branch, "feat/x", "{dialect:?}");
        assert_eq!(prs[0].base_branch, "main", "{dialect:?}");
        assert_eq!(prs[0].url, "https://gitea/7", "{dialect:?}");
        // tea folds the merge flag into the state column; the wrapper derives
        // `merged` from it.
        assert_eq!(prs[1].state, "merged", "{dialect:?}");
        assert!(prs[1].merged, "{dialect:?}");
    }
}

// A value that a real `tea` 0.14+ would have to quote (a comma, an embedded
// quote) must survive the round trip in the RFC-4180 dialect.
#[tokio::test]
async fn tea_rfc4180_fixture_round_trips_quoted_values() {
    use vcs_gitea::GiteaApi;

    let fixture = TeaPr::list(
        TeaDsv::Rfc4180,
        &[TeaPr::new(3, "Fix a, b and \"c\"").head("feat/comma,branch")],
    );
    let prs = tea(["tea", "pr", "list"], fixture)
        .pr_list(dir())
        .await
        .expect("quoted values parse");
    assert_eq!(prs[0].title, "Fix a, b and \"c\"");
    assert_eq!(prs[0].head_branch, "feat/comma,branch");
}

// `tea` renders a fork PR's head as `owner:branch`; the wrapper strips the owner
// back to a flat branch. Passing that spelling through the fixture is how a
// consumer exercises it.
#[tokio::test]
async fn tea_fork_head_fixture_reaches_the_owner_stripping() {
    use vcs_gitea::GiteaApi;

    let fixture = TeaPr::list(
        TeaDsv::Naive,
        &[TeaPr::new(8, "From a fork").head("alice:feature")],
    );
    let prs = tea(["tea", "pr", "list"], fixture)
        .pr_list(dir())
        .await
        .expect("fork-head fixture parses");
    assert_eq!(prs[0].head_branch, "feature");
}

// `tea` has no single-PR view, so the wrapper synthesizes one by paging the
// list — a one-row list fixture is what a `pr_view` test scripts.
#[tokio::test]
async fn tea_pr_view_is_served_by_a_list_fixture() {
    use vcs_gitea::GiteaApi;

    let fixture = TeaPr::list(TeaDsv::Rfc4180, &[TeaPr::new(9, "Nine").state("merged")]);
    let pr = tea(["tea", "pr", "list"], fixture)
        .pr_view(dir(), 9)
        .await
        .expect("pr_view pages the same list fixture");
    assert_eq!(pr.number, 9);
    assert!(pr.merged);
}

// The header-only table an empty slice produces is what `tea` prints for an
// empty list — and what the wrapper's pagination reads as "past the last page",
// i.e. a confirmed absence rather than format drift.
#[tokio::test]
async fn tea_empty_list_fixture_is_a_header_only_table() {
    use vcs_gitea::GiteaApi;

    for dialect in TeaDsv::ALL {
        let prs = tea(["tea", "pr", "list"], TeaPr::list(*dialect, &[]))
            .pr_list(dir())
            .await
            .unwrap_or_else(|e| panic!("{dialect:?} empty table must parse: {e}"));
        assert!(prs.is_empty(), "{dialect:?}");

        let err = tea(["tea", "pr", "list"], TeaPr::list(*dialect, &[]))
            .pr_view(dir(), 1)
            .await
            .expect_err("an empty page means the PR is absent");
        assert!(
            vcs_gitea::is_view_absence(&err),
            "{dialect:?}: an empty page is a confirmed absence, not format drift: {err}"
        );
    }
}

#[tokio::test]
async fn tea_issue_fixtures_parse_in_both_dialects() {
    use vcs_gitea::GiteaApi;

    for dialect in TeaDsv::ALL {
        // A multi-line body is a quoted field spanning physical lines in both
        // dialects — the reader must rejoin it, not split a bogus row off it.
        let fixture = TeaIssue::list(
            *dialect,
            &[TeaIssue::new(12, "Bug")
                .body("line1\nline2")
                .url("https://gitea/issues/12")],
        );
        let issues = tea(["tea", "issues", "list"], fixture)
            .issue_list(dir())
            .await
            .unwrap_or_else(|e| panic!("{dialect:?} issue fixture must parse: {e}"));

        assert_eq!(issues.len(), 1, "{dialect:?}");
        assert_eq!(issues[0].number, 12, "{dialect:?}");
        assert_eq!(issues[0].title, "Bug", "{dialect:?}");
        assert_eq!(issues[0].state, "open", "{dialect:?}");
        assert_eq!(issues[0].body, "line1\nline2", "{dialect:?}");
        assert_eq!(issues[0].url, "https://gitea/issues/12", "{dialect:?}");

        // `tea issues <n>` renders Markdown and ignores `--output`, so the
        // wrapper pages the list here too.
        let issue = tea(
            ["tea", "issues", "list"],
            TeaIssue::list(*dialect, &[TeaIssue::new(6, "Six").state("closed")]),
        )
        .issue_view(dir(), 6)
        .await
        .unwrap_or_else(|e| panic!("{dialect:?} issue_view pages the list: {e}"));
        assert_eq!(issue.number, 6, "{dialect:?}");
        assert_eq!(issue.state, "closed", "{dialect:?}");
    }
}

// tea's release table has no `--fields` pin, so its five columns are positional
// and tea-intrinsic; `Status` alone carries both the draft and prerelease
// markers, and there is no release-page URL at all.
#[tokio::test]
async fn tea_release_list_fixture_parses_status_and_missing_url() {
    use vcs_gitea::GiteaApi;

    for dialect in TeaDsv::ALL {
        let fixture = TeaRelease::list(
            *dialect,
            &[
                TeaRelease::new("0.1")
                    .title("First")
                    .published_at("2023-07-26T13:02:36Z"),
                TeaRelease::new("v2").title("Two").draft(),
                TeaRelease::new("v3-rc1").title("RC").prerelease(),
            ],
        );
        let releases = tea(["tea", "releases", "list"], fixture)
            .release_list(dir())
            .await
            .unwrap_or_else(|e| panic!("{dialect:?} release fixture must parse: {e}"));

        assert_eq!(releases.len(), 3, "{dialect:?}");
        assert_eq!(releases[0].tag, "0.1", "{dialect:?}");
        assert_eq!(releases[0].title, "First", "{dialect:?}");
        assert_eq!(
            releases[0].published_at, "2023-07-26T13:02:36Z",
            "{dialect:?}"
        );
        assert!(!releases[0].draft, "{dialect:?}");
        assert!(!releases[0].prerelease, "{dialect:?}");
        assert_eq!(
            releases[0].url, "",
            "{dialect:?}: tea exposes no release-page URL"
        );

        assert!(releases[1].draft, "{dialect:?}");
        assert_eq!(
            releases[1].published_at, "",
            "{dialect:?}: a draft is unpublished"
        );
        assert!(releases[2].prerelease, "{dialect:?}");
    }
}

// ---------------------------------------------------------------------------
// Field-set pins — a fixture must answer everything its wrapper ASKS the CLI for
// ---------------------------------------------------------------------------

/// The comma-separated list the wrapper passed to `flag` in the argv it actually
/// built (`gh --json <fields>`, `tea --fields <fields>`).
fn requested_fields(args: &[String], flag: &str) -> Vec<String> {
    let at = args
        .iter()
        .position(|arg| arg == flag)
        .unwrap_or_else(|| panic!("the wrapper's argv carries `{flag}`: {args:?}"));
    let fields: Vec<String> = args[at + 1].split(',').map(str::to_string).collect();
    // Guard against a vacuous pin: an empty list would make every check below
    // pass without comparing anything.
    assert!(
        fields.iter().all(|field| !field.is_empty()),
        "`{flag}` carries a non-empty field list: {args:?}"
    );
    fields
}

/// Every field the wrapper asked `gh` for must be a key of the fixture. Parsing
/// cannot catch this on its own: an unknown-to-the-fixture field just parses as
/// absent, so a widened `--json` list would leave the fixture quietly modelling
/// a payload the real `gh` no longer prints.
fn assert_answers_every_json_field(fixture: &str, args: &[String], what: &str) {
    for field in requested_fields(args, "--json") {
        assert!(
            fixture.contains(&format!("\"{field}\":")),
            "{what}: the fixture has no `{field}` key, but the wrapper asks gh for it — \
             widen the fixture (or drop the field) so consumers keep testing against \
             what gh really prints: {fixture}"
        );
    }
}

#[tokio::test]
async fn gh_fixtures_answer_every_field_the_wrapper_requests() {
    use vcs_github::GitHubApi;

    // `pr view` and `pr list` share one field list in the wrapper; both are the
    // same fixture object, so covering one covers the other.
    {
        let fixture = GhPr::new(12, "Add feature").view();
        let rec = RecordingRunner::new(
            ScriptedRunner::new().on(["gh", "pr", "view"], Reply::ok(fixture.clone())),
        );
        vcs_github::GitHub::with_runner(&rec)
            .pr_view(dir(), 12)
            .await
            .expect("pr view fixture parses");
        assert_answers_every_json_field(&fixture, &rec.only_call().args_str(), "gh pr view");
    }
    {
        let fixture = GhPr::list(&[GhPr::new(12, "Add feature")]);
        let rec = RecordingRunner::new(
            ScriptedRunner::new().on(["gh", "pr", "list"], Reply::ok(fixture.clone())),
        );
        vcs_github::GitHub::with_runner(&rec)
            .pr_list(dir())
            .await
            .expect("pr list fixture parses");
        assert_answers_every_json_field(&fixture, &rec.only_call().args_str(), "gh pr list");
    }
    {
        let fixture = GhIssue::new(3, "Docs").view();
        let rec = RecordingRunner::new(
            ScriptedRunner::new().on(["gh", "issue", "view"], Reply::ok(fixture.clone())),
        );
        vcs_github::GitHub::with_runner(&rec)
            .issue_view(dir(), 3)
            .await
            .expect("issue view fixture parses");
        assert_answers_every_json_field(&fixture, &rec.only_call().args_str(), "gh issue view");
    }
    {
        let fixture = GhIssue::list(&[GhIssue::new(3, "Docs")]);
        let rec = RecordingRunner::new(
            ScriptedRunner::new().on(["gh", "issue", "list"], Reply::ok(fixture.clone())),
        );
        vcs_github::GitHub::with_runner(&rec)
            .issue_list(dir())
            .await
            .expect("issue list fixture parses");
        assert_answers_every_json_field(&fixture, &rec.only_call().args_str(), "gh issue list");
    }
    // The release pair is where this pin earns its keep: the two subcommands
    // request genuinely different field sets, so each fixture shape is checked
    // against the field list of *its own* subcommand.
    {
        let fixture = GhRelease::new("v1.0.0").view();
        let rec = RecordingRunner::new(
            ScriptedRunner::new().on(["gh", "release", "view"], Reply::ok(fixture.clone())),
        );
        vcs_github::GitHub::with_runner(&rec)
            .release_view(dir(), "v1.0.0")
            .await
            .expect("release view fixture parses");
        assert_answers_every_json_field(&fixture, &rec.only_call().args_str(), "gh release view");
    }
    {
        let fixture = GhRelease::list(&[GhRelease::new("v1.0.0")]);
        let rec = RecordingRunner::new(
            ScriptedRunner::new().on(["gh", "release", "list"], Reply::ok(fixture.clone())),
        );
        vcs_github::GitHub::with_runner(&rec)
            .release_list(dir())
            .await
            .expect("release list fixture parses");
        assert_answers_every_json_field(&fixture, &rec.only_call().args_str(), "gh release list");
    }
}

// tea's columns are POSITIONAL: a fixture whose header drifted from the
// `--fields` the wrapper pins would still parse, silently mapping values to the
// wrong fields. So compare the whole column list, in order — not just presence.
#[tokio::test]
async fn tea_fixture_columns_are_exactly_the_fields_the_wrapper_requests() {
    use vcs_gitea::GiteaApi;

    /// The fixture's header row, unquoted — both dialects quote it differently.
    fn header(table: &str) -> Vec<String> {
        table
            .lines()
            .next()
            .expect("tea prints a header even for an empty list")
            .split(',')
            .map(|cell| cell.trim_matches('"').to_string())
            .collect()
    }

    for dialect in TeaDsv::ALL {
        let fixture = TeaPr::list(*dialect, &[TeaPr::new(7, "Add X")]);
        let rec = RecordingRunner::new(
            ScriptedRunner::new().on(["tea", "pr", "list"], Reply::ok(fixture.clone())),
        );
        vcs_gitea::Gitea::with_runner(&rec)
            .pr_list(dir())
            .await
            .unwrap_or_else(|e| panic!("{dialect:?} pr fixture parses: {e}"));
        let args = rec.only_call().args_str();
        assert_eq!(
            header(&fixture),
            requested_fields(&args, "--fields"),
            "{dialect:?}: tea's columns are positional — the fixture's header must be \
             exactly the `--fields` the wrapper pins, in order"
        );

        let fixture = TeaIssue::list(*dialect, &[TeaIssue::new(12, "Bug")]);
        let rec = RecordingRunner::new(
            ScriptedRunner::new().on(["tea", "issues", "list"], Reply::ok(fixture.clone())),
        );
        vcs_gitea::Gitea::with_runner(&rec)
            .issue_list(dir())
            .await
            .unwrap_or_else(|e| panic!("{dialect:?} issue fixture parses: {e}"));
        let args = rec.only_call().args_str();
        assert_eq!(
            header(&fixture),
            requested_fields(&args, "--fields"),
            "{dialect:?}: the issue table's columns must match the pinned `--fields`"
        );
    }
}

// `glab` builds no field list (it forwards GitLab's REST object as-is), so the
// pin here is the *format* the wrapper asks for: JSON, not glab's default table.
#[tokio::test]
async fn glab_fixtures_answer_the_output_mode_the_wrapper_requests() {
    use vcs_gitlab::GitLabApi;

    fn asks_for_json(args: &[String]) -> bool {
        args.windows(2)
            .any(|pair| pair[0] == "--output" && pair[1] == "json")
    }

    {
        let rec = RecordingRunner::new(ScriptedRunner::new().on(
            ["glab", "mr", "list"],
            Reply::ok(GlabMr::list(&[GlabMr::new(1, "One")])),
        ));
        vcs_gitlab::GitLab::with_runner(&rec)
            .mr_list(dir())
            .await
            .expect("mr list fixture parses");
        let args = rec.only_call().args_str();
        assert!(asks_for_json(&args), "glab mr list asks for JSON: {args:?}");
    }
    {
        let rec = RecordingRunner::new(ScriptedRunner::new().on(
            ["glab", "issue", "list"],
            Reply::ok(GlabIssue::list(&[GlabIssue::new(1, "One")])),
        ));
        vcs_gitlab::GitLab::with_runner(&rec)
            .issue_list(dir())
            .await
            .expect("issue list fixture parses");
        let args = rec.only_call().args_str();
        assert!(
            asks_for_json(&args),
            "glab issue list asks for JSON: {args:?}"
        );
    }
    {
        let rec = RecordingRunner::new(ScriptedRunner::new().on(
            ["glab", "release", "list"],
            Reply::ok(GlabRelease::list(&[GlabRelease::new("v1.0.0")])),
        ));
        vcs_gitlab::GitLab::with_runner(&rec)
            .release_list(dir())
            .await
            .expect("release list fixture parses");
        let args = rec.only_call().args_str();
        assert!(
            asks_for_json(&args),
            "glab release list asks for JSON: {args:?}"
        );
    }
}
