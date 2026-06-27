//! GitHub-backed implementations of the facade operations: thin calls to the
//! `vcs-github` client plus pure mappers from its types into the unified DTOs.

use std::path::Path;

use processkit::ProcessRunner;
use vcs_github::{
    CheckRun, GitHub, GitHubApi, Issue, PrCreate as GhPrCreate, PrEdit as GhPrEdit, PrMerge,
    PullRequest, Release, RepoView,
};

use crate::dto::{
    CiStatus, ForgeIssue, ForgeIssueState, ForgePr, ForgePrState, ForgeRelease, ForgeRepo,
    MergeStrategy, PrCreate, PrEdit,
};
use crate::error::Result;

pub(crate) async fn auth_status<R: ProcessRunner>(gh: &GitHub<R>) -> Result<bool> {
    Ok(gh.auth_status().await?)
}

pub(crate) async fn repo_view<R: ProcessRunner>(gh: &GitHub<R>, dir: &Path) -> Result<ForgeRepo> {
    Ok(map_repo(gh.repo_view(dir).await?))
}

pub(crate) async fn pr_list<R: ProcessRunner>(gh: &GitHub<R>, dir: &Path) -> Result<Vec<ForgePr>> {
    Ok(gh.pr_list(dir).await?.into_iter().map(map_pr).collect())
}

pub(crate) async fn pr_view<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
) -> Result<ForgePr> {
    Ok(map_pr(gh.pr_view(dir, number).await?))
}

pub(crate) async fn pr_create<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    spec: PrCreate,
) -> Result<String> {
    // The unified source/target map onto gh's head/base.
    let mut create = GhPrCreate::new(spec.title, spec.body);
    if let Some(source) = spec.source {
        create = create.head(source);
    }
    if let Some(target) = spec.target {
        create = create.base(target);
    }
    Ok(gh.pr_create(dir, create).await?)
}

pub(crate) async fn pr_comment<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
    body: &str,
) -> Result<String> {
    Ok(gh.pr_comment(dir, number, body).await?)
}

pub(crate) async fn pr_edit<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
    edit: PrEdit,
) -> Result<()> {
    // The unified spec is 1:1 with gh's per-field setter; the title/body
    // rename to the unified spec is the only thing the facade does.
    let mut gh_edit = GhPrEdit::new();
    if let Some(title) = edit.title {
        gh_edit = gh_edit.title(title);
    }
    if let Some(body) = edit.body {
        gh_edit = gh_edit.body(body);
    }
    gh.pr_edit(dir, number, gh_edit).await?;
    Ok(())
}

pub(crate) async fn pr_merge<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
    strategy: MergeStrategy,
) -> Result<()> {
    let merge = match strategy {
        MergeStrategy::Merge => PrMerge::merge(),
        MergeStrategy::Squash => PrMerge::squash(),
        MergeStrategy::Rebase => PrMerge::rebase(),
    };
    gh.pr_merge(dir, number, merge).await?;
    Ok(())
}

pub(crate) async fn pr_mark_ready<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
) -> Result<()> {
    gh.pr_ready(dir, number).await?;
    Ok(())
}

pub(crate) async fn pr_close<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
    delete_branch: bool,
) -> Result<()> {
    gh.pr_close(dir, number, delete_branch).await?;
    Ok(())
}

pub(crate) async fn pr_checks<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
) -> Result<CiStatus> {
    Ok(aggregate(&gh.pr_checks(dir, number).await?))
}

pub(crate) async fn issue_list<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
) -> Result<Vec<ForgeIssue>> {
    Ok(gh
        .issue_list(dir)
        .await?
        .into_iter()
        .map(map_issue)
        .collect())
}

pub(crate) async fn issue_view<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
) -> Result<ForgeIssue> {
    Ok(map_issue(gh.issue_view(dir, number).await?))
}

pub(crate) async fn issue_create<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    title: &str,
    body: &str,
) -> Result<String> {
    Ok(gh.issue_create(dir, title, body).await?)
}

pub(crate) async fn release_list<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
) -> Result<Vec<ForgeRelease>> {
    Ok(gh
        .release_list(dir)
        .await?
        .into_iter()
        .map(map_release)
        .collect())
}

pub(crate) async fn release_view<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    tag: &str,
) -> Result<ForgeRelease> {
    Ok(map_release(gh.release_view(dir, tag).await?))
}

fn map_pr(pr: PullRequest) -> ForgePr {
    ForgePr {
        number: pr.number,
        state: state_of(&pr.state),
        title: pr.title,
        source_branch: pr.head_ref_name,
        target_branch: pr.base_ref_name,
        url: pr.url,
        // gh's lean `--json` fields don't include `isDraft`, so it's reported
        // `false` here (see `ForgePr::draft`).
        draft: false,
    }
}

fn state_of(state: &str) -> ForgePrState {
    match state.to_ascii_uppercase().as_str() {
        "MERGED" => ForgePrState::Merged,
        "CLOSED" => ForgePrState::Closed,
        _ => ForgePrState::Open,
    }
}

fn map_issue(i: Issue) -> ForgeIssue {
    ForgeIssue {
        number: i.number,
        title: i.title,
        state: issue_state_of(&i.state),
        body: i.body,
        url: i.url,
    }
}

fn issue_state_of(state: &str) -> ForgeIssueState {
    // gh reports "OPEN"/"CLOSED"; anything unknown reads as live (Open), the
    // same documented fallback as `state_of` above.
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
        url: r.url,
        // gh reports an empty `publishedAt` for a draft — surface that as None.
        published_at: Some(r.published_at).filter(|s| !s.is_empty()),
        body: Some(r.body).filter(|s| !s.is_empty()),
        draft: r.is_draft,
        prerelease: r.is_prerelease,
    }
}

fn map_repo(r: RepoView) -> ForgeRepo {
    ForgeRepo {
        name: r.name,
        owner: r.owner,
        default_branch: r.default_branch,
        url: r.url,
        private: r.is_private,
    }
}

/// Fold gh's per-check buckets into one coarse status: any fail/cancel ⇒
/// Failing; else any pending ⇒ Pending; else any pass ⇒ Passing; else — if there
/// are only unmodeled (`Unknown`) checks — Pending (conservatively "not known to be
/// done", matching the GitLab mapper); else None.
fn aggregate(checks: &[CheckRun]) -> CiStatus {
    let mut any_pending = false;
    let mut any_pass = false;
    let mut any_unknown = false;
    for c in checks {
        if c.bucket.is_failing() {
            return CiStatus::Failing;
        } else if c.bucket.is_pending() {
            any_pending = true;
        } else if c.bucket.is_passing() {
            any_pass = true;
        } else if c.bucket.is_unknown() {
            any_unknown = true;
        }
        // `Skipping` is a deliberate, terminal no-op — it doesn't move the needle.
    }
    if any_pending {
        CiStatus::Pending
    } else if any_pass {
        // A modeled pass wins over an unmodeled bucket: the checks we understand
        // passed, and an `Unknown` is specifically not a recognized failure.
        CiStatus::Passing
    } else if any_unknown {
        // Checks exist but are *all* an unmodeled bucket (a future `gh` value or a
        // missing field) — report Pending ("not known to be done") rather than the
        // misleading None ("no CI ran"), consistent with `gitlab_forge::map_ci`.
        CiStatus::Pending
    } else {
        CiStatus::None // no checks, or only deliberate skips
    }
}

// `state_of` is private; the proptest lives in-module where it's visible. The
// mapper must never panic on an arbitrary state string, and an UNKNOWN state
// must default to `Open` (the documented fallback) — so a future GitHub state
// we don't model is treated as live, never silently as closed/merged.
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        // Same contract for issues: only "closed" (any case) maps off Open.
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
            // The only inputs that map off `Open` are the three known states
            // (case-insensitively); everything else must default to `Open`.
            match s.to_ascii_uppercase().as_str() {
                "MERGED" => prop_assert_eq!(mapped, ForgePrState::Merged),
                "CLOSED" => prop_assert_eq!(mapped, ForgePrState::Closed),
                _ => prop_assert_eq!(mapped, ForgePrState::Open, "unknown must default to Open: {:?}", s),
            }
        }
    }
}
