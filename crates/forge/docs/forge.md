# vcs-forge — the forge facade

`vcs-forge` is a **forge-agnostic facade** over [`vcs-github`](https://docs.rs/vcs-github/latest/vcs_github/guide/),
[`vcs-gitlab`](https://docs.rs/vcs-gitlab/latest/vcs_gitlab/guide/), and [`vcs-gitea`](https://docs.rs/vcs-gitea/latest/vcs_gitea/guide/) — the `gh`/`glab`/`tea`
analogue of how [`vcs-core`](https://docs.rs/vcs-core/latest/vcs_core/guide/) sits over git and jj. A [`Forge`] handle
dispatches the common forge operations to whichever CLI backs it and returns
**unified DTOs**, so a tool can target "the forge" instead of one specifically.

Consumers can hold a `&dyn ForgeApi` to stay generic over the runner; build a
`Forge` over a fake runner for hermetic tests.

## No auto-detection — construct explicitly

A repository has a filesystem marker (`.git`/`.jj`) that [`vcs-core`](https://docs.rs/vcs-core/latest/vcs_core/guide/)
detects; a **forge does not** — it's identified by the remote *host*. So a
`Forge` is built explicitly:

```rust,ignore
use vcs_forge::{Forge, ForgeApi};

let forge = Forge::github(".");   // or ::gitlab(".") / ::gitea(".")
```

[`ForgeKind::from_remote_url`] is a pure, best-effort helper for picking the kind
from a remote URL you already hold (e.g. from a `vcs_core::Repo`):

```rust,ignore
use vcs_forge::{Forge, ForgeKind};

# fn pick(url: &str) -> Forge {
let forge = match ForgeKind::from_remote_url(url) {
    Some(ForgeKind::GitLab) => Forge::gitlab("."),
    Some(ForgeKind::Gitea)  => Forge::gitea("."),
    _                       => Forge::github("."), // github.com or unknown
};
# forge }
```

It recognises the **public SaaS** hosts — `github.com`, `gitlab.com`,
`gitea.com`, `codeberg.org`, and their proper subdomains — with an anchored
match, so a lookalike like `gitlab.com.attacker.net` returns `None`, not GitLab. A
**self-hosted** instance on an arbitrary domain also returns `None`
(indistinguishable by host alone — pick the kind yourself).

`Forge::github(cwd)` / `gitlab` / `gitea` build over the real runner using the CLI's
ambient login; `Forge::github_with_token(cwd, token)` / `gitlab_with_token`
authenticate with an explicit token instead (injected as `GH_TOKEN` / `GITLAB_TOKEN`;
`token` takes `impl Into<Secret>`, so a `&str`/`String` works). Gitea is
**ambient-only** — `tea` reads its own config and has no token-via-environment
override, so there is no `gitea_with_token`; run `tea login` out of band.
`Forge::from_github(cwd, client)` / `from_gitlab` / `from_gitea` take an explicit
client (the test seam); `forge.at(dir)` re-binds the cwd, sharing the client.

## Operations

```rust,ignore
pub async fn auth_status(&self)  -> Result<bool>;
pub async fn repo_view(&self)    -> Result<ForgeRepo>;
pub async fn pr_list(&self)      -> Result<Vec<ForgePr>>;
pub async fn pr_view(&self, number: u64) -> Result<ForgePr>;
pub async fn pr_create(&self, spec: PrCreate) -> Result<String>;
pub async fn pr_comment(&self, number: u64, body: &str) -> Result<String>;
pub async fn pr_edit(&self, number: u64, edit: PrEdit) -> Result<()>;
pub async fn pr_merge(&self, number: u64, strategy: MergeStrategy) -> Result<()>;
pub async fn pr_mark_ready(&self, number: u64) -> Result<()>;
pub async fn pr_close(&self, spec: PrClose) -> Result<()>; // PrClose::new(n)[.delete_branch()] — delete_branch is GitHub-only
pub async fn pr_checks(&self, number: u64) -> Result<CiStatus>;
pub async fn issue_list(&self)   -> Result<Vec<ForgeIssue>>;
pub async fn issue_view(&self, number: u64) -> Result<ForgeIssue>;
pub async fn issue_create(&self, spec: IssueCreate) -> Result<String>; // IssueCreate::new(title, body)
pub async fn release_list(&self) -> Result<Vec<ForgeRelease>>;
pub async fn release_view(&self, tag: &str) -> Result<ForgeRelease>;
```

[`PrCreate`] is the unified open-a-PR/MR spec —
`PrCreate::new(title, body).source(branch).target(branch)`, where `source`
defaults to the current branch and `target` to the repo default; the facade maps
them to each CLI's own flags (gh/tea `--head`/`--base`, glab
`--source-branch`/`--target-branch`).

[`PrEdit`] is the unified edit spec — `PrEdit::new().title(t).body(b)`, each field
optional; `pr_edit` rejects both-`None` with `Error::InvalidInput` before any
spawn. `pr_comment` likewise rejects an empty/whitespace-only body up front.

Every method mirrors an inherent method on [`Forge`]; the object-safe `ForgeApi`
trait adds nothing but the `&dyn` boundary.

## Unified DTOs

[`ForgePr`] generalises GitHub's PR, GitLab's MR, and Gitea's PR: `number` (the id
each CLI takes — GitLab's `iid`), `title`, `state` ([`ForgePrState`]),
`source_branch`, `target_branch`, `url`, `draft`.

**State normalisation** ([`ForgePrState`]):

| Forge | "open" | "closed" | "merged" |
|---|---|---|---|
| GitHub | `OPEN` | `CLOSED` | `MERGED` |
| GitLab | `opened` | `closed` / `locked` | `merged` |
| Gitea | `state="open"` | `state="closed"` | `merged=true` |

[`ForgeRepo`] is `name` / `owner` / `default_branch` / `url` / `private` (GitLab's
owner is the namespace path). [`CiStatus`] is `Passing` / `Failing` / `Pending` /
`None` — GitHub aggregates its per-check buckets into it, GitLab maps its pipeline
status. [`MergeStrategy`] (`Merge` / `Squash` / `Rebase`) maps to each CLI's flag.

`draft` is **best-effort**: only GitLab reports it on the lean surface; GitHub and
Gitea report `false` (their lean JSON doesn't carry the flag).

[`ForgeIssue`] generalises the three issue shapes: `number` (GitLab's `iid`),
`title`, `state` ([`ForgeIssueState`] — `Closed` for any case of "closed",
everything else reads as `Open`, so an unmodelled state is treated as live),
`body`, `url` — both populated by `issue_list` and `issue_view` on every forge.

[`ForgeRelease`] is `tag` / `title` / `url` / `published_at: Option<String>`
(`None` for an unpublished draft or when the backend doesn't report one) /
`body: Option<String>` / `draft: bool` / `prerelease: bool`. The `url` is
**best-effort**: empty from GitHub's lean `release_list` (filled by
`release_view`), and **always empty on Gitea** — `tea releases list` exposes no
release-page URL at all (only a tar/zip download URL, deliberately not surfaced),
and `tea` has no `release_view`. `body` (release notes) is **best-effort**: `None`
from GitHub's lean `release_list` (only `release_view` fills it) and always `None`
on Gitea (`tea` has no body column); GitLab carries it on both. `draft` /
`prerelease` are reported by GitHub and Gitea, but GitLab has no such concept so
both are always `false` there.

## Capability matrix

The CLIs differ in coverage. Gitea's `tea` lacks four operations, which return
[`Error::Unsupported { forge, operation }`] (the call does **not** spawn);
`delete_branch` on `pr_close` is GitHub-only.

| Operation | GitHub | GitLab | Gitea |
|---|:---:|:---:|:---:|
| `auth_status` / `pr_list` / `pr_view` / `pr_create` / `pr_merge` / `pr_close` | ✅ | ✅ | ✅ |
| `issue_list` / `issue_view` / `issue_create` / `release_list` | ✅ | ✅ | ✅ |
| `repo_view` | ✅ | ✅ | ❌ Unsupported |
| `pr_mark_ready` | ✅ | ✅ | ❌ Unsupported |
| `pr_checks` | ✅ | ✅ | ❌ Unsupported |
| `release_view` | ✅ | ✅ | ❌ Unsupported (`tea releases` only lists — filter `release_list`) |
| `pr_close` honours `delete_branch` | ✅ | ignored | ignored |
| `pr_create` / `issue_create` return the **URL** | ✅ | ✅ | textual summary (tea ends `issue create` output with the URL; `pr create` prints none) |
| `pr_list` / `issue_list` / `release_list` result cap (explicit, documented) | 100 | 100 | ~50 (server page cap) |

Handle a gap **reactively** — call and classify the error:

```rust,ignore
# use vcs_forge::{Forge, ForgeApi, Error};
# async fn demo(forge: &Forge) {
match forge.pr_checks(7).await {
    Ok(status) => println!("CI: {status:?}"),
    Err(e) if e.is_unsupported() => println!("this forge has no checks command"),
    Err(e) => eprintln!("{e}"),
}
# }
```

…or **proactively** — ask up front with [`Forge::supports`] (a pure, spawn-free
match on the backend) or [`Forge::capabilities`] (one auth probe, then the whole
flat map), e.g. to hide an unavailable button:

```rust,ignore
# use vcs_forge::{Forge, ForgeOp};
# fn demo(forge: &Forge) {
if forge.supports(ForgeOp::PrChecks) {
    // render the "CI checks" button
}
if forge.supports(ForgeOp::ReleaseView) { /* show a release detail link */ }
# }
```

`Error` is `Forge(processkit::Error)`, `Unsupported { forge, operation }`, or
`InvalidInput(String)`, with `is_unsupported()`, `is_invalid_input()`,
`is_resource_not_found()`, and `is_transient_fetch_error()` classifiers.

## When to drop to the wrapped client (the escape hatch)

The facade carries the **portable intersection**; the wrappers are re-exported
(`vcs_forge::vcs_github` / `vcs_gitlab` / `vcs_gitea`) so anything beyond it is
one constructor away — without adding a dependency.

| You need… | Use |
|---|---|
| The common lifecycle, portably (list/view/create/merge/close PRs, issues, releases) | the `Forge` facade |
| An op the facade marks `Unsupported` on *your* forge (e.g. a Gitea release by tag) | there's nothing to call — the CLI can't do it; go through the forge's REST API (`gh api` via `vcs_github::GitHubApi::api`, `glab api` via `vcs_gitlab::GitLabApi::api`, or your own HTTP) |
| A forge-specific op (GitHub workflow runs, review submission, draft toggle, gist…) | the wrapper client directly: `GitHub::new().run_list(dir)…` |
| More than 100 list results, custom JSON fields, exotic flags | the wrapper's raw `run(dir, args)` |
| A field the unified DTO drops (e.g. a release's draft/prerelease flags) | the wrapper method — its DTO keeps the per-CLI fields |

## See also

- [vcs-github](https://docs.rs/vcs-github/latest/vcs_github/guide/) / [vcs-gitlab](https://docs.rs/vcs-gitlab/latest/vcs_gitlab/guide/) / [vcs-gitea](https://docs.rs/vcs-gitea/latest/vcs_gitea/guide/) — the
  wrapped clients and their per-CLI surfaces.
- [vcs-core guide](https://docs.rs/vcs-core/latest/vcs_core/guide/) — the sibling facade over git/jj.
- [Cookbook](https://docs.rs/vcs-core/latest/vcs_core/guide/cookbook/) — the open-a-PR recipe.
- [Process model & errors](https://docs.rs/vcs-core/latest/vcs_core/guide/process_model/) — OS-job containment and the `Error`
  shapes underneath.
- [crate docs](https://docs.rs/vcs-forge) — quickstart and crate-level docs.
