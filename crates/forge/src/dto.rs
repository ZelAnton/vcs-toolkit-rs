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
}

impl ForgeKind {
    /// The forge's short name (`"github"` / `"gitlab"` / `"gitea"`).
    pub fn as_str(self) -> &'static str {
        match self {
            ForgeKind::GitHub => "github",
            ForgeKind::GitLab => "gitlab",
            ForgeKind::Gitea => "gitea",
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
}
