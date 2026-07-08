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
    Gitea, GiteaApi, Issue, MergeStrategy as GtMs, PrCreate as GtPrCreate, PrEdit as GtPrEdit,
    PullRequest, Release,
};

use crate::dto::{
    ForgeIssue, ForgeIssueState, ForgePr, ForgePrState, ForgeRelease, MergeStrategy, PrCreate,
    PrEdit,
};
use crate::error::Result;

pub(crate) async fn auth_status<R: ProcessRunner>(tea: &Gitea<R>) -> Result<bool> {
    Ok(tea.auth_status().await?)
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
    strategy: MergeStrategy,
) -> Result<()> {
    let ms = match strategy {
        MergeStrategy::Merge => GtMs::Merge,
        MergeStrategy::Squash => GtMs::Squash,
        MergeStrategy::Rebase => GtMs::Rebase,
    };
    tea.pr_merge(dir, number, ms).await?;
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
        // `tea`'s issue list/view has no labels/assignees column — always empty
        // (see `ForgeIssue::labels`/`ForgeIssue::assignees` doc).
        labels: Vec::new(),
        assignees: Vec::new(),
    }
}

fn map_release(r: Release) -> ForgeRelease {
    ForgeRelease {
        tag: r.tag,
        title: r.title,
        url: r.url,
        // An empty `published_at` (an unpublished draft) surfaces as None.
        published_at: Some(r.published_at).filter(|s| !s.is_empty()),
        // `tea` has no release body/notes column.
        body: None,
        draft: r.draft,
        prerelease: r.prerelease,
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
        draft: false,
        // `tea`'s PR list/view has no labels/assignees column — always empty
        // (see `ForgePr::labels`/`ForgePr::assignees` doc).
        labels: Vec::new(),
        assignees: Vec::new(),
    }
}
