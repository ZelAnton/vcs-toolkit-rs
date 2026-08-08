# vcs-gitea — Gitea CLI guide

**What you can do:** check auth, the lean pull-request lifecycle (list/view/create/
merge/close, review approve/reject), issues (list/view/create), and releases
(list/create/delete) — deliberately narrower than `gh`/`glab` (see the capability
note below). This guide is the full reference — every command by theme, with examples.

`vcs-gitea` drives the Gitea (and Forgejo) CLI (`tea`) from Rust. Every operation
is `async`, runs inside an OS job (via [`processkit`]) so a `tea` subprocess is
never orphaned, and returns the structured `processkit::Error`. Read commands ask
for `--output csv` and parse tea's quoted delimiter table into typed structs; the
crate never scrapes human-readable output.

> **`tea --output csv` is a print-table format, not the Gitea REST shape.** We
> choose columns with `--fields` and parse the quoted header/rows. `tea` 0.9.x
> does not provide usable structured single-item PR/issue views, so both typed
> view methods page these list tables and filter by number. Hermetic parser/argv
> tests pin the contract; ignored live tests are the drift check.

The surface is the **lean pull-request lifecycle** `tea` actually supports. It is
deliberately **narrower** than `vcs-github` / `vcs-gitlab` — see the capability
note below. The [`vcs-forge`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/) facade unifies it with the other two.

Consumers code against the [`GiteaApi`] trait and substitute a fake in tests. See
[Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/) for the two seams (the `mock` feature →
`MockGiteaApi`, or a `ScriptedRunner`).

Requires the `tea` binary on `PATH`, configured via `tea login add`.

[`processkit`]: https://crates.io/crates/processkit

> ⚠️ **CLI surface tracks the installed `tea`, not a frozen contract.** The argv
> the code builds and the CSV it parses are pinned by the hermetic tests; the
> `#[ignore]` integration smoke tests additionally check, against the real binary
> in CI, that `tea` integrates at all (`version` + `auth_status`). The PR
> **lifecycle** argv follows the documented `tea` CLI but is **not** exercised
> end-to-end in CI (that needs a live, authenticated Gitea); confirm it against
> your installed `tea` if a flag ever drifts.

## What `tea` does **not** do

`tea` has no usable single-PR or single-issue structured `view`, no current-repo view, no draft toggle, no
PR-checks command, and no single-release view (`tea releases` ignores any
positional and always lists). Consequences:

- **`pr_view` is synthesized** by **paging** `tea pr list --state all` (`--page N`,
  50 rows each) and filtering by number. The Gitea *server* caps a page at
  `MAX_RESPONSE_ITEMS` (default 50), so a single large `--limit` is silently clamped
  — paging is what lets `pr_view` find a PR past that cap instead of a false "not
  found". It stops at the first empty page (a genuine absence → `ErrorReason::Parse`) or a
  large safety bound. `issue_view` uses the same paging strategy because the
  bare-index issue command renders Markdown and ignores `--output`.
- **`repo_view`, `pr_mark_ready`, `pr_checks`, and `release_view` are simply
  absent** from `GiteaApi`. Through the [`vcs-forge`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/) facade they return
  `Error::Unsupported` for the Gitea backend (`err.is_unsupported()`).
- **No labels/assignees/author/timestamp/milestone columns.** `tea`'s PR/issue
  table output carries none of these, so this crate's
  `PullRequest`/`Issue`/`Release` types don't model them either — through the
  [`vcs-forge`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/) facade a
  Gitea-backed `ForgePr`/`ForgeIssue`/`ForgeRelease` reports `labels`,
  `assignees`, `author`, `created_at`, `updated_at`, and `milestone` as `None` —
  *unknown*, never a false empty/confirmed value (GitHub/GitLab report `Some(..)`
  for all of these).

## Construction

```rust,ignore
use vcs_gitea::Gitea;
let tea = Gitea::new();                 // real job-backed runner
```

`Gitea::with_runner(runner)` injects a fake `ProcessRunner` for tests;
`tea.at(dir)` returns a [`GiteaAt`] view whose repo-scoped methods drop `dir`.

## Auth & version

```rust,ignore
# use vcs_gitea::{Gitea, GiteaApi};
# async fn demo(tea: &Gitea) -> Result<(), processkit::Error> {
let v = tea.version().await?;          // String
let authed = tea.auth_status().await?; // bool — a non-empty `tea login list`
# Ok(()) }
```

`tea` has no per-instance `auth status`, so `auth_status` reads
`tea login list --output csv` and reports whether at least one login is
configured.

## Pull requests

| Method | Runs | Returns |
|---|---|---|
| `pr_list(dir)` | `tea pr list --state open --limit 100 --fields index,title,state,head,base,url --output csv` | `Vec<PullRequest>` (compatibility default) |
| `pr_list_with(dir, spec)` | `tea pr list --state open\|closed\|all --limit <limit> [--page N] --fields … --output csv` | `Vec<PullRequest>` via `PrList`; `Closed` pages/filter merged rows; `Merged` is `Unsupported` |
| `pr_view(dir, number)` | `tea pr list --state all --limit 50 --page N --fields … --output csv` (paged) + filter | [`PullRequest`] |
| `pr_create(dir, spec)` | `tea pr create --title … --description … [--head …] [--base …] [--labels a,b]` | `String` |
| `pr_add_labels` / `pr_remove_labels` | **Unsupported** (`tea` has no PR edit subcommand) | Use the Gitea REST API. |
| `pr_merge(dir, number, merge)` | `tea pr merge <number> --style merge\|rebase\|squash` | `()` |
| `pr_close(dir, number)` | `tea pr close <number>` | `()` |
| `pr_comment(dir, number, body)` | `tea comment <number> <body>` | `String` |
| `pr_edit(dir, number, spec)` | **Unsupported** (`tea` has no `pr edit` subcommand) | Use the Gitea REST API. |
| `pr_approve(dir, number)` | `tea pr approve <number>` | `()` |
| `pr_reject(dir, number, body)` | `tea pr reject <number> <reason>` | `()` |

`PullRequest` carries `number` (tea's `index` column), `title`, `state`, `merged`,
`head_branch`, `base_branch`, and `url` — read from tea's table columns (we select
them with `--fields`). tea folds the merge flag into the `state` column: a merged
PR reads `state="merged"` (not `"closed"`), and `merged` is derived from that. A
**fork** PR's head is rendered `owner:branch` by tea; the parser strips the `owner:`
prefix so `head_branch` is always the bare branch (matching GitHub/GitLab — the fork
owner is not modelled).

```rust,ignore
# use std::path::Path;
# use vcs_gitea::{Gitea, GiteaApi, PrCreate, PrMerge};
# async fn demo(tea: &Gitea, repo: &Path) -> Result<(), processkit::Error> {
for pr in tea.pr_list(repo).await? {
    println!("#{} [{}] {} — {}", pr.number, pr.state, pr.title, pr.url);
}
let out = tea
    .pr_create(repo, PrCreate::new("Add streaming", "Implements …")
        .head("feat/streaming").base("main"))
    .await?;
tea.pr_merge(repo, 7, PrMerge::squash()).await?;
# let _ = out; Ok(()) }
```

`pr_merge` takes a [`PrMerge`] spec — a [`MergeStrategy`] (`Merge` / `Squash` /
`Rebase`, mapped to `tea pr merge --style`) built through
`PrMerge::merge()`/`squash()`/`rebase()`. The gh-style `.auto()` /
`.delete_branch()` options are **not expressible on `tea`** (it has no
merge-when-checks flag), so setting either makes `pr_merge` return
`ErrorReason::Unsupported` rather than silently dropping it.

`pr_create` takes a [`PrCreate`] spec — build it through `PrCreate::new(title,
body)` and chain the optional `.head(b)` (`--head`; `None` = the current branch) /
`.base(b)` (`--base`; `None` = the repo default) setters. Public fields:
`title: String`, `body: String`, `head: Option<String>`, `base: Option<String>`,
`labels: Vec<String>`. Creation labels are encoded in tea's single comma-separated
`--labels` value; a label containing a comma is rejected rather than silently split.
Unlike `gh`/`glab`, `tea` prints a **textual summary** on success, not the new
PR's URL (it has no flag to shape create output), so do **not** parse the returned
`String` as a URL.

### Review

`pr_approve(dir, number)` records an approving review (`tea pr approve <index>`);
`pr_reject(dir, number, body)` requests changes with a **required** reason
(`tea pr reject <index> <reason>`). The reason is a bare positional, so — like
`pr_comment`'s body — it is refused before spawning if it is empty or begins with
`-` (`reject_flag_like`). On the
[`vcs-forge`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/) facade,
`Forge::pr_approve` maps to `pr_approve` and `Forge::pr_request_changes` maps to
`pr_reject`.

## Issues & releases

| Method | Runs | Returns |
|---|---|---|
| `issue_list(dir)` | `tea issues list --state open --limit 100 --fields index,title,state,body,url --output csv` | `Vec<Issue>` (compatibility default) |
| `issue_list_with(dir, spec)` | `tea issues list --state open\|closed\|all --limit <limit> --fields … --output csv` | `Vec<Issue>` via `IssueList` |
| `issue_view(dir, number)` | `tea issues list --state all --page N --output csv` (paged) + filter | [`Issue`] |
| `issue_create(dir, title, body)` / `issue_create_with(dir, spec)` | `tea issues create --title … --description … [--labels a,b]` | `String` |
| `issue_add_labels` / `issue_remove_labels` | **Unsupported** (`tea` has no issue edit subcommand) | Use the Gitea REST API. |
| `release_list(dir)` | `tea releases list --limit 100 --output csv` | `Vec<Release>` |
| `release_create(dir, spec)` | `tea releases create --tag <tag> [--title …] [--note …] [--draft] [--prerelease]` | `String` (tea's output) |
| `release_delete(dir, tag)` | `tea releases delete <tag>` | `()` |

The compatibility list methods request `state=open, limit=100`; the `*_with`
forms select other states and limits. `tea` cannot filter merged-only PRs, so
`PrListState::Merged` is a structured `Unsupported` without spawning. The Gitea **server** caps a page at
`MAX_RESPONSE_ITEMS` (default 50). Open/all PR lists still return at most ~50
rows in one call — a busier repo is silently truncated, so page beyond that through
`run` (`--page N`) or the API. Closed PR lists page until they collect the requested
number of unique non-merged rows or receive an empty page; a merged row therefore
does not consume the requested closed result limit, and a safety-bound walk returns
a parse error rather than a partial result. `issue_list` also pins `--fields` to fetch `body`/`url` (tea's default issue
columns omit them). Like `pr_view`, **`issue_view` is synthesized by paging and
filtering** because `tea issues <number>` (the bare-index form) renders Markdown
and ignores `--output`.
`issue_create_with(IssueCreate)` adds creation labels while the original
`issue_create(title, body)` remains the compatibility shorthand. `issue_create`,
like `pr_create`, returns tea's textual summary verbatim — its
final line is the new issue's URL, but there is no flag to shape the output, so it
is **not** a parsed URL. There is intentionally **no `release_view`**: `tea
releases` takes no positional and always lists, so a single-release-by-tag view
doesn't exist in `tea` (the [`vcs-forge`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/) facade reports it
`Unsupported`).

`Issue` carries `number` (tea's `index`), `title`, `state` (`"open"`/`"closed"`),
`body`, and `url` — from tea's table columns for both list and synthesized view.

`Release` carries `tag` (tea's `Tag-Name` column), `title`, `published_at` (e.g.
`"2023-07-26T13:02:36Z"`, empty for an unpublished draft), and `draft`/`prerelease`
(derived from tea's `Status` column). **`url` is always empty**: `tea releases
list` exposes no release-page URL (only a tar/zip download URL, which is
deliberately not surfaced).

`release_create` takes the [`ReleaseCreate`] spec (`new(tag)` plus chained `title`
/ `notes` / `draft` / `prerelease` setters) and returns tea's textual summary
verbatim (like `pr_create`/`issue_create`). Note the per-CLI shape: unlike gh/glab,
`tea` takes the tag as a **flag** (`--tag`, not a bare positional) and its notes flag
is the singular `--note`; `tea` *does* support `--draft`/`--prerelease`. Asset
uploads are **out of scope** (attach files with `run`). `release_delete`
(`tea releases delete <tag>`) takes the tag as a bare positional — flag-injection
guarded like `pr_comment`'s body — and, like tea's other mutators (`pr close`/`pr
merge`), passes no confirmation flag.

```rust,ignore
# use std::path::Path;
# use vcs_gitea::{Gitea, GiteaApi};
# async fn demo(tea: &Gitea, repo: &Path) -> Result<(), processkit::Error> {
for issue in tea.issue_list(repo).await? {
    println!("#{} [{}] {}", issue.number, issue.state, issue.title);
}
let one = tea.issue_view(repo, 7).await?;        // paged list + filter
for rel in tea.release_list(repo).await? {
    println!("{} — {}", rel.tag, rel.title);
}
# let _ = one; Ok(()) }
```

## Escape hatch

`run`/`run_raw` (and the inherent `run_args`/`run_raw_args`) drive any unmodelled
`tea` command. Editing a Gitea PR title or description (including a `WIP:` draft
prefix) requires the Gitea REST API because `tea` has no `pr edit` subcommand.

**cwd (T-035).** On the **client** (`tea.run(…)`) these run in the **process's
current directory**. On the **bound view** (`tea.at(dir).run(…)`) they are instead
bound to `dir`: the view forwards to the client's dir-taking `run_in`/`run_raw_in`/
`run_args_in`/`run_raw_args_in`, so a raw call through the handle runs in the bound
repo, like every other `GiteaAt` method. Reach for the client's `run` when you
deliberately want the process cwd.

## See also

- [Supported CLI versions](https://github.com/ZelAnton/vcs-toolkit-rs/blob/main/docs/supported-clis.md) — the tea 0.9.0
  floor plus the 0.9.2/latest live-Gitea matrix.

- [vcs-forge guide](https://docs.rs/vcs-forge/latest/vcs_forge/guide/) — the facade; note the Gitea `Unsupported` ops.
- [vcs-github guide](https://docs.rs/vcs-github/latest/vcs_github/guide/) — the fuller-surfaced sibling this mirrors.
- [Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/) — the `mock` feature and the `ScriptedRunner` seam.
- [Process model & errors](https://docs.rs/vcs-core/latest/vcs_core/guide/process_model/) — OS-job containment, timeouts, and
  the `Error` / `ProcessResult` shapes.
- [crate docs](https://docs.rs/vcs-gitea) — quickstart and crate-level docs.
