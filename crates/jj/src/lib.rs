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

use std::path::{Path, PathBuf};

use processkit::ProcessRunner;
// Re-export the processkit types in this crate's public API (also brings
// `Error`/`Result`/`ProcessResult` into scope here).
pub use processkit::{Error, ProcessResult, Result};

mod parse;
pub use parse::{
    Bookmark, Change, ChangeKind, ChangedPath, DiffLine, DiffStat, FileDiff, Hunk, Workspace,
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
    /// Working-copy status (`jj status`).
    async fn status(&self, dir: &Path) -> Result<String>;
    /// Changes matching `revset`, newest first, up to `max` (`jj log`).
    async fn log(&self, dir: &Path, revset: &str, max: usize) -> Result<Vec<Change>>;
    /// The working-copy change (`jj log -r @`).
    async fn current_change(&self, dir: &Path) -> Result<Change>;
    /// Set the working-copy change's description (`jj describe -m`).
    async fn describe(&self, dir: &Path, message: &str) -> Result<()>;
    /// Start a new change on top of the working copy (`jj new -m`).
    async fn new_change(&self, dir: &Path, message: &str) -> Result<()>;
    /// Local bookmarks (`jj bookmark list`).
    async fn bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>>;
    /// Point a bookmark at `revision` (`jj bookmark set <name> -r <revision>`).
    async fn bookmark_set(&self, dir: &Path, name: &str, revision: &str) -> Result<()>;
    /// Fetch from the git remote (`jj git fetch`).
    async fn git_fetch(&self, dir: &Path) -> Result<()>;
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
    /// Run an arbitrary templated `jj log` query and return raw stdout
    /// (`log -r <revset> --no-graph [--limit n] -T <template>`).
    async fn template_query(
        &self,
        dir: &Path,
        revset: &str,
        template: &str,
        limit: Option<usize>,
    ) -> Result<String>;

    // --- Mutations -----------------------------------------------------------

    /// Rebase onto a destination (`rebase -d <onto>`).
    async fn rebase(&self, dir: &Path, onto: &str) -> Result<()>;
    /// Move the working copy to a revision (`edit <rev>`).
    async fn edit(&self, dir: &Path, revset: &str) -> Result<()>;
    /// Squash the working copy into a revision (`squash --into <rev>`).
    async fn squash_into(&self, dir: &Path, into: &str) -> Result<()>;
    /// Finalise a commit from exactly these filesets (`commit -m <message>
    /// <filesets>`); the rest stay in the new working-copy change.
    async fn commit_paths(&self, dir: &Path, filesets: &[JjFileset], message: &str) -> Result<()>;
    /// Squash exactly these filesets from one revision into another
    /// (`squash --from <from> --into <into> <filesets>`).
    async fn squash_paths(
        &self,
        dir: &Path,
        from: &str,
        into: &str,
        filesets: &[JjFileset],
    ) -> Result<()>;
    /// Set the working copy's sparse patterns to exactly `patterns`
    /// (`sparse set --clear --add <p>…`); an empty list clears the working copy.
    async fn sparse_set(&self, dir: &Path, patterns: &[String]) -> Result<()>;
    /// Create a new change with the given parents (`new -m <msg> <p1> <p2> …`).
    async fn new_merge(&self, dir: &Path, message: &str, parents: Vec<String>) -> Result<()>;
    /// Abandon a revision (`abandon <rev>`).
    async fn abandon(&self, dir: &Path, revset: &str) -> Result<()>;
    /// Fetch a single bookmark from origin (`git fetch --remote origin -b <branch>`).
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

    async fn status(&self, dir: &Path) -> Result<String> {
        self.core.text(self.core.command_in(dir, ["status"])).await
    }

    async fn log(&self, dir: &Path, revset: &str, max: usize) -> Result<Vec<Change>> {
        let n = format!("-n{max}");
        self.core
            .parse(
                self.core.command_in(
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
            .unit(self.core.command_in(dir, ["describe", "-m", message]))
            .await
    }

    async fn new_change(&self, dir: &Path, message: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["new", "-m", message]))
            .await
    }

    async fn bookmarks(&self, dir: &Path) -> Result<Vec<Bookmark>> {
        self.core
            .parse(
                self.core.command_in(dir, ["bookmark", "list"]),
                parse::parse_bookmarks,
            )
            .await
    }

    async fn bookmark_set(&self, dir: &Path, name: &str, revision: &str) -> Result<()> {
        self.core
            .unit(
                self.core
                    .command_in(dir, ["bookmark", "set", name, "-r", revision]),
            )
            .await
    }

    async fn git_fetch(&self, dir: &Path) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["git", "fetch"]))
            .await
    }

    async fn git_push(&self, dir: &Path, bookmark: Option<String>) -> Result<()> {
        let mut args = vec!["git", "push"];
        if let Some(name) = bookmark.as_deref() {
            args.push("-b");
            args.push(name);
        }
        self.core.unit(self.core.command_in(dir, args)).await
    }

    async fn root(&self, dir: &Path) -> Result<PathBuf> {
        Ok(PathBuf::from(
            self.core.text(self.core.command_in(dir, ["root"])).await?,
        ))
    }

    async fn current_bookmark(&self, dir: &Path) -> Result<Option<String>> {
        let out = self
            .core
            .text(self.core.command_in(
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
            .text(self.core.command_in(
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
            .unit(
                self.core
                    .command_in(dir, ["bookmark", "create", name, "-r", revision]),
            )
            .await
    }

    async fn bookmark_rename(&self, dir: &Path, old: &str, new: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["bookmark", "rename", old, new]))
            .await
    }

    async fn bookmark_delete(&self, dir: &Path, name: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["bookmark", "delete", name]))
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
        self.core.unit(self.core.command_in(dir, args)).await
    }

    async fn diff_summary(&self, dir: &Path, from: &str, to: &str) -> Result<Vec<ChangedPath>> {
        let range = format!("{from}..{to}");
        self.core
            .parse(
                self.core
                    .command_in(dir, ["diff", "-r", range.as_str(), "--summary"]),
                parse::parse_diff_summary,
            )
            .await
    }

    async fn diff_stat(&self, dir: &Path, revset: &str) -> Result<DiffStat> {
        self.core
            .parse(
                self.core.command_in(dir, ["diff", "-r", revset, "--stat"]),
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
            .text(
                self.core
                    .command_in(dir, ["diff", "-r", revset.as_str(), "--git"]),
            )
            .await
    }

    async fn diff(&self, dir: &Path, spec: DiffSpec) -> Result<Vec<FileDiff>> {
        let text = self.diff_text(dir, spec).await?;
        Ok(parse::parse_diff(&text))
    }

    async fn commit_count(&self, dir: &Path, revset: &str) -> Result<usize> {
        self.core
            .parse(
                self.core.command_in(
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
            .text(self.core.command_in(
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
        self.core.text(self.core.command_in(dir, args)).await
    }

    async fn rebase(&self, dir: &Path, onto: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["rebase", "-d", onto]))
            .await
    }

    async fn edit(&self, dir: &Path, revset: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["edit", revset]))
            .await
    }

    async fn squash_into(&self, dir: &Path, into: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["squash", "--into", into]))
            .await
    }

    async fn commit_paths(&self, dir: &Path, filesets: &[JjFileset], message: &str) -> Result<()> {
        let mut args: Vec<String> = vec!["commit".into(), "-m".into(), message.into()];
        args.extend(filesets.iter().map(|f| f.as_str().to_string()));
        self.core.unit(self.core.command_in(dir, args)).await
    }

    async fn squash_paths(
        &self,
        dir: &Path,
        from: &str,
        into: &str,
        filesets: &[JjFileset],
    ) -> Result<()> {
        let mut args: Vec<String> = vec![
            "squash".into(),
            "--from".into(),
            from.into(),
            "--into".into(),
            into.into(),
        ];
        args.extend(filesets.iter().map(|f| f.as_str().to_string()));
        self.core.unit(self.core.command_in(dir, args)).await
    }

    async fn sparse_set(&self, dir: &Path, patterns: &[String]) -> Result<()> {
        // `--clear` empties the working copy first, then each `--add` reinstates a
        // pattern — so the working copy ends up holding exactly `patterns`.
        let mut args: Vec<String> = vec!["sparse".into(), "set".into(), "--clear".into()];
        for pattern in patterns {
            args.push("--add".into());
            args.push(pattern.clone());
        }
        self.core.unit(self.core.command_in(dir, args)).await
    }

    async fn new_merge(&self, dir: &Path, message: &str, parents: Vec<String>) -> Result<()> {
        let mut args: Vec<String> = vec!["new".into(), "-m".into(), message.into()];
        args.extend(parents);
        self.core.unit(self.core.command_in(dir, args)).await
    }

    async fn abandon(&self, dir: &Path, revset: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["abandon", revset]))
            .await
    }

    async fn git_fetch_branch(&self, dir: &Path, branch: &str) -> Result<()> {
        self.core
            .unit(
                self.core
                    .command_in(dir, ["git", "fetch", "--remote", "origin", "-b", branch]),
            )
            .await
    }

    async fn git_import(&self, dir: &Path) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["git", "import"]))
            .await
    }

    async fn op_head(&self, dir: &Path) -> Result<String> {
        self.core
            .text(self.core.command_in(
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
            .unit(self.core.command_in(dir, ["op", "restore", op_id]))
            .await
    }

    async fn op_undo(&self, dir: &Path) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["op", "undo"]))
            .await
    }

    async fn workspace_list(&self, dir: &Path) -> Result<Vec<Workspace>> {
        self.core
            .parse(
                self.core
                    .command_in(dir, ["workspace", "list", "-T", parse::WORKSPACE_TEMPLATE]),
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
        Ok(PathBuf::from(
            self.core.text(self.core.command_in(dir, args)).await?,
        ))
    }

    async fn workspace_add(&self, dir: &Path, spec: WorkspaceAdd) -> Result<()> {
        let mut command = self
            .core
            .command_in(dir, ["workspace", "add", "--name"])
            .arg(&spec.name)
            .arg("-r")
            .arg(&spec.base);
        if let Some(mode) = spec.sparse_patterns {
            command = command.arg("--sparse-patterns").arg(mode.as_arg());
        }
        command = command.arg(&spec.path);
        self.core.unit(command).await
    }

    async fn workspace_forget(&self, dir: &Path, name: &str) -> Result<()> {
        self.core
            .unit(self.core.command_in(dir, ["workspace", "forget", name]))
            .await
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
            ["workspace", "add", "--name", "ws1", "-r", "main", "/wt"]
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
                "/wt"
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
            ["commit", "-m", "msg", "file:\"x|y.rs\"", "file:\"z.rs\""]
        );
    }

    #[tokio::test]
    async fn squash_paths_builds_from_into_filesets() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.squash_paths(Path::new("."), "@", "feat", &[JjFileset::path("a.rs")])
            .await
            .expect("squash_paths");
        assert_eq!(
            rec.only_call().args_str(),
            ["squash", "--from", "@", "--into", "feat", "file:\"a.rs\""]
        );
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
                "lib"
            ]
        );
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
            ["bookmark", "move", "main", "--to", "@", "--allow-backwards"]
        );
    }

    #[tokio::test]
    async fn new_merge_appends_parents() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.new_merge(Path::new("/r"), "m", vec!["p1".into(), "p2".into()])
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["new", "-m", "m", "p1", "p2"]);
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

    // `diff_text` for the working copy must build `diff -r @ --git`.
    #[tokio::test]
    async fn diff_text_builds_working_copy_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let jj = Jj::with_runner(&rec);
        jj.diff_text(Path::new("."), DiffSpec::WorkingTree)
            .await
            .expect("diff_text");
        assert_eq!(rec.only_call().args_str(), ["diff", "-r", "@", "--git"]);
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
