//! `vcs-jj` — automate Jujutsu (`jj`) from Rust through CLI process execution.
//!
//! Async, mockable, and structured-error: consumers depend on the [`JjApi`]
//! trait and substitute a mock for the real [`Jj`] client in tests. Commands run
//! inside an OS job (via [`processkit`]) so a `jj` subprocess is never orphaned,
//! and honour an optional [timeout](Jj::default_timeout).
//!
//! Two test seams: enable the `mock` feature for a `mockall`-generated
//! `MockJjApi`, or inject a fake runner with
//! `Jj::with_runner(`[`ScriptedRunner`](processkit::ScriptedRunner)`)`.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use processkit::ProcessRunner;
// Re-export the processkit types in this crate's public API (also brings
// `Error`/`Result`/`ProcessResult` into scope here).
pub use processkit::{Error, ProcessResult, Result};

mod parse;
pub use parse::{
    Bookmark, BookmarkRef, Change, ChangeKind, ChangedPath, DiffLine, DiffStat, FileDiff, Hunk,
    Workspace,
};

/// Name of the underlying CLI binary this crate drives.
pub const BINARY: &str = "jj";

/// What a [`JjApi::diff`] / [`JjApi::diff_text`] call compares.
///
/// `#[non_exhaustive]` so more comparison shapes can be added later.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum DiffSpec {
    /// The working-copy change's diff (`jj diff -r @`).
    WorkingTree,
    /// A specific revset, e.g. `@-` or `main..@` (`jj diff -r <revset>`).
    Rev(String),
}

/// How a new workspace inherits sparse patterns (`jj workspace add
/// --sparse-patterns <mode>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SparseMode {
    /// Copy all sparse patterns from the current workspace (jj's default).
    Copy,
    /// Include every file in the new workspace.
    Full,
    /// Start with no files — the caller sets patterns afterwards (CoW flow).
    Empty,
}

impl SparseMode {
    /// The `--sparse-patterns` value jj expects.
    fn as_arg(self) -> &'static str {
        match self {
            SparseMode::Copy => "copy",
            SparseMode::Full => "full",
            SparseMode::Empty => "empty",
        }
    }
}

/// An exact-path jj fileset (`file:"<path>"`), so path metacharacters like `(`,
/// `)`, `|`, `*` are treated literally rather than as fileset operators.
///
/// Build it with [`JjFileset::path`]; the path is repo-root-relative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JjFileset(String);

impl JjFileset {
    /// Wrap a repo-relative `path` as an exact-path fileset, escaping `\` and `"`.
    pub fn path(path: impl AsRef<str>) -> Self {
        let escaped = path.as_ref().replace('\\', "\\\\").replace('"', "\\\"");
        JjFileset(format!("file:\"{escaped}\""))
    }

    /// The rendered `file:"…"` expression.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Options for [`JjApi::workspace_add`] (`jj workspace add`).
///
/// `#[non_exhaustive]`, so build it through [`WorkspaceAdd::new`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WorkspaceAdd {
    /// Name for the new workspace.
    pub name: String,
    /// Revision the workspace's working copy starts at (`-r <base>`).
    pub base: String,
    /// Filesystem path for the new workspace.
    pub path: PathBuf,
    /// How to seed the new workspace's sparse patterns (`--sparse-patterns`);
    /// `None` leaves jj's default (inherit from the current workspace).
    pub sparse_patterns: Option<SparseMode>,
}

impl WorkspaceAdd {
    /// A workspace named `name`, based at `base`, materialised at `path`.
    pub fn new(name: impl Into<String>, base: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            base: base.into(),
            path: path.into(),
            sparse_patterns: None,
        }
    }

    /// Seed the new workspace's sparse patterns with `mode` (`--sparse-patterns`).
    pub fn sparse(mut self, mode: SparseMode) -> Self {
        self.sparse_patterns = Some(mode);
        self
    }
}

/// The first bookmark name from a comma-joined [`BOOKMARKS_TEMPLATE`](parse::BOOKMARKS_TEMPLATE)
/// render; `None` when the commit carries no local bookmark.
fn first_bookmark(rendered: &str) -> Option<String> {
    let rendered = rendered.trim();
    (!rendered.is_empty()).then(|| rendered.split(',').next().unwrap_or(rendered).to_string())
}

/// The jj operations this crate exposes — the interface consumers code against
/// and mock in tests.
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait::async_trait]
pub trait JjApi: Send + Sync {
    /// Run `jj <args>`, returning trimmed stdout (throws on a non-zero exit).
    async fn run(&self, args: &[String]) -> Result<String>;
    /// Like [`JjApi::run`] but never errors on a non-zero exit — returns the
    /// captured [`ProcessResult`].
    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>>;
    /// Installed Jujutsu version (`jj --version`).
    async fn version(&self) -> Result<String>;
    /// Parsed working-copy changes — the files changed in `@`
    /// (`jj diff -r @ --summary`), mirroring `vcs_git` `status`.
    async fn status(&self, dir: &Path) -> Result<Vec<ChangedPath>>;
    /// Raw `jj status` text (human-readable) — the unparsed counterpart of
    /// [`status`](JjApi::status), mirroring `vcs_git` `status_text`.
    async fn status_text(&self, dir: &Path) -> Result<String>;
    /// Changes matching `revset`, newest first, up to `max` (`jj log`).
    async fn log(&self, dir: &Path, revset: &str, max: usize) -> Result<Vec<Change>>;
    /// The working-copy change (`jj log -r @`).
    async fn current_change(&self, dir: &Path) -> Result<Change>;
    /// Set the working-copy change's description (`jj describe -m`).
    async fn describe(&self, dir: &Path, message: &str) -> Result<()>;
    /// Set the description of an arbitrary revision (`jj describe -r <revset> -m`).
    async fn describe_rev(&self, dir: &Path, revset: &str, message: &str) -> Result<()>;
    /// Start a new change on top of the working copy (`jj new -m`).
    async fn new_change(&self, dir: &Path, message: &str) -> Result<()>;
    /// Local bookmarks (`jj bookmark list`).
    async fn bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>>;
    /// Local *and* remote-tracking bookmarks (`jj bookmark list -a`).
    async fn bookmarks_all(&self, dir: &Path) -> Result<Vec<BookmarkRef>>;
    /// Local bookmarks on the nearest commits reachable from `@`
    /// (`log -r 'heads(::@ & bookmarks())'`) — the candidate targets a commit
    /// "belongs to". A commit carrying several bookmarks yields one entry each.
    async fn reachable_bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>>;
    /// Track a remote bookmark (`jj bookmark track <name>@<remote>`).
    async fn bookmark_track(&self, dir: &Path, name: &str, remote: &str) -> Result<()>;
    /// Point a bookmark at `revision` (`jj bookmark set <name> -r <revision>`).
    async fn bookmark_set(&self, dir: &Path, name: &str, revision: &str) -> Result<()>;
    /// Fetch from the git remote (`jj git fetch`); transient (network) failures
    /// are retried (3 attempts, 500 ms backoff).
    async fn git_fetch(&self, dir: &Path) -> Result<()>;
    /// Fetch from a *named* git remote (`jj git fetch --remote <remote>`);
    /// transient failures are retried like [`git_fetch`](JjApi::git_fetch).
    async fn git_fetch_from(&self, dir: &Path, remote: &str) -> Result<()>;
    /// Push to the git remote (`jj git push`, optionally `-b <bookmark>`). The
    /// bookmark is owned (`Option<String>`) to keep the trait `mockall`-friendly.
    async fn git_push(&self, dir: &Path, bookmark: Option<String>) -> Result<()>;

    // --- Discovery / identity ------------------------------------------------

    /// Working-copy root of the current workspace (`jj root`).
    async fn root(&self, dir: &Path) -> Result<PathBuf>;
    /// The local bookmark on the working-copy change `@`, if exactly one (or the
    /// first of several); `None` when `@` carries no bookmark. `ws` enforces the
    /// one-bookmark policy on top.
    async fn current_bookmark(&self, dir: &Path) -> Result<Option<String>>;
    /// The trunk bookmark (`jj log -r 'trunk()'`); `None` when unresolved.
    async fn trunk(&self, dir: &Path) -> Result<Option<String>>;

    // --- Bookmarks -----------------------------------------------------------

    /// Create a bookmark at a revision (`bookmark create <name> -r <rev>`).
    async fn bookmark_create(&self, dir: &Path, name: &str, revision: &str) -> Result<()>;
    /// Rename a bookmark (`bookmark rename <old> <new>`).
    async fn bookmark_rename(&self, dir: &Path, old: &str, new: &str) -> Result<()>;
    /// Delete a bookmark (`bookmark delete <name>`).
    async fn bookmark_delete(&self, dir: &Path, name: &str) -> Result<()>;
    /// Move a bookmark to a revision (`bookmark move <name> --to <rev>
    /// [--allow-backwards]`).
    async fn bookmark_move(
        &self,
        dir: &Path,
        name: &str,
        to: &str,
        allow_backwards: bool,
    ) -> Result<()>;

    // --- Diff / query / state ------------------------------------------------

    /// Per-file change summary for a range (`diff -r <from>..<to> --summary`).
    async fn diff_summary(&self, dir: &Path, from: &str, to: &str) -> Result<Vec<ChangedPath>>;
    /// Aggregate change stats for a revset (`diff -r <revset> --stat`).
    async fn diff_stat(&self, dir: &Path, revset: &str) -> Result<DiffStat>;
    /// Raw git-format unified diff text for `spec` (`diff -r <spec> --git`) —
    /// stable machine output.
    async fn diff_text(&self, dir: &Path, spec: DiffSpec) -> Result<String>;
    /// Parsed per-file unified diff for `spec`, layered on [`diff_text`](JjApi::diff_text).
    async fn diff(&self, dir: &Path, spec: DiffSpec) -> Result<Vec<FileDiff>>;
    /// Count commits in a revset (`log -r <revset> --no-graph`, one id per line).
    async fn commit_count(&self, dir: &Path, revset: &str) -> Result<usize>;
    /// Whether the commit a revset resolves to has a conflict.
    async fn is_conflicted(&self, dir: &Path, revset: &str) -> Result<bool>;
    /// Whether the working copy has unresolved conflicts (`jj status`).
    async fn has_workingcopy_conflict(&self, dir: &Path) -> Result<bool>;
    /// Paths with unresolved conflicts in `revset` (`jj resolve --list -r <revset>`).
    /// Empty when there are none.
    async fn resolve_list(&self, dir: &Path, revset: &str) -> Result<Vec<String>>;
    /// Run an arbitrary templated `jj log` query and return raw stdout
    /// (`log -r <revset> --no-graph [--limit n] -T <template>`).
    async fn template_query(
        &self,
        dir: &Path,
        revset: &str,
        template: &str,
        limit: Option<usize>,
    ) -> Result<String>;
    /// The full (possibly multiline) description of the commit `revset` resolves
    /// to, trailing whitespace trimmed; empty for an undescribed change — or for
    /// a revset matching no commit (an *invalid* revset still errors). A
    /// multi-commit revset yields only the newest commit's description
    /// (`jj log` order, `--limit 1`).
    async fn description(&self, dir: &Path, revset: &str) -> Result<String>;

    // --- Mutations -----------------------------------------------------------

    /// Rebase the working copy onto a destination (`rebase -d <onto>`).
    async fn rebase(&self, dir: &Path, onto: &str) -> Result<()>;
    /// Rebase a whole branch onto a destination (`rebase -b <branch> -d <dest>`).
    async fn rebase_branch(&self, dir: &Path, branch: &str, dest: &str) -> Result<()>;
    /// Move the working copy to a revision (`edit <rev>`).
    async fn edit(&self, dir: &Path, revset: &str) -> Result<()>;
    /// Squash the working copy into a revision (`squash --into <rev>`). When
    /// `use_destination_message`, keep the destination's description
    /// (`--use-destination-message`) instead of combining the two.
    async fn squash_into(
        &self,
        dir: &Path,
        into: &str,
        use_destination_message: bool,
    ) -> Result<()>;
    /// Finalise a commit from exactly these filesets (`commit -m <message>
    /// <filesets>`); the rest stay in the new working-copy change.
    async fn commit_paths(&self, dir: &Path, filesets: &[JjFileset], message: &str) -> Result<()>;
    /// Squash exactly these filesets from one revision into another
    /// (`squash --from <from> --into <into> [--use-destination-message] <filesets>`).
    async fn squash_paths(
        &self,
        dir: &Path,
        from: &str,
        into: &str,
        filesets: &[JjFileset],
        use_destination_message: bool,
    ) -> Result<()>;
    /// Set the working copy's sparse patterns to exactly `patterns`
    /// (`sparse set --clear --add <p>…`); an empty list clears the working copy.
    async fn sparse_set(&self, dir: &Path, patterns: &[String]) -> Result<()>;
    /// Create a new change with the given parents (`new -m <msg> <p1> <p2> …`).
    async fn new_merge(&self, dir: &Path, message: &str, parents: Vec<String>) -> Result<()>;
    /// Abandon a revision (`abandon <rev>`).
    async fn abandon(&self, dir: &Path, revset: &str) -> Result<()>;
    /// Fetch a single bookmark from origin (`git fetch --remote origin -b <branch>`);
    /// transient failures are retried (3×, 500 ms).
    async fn git_fetch_branch(&self, dir: &Path, branch: &str) -> Result<()>;
    /// Import git refs into jj (`jj git import`) — colocated-repo sync.
    async fn git_import(&self, dir: &Path) -> Result<()>;

    // --- Operation log -------------------------------------------------------

    /// The current operation id (`op log --no-graph --limit 1`) — capture before
    /// a risky sequence to roll back to.
    async fn op_head(&self, dir: &Path) -> Result<String>;
    /// Restore the repo to an operation (`op restore <id>`).
    async fn op_restore(&self, dir: &Path, op_id: &str) -> Result<()>;
    /// Undo the latest operation (`op undo`).
    async fn op_undo(&self, dir: &Path) -> Result<()>;

    // --- Workspaces ----------------------------------------------------------

    /// List workspaces (`workspace list`).
    async fn workspace_list(&self, dir: &Path) -> Result<Vec<Workspace>>;
    /// Resolve a workspace's root path (`workspace root [--name <name>]`).
    async fn workspace_root(&self, dir: &Path, name: Option<String>) -> Result<PathBuf>;
    /// Add a workspace (`workspace add --name <name> -r <base> <path>`).
    async fn workspace_add(&self, dir: &Path, spec: WorkspaceAdd) -> Result<()>;
    /// Forget a workspace (`workspace forget <name>`).
    async fn workspace_forget(&self, dir: &Path, name: &str) -> Result<()>;
}

processkit::cli_client!(
    /// The real jj client. Generic over the [`ProcessRunner`] so tests can inject
    /// a fake process executor; `Jj::new()` uses the real job-backed runner.
    pub struct Jj => BINARY
);

impl<R: ProcessRunner> Jj<R> {
    /// A repo-scoped `jj` command with `--color never` forced on. jj honours
    /// `ui.color = "always"` from user config even when its output is piped, which
    /// would wrap our templated output — and the command error text we classify —
    /// in ANSI escapes and break parsing; `--color never` is the only thing that
    /// overrides that config (`NO_COLOR`/`CLICOLOR` do not). It is a global flag,
    /// appended here (no jj subcommand takes a trailing `--`, so this is safe).
    fn cmd_in<I, S>(&self, dir: &Path, args: I) -> processkit::Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.core.command_in(dir, args).arg("--color").arg("never")
    }
}

#[async_trait::async_trait]
impl<R: ProcessRunner> JjApi for Jj<R> {
    async fn run(&self, args: &[String]) -> Result<String> {
        self.core.text(self.core.command(args)).await
    }

    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>> {
        self.core.capture(self.core.command(args)).await
    }

    async fn version(&self) -> Result<String> {
        self.core.text(self.core.command(["--version"])).await
    }

    async fn status(&self, dir: &Path) -> Result<Vec<ChangedPath>> {
        // `diff -r @ --summary` is the machine-stable form of the working-copy
        // changes that `jj status` renders for humans: one `<letter> <path>` line.
        self.core
            .parse(
                self.cmd_in(dir, ["diff", "-r", "@", "--summary"]),
                parse::parse_diff_summary,
            )
            .await
    }

    async fn status_text(&self, dir: &Path) -> Result<String> {
        self.core.text(self.cmd_in(dir, ["status"])).await
    }

    async fn log(&self, dir: &Path, revset: &str, max: usize) -> Result<Vec<Change>> {
        let n = format!("-n{max}");
        self.core
            .parse(
                self.cmd_in(
                    dir,
                    [
                        "log",
                        "-r",
                        revset,
                        n.as_str(),
                        "--no-graph",
                        "-T",
                        parse::CHANGE_TEMPLATE,
                    ],
                ),
                parse::parse_changes,
            )
            .await
    }

    async fn current_change(&self, dir: &Path) -> Result<Change> {
        let mut changes = self.log(dir, "@", 1).await?;
        changes.pop().ok_or_else(|| Error::Parse {
            program: BINARY.to_string(),
            message: "no working-copy change found".to_string(),
        })
    }

    async fn describe(&self, dir: &Path, message: &str) -> Result<()> {
        self.core
            .unit(self.cmd_in(dir, ["describe", "-m", message]))
            .await
    }

    async fn describe_rev(&self, dir: &Path, revset: &str, message: &str) -> Result<()> {
        self.core
            .unit(self.cmd_in(dir, ["describe", "-r", revset, "-m", message]))
            .await
    }

    async fn new_change(&self, dir: &Path, message: &str) -> Result<()> {
        self.core
            .unit(self.cmd_in(dir, ["new", "-m", message]))
            .await
    }

    async fn bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>> {
        self.core
            .parse(
                self.cmd_in(dir, ["bookmark", "list"]),
                parse::parse_bookmarks,
            )
            .await
    }

    async fn bookmarks_all(&self, dir: &Path) -> Result<Vec<BookmarkRef>> {
        self.core
            .parse(
                self.cmd_in(
                    dir,
                    ["bookmark", "list", "-a", "-T", parse::BOOKMARK_ALL_TEMPLATE],
                ),
                parse::parse_bookmarks_all,
            )
            .await
    }

    async fn reachable_bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>> {
        self.core
            .parse(
                self.cmd_in(
                    dir,
                    [
                        "log",
                        "-r",
                        "heads(::@ & bookmarks())",
                        "--no-graph",
                        "-T",
                        parse::REACHABLE_BOOKMARKS_TEMPLATE,
                    ],
                ),
                parse::parse_reachable_bookmarks,
            )
            .await
    }

    async fn bookmark_track(&self, dir: &Path, name: &str, remote: &str) -> Result<()> {
        let target = format!("{name}@{remote}");
        self.core
            .unit(self.cmd_in(dir, ["bookmark", "track", target.as_str()]))
            .await
    }

    async fn bookmark_set(&self, dir: &Path, name: &str, revision: &str) -> Result<()> {
        self.core
            .unit(self.cmd_in(dir, ["bookmark", "set", name, "-r", revision]))
            .await
    }

    async fn git_fetch(&self, dir: &Path) -> Result<()> {
        // Idempotent → `retry` replays it on a transient (network) failure.
        let cmd = self.cmd_in(dir, ["git", "fetch"]).retry(
            FETCH_ATTEMPTS,
            FETCH_BACKOFF,
            is_transient_fetch_error,
        );
        self.core.unit(cmd).await
    }

    async fn git_fetch_from(&self, dir: &Path, remote: &str) -> Result<()> {
        // Idempotent → `retry` replays it on a transient (network) failure.
        let cmd = self
            .cmd_in(dir, ["git", "fetch", "--remote", remote])
            .retry(FETCH_ATTEMPTS, FETCH_BACKOFF, is_transient_fetch_error);
        self.core.unit(cmd).await
    }

    async fn git_push(&self, dir: &Path, bookmark: Option<String>) -> Result<()> {
        let mut args = vec!["git", "push"];
        if let Some(name) = bookmark.as_deref() {
            args.push("-b");
            args.push(name);
        }
        self.core.unit(self.cmd_in(dir, args)).await
    }

    async fn root(&self, dir: &Path) -> Result<PathBuf> {
        Ok(PathBuf::from(
            self.core.text(self.cmd_in(dir, ["root"])).await?,
        ))
    }

    async fn current_bookmark(&self, dir: &Path) -> Result<Option<String>> {
        let out = self
            .core
            .text(self.cmd_in(
                dir,
                [
                    "log",
                    "-r",
                    "@",
                    "--no-graph",
                    "--limit",
                    "1",
                    "-T",
                    parse::BOOKMARKS_TEMPLATE,
                ],
            ))
            .await?;
        Ok(first_bookmark(&out))
    }

    async fn trunk(&self, dir: &Path) -> Result<Option<String>> {
        let out = self
            .core
            .text(self.cmd_in(
                dir,
                [
                    "log",
                    "-r",
                    "trunk()",
                    "--no-graph",
                    "--limit",
                    "1",
                    "-T",
                    parse::BOOKMARKS_TEMPLATE,
                ],
            ))
            .await?;
        Ok(first_bookmark(&out))
    }

    async fn bookmark_create(&self, dir: &Path, name: &str, revision: &str) -> Result<()> {
        self.core
            .unit(self.cmd_in(dir, ["bookmark", "create", name, "-r", revision]))
            .await
    }

    async fn bookmark_rename(&self, dir: &Path, old: &str, new: &str) -> Result<()> {
        self.core
            .unit(self.cmd_in(dir, ["bookmark", "rename", old, new]))
            .await
    }

    async fn bookmark_delete(&self, dir: &Path, name: &str) -> Result<()> {
        self.core
            .unit(self.cmd_in(dir, ["bookmark", "delete", name]))
            .await
    }

    async fn bookmark_move(
        &self,
        dir: &Path,
        name: &str,
        to: &str,
        allow_backwards: bool,
    ) -> Result<()> {
        let mut args = vec!["bookmark", "move", name, "--to", to];
        if allow_backwards {
            args.push("--allow-backwards");
        }
        self.core.unit(self.cmd_in(dir, args)).await
    }

    async fn diff_summary(&self, dir: &Path, from: &str, to: &str) -> Result<Vec<ChangedPath>> {
        // Parenthesise each endpoint so a compound revset (e.g. `x | y`) keeps its
        // meaning inside the `..` range instead of binding by operator precedence.
        let range = format!("({from})..({to})");
        self.core
            .parse(
                self.cmd_in(dir, ["diff", "-r", range.as_str(), "--summary"]),
                parse::parse_diff_summary,
            )
            .await
    }

    async fn diff_stat(&self, dir: &Path, revset: &str) -> Result<DiffStat> {
        self.core
            .parse(
                self.cmd_in(dir, ["diff", "-r", revset, "--stat"]),
                parse::parse_diff_stat,
            )
            .await
    }

    async fn diff_text(&self, dir: &Path, spec: DiffSpec) -> Result<String> {
        // `@` selects the working-copy change; otherwise the caller's revset.
        // `--git` emits stable git-format output the shared parser understands.
        let revset = match spec {
            DiffSpec::WorkingTree => "@".to_string(),
            DiffSpec::Rev(rev) => rev,
        };
        self.core
            .text(self.cmd_in(dir, ["diff", "-r", revset.as_str(), "--git"]))
            .await
    }

    async fn diff(&self, dir: &Path, spec: DiffSpec) -> Result<Vec<FileDiff>> {
        let text = self.diff_text(dir, spec).await?;
        Ok(parse::parse_diff(&text))
    }

    async fn commit_count(&self, dir: &Path, revset: &str) -> Result<usize> {
        self.core
            .parse(
                self.cmd_in(
                    dir,
                    [
                        "log",
                        "-r",
                        revset,
                        "--no-graph",
                        "-T",
                        parse::COUNT_TEMPLATE,
                    ],
                ),
                |s| s.lines().filter(|line| !line.is_empty()).count(),
            )
            .await
    }

    async fn is_conflicted(&self, dir: &Path, revset: &str) -> Result<bool> {
        let out = self
            .core
            .text(self.cmd_in(
                dir,
                [
                    "log",
                    "-r",
                    revset,
                    "--no-graph",
                    "--limit",
                    "1",
                    "-T",
                    parse::CONFLICT_TEMPLATE,
                ],
            ))
            .await?;
        Ok(out.trim() == "1")
    }

    async fn has_workingcopy_conflict(&self, dir: &Path) -> Result<bool> {
        // Ask the template engine directly rather than string-matching localized
        // `jj status` prose: `@` is conflicted iff its `conflict` flag is set.
        self.is_conflicted(dir, "@").await
    }

    async fn resolve_list(&self, dir: &Path, revset: &str) -> Result<Vec<String>> {
        let res = self
            .core
            .capture(self.cmd_in(dir, ["resolve", "--list", "-r", revset]))
            .await?;
        match res.code() {
            Some(0) => Ok(parse::parse_resolve_list(res.stdout())),
            // jj exits non-zero with "No conflicts found …" when the revision is
            // conflict-free — the one non-zero we read as an empty list. Any other
            // failure (bad revset, not a repo, …) must surface, not masquerade as
            // "no conflicts".
            _ if res.stderr().contains("No conflicts") => Ok(Vec::new()),
            _ => {
                res.ensure_success()?;
                Ok(Vec::new()) // unreachable: a non-zero exit always errors above.
            }
        }
    }

    async fn template_query(
        &self,
        dir: &Path,
        revset: &str,
        template: &str,
        limit: Option<usize>,
    ) -> Result<String> {
        let mut args: Vec<String> = vec![
            "log".into(),
            "-r".into(),
            revset.into(),
            "--no-graph".into(),
        ];
        if let Some(n) = limit {
            args.push("--limit".into());
            args.push(n.to_string());
        }
        args.push("-T".into());
        args.push(template.into());
        self.core.text(self.cmd_in(dir, args)).await
    }

    async fn description(&self, dir: &Path, revset: &str) -> Result<String> {
        self.template_query(dir, revset, "description", Some(1))
            .await
    }

    async fn rebase(&self, dir: &Path, onto: &str) -> Result<()> {
        self.core
            .unit(self.cmd_in(dir, ["rebase", "-d", onto]))
            .await
    }

    async fn rebase_branch(&self, dir: &Path, branch: &str, dest: &str) -> Result<()> {
        self.core
            .unit(self.cmd_in(dir, ["rebase", "-b", branch, "-d", dest]))
            .await
    }

    async fn edit(&self, dir: &Path, revset: &str) -> Result<()> {
        self.core.unit(self.cmd_in(dir, ["edit", revset])).await
    }

    async fn squash_into(
        &self,
        dir: &Path,
        into: &str,
        use_destination_message: bool,
    ) -> Result<()> {
        let mut command = self.cmd_in(dir, ["squash", "--into", into]);
        if use_destination_message {
            command = command.arg("--use-destination-message");
        }
        self.core.unit(command).await
    }

    async fn commit_paths(&self, dir: &Path, filesets: &[JjFileset], message: &str) -> Result<()> {
        let mut args: Vec<String> = vec!["commit".into(), "-m".into(), message.into()];
        args.extend(filesets.iter().map(|f| f.as_str().to_string()));
        self.core.unit(self.cmd_in(dir, args)).await
    }

    async fn squash_paths(
        &self,
        dir: &Path,
        from: &str,
        into: &str,
        filesets: &[JjFileset],
        use_destination_message: bool,
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "squash".into(),
            "--from".into(),
            from.into(),
            "--into".into(),
            into.into(),
        ];
        if use_destination_message {
            args.push("--use-destination-message".into());
        }
        args.extend(filesets.iter().map(|f| f.as_str().to_string()));
        self.core.unit(self.cmd_in(dir, args)).await
    }

    async fn sparse_set(&self, dir: &Path, patterns: &[String]) -> Result<()> {
        // `--clear` empties the working copy first, then each `--add` reinstates a
        // pattern — so the working copy ends up holding exactly `patterns`.
        let mut args: Vec<String> = vec!["sparse".into(), "set".into(), "--clear".into()];
        for pattern in patterns {
            args.push("--add".into());
            args.push(pattern.clone());
        }
        self.core.unit(self.cmd_in(dir, args)).await
    }

    async fn new_merge(&self, dir: &Path, message: &str, parents: Vec<String>) -> Result<()> {
        let mut args: Vec<String> = vec!["new".into(), "-m".into(), message.into()];
        args.extend(parents);
        self.core.unit(self.cmd_in(dir, args)).await
    }

    async fn abandon(&self, dir: &Path, revset: &str) -> Result<()> {
        self.core.unit(self.cmd_in(dir, ["abandon", revset])).await
    }

    async fn git_fetch_branch(&self, dir: &Path, branch: &str) -> Result<()> {
        let cmd = self
            .cmd_in(dir, ["git", "fetch", "--remote", "origin", "-b", branch])
            .retry(FETCH_ATTEMPTS, FETCH_BACKOFF, is_transient_fetch_error);
        self.core.unit(cmd).await
    }

    async fn git_import(&self, dir: &Path) -> Result<()> {
        self.core.unit(self.cmd_in(dir, ["git", "import"])).await
    }

    async fn op_head(&self, dir: &Path) -> Result<String> {
        self.core
            .text(self.cmd_in(
                dir,
                [
                    "op",
                    "log",
                    "--no-graph",
                    "--limit",
                    "1",
                    "-T",
                    "id.short()",
                ],
            ))
            .await
    }

    async fn op_restore(&self, dir: &Path, op_id: &str) -> Result<()> {
        self.core
            .unit(self.cmd_in(dir, ["op", "restore", op_id]))
            .await
    }

    async fn op_undo(&self, dir: &Path) -> Result<()> {
        self.core.unit(self.cmd_in(dir, ["op", "undo"])).await
    }

    async fn workspace_list(&self, dir: &Path) -> Result<Vec<Workspace>> {
        self.core
            .parse(
                self.cmd_in(dir, ["workspace", "list", "-T", parse::WORKSPACE_TEMPLATE]),
                parse::parse_workspaces,
            )
            .await
    }

    async fn workspace_root(&self, dir: &Path, name: Option<String>) -> Result<PathBuf> {
        let mut args: Vec<String> = vec!["workspace".into(), "root".into()];
        if let Some(n) = name.as_deref() {
            args.push("--name".into());
            args.push(n.to_string());
        }
        Ok(PathBuf::from(self.core.text(self.cmd_in(dir, args)).await?))
    }

    async fn workspace_add(&self, dir: &Path, spec: WorkspaceAdd) -> Result<()> {
        // Built directly on `command_in` (not `cmd_in`) because the trailing
        // `--color never` must come after the chained value args, not between
        // `--name` and its value.
        let mut command = self
            .core
            .command_in(dir, ["workspace", "add", "--name"])
            .arg(&spec.name)
            .arg("-r")
            .arg(&spec.base);
        if let Some(mode) = spec.sparse_patterns {
            command = command.arg("--sparse-patterns").arg(mode.as_arg());
        }
        command = command.arg(&spec.path).arg("--color").arg("never");
        self.core.unit(command).await
    }

    async fn workspace_forget(&self, dir: &Path, name: &str) -> Result<()> {
        self.core
            .unit(self.cmd_in(dir, ["workspace", "forget", name]))
            .await
    }
}

// --- Error classification ----------------------------------------------------

/// Total attempts for a transient-retried fetch (1 try + 2 retries).
const FETCH_ATTEMPTS: u32 = 3;
/// Fixed backoff between fetch retries.
const FETCH_BACKOFF: Duration = Duration::from_millis(500);

/// Lower-case substrings marking a transient (retryable) network/fetch failure.
/// `jj git fetch` surfaces the underlying git transport error.
const TRANSIENT_FETCH_MARKERS: &[&str] = &[
    "could not resolve host",
    "couldn't resolve host",
    "temporary failure in name resolution",
    "connection timed out",
    "connection refused",
    "operation timed out",
    "timed out",
    "network is unreachable",
    "failed to connect",
    "could not read from remote repository",
    "the remote end hung up",
    "early eof",
    "rpc failed",
];

/// Whether a failed `git_fetch`/`git_fetch_branch` looks transient (DNS, timeout,
/// dropped connection) and is worth retrying. Mirrors `vcs_git`'s classifier.
pub fn is_transient_fetch_error(err: &Error) -> bool {
    // A processkit-level timeout carries no captured output but is inherently
    // transient; treat it as retryable too.
    if matches!(err, Error::Timeout { .. }) {
        return true;
    }
    let Error::Exit { stdout, stderr, .. } = err else {
        return false;
    };
    let out = stdout.to_ascii_lowercase();
    let errt = stderr.to_ascii_lowercase();
    TRANSIENT_FETCH_MARKERS
        .iter()
        .any(|m| out.contains(m) || errt.contains(m))
}

impl<R: ProcessRunner> Jj<R> {
    /// Run `jj <args>` over string slices — `jj.run_args(&["log", "-r", "@"])`
    /// without allocating a `Vec<String>`. Inherent (not on the object-safe
    /// trait), so it can take `&[&str]`; forwards to the same path as
    /// [`JjApi::run`].
    pub async fn run_args(&self, args: &[&str]) -> Result<String> {
        self.core.text(self.core.command(args)).await
    }

    /// Like [`run_args`](Jj::run_args) but never errors on a non-zero exit
    /// (mirrors [`JjApi::run_raw`]).
    pub async fn run_raw_args(&self, args: &[&str]) -> Result<ProcessResult<String>> {
        self.core.capture(self.core.command(args)).await
    }

    /// Bind this client to `dir`, returning a [`JjAt`] handle whose methods omit
    /// the `dir` argument: `jj.at(dir).status()` runs [`status`](JjApi::status)
    /// against `dir`. The dir-taking [`JjApi`] methods stay on [`Jj`] for driving
    /// many directories (e.g. workspaces) from one client.
    pub fn at<'a>(&'a self, dir: &'a Path) -> JjAt<'a, R> {
        JjAt { jj: self, dir }
    }

    /// Run a mutation sequence with op-log rollback: capture the current
    /// operation ([`op_head`](JjApi::op_head)), run `f` with a [`JjAt`] bound to
    /// `dir`, and on `Err` restore the repo to the captured operation
    /// ([`op_restore`](JjApi::op_restore)) before returning the error.
    ///
    /// ```no_run
    /// # async fn demo(jj: &vcs_jj::Jj) -> Result<(), processkit::Error> {
    /// jj.transaction(std::path::Path::new("."), |tx| async move {
    ///     tx.describe("wip").await?;
    ///     tx.new_change("next").await // an Err here rolls back the describe
    /// })
    /// .await?;
    /// # Ok(()) }
    /// ```
    ///
    /// Inherent (not on the object-safe trait): the closure parameter is
    /// generic, which `mockall` / trait objects can't express.
    ///
    /// Caveats:
    /// - Rollback runs on `Err` only — **not** on panic or cancellation (a
    ///   dropped future); there is no async `Drop`. Convert panics to `Err`
    ///   inside `f` if you need that safety.
    /// - If the restore itself fails, the *original* error from `f` is returned
    ///   and the repo may be left mid-transaction; re-probe
    ///   [`op_head`](JjApi::op_head) to detect that.
    pub async fn transaction<'a, T, F, Fut>(&'a self, dir: &'a Path, f: F) -> Result<T>
    where
        F: FnOnce(JjAt<'a, R>) -> Fut,
        Fut: Future<Output = Result<T>> + 'a,
    {
        let pre = self.op_head(dir).await?;
        match f(self.at(dir)).await {
            Ok(value) => Ok(value),
            Err(err) => {
                // Best-effort restore; the closure's error is the cause and is
                // what the caller must see even when the restore also fails.
                let _ = self.op_restore(dir, &pre).await;
                Err(err)
            }
        }
    }
}

/// A [`Jj`] client with a working directory bound, so calls drop the leading
/// `dir` argument — `jj.at(dir).status()` is `jj.status(dir)`. Construct one with
/// [`Jj::at`] (or, through the facade, `vcs_core::Repo::jj_at`). Cheap to copy: it
/// only borrows the client and the path.
pub struct JjAt<'a, R: ProcessRunner = processkit::JobRunner> {
    jj: &'a Jj<R>,
    dir: &'a Path,
}

// Hand-written rather than derived: holding only references, the view is `Copy`
// for *every* runner. `#[derive(Copy)]` would add a spurious `R: Copy` bound the
// default `JobRunner` doesn't satisfy, silently dropping `Copy` on the production
// handle.
impl<R: ProcessRunner> Clone for JjAt<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<R: ProcessRunner> Copy for JjAt<'_, R> {}

/// Generate [`JjAt`] forwarders from a method list: `bare` methods forward
/// verbatim, `dir` methods inject `self.dir` as the first argument.
macro_rules! jj_at_forwarders {
    (
        bare { $( fn $bn:ident( $($ba:ident: $bt:ty),* $(,)? ) -> $br:ty; )* }
        dir  { $( fn $dn:ident( $($da:ident: $dt:ty),* $(,)? ) -> $dr:ty; )* }
    ) => {
        impl<'a, R: ProcessRunner> JjAt<'a, R> {
            $(
                #[doc = concat!("Bound form of [`Jj`]'s `", stringify!($bn), "`.")]
                pub async fn $bn(&self, $($ba: $bt),*) -> $br {
                    self.jj.$bn($($ba),*).await
                }
            )*
            $(
                #[doc = concat!("Bound form of [`Jj`]'s `", stringify!($dn), "` (with `dir` pre-bound).")]
                pub async fn $dn(&self, $($da: $dt),*) -> $dr {
                    self.jj.$dn(self.dir, $($da),*).await
                }
            )*
        }
    };
}

jj_at_forwarders! {
    bare {
        fn run(args: &[String]) -> Result<String>;
        fn run_raw(args: &[String]) -> Result<ProcessResult<String>>;
        fn run_args(args: &[&str]) -> Result<String>;
        fn run_raw_args(args: &[&str]) -> Result<ProcessResult<String>>;
        fn version() -> Result<String>;
    }
    dir {
        fn status() -> Result<Vec<ChangedPath>>;
        fn status_text() -> Result<String>;
        fn log(revset: &str, max: usize) -> Result<Vec<Change>>;
        fn current_change() -> Result<Change>;
        fn describe(message: &str) -> Result<()>;
        fn describe_rev(revset: &str, message: &str) -> Result<()>;
        fn new_change(message: &str) -> Result<()>;
        fn bookmarks() -> Result<Vec<Bookmark>>;
        fn bookmarks_all() -> Result<Vec<BookmarkRef>>;
        fn reachable_bookmarks() -> Result<Vec<Bookmark>>;
        fn bookmark_track(name: &str, remote: &str) -> Result<()>;
        fn bookmark_set(name: &str, revision: &str) -> Result<()>;
        fn git_fetch() -> Result<()>;
        fn git_fetch_from(remote: &str) -> Result<()>;
        fn git_push(bookmark: Option<String>) -> Result<()>;
        fn root() -> Result<PathBuf>;
        fn current_bookmark() -> Result<Option<String>>;
        fn trunk() -> Result<Option<String>>;
        fn bookmark_create(name: &str, revision: &str) -> Result<()>;
        fn bookmark_rename(old: &str, new: &str) -> Result<()>;
        fn bookmark_delete(name: &str) -> Result<()>;
        fn bookmark_move(name: &str, to: &str, allow_backwards: bool) -> Result<()>;
        fn diff_summary(from: &str, to: &str) -> Result<Vec<ChangedPath>>;
        fn diff_stat(revset: &str) -> Result<DiffStat>;
        fn diff_text(spec: DiffSpec) -> Result<String>;
        fn diff(spec: DiffSpec) -> Result<Vec<FileDiff>>;
        fn commit_count(revset: &str) -> Result<usize>;
        fn is_conflicted(revset: &str) -> Result<bool>;
        fn has_workingcopy_conflict() -> Result<bool>;
        fn resolve_list(revset: &str) -> Result<Vec<String>>;
        fn template_query(revset: &str, template: &str, limit: Option<usize>) -> Result<String>;
        fn description(revset: &str) -> Result<String>;
        fn rebase(onto: &str) -> Result<()>;
        fn rebase_branch(branch: &str, dest: &str) -> Result<()>;
        fn edit(revset: &str) -> Result<()>;
        fn squash_into(into: &str, use_destination_message: bool) -> Result<()>;
        fn commit_paths(filesets: &[JjFileset], message: &str) -> Result<()>;
        fn squash_paths(from: &str, into: &str, filesets: &[JjFileset], use_destination_message: bool) -> Result<()>;
        fn sparse_set(patterns: &[String]) -> Result<()>;
        fn new_merge(message: &str, parents: Vec<String>) -> Result<()>;
        fn abandon(revset: &str) -> Result<()>;
        fn git_fetch_branch(branch: &str) -> Result<()>;
        fn git_import() -> Result<()>;
        fn op_head() -> Result<String>;
        fn op_restore(op_id: &str) -> Result<()>;
        fn op_undo() -> Result<()>;
        fn workspace_list() -> Result<Vec<Workspace>>;
        fn workspace_root(name: Option<String>) -> Result<PathBuf>;
        fn workspace_add(spec: WorkspaceAdd) -> Result<()>;
        fn workspace_forget(name: &str) -> Result<()>;
    }
}

// Manual forwarder: `transaction` takes a generic closure, which the declarative
// forwarder macro (fixed argument lists) cannot express.
impl<'a, R: ProcessRunner> JjAt<'a, R> {
    /// Bound form of [`Jj::transaction`] (with `dir` pre-bound): run `f` with
    /// op-log rollback on `Err`. See [`Jj::transaction`] for the caveats.
    pub async fn transaction<T, F, Fut>(&self, f: F) -> Result<T>
    where
        F: FnOnce(JjAt<'a, R>) -> Fut,
        Fut: Future<Output = Result<T>> + 'a,
    {
        self.jj.transaction(self.dir, f).await
    }
}

/// Synchronous, best-effort helpers for contexts that cannot `.await` — chiefly
/// a `Drop` guard. They shell out through `std::process` directly (no async, no
/// job-containment), so reserve them for short-lived cleanup.
pub mod blocking {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Forget a workspace synchronously (`jj workspace forget <name>`).
    pub fn workspace_forget(dir: &Path, name: &str) -> std::io::Result<()> {
        let status = Command::new(super::BINARY)
            .current_dir(dir)
            .args(["workspace", "forget", name])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "`jj workspace forget` exited with {status}"
            )))
        }
    }

    /// Resolve the workspace *name* whose root matches `path`, synchronously —
    /// for `Drop`, which can't `.await` the typed `workspace_list`/`workspace_root`.
    /// Lists workspaces (`workspace list -T name`), then matches each
    /// `workspace root --name <n>` against `path` (canonicalised, Windows
    /// verbatim-prefix stripped). `None` when jj is missing or nothing matches —
    /// the caller then skips the forget rather than guessing.
    pub fn workspace_name_for_path(dir: &Path, path: &Path) -> Option<String> {
        let target = normalize(path);
        let out = Command::new(super::BINARY)
            .current_dir(dir)
            .args(["workspace", "list", "-T", "name ++ \"\\n\""])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        for name in String::from_utf8_lossy(&out.stdout).lines() {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let root = Command::new(super::BINARY)
                .current_dir(dir)
                .args(["workspace", "root", "--name", name])
                .output();
            if let Ok(r) = root
                && r.status.success()
            {
                let p = PathBuf::from(String::from_utf8_lossy(&r.stdout).trim().to_string());
                if normalize(&p) == target || p == target || p == path {
                    return Some(name.to_string());
                }
            }
        }
        None
    }

    /// Canonicalise + strip the Windows verbatim prefix (`\\?\…`, which
    /// `canonicalize` adds but jj never emits) for stable path comparison.
    fn normalize(p: &Path) -> PathBuf {
        let canonical = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        #[cfg(windows)]
        {
            let s = canonical.to_string_lossy();
            if let Some(rest) = s.strip_prefix(r"\\?\")
                && !rest.starts_with("UNC\\")
            {
                return PathBuf::from(rest.to_string());
            }
        }
        canonical
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::{RecordingRunner, Reply, ScriptedRunner};

    #[test]
    fn binary_name_is_jj() {
        assert_eq!(BINARY, "jj");
    }

    // Compile-time guard: the bound view stays `Copy` for the default `JobRunner`.
    #[allow(dead_code)]
    fn bound_view_is_copy_for_default_runner() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<JjAt<'static, processkit::JobRunner>>();
    }

    // The bound view (`jj.at(dir)`) must produce byte-identical argv to the
    // dir-taking call — including the forced `--color never`.
    #[tokio::test]
    async fn bound_view_matches_dir_taking_calls() {
        let dir = Path::new("/repo");
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);

        jj.bookmark_move(dir, "main", "@", true).await.unwrap();
        jj.at(dir).bookmark_move("main", "@", true).await.unwrap();
        jj.describe_rev(dir, "feat", "msg").await.unwrap();
        jj.at(dir).describe_rev("feat", "msg").await.unwrap();
        jj.description(dir, "@-").await.unwrap();
        jj.at(dir).description("@-").await.unwrap();

        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), calls[1].args_str());
        assert_eq!(calls[2].args_str(), calls[3].args_str());
        assert_eq!(calls[4].args_str(), calls[5].args_str());
        assert_eq!(calls[1].cwd.as_deref(), Some(dir.as_os_str()));
    }

    #[tokio::test]
    async fn workspace_list_parses_template_rows() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(
            ["workspace", "list"],
            Reply::ok("default\te2aa3420\tmain\nws1\t12345678\t\n"),
        ));
        let got = jj.workspace_list(Path::new(".")).await.expect("list");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "default");
        assert_eq!(got[0].bookmarks, vec!["main".to_string()]);
        assert!(got[1].bookmarks.is_empty());
    }

    // `workspace add` must build `--name <n> -r <base> <path>` in order.
    #[tokio::test]
    async fn workspace_add_builds_name_base_path() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.workspace_add(Path::new("/repo"), WorkspaceAdd::new("ws1", "main", "/wt"))
            .await
            .expect("workspace add");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "workspace",
                "add",
                "--name",
                "ws1",
                "-r",
                "main",
                "/wt",
                "--color",
                "never"
            ]
        );
    }

    // `--sparse-patterns <mode>` lands between `-r <base>` and the path.
    #[tokio::test]
    async fn workspace_add_with_sparse_mode() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.workspace_add(
            Path::new("/repo"),
            WorkspaceAdd::new("ws1", "main", "/wt").sparse(SparseMode::Empty),
        )
        .await
        .expect("workspace add");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "workspace",
                "add",
                "--name",
                "ws1",
                "-r",
                "main",
                "--sparse-patterns",
                "empty",
                "/wt",
                "--color",
                "never"
            ]
        );
    }

    #[test]
    fn fileset_quotes_metacharacters() {
        assert_eq!(
            JjFileset::path("src/a(b).rs").as_str(),
            "file:\"src/a(b).rs\""
        );
        // Backslash and quote are escaped.
        assert_eq!(JjFileset::path("a\\\"b").as_str(), "file:\"a\\\\\\\"b\"");
    }

    #[tokio::test]
    async fn commit_paths_builds_filesets() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.commit_paths(
            Path::new("."),
            &[JjFileset::path("x|y.rs"), JjFileset::path("z.rs")],
            "msg",
        )
        .await
        .expect("commit_paths");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "commit",
                "-m",
                "msg",
                "file:\"x|y.rs\"",
                "file:\"z.rs\"",
                "--color",
                "never"
            ]
        );
    }

    #[tokio::test]
    async fn squash_paths_builds_from_into_filesets() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.squash_paths(
            Path::new("."),
            "@",
            "feat",
            &[JjFileset::path("a.rs")],
            false,
        )
        .await
        .expect("squash_paths");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "squash",
                "--from",
                "@",
                "--into",
                "feat",
                "file:\"a.rs\"",
                "--color",
                "never"
            ]
        );
    }

    #[tokio::test]
    async fn squash_paths_keeps_destination_message() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.squash_paths(
            Path::new("."),
            "@",
            "feat",
            &[JjFileset::path("a.rs")],
            true,
        )
        .await
        .expect("squash_paths");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "squash",
                "--from",
                "@",
                "--into",
                "feat",
                "--use-destination-message",
                "file:\"a.rs\"",
                "--color",
                "never"
            ]
        );
    }

    #[tokio::test]
    async fn jj_new_revision_scoped_ops_build_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.describe_rev(Path::new("."), "feat", "msg")
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["describe", "-r", "feat", "-m", "msg", "--color", "never"]
        );

        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.rebase_branch(Path::new("."), "feat", "main")
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["rebase", "-b", "feat", "-d", "main", "--color", "never"]
        );

        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.bookmark_track(Path::new("."), "feat", "origin")
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["bookmark", "track", "feat@origin", "--color", "never"]
        );
    }

    #[tokio::test]
    async fn bookmarks_all_parses_local_and_remote() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(
            ["bookmark", "list"],
            Reply::ok("main\t\t0\tabc123\nmain\torigin\t1\tabc123\n"),
        ));
        let refs = jj.bookmarks_all(Path::new(".")).await.unwrap();
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].name, "main");
        assert!(refs[0].remote.is_none() && !refs[0].tracked);
        assert_eq!(refs[1].remote.as_deref(), Some("origin"));
        assert!(refs[1].tracked);
    }

    #[tokio::test]
    async fn sparse_set_clears_then_adds() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.sparse_set(Path::new("."), &["README.md".into(), "lib".into()])
            .await
            .expect("sparse_set");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "sparse",
                "set",
                "--clear",
                "--add",
                "README.md",
                "--add",
                "lib",
                "--color",
                "never"
            ]
        );
    }

    // Parsed status() is backed by `diff -r @ --summary`, not `jj status`.
    #[tokio::test]
    async fn status_parses_diff_summary() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(
            ["diff", "-r", "@", "--summary"],
            Reply::ok("M a.rs\nA b.rs\n"),
        ));
        let entries = jj.status(Path::new(".")).await.expect("status");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].status, 'M');
        assert_eq!(entries[1].path, "b.rs");
    }

    #[tokio::test]
    async fn status_text_is_raw_jj_status() {
        let jj = Jj::with_runner(
            ScriptedRunner::new().on(["status"], Reply::ok("Working copy changes:\n")),
        );
        assert!(
            jj.status_text(Path::new("."))
                .await
                .expect("status_text")
                .contains("Working copy changes")
        );
    }

    #[tokio::test]
    async fn run_args_forwards_str_slices() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(["root"], Reply::ok("/r\n")));
        assert_eq!(jj.run_args(&["root"]).await.unwrap(), "/r");
    }

    #[test]
    fn classifies_transient_fetch() {
        let dns = Error::Exit {
            program: "jj".into(),
            code: 1,
            stdout: String::new(),
            stderr: "Error: Could not resolve host: example.com".into(),
        };
        assert!(is_transient_fetch_error(&dns));
        let other = Error::Exit {
            program: "jj".into(),
            code: 1,
            stdout: String::new(),
            stderr: "Error: No such revision".into(),
        };
        assert!(!is_transient_fetch_error(&other));

        // A processkit timeout (no captured output) is transient too.
        let timeout = Error::Timeout {
            program: "jj".into(),
            timeout: std::time::Duration::from_secs(10),
        };
        assert!(is_transient_fetch_error(&timeout));
    }

    #[tokio::test]
    async fn bookmark_move_appends_allow_backwards() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.bookmark_move(Path::new("/r"), "main", "@", true)
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            [
                "bookmark",
                "move",
                "main",
                "--to",
                "@",
                "--allow-backwards",
                "--color",
                "never"
            ]
        );
    }

    #[tokio::test]
    async fn new_merge_appends_parents() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.new_merge(Path::new("/r"), "m", vec!["p1".into(), "p2".into()])
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["new", "-m", "m", "p1", "p2", "--color", "never"]
        );
    }

    #[tokio::test]
    async fn is_conflicted_reads_template_flag() {
        let yes = Jj::with_runner(ScriptedRunner::new().on(["log"], Reply::ok("1\n")));
        assert!(yes.is_conflicted(Path::new("."), "@").await.unwrap());
        let no = Jj::with_runner(ScriptedRunner::new().on(["log"], Reply::ok("0\n")));
        assert!(!no.is_conflicted(Path::new("."), "@").await.unwrap());
    }

    #[tokio::test]
    async fn commit_count_counts_template_lines() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(["log"], Reply::ok("a\nb\nc\n")));
        assert_eq!(jj.commit_count(Path::new("."), "::@").await.unwrap(), 3);
    }

    #[tokio::test]
    async fn reachable_bookmarks_queries_heads_revset() {
        let rec = RecordingRunner::replying(Reply::ok("main\tabc123\n"));
        let jj = Jj::with_runner(&rec);
        let got = jj.reachable_bookmarks(Path::new(".")).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "main");
        let args = rec.only_call().args_str();
        assert_eq!(
            &args[..4],
            &["log", "-r", "heads(::@ & bookmarks())", "--no-graph"]
        );
    }

    #[tokio::test]
    async fn resolve_list_distinguishes_no_conflicts_from_errors() {
        // The benign "no conflicts" non-zero exit → empty list.
        let none = Jj::with_runner(ScriptedRunner::new().on(
            ["resolve"],
            Reply::fail(2, "Error: No conflicts found at this revision"),
        ));
        assert!(
            none.resolve_list(Path::new("."), "@")
                .await
                .unwrap()
                .is_empty()
        );
        // A real failure (e.g. bad revset) must surface, not read as "no conflicts".
        let bad = Jj::with_runner(ScriptedRunner::new().on(
            ["resolve"],
            Reply::fail(1, "Error: Revision `bogus` doesn't exist"),
        ));
        assert!(bad.resolve_list(Path::new("."), "bogus").await.is_err());
        // Success with conflicts → parsed paths.
        let some = Jj::with_runner(
            ScriptedRunner::new().on(["resolve"], Reply::ok("a.rs    2-sided conflict\n")),
        );
        assert_eq!(
            some.resolve_list(Path::new("."), "@").await.unwrap(),
            ["a.rs"]
        );
    }

    #[tokio::test]
    async fn current_bookmark_takes_first_or_none() {
        let some = Jj::with_runner(ScriptedRunner::new().on(["log"], Reply::ok("main\n")));
        assert_eq!(
            some.current_bookmark(Path::new("."))
                .await
                .unwrap()
                .as_deref(),
            Some("main")
        );
        let none = Jj::with_runner(ScriptedRunner::new().on(["log"], Reply::ok("\n")));
        assert!(
            none.current_bookmark(Path::new("."))
                .await
                .unwrap()
                .is_none()
        );
    }

    // Hermetic: real log() arg-building + template parsing against canned output.
    #[tokio::test]
    async fn current_change_parses_scripted_output() {
        let jj = Jj::with_runner(
            ScriptedRunner::new().on(["log"], Reply::ok("kztuxlro\t38e00654\tfalse\thello jj\n")),
        );
        let change = jj
            .current_change(Path::new("."))
            .await
            .expect("current_change");
        assert_eq!(change.change_id, "kztuxlro");
        assert!(!change.empty);
        assert_eq!(change.description, "hello jj");
    }

    // With a bookmark, the run must build `git push -b <name>`. Only that 4-token
    // command is scripted (no fallback), so a regression that dropped the flag
    // would match no rule and error.
    #[tokio::test]
    async fn git_push_appends_bookmark_flag() {
        let jj = Jj::with_runner(
            ScriptedRunner::new().on(["git", "push", "-b", "feature"], Reply::ok("")),
        );
        jj.git_push(Path::new("."), Some("feature".to_string()))
            .await
            .expect("should build `git push -b feature`");
    }

    // Without a bookmark, the run is a bare `git push`.
    #[tokio::test]
    async fn git_push_without_bookmark_is_bare() {
        let jj = Jj::with_runner(ScriptedRunner::new().on(["git", "push"], Reply::ok("")));
        jj.git_push(Path::new("."), None).await.expect("bare push");
    }

    // `git_fetch` retries a transient (network) failure up to FETCH_ATTEMPTS times.
    #[tokio::test]
    async fn git_fetch_retries_transient_failures() {
        let rec = RecordingRunner::replying(Reply::fail(1, "Error: Could not resolve host: x"));
        let jj = Jj::with_runner(&rec);
        assert!(jj.git_fetch(Path::new(".")).await.is_err());
        assert_eq!(rec.calls().len(), FETCH_ATTEMPTS as usize);
    }

    // `git_fetch_from` names the remote and shares `git_fetch`'s transient retry.
    #[tokio::test]
    async fn git_fetch_from_builds_args_and_retries() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.git_fetch_from(Path::new("."), "upstream")
            .await
            .expect("git_fetch_from");
        assert_eq!(
            rec.only_call().args_str(),
            ["git", "fetch", "--remote", "upstream", "--color", "never"]
        );

        let failing = RecordingRunner::replying(Reply::fail(1, "Error: Connection timed out"));
        let jj = Jj::with_runner(&failing);
        assert!(jj.git_fetch_from(Path::new("."), "upstream").await.is_err());
        assert_eq!(failing.calls().len(), FETCH_ATTEMPTS as usize);
    }

    // `transaction` captures the op head and restores it when the closure errors —
    // and the original (closure) error is what surfaces.
    #[tokio::test]
    async fn transaction_restores_op_head_on_error() {
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["op", "log"], Reply::ok("abc123\n"))
                .on(["op", "restore"], Reply::ok(""))
                .on(["describe"], Reply::fail(1, "boom")),
        );
        let jj = Jj::with_runner(&rec);
        let res = jj
            .transaction(
                Path::new("/r"),
                |tx| async move { tx.describe("wip").await },
            )
            .await;
        let err = res.expect_err("closure error must surface");
        assert!(matches!(err, Error::Exit { .. }));
        let calls = rec.calls();
        assert_eq!(calls.len(), 3, "op head, mutation, restore: {calls:?}");
        assert_eq!(calls[0].args_str()[..2], ["op", "log"]);
        assert_eq!(calls[1].args_str()[0], "describe");
        assert_eq!(calls[2].args_str()[..3], ["op", "restore", "abc123"]);
    }

    // A successful transaction must NOT restore (that would undo the work).
    #[tokio::test]
    async fn transaction_keeps_changes_on_success() {
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["op", "log"], Reply::ok("abc123\n"))
                .on(["describe"], Reply::ok("")),
        );
        let jj = Jj::with_runner(&rec);
        jj.transaction(
            Path::new("/r"),
            |tx| async move { tx.describe("wip").await },
        )
        .await
        .expect("transaction");
        let calls = rec.calls();
        assert_eq!(calls.len(), 2);
        assert!(
            calls.iter().all(|c| c.args_str()[..2] != ["op", "restore"]),
            "no restore on success: {calls:?}"
        );
    }

    // The bound view forwards `transaction` with `dir` pre-bound.
    #[tokio::test]
    async fn bound_view_forwards_transaction() {
        let dir = Path::new("/repo");
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["op", "log"], Reply::ok("op9\n"))
                .on(["new"], Reply::ok("")),
        );
        let jj = Jj::with_runner(&rec);
        jj.at(dir)
            .transaction(|tx| async move { tx.new_change("x").await })
            .await
            .expect("transaction");
        assert_eq!(rec.calls()[1].cwd.as_deref(), Some(dir.as_os_str()));
    }

    // `description` is a fixed template query: first match only, raw description.
    #[tokio::test]
    async fn description_builds_single_commit_template_query() {
        let rec = RecordingRunner::replying(Reply::ok("feat: parser\n\nbody\n"));
        let jj = Jj::with_runner(&rec);
        let text = jj
            .description(Path::new("."), "abc123")
            .await
            .expect("description");
        assert_eq!(text, "feat: parser\n\nbody");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "log",
                "-r",
                "abc123",
                "--no-graph",
                "--limit",
                "1",
                "-T",
                "description",
                "--color",
                "never"
            ]
        );
    }

    // `diff_text` for the working copy must build `diff -r @ --git`.
    #[tokio::test]
    async fn diff_text_builds_working_copy_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.diff_text(Path::new("."), DiffSpec::WorkingTree)
            .await
            .expect("diff_text");
        assert_eq!(
            rec.only_call().args_str(),
            ["diff", "-r", "@", "--git", "--color", "never"]
        );
    }

    // Every repo-scoped command forces `--color never` so a user's
    // `ui.color = "always"` config can't wrap parsed output in ANSI escapes.
    #[tokio::test]
    async fn commands_force_color_off() {
        let rec = RecordingRunner::replying(Reply::ok("x\n"));
        let jj = Jj::with_runner(&rec);
        jj.status_text(Path::new(".")).await.expect("status_text");
        let args = rec.only_call().args_str();
        let pos = args.iter().position(|a| a == "--color");
        assert_eq!(
            pos.map(|p| args.get(p + 1).map(String::as_str)),
            Some(Some("never"))
        );
    }

    // Hermetic: real diff() arg-building (`Rev`) + the ported parser against
    // canned git-format output.
    #[tokio::test]
    async fn diff_parses_scripted_output() {
        let out = "diff --git a/m b/m\n--- a/m\n+++ b/m\n@@ -1 +1 @@\n-a\n+b\n";
        let jj = Jj::with_runner(ScriptedRunner::new().on(["diff"], Reply::ok(out)));
        let files = jj
            .diff(Path::new("."), DiffSpec::Rev("@-".into()))
            .await
            .expect("diff");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "m");
        assert_eq!(files[0].change, ChangeKind::Modified);
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn consumer_mocks_the_interface() {
        let mut mock = MockJjApi::new();
        mock.expect_describe().returning(|_, _| Ok(()));
        assert!(mock.describe(Path::new("."), "msg").await.is_ok());
    }
}
