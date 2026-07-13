//! Gitea-backed implementations of the facade operations: thin calls to the
//! `vcs-gitea` client plus pure mappers from its types into the unified DTOs.
//!
//! `tea` has no current-repo view, draft toggle, PR-checks command, or
//! single-release view, so `repo_view` / `pr_mark_ready` / `pr_checks` /
//! `release_view` have no function here — the [`Forge`](crate::Forge) dispatch
//! returns [`Unsupported`](crate::Error::Unsupported) for the Gitea backend
//! instead.

use std::path::Path;

use processkit::ProcessRunner;
use vcs_gitea::{
    Gitea, GiteaApi, Issue, PrCreate as GtPrCreate, PrEdit as GtPrEdit, PrMerge as GtPrMerge,
    PullRequest, Release,
};

use crate::dto::{
    ForgeIssue, ForgeIssueState, ForgePr, ForgePrState, ForgeRelease, MergeStrategy, PrCreate,
    PrEdit, PrMerge,
};
use crate::error::Result;

pub(crate) async fn auth_status<R: ProcessRunner>(tea: &Gitea<R>) -> Result<bool> {
    Ok(tea.auth_status().await?)
}

/// Probe the `tea` version for the capability map: `(installed version, meets the
/// crate floor)`. An unrecognisable `tea --version` banner degrades to `(None,
/// false)` — we can't confirm the floor, so the map conservatively reports the ops
/// unavailable rather than erroring the whole probe. A real spawn/timeout failure
/// (a missing `tea`, a killed process) still propagates.
pub(crate) async fn version_support<R: ProcessRunner>(
    tea: &Gitea<R>,
) -> Result<(Option<vcs_gitea::GiteaVersion>, bool)> {
    match tea.capabilities().await {
        Ok(caps) => Ok((Some(caps.version), caps.is_supported())),
        Err(processkit::Error::Parse { .. }) => Ok((None, false)),
        Err(e) => Err(e.into()),
    }
}

pub(crate) async fn pr_list<R: ProcessRunner>(tea: &Gitea<R>, dir: &Path) -> Result<Vec<ForgePr>> {
    Ok(tea.pr_list(dir).await?.into_iter().map(map_pr).collect())
}

pub(crate) async fn pr_view<R: ProcessRunner>(
    tea: &Gitea<R>,
    dir: &Path,
    number: u64,
) -> Result<ForgePr> {
    Ok(map_pr(tea.pr_view(dir, number).await?))
}

pub(crate) async fn pr_create<R: ProcessRunner>(
    tea: &Gitea<R>,
    dir: &Path,
    spec: PrCreate,
) -> Result<String> {
    // The unified source/target map onto tea's head/base.
    let mut create = GtPrCreate::new(spec.title, spec.body);
    if let Some(source) = spec.source {
        create = create.head(source);
    }
    if let Some(target) = spec.target {
        create = create.base(target);
    }
    Ok(tea.pr_create(dir, create).await?)
}

pub(crate) async fn pr_comment<R: ProcessRunner>(
    tea: &Gitea<R>,
    dir: &Path,
    number: u64,
    body: &str,
) -> Result<String> {
    Ok(tea.pr_comment(dir, number, body).await?)
}

pub(crate) async fn pr_edit<R: ProcessRunner>(
    tea: &Gitea<R>,
    dir: &Path,
    number: u64,
    edit: PrEdit,
) -> Result<()> {
    let mut t_edit = GtPrEdit::new();
    if let Some(title) = edit.title {
        t_edit = t_edit.title(title);
    }
    if let Some(body) = edit.body {
        t_edit = t_edit.body(body);
    }
    tea.pr_edit(dir, number, t_edit).await?;
    Ok(())
}

pub(crate) async fn pr_merge<R: ProcessRunner>(
    tea: &Gitea<R>,
    dir: &Path,
    number: u64,
    merge: PrMerge,
) -> Result<()> {
    // Map the unified spec onto tea's `PrMerge`. The strategy maps to `--style`;
    // `auto`/`delete_branch` pass through verbatim — tea's wrapper reports them
    // `Unsupported` rather than silently dropping them (see `vcs_gitea::PrMerge`).
    // The exhaustive `match` (no catch-all) makes a new `MergeStrategy` variant a
    // compile error here.
    let mut pr = match merge.strategy {
        MergeStrategy::Merge => GtPrMerge::merge(),
        MergeStrategy::Squash => GtPrMerge::squash(),
        MergeStrategy::Rebase => GtPrMerge::rebase(),
    };
    if merge.auto {
        pr = pr.auto();
    }
    if merge.delete_branch {
        pr = pr.delete_branch();
    }
    tea.pr_merge(dir, number, pr).await?;
    Ok(())
}

// `tea pr close` takes no branch-deletion flag, so `delete_branch` is ignored.
pub(crate) async fn pr_close<R: ProcessRunner>(
    tea: &Gitea<R>,
    dir: &Path,
    number: u64,
) -> Result<()> {
    tea.pr_close(dir, number).await?;
    Ok(())
}

pub(crate) async fn pr_checkout<R: ProcessRunner>(
    tea: &Gitea<R>,
    dir: &Path,
    number: u64,
) -> Result<()> {
    tea.pr_checkout(dir, number).await?;
    Ok(())
}

// The facade's `pr_approve` → tea's `pr approve`.
pub(crate) async fn pr_approve<R: ProcessRunner>(
    tea: &Gitea<R>,
    dir: &Path,
    number: u64,
) -> Result<()> {
    tea.pr_approve(dir, number).await?;
    Ok(())
}

// The facade's `pr_request_changes` → tea's `pr reject <index> <reason>` (tea's
// negative review action). The `body` becomes the required reason; the Gitea
// wrapper guards it (bare positional), and the facade rejects an empty body up front.
pub(crate) async fn pr_request_changes<R: ProcessRunner>(
    tea: &Gitea<R>,
    dir: &Path,
    number: u64,
    body: &str,
) -> Result<()> {
    tea.pr_reject(dir, number, body).await?;
    Ok(())
}

pub(crate) async fn issue_list<R: ProcessRunner>(
    tea: &Gitea<R>,
    dir: &Path,
) -> Result<Vec<ForgeIssue>> {
    Ok(tea
        .issue_list(dir)
        .await?
        .into_iter()
        .map(map_issue)
        .collect())
}

pub(crate) async fn issue_view<R: ProcessRunner>(
    tea: &Gitea<R>,
    dir: &Path,
    number: u64,
) -> Result<ForgeIssue> {
    Ok(map_issue(tea.issue_view(dir, number).await?))
}

pub(crate) async fn issue_create<R: ProcessRunner>(
    tea: &Gitea<R>,
    dir: &Path,
    title: &str,
    body: &str,
) -> Result<String> {
    Ok(tea.issue_create(dir, title, body).await?)
}

pub(crate) async fn release_list<R: ProcessRunner>(
    tea: &Gitea<R>,
    dir: &Path,
) -> Result<Vec<ForgeRelease>> {
    Ok(tea
        .release_list(dir)
        .await?
        .into_iter()
        .map(map_release)
        .collect())
}

fn map_issue(i: Issue) -> ForgeIssue {
    ForgeIssue {
        number: i.number,
        title: i.title,
        // Gitea spells it "closed"; anything unknown reads as live (Open),
        // matching `map_pr` below.
        state: if i.state.eq_ignore_ascii_case("closed") {
            ForgeIssueState::Closed
        } else {
            ForgeIssueState::Open
        },
        body: i.body,
        url: i.url,
        // `tea`'s issue list/view has no labels/assignees column, so they are
        // *unknown* (`None`) — not a false empty list a consumer could read as a
        // confirmed "no labels / unassigned" (see `ForgeIssue::labels`/`assignees`).
        labels: None,
        assignees: None,
    }
}

fn map_release(r: Release) -> ForgeRelease {
    ForgeRelease {
        tag: r.tag,
        title: r.title,
        // `tea releases` exposes no release-page URL column (the raw `url` is
        // always empty), so it is *unknown* (`None`), not a false empty string.
        url: None,
        // An empty `published_at` (an unpublished draft) surfaces as None.
        published_at: Some(r.published_at).filter(|s| !s.is_empty()),
        // `tea` has no release body/notes column.
        body: None,
        // `tea` derives draft/prerelease from its `Status` column, so these are
        // confirmed values.
        draft: Some(r.draft),
        prerelease: Some(r.prerelease),
    }
}

fn map_pr(pr: PullRequest) -> ForgePr {
    ForgePr {
        number: pr.number,
        // tea folds the merge flag into its `state` column: a merged PR reads
        // `"merged"` (not `"closed"`). `pr.merged` is derived from that, so key
        // off it first, then the closed/open spelling.
        state: if pr.merged {
            ForgePrState::Merged
        } else if pr.state.eq_ignore_ascii_case("closed") {
            ForgePrState::Closed
        } else {
            ForgePrState::Open
        },
        title: pr.title,
        source_branch: pr.head_branch,
        target_branch: pr.base_branch,
        url: pr.url,
        // `tea`'s PR list/view carries no draft flag and no labels/assignees
        // column, so all three are *unknown* (`None`) — never a false
        // `Some(false)`/empty list a consumer could read as confirmed (see the
        // `ForgePr::draft`/`labels`/`assignees` docs).
        draft: None,
        labels: None,
        assignees: None,
    }
}
