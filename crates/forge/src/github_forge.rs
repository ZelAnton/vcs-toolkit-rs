//! GitHub-backed implementations of the facade operations: thin calls to the
//! `vcs-github` client plus pure mappers from its types into the unified DTOs.

use std::path::Path;

use processkit::ProcessRunner;
use vcs_github::{
    CheckRun, GitHub, GitHubApi, Issue, PrClose as GhPrClose, PrCreate as GhPrCreate,
    PrEdit as GhPrEdit, PrMerge as GhPrMerge, PullRequest, Release,
    ReleaseCreate as GhReleaseCreate, RepoView, ReviewAction,
};

use crate::dto::{
    CiStatus, ForgeIssue, ForgeIssueState, ForgePr, ForgePrState, ForgeRelease, ForgeRepo,
    MergeStrategy, PrCreate, PrEdit, PrMerge, ReleaseCreate,
};
use crate::error::Result;

pub(crate) async fn auth_status<R: ProcessRunner>(gh: &GitHub<R>) -> Result<bool> {
    Ok(gh.auth_status().await?)
}

/// Probe the `gh` version for the capability map: `(installed version, meets the
/// crate floor)`. An unrecognisable `gh --version` banner degrades to `(None,
/// false)` — we can't confirm the floor, so the map conservatively reports the ops
/// unavailable rather than erroring the whole probe. A real spawn/timeout failure
/// (a missing `gh`, a killed process) still propagates.
pub(crate) async fn version_support<R: ProcessRunner>(
    gh: &GitHub<R>,
) -> Result<(Option<vcs_github::GitHubVersion>, bool)> {
    match gh.capabilities().await {
        Ok(caps) => Ok((Some(caps.version), caps.is_supported())),
        Err(processkit::Error::Parse { .. }) => Ok((None, false)),
        Err(e) => Err(e.into()),
    }
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
    merge: PrMerge,
) -> Result<()> {
    // Map the unified spec onto gh's rich `PrMerge` — the strategy plus the two
    // GitHub-native options (`--auto`, `--delete-branch`). The exhaustive `match`
    // (no catch-all) makes a new `MergeStrategy` variant a compile error here.
    let mut gh_merge = match merge.strategy {
        MergeStrategy::Merge => GhPrMerge::merge(),
        MergeStrategy::Squash => GhPrMerge::squash(),
        MergeStrategy::Rebase => GhPrMerge::rebase(),
    };
    if merge.auto {
        gh_merge = gh_merge.auto();
    }
    if merge.delete_branch {
        gh_merge = gh_merge.delete_branch();
    }
    gh.pr_merge(dir, number, gh_merge).await?;
    Ok(())
}

pub(crate) async fn pr_mark_ready<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
) -> Result<()> {
    gh.pr_mark_ready(dir, number).await?;
    Ok(())
}

// The facade's `pr_approve` maps to gh's typed `pr review --approve` (no body).
pub(crate) async fn pr_approve<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
) -> Result<()> {
    gh.pr_review(dir, number, ReviewAction::approve()).await?;
    Ok(())
}

// `pr_request_changes` maps to gh's `pr review --request-changes --body <body>`.
// `ReviewAction::request_changes` encodes gh's "a request-changes review requires a
// body" invariant by construction (the facade also rejects an empty body up front).
pub(crate) async fn pr_request_changes<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
    body: &str,
) -> Result<()> {
    gh.pr_review(dir, number, ReviewAction::request_changes(body))
        .await?;
    Ok(())
}

pub(crate) async fn pr_close<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
    delete_branch: bool,
) -> Result<()> {
    let mut spec = GhPrClose::new();
    if delete_branch {
        spec = spec.delete_branch();
    }
    gh.pr_close(dir, number, spec).await?;
    Ok(())
}

pub(crate) async fn pr_checkout<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
) -> Result<()> {
    gh.pr_checkout(dir, number).await?;
    Ok(())
}

pub(crate) async fn pr_checks<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
) -> Result<CiStatus> {
    Ok(aggregate(&gh.pr_checks(dir, number).await?))
}

// `gh.pr_diff` already returns `vcs-diff`'s model directly (gh emits the same
// git-format diff `git diff` does), so this is a plain forward — no mapping.
pub(crate) async fn pr_diff<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
) -> Result<Vec<vcs_diff::FileDiff>> {
    Ok(gh.pr_diff(dir, number).await?)
}

// The per-call output-budget override — forwards to the client's `pr_diff_within`.
pub(crate) async fn pr_diff_within<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    number: u64,
    budget: vcs_cli_support::OutputBudget,
) -> Result<Vec<vcs_diff::FileDiff>> {
    Ok(gh.pr_diff_within(dir, number, budget).await?)
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

pub(crate) async fn release_create<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    spec: ReleaseCreate,
) -> Result<String> {
    // The unified spec maps 1:1 onto gh's `ReleaseCreate`; gh supports the full
    // title/notes/draft/prerelease surface, so every field carries over.
    Ok(gh.release_create(dir, map_release_create(spec)).await?)
}

pub(crate) async fn release_delete<R: ProcessRunner>(
    gh: &GitHub<R>,
    dir: &Path,
    tag: &str,
) -> Result<()> {
    gh.release_delete(dir, tag).await?;
    Ok(())
}

fn map_release_create(spec: ReleaseCreate) -> GhReleaseCreate {
    let mut create = GhReleaseCreate::new(spec.tag);
    if let Some(title) = spec.title {
        create = create.title(title);
    }
    if let Some(notes) = spec.notes {
        create = create.notes(notes);
    }
    if spec.draft {
        create = create.draft();
    }
    if spec.prerelease {
        create = create.prerelease();
    }
    create
}

fn map_pr(pr: PullRequest) -> ForgePr {
    ForgePr {
        number: pr.number,
        state: state_of(&pr.state),
        title: pr.title,
        source_branch: pr.head_ref_name,
        target_branch: pr.base_ref_name,
        url: pr.url,
        // gh always reports these when requested (`--json isDraft,labels,assignees`
        // are in PR_FIELDS), so they are confirmed values, never unknown.
        draft: Some(pr.is_draft),
        labels: Some(pr.labels),
        assignees: Some(pr.assignees),
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
        // gh always reports labels/assignees when requested — confirmed, not unknown.
        labels: Some(i.labels),
        assignees: Some(i.assignees),
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
        // The raw `url`/`body` are `Option`: `None` from the lean `release_list`
        // (RELEASE_LIST_FIELDS omits them), `Some` from `release_view`. Drop an
        // empty string to `None` too, so an unexpected `""` never reads as a URL.
        url: r.url.filter(|s| !s.is_empty()),
        // gh reports an empty `publishedAt` for a draft — surface that as None.
        published_at: Some(r.published_at).filter(|s| !s.is_empty()),
        body: r.body.filter(|s| !s.is_empty()),
        // gh always reports isDraft/isPrerelease (both in the list and view field
        // sets), so these are confirmed values.
        draft: Some(r.is_draft),
        prerelease: Some(r.is_prerelease),
    }
}

fn map_repo(r: RepoView) -> ForgeRepo {
    ForgeRepo {
        name: r.name,
        owner: r.owner,
        default_branch: r.default_branch,
        url: r.url,
        // gh's `repo view` always reports `isPrivate` — a confirmed value.
        private: Some(r.is_private),
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
