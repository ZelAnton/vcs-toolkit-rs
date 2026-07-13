//! Live end-to-end PR/issue/release lifecycle for the **Gitea** backend of the
//! `vcs-forge` facade (and, through it, `vcs-gitea`). Ignored by default and
//! **skips gracefully** unless a live Gitea is wired up, so a maintainer's plain
//! `cargo test -- --ignored` never fails here.
//!
//! # Why this exists
//!
//! ROADMAP §6.1 flags the gap this closes: the forge wrappers' create/merge argv
//! "tracks the documented CLIs but isn't exercised end-to-end in CI (needs a live
//! forge)". The hermetic scripted-runner tests pin the argv and JSON parsing
//! against *assumed* fixtures; only a real `tea` against a real Gitea proves the
//! whole create → view → comment → edit → merge (plus issues/releases) round-trip
//! — the exact class of bug (`tea`'s real JSON shape diverging from our structs)
//! the `vcs-gitea` re-model once caught.
//!
//! # Wiring (set by the CI live-forge lane; see `.github/workflows/scheduled-cli-drift.yml`)
//!
//! - `VCS_GITEA_LIVE=1` — the "a live forge is configured" switch; absent ⇒ every
//!   case here skips.
//! - `VCS_GITEA_TEST_REPO=<dir>` — a git checkout whose `origin` points at the
//!   one-shot Gitea (so `tea` infers owner/repo/host).
//! - `VCS_GITEA_BASE_BRANCH` — the PR target branch (default `main`).
//! - `VCS_GITEA_HEAD_BRANCH` — the PR source branch, already pushed one commit
//!   ahead of the base (default `feature`).
//! - `VCS_GITEA_RELEASE_TAG` — a release tag the lane seeded, asserted present by
//!   the release round-trip (optional).
//!
//! No credential ever reaches this process: it drives `tea`'s ambient login and
//! the already-authenticated git remote the lane set up, so nothing here can print
//! a token.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use vcs_forge::vcs_gitea::{Gitea, GiteaApi};
use vcs_forge::{
    Forge, ForgeIssueState, ForgePrState, IssueCreate, MergeStrategy, PrCreate, PrEdit, PrMerge,
};

/// The env switch the CI live-forge lane sets once a one-shot Gitea is up and
/// `tea` is logged in. Absent ⇒ no live forge ⇒ skip.
const LIVE_ENV: &str = "VCS_GITEA_LIVE";

/// Whether `tea` is on PATH (a successful `--version` spawn) — a live run needs
/// the real binary; without it we skip rather than fail.
async fn tea_present() -> bool {
    Gitea::new().version().await.is_ok()
}

/// The checkout the lifecycle runs against, or `None` to skip. Every "not wired
/// for live" path — the switch unset, `tea` absent, or no repo dir — is a skip,
/// never a failure, mirroring the graceful-skip contract of the sibling
/// `vcs-gitea`/`vcs-gitlab` integration suites.
async fn live_repo() -> Option<PathBuf> {
    if std::env::var_os(LIVE_ENV).is_none() {
        eprintln!("skipping: {LIVE_ENV} unset (no live Gitea configured)");
        return None;
    }
    if !tea_present().await {
        eprintln!("skipping: tea not installed");
        return None;
    }
    match std::env::var_os("VCS_GITEA_TEST_REPO") {
        Some(dir) => Some(PathBuf::from(dir)),
        None => {
            eprintln!("skipping: {LIVE_ENV} set but VCS_GITEA_TEST_REPO unset");
            None
        }
    }
}

/// The PR target branch the lane pushed (default `main`).
fn base_branch() -> String {
    std::env::var("VCS_GITEA_BASE_BRANCH").unwrap_or_else(|_| "main".to_string())
}

/// The PR source branch the lane pushed one commit ahead of the base
/// (default `feature`).
fn head_branch() -> String {
    std::env::var("VCS_GITEA_HEAD_BRANCH").unwrap_or_else(|_| "feature".to_string())
}

/// A run-unique suffix so re-runs and the parallel test cases never collide on a
/// title the lifecycle later looks the resource up by (`tea … create` returns free
/// text, not a number, so title is the only handle back to the new PR/issue).
fn unique(tag: &str) -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{tag} {stamp}")
}

/// The full create → list → view → comment → edit → merge PR lifecycle through
/// the facade against a real Gitea — the end-to-end proof the ROADMAP asks for.
#[tokio::test]
#[ignore = "requires a live one-shot Gitea (set VCS_GITEA_LIVE); see scheduled-cli-drift.yml"]
async fn pr_lifecycle_round_trip() {
    let Some(dir) = live_repo().await else { return };
    let forge = Forge::gitea(dir);
    let base = base_branch();
    let head = head_branch();

    // Open a PR from the pre-pushed head branch into base — the "create" half of
    // the create/merge lifecycle hermetic fixtures can't prove.
    let title = unique("vcs-forge live PR");
    forge
        .pr_create(
            PrCreate::new(&title, "opened by the vcs-forge live lifecycle test")
                .source(&head)
                .target(&base),
        )
        .await
        .expect("pr_create against a live Gitea");

    // `tea pr create` returns free text (no number/URL), so find our PR by its
    // unique title — which also exercises `pr_list`'s real `tea --output json` parse.
    let prs = forge.pr_list().await.expect("pr_list");
    let pr = prs
        .iter()
        .find(|p| p.title == title)
        .unwrap_or_else(|| panic!("freshly created PR {title:?} not in pr_list: {prs:?}"));
    assert_eq!(pr.state, ForgePrState::Open);
    let number = pr.number;

    // View it: number/title/branches round-trip through the typed single-PR view.
    let viewed = forge.pr_view(number).await.expect("pr_view");
    assert_eq!(viewed.number, number);
    assert_eq!(viewed.title, title);
    assert_eq!(viewed.source_branch, head);
    assert_eq!(viewed.target_branch, base);

    // Comment on it.
    forge
        .pr_comment(number, "commented by the vcs-forge live lifecycle test")
        .await
        .expect("pr_comment");

    // Edit the title, then confirm the change stuck through a fresh view.
    let edited_title = unique("vcs-forge live PR edited");
    forge
        .pr_edit(number, PrEdit::new().title(&edited_title))
        .await
        .expect("pr_edit");
    let after_edit = forge.pr_view(number).await.expect("pr_view after edit");
    assert_eq!(after_edit.title, edited_title);

    // Merge it — the "merge" half — then confirm the state flips to Merged (the
    // exact `merged`-vs-`closed` mapping the gitea backend re-modelled).
    forge
        .pr_merge(number, PrMerge::new(MergeStrategy::Merge))
        .await
        .expect("pr_merge");
    let after_merge = forge.pr_view(number).await.expect("pr_view after merge");
    assert_eq!(after_merge.state, ForgePrState::Merged);
}

/// The create → list → view issue lifecycle through the facade against a real
/// Gitea (issues share the `index` space with PRs on Gitea, so this also confirms
/// the wrappers don't confuse the two).
#[tokio::test]
#[ignore = "requires a live one-shot Gitea (set VCS_GITEA_LIVE); see scheduled-cli-drift.yml"]
async fn issue_lifecycle_round_trip() {
    let Some(dir) = live_repo().await else { return };
    let forge = Forge::gitea(dir);

    let title = unique("vcs-forge live issue");
    forge
        .issue_create(IssueCreate::new(
            &title,
            "opened by the vcs-forge live lifecycle test",
        ))
        .await
        .expect("issue_create");

    let issues = forge.issue_list().await.expect("issue_list");
    let issue = issues
        .iter()
        .find(|i| i.title == title)
        .unwrap_or_else(|| panic!("freshly created issue {title:?} not in issue_list: {issues:?}"));
    assert_eq!(issue.state, ForgeIssueState::Open);

    let viewed = forge.issue_view(issue.number).await.expect("issue_view");
    assert_eq!(viewed.number, issue.number);
    assert_eq!(viewed.title, title);
}

/// The release listing through the facade must return without a parse error
/// against a real `tea releases --output json` (the contract-drift check). When
/// the lane seeded a release tag, assert it's present too.
#[tokio::test]
#[ignore = "requires a live one-shot Gitea (set VCS_GITEA_LIVE); see scheduled-cli-drift.yml"]
async fn release_list_round_trip() {
    let Some(dir) = live_repo().await else { return };
    let forge = Forge::gitea(dir);

    let releases = forge
        .release_list()
        .await
        .expect("release_list against a live repo");
    if let Ok(tag) = std::env::var("VCS_GITEA_RELEASE_TAG") {
        assert!(
            releases.iter().any(|r| r.tag == tag),
            "seeded release {tag:?} not in release_list: {releases:?}"
        );
    }
}
