//! Forge-agnostic data types the facade returns, generalising the per-CLI shapes
//! of `vcs-github`, `vcs-gitlab`, and `vcs-gitea` into one set a consumer can use
//! without knowing which forge is in play.

/// Which forge backs a [`Forge`](crate::Forge) handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ForgeKind {
    /// GitHub (the `gh` CLI).
    GitHub,
    /// GitLab (the `glab` CLI).
    GitLab,
    /// Gitea / Forgejo (the `tea` CLI).
    Gitea,
    /// The remote URL doesn't classify as a known forge (self-hosted, lookalike,
    /// or no remote). The [`Forge::capabilities`](crate::Forge::capabilities) of
    /// an unknown-forge handle is the all-`false` shape — the honest answer when
    /// no CLI can be picked. **Distinct from a forge that the CLI is just not
    /// authenticated against**: `authed: false` reports that; `kind: Unknown`
    /// reports that *no* CLI is reachable.
    Unknown,
}

impl ForgeKind {
    /// The forge's short name (`"github"` / `"gitlab"` / `"gitea"` / `"unknown"`).
    pub fn as_str(self) -> &'static str {
        match self {
            ForgeKind::GitHub => "github",
            ForgeKind::GitLab => "gitlab",
            ForgeKind::Gitea => "gitea",
            ForgeKind::Unknown => "unknown",
        }
    }

    /// Best-effort guess of the forge from a git remote URL's host, for the
    /// **public SaaS** hosts: `github.com` → [`GitHub`](ForgeKind::GitHub),
    /// `gitlab.com` → [`GitLab`](ForgeKind::GitLab), and `gitea.com` /
    /// `codeberg.org` → [`Gitea`](ForgeKind::Gitea) — each matching the exact host
    /// or a proper subdomain (`*.gitlab.com`), never a lookalike
    /// (`gitlab.com.evil.example` → `None`).
    ///
    /// Returns `None` for everything else: a **self-hosted** GitLab/Gitea lives on
    /// an arbitrary domain that can't be distinguished from any other host (and
    /// must not be guessed from a substring, which an attacker-controlled host
    /// could spoof), so pick the kind explicitly there. Accepts both
    /// `https://host/owner/repo(.git)` and scp-like `git@host:owner/repo.git`.
    pub fn from_remote_url(url: &str) -> Option<ForgeKind> {
        let host = host_of(url)?.to_ascii_lowercase();
        if host_is(&host, "github.com") {
            Some(ForgeKind::GitHub)
        } else if host_is(&host, "gitlab.com") {
            Some(ForgeKind::GitLab)
        } else if host_is(&host, "gitea.com") || host_is(&host, "codeberg.org") {
            Some(ForgeKind::Gitea)
        } else {
            None
        }
    }
}

/// Whether `host` is exactly `domain` or a **proper subdomain** of it
/// (`*.domain`) — an anchored match. Crucially, a lookalike such as
/// `gitlab.com.attacker.net` does NOT match `gitlab.com` (it doesn't *end* with
/// it after a `.`), and `notgithub.com` does NOT match `github.com`.
fn host_is(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

/// Extract the host from a git remote URL — scheme URLs (`https://host/…`,
/// `ssh://git@host:22/…`, `https://[::1]:443/…`) and scp-like
/// (`git@host:owner/repo.git`). For a scheme URL the host is bracket-aware, so
/// an IPv6 authority `[::1]:443` yields `::1` rather than `[`. (scp-like syntax
/// has no bracketed-IPv6 form — the `:` is the path separator — so a bare IPv6
/// literal there is not extracted.)
fn host_of(url: &str) -> Option<&str> {
    let rest = match url.split_once("://") {
        // A scheme URL: take the authority up to the next `/`, then drop userinfo.
        Some((_scheme, after)) => {
            let authority = after.split(['/', '?', '#']).next().unwrap_or(after);
            let host_port = authority.rsplit('@').next().unwrap_or(authority);
            return match host_port.strip_prefix('[') {
                // IPv6 literal `[::1]:443` → `::1`. Unwrap brackets ONLY when the
                // content parses as a real IPv6 address — a mere colon is not
                // enough: a bracketed name like `[gitlab.com]`, or a colon-bearing
                // fake like `[a:b.gitlab.com]`, would otherwise be unwrapped and
                // spoof a trusted SaaS host (`a:b.gitlab.com` matches the
                // `.gitlab.com` proper-subdomain test). A genuine IPv6 literal can
                // never equal or be a subdomain of a trusted DNS host, so this is
                // spoof-safe. (Zone IDs like `fe80::1%eth0` don't parse and so are
                // conservatively dropped — vanishingly rare in a git remote.)
                Some(inner) => inner
                    .split(']')
                    .next()
                    .filter(|h| h.parse::<std::net::Ipv6Addr>().is_ok()),
                // Otherwise strip an optional `:port`.
                None => host_port.split(':').next().filter(|h| !h.is_empty()),
            };
        }
        // No scheme: scp-like `user@host:path` or bare `host:path` / `host/path`.
        None => url,
    };
    let after_user = rest.rsplit('@').next().unwrap_or(rest);
    after_user
        .split([':', '/'])
        .next()
        .filter(|h| !h.is_empty())
}

/// A facade operation a consumer can probe with
/// [`Forge::supports`](crate::Forge::supports) before calling — the operations
/// worth a capability check, so a consumer (an agent, a TUI) hides an unavailable
/// button instead of issuing the call and handling the error. Every operation
/// *not* listed here is supported on all three real forges.
///
/// Most of these **vary by backend** — a backend may return
/// [`Unsupported`](crate::Error::Unsupported) (Gitea's `tea` lacks a current-repo
/// view, draft toggle, checks command, single-release view, and diff view; GitLab
/// has no "request changes" review action). [`PrCheckout`](ForgeOp::PrCheckout),
/// [`PrApprove`](ForgeOp::PrApprove), and the three issue-lifecycle ops
/// [`IssueClose`](ForgeOp::IssueClose) / [`IssueReopen`](ForgeOp::IssueReopen) /
/// [`IssueComment`](ForgeOp::IssueComment) are the exceptions: every real backend
/// (GitHub/GitLab/Gitea) supports them, and they are enumerated here so the support
/// matrix covers the full checkout/mutation/review surface — a consumer iterating
/// [`ALL`](ForgeOp::ALL) sees them reported available on all three. An
/// [`Unknown`](ForgeKind::Unknown) handle (no classified CLI) supports **none** of
/// them.
///
/// This is the *static* support set — distinct from [`ForgeCapabilities`], the
/// *auth-gated* action menu from [`Forge::capabilities`](crate::Forge::capabilities).
/// They overlap only on `pr_checks`: here it means "this backend ships a checks
/// command"; in `ForgeCapabilities` it additionally requires an authenticated CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ForgeOp {
    /// [`repo_view`](crate::Forge::repo_view) — current repo/project metadata.
    RepoView,
    /// [`pr_mark_ready`](crate::Forge::pr_mark_ready) — flip a draft PR to ready.
    PrMarkReady,
    /// [`pr_checks`](crate::Forge::pr_checks) — coarse CI status for a PR.
    PrChecks,
    /// [`release_view`](crate::Forge::release_view) — a single release by tag.
    ReleaseView,
    /// [`pr_diff`](crate::Forge::pr_diff) — a PR/MR's file-by-file diff.
    PrDiff,
    /// [`pr_checkout`](crate::Forge::pr_checkout) — check a PR/MR's branch out into
    /// the working copy. Supported on all three real backends (only an
    /// [`Unknown`](ForgeKind::Unknown) handle lacks it).
    PrCheckout,
    /// [`pr_approve`](crate::Forge::pr_approve) — submit an approving review.
    /// Supported on all three real backends (`gh pr review --approve` / `glab mr
    /// approve` / `tea pr approve`); only an [`Unknown`](ForgeKind::Unknown) handle
    /// lacks it.
    PrApprove,
    /// [`pr_request_changes`](crate::Forge::pr_request_changes) — submit a
    /// request-changes review. **Unsupported on GitLab** (its review model is
    /// approve/revoke, with no request-changes action); available on GitHub
    /// (`gh pr review --request-changes`) and Gitea (`tea pr reject`).
    PrRequestChanges,
    /// [`release_create`](crate::Forge::release_create) — create a release. Supported
    /// on all three real backends (`gh release create` / `glab release create` /
    /// `tea releases create`); only an [`Unknown`](ForgeKind::Unknown) handle lacks it.
    /// (The `draft`/`prerelease` *options* are a separate GitLab gap — see
    /// [`ReleaseCreate`] — but creating a release itself is available everywhere.)
    ReleaseCreate,
    /// [`release_delete`](crate::Forge::release_delete) — delete a release by tag.
    /// Supported on all three real backends (`gh release delete` / `glab release
    /// delete` / `tea releases delete`); only an [`Unknown`](ForgeKind::Unknown)
    /// handle lacks it.
    ReleaseDelete,
    /// [`issue_close`](crate::Forge::issue_close) — close an issue. Supported on all
    /// three real backends (`gh issue close` / `glab issue close` / `tea issues
    /// close`); only an [`Unknown`](ForgeKind::Unknown) handle lacks it.
    IssueClose,
    /// [`issue_reopen`](crate::Forge::issue_reopen) — reopen a closed issue. Supported
    /// on all three real backends (`gh issue reopen` / `glab issue reopen` / `tea
    /// issues reopen`); only an [`Unknown`](ForgeKind::Unknown) handle lacks it.
    IssueReopen,
    /// [`issue_comment`](crate::Forge::issue_comment) — post a comment to an issue.
    /// Supported on all three real backends (`gh issue comment` / `glab issue note` /
    /// `tea comment`); only an [`Unknown`](ForgeKind::Unknown) handle lacks it.
    IssueComment,
}

impl ForgeOp {
    /// Every operation a consumer may probe with
    /// [`supports`](crate::Forge::supports) — iterate it to build a full support
    /// matrix (e.g. to render an availability list). Most vary by backend;
    /// [`PrCheckout`](ForgeOp::PrCheckout) / [`PrApprove`](ForgeOp::PrApprove) /
    /// [`ReleaseCreate`](ForgeOp::ReleaseCreate) / [`ReleaseDelete`](ForgeOp::ReleaseDelete)
    /// are available on every real backend.
    pub const ALL: &'static [ForgeOp] = &[
        ForgeOp::RepoView,
        ForgeOp::PrMarkReady,
        ForgeOp::PrChecks,
        ForgeOp::ReleaseView,
        ForgeOp::PrDiff,
        ForgeOp::PrCheckout,
        ForgeOp::PrApprove,
        ForgeOp::PrRequestChanges,
        ForgeOp::ReleaseCreate,
        ForgeOp::ReleaseDelete,
        ForgeOp::IssueClose,
        ForgeOp::IssueReopen,
        ForgeOp::IssueComment,
    ];
}

/// A pull request (GitHub) / merge request (GitLab) / pull request (Gitea),
/// unified across the three forges.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ForgePr {
    /// The PR/MR number a caller passes to the other operations (GitHub/Gitea
    /// `number`, GitLab `iid`).
    pub number: u64,
    /// Title.
    pub title: String,
    /// Normalised state (see [`ForgePrState`]).
    pub state: ForgePrState,
    /// Source (head) branch name.
    pub source_branch: String,
    /// Target (base) branch name.
    pub target_branch: String,
    /// Web URL.
    pub url: String,
    /// Whether the PR/MR is a draft, or `None` when the backend can't report it.
    /// **Per-backend support:** GitHub (`gh --json isDraft`) and GitLab report a
    /// definite `Some(true)`/`Some(false)`; Gitea is always `None` — `tea`'s PR
    /// list/view carries no draft flag, so "not a draft" can't be told apart from
    /// "unknown", and the honest answer is `None` rather than a false `Some(false)`.
    pub draft: Option<bool>,
    /// Labels attached to the PR/MR, or `None` when the backend can't report them.
    /// **Per-backend support:** GitHub and GitLab report `Some(..)` — an empty
    /// `Some(vec![])` is a *confirmed* "no labels"; Gitea is always `None` —
    /// `tea`'s PR list/view has no labels column, so an empty list there would be a
    /// false "no labels" rather than the truthful "unknown".
    pub labels: Option<Vec<String>>,
    /// Usernames/logins of assigned users, or `None` when the backend can't report
    /// them. **Per-backend support:** GitHub (`gh --json assignees` → `login`) and
    /// GitLab (`assignees` → `username`) report `Some(..)` — an empty `Some(vec![])`
    /// is a *confirmed* "unassigned"; Gitea is always `None` — `tea`'s PR list/view
    /// has no assignees column.
    pub assignees: Option<Vec<String>>,
}

impl ForgePr {
    /// A PR/MR with the given number, title, and state; empty branches/url and
    /// **unknown** (`None`) draft/labels/assignees — chain the setters to record a
    /// confirmed value. Lets a custom [`ForgeApi`](crate::ForgeApi) backend or a
    /// test build one despite the `#[non_exhaustive]`.
    pub fn new(number: u64, title: impl Into<String>, state: ForgePrState) -> Self {
        Self {
            number,
            title: title.into(),
            state,
            source_branch: String::new(),
            target_branch: String::new(),
            url: String::new(),
            draft: None,
            labels: None,
            assignees: None,
        }
    }

    /// Set the source (head) branch.
    pub fn source_branch(mut self, branch: impl Into<String>) -> Self {
        self.source_branch = branch.into();
        self
    }

    /// Set the target (base) branch.
    pub fn target_branch(mut self, branch: impl Into<String>) -> Self {
        self.target_branch = branch.into();
        self
    }

    /// Set the web URL.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }

    /// Record a *confirmed* draft flag (`Some(draft)`) — pass `true` to mark it a
    /// draft, `false` to record a confirmed non-draft (distinct from the `None`
    /// "unknown" a fresh [`new`](ForgePr::new) starts with).
    pub fn draft(mut self, draft: bool) -> Self {
        self.draft = Some(draft);
        self
    }

    /// Record a *confirmed* label set (`Some(labels)`) — an empty list is a
    /// confirmed "no labels", distinct from the `None` "unknown".
    pub fn labels(mut self, labels: impl Into<Vec<String>>) -> Self {
        self.labels = Some(labels.into());
        self
    }

    /// Record a *confirmed* assignee set (`Some(assignees)`) — an empty list is a
    /// confirmed "unassigned", distinct from the `None` "unknown".
    pub fn assignees(mut self, assignees: impl Into<Vec<String>>) -> Self {
        self.assignees = Some(assignees.into());
        self
    }
}

/// The normalised state of a [`ForgePr`], unifying GitHub's `OPEN`/`CLOSED`/
/// `MERGED`, GitLab's `opened`/`closed`/`locked`/`merged`, and Gitea's
/// `open`/`closed` (+ a `merged` flag).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ForgePrState {
    /// Open / awaiting review.
    Open,
    /// Closed without merging (GitLab's `locked` folds in here too).
    Closed,
    /// Merged.
    Merged,
}

/// A repository (GitHub) / project (GitLab), unified. (Gitea's `tea` has no
/// current-repo view, so [`repo_view`](crate::ForgeApi::repo_view) is
/// [`Unsupported`](crate::Error::Unsupported) there.)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ForgeRepo {
    /// Repository / project name.
    pub name: String,
    /// Owner / namespace (GitHub owner login; GitLab the namespace path).
    pub owner: String,
    /// Default branch name (empty for an empty repo).
    pub default_branch: String,
    /// Web URL.
    pub url: String,
    /// Whether the repository is private/non-public, or `None` when the backend
    /// doesn't report visibility. **Per-backend support:** GitHub always reports a
    /// definite `Some(true)`/`Some(false)`; GitLab reports `Some(..)` when the
    /// project's `visibility` is present but `None` when `glab` omits it — an absent
    /// visibility is *unknown*, never a silent `Some(false)`, so a consumer is never
    /// told a repo is public/private without proof.
    pub private: Option<bool>,
}

impl ForgeRepo {
    /// A repo/project with the given name and owner; empty default-branch/url and
    /// **unknown** (`None`) visibility — chain the setters to record a confirmed
    /// value. For a custom [`ForgeApi`](crate::ForgeApi) backend or test.
    pub fn new(name: impl Into<String>, owner: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            owner: owner.into(),
            default_branch: String::new(),
            url: String::new(),
            private: None,
        }
    }

    /// Set the default branch name.
    pub fn default_branch(mut self, branch: impl Into<String>) -> Self {
        self.default_branch = branch.into();
        self
    }

    /// Set the web URL.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }

    /// Record a *confirmed* visibility (`Some(private)`) — pass `true` for
    /// private/non-public, `false` for a confirmed public repo (distinct from the
    /// `None` "unknown" a fresh [`new`](ForgeRepo::new) starts with).
    pub fn private(mut self, private: bool) -> Self {
        self.private = Some(private);
        self
    }
}

/// An issue, unified across the three forges.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ForgeIssue {
    /// The issue number a caller passes to the other operations (GitHub/Gitea
    /// `number`, GitLab `iid`).
    pub number: u64,
    /// Title.
    pub title: String,
    /// Normalised state (see [`ForgeIssueState`]).
    pub state: ForgeIssueState,
    /// Issue body (markdown). Populated by both
    /// [`issue_list`](crate::Forge::issue_list) and
    /// [`issue_view`](crate::Forge::issue_view) on every forge.
    pub body: String,
    /// Web URL. Populated by both [`issue_list`](crate::Forge::issue_list) and
    /// [`issue_view`](crate::Forge::issue_view) on every forge.
    pub url: String,
    /// Labels attached to the issue, or `None` when the backend can't report them.
    /// **Per-backend support:** GitHub and GitLab report `Some(..)` — an empty
    /// `Some(vec![])` is a *confirmed* "no labels"; Gitea is always `None` —
    /// `tea`'s issue list/view has no labels column.
    pub labels: Option<Vec<String>>,
    /// Usernames/logins of assigned users, or `None` when the backend can't report
    /// them. **Per-backend support:** GitHub (`gh --json assignees` → `login`) and
    /// GitLab (`assignees` → `username`) report `Some(..)` — an empty `Some(vec![])`
    /// is a *confirmed* "unassigned"; Gitea is always `None` — `tea`'s issue
    /// list/view has no assignees column.
    pub assignees: Option<Vec<String>>,
}

impl ForgeIssue {
    /// An issue with the given number, title, and state; empty body/url and
    /// **unknown** (`None`) labels/assignees — chain the setters to record a
    /// confirmed value. For a custom [`ForgeApi`](crate::ForgeApi) backend or test.
    pub fn new(number: u64, title: impl Into<String>, state: ForgeIssueState) -> Self {
        Self {
            number,
            title: title.into(),
            state,
            body: String::new(),
            url: String::new(),
            labels: None,
            assignees: None,
        }
    }

    /// Set the issue body (markdown).
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Set the web URL.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }

    /// Record a *confirmed* label set (`Some(labels)`) — an empty list is a
    /// confirmed "no labels", distinct from the `None` "unknown".
    pub fn labels(mut self, labels: impl Into<Vec<String>>) -> Self {
        self.labels = Some(labels.into());
        self
    }

    /// Record a *confirmed* assignee set (`Some(assignees)`) — an empty list is a
    /// confirmed "unassigned", distinct from the `None` "unknown".
    pub fn assignees(mut self, assignees: impl Into<Vec<String>>) -> Self {
        self.assignees = Some(assignees.into());
        self
    }
}

/// The normalised state of a [`ForgeIssue`], unifying GitHub's `OPEN`/`CLOSED`,
/// GitLab's `opened`/`closed`, and Gitea's `open`/`closed`. An unknown state
/// reads as [`Open`](ForgeIssueState::Open) — a state we don't model is treated
/// as live, never silently as resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ForgeIssueState {
    /// Open / unresolved.
    Open,
    /// Closed.
    Closed,
}

/// A release, unified across the three forges. (Gitea's `tea` always lists —
/// it has no single-release view — so
/// [`release_view`](crate::ForgeApi::release_view) is
/// [`Unsupported`](crate::Error::Unsupported) there.)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ForgeRelease {
    /// The Git tag the release is attached to (what
    /// [`release_view`](crate::ForgeApi::release_view) takes).
    pub tag: String,
    /// Release title (may be empty — forges commonly default it to the tag).
    pub title: String,
    /// Web URL, or `None` when the backend doesn't report one. **Per-backend
    /// support:** `release_view` fills `Some(..)` on GitHub and GitLab; GitHub's
    /// lean `release_list` doesn't request the URL, so it is `None` there, and
    /// Gitea is always `None` — `tea releases` exposes no release-page URL column.
    pub url: Option<String>,
    /// Publication timestamp (RFC 3339); `None` for an unpublished draft or
    /// when the backend doesn't report one.
    pub published_at: Option<String>,
    /// Release notes (markdown). `None` when the backend doesn't carry them —
    /// always on Gitea (`tea` has no release body), and on GitHub's lean
    /// `release_list` (only [`release_view`](crate::Forge::release_view) fills it).
    pub body: Option<String>,
    /// Whether this is an unpublished draft, or `None` when the backend has no
    /// draft concept. **Per-backend support:** GitHub and Gitea report a definite
    /// `Some(true)`/`Some(false)`; GitLab is always `None` — a GitLab release has
    /// no draft flag, so `Some(false)` would be a false "not a draft".
    pub draft: Option<bool>,
    /// Whether this is a pre-release, or `None` when the backend has no pre-release
    /// concept. **Per-backend support:** GitHub and Gitea report `Some(..)`; GitLab
    /// is always `None` — a GitLab release has no pre-release flag.
    pub prerelease: Option<bool>,
}

impl ForgeRelease {
    /// A release on `tag`; empty title, no url/timestamp/body and **unknown**
    /// (`None`) draft/pre-release — chain the setters to record a confirmed value.
    /// For a custom [`ForgeApi`](crate::ForgeApi) backend or test.
    pub fn new(tag: impl Into<String>) -> Self {
        Self {
            tag: tag.into(),
            title: String::new(),
            url: None,
            published_at: None,
            body: None,
            draft: None,
            prerelease: None,
        }
    }

    /// Set the release title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Set the web URL (`Some(url)`).
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set the publication timestamp (RFC 3339).
    pub fn published_at(mut self, ts: impl Into<String>) -> Self {
        self.published_at = Some(ts.into());
        self
    }

    /// Set the release notes (markdown) body.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Record a *confirmed* draft flag (`Some(draft)`) — pass `true` to mark it an
    /// unpublished draft, `false` for a confirmed published release (distinct from
    /// the `None` "unknown" a fresh [`new`](ForgeRelease::new) starts with).
    pub fn draft(mut self, draft: bool) -> Self {
        self.draft = Some(draft);
        self
    }

    /// Record a *confirmed* pre-release flag (`Some(prerelease)`) — distinct from
    /// the `None` "unknown" a fresh [`new`](ForgeRelease::new) starts with.
    pub fn prerelease(mut self, prerelease: bool) -> Self {
        self.prerelease = Some(prerelease);
        self
    }
}

/// The coarse CI status for a PR/MR, bucketed into the four states a caller acts
/// on. GitHub aggregates its per-check buckets into this; GitLab maps its
/// pipeline status; Gitea's `tea` has no checks command, so
/// [`pr_checks`](crate::ForgeApi::pr_checks) is
/// [`Unsupported`](crate::Error::Unsupported) there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum CiStatus {
    /// Everything that ran passed.
    Passing,
    /// At least one check failed or was canceled.
    Failing,
    /// At least one check is still running, and none failed.
    Pending,
    /// No checks/pipeline ran.
    None,
}

/// Options for [`pr_close`](crate::Forge::pr_close).
///
/// `#[non_exhaustive]`, so build it through [`PrClose::new`] and the chained
/// [`delete_branch`](PrClose::delete_branch) setter rather than a struct literal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PrClose {
    /// The PR/MR number to close.
    pub number: u64,
    /// Also delete the source branch. **GitHub only** (`gh pr close --delete-branch`):
    /// GitLab's `glab mr close` and Gitea's `tea pr close` have no such flag, so this
    /// is **ignored** on those backends.
    pub delete_branch: bool,
}

impl PrClose {
    /// Close PR/MR `number`, leaving the source branch in place.
    pub fn new(number: u64) -> Self {
        Self {
            number,
            delete_branch: false,
        }
    }

    /// Also delete the source branch (GitHub only; ignored on GitLab/Gitea).
    pub fn delete_branch(mut self) -> Self {
        self.delete_branch = true;
        self
    }
}

/// Options for [`pr_create`](crate::ForgeApi::pr_create) — the unified
/// open-a-PR/MR spec, mapped to each CLI's own flags (gh `--head`/`--base`,
/// glab `--source-branch`/`--target-branch`, tea `--head`/`--base`).
///
/// `#[non_exhaustive]`, so build it through [`PrCreate::new`] and the chained
/// setters rather than a struct literal.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PrCreate {
    /// Title.
    pub title: String,
    /// Body / description.
    pub body: String,
    /// Source (head) branch; `None` = the current branch.
    pub source: Option<String>,
    /// Target (base) branch; `None` = the repository default.
    pub target: Option<String>,
}

impl PrCreate {
    /// A PR/MR from the current branch into the repository's default branch.
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            source: None,
            target: None,
        }
    }

    /// Open from this source (head) branch instead of the current one.
    pub fn source(mut self, branch: impl Into<String>) -> Self {
        self.source = Some(branch.into());
        self
    }

    /// Open against this target (base) branch instead of the repo default.
    pub fn target(mut self, branch: impl Into<String>) -> Self {
        self.target = Some(branch.into());
        self
    }
}

/// Options for [`issue_create`](crate::Forge::issue_create) — the unified open-an-issue
/// spec, mirroring [`PrCreate`]'s shape.
///
/// `#[non_exhaustive]`, so build it through [`IssueCreate::new`] rather than a struct
/// literal — which also leaves room to grow (labels, assignees) without a breaking
/// signature change, the reason `issue_create` takes a spec rather than bare strings.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct IssueCreate {
    /// Title.
    pub title: String,
    /// Body / description (may be empty).
    pub body: String,
}

impl IssueCreate {
    /// An issue with `title` and `body`.
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
        }
    }
}

/// Options for [`release_create`](crate::Forge::release_create) — the unified
/// create-a-release spec, mapped to each CLI's own flags (gh `--title`/`--notes`,
/// glab `--name`/`--notes`, tea `--title`/`--note`).
///
/// `#[non_exhaustive]`, so build it through [`ReleaseCreate::new`] (the tag) and
/// the chained setters rather than a struct literal. Asset uploads are deliberately
/// **out of scope** — reach for the wrapper client (`gh release create` via
/// [`vcs_github`](crate::vcs_github), etc.) to attach files.
///
/// **Backend capability.** GitHub and Gitea support `draft`/`prerelease`; **GitLab
/// has no draft or pre-release concept**, so setting either returns
/// [`Unsupported`](crate::Error::Unsupported) on a GitLab handle rather than
/// silently ignoring it (mirroring [`PrMerge`]'s `auto`/`delete_branch`). The
/// default (neither set) creates a published release on every backend.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ReleaseCreate {
    /// The Git tag the release is attached to. GitHub creates the tag from the
    /// default branch if it doesn't yet exist; GitLab/Gitea expect it to exist.
    pub tag: String,
    /// The release title; `None` lets the forge default it (commonly to the tag).
    pub title: Option<String>,
    /// The release notes / description (markdown); `None` leaves it unset.
    pub notes: Option<String>,
    /// Save as a draft instead of publishing. **GitHub/Gitea only**; GitLab returns
    /// [`Unsupported`](crate::Error::Unsupported) when it is set.
    pub draft: bool,
    /// Mark as a prerelease. **GitHub/Gitea only**; GitLab returns
    /// [`Unsupported`](crate::Error::Unsupported) when it is set.
    pub prerelease: bool,
}

impl ReleaseCreate {
    /// A published release on `tag`, with the forge's default title/notes and
    /// neither draft nor prerelease. Chain the setters to change any of those.
    pub fn new(tag: impl Into<String>) -> Self {
        Self {
            tag: tag.into(),
            title: None,
            notes: None,
            draft: false,
            prerelease: false,
        }
    }

    /// Set the release title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the release notes / description.
    pub fn notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }

    /// Save as a draft. **GitHub/Gitea only** — GitLab returns
    /// [`Unsupported`](crate::Error::Unsupported) when it is set.
    pub fn draft(mut self) -> Self {
        self.draft = true;
        self
    }

    /// Mark as a prerelease. **GitHub/Gitea only** — GitLab returns
    /// [`Unsupported`](crate::Error::Unsupported) when it is set.
    pub fn prerelease(mut self) -> Self {
        self.prerelease = true;
        self
    }
}

/// How [`pr_merge`](crate::ForgeApi::pr_merge) merges — mapped to each CLI's own
/// merge-strategy flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MergeStrategy {
    /// A merge commit.
    Merge,
    /// Squash the commits into one.
    Squash,
    /// Rebase the source onto the target.
    Rebase,
}

/// Options for [`pr_merge`](crate::ForgeApi::pr_merge) — the unified merge spec
/// carrying a [`MergeStrategy`] plus the optional `auto` (merge once requirements
/// are met) and `delete_branch` (remove the source branch after merge) flags. It
/// generalises the per-CLI merge specs (`vcs-github`'s `PrMerge`, `vcs-gitlab`'s
/// `MrMerge`, `vcs-gitea`'s `PrMerge`) into one shape the facade drives across all
/// three backends.
///
/// `#[non_exhaustive]`, so build it through the strategy constructors —
/// [`merge`](PrMerge::merge) / [`squash`](PrMerge::squash) /
/// [`rebase`](PrMerge::rebase) (or [`new`](PrMerge::new) from a runtime
/// [`MergeStrategy`]) — then [`auto`](PrMerge::auto) /
/// [`delete_branch`](PrMerge::delete_branch), rather than a struct literal.
///
/// **Backend capability.** Only **GitHub** (`gh pr merge --auto --delete-branch`)
/// can express `auto`/`delete_branch`. On **GitLab** and **Gitea** these options
/// are not expressible, so requesting either returns
/// [`Unsupported`](crate::Error::Unsupported) rather than silently merging without
/// them — a deliberate contract, since for an irreversible merge a quietly dropped
/// option could produce the wrong side effects. The default (neither set) merges
/// on every backend.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PrMerge {
    /// The merge strategy.
    pub strategy: MergeStrategy,
    /// Enable auto-merge — merge once requirements/checks are met. **GitHub only**
    /// (`--auto`); GitLab/Gitea return [`Unsupported`](crate::Error::Unsupported)
    /// when it is set.
    pub auto: bool,
    /// Delete the source branch after the merge. **GitHub only**
    /// (`--delete-branch`); GitLab/Gitea return
    /// [`Unsupported`](crate::Error::Unsupported) when it is set.
    pub delete_branch: bool,
}

impl PrMerge {
    /// A merge with the given strategy, `auto`/`delete_branch` off — the runtime
    /// entry point (e.g. mapping an MCP argument) alongside the named
    /// [`merge`](PrMerge::merge)/[`squash`](PrMerge::squash)/[`rebase`](PrMerge::rebase)
    /// constructors.
    pub fn new(strategy: MergeStrategy) -> Self {
        Self {
            strategy,
            auto: false,
            delete_branch: false,
        }
    }

    /// Merge with a merge commit.
    pub fn merge() -> Self {
        Self::new(MergeStrategy::Merge)
    }

    /// Squash-merge.
    pub fn squash() -> Self {
        Self::new(MergeStrategy::Squash)
    }

    /// Rebase-merge.
    pub fn rebase() -> Self {
        Self::new(MergeStrategy::Rebase)
    }

    /// Enable auto-merge (merge once requirements are met). **GitHub only** —
    /// GitLab/Gitea report [`Unsupported`](crate::Error::Unsupported) when it is
    /// set.
    pub fn auto(mut self) -> Self {
        self.auto = true;
        self
    }

    /// Delete the source branch after merging. **GitHub only** — GitLab/Gitea
    /// report [`Unsupported`](crate::Error::Unsupported) when it is set.
    pub fn delete_branch(mut self) -> Self {
        self.delete_branch = true;
        self
    }
}

/// Options for [`pr_edit`](crate::ForgeApi::pr_edit) — the unified
/// edit-a-PR/MR spec, mapped to each CLI's own flags
/// (gh `--title`/`--body`, glab `--title`/`--description`, tea
/// `--title`/`--description`).
///
/// `#[non_exhaustive]`, so build it through [`PrEdit::new`] and the chained
/// setters rather than a struct literal. At least one of `title` or `body` must
/// be `Some`; both `None` is rejected by the facade before spawning (an explicit
/// error, not a silent no-op). An empty string is a real value (clears the
/// field) — not a `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct PrEdit {
    /// The new title (`--title`); `None` leaves the title alone.
    pub title: Option<String>,
    /// The new body / description (`--body` / `--description`); `None` leaves
    /// the body alone.
    pub body: Option<String>,
}

impl PrEdit {
    /// An edit that leaves both fields alone (the facade rejects both-`None`
    /// before reaching the wrapper). Start with this and add what you want to
    /// change via [`title`](PrEdit::title) / [`body`](PrEdit::body).
    pub fn new() -> Self {
        Self {
            title: None,
            body: None,
        }
    }

    /// Set the new title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the new body / description.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }
}

impl Default for PrEdit {
    fn default() -> Self {
        Self::new()
    }
}

/// The flat capability map for a configured forge — what its CLI is honest
/// about doing, intersected with whether the CLI is authenticated. Returned by
/// [`Forge::capabilities`](crate::Forge::capabilities); the
/// [`forge_info`](crate::ForgeApi::capabilities) MCP tool surfaces it as JSON.
///
/// Each `bool` is `true` iff the operation is available on this forge's CLI **and**
/// the CLI reports an authenticated session. The split between "the CLI ships
/// the command" and "the user is logged in" is preserved by the `authed` field
/// itself; a consumer that needs only one of the two can read it directly.
///
/// This is the **auth-gated action menu** — a different surface from
/// [`supports`](crate::Forge::supports)/[`ForgeOp`], which reports only the
/// *static* set of capability-*varying* operations (the ones some backends lack,
/// e.g. `repo_view`) without an auth probe. The two answer different questions and
/// deliberately do not share a field set.
///
/// The map is the intersection of three facts, so it never advertises an operation
/// the caller can't actually run: (1) the CLI ships the command, (2) the installed
/// CLI meets the wrapper's declared version floor ([`supported`](Self::supported)),
/// and (3) the CLI reports an authenticated session ([`authed`](Self::authed)). A
/// CLI **below the version floor** zeroes the per-op flags exactly like an unauthed
/// one — an old `gh`/`glab`/`tea` that lacks the modern command surface is honestly
/// reported as unable to perform the ops, rather than advertising a command that
/// would fail deep with a cryptic `unknown flag`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ForgeCapabilities {
    /// The CLI can open a PR/MR.
    pub pr_create: bool,
    /// The CLI can post a comment to an existing PR/MR.
    pub pr_comment: bool,
    /// The CLI can edit a PR/MR's title and/or body.
    pub pr_edit: bool,
    /// The CLI can report a PR/MR's CI status (passing / failing / pending /
    /// none).
    pub pr_checks: bool,
    /// The CLI can merge a PR/MR.
    pub pr_merge: bool,
    /// The CLI can submit an approving review (`gh pr review --approve` / `glab mr
    /// approve` / `tea pr approve`).
    pub pr_approve: bool,
    /// The CLI can submit a request-changes review (`gh pr review
    /// --request-changes` / `tea pr reject`). **Always `false` for GitLab** even
    /// when authed — GitLab's review model is approve/revoke, with no
    /// request-changes action.
    pub pr_request_changes: bool,
    /// The CLI can open an issue.
    pub issue_create: bool,
    /// The CLI can close an issue (`gh issue close` / `glab issue close` / `tea
    /// issues close`).
    pub issue_close: bool,
    /// The CLI can reopen a closed issue (`gh issue reopen` / `glab issue reopen` /
    /// `tea issues reopen`).
    pub issue_reopen: bool,
    /// The CLI can post a comment to an existing issue (`gh issue comment` / `glab
    /// issue note` / `tea comment`).
    pub issue_comment: bool,
    /// The CLI can create a release (`gh release create` / `glab release create` /
    /// `tea releases create`). The `draft`/`prerelease` *options* are a separate
    /// GitLab gap (see [`ReleaseCreate`]); this flag only reports that the CLI ships
    /// the create command at all — `true` for every authed, modern backend.
    pub release_create: bool,
    /// The CLI can delete a release by tag (`gh release delete` / `glab release
    /// delete` / `tea releases delete`).
    pub release_delete: bool,
    /// The installed CLI's parsed version (`gh`/`glab`/`tea --version`), or `None`
    /// when the backend is [`Unknown`](ForgeKind::Unknown) (no CLI) or the
    /// `--version` banner was unrecognisable. Read it to report the concrete
    /// version behind [`supported`](Self::supported).
    pub version: Option<vcs_diff::Version>,
    /// Whether the installed CLI meets the wrapper's declared **version floor** (the
    /// minimum `gh`/`glab`/`tea` whose command surface this toolkit models). `false`
    /// for a too-old CLI, an [`Unknown`](ForgeKind::Unknown) backend, or an
    /// unrecognisable version — in every such case the per-op flags above are all
    /// `false`, since the ops can't be guaranteed. The version twin of
    /// [`authed`](Self::authed): both must hold for an op flag to be `true`.
    pub supported: bool,
    /// The CLI reports an authenticated session. The six op flags are all
    /// `false` when this is `false` (or when [`supported`](Self::supported) is);
    /// the spec's per-op table is the intersection. **Best-effort for GitLab:**
    /// `glab auth status` can exit `0` while unauthenticated ([gitlab-org/cli#911]),
    /// so a `true` here means "probably authed" for the GitLab backend; a real API
    /// call is the only sure test. GitHub/Gitea probes are faithful.
    ///
    /// [gitlab-org/cli#911]: https://gitlab.com/gitlab-org/cli/-/issues/911
    pub authed: bool,
}

impl ForgeCapabilities {
    /// The all-`false` shape, for the [`Unknown`](ForgeKind::Unknown) case and
    /// as the trait's defaulted answer for any external implementer: no op, no
    /// known version, unsupported, unauthed.
    pub fn all_false() -> Self {
        Self {
            pr_create: false,
            pr_comment: false,
            pr_edit: false,
            pr_checks: false,
            pr_merge: false,
            pr_approve: false,
            pr_request_changes: false,
            issue_create: false,
            issue_close: false,
            issue_reopen: false,
            issue_comment: false,
            release_create: false,
            release_delete: false,
            version: None,
            supported: false,
            authed: false,
        }
    }

    /// Mark `pr_create` available. Chain from [`all_false`](Self::all_false) to
    /// report what a custom `ForgeApi` backend's `capabilities()` override supports
    /// (the struct is `#[non_exhaustive]`, so this is the way to build a non-all-false
    /// map): `ForgeCapabilities::all_false().pr_create().pr_merge().authed()`.
    pub fn pr_create(mut self) -> Self {
        self.pr_create = true;
        self
    }

    /// Mark `pr_comment` available.
    pub fn pr_comment(mut self) -> Self {
        self.pr_comment = true;
        self
    }

    /// Mark `pr_edit` available.
    pub fn pr_edit(mut self) -> Self {
        self.pr_edit = true;
        self
    }

    /// Mark `pr_checks` available.
    pub fn pr_checks(mut self) -> Self {
        self.pr_checks = true;
        self
    }

    /// Mark `pr_merge` available.
    pub fn pr_merge(mut self) -> Self {
        self.pr_merge = true;
        self
    }

    /// Mark `pr_approve` available.
    pub fn pr_approve(mut self) -> Self {
        self.pr_approve = true;
        self
    }

    /// Mark `pr_request_changes` available.
    pub fn pr_request_changes(mut self) -> Self {
        self.pr_request_changes = true;
        self
    }

    /// Mark `issue_create` available.
    pub fn issue_create(mut self) -> Self {
        self.issue_create = true;
        self
    }

    /// Mark `release_create` available.
    pub fn release_create(mut self) -> Self {
        self.release_create = true;
        self
    }

    /// Mark `release_delete` available.
    pub fn release_delete(mut self) -> Self {
        self.release_delete = true;
        self
    }

    /// Record the installed CLI version (`Some(version)`).
    pub fn version(mut self, version: vcs_diff::Version) -> Self {
        self.version = Some(version);
        self
    }

    /// Mark the installed CLI as meeting the version floor.
    pub fn supported(mut self) -> Self {
        self.supported = true;
        self
    }

    /// Mark the CLI authenticated.
    pub fn authed(mut self) -> Self {
        self.authed = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_remote_url_classifies_saas_hosts() {
        use ForgeKind::*;
        for (url, want) in [
            ("https://github.com/o/r.git", Some(GitHub)),
            ("git@github.com:o/r.git", Some(GitHub)),
            ("https://foo.github.com/o/r", Some(GitHub)), // proper subdomain
            ("https://gitlab.com/o/r", Some(GitLab)),
            ("https://user:pass@gitlab.com/o/r", Some(GitLab)), // userinfo stripped
            ("ssh://git@gitlab.com:22/o/r.git", Some(GitLab)),
            ("https://gitea.com/o/r.git", Some(Gitea)),
            ("git@codeberg.org:o/r.git", Some(Gitea)),
            ("https://docs.codeberg.org/o/r", Some(Gitea)), // proper subdomain
        ] {
            assert_eq!(ForgeKind::from_remote_url(url), want, "{url}");
        }
    }

    // A self-hosted instance on an arbitrary domain, and — crucially — a
    // *lookalike* host an attacker controls, must NOT be classified as a trusted
    // forge: the safe answer is `None` (the caller picks the kind explicitly).
    #[test]
    fn from_remote_url_rejects_self_hosted_and_lookalikes() {
        for url in [
            "https://gitlab.example.com/o/r.git",  // self-hosted GitLab
            "https://gitea.example.org/o/r.git",   // self-hosted Gitea
            "https://git.acme.io/o/r.git",         // arbitrary
            "https://gitlab.com.attacker.net/o/r", // lookalike — must not be GitLab
            "git@gitlab.attacker.com:o/r.git",     // lookalike
            "https://my-gitea-host.evil.com/o/r",  // substring spoof — must not be Gitea
            "https://notgithub.com/o/r",           // suffix without the dot
            "https://github.com.evil.example/o/r", // lookalike — must not be GitHub
            "",
        ] {
            assert_eq!(ForgeKind::from_remote_url(url), None, "{url}");
        }
    }

    // `host_of` is bracket-aware for IPv6 scheme-URL authorities — it returns the
    // address inside the brackets, not the literal `[`. (No SaaS host is an IPv6
    // literal, so `from_remote_url` still answers `None`, but the host extraction
    // itself is correct for any future consumer.)
    #[test]
    fn host_of_extracts_ipv6_authority() {
        assert_eq!(host_of("https://[::1]:443/o/r.git"), Some("::1"));
        assert_eq!(host_of("https://[2001:db8::1]/o/r"), Some("2001:db8::1"));
        assert_eq!(host_of("ssh://git@[fe80::1]:22/o/r.git"), Some("fe80::1"));
        // Regular hosts are unaffected by the bracket branch.
        assert_eq!(
            host_of("https://github.com:443/o/r.git"),
            Some("github.com")
        );
        assert_eq!(host_of("git@gitlab.com:o/r.git"), Some("gitlab.com"));
        // An IPv6 literal is never a trusted SaaS host.
        assert_eq!(ForgeKind::from_remote_url("https://[::1]/o/r"), None);
    }

    // A bracketed authority is unwrapped ONLY when it is a genuine IPv6 literal.
    // A bracketed *name* — or a colon-bearing fake crafted to slip past a naive
    // "contains a colon" check and then match a `.trusted` proper-subdomain suffix
    // (`[a:b.gitlab.com]`) — must NOT be unwrapped, or it could spoof a trusted
    // SaaS forge. The `Ipv6Addr` parse rejects every one of these → `None`.
    #[test]
    fn host_of_rejects_bracketed_name_spoof() {
        for url in [
            "https://[gitlab.com]/o/r",
            "https://[gitlab.com]:443/o/r",
            "https://[github.com]/o/r",
            "https://[gitea.com]/o/r",
            "https://[codeberg.org]/o/r",
            "https://[]/o/r",                 // empty brackets
            "https://[evil.gitlab.com]/o/r",  // bracketed subdomain-looking name
            "https://[a:b.gitlab.com]/o/r",   // colon-bearing fake ending in .gitlab.com
            "https://[x:y.github.com]/o/r",   // ditto for github.com
            "https://[::ffff:gitea.com]/o/r", // IPv4-mapped-looking fake, not real IPv6
            "https://[a:b.c.codeberg.org]/o/r",
        ] {
            assert_eq!(
                ForgeKind::from_remote_url(url),
                None,
                "bracketed non-IPv6 authority must not classify as a trusted forge: {url}"
            );
        }
        // The host extraction itself yields no host for a non-IPv6 bracket.
        assert_eq!(host_of("https://[gitlab.com]/o/r"), None);
        assert_eq!(host_of("https://[a:b.gitlab.com]/o/r"), None);
    }

    #[test]
    fn as_str_maps_each_kind() {
        assert_eq!(ForgeKind::GitHub.as_str(), "github");
        assert_eq!(ForgeKind::GitLab.as_str(), "gitlab");
        assert_eq!(ForgeKind::Gitea.as_str(), "gitea");
        assert_eq!(ForgeKind::Unknown.as_str(), "unknown");
    }

    #[test]
    fn pr_edit_default_has_both_fields_none() {
        let edit = PrEdit::new();
        assert!(edit.title.is_none());
        assert!(edit.body.is_none());
    }

    #[test]
    fn forge_capabilities_all_false_is_uniform() {
        let c = ForgeCapabilities::all_false();
        assert!(!c.pr_create);
        assert!(!c.pr_comment);
        assert!(!c.pr_edit);
        assert!(!c.pr_checks);
        assert!(!c.pr_merge);
        assert!(!c.pr_approve);
        assert!(!c.pr_request_changes);
        assert!(!c.issue_create);
        // No known version, unsupported, unauthed — the honest "no CLI" shape.
        assert_eq!(c.version, None);
        assert!(!c.supported);
        assert!(!c.authed);
    }

    // A4: the public builders let a custom `ForgeApi` backend / test build the
    // `#[non_exhaustive]` return DTOs, landing fields where expected.
    #[test]
    fn forge_dto_constructors_populate_fields() {
        let pr = ForgePr::new(7, "Add widget", ForgePrState::Open)
            .source_branch("feature")
            .target_branch("main")
            .url("https://x/pr/7")
            .draft(true)
            .labels(vec!["bug".to_string()])
            .assignees(vec!["octocat".to_string()]);
        assert_eq!(pr.number, 7);
        assert_eq!(pr.title, "Add widget");
        assert_eq!(pr.state, ForgePrState::Open);
        assert_eq!(pr.source_branch, "feature");
        assert_eq!(pr.target_branch, "main");
        assert_eq!(pr.url, "https://x/pr/7");
        assert_eq!(pr.draft, Some(true));
        assert_eq!(pr.labels, Some(vec!["bug".to_string()]));
        assert_eq!(pr.assignees, Some(vec!["octocat".to_string()]));
        // A fresh PR leaves the support-gated fields `None` (unknown), and
        // `.draft(false)` records a *confirmed* non-draft (not the same as `None`).
        let bare = ForgePr::new(8, "Bare", ForgePrState::Open);
        assert_eq!(bare.draft, None);
        assert_eq!(bare.labels, None);
        assert_eq!(bare.assignees, None);
        assert_eq!(
            ForgePr::new(9, "x", ForgePrState::Open).draft(false).draft,
            Some(false)
        );

        let repo = ForgeRepo::new("proj", "acme")
            .default_branch("main")
            .url("https://x/proj")
            .private(true);
        assert_eq!(repo.name, "proj");
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.default_branch, "main");
        assert_eq!(repo.url, "https://x/proj");
        assert_eq!(repo.private, Some(true));
        assert_eq!(ForgeRepo::new("p", "o").private, None);
        assert_eq!(ForgeRepo::new("p", "o").private(false).private, Some(false));

        let issue = ForgeIssue::new(3, "Bug", ForgeIssueState::Closed)
            .body("desc")
            .url("https://x/i/3")
            .labels(vec!["wontfix".to_string()])
            .assignees(vec!["andyfeller".to_string()]);
        assert_eq!(issue.number, 3);
        assert_eq!(issue.title, "Bug");
        assert_eq!(issue.state, ForgeIssueState::Closed);
        assert_eq!(issue.body, "desc");
        assert_eq!(issue.url, "https://x/i/3");
        assert_eq!(issue.labels, Some(vec!["wontfix".to_string()]));
        assert_eq!(issue.assignees, Some(vec!["andyfeller".to_string()]));
        assert_eq!(ForgeIssue::new(4, "x", ForgeIssueState::Open).labels, None);

        let rel = ForgeRelease::new("v1.0")
            .title("First")
            .url("https://x/rel/v1.0")
            .published_at("2026-07-03T10:00:00+02:00")
            .body("notes")
            .draft(true)
            .prerelease(true);
        assert_eq!(rel.url.as_deref(), Some("https://x/rel/v1.0"));
        assert_eq!(rel.draft, Some(true));
        assert_eq!(rel.tag, "v1.0");
        assert_eq!(rel.title, "First");
        assert_eq!(
            rel.published_at.as_deref(),
            Some("2026-07-03T10:00:00+02:00")
        );
        assert_eq!(rel.body.as_deref(), Some("notes"));
        assert_eq!(rel.prerelease, Some(true));
        // A fresh release leaves url/draft/prerelease `None` (unknown).
        let bare_rel = ForgeRelease::new("v2.0");
        assert_eq!(bare_rel.url, None);
        assert_eq!(bare_rel.draft, None);
        assert_eq!(bare_rel.prerelease, None);
        assert_eq!(ForgeRelease::new("v3").draft(false).draft, Some(false));

        // ForgeCapabilities builds a non-all-false map for a custom backend.
        let caps = ForgeCapabilities::all_false()
            .pr_create()
            .pr_merge()
            .version(vcs_diff::Version {
                major: 2,
                minor: 40,
                patch: 1,
            })
            .supported()
            .authed();
        assert!(caps.pr_create && caps.pr_merge && caps.authed);
        assert!(caps.supported);
        assert_eq!(
            caps.version,
            Some(vcs_diff::Version {
                major: 2,
                minor: 40,
                patch: 1
            })
        );
        assert!(!caps.pr_comment && !caps.pr_edit && !caps.pr_checks && !caps.issue_create);
        assert!(!caps.pr_approve && !caps.pr_request_changes);
        // The remaining setters land their own fields too.
        let rest = ForgeCapabilities::all_false()
            .pr_comment()
            .pr_edit()
            .pr_checks()
            .pr_approve()
            .pr_request_changes()
            .issue_create();
        assert!(rest.pr_comment && rest.pr_edit && rest.pr_checks && rest.issue_create);
        assert!(rest.pr_approve && rest.pr_request_changes);
        assert!(!rest.pr_create && !rest.pr_merge && !rest.authed);
    }
}

// Property-based fuzzing of `from_remote_url`. The URL/host parsing slices on
// `://`, `@`, `:`, and `/` and must never panic on a hostile string; and the
// anchored `host_is` match must never classify a *lookalike* host (an
// attacker-controlled `github.com.evil.net`) as a trusted forge — the
// regression net for the unit tests above, which only cover hand-picked cases.
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// A URL shape embedding `host` in each position `from_remote_url` parses —
    /// scheme URLs (with/without userinfo and port) and the scp-like form — so a
    /// lookalike host is tested wherever it could appear.
    fn url_around(host: impl Strategy<Value = String>) -> impl Strategy<Value = String> {
        host.prop_flat_map(|h| {
            prop_oneof![
                Just(format!("https://{h}/o/r.git")),
                Just(format!("https://user:pass@{h}/o/r")),
                Just(format!("ssh://git@{h}:22/o/r.git")),
                Just(format!("git@{h}:o/r.git")),
                Just(format!("{h}/o/r")),
                // Bracketed forms — a bracketed *name* must never be unwrapped into
                // a trusted host (the IPv6-aware `host_of` guards on a colon).
                Just(format!("https://[{h}]/o/r")),
                Just(format!("https://[{h}]:443/o/r")),
            ]
        })
    }

    /// Hosts that merely *resemble* a trusted SaaS host but aren't it: a trusted
    /// domain as a left label (`github.com.evil.net`), a no-dot suffix
    /// (`notgithub.com`), or the trusted domain buried mid-host — every one must
    /// classify as `None`.
    fn lookalike_host() -> impl Strategy<Value = String> {
        // `prop_oneof!` consumes its strategies, so name the reusable ones as
        // closures that build a fresh strategy at each use site.
        let trusted = || {
            prop_oneof![
                Just("github.com"),
                Just("gitlab.com"),
                Just("gitea.com"),
                Just("codeberg.org"),
            ]
        };
        // TLDs disjoint from every trusted domain's (`com`/`org`), so a generated
        // suffix can never BE a trusted domain — `github.com.gitea.com` would be
        // a genuine subdomain of gitea.com and *correctly* classify, which is not
        // what this strategy probes.
        let evil = || "[a-z]{1,8}\\.(net|io|dev|xyz)";
        prop_oneof![
            // Trusted domain as a *prefix* label of an attacker domain.
            (trusted(), evil()).prop_map(|(t, e)| format!("{t}.{e}")),
            // Trusted domain glued on with no separating dot.
            (prop_oneof![Just("not"), Just("my"), Just("x")], trusted())
                .prop_map(|(p, t)| format!("{p}{t}")),
            // Trusted domain buried as an *inner* label, not the suffix.
            (evil(), trusted()).prop_map(|(e, t)| format!("x.{t}.{e}")),
        ]
    }

    proptest! {
        // Panic-freedom on completely arbitrary input.
        #[test]
        fn from_remote_url_never_panics(s in any::<String>()) {
            let _ = ForgeKind::from_remote_url(&s);
        }

        // A lookalike host must NEVER be classified as a trusted forge.
        #[test]
        fn from_remote_url_rejects_lookalikes(url in url_around(lookalike_host())) {
            prop_assert_eq!(
                ForgeKind::from_remote_url(&url),
                None,
                "lookalike must not classify: {}",
                url
            );
        }

        // A bracketed authority whose content ends in a *trusted* domain but has a
        // colon to its left (`https://[<junk>:<more>.gitlab.com]/…`) is crafted to
        // pass a naive "looks like IPv6 (has a colon)" check and then satisfy the
        // `.gitlab.com` proper-subdomain test. The `Ipv6Addr` parse rejects all of
        // them — none is a valid literal — so they must classify as `None`.
        #[test]
        fn from_remote_url_rejects_colon_bracket_trusted_suffix(
            left in "[a-z0-9:]{0,12}",
            trusted in prop_oneof![
                Just("github.com"),
                Just("gitlab.com"),
                Just("gitea.com"),
                Just("codeberg.org"),
            ],
        ) {
            let url = format!("https://[{left}:x.{trusted}]/o/r");
            prop_assert_eq!(
                ForgeKind::from_remote_url(&url),
                None,
                "colon-bracket trusted-suffix spoof must not classify: {}",
                url
            );
        }
    }
}

// The optional `serde` feature derives `Serialize` on the unified DTOs.
#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use super::*;

    #[test]
    fn forge_pr_serializes_to_clean_json() {
        let pr = ForgePr {
            number: 7,
            title: "Add X".into(),
            state: ForgePrState::Merged,
            source_branch: "feat".into(),
            target_branch: "main".into(),
            url: "u".into(),
            draft: Some(false),
            labels: Some(vec!["bug".into()]),
            assignees: Some(vec!["octocat".into()]),
        };
        let v = serde_json::to_value(&pr).unwrap();
        assert_eq!(v["number"], 7);
        assert_eq!(v["state"], "Merged"); // enum → variant name
        assert_eq!(v["source_branch"], "feat");
        // A *confirmed* `Some(false)`/`Some([..])` serialises to the plain value,
        // not `null` — the wire tells "confirmed" apart from "unknown".
        assert_eq!(v["draft"], false);
        assert_eq!(v["labels"], serde_json::json!(["bug"]));
        assert_eq!(v["assignees"], serde_json::json!(["octocat"]));
    }

    // The support contract on the wire: a `None` (backend can't report the field)
    // serialises to JSON `null`, distinct from a *confirmed* `Some(false)` / empty
    // `Some([])`. This is the MCP JSON contract for "unknown vs confirmed
    // false/empty" — pin it so a consumer can rely on the distinction.
    #[test]
    fn unknown_fields_serialize_as_null_distinct_from_confirmed() {
        // Gitea-shaped PR: draft/labels/assignees are unknown (`None` → `null`).
        let unknown = ForgePr {
            number: 1,
            title: "t".into(),
            state: ForgePrState::Open,
            source_branch: "f".into(),
            target_branch: "main".into(),
            url: "u".into(),
            draft: None,
            labels: None,
            assignees: None,
        };
        let v = serde_json::to_value(&unknown).unwrap();
        assert!(v["draft"].is_null(), "unknown draft must be null");
        assert!(v["labels"].is_null(), "unknown labels must be null");
        assert!(v["assignees"].is_null(), "unknown assignees must be null");

        // A confirmed *empty* label set is `[]`, NOT `null` — "we asked and there
        // are none" reads differently from "we couldn't ask".
        let confirmed_empty = ForgePr {
            labels: Some(Vec::new()),
            assignees: Some(Vec::new()),
            draft: Some(false),
            ..unknown.clone()
        };
        let v = serde_json::to_value(&confirmed_empty).unwrap();
        assert_eq!(v["labels"], serde_json::json!([]));
        assert_eq!(v["assignees"], serde_json::json!([]));
        assert_eq!(v["draft"], false);

        // ForgeRepo: absent visibility → `null`; confirmed public → `false`.
        let repo_unknown = ForgeRepo::new("p", "o");
        assert!(serde_json::to_value(&repo_unknown).unwrap()["private"].is_null());
        let repo_public = ForgeRepo::new("p", "o").private(false);
        assert_eq!(
            serde_json::to_value(&repo_public).unwrap()["private"],
            false
        );

        // ForgeRelease: GitLab-shaped (no draft/pre-release concept, url unknown).
        let rel_unknown = ForgeRelease::new("v1");
        let v = serde_json::to_value(&rel_unknown).unwrap();
        assert!(v["url"].is_null(), "unknown url must be null");
        assert!(v["draft"].is_null(), "no-draft-concept must be null");
        assert!(
            v["prerelease"].is_null(),
            "no-prerelease-concept must be null"
        );
    }

    // The Wave-A DTOs are part of vcs-mcp's JSON wire format — pin their shape:
    // the state enum serializes as the variant name, an absent publish date as
    // `null`, and the PrCreate spec keeps its field names.
    #[test]
    fn issue_release_and_pr_create_serialize_to_clean_json() {
        let issue = ForgeIssue {
            number: 3,
            title: "Bug".into(),
            state: ForgeIssueState::Closed,
            body: "b".into(),
            url: "u".into(),
            labels: Some(vec!["wontfix".into()]),
            assignees: Some(Vec::new()),
        };
        let v = serde_json::to_value(&issue).unwrap();
        assert_eq!(v["number"], 3);
        assert_eq!(v["state"], "Closed");
        assert_eq!(v["body"], "b");
        assert_eq!(v["labels"], serde_json::json!(["wontfix"]));
        assert_eq!(v["assignees"], serde_json::json!([]));

        let release = ForgeRelease {
            tag: "v1".into(),
            title: "One".into(),
            url: Some("u".into()),
            published_at: None,
            body: Some("notes".into()),
            draft: Some(false),
            prerelease: Some(true),
        };
        let v = serde_json::to_value(&release).unwrap();
        assert_eq!(v["tag"], "v1");
        assert_eq!(v["url"], "u");
        assert!(v["published_at"].is_null(), "draft date must be null");
        assert_eq!(v["body"], "notes");
        assert_eq!(v["draft"], false);
        assert_eq!(v["prerelease"], true);

        let spec = PrCreate::new("T", "B").source("feat");
        let v = serde_json::to_value(&spec).unwrap();
        assert_eq!(v["title"], "T");
        assert_eq!(v["source"], "feat");
        assert!(v["target"].is_null());
    }

    // `pr_diff` returns `vcs-diff`'s model directly — pin that the `serde`
    // feature forward (`vcs-diff/serde`, wired in `Cargo.toml`) actually turns
    // on `Serialize` there too, not just on this crate's own DTOs.
    #[test]
    fn file_diff_serializes_when_serde_feature_forwards() {
        let files =
            vcs_diff::parse_diff("diff --git a/m b/m\n--- a/m\n+++ b/m\n@@ -1 +1 @@\n-a\n+b\n");
        let v = serde_json::to_value(&files).unwrap();
        assert_eq!(v[0]["path"], "m");
        assert_eq!(v[0]["change"], "Modified");
    }
}
