# vcs-gitlab — GitLab CLI guide

**What you can do:** check auth, view the project, the lean merge-request lifecycle
(list/view/create/merge/ready/close), CI/pipeline status, issues, and releases.
This guide is the full reference — every command by theme, with examples.

`vcs-gitlab` drives the GitLab CLI (`glab`) from Rust. Every operation is `async`,
runs inside an OS job (via [`processkit`]) so a `glab` subprocess is never
orphaned, and returns the structured `processkit::Error` instead of a stringly
exit. Commands ask for `--output json` and are deserialized into typed structs
(GitLab's REST JSON, which `glab` passes through); the crate never scrapes
human-readable output.

The surface is the **lean merge-request lifecycle** — it mirrors `vcs-github`'s
shape but covers only auth, project view, and the MR lifecycle. The
[`vcs-forge`](https://docs.rs/vcs-forge/latest/vcs_forge/guide/) facade unifies it with `vcs-github` and `vcs-gitea`.

Consumers code against the [`GitLabApi`] trait and substitute a fake in tests —
the real [`GitLab`] client only appears at the edges. See
[Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/) for the two seams (the `mock` feature →
`MockGitLabApi`, or a `ScriptedRunner`).

Requires the `glab` binary on `PATH`, authenticated via `glab auth login`.

[`processkit`]: https://crates.io/crates/processkit

> ⚠️ **CLI surface tracks the installed `glab`, not a frozen contract.** The argv
> the code builds and the JSON shapes it parses are pinned by the hermetic tests
> (so a refactor can't silently change them). The `#[ignore]` integration smoke
> tests additionally check, against the real binary in CI, that `glab` integrates
> at all — `version` and `auth_status`. The create/merge/close **lifecycle** argv
> follows the documented `glab` CLI but is **not** exercised end-to-end in CI (that
> needs a live, authenticated GitLab); confirm it against your installed `glab` if
> a flag ever drifts.

## Construction

```rust,ignore
use vcs_gitlab::GitLab;
use std::time::Duration;

let glab = GitLab::new();                                  // real job-backed runner
let glab = GitLab::new().default_timeout(Duration::from_secs(30)); // kill long runs
```

`GitLab::with_runner(runner)` injects a fake `ProcessRunner` for tests.
`glab.at(dir)` returns a [`GitLabAt`] view whose project-scoped methods drop the
leading `dir` argument.

## Auth & version

```rust,ignore
# use vcs_gitlab::{GitLab, GitLabApi};
# async fn demo(glab: &GitLab) -> Result<(), processkit::Error> {
let v = glab.version().await?;          // String — "glab version 1.x …"
let authed = glab.auth_status().await?; // bool — true when `glab auth status` exits 0
# Ok(()) }
```

`auth_status` reads the exit code via `probe`: a timeout or unexpected failure
still surfaces as `processkit::Error`, never a silent `false`. **Caveat:** a
[known glab bug](https://gitlab.com/gitlab-org/cli/-/issues/911) can make
`glab auth status` exit `0` even when unauthenticated, so treat a `true` as
best-effort (the next API call is the real test); `false`/timeout are faithful.

## Project & merge requests

| Method | Runs | Returns |
|---|---|---|
| `repo_view(dir)` | `glab repo view --output json` | [`RepoView`] |
| `mr_list(dir)` | `glab mr list --output json` | `Vec<MergeRequest>` |
| `mr_view(dir, id)` | `glab mr view <id> --output json` | [`MergeRequest`] |
| `mr_create(dir, spec)` | `glab mr create --title … --description … [--source-branch …] [--target-branch …] --yes` | `String` (the MR URL) |
| `mr_merge(dir, id, strategy)` | `glab mr merge <id> --yes --auto-merge=false [--squash\|--rebase]` | `()` |
| `mr_mark_ready(dir, id)` | `glab mr update <id> --ready` | `()` |
| `mr_close(dir, id)` | `glab mr close <id>` | `()` |
| `mr_checks(dir, id)` | `glab mr view <id> --output json` (reads `head_pipeline.status`) | [`CiStatus`] |
| `mr_diff(dir, id)` | `glab mr diff <id> --color never` | `Vec<`[`FileDiff`]`>` |

`MergeRequest` carries `iid` (the project-scoped id the commands take), `title`,
`state` (GitLab's `"opened"`/`"closed"`/`"merged"`/`"locked"`), `source_branch`,
`target_branch`, `web_url`, and `draft`.

```rust,ignore
# use std::path::Path;
# use vcs_gitlab::{GitLab, GitLabApi, MergeStrategy, MrCreate};
# async fn demo(glab: &GitLab, repo: &Path) -> Result<(), processkit::Error> {
for mr in glab.mr_list(repo).await? {
    println!("!{} [{}] {} — {}", mr.iid, mr.state, mr.title, mr.web_url);
}
let url = glab
    .mr_create(repo, MrCreate::new("Add streaming", "Implements …")
        .source("feat/streaming").target("main"))
    .await?;
glab.mr_merge(repo, 12, MergeStrategy::Squash).await?;
# let _ = url; Ok(()) }
```

[`MergeStrategy`] is `Merge` (glab's default merge commit), `Squash`, or `Rebase`.
`mr_merge` passes `--auto-merge=false` so it merges **immediately** rather than
enabling glab's default merge-when-pipeline-succeeds. [`CiStatus`] buckets the
pipeline into `Passing` / `Failing` / `Pending` / `None`.

`mr_create` takes an [`MrCreate`] spec — build it through `MrCreate::new(title,
body)` and chain the optional `.source(b)` (`--source-branch`; `None` = the
current branch) / `.target(b)` (`--target-branch`; `None` = the project default)
setters. Public fields: `title: String`, `body: String`, `source: Option<String>`,
`target: Option<String>`.

`mr_diff` returns the MR's diff as one [`FileDiff`] per changed file, parsed
through the same unified-diff parser
[`vcs-git`](https://docs.rs/vcs-git/latest/vcs_git/guide/)/[`vcs-jj`](https://docs.rs/vcs-jj/latest/vcs_jj/guide/)
use — `glab mr diff` emits the same git-format diff `git diff` does, so
`vcs-gitlab` re-exports [`FileDiff`] (and [`ChangeKind`], [`Hunk`], [`DiffLine`])
rather than depending on `vcs-diff` directly for the type alone.

## Issues & releases

| Method | Runs | Returns |
|---|---|---|
| `issue_list(dir)` | `glab issue list --per-page 100 --output json` | `Vec<Issue>` |
| `issue_view(dir, number)` | `glab issue view <number> --output json` | [`Issue`] |
| `issue_create(dir, title, body)` | `glab issue create --title … --description … --yes` | `String` (the issue URL) |
| `release_list(dir)` | `glab release list --per-page 100 --output json` | `Vec<Release>` |
| `release_view(dir, tag)` | `glab release view <tag> --output json` | [`Release`] |

The list methods pin `--per-page 100` (the GitLab API per-page max) so glab's
default page size of 30 can't silently truncate them; reach beyond 100 through
`run`. `issue_create` passes `--yes` to skip glab's interactive submission prompt,
mirroring `mr_create`. `release_view`'s bare `<tag>` positional is flag-injection
guarded (a leading `-` or empty value is refused before any process spawns).

`Issue` carries `number` (the project-scoped `iid` GitLab's `issue` commands take,
surfaced as `number` for cross-forge consistency with `vcs-github`), `title`,
`state` (GitLab's `"opened"`/`"closed"`), `body` (GitLab's `description`), and
`url` (GitLab's `web_url`).

`Release` carries `tag_name` (the `<tag>` `release_view` takes), `name` (the
release title, may default to the tag), `url` (pulled off GitLab's nested
`_links.self` — releases have no top-level `web_url`), `published_at` (GitLab's
`released_at`, ISO 8601, empty for an unpublished release), and `description`
(release notes/markdown, empty when absent).

```rust,ignore
# use std::path::Path;
# use vcs_gitlab::{GitLab, GitLabApi};
# async fn demo(glab: &GitLab, repo: &Path) -> Result<(), processkit::Error> {
for issue in glab.issue_list(repo).await? {
    println!("#{} [{}] {}", issue.number, issue.state, issue.title);
}
let url = glab.issue_create(repo, "Flaky pipeline", "fails on …").await?;
for rel in glab.release_list(repo).await? {
    println!("{} — {}", rel.tag_name, rel.url);
}
# let _ = url; Ok(()) }
```

## Escape hatch

`run(args)` / `run_raw(args)` (and the inherent `run_args(&[&str])` /
`run_raw_args`) drive any unmodelled `glab` command; `run` returns trimmed stdout
and errors on a non-zero exit, `run_raw` hands back the captured `ProcessResult`.
These run in the **process's current directory** — the raw hatch supplies the whole
argv, so target a specific repo with `-R group/project`; the bound `at(dir)` view
does *not* re-bind them.
`api(dir, endpoint)` is the `glab api <endpoint>` shortcut for any REST/GraphQL
endpoint (mirrors `vcs-github`'s `api`), with the `endpoint` guarded against flag
injection. Unlike `run`, it **is** dir-bound, so a relative endpoint resolves the
project against `dir`'s remote (its `at(dir)` form is `api(endpoint)`).

## See also

- [vcs-forge guide](https://docs.rs/vcs-forge/latest/vcs_forge/guide/) — the facade that unifies this with GitHub/Gitea.
- [vcs-github guide](https://docs.rs/vcs-github/latest/vcs_github/guide/) — the broader-surfaced sibling this mirrors.
- [Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/) — the `mock` feature and the `ScriptedRunner` seam.
- [Process model & errors](https://docs.rs/vcs-core/latest/vcs_core/guide/process_model/) — OS-job containment, timeouts, and
  the `Error` / `ProcessResult` shapes.
- [crate docs](https://docs.rs/vcs-gitlab) — quickstart and crate-level docs.
