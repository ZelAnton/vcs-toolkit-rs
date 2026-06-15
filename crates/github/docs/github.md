# vcs-github — GitHub CLI guide

**What you can do:** check auth, view the repo, the full pull-request lifecycle
(list/view/create/merge/ready/close, review/comment, CI checks, feedback), issues,
releases, and GitHub Actions runs (list/view/watch). This guide is the full
reference — every command by theme, with examples.

`vcs-github` drives the GitHub CLI (`gh`) from Rust. Every operation is `async`,
runs inside an OS job (via [`processkit`]) so a `gh` subprocess is never
orphaned, and returns the structured `processkit::Error` instead of a stringly
exit. Commands that ask for `--json` are deserialized into typed structs; the
crate never scrapes human-readable output.

Consumers code against the [`GitHubApi`] trait and substitute a fake in tests —
the real [`GitHub`] client only appears at the edges. See
[Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/) for the two seams.

Requires the `gh` binary on `PATH`, authenticated via `gh auth login`. An
unauthenticated `gh` surfaces as an `Error::Exit` (gh's auth-required exit), not
a silent empty result.

[`processkit`]: https://crates.io/crates/processkit

## Construction & configuration

```rust,ignore
use vcs_github::GitHub;

let gh = GitHub::new(); // GitHub<JobRunner> — the real job-backed client
```

`GitHub::new()` builds a client over `processkit`'s real job-backed runner. Two
knobs and one test seam:

```rust,ignore
# use vcs_github::GitHub;
use std::time::Duration;
use processkit::testing::ScriptedRunner;

// Cap every spawned `gh` — a slow/hung command becomes `Error::Timeout`.
let gh = GitHub::new().default_timeout(Duration::from_secs(30));

// Inject a fake process executor instead of spawning `gh` (tests, CI).
let gh = GitHub::with_runner(ScriptedRunner::new());
```

The timeout matters for blocking calls — see [`run_watch`](#actions-runs), which
parks for the lifetime of a CI run.

### cwd-bound handle — `gh.at(&path)`

Most methods take a leading `dir: &Path`. When you make several calls against
one repo, bind it once and drop the argument:

```rust,ignore
# use vcs_github::{GitHub, GitHubApi};
use std::path::Path;
# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
let gh = GitHub::new();
let at = gh.at(repo); // GitHubAt<'_, R> — Copy, cheap to pass around

let prs = at.pr_list().await?; // == gh.pr_list(repo)
let issues = at.issue_list().await?;
# Ok(()) }
```

`gh.at(dir)` returns a [`GitHubAt`] — a `Copy` view holding two references. Its
bound methods produce byte-identical argv to the `dir`-taking calls (the crate
guards this with a test); the only difference is ergonomics. `bare` methods that
take no `dir` (`version`, `auth_status`, `api`, the raw escape hatches) forward
verbatim.

### Inherent `&[&str]` helpers

`GitHubApi::run`/`run_raw` take `&[String]` (the trait must stay object-safe and
`mockall`-friendly). On the concrete `GitHub`, two inherent methods take string
slices so you skip the `Vec<String>` allocation:

```rust,ignore
# use vcs_github::GitHub;
# async fn demo() -> Result<(), processkit::Error> {
let gh = GitHub::new();
let out = gh.run_args(&["pr", "list"]).await?;        // String — trimmed stdout
let res = gh.run_raw_args(&["pr", "list"]).await?;    // ProcessResult<String> — no error on non-zero
# Ok(()) }
```

Both are also available on the bound handle (`gh.at(dir).run_args(…)`).

## Auth & repo

```rust,ignore
async fn version(&self) -> Result<String>;            // `gh --version`
async fn auth_status(&self) -> Result<bool>;          // `gh auth status` exits 0
async fn api(&self, endpoint: &str) -> Result<String>;// `gh api <endpoint>`
async fn repo_view(&self, dir: &Path) -> Result<Repo>;// `gh repo view --json …`
```

`auth_status` reads the *exit code* as a bool — `gh auth status` exits 0 when
authenticated, non-zero when not. But that is the only thing folded into the
bool: a spawn failure, a timeout, or any unexpected exit still errors rather
than reporting a silent `false`.

```rust,ignore
# use vcs_github::{GitHub, GitHubApi};
# async fn demo() -> Result<(), processkit::Error> {
let gh = GitHub::new();
match gh.auth_status().await {
    Ok(true)  => println!("authenticated"),
    Ok(false) => println!("not logged in (run `gh auth login`)"),
    Err(processkit::Error::Timeout { .. }) => eprintln!("gh timed out"),
    Err(e) => eprintln!("{e}"),
}
# Ok(()) }
```

`api` returns the raw REST/GraphQL response body unparsed — your escape hatch
to any endpoint the typed methods don't cover. The `endpoint` is guarded
against flag-injection: a leading `-` or an empty string is refused *before*
anything spawns (gh would otherwise parse `gh api -evil` as a flag).

`repo_view` flattens gh's nested `owner`/`defaultBranchRef` objects into a flat
[`Repo`] — `owner` is the login string, `default_branch` is the ref name (empty
for an empty repository).

## Pull requests — listing & creation

```rust,ignore
async fn pr_list(&self, dir: &Path) -> Result<Vec<PullRequest>>;
async fn pr_list_for_branch(&self, dir: &Path, head: &str, base: &str) -> Result<Vec<PullRequest>>;
async fn pr_view(&self, dir: &Path, number: u64) -> Result<PullRequest>;
async fn pr_create(&self, dir: &Path, spec: PrCreate) -> Result<String>;
```

`pr_list` returns open PRs (gh's default). `pr_list_for_branch` passes
`--state all`, so a closed or merged PR for the `head`→`base` pair is reported
too — branch on each entry's `state`. Empty when none match.

`pr_create` returns the new PR's **URL** (trimmed stdout). It takes a
[`PrCreate`](#prcreate) spec carrying the title/body and the optional `head`
(`None` = the current branch) and `base` (`None` = the repo default) branches;
each branch is appended as `--head <b>` / `--base <b>` only when set.

```rust,ignore
# use vcs_github::{GitHub, GitHubApi, PrCreate};
use std::path::Path;
# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
let gh = GitHub::new();

for pr in gh.pr_list_for_branch(repo, "feat/streaming", "main").await? {
    println!("#{} [{}] {} — {}", pr.number, pr.state, pr.title, pr.url);
}

let url = gh
    .pr_create(repo, PrCreate::new("Add streaming", "Implements …")
        .head("feat/streaming").base("main"))
    .await?;
println!("opened {url}");
# Ok(()) }
```

## Pull requests — lifecycle

```rust,ignore
async fn pr_merge(&self, dir: &Path, number: u64, merge: PrMerge) -> Result<()>;
async fn pr_ready(&self, dir: &Path, number: u64) -> Result<()>;
async fn pr_close(&self, dir: &Path, number: u64, delete_branch: bool) -> Result<()>;
```

`pr_merge` takes a [`PrMerge`] config (strategy + optional `--auto` /
`--delete-branch`). `pr_ready` flips a draft to ready-for-review. `pr_close`
closes without merging, optionally deleting the head branch.

```rust,ignore
# use vcs_github::{GitHub, GitHubApi, PrMerge};
use std::path::Path;
# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
let gh = GitHub::new();
gh.pr_ready(repo, 7).await?;
gh.pr_merge(repo, 7, PrMerge::squash().delete_branch()).await?;
// or bail out:
gh.pr_close(repo, 8, true).await?; // --delete-branch
# Ok(()) }
```

## Pull requests — review & feedback

```rust,ignore
async fn pr_checks(&self, dir: &Path, number: u64) -> Result<Vec<CheckRun>>;
async fn pr_review(&self, dir: &Path, number: u64, action: ReviewAction) -> Result<()>;
async fn pr_comment(&self, dir: &Path, number: u64, body: &str) -> Result<String>;
async fn pr_feedback(&self, dir: &Path, number: u64) -> Result<PrFeedback>;
```

`pr_checks` returns the PR's checks as `Vec<CheckRun>`. gh encodes the *overall*
outcome in its exit code — **0** all passed, **8** still pending, **1** some
failed — but prints the same JSON for all three, so the crate parses the list in
every case and lets you branch on each entry's [`bucket`](#checkrun). A PR with
no checks at all (gh exits 1 with a "no checks reported" message and no JSON)
yields an empty list. Any *other* non-zero exit — no such PR, auth required,
timeout — is a genuine error. A JSON that fails to parse surfaces as
`Error::Parse`, never masked by the exit code.

```rust,ignore
# use vcs_github::{CheckBucket, GitHub, GitHubApi};
use std::path::Path;
# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
let gh = GitHub::new();
for c in gh.pr_checks(repo, 7).await? {
    match c.bucket {
        CheckBucket::Fail | CheckBucket::Cancel => println!("✗ {} ({})", c.name, c.link),
        CheckBucket::Pending => println!("… {}", c.name),
        _ => {}
    }
}
# Ok(()) }
```

`pr_review` submits a review described by [`ReviewAction`]; the body lives in
the variant because gh *requires* one for request-changes and comment reviews.
`pr_comment` adds a conversation comment and returns its **URL** (`--body` is
mandatory — without it gh would drop into an interactive prompt and hang a
headless run). `pr_feedback` fetches the PR's submitted reviews and conversation
comments into a [`PrFeedback`], flattening gh's nested author objects (a deleted
account's `null` author becomes an empty login).

```rust,ignore
# use vcs_github::{GitHub, GitHubApi, ReviewAction};
use std::path::Path;
# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
let gh = GitHub::new();
gh.pr_review(repo, 7, ReviewAction::request_changes("fix the parser")).await?;

let fb = gh.pr_feedback(repo, 7).await?;
for r in &fb.reviews { println!("{} {}", r.author, r.state); }
# Ok(()) }
```

## Issues

```rust,ignore
async fn issue_list(&self, dir: &Path) -> Result<Vec<Issue>>;
async fn issue_view(&self, dir: &Path, number: u64) -> Result<Issue>;
async fn issue_create(&self, dir: &Path, title: &str, body: &str) -> Result<String>;
```

`issue_list` fetches `number,title,state,body,url`, so the listed issues carry
`body`/`url` too (see [`Issue`](#issue)); `issue_view` returns the same fields for
a single issue. `issue_create` returns the new issue's **URL**.

```rust,ignore
# use vcs_github::{GitHub, GitHubApi};
use std::path::Path;
# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
let gh = GitHub::new();
let url = gh.issue_create(repo, "Flaky test", "`pr_checks` hangs on …").await?;
let full = gh.issue_view(repo, 3).await?; // a single issue, body + url populated
# let _ = (url, full);
# Ok(()) }
```

## Actions runs

```rust,ignore
async fn run_list(&self, dir: &Path, limit: u64, branch: Option<String>) -> Result<Vec<WorkflowRun>>;
async fn run_view(&self, dir: &Path, id: u64) -> Result<WorkflowRun>;
async fn run_watch(&self, dir: &Path, id: u64) -> Result<WorkflowRun>;
```

`run_list` returns recent runs, newest first, capped at `limit`; `branch`
(owned `Option<String>`, again for `mockall`) adds `--branch <b>` when `Some`.
`run_view` fetches one run by its id — which is [`WorkflowRun::database_id`],
not the URL number.

`run_watch` **blocks until the run finishes**, then reads its final state via a
follow-up `run view`. It deliberately omits gh's `--exit-status`: that flag
would fold the run's outcome onto the process exit code, which can't distinguish
a failed run from a cancelled one — the follow-up view's
[`conclusion`](#workflowrun) can. A client `default_timeout` kills the watch
when it elapses (`Error::Timeout`), so drive `run_watch` from a client with no
(or a generous) timeout.

```rust,ignore
# use vcs_github::{GitHub, GitHubApi};
use std::path::Path;
# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
let gh = GitHub::new(); // no default_timeout — the watch may park for minutes
let run = gh.run_watch(repo, 27023111945).await?;
match run.conclusion.as_str() {
    "success" => println!("green"),
    other     => println!("ended: {other}"), // "failure", "cancelled", …
}
# Ok(()) }
```

## Releases

```rust,ignore
async fn release_list(&self, dir: &Path) -> Result<Vec<Release>>;
async fn release_view(&self, dir: &Path, tag: &str) -> Result<Release>;
```

`release_list` returns releases newest first; it does **not** fetch
`body`/`url` (both empty — use `release_view`), but it *is* the only endpoint
that reports [`is_latest`](#release). `release_view` fills `body`/`url` for one
tag but has no `isLatest` field, so `is_latest` defaults to `false` there. The
`tag` is flag-injection guarded like `api`'s endpoint.

## Raw escape hatches

```rust,ignore
async fn run(&self, args: &[String]) -> Result<String>;                 // trimmed stdout; errors on non-zero
async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>>; // never errors on non-zero
```

`run` runs `gh <args>` and returns trimmed stdout, erroring on a non-zero exit.
`run_raw` captures the full [`ProcessResult`] and never treats a non-zero exit
as an error — inspect `.code()` / `.stdout()` / `.stderr()` yourself. Use these
for any `gh` subcommand the typed API doesn't wrap. (The inherent `&[&str]`
variants `run_args` / `run_raw_args` are documented under
[Construction](#inherent-str-helpers).)

## Result types

All result structs are `#[non_exhaustive]` (match with `..`, construct via the
crate). Fields populated by some endpoints but not others come back as empty
strings/`false`, never panicking — note the per-method gaps below.

### `PullRequest`

From `pr_list` / `pr_list_for_branch` / `pr_view`. Fields:
`number: u64`, `title: String`, `state: String` (`"OPEN"`, `"MERGED"`,
`"CLOSED"`), `head_ref_name: String`, `base_ref_name: String`, `url: String`.

### `Issue`

From `issue_list` and `issue_view` (both fetch `number`, `title`, `state`,
`body`, `url`). Fields: `number: u64`, `title: String`, `state: String`,
`body: String`, `url: String`.

### `WorkflowRun`

From `run_list` / `run_view` / `run_watch`. Fields: `database_id: u64` (the
`<run-id>` other commands take), `name: String`, `display_title: String`,
`status: String` (`"queued"`, `"in_progress"`, `"completed"`),
`conclusion: String` (`"success"`, `"failure"`, `"cancelled"`, `"skipped"`) —
gh reports an **empty string until the run completes** (not `null`),
`workflow_name: String`, `head_branch: String`, `event: String`, `url: String`,
`created_at: String` (ISO 8601).

### `CheckRun`

From `pr_checks`. Fields: `name: String`, `state: String` (`"SUCCESS"`,
`"FAILURE"`, `"IN_PROGRESS"`, …), `bucket: CheckBucket` — gh's categorisation of
`state` and the field to branch on: the typed enum `Pass`/`Fail`/`Pending`/
`Skipping`/`Cancel` (+ an `Unknown` catch-all for forward compatibility), with
`is_failing()`/`is_pending()`/`is_passing()`/`is_unknown()` helpers; `workflow: String` (empty for non-Actions checks),
`link: String`, `started_at: String` — **empty until the check starts**,
`completed_at: String` — **empty until it completes**.

### `Release`

From `release_list` / `release_view`. Fields: `tag_name: String`,
`name: String` (may be empty), `body: String` — **empty from `release_list`**,
`url: String` — **empty from `release_list`**, `published_at: String` (ISO 8601,
empty for a draft), `is_draft: bool`, `is_prerelease: bool`, `is_latest: bool` —
**only `release_list` reports this; from `release_view` it defaults to
`false`**.

### `Review`

From `pr_feedback` (`pr view --json reviews`). Fields: `author: String` (login;
empty for a deleted account), `state: String` (`"APPROVED"`,
`"CHANGES_REQUESTED"`, `"COMMENTED"`, `"DISMISSED"`, `"PENDING"`),
`body: String` (may be empty), `submitted_at: String` (ISO 8601).

### `Comment`

From `pr_feedback` (`pr view --json comments`). Fields: `author: String` (login;
empty for a deleted account), `body: String`, `url: String`,
`created_at: String` (ISO 8601).

### `PrFeedback`

From `pr_feedback`. Fields: `reviews: Vec<Review>` and `comments: Vec<Comment>`,
each in gh's order (oldest first).

### `Repo`

From `repo_view`, flattening gh's nested objects. Fields: `name: String`,
`owner: String` (the login), `description: Option<String>` (`None` when GitHub
returns `null`), `url: String`, `is_private: bool`, `default_branch: String`
(empty for an empty repository).

## Config types

### `MergeStrategy`

`#[non_exhaustive]` enum naming gh's mutually exclusive strategy flags:

```rust,ignore
pub enum MergeStrategy {
    Merge,  // --merge   (a merge commit)
    Squash, // --squash  (one commit)
    Rebase, // --rebase  (onto the base)
}
```

### `PrMerge`

The [`pr_merge`](#pull-requests--lifecycle) options. `#[non_exhaustive]` — build
it through the strategy constructor, then chain the optional flags, rather than
a struct literal:

```rust,ignore
# use vcs_github::PrMerge;
let _ = PrMerge::merge();                          // --merge
let _ = PrMerge::squash().delete_branch();         // --squash --delete-branch
let _ = PrMerge::rebase().auto();                  // --rebase --auto
let _ = PrMerge::squash().auto().delete_branch();  // --squash --auto --delete-branch
```

`merge()` / `squash()` / `rebase()` pick the strategy (all default `auto: false`,
`delete_branch: false`); `auto()` enables `--auto` (merge once requirements are
met); `delete_branch()` enables `--delete-branch`. Public fields: `strategy:
MergeStrategy`, `auto: bool`, `delete_branch: bool`.

### `PrCreate`

The [`pr_create`](#pull-requests--listing--creation) options. `#[non_exhaustive]`
with private-by-spec ergonomics — build through `PrCreate::new(title, body)` and
chain the optional branch setters rather than a struct literal:

```rust,ignore
# use vcs_github::PrCreate;
let _ = PrCreate::new("Add streaming", "Implements …");        // current branch → repo default
let _ = PrCreate::new("Add streaming", "Implements …")
    .head("feat/streaming").base("main");                      // --head feat/streaming --base main
```

`new(title, body)` takes `impl Into<String>` (source/target left to gh's
defaults); `.head(b)` sets `--head` (the source branch), `.base(b)` sets `--base`
(the target). Public fields: `title: String`, `body: String`,
`head: Option<String>`, `base: Option<String>`.

### `ReviewAction`

What [`pr_review`](#pull-requests--review--feedback) submits. Now a
`#[non_exhaustive]` **struct** with private fields, so the invariant holds by
construction — gh *requires* a body for request-changes/comment reviews, so those
are only reachable through the constructors that take one, and an empty-body
request-changes is unrepresentable. The review kind is a separate
[`ReviewKind`](#reviewkind) enum read back via `.kind()`.

```rust,ignore
# use vcs_github::{ReviewAction, ReviewKind};
let _ = ReviewAction::approve();                          // --approve (no body)
let _ = ReviewAction::approve().with_body("LGTM");        // --approve --body LGTM
let _ = ReviewAction::request_changes("fix the parser");  // --request-changes --body <body>
let _ = ReviewAction::comment("nice");                    // --comment --body <body>

let a = ReviewAction::approve().with_body("LGTM");
assert_eq!(a.kind(), ReviewKind::Approve);
assert_eq!(a.body(), Some("LGTM"));
```

- `approve()` — approve with no body; attach one with `.with_body(b)`.
- `request_changes(body)` / `comment(body)` — gh requires the body, so it is
  taken by construction.
- `.with_body(body)` — attach or replace the body (mainly to give an approve a
  message).
- `.kind() -> ReviewKind` / `.body() -> Option<&str>` — read the parts back.

### `ReviewKind`

`#[non_exhaustive]`, `Copy` enum naming which review `ReviewAction` submits, read
back via [`ReviewAction::kind`](#reviewaction):

```rust,ignore
pub enum ReviewKind {
    Approve,         // --approve
    RequestChanges,  // --request-changes
    Comment,         // --comment
}
```

## See also

- [Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/) — the `mock` feature (`MockGitHubApi`) and the
  `ScriptedRunner` seam.
- [Process model & errors](https://docs.rs/vcs-core/latest/vcs_core/guide/process_model/) — OS-job containment, timeouts, and
  the `Error` / `ProcessResult` shapes.
- [crate docs](https://docs.rs/vcs-github) — quickstart and crate-level docs.
