//! GitLab-backed implementations of the facade operations: thin calls to the
//! `vcs-gitlab` client plus pure mappers from its types into the unified DTOs.

use std::path::Path;

use processkit::ProcessRunner;
use vcs_gitlab::{
    CiStatus as GlCi, GitLab, GitLabApi, MergeRequest, MergeStrategy as GlMs, Project,
};

use crate::dto::{CiStatus, ForgePr, ForgePrState, ForgeRepo, MergeStrategy};
use crate::error::Result;

pub(crate) async fn auth_status<R: ProcessRunner>(glab: &GitLab<R>) -> Result<bool> {
    Ok(glab.auth_status().await?)
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
    title: &str,
    body: &str,
    source: Option<String>,
    target: Option<String>,
) -> Result<String> {
    Ok(glab.mr_create(dir, title, body, source, target).await?)
}

pub(crate) async fn pr_merge<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    number: u64,
    strategy: MergeStrategy,
) -> Result<()> {
    let ms = match strategy {
        MergeStrategy::Merge => GlMs::Merge,
        MergeStrategy::Squash => GlMs::Squash,
        MergeStrategy::Rebase => GlMs::Rebase,
    };
    glab.mr_merge(dir, number, ms).await?;
    Ok(())
}

pub(crate) async fn pr_mark_ready<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    number: u64,
) -> Result<()> {
    glab.mr_ready(dir, number).await?;
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

pub(crate) async fn pr_checks<R: ProcessRunner>(
    glab: &GitLab<R>,
    dir: &Path,
    number: u64,
) -> Result<CiStatus> {
    Ok(map_ci(glab.mr_checks(dir, number).await?))
}

fn map_mr(mr: MergeRequest) -> ForgePr {
    ForgePr {
        number: mr.iid,
        state: state_of(&mr.state),
        title: mr.title,
        source_branch: mr.source_branch,
        target_branch: mr.target_branch,
        url: mr.web_url,
        draft: mr.draft,
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

fn map_project(p: Project) -> ForgeRepo {
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
        // Conservative: only claim privacy when the visibility is *known* and not
        // "public". An absent visibility (`None`) is unknown, so it maps to
        // `false` (public) — we never assert a privacy we can't prove.
        private: p.visibility.as_deref().is_some_and(|v| v != "public"),
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
