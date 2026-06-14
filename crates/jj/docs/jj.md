# vcs-jj — Jujutsu CLI guide

**What you can do:** working-copy status & the change log, describe/new change,
bookmarks, the operation log (restore/undo), workspaces, squash/split/absorb/
duplicate/abandon, diff & template queries, git sync, parse & resolve jj's native
conflicts, and op-log-rollback transactions. This guide is the full reference —
every command by theme, with examples.

Typed, repo-scoped, **async** commands over the `jj` binary, behind a mockable
interface. Every method runs `jj` inside an OS job (via [`processkit`]) so a
subprocess is never orphaned, returns the structured `Error`, and honours an
optional timeout.

There is deliberately **no `Jj::hardened()`** — jj has no repo-local hooks, and
its config comes from the user/repo TOML files jj itself trusts. In a *colocated*
repo the risk lives on the git side (git hooks fire when **git** commands run
there), so harden the `Git` client you point at it instead.

[`processkit`]: https://crates.io/crates/processkit

## Construction & configuration

```rust,ignore
# use std::time::Duration;
use vcs_jj::Jj;

let jj = Jj::new();                                       // real, job-backed runner
let jj = Jj::new().default_timeout(Duration::from_secs(10)); // every cmd → Error::Timeout past 10s
```

- `Jj::new()` — the production client over the real job-backed runner.
- `Jj::with_runner(runner)` — inject a fake `ProcessRunner` (e.g.
  `processkit::testing::ScriptedRunner`) for hermetic tests; see [Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/).
- `default_timeout(Duration)` — builder; arms a per-command timeout.

All three come from the `processkit::cli_client!` macro that defines `Jj`.

### The cwd-bound view (`JjAt`)

Most `JjApi` methods take a leading `dir: &Path`. When you drive one directory
repeatedly, bind it once with `jj.at(&path)` — the returned `JjAt` drops that
argument:

```rust,ignore
# use std::path::Path;
# use vcs_jj::Jj;
# async fn demo(repo: &Path) -> Result<(), processkit::Error> {
let jj = Jj::new();
let at = jj.at(repo);          // JjAt — Copy, borrows the client + path
let head = at.current_change().await?;   // == jj.current_change(repo)
at.describe("feat: thing").await?;        // == jj.describe(repo, "…")
# Ok(()) }
```

`JjAt` is `Copy` for every runner (it holds only references). The dir-taking
`JjApi` methods stay on `Jj` so one client can drive many directories (e.g.
workspaces). Through the facade, `vcs_core::Repo::jj_at` yields the same handle.

### Inherent `run_args` / `run_raw_args`

The object-safe `JjApi` trait can't take `&[&str]`, so two inherent helpers do —
no `Vec<String>` allocation:

```rust,ignore
# use vcs_jj::Jj;
# async fn demo(jj: &Jj) -> Result<(), processkit::Error> {
let out = jj.run_args(&["log", "-r", "@"]).await?;          // String, errors on non-zero exit
let res = jj.run_raw_args(&["status"]).await?;              // ProcessResult<String>, never errors on exit
# let _ = (out, res); Ok(()) }
```

### `transaction` — op-log rollback

Run a closure with op-log rollback: capture the current operation
([`op_head`]), run `f` against a bound `JjAt`, and on `Err` restore the repo to
that operation ([`op_restore`]) before propagating the error.

```rust,ignore
# use std::path::Path;
# use vcs_jj::Jj;
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
jj.transaction(repo, |tx| async move {
    tx.describe("wip").await?;
    tx.new_change("next").await        // an Err here rolls back the describe
})
.await?;
# Ok(()) }
```

Signature:

```rust,ignore
pub async fn transaction<'a, T, F, Fut>(&'a self, dir: &'a Path, f: F) -> Result<T>
where
    F: FnOnce(JjAt<'a, R>) -> Fut,
    Fut: Future<Output = Result<T>> + 'a;
```

Inherent (not on the object-safe trait): the closure parameter is generic, which
`mockall`/trait objects can't express. `JjAt::transaction(f)` is the bound form.

**Caveats** (verbatim from source): rollback runs on `Err` only — **not** on
panic or a dropped future (no async `Drop`); convert panics to `Err` inside `f`
if you need that. If the restore itself fails, the *original* error is returned
and the repo may be left mid-transaction — re-probe [`op_head`] to detect that.

---

## Status & changes

```rust,ignore
async fn status(&self, dir: &Path) -> Result<Vec<ChangedPath>>;
async fn status_text(&self, dir: &Path) -> Result<String>;
async fn current_change(&self, dir: &Path) -> Result<Change>;
```

`status` is the machine-stable form of `jj status` — it runs `diff -r @
--summary` and parses one `ChangedPath` per `<letter> <path>` line (mirrors
`vcs_git::status`). `status_text` is the raw human-readable `jj status` text.
`current_change` is `log -r @` reduced to one [`Change`].

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
for c in jj.status(repo).await? {                  // Vec<ChangedPath>
    println!("{} {}", c.status, c.path);           // e.g. 'M' src/lib.rs
}
let head = jj.current_change(repo).await?;         // Change { change_id, commit_id, empty, description }
# Ok(()) }
```

## Log

```rust,ignore
async fn log(&self, dir: &Path, revset: &str, max: usize) -> Result<Vec<Change>>;
async fn evolog(&self, dir: &Path, revset: &str, max: usize) -> Result<Vec<Change>>;
```

`log` returns changes matching `revset`, newest first, up to `max` (`jj log`).
`evolog` returns how the commit `revset` resolves to evolved — newest snapshot
first, one [`Change`] per recorded predecessor (`jj evolog`).

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
for c in jj.log(repo, "::@", 10).await? {          // Vec<Change>
    println!("{} {}{}", c.change_id, if c.empty { "(empty) " } else { "" }, c.description);
}
let history = jj.evolog(repo, "@", 5).await?;      // Vec<Change>
# let _ = history; Ok(()) }
```

## Descriptions

```rust,ignore
async fn describe(&self, dir: &Path, message: &str) -> Result<()>;
async fn describe_rev(&self, dir: &Path, revset: &str, message: &str) -> Result<()>;
async fn new_change(&self, dir: &Path, message: &str) -> Result<()>;
async fn description(&self, dir: &Path, revset: &str) -> Result<String>;
```

`describe` sets `@`'s description (`describe -m`); `describe_rev` an arbitrary
revision (`describe -r <revset> -m`). `new_change` starts a fresh change on top
(`new -m`). `description` returns the full (possibly multiline) description of
the commit `revset` resolves to, trailing whitespace trimmed — empty for an
undescribed change *or* for a revset matching no commit (an *invalid* revset
still errors); a multi-commit revset yields only the newest commit's
description.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
jj.describe(repo, "feat: parser").await?;
jj.new_change(repo, "wip: follow-up").await?;
let msg = jj.description(repo, "@-").await?;        // String (empty if undescribed)
# let _ = msg; Ok(()) }
```

## Bookmarks

```rust,ignore
async fn bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>>;
async fn bookmarks_all(&self, dir: &Path) -> Result<Vec<BookmarkRef>>;
async fn reachable_bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>>;
async fn current_bookmark(&self, dir: &Path) -> Result<Option<String>>;
async fn trunk(&self, dir: &Path) -> Result<Option<String>>;
async fn bookmark_create(&self, dir: &Path, name: &str, revision: &str) -> Result<()>;
async fn bookmark_delete(&self, dir: &Path, name: &str) -> Result<()>;
async fn bookmark_rename(&self, dir: &Path, old: &str, new: &str) -> Result<()>;
async fn bookmark_track(&self, dir: &Path, name: &str, remote: &str) -> Result<()>;
async fn bookmark_set(&self, dir: &Path, name: &str, revision: &str) -> Result<()>;
async fn bookmark_move(&self, dir: &Path, name: &str, to: &str, allow_backwards: bool) -> Result<()>;
```

- `bookmarks` — local bookmarks (`bookmark list`).
- `bookmarks_all` — local *and* remote-tracking (`bookmark list -a`); richer
  [`BookmarkRef`] rows.
- `reachable_bookmarks` — local bookmarks on the nearest commits reachable from
  `@` (`log -r 'heads(::@ & bookmarks())'`); the candidate targets a commit
  "belongs to". A commit carrying several bookmarks yields one entry each.
- `current_bookmark` — the single bookmark on `@` (or the first of several);
  `None` when `@` carries none.
- `trunk` — the trunk bookmark (`log -r 'trunk()'`); `None` when unresolved.
- `bookmark_create` / `bookmark_delete` / `bookmark_rename` — at/by name.
- `bookmark_track` — track a remote bookmark (`bookmark track <name>@<remote>`).
- `bookmark_set` — point a bookmark at `revision` (`bookmark set <name> -r`).
- `bookmark_move` — move to `to`; pass `allow_backwards` to append
  `--allow-backwards`.

Every name-taking method rejects an empty or leading-`-` name *before* spawning
(see [Validating newtypes](#validating-newtypes--filesets)).

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
jj.bookmark_set(repo, "main", "@").await?;           // point `main` at @
for b in jj.bookmarks(repo).await? {                 // Vec<Bookmark>
    println!("{} -> {}", b.name, b.target);
}
if let Some(trunk) = jj.trunk(repo).await? {          // Option<String>
    println!("trunk = {trunk}");
}
# Ok(()) }
```

## Diff & query

```rust,ignore
async fn diff(&self, dir: &Path, spec: DiffSpec) -> Result<Vec<FileDiff>>;
async fn diff_text(&self, dir: &Path, spec: DiffSpec) -> Result<String>;
async fn diff_summary(&self, dir: &Path, from: &str, to: &str) -> Result<Vec<ChangedPath>>;
async fn diff_stat(&self, dir: &Path, revset: &str) -> Result<DiffStat>;
async fn commit_count(&self, dir: &Path, revset: &str) -> Result<usize>;
async fn template_query(&self, dir: &Path, revset: &str, template: &str, limit: Option<usize>) -> Result<String>;
```

- `diff` — parsed per-file unified diff for [`DiffSpec`] (layered on `diff_text`).
- `diff_text` — raw git-format unified diff (`diff -r <spec> --git`); stable
  machine output.
- `diff_summary` — per-file change summary for a range; the endpoints are
  parenthesised internally (`(from)..(to)`) so a compound revset keeps its
  meaning.
- `diff_stat` — aggregate counts for a revset (`diff -r <revset> --stat`).
- `commit_count` — number of commits in a revset (one id per line, counted).
- `template_query` — run an arbitrary templated `jj log` query and return raw
  stdout (`log -r <revset> --no-graph [--limit n] -T <template>`); the escape
  hatch the typed queries are built on.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi, DiffSpec};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
let files = jj.diff(repo, DiffSpec::WorkingTree).await?;     // Vec<FileDiff>
let text  = jj.diff_text(repo, DiffSpec::Rev("@-".into())).await?; // String (git format)
let stat  = jj.diff_stat(repo, "@").await?;                  // DiffStat
let n     = jj.commit_count(repo, "main..@").await?;         // usize
let raw   = jj.template_query(repo, "@", "change_id.short()", Some(1)).await?; // String
# let _ = (files, text, stat, n, raw); Ok(()) }
```

## File inspection

```rust,ignore
async fn file_show(&self, dir: &Path, revset: &str, path: &str) -> Result<String>;
async fn file_annotate(&self, dir: &Path, path: &str, revset: Option<String>) -> Result<Vec<AnnotationLine>>;
```

`file_show` returns a file's content at a revision. `path` is wrapped as an
exact-path fileset (`file:"<path>"`) so fileset metacharacters in the name stay
literal; content is decoded **lossily** — a binary file comes back mangled
rather than erroring.

`file_annotate` returns per-line authorship (`file annotate`; `revset: None` =
`@`): which change introduced each line. Here `path` is a plain PATH (jj's
`file annotate` rejects the `file:"…"` form), passed after a `--` separator so a
`-dash.txt` stays literal.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
let src = jj.file_show(repo, "@", "src/lib.rs").await?;           // String
for line in jj.file_annotate(repo, "src/lib.rs", None).await? {   // Vec<AnnotationLine>
    println!("{:>4} {} {}", line.line, line.change_id, line.content);
}
# let _ = src; Ok(()) }
```

## Conflict probing

```rust,ignore
async fn is_conflicted(&self, dir: &Path, revset: &str) -> Result<bool>;
async fn has_workingcopy_conflict(&self, dir: &Path) -> Result<bool>;
async fn resolve_list(&self, dir: &Path, revset: &str) -> Result<Vec<String>>;
```

`is_conflicted` asks the template engine whether the commit a revset resolves to
has a conflict (no localized-prose matching). `has_workingcopy_conflict` is
`is_conflicted(dir, "@")`. `resolve_list` returns the paths with unresolved
conflicts in `revset` (`resolve --list -r <revset>`), forward-slash normalised —
empty when there are none. Parsing the *materialized* markers in a conflicted
file is a separate, pure module: see [Conflict resolution](https://docs.rs/vcs-git/latest/vcs_git/guide/conflicts/).

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
if jj.has_workingcopy_conflict(repo).await? {
    for p in jj.resolve_list(repo, "@").await? {     // Vec<String>
        eprintln!("conflict: {p}");
    }
}
# Ok(()) }
```

## Rebasing & editing

```rust,ignore
async fn rebase(&self, dir: &Path, onto: &str) -> Result<()>;
async fn rebase_branch(&self, dir: &Path, branch: &str, dest: &str) -> Result<()>;
async fn edit(&self, dir: &Path, revset: &str) -> Result<()>;
```

`rebase` moves the working copy onto a destination (`rebase -d <onto>`);
`rebase_branch` a whole branch (`rebase -b <branch> -d <dest>`); `edit` moves
the working copy to a revision (`edit <rev>`). `edit`'s revset is guarded
against a leading-`-` value.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
jj.rebase(repo, "main").await?;
jj.edit(repo, "@-").await?;
# Ok(()) }
```

## Squash & split

```rust,ignore
async fn squash_into(&self, dir: &Path, into: &str, use_destination_message: bool) -> Result<()>;
async fn commit_paths(&self, dir: &Path, filesets: &[JjFileset], message: &str) -> Result<()>;
async fn squash_paths(&self, dir: &Path, spec: SquashPaths) -> Result<()>;
async fn split_paths(&self, dir: &Path, filesets: &[JjFileset], message: &str) -> Result<()>;
async fn absorb(&self, dir: &Path, from: Option<String>, filesets: &[JjFileset]) -> Result<()>;
```

- `squash_into` — squash the working copy into `into` (`squash --into`). With
  `use_destination_message`, keep the destination's description
  (`--use-destination-message`) instead of combining the two.
- `commit_paths` — finalise a commit from exactly these [`JjFileset`]s
  (`commit -m <message> <filesets>`); the rest stay in the new working-copy
  change.
- `squash_paths` — squash exactly the spec's filesets from one revision into
  another (`squash --from <from> --into <into> [--use-destination-message]
  <filesets>`); built through [`SquashPaths`](#squashpaths).
- `split_paths` — split exactly these filesets out of `@` into their own commit
  (`split -m <message> <filesets>`). `filesets` must be **non-empty** — a
  fileset-less split opens jj's interactive diff editor (a headless hang), so it
  is refused with an [`Error::Spawn`] before spawning.
- `absorb` — fold working-copy edits into the mutable ancestors that introduced
  the touched lines (`absorb [--from <revset>] [<filesets>…]`); an empty
  `filesets` absorbs everything.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi, JjFileset, SquashPaths};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
let only = [JjFileset::path("src/parser.rs")];
jj.split_paths(repo, &only, "feat: parser").await?;
jj.commit_paths(repo, &only, "feat: parser").await?;
jj.squash_into(repo, "@-", false).await?;
jj.squash_paths(repo, SquashPaths::new("@", "@-").filesets(only)).await?;
jj.absorb(repo, None, &[]).await?;            // absorb everything into ancestors
# Ok(()) }
```

## Sparse

```rust,ignore
async fn sparse_set(&self, dir: &Path, patterns: &[String]) -> Result<()>;
```

Set the working copy's sparse patterns to exactly `patterns` (`sparse set
--clear --add <p>…`): `--clear` empties first, then each `--add` reinstates one
pattern — an empty list clears the working copy.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
jj.sparse_set(repo, &["src".into(), "Cargo.toml".into()]).await?;
# Ok(()) }
```

## Merging

```rust,ignore
async fn new_merge(&self, dir: &Path, message: &str, parents: Vec<String>) -> Result<()>;
async fn duplicate(&self, dir: &Path, revset: &str) -> Result<()>;
async fn abandon(&self, dir: &Path, revset: &str) -> Result<()>;
```

`new_merge` creates a new change with the given parents (`new -m <msg> <p1>
<p2> …`); each parent is a bare positional and is guarded against a leading-`-`
value. `duplicate` duplicates the commits a revset resolves to. `abandon`
abandons a revision; its revset is guarded too.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
jj.new_merge(repo, "merge: a + b", vec!["feature-a".into(), "feature-b".into()]).await?;
jj.duplicate(repo, "abc123").await?;
jj.abandon(repo, "@-").await?;
# Ok(()) }
```

## Git integration

```rust,ignore
async fn git_fetch(&self, dir: &Path) -> Result<()>;
async fn git_fetch_from(&self, dir: &Path, remote: &str) -> Result<()>;
async fn git_fetch_branch(&self, dir: &Path, branch: &str) -> Result<()>;
async fn git_push(&self, dir: &Path, bookmark: Option<String>) -> Result<()>;
async fn git_import(&self, dir: &Path) -> Result<()>;
async fn git_clone(&self, url: &str, dest: &Path, colocate: bool) -> Result<()>;
```

- `git_fetch` — `jj git fetch`. Transient (network) failures are retried: 3
  attempts, 500 ms backoff (DNS, timeout, dropped connection — see
  `is_transient_fetch_error`).
- `git_fetch_from` — fetch a named remote (`git fetch --remote <remote>`); same
  retry policy.
- `git_fetch_branch` — fetch a single bookmark from origin (`git fetch --remote
  origin -b <branch>`); same retry policy.
- `git_push` — `jj git push`, optionally `-b <bookmark>`. The bookmark is owned
  (`Option<String>`) to keep the trait `mockall`-friendly.
- `git_import` — import git refs into jj (`jj git import`) — colocated-repo sync.
- `git_clone` — clone into `dest` (`git clone <url> <dest>
  --colocate|--no-colocate`). Runs **without** a working directory — pass an
  **absolute** `dest`. The colocate flag is *always* passed explicitly:
  whether colocation is jj's default depends on the jj version and the user's
  `git.colocate` config, so `colocate` decides deterministically. `url` is
  guarded against a leading-`-` value.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
jj.git_fetch(repo).await?;
jj.git_push(repo, Some("main".to_string())).await?;     // `jj git push -b main`
jj.git_clone("https://example.com/r.git", Path::new("/abs/dest"), true).await?;
# Ok(()) }
```

## Workspaces

```rust,ignore
async fn workspace_list(&self, dir: &Path) -> Result<Vec<Workspace>>;
async fn workspace_root(&self, dir: &Path, name: Option<String>) -> Result<PathBuf>;
async fn workspace_add(&self, dir: &Path, spec: WorkspaceAdd) -> Result<()>;
async fn workspace_forget(&self, dir: &Path, name: &str) -> Result<()>;
```

jj's worktrees, with structured results. `workspace_list` returns
[`Workspace`] rows; `workspace_root` resolves a workspace's root path
(`workspace root [--name <name>]`); `workspace_add` adds one from a
[`WorkspaceAdd`] spec; `workspace_forget` forgets one by name.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi, WorkspaceAdd};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
jj.workspace_add(repo, WorkspaceAdd::new("feature", "@", "/tmp/feature")).await?;
for ws in jj.workspace_list(repo).await? {              // Vec<Workspace>
    println!("{} @ {} {:?}", ws.name, ws.commit, ws.bookmarks);
}
jj.workspace_forget(repo, "feature").await?;
# Ok(()) }
```

> A synchronous, best-effort `vcs_jj::blocking` module mirrors `workspace_forget`
> (and `workspace_name_for_path`) for `Drop` guards that cannot `.await`. It
> shells out via `std::process` directly — no async, no job containment — so
> reserve it for short-lived cleanup.

## Operation log

```rust,ignore
async fn op_head(&self, dir: &Path) -> Result<String>;
async fn op_log(&self, dir: &Path, limit: usize) -> Result<Vec<Operation>>;
async fn op_restore(&self, dir: &Path, op_id: &str) -> Result<()>;
async fn op_undo(&self, dir: &Path) -> Result<()>;
```

`op_head` returns the current operation id (`op log --no-graph --limit 1`) —
capture it before a risky sequence to roll back to. `op_log` returns the newest
`limit` [`Operation`]s, newest first. `op_restore` restores the repo to an
operation (`op restore <id>`; the id is guarded). `op_undo` undoes the latest
operation. (`transaction` is the higher-level wrapper around capture + restore.)

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
let head = jj.op_head(repo).await?;            // String — capture before mutating
// … risky work …
jj.op_restore(repo, &head).await?;             // roll back
for op in jj.op_log(repo, 5).await? {          // Vec<Operation>
    println!("{} {} {}", op.id, op.time, op.description);
}
# Ok(()) }
```

## Discovery

```rust,ignore
async fn root(&self, dir: &Path) -> Result<PathBuf>;
async fn version(&self) -> Result<String>;
async fn capabilities(&self) -> Result<JjCapabilities>;
```

`root` is the working-copy root of the current workspace (`jj root`). `version`
is the raw `jj --version` string. `capabilities` parses that into
[`JjCapabilities`] — a value type; probe once and keep the result. The crate's
validated floor is **jj ≥ 0.38** (`JjCapabilities::is_supported`); an
unrecognisable version string is an `Error::Parse`.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
let caps = jj.capabilities().await?;           // JjCapabilities
caps.ensure_supported()?;                      // clear "needs jj >= 0.38, found 0.35.0"
println!("jj {} (root {})", caps.version, jj.root(repo).await?.display());
# Ok(()) }
```

## Raw escape hatches

```rust,ignore
async fn run(&self, args: &[String]) -> Result<String>;
async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>>;
```

`run` executes `jj <args>` and returns trimmed stdout (errors on a non-zero
exit). `run_raw` never errors on a non-zero exit — it returns the captured
[`ProcessResult`] so the caller inspects `code()`/`stdout()`/`stderr()`. These
are **not** injection-guarded; the inherent `run_args`/`run_raw_args` are the
`&[&str]` siblings.

```rust,ignore
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj) -> Result<(), processkit::Error> {
let out = jj.run(&["log".into(), "-r".into(), "@".into()]).await?;   // String
let res = jj.run_raw(&["status".into()]).await?;                    // ProcessResult<String>
# let _ = (out, res); Ok(()) }
```

---

## Result types

The diff types (`ChangeKind`, `DiffLine`, `Hunk`, `FileDiff`, `DiffStat`,
`parse_diff`) and `JjVersion` actually live in the shared
[`vcs-diff`](https://crates.io/crates/vcs-diff) crate — `jj diff --git` and
`git diff` are byte-identical, so `vcs-jj` and `vcs-git` share one parser. They're
re-exported here, so `vcs_jj::FileDiff` etc. still resolve (`JjVersion` is an
alias of `vcs_diff::Version`).

### `Change`
A jj change, parsed from a tab-delimited template row.

| Field | Type | Notes |
| --- | --- | --- |
| `change_id` | `String` | Short change id (`change_id.short()`). |
| `commit_id` | `String` | Short commit id. |
| `empty` | `bool` | `true` when the change makes no file modifications. |
| `description` | `String` | First line of the description (empty if undescribed). |

### `Bookmark`
| Field | Type | Notes |
| --- | --- | --- |
| `name` | `String` | Bookmark name. |
| `target` | `String` | Short id of the commit it points at. |

### `BookmarkRef`
From `bookmark list -a` — local *or* remote-tracking.

| Field | Type | Notes |
| --- | --- | --- |
| `name` | `String` | Bookmark name. |
| `remote` | `Option<String>` | Remote (e.g. `origin`/`git`); `None` for a local. |
| `target` | `String` | Short commit id (empty for a conflicted bookmark). |
| `tracked` | `bool` | Whether this remote-tracking bookmark is tracked (`false` for locals). |

### `Workspace`
| Field | Type | Notes |
| --- | --- | --- |
| `name` | `String` | Workspace name (`default` for the main one). |
| `commit` | `String` | Short commit id of the working-copy commit. |
| `bookmarks` | `Vec<String>` | Local bookmarks at that commit (empty when none). |

### `ChangedPath`
One `jj diff --summary` entry.

| Field | Type | Notes |
| --- | --- | --- |
| `status` | `char` | `M` modified, `A` added, `D` deleted, `R` renamed, `C` copied. |
| `path` | `String` | The path the status applies to — the *new* path for a rename/copy (forward-slash normalised). |
| `old_path` | `Option<String>` | For `R`/`C`, the original path; `None` otherwise. |

### `DiffStat`
Aggregate counts from the `diff --stat` footer (`Copy`, `Default`).

| Field | Type |
| --- | --- |
| `files_changed` | `usize` |
| `insertions` | `usize` |
| `deletions` | `usize` |

### `FileDiff`
One file's entry in a parsed git-format unified diff.

| Field | Type | Notes |
| --- | --- | --- |
| `change` | `ChangeKind` | How the file changed. |
| `path` | `String` | Path — the *new* path for a rename — forward-slash normalised. |
| `old_path` | `Option<String>` | For a rename, the original path; `None` otherwise. |
| `hunks` | `Vec<Hunk>` | The `@@` hunks; empty for a binary file or pure rename. |
| `raw` | `String` | The verbatim `diff --git …` section, for callers that display raw text. |

#### `Hunk`
| Field | Type | Notes |
| --- | --- | --- |
| `old_start` | `usize` | Start line in the old file. |
| `old_lines` | `usize` | Old-file line count (defaults to 1 when `,<count>` omitted). |
| `new_start` | `usize` | Start line in the new file. |
| `new_lines` | `usize` | New-file line count (defaults to 1 when omitted). |
| `section` | `String` | Text after the closing `@@` (function/section heading); empty when none. |
| `lines` | `Vec<DiffLine>` | One entry per `+`/`-`/` ` line. |

#### `DiffLine` (enum)
The stored text excludes the leading ` `/`+`/`-` marker.
- `Context(String)` — unchanged context line.
- `Added(String)` — added line.
- `Removed(String)` — removed line.

#### `ChangeKind` (enum, `Copy`)
- `Added` — `new file mode …`.
- `Modified` — contents changed.
- `Deleted` — `deleted file mode …`.
- `Renamed` — `rename from …` / `rename to …`.

### `Operation`
One `jj op log` row.

| Field | Type | Notes |
| --- | --- | --- |
| `id` | `String` | Short operation id — what `op restore`/`op undo` take. |
| `user` | `String` | OS-level `user@host` that ran the operation (not the jj author). |
| `time` | `String` | Start timestamp, ISO 8601 with offset. |
| `description` | `String` | First line of the operation description (e.g. `new empty commit`). |

### `AnnotationLine`
One `jj file annotate` line.

| Field | Type | Notes |
| --- | --- | --- |
| `change_id` | `String` | Short change id that introduced the line. |
| `line` | `u32` | 1-based line number in the annotated file. |
| `content` | `String` | The line's content (no trailing newline). |

### `JjVersion`
Parsed `jj --version` (`Copy`, `Ord`). Fields: `major: u64`, `minor: u64`,
`patch: u64` (patch reads `0` when the binary reports only `major.minor`).
`Display` renders `major.minor.patch`.

### `JjCapabilities`
What the installed binary supports (`Copy`, `#[non_exhaustive]`).

| Field | Type |
| --- | --- |
| `version` | `JjVersion` |

Methods: `is_supported() -> bool` (jj ≥ 0.38) and `ensure_supported() ->
Result<()>` (a clear "needs jj >= 0.38, found …" error otherwise).

---

## Config & builder types

### `DiffSpec` (enum, `#[non_exhaustive]`)
What `diff` / `diff_text` compares.
- `WorkingTree` — the working-copy change's diff (`jj diff -r @`).
- `Rev(String)` — a specific revset, e.g. `@-` or `main..@` (`jj diff -r <revset>`).

### `SparseMode` (enum, `Copy`, `#[non_exhaustive]`)
How a new workspace inherits sparse patterns (`--sparse-patterns <mode>`).
- `Copy` — copy all patterns from the current workspace (jj's default).
- `Full` — include every file.
- `Empty` — start with no files; the caller sets patterns afterwards (CoW flow).

### `WorkspaceAdd` (`#[non_exhaustive]`)
Options for `workspace_add`; build through `WorkspaceAdd::new`.

| Field | Type | Notes |
| --- | --- | --- |
| `name` | `String` | Name for the new workspace. |
| `base` | `String` | Revision the working copy starts at (`-r <base>`). |
| `path` | `PathBuf` | Filesystem path for the new workspace. |
| `sparse_patterns` | `Option<SparseMode>` | `--sparse-patterns`; `None` leaves jj's default. |

```rust,ignore
# use vcs_jj::{WorkspaceAdd, SparseMode};
let spec = WorkspaceAdd::new("feature", "@", "/tmp/feature")
    .sparse(SparseMode::Empty);    // start empty, then sparse_set later
# let _ = spec;
```

`WorkspaceAdd::new(name, base, path)` takes `impl Into<String>` /
`impl Into<String>` / `impl Into<PathBuf>`; `.sparse(mode)` is the builder for
`sparse_patterns`.

### `SquashPaths` (`#[non_exhaustive]`)
Options for `squash_paths`; build through `SquashPaths::new` and the chained
setters.

| Field | Type | Notes |
| --- | --- | --- |
| `from` | `String` | Source revision the filesets are squashed out of (`--from`). |
| `into` | `String` | Destination revision they squash into (`--into`). |
| `filesets` | `Vec<JjFileset>` | The exact filesets to move; empty squashes the whole `from` change. |
| `use_destination_message` | `bool` | Keep the destination's description (`--use-destination-message`). |

```rust,ignore
# use vcs_jj::{SquashPaths, JjFileset};
let spec = SquashPaths::new("@", "@-")
    .filesets([JjFileset::path("src/parser.rs")])
    .use_destination_message();
# let _ = spec;
```

`SquashPaths::new(from, into)` takes `impl Into<String>` / `impl Into<String>`
(no filesets selected yet); `.filesets(impl IntoIterator<Item = JjFileset>)` sets
them (replacing any already added), and `.use_destination_message()` keeps the
destination's description instead of combining the two.

---

## Validating newtypes & filesets

### `RevsetExpr`
Optional up-front validation for callers that accept revsets from untrusted
input (UIs, bots, agents) and want to fail early. Deliberately *minimal* — jj's
revset grammar is too rich to validate here — it only guarantees the expression
is non-empty and cannot be parsed as a flag (no leading `-`), matching the
internal guard the positional-revset methods apply anyway. The dir-taking
methods stay `&str`; this type is optional validation, **not** a required
wrapper.

```rust,ignore
# use vcs_jj::RevsetExpr;
let r = RevsetExpr::new("main..@")?;       // Ok
assert!(RevsetExpr::new("").is_err());     // empty
assert!(RevsetExpr::new("-x").is_err());   // leading `-` → would parse as a flag
# Ok::<(), processkit::Error>(())
```

`RevsetExpr::new(impl Into<String>) -> Result<Self>`; `.as_str() -> &str`;
implements `Display`.

### `JjFileset`
An exact-path jj fileset (`file:"<path>"`), so path metacharacters like `(`,
`)`, `|`, `*` are treated literally rather than as fileset operators. Build it
with `JjFileset::path(path)` (repo-root-relative); a Windows backslash separator
is normalised to `/` (jj filesets are forward-slash — a literal-backslash path
would match nothing), and a `"` is escaped for the `file:"…"` literal.

```rust,ignore
# use vcs_jj::JjFileset;
let fs = JjFileset::path(r#"src/a (copy).rs"#);
assert_eq!(fs.as_str(), r#"file:"src/a (copy).rs""#);
```

`JjFileset::path(impl AsRef<str>) -> Self`; `.as_str() -> &str`.

### Why injection guards, and why filesets

Every method that places a caller-supplied bookmark name, revset, parent, url,
or operation id in a *bare positional* argv slot refuses an empty or leading-`-`
value with an `Error::Spawn` **before** spawning (verified: `jj edit -evil` →
"unexpected argument"). Flag-*value* slots (`-r <revset>`, `-m <msg>`) and the
`run`/`run_raw` escape hatches are *not* guarded — jj itself rejects dash-values
there with a clear error rather than misparsing them.

`split_paths`/`commit_paths`/`squash_paths`/`absorb` take `&[JjFileset]` rather
than raw strings so path metacharacters can never be reinterpreted as fileset
operators. For `split_paths` this is load-bearing for a different reason: an
empty fileset list makes `jj split` open its **interactive diff editor**, which
would hang a headless run indefinitely — so `split_paths` refuses an empty slice
before spawning.

---

## See also

- [Conflict resolution](https://docs.rs/vcs-git/latest/vcs_git/guide/conflicts/) — the `vcs_jj::conflict` module (parse /
  render / resolve materialized jj conflict markers).
- [Testing & mocking](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/) — `MockJjApi` and `ScriptedRunner`.
- [Security & hardening](https://docs.rs/vcs-git/latest/vcs_git/guide/security/) — why there is no `Jj::hardened()`, and the
  injection-guard model.
- [Process model & errors](https://docs.rs/vcs-core/latest/vcs_core/guide/process_model/) — job containment, timeouts, the
  `Error` variants.
- [crate docs](https://docs.rs/vcs-jj)

[`op_head`]: #operation-log
[`op_restore`]: #operation-log
[`Error::Spawn`]: process-model.md
[`ProcessResult`]: process-model.md
[`Change`]: #change
[`Bookmark`]: #bookmark
[`BookmarkRef`]: #bookmarkref
[`Workspace`]: #workspace
[`ChangedPath`]: #changedpath
[`DiffStat`]: #diffstat
[`FileDiff`]: #filediff
[`Operation`]: #operation
[`AnnotationLine`]: #annotationline
[`JjCapabilities`]: #jjcapabilities
[`DiffSpec`]: #diffspec-enum-non_exhaustive
[`WorkspaceAdd`]: #workspaceadd-non_exhaustive
[`SquashPaths`]: #squashpaths-non_exhaustive
[`JjFileset`]: #jjfileset
