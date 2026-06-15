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
/// `ssh://git@host:22/…`) and scp-like (`git@host:owner/repo.git`).
fn host_of(url: &str) -> Option<&str> {
    let rest = match url.split_once("://") {
        // A scheme URL: take the authority up to the next `/`, then drop userinfo.
        Some((_scheme, after)) => {
            let authority = after.split(['/', '?', '#']).next().unwrap_or(after);
            let host_port = authority.rsplit('@').next().unwrap_or(authority);
            return host_port.split(':').next().filter(|h| !h.is_empty());
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

/// A facade operation whose availability varies by backend — i.e. one that can
/// return [`Unsupported`](crate::Error::Unsupported). Pass it to
/// [`Forge::supports`](crate::Forge::supports) to branch *before* calling, so a
/// consumer (an agent, a TUI) hides an unavailable button instead of issuing the
/// call and handling the error. Every other facade operation is supported on all
/// three forges.
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
}

impl ForgeOp {
    /// Every capability-varying operation — iterate it to build a full support
    /// matrix (e.g. to render an availability list).
    pub const ALL: &'static [ForgeOp] = &[
        ForgeOp::RepoView,
        ForgeOp::PrMarkReady,
        ForgeOp::PrChecks,
        ForgeOp::ReleaseView,
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
    /// Whether the PR/MR is a draft. **Best-effort**: only GitLab reports it on
    /// the lean surface; GitHub and Gitea report `false` here (their lean JSON
    /// doesn't carry the draft flag).
    pub draft: bool,
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
    /// Whether the repository is private/non-public. **Conservative when
    /// unknown:** if the backend doesn't report visibility (e.g. GitLab omits the
    /// field), this is `false` (public) rather than `true` — a consumer is never
    /// told a repo is private without proof.
    pub private: bool,
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
    /// Web URL. **Best-effort:** empty from GitHub's lean `release_list`;
    /// `release_view` fills it where supported.
    pub url: String,
    /// Publication timestamp (ISO 8601); `None` for an unpublished draft or
    /// when the backend doesn't report one.
    pub published_at: Option<String>,
    /// Release notes (markdown). `None` when the backend doesn't carry them —
    /// always on Gitea (`tea` has no release body), and on GitHub's lean
    /// `release_list` (only [`release_view`](crate::Forge::release_view) fills it).
    pub body: Option<String>,
    /// Whether this is an unpublished draft. **Best-effort:** GitHub and Gitea
    /// report it; GitLab has no draft concept, so it is always `false` there.
    pub draft: bool,
    /// Whether this is a pre-release. **Best-effort:** GitHub and Gitea report it;
    /// GitLab has no pre-release concept, so it is always `false` there.
    pub prerelease: bool,
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
    /// The CLI can open an issue.
    pub issue_create: bool,
    /// The CLI reports an authenticated session. The other six flags are all
    /// `false` when this is `false`; the spec's per-op table is the
    /// intersection. **Best-effort for GitLab:** `glab auth status` can exit `0`
    /// while unauthenticated ([gitlab-org/cli#911]), so a `true` here means
    /// "probably authed" for the GitLab backend; a real API call is the only sure
    /// test. GitHub/Gitea probes are faithful.
    ///
    /// [gitlab-org/cli#911]: https://gitlab.com/gitlab-org/cli/-/issues/911
    pub authed: bool,
}

impl ForgeCapabilities {
    /// The all-`false` shape, for the [`Unknown`](ForgeKind::Unknown) case and
    /// as the trait's defaulted answer for any external implementer.
    pub fn all_false() -> Self {
        Self {
            pr_create: false,
            pr_comment: false,
            pr_edit: false,
            pr_checks: false,
            pr_merge: false,
            issue_create: false,
            authed: false,
        }
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
        assert!(!c.issue_create);
        assert!(!c.authed);
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
            draft: false,
        };
        let v = serde_json::to_value(&pr).unwrap();
        assert_eq!(v["number"], 7);
        assert_eq!(v["state"], "Merged"); // enum → variant name
        assert_eq!(v["source_branch"], "feat");
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
        };
        let v = serde_json::to_value(&issue).unwrap();
        assert_eq!(v["number"], 3);
        assert_eq!(v["state"], "Closed");
        assert_eq!(v["body"], "b");

        let release = ForgeRelease {
            tag: "v1".into(),
            title: "One".into(),
            url: "u".into(),
            published_at: None,
            body: Some("notes".into()),
            draft: false,
            prerelease: true,
        };
        let v = serde_json::to_value(&release).unwrap();
        assert_eq!(v["tag"], "v1");
        assert!(v["published_at"].is_null(), "draft date must be null");
        assert_eq!(v["body"], "notes");
        assert_eq!(v["prerelease"], true);

        let spec = PrCreate::new("T", "B").source("feat");
        let v = serde_json::to_value(&spec).unwrap();
        assert_eq!(v["title"], "T");
        assert_eq!(v["source"], "feat");
        assert!(v["target"].is_null());
    }
}
