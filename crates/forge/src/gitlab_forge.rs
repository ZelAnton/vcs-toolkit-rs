//! GitLab-backed implementations of the facade operations: thin calls to the
//! `vcs-gitlab` client plus pure mappers from its types into the unified DTOs.

use std::path::Path;

use processkit::ProcessRunner;
use vcs_gitlab::{
    CiStatus as GlCi, GitLab, GitLabApi, Issue, MergeRequest, MrCreate, MrEdit as GlMrEdit,
    MrMerge, Release, RepoView,
};

use crate::dto::{
    CiStatus, ForgeIssue, ForgeIssueState, ForgePr, ForgePrState, ForgeRelease, ForgeRepo,
    MergeStrategy, PrCreate, PrEdit, PrMerge,
};
use crate::error::Result;

pub(crate) async fn auth_status<R: ProcessRunner>(glab: &GitLab<R>) -> Result<bool> {
    Ok(glab.auth_status().await?)
}

/// Probe the `glab` version for the capability map: `(installed version, meets the
/// crate floor)`. An unrecognisable `glab --version` banner degrades to `(None,
/// false)` — we can't confirm the floor, so the map conservatively reports the ops
/// unavailable rather than erroring the whole probe. A real spawn/timeout failure
/// (a missing `glab`, a killed process) still propagates.
pub(crate) async fn version_support<R: ProcessRunner>(
    glab: &GitLab<R>,
) -> Result<(Option<vcs_gitlab::GitLabVersion>, bool)> {
    match glab.capabilities().await {
        Ok(caps) => Ok((Some(caps.version), caps.is_supported())),
        Err(processkit::Error::Parse { .. }) => Ok((None, false)),
        Err(e) => Err(e.into()),
    }
}

pub(crate) async fn repo_view<R: ProcessRunner>(glab: &GitLab<R>, dir: &Path) -> Result<ForgeRepo> {
    Ok(map_project(glab.repo_view(dir).await?))
}

pub(crate) async fn pr_list<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
) -> Result<Vec<ForgePr>> {
    Ok(glab.mr_list(dir).await?.into_iter().map(map_mr).collect())
}

pub(crate) async fn pr_view<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    number: u64,
) -> Result<ForgePr> {
    Ok(map_mr(glab.mr_view(dir, number).await?))
}

pub(crate) async fn pr_create<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    spec: PrCreate,
) -> Result<String> {
    // The unified source/target ARE glab's naming — a 1:1 field map.
    let mut create = MrCreate::new(spec.title, spec.body);
    if let Some(source) = spec.source {
        create = create.source(source);
    }
    if let Some(target) = spec.target {
        create = create.target(target);
    }
    Ok(glab.mr_create(dir, create).await?)
}

pub(crate) async fn mr_comment<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    id: u64,
    body: &str,
) -> Result<String> {
    Ok(glab.mr_comment(dir, id, body).await?)
}

pub(crate) async fn mr_edit<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    id: u64,
    edit: PrEdit,
) -> Result<()> {
    let mut gl_edit = GlMrEdit::new();
    if let Some(title) = edit.title {
        gl_edit = gl_edit.title(title);
    }
    if let Some(body) = edit.body {
        gl_edit = gl_edit.body(body);
    }
    glab.mr_edit(dir, id, gl_edit).await?;
    Ok(())
}

pub(crate) async fn pr_merge<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    number: u64,
    merge: PrMerge,
) -> Result<()> {
    // Map the unified spec onto glab's `MrMerge`. The strategy maps to a flag;
    // `auto`/`delete_branch` pass through verbatim — glab's wrapper reports them
    // `Unsupported` rather than silently dropping them (see `vcs_gitlab::MrMerge`).
    // The exhaustive `match` (no catch-all) makes a new `MergeStrategy` variant a
    // compile error here.
    let mut mr = match merge.strategy {
        MergeStrategy::Merge => MrMerge::merge(),
        MergeStrategy::Squash => MrMerge::squash(),
        MergeStrategy::Rebase => MrMerge::rebase(),
    };
    if merge.auto {
        mr = mr.auto();
    }
    if merge.delete_branch {
        mr = mr.delete_branch();
    }
    glab.mr_merge(dir, number, mr).await?;
    Ok(())
}

pub(crate) async fn pr_mark_ready<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    number: u64,
) -> Result<()> {
    glab.mr_mark_ready(dir, number).await?;
    Ok(())
}

// The facade's `pr_approve` → glab's `mr_approve`. GitLab has no "request changes"
// review action (only approve/revoke), so there is no `pr_request_changes` bridge
// here — the `Forge` dispatch reports it `Unsupported` for GitLab.
pub(crate) async fn pr_approve<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    number: u64,
) -> Result<()> {
    glab.mr_approve(dir, number).await?;
    Ok(())
}

// `delete_branch` has no `glab mr close` equivalent (GitLab honours the MR's own
// "delete source branch" setting on merge, not on close), so it is ignored here.
pub(crate) async fn pr_close<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    number: u64,
) -> Result<()> {
    glab.mr_close(dir, number).await?;
    Ok(())
}

// Named `pr_checkout` (not `mr_checkout`) to match the facade-level naming used
// by the other bridging fns here — it calls glab's `mr_checkout` under the `pr_*`
// facade name (like `pr_merge` → `mr_merge`, `pr_diff` → `mr_diff`).
pub(crate) async fn pr_checkout<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    number: u64,
) -> Result<()> {
    glab.mr_checkout(dir, number).await?;
    Ok(())
}

pub(crate) async fn pr_checks<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    number: u64,
) -> Result<CiStatus> {
    Ok(map_ci(glab.mr_checks(dir, number).await?))
}

// `glab.mr_diff` already returns `vcs-diff`'s model directly (glab emits the
// same git-format diff `git diff` does), so this is a plain forward — no
// mapping. Named `pr_diff` (not `mr_diff`) to match the facade-level naming
// used by every other bridging fn here (`pr_view`, `pr_merge`, `pr_checks`, …
// all call into glab's `mr_*` methods under a `pr_*` facade name).
pub(crate) async fn pr_diff<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    number: u64,
) -> Result<Vec<vcs_diff::FileDiff>> {
    Ok(glab.mr_diff(dir, number).await?)
}

// The per-call output-budget override — forwards to the client's `mr_diff_within`.
pub(crate) async fn pr_diff_within<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    number: u64,
    budget: vcs_cli_support::OutputBudget,
) -> Result<Vec<vcs_diff::FileDiff>> {
    Ok(glab.mr_diff_within(dir, number, budget).await?)
}

pub(crate) async fn issue_list<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
) -> Result<Vec<ForgeIssue>> {
    Ok(glab
        .issue_list(dir)
        .await?
        .into_iter()
        .map(map_issue)
        .collect())
}

pub(crate) async fn issue_view<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    number: u64,
) -> Result<ForgeIssue> {
    Ok(map_issue(glab.issue_view(dir, number).await?))
}

pub(crate) async fn issue_create<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    title: &str,
    body: &str,
) -> Result<String> {
    Ok(glab.issue_create(dir, title, body).await?)
}

pub(crate) async fn release_list<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
) -> Result<Vec<ForgeRelease>> {
    Ok(glab
        .release_list(dir)
        .await?
        .into_iter()
        .map(map_release)
        .collect())
}

pub(crate) async fn release_view<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    tag: &str,
) -> Result<ForgeRelease> {
    Ok(map_release(glab.release_view(dir, tag).await?))
}

fn map_issue(i: Issue) -> ForgeIssue {
    ForgeIssue {
        number: i.number,
        title: i.title,
        state: issue_state_of(&i.state),
        body: i.body,
        url: i.url,
        // GitLab's REST issue always carries labels/assignees — confirmed values.
        labels: Some(i.labels),
        assignees: Some(i.assignees),
    }
}

fn issue_state_of(state: &str) -> ForgeIssueState {
    // GitLab spells it "closed" (note: open is "opened"); anything unknown
    // reads as live (Open), the same documented fallback as `state_of` below.
    if state.eq_ignore_ascii_case("closed") {
        ForgeIssueState::Closed
    } else {
        ForgeIssueState::Open
    }
}

fn map_release(r: Release) -> ForgeRelease {
    ForgeRelease {
        tag: r.tag_name,
        title: r.name,
        // GitLab carries the URL as `_links.self`; an empty (absent-links) value
        // surfaces as None rather than an empty string.
        url: Some(r.url).filter(|s| !s.is_empty()),
        // An empty `released_at` (unpublished/upcoming release) surfaces as None.
        published_at: Some(r.published_at).filter(|s| !s.is_empty()),
        body: Some(r.description).filter(|s| !s.is_empty()),
        // GitLab has no draft/pre-release concept on a release, so these are
        // *unknown* (`None`) — never a false `Some(false)` that would read as a
        // confirmed "not a draft / not a pre-release".
        draft: None,
        prerelease: None,
    }
}

fn map_mr(mr: MergeRequest) -> ForgePr {
    ForgePr {
        number: mr.iid,
        state: state_of(&mr.state),
        title: mr.title,
        source_branch: mr.source_branch,
        target_branch: mr.target_branch,
        url: mr.web_url,
        // GitLab's REST MR always carries draft/labels/assignees — confirmed values.
        draft: Some(mr.draft),
        labels: Some(mr.labels),
        assignees: Some(mr.assignees),
    }
}

fn state_of(state: &str) -> ForgePrState {
    // GitLab REST emits lowercase, but match case-insensitively for parity with
    // the GitHub/Gitea mappers (and robustness to a future shape change).
    match state.to_ascii_lowercase().as_str() {
        "merged" => ForgePrState::Merged,
        "closed" | "locked" => ForgePrState::Closed,
        _ => ForgePrState::Open,
    }
}

fn map_project(p: RepoView) -> ForgeRepo {
    // GitLab has no separate "owner" — split the namespace path: everything
    // before the last `/` is the owner, the last segment the project slug.
    let owner = p
        .path_with_namespace
        .rsplit_once('/')
        .map(|(ns, _)| ns.to_string())
        .unwrap_or_default();
    ForgeRepo {
        name: p.name,
        owner,
        default_branch: p.default_branch,
        url: p.web_url,
        // Only claim a *known* visibility: a present value maps to `Some(v !=
        // "public")`, but an absent visibility (`glab` omitted the field) is
        // genuinely unknown and maps to `None` — never a false `Some(false)` a
        // consumer could read as a proven-public repo.
        private: p.visibility.as_deref().map(|v| v != "public"),
    }
}

fn map_ci(c: GlCi) -> CiStatus {
    match c {
        GlCi::Passing => CiStatus::Passing,
        GlCi::Failing => CiStatus::Failing,
        GlCi::Pending => CiStatus::Pending,
        GlCi::None => CiStatus::None,
        // `vcs_gitlab::CiStatus` is `#[non_exhaustive]`; map any future bucket
        // conservatively to "not known to be done".
        _ => CiStatus::Pending,
    }
}

// `state_of` is private; the proptest lives in-module where it's visible. The
// mapper must never panic on an arbitrary state string, and an UNKNOWN state
// must default to `Open` (the documented fallback) — so a future GitLab state
// we don't model is treated as live, never silently as closed/merged.
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        // Same contract for issues: only "closed" (any case) maps off Open —
        // GitLab's "opened" and any future state both read as live.
        #[test]
        fn issue_state_mapping_never_panics_and_unknowns_default(s in any::<String>()) {
            let mapped = issue_state_of(&s);
            if s.eq_ignore_ascii_case("closed") {
                prop_assert_eq!(mapped, ForgeIssueState::Closed);
            } else {
                prop_assert_eq!(mapped, ForgeIssueState::Open, "unknown must default to Open: {:?}", s);
            }
        }

        #[test]
        fn pr_state_mapping_never_panics_and_unknowns_default(s in any::<String>()) {
            let mapped = state_of(&s);
            // The only inputs that map off `Open` are the known states
            // (case-insensitively); everything else must default to `Open`.
            match s.to_ascii_lowercase().as_str() {
                "merged" => prop_assert_eq!(mapped, ForgePrState::Merged),
                "closed" | "locked" => prop_assert_eq!(mapped, ForgePrState::Closed),
                _ => prop_assert_eq!(mapped, ForgePrState::Open, "unknown must default to Open: {:?}", s),
            }
        }
    }
}
