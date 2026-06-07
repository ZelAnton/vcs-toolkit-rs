//! GitHub-backed implementations of the facade operations: thin calls to the
//! `vcs-github` client plus pure mappers from its types into the unified DTOs.

use std::path::Path;

use processkit::ProcessRunner;
use vcs_github::{CheckRun, GitHub, GitHubApi, PrMerge, PullRequest, Repo};

use crate::dto::{CiStatus, ForgePr, ForgePrState, ForgeRepo, MergeStrategy};
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
    title: &str,
    body: &str,
    source: Option<String>,
    target: Option<String>,
) -> Result<String> {
    Ok(gh.pr_create(dir, title, body, source, target).await?)
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

fn map_repo(r: Repo) -> ForgeRepo {
    ForgeRepo {
        name: r.name,
        owner: r.owner,
        default_branch: r.default_branch,
        url: r.url,
        private: r.is_private,
    }
}

/// Fold gh's per-check buckets into one coarse status: any fail/cancel ⇒
/// Failing; else any pending ⇒ Pending; else any pass ⇒ Passing; else None.
fn aggregate(checks: &[CheckRun]) -> CiStatus {
    let mut any_pending = false;
    let mut any_pass = false;
    for c in checks {
        match c.bucket.as_str() {
            "fail" | "cancel" => return CiStatus::Failing,
            "pending" => any_pending = true,
            "pass" => any_pass = true,
            _ => {} // "skipping" and unknowns don't move the needle.
        }
    }
    if any_pending {
        CiStatus::Pending
    } else if any_pass {
        CiStatus::Passing
    } else {
        CiStatus::None
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
