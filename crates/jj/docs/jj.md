# vcs-jj — Jujutsu CLI guide

**What you can do:** working-copy status & the change log, describe/new change,
bookmarks, Git remote management and sync, the operation log (restore/undo),
workspaces, squash/split/absorb/duplicate/abandon, diff & template queries, parse
& resolve jj's native conflicts, and op-log-rollback transactions. This guide is the full reference —
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

Run a closure with concurrency-safe op-log rollback: capture the current operation
([`op_head`]), run `f` against a bound `JjAt`, and on `Err` roll the repo back to
that operation ([`rollback_to`]), reporting what the rollback did on the returned
`TransactionError`.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, TransactionError};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), TransactionError> {
jj.transaction(repo, |tx| async move {
    tx.describe("wip").await?;
    tx.new_change("next").await        // an Err here rolls back the describe
})
.await?;
# Ok(()) }
```

Signature:

```rust,ignore
pub async fn transaction<'a, T, F, Fut>(
    &'a self,
    dir: &'a Path,
    f: F,
) -> Result<T, TransactionError>
where
    F: FnOnce(JjAt<'a, R>) -> Fut,
    Fut: Future<Output = Result<T>> + 'a;
```

Inherent (not on the object-safe trait): the closure parameter is generic, which
`mockall`/trait objects can't express. `JjAt::transaction(f)` is the bound form.

On the closure's `Err`, the `TransactionError` preserves that error as `cause` and
carries the `rollback` outcome (`Rollback`: `Restored` / `SkippedDiverged` /
`Failed` / `NotAttempted`) — so a failed or refused rollback is visible, not
swallowed. Use `TransactionError::into_cause()` for just the old closure error.

**Caveats** (see the rustdoc for the full wording): it is **single-actor**, but no
longer silent about it — the rollback restores the *whole* repo view, so if another
jj process advances the op log between the [`op_head`] capture and the restore, the
rollback now **detects** the divergence and **refuses** to revert (returning
`Rollback::SkippedDiverged`) rather than clobbering that foreign work. Rollback runs
on `Err` only — **not** on panic or a dropped future (no async `Drop`); convert
panics to `Err` inside `f` if you need that. A **cancelled `f` no longer cancels the
rollback**: the cleanup runs on a fresh cancellation context with its own deadline,
so a fired `default_cancel_on` token does not short-circuit the restore. If the
restore itself fails, the closure's error is still returned as `cause` and the
failure is surfaced as `Rollback::Failed` (no longer discarded); the repo may be
left mid-transaction.

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
    println!("{} {}", c.status, c.path.display());  // e.g. 'M' src/lib.rs
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
async fn bookmark_forget(&self, dir: &Path, name: &str) -> Result<()>;
async fn bookmark_untrack(&self, dir: &Path, name: &str, remote: &str) -> Result<()>;
async fn bookmark_set(&self, dir: &Path, name: &str, revision: &str) -> Result<()>;
async fn bookmark_move(&self, dir: &Path, spec: BookmarkMove) -> Result<()>;
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
- `bookmark_forget` — forget a bookmark locally without marking it for deletion
  on the remote (`bookmark forget <name>`), the inverse of `bookmark_track`:
  where `bookmark_delete` propagates the deletion on the next push, `forget`
  drops the local bookmark (and its remote-tracking state) silently — a
  subsequent fetch recreates it if it still exists on the remote.
- `bookmark_untrack` — stop tracking a bookmark's remote counterpart without
  forgetting the local bookmark (`bookmark untrack <name> --remote <remote>`),
  the inverse of `bookmark_track`. Unlike `bookmark_track`'s deprecated
  composite `<name>@<remote>` positional (verified on jj 0.42: it now prints a
  deprecation warning), this uses the current, non-deprecated separate
  `--remote` flag. `remote` must be non-empty after trimming.
- `bookmark_set` — point a bookmark at `revision` (`bookmark set <name> -r`).
- `bookmark_move` — move a bookmark to a revision, built with
  `BookmarkMove::new(name, to)`; chain `.allow_backwards()` to append
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

`file_show` returns a file's content at a revision. `path` is wrapped as a
workspace-root-relative exact-path fileset (`root-file:"<path>"`) so fileset
metacharacters in the name stay literal and the path resolves from the workspace root
regardless of `dir`; content is decoded **lossily** — a binary file comes back mangled
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
async fn resolve_list(&self, dir: &Path, revset: &str) -> Result<Vec<PathBuf>>;
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
    for p in jj.resolve_list(repo, "@").await? {     // Vec<PathBuf>
        eprintln!("conflict: {}", p.display());
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

`rebase` moves the working-copy change and its branch onto a destination (`rebase
-d <onto>`, jj's default `-b @` = `(onto..@)::`): the fork-point-to-`@` line **and
its whole descendant closure** — `@`, anything stacked on `@`, and any sibling off
an *intermediate* commit of that line. This is **not** identical to git's `rebase
<onto>` (`merge-base(@,onto)..@`, which leaves stacked descendants and intermediate
siblings in place); they agree only on a linear `@`, and a sibling off the fork
point itself is moved by neither. `rebase_branch` rebases an explicitly-named
branch (`rebase -b <branch> -d <dest>`); `edit` moves the working copy to a revision
(`edit <rev>`). `edit`'s revset is guarded
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
async fn squash_into(&self, dir: &Path, spec: SquashInto) -> Result<()>;
async fn commit_paths(&self, dir: &Path, filesets: &[JjFileset], message: &str) -> Result<()>;
async fn squash_paths(&self, dir: &Path, spec: SquashPaths) -> Result<()>;
async fn split_paths(&self, dir: &Path, filesets: &[JjFileset], message: &str) -> Result<()>;
async fn absorb(&self, dir: &Path, from: Option<String>, filesets: &[JjFileset]) -> Result<()>;
```

- `squash_into` — squash the working copy into `into` (`squash --into`), built
  with `SquashInto::new(into)`. Chain `.use_destination_message()` to keep the
  destination's description (`--use-destination-message`) instead of combining the two.
- `commit_paths` — finalise a commit from exactly these [`JjFileset`]s
  (`commit -m <message> <filesets>`); the rest stay in the new working-copy
  change. Like `split_paths`, `filesets` must be **non-empty** — a fileset-less
  `commit` would commit the whole working copy, so it is refused before spawning.
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
# use vcs_jj::{Jj, JjApi, JjFileset, SquashInto, SquashPaths};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
let only = [JjFileset::path("src/parser.rs")];
jj.split_paths(repo, &only, "feat: parser").await?;
jj.commit_paths(repo, &only, "feat: parser").await?;
jj.squash_into(repo, SquashInto::new("@-")).await?;
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

## Merging & undo

```rust,ignore
async fn new_merge(&self, dir: &Path, message: &str, parents: Vec<String>) -> Result<()>;
async fn duplicate(&self, dir: &Path, revset: &str) -> Result<()>;
async fn abandon(&self, dir: &Path, revset: &str) -> Result<()>;
async fn revert(&self, dir: &Path, revset: &str) -> Result<()>;
```

`new_merge` creates a new change with the given parents (`new -m <msg> <p1>
<p2> …`); each parent is a bare positional and is guarded against a leading-`-`
value. `duplicate` duplicates the commits a revset resolves to. `abandon`
abandons a revision; its revset is guarded too.

`revert` undoes `revset` by creating a new commit that applies its reverse, as
a new child of `@` (`revert -r <revset> --onto @`) — mirroring
`GitApi::revert`'s "create an inverse commit" shape. **Verb history:** jj's
older `backout` was deprecated in jj 0.28.0 in favor of the newly-added
`revert`, and fully removed in jj 0.35.0 — both *below* this crate's validated
floor (jj ≥ 0.38) — so `revert` is the only verb across the crate's whole
supported range; no `JjCapabilities` gate is needed. The `--onto`/`-o` flag
(aliased `-d`/`--destination`) was introduced in exactly jj 0.38.0, so it too
is safe across the whole range. **Divergence from `GitApi::revert` (verified
on jj 0.42):** git's `revert --no-edit <rev>` both creates the inverse commit
*and* advances the current branch tip to it; jj's `--onto @` only creates the
new commit as a new head off `@` — it does not move `@` onto it, nor rebase
`@`'s other existing descendants. Call `edit` afterwards if the working copy
must move onto the reverted change.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
jj.new_merge(repo, "merge: a + b", vec!["feature-a".into(), "feature-b".into()]).await?;
jj.duplicate(repo, "abc123").await?;
jj.abandon(repo, "@-").await?;
jj.revert(repo, "abc123").await?;   // new commit reversing abc123, as a child of @
# Ok(()) }
```

## Git integration

```rust,ignore
async fn git_fetch(&self, dir: &Path) -> Result<()>;
async fn git_fetch_from(&self, dir: &Path, remote: &str) -> Result<()>;
async fn git_fetch_branch(&self, dir: &Path, branch: &str) -> Result<()>;
async fn git_push(&self, dir: &Path, bookmark: Option<String>) -> Result<()>;
async fn git_import(&self, dir: &Path) -> Result<()>;
async fn git_clone(&self, url: &str, dest: &Path, spec: GitClone) -> Result<()>;
```

- `git_fetch` — `jj git fetch`. Transient (network) failures are retried: 3
  attempts, 500 ms backoff (DNS, a dropped connection — see
  `is_transient_fetch_error`). A **timeout is not** retried (it already spent the full
  deadline; retrying would triple the wall-clock).
- `git_fetch_from` — fetch a named remote (`git fetch --remote <remote>`); same
  retry policy.
- `git_fetch_branch` — fetch a single bookmark from origin (`git fetch --remote
  origin -b <branch>`); same retry policy.
- `git_push` — `jj git push`, optionally `-b <bookmark>`. The bookmark is owned
  (`Option<BookmarkName>`) to keep the trait `mockall`-friendly.
- `git_import` — import git refs into jj (`jj git import`) — colocated-repo sync.
- `git_clone` — clone into `dest` (`git clone <url> <dest>
  --colocate|--no-colocate`), the colocation chosen by `GitClone::colocated()`
  or `GitClone::separate()`. Runs **without** a working directory — pass an
  **absolute** `dest`. The colocate flag is *always* passed explicitly:
  whether colocation is jj's default depends on the jj version and the user's
  `git.colocate` config, so the `GitClone` choice decides deterministically. `url` is
  guarded against a leading-`-` value.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{GitClone, Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
jj.git_fetch(repo).await?;
jj.git_push(repo, Some("main".to_string())).await?;     // `jj git push -b main`
jj.git_clone("https://example.com/r.git", Path::new("/abs/dest"), GitClone::colocated()).await?;
# Ok(()) }
```

## Remotes

```rust,ignore
async fn remote_add(&self, dir: &Path, name: &str, url: &str) -> Result<()>;
async fn remote_list(&self, dir: &Path) -> Result<Vec<Remote>>;
async fn remote_remove(&self, dir: &Path, name: &str) -> Result<()>;
async fn remote_rename(&self, dir: &Path, old: &str, new: &str) -> Result<()>;
async fn remote_set_url(&self, dir: &Path, name: &str, url: &str) -> Result<()>;
```

`remote_add`, `remote_remove`, `remote_rename`, and `remote_set_url` run their
`jj git remote` counterparts; `remote_list` returns typed [`Remote`] rows.
Remote names and URLs are bare positional arguments, so all mutating methods
reject empty or leading-`-` values before starting jj. `remote_set_url` returns
the jj non-zero exit as an error when the named remote does not exist.

The list command has no jj template or JSON output. Its `<name> <url>` display
rows are pinned by the ignored real-jj suite across the supported version matrix,
with only the first whitespace boundary separating the two fields. Non-ASCII URL
text round-trips through `remote_set_url`, but not necessarily immediately after
`remote_add`: jj percent-encodes non-ASCII bytes at add time on jj 0.40.0 and
0.42.0 on both tested platforms. jj remote configuration is UTF-8 text, so there
is no separate non-UTF-8 decoding path.

This matches `vcs-git`'s remote add/set-url capability and adds jj-native list,
remove, and rename. `vcs-jj` deliberately does not provide `vcs-git`'s singular
`remote_url` or network-backed `remote_branches`: select a row from
`remote_list` for configured URLs, and use the git wrapper when branch discovery
against a remote is required.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
jj.remote_add(repo, "upstream", "https://example.com/project.git").await?;
for remote in jj.remote_list(repo).await? {
    println!("{} -> {}", remote.name, remote.url);
}
jj.remote_rename(repo, "upstream", "mirror").await?;
jj.remote_set_url(repo, "mirror", "ssh://example.com/project.git").await?;
jj.remote_remove(repo, "mirror").await?;
# Ok(()) }
```

## Config

```rust,ignore
async fn config_get(&self, dir: &Path, key: &str) -> Result<Option<String>>;
async fn config_set(&self, dir: &Path, key: &str, value: &str) -> Result<()>;
```

Parity with `GitApi::config_get`/`config_set`. `config_get` runs `config get
<key>` and maps jj's exit codes: `0` → `Some(value)` (only jj's trailing line
terminator is stripped, not all trailing whitespace), `1` → `None` (unset —
verified on jj 0.42: "Config error: Value not found"), anything else is a real
error. `config_set` runs `config set --repo -- <key> <value>`: the `--`
terminator pins both as bare positionals, so a flag-shaped `value` (or `key`)
is taken literally rather than rejected as an unrecognised argument. `key` is
guarded like a bare positional (empty/leading `-` refused before spawning);
`value` deliberately keeps no such guard — a config value may legitimately
begin with `-` — matching `GitApi::config_set`. `value` is jj's own
TOML-expression grammar: an unquoted value that isn't valid TOML syntax is
stored as that literal string, but a value that *is* valid TOML (`"42"`,
`"true"`) is stored as that TOML type rather than a string — a real divergence
from git's always-literal-string `config_set` (a plain round-trip through
`config_get` is unaffected either way, since both render back to the same
text). Like `GitApi::config_set`, this is a **trusted-input sink**: the `--`
guard only stops `value` being misparsed as a flag, never wire untrusted input
into a key jj interprets as a command to run.

```rust,ignore
# use std::path::Path;
# use vcs_jj::{Jj, JjApi};
# async fn demo(jj: &Jj, repo: &Path) -> Result<(), processkit::Error> {
jj.config_set(repo, "user.name", "Ada Lovelace").await?;
let name = jj.config_get(repo, "user.name").await?;  // Option<String>
# let _ = name; Ok(()) }
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
> reserve it for short-lived cleanup. `workspace_name_for_path` returns
> `io::Result<Option<String>>`: `Ok(Some(name))` on a match, `Ok(None)` for a
> clean "no such workspace" (skip the cleanup), and `Err` when the probe itself
> could not answer (`jj` missing, `workspace list` failed, or a registered
> workspace did not resolve) — so a real failure is no longer folded into a silent
> `None`.

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

**cwd (T-035).** On the **client** (`jj.run(…)`) these run in the **process's
current directory**. On the **bound view** (`jj.at(dir).run(…)`) they are instead
bound to `dir`: the view forwards to the client's dir-taking `run_in`/`run_raw_in`/
`run_args_in`/`run_raw_args_in`, so a raw call through the handle runs in the bound
repo, like every other `JjAt` method. The bound raw hatch stays verbatim — unlike
the modelled methods it does **not** inject `--color never`. Reach for the client's
`run` when you deliberately want the process cwd.

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
`git diff` are byte-identical for ASCII paths (they differ only in how a non-ASCII
filename is rendered, which the shared parser decodes), so `vcs-jj` and `vcs-git`
share one parser. They're
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
| `target` | `String` | **Full** commit id it points at (empty for a conflicted bookmark) — a cross-referenceable id, not a display prefix. |

### `BookmarkRef`
From `bookmark list -a` — local *or* remote-tracking.

| Field | Type | Notes |
| --- | --- | --- |
| `name` | `String` | Bookmark name. |
| `remote` | `Option<String>` | Remote (e.g. `origin`/`git`); `None` for a local. |
| `target` | `String` | **Full** commit id (empty for a conflicted bookmark) — a cross-referenceable id, not a display prefix. |
| `tracked` | `bool` | Whether this remote-tracking bookmark is tracked (`false` for locals). |

### `Remote`
From `git remote list`.

| Field | Type | Notes |
| --- | --- | --- |
| `name` | `String` | Configured remote name. |
| `url` | `String` | Configured fetch/push URL. |

### `Workspace`
| Field | Type | Notes |
| --- | --- | --- |
| `name` | `String` | Workspace name (`default` for the main one). |
| `commit` | `String` | **Full** commit id of the working-copy commit (the identity `WorktreeInfo.commit` carries), not a display prefix. |
| `bookmarks` | `Vec<String>` | Local bookmarks at that commit (empty when none). |

### `ChangedPath`
One `jj diff --summary` entry.

| Field | Type | Notes |
| --- | --- | --- |
| `status` | `char` | `M` modified, `A` added, `D` deleted, `R` renamed, `C` copied. |
| `path` | `PathBuf` | The path the status applies to — the *new* path for a rename/copy (forward-slash normalised); lossless (non-UTF-8-safe on Unix). |
| `old_path` | `Option<PathBuf>` | For `R`/`C`, the original path; `None` otherwise. |

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
| `time` | `String` | Start timestamp, RFC 3339 (colon offset, e.g. `…+02:00`). |
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

### `DiffSpec` (enum)
What `diff` / `diff_text` compares — a re-export of `vcs_diff::DiffSpec`,
deliberately exhaustive (not `#[non_exhaustive]`).
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

### `RevsetExpr` and `BookmarkName`
Every operation that resolves a **revset** takes a `RevsetExpr`, and every
operation that names a **bookmark** (jj's equivalent of a branch) to
create/move/rename/delete/track/fetch/push takes a `BookmarkName` — not a bare
`&str`. Construct one at your input boundary; a flag-like or malformed value is
rejected there (a classifiable `Error::is_invalid_input`) and can never reach an
argv slot. Both are deliberately *minimal* — jj's revset grammar is too rich to
validate, and jj bookmark names are permissive — so the load-bearing guarantee is
non-empty and not flag-shaped (no leading `-`). The typed bookmark methods
additionally wrap the name in jj's `exact:` pattern so a `*`/`?` can never fan the
operation out across every bookmark.

```rust,ignore
# use vcs_jj::{BookmarkName, RevsetExpr};
let r = RevsetExpr::new("main..@")?;        // Ok
let b = BookmarkName::new("feature/x")?;    // Ok
assert!(RevsetExpr::new("").is_err());      // empty
assert!(RevsetExpr::new("-x").is_err());    // leading `-` → would parse as a flag
assert!(BookmarkName::new("--all").is_err());
# let _ = (r, b);
# Ok::<(), processkit::Error>(())
```

`RevsetExpr::new` / `BookmarkName::new(impl Into<String>) -> Result<Self>`;
`.as_str() -> &str`; both implement `Display`. The remaining bare-positional
`&str` inputs that are *not* bookmarks/revsets (remote names, operation ids,
workspace names) keep an internal flag-injection guard.

### `JjFileset`
An exact-path jj fileset (`root-file:"<path>"`), so path metacharacters like `(`,
`)`, `|`, `*` are treated literally rather than as fileset operators. Build it
with `JjFileset::path(path)` (workspace-root-relative — jj's `root-file:` anchor, so
the path resolves from the workspace root even when the command runs from a
subdirectory); on **Windows** a `\` separator is normalised to `/` (on Unix `\` is a
legitimate filename byte, preserved), and a `"` is escaped for the string literal.

```rust,ignore
# use vcs_jj::JjFileset;
let fs = JjFileset::path(r#"src/a (copy).rs"#);
assert_eq!(fs.as_str(), r#"root-file:"src/a (copy).rs""#);
```

`JjFileset::path(impl AsRef<str>) -> Self`; `.as_str() -> &str`.

### Why injection guards, and why filesets

Bookmark names and revsets are taken as the validated `BookmarkName` /
`RevsetExpr` newtypes, so an empty or leading-`-` value is refused at
construction — before it can reach any argv slot (verified: `jj edit -evil` →
"unexpected argument"). The remaining caller-supplied bare positionals that are
*not* bookmarks/revsets (remote names, operation ids, workspace names, urls) keep
an internal guard that refuses an empty or leading-`-` value with an
`Error::Spawn` **before** spawning. The `run`/`run_raw` escape hatches are *not*
guarded — you build the whole argv.

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
[`rollback_to`]: #operation-log
[`Error::Spawn`]: https://docs.rs/vcs-core/latest/vcs_core/guide/process_model/
[`ProcessResult`]: https://docs.rs/vcs-core/latest/vcs_core/guide/process_model/
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
[`DiffSpec`]: #diffspec-enum
[`WorkspaceAdd`]: #workspaceadd-non_exhaustive
[`SquashPaths`]: #squashpaths-non_exhaustive
[`JjFileset`]: #jjfileset
