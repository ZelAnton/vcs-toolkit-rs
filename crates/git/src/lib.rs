#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-git` — automate Git from Rust by driving the `git` CLI.
//!
//! You call typed `async` methods; `vcs-git` runs the real `git`, parses its
//! output, and hands you structured values — so you get *git's own* behaviour,
//! config, and credentials, not a reimplementation of the object format. Async,
//! structured errors, mockable. Every command runs inside an OS **job** (an
//! OS-level container that kills the whole process tree if your program exits, via
//! [`processkit`]) so a `git` subprocess is never orphaned, with an optional
//! per-client [timeout](Git::default_timeout).
//!
//! # What you can do
//!
//! Status & branches · stage, commit, checkout · diff & log · merge / rebase /
//! reset · worktrees · tags · blame · clone · config · cherry-pick / revert · parse
//! & resolve conflict markers · a hardened (hooks-off) profile for untrusted repos.
//! One tiny call to start:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_git::{Git, GitApi};
//! # async fn demo() -> Result<(), processkit::Error> {
//! let git = Git::new();
//! // `current_branch` is `Option` — `None` on a detached HEAD.
//! println!("{:?}", git.current_branch(Path::new(".")).await?); // e.g. Some("main")
//! # Ok(()) }
//! ```
//!
//! # The surface (engineering reference)
//!
//! - **[`GitApi`]** — the object-safe trait every operation lives on. Depend on
//!   `&dyn GitApi` (or generically on `impl GitApi`) so a test can swap the real
//!   client for a double. Methods take the working directory as the first
//!   argument and return typed results ([`StatusEntry`], [`Branch`], [`Commit`],
//!   [`FileDiff`], [`BlameLine`], …) or a structured [`Error`].
//! - **[`Git`]** — the real client. [`Git::new`] uses the job-backed runner;
//!   [`Git::with_runner`] injects a fake one for tests. It is generic over the
//!   [`ProcessRunner`] seam, defaulting to the production runner.
//!   [`with_credentials`](Git::with_credentials) attaches a [`CredentialProvider`]
//!   to authenticate HTTPS remote ops (fetch/push/clone/ls-remote) with a token
//!   kept out of `argv` — opt-in, off by default (ambient helpers / SSH agent).
//! - **[`GitAt`]** — a cwd-bound view ([`Git::at`]) whose methods drop the
//!   leading `dir`, so `git.at(dir).status()` reads as `git.status(dir)` — handy
//!   when one client drives one checkout.
//! - **Builder specs** for the multi-option commands — [`CommitPaths`],
//!   [`MergeCommit`] / [`MergeNoCommit`], [`GitPush`], [`CloneSpec`],
//!   [`WorktreeAdd`], [`AnnotatedTag`] — each `#[non_exhaustive]`, built with a
//!   constructor + chained setters, named after the flags they emit.
//! - **[`conflict`]** — a typed conflict-marker model: parse marker soup into
//!   structured regions, re-render byte-exact, and resolve to a chosen side.
//! - **[`Git::hardened`]** — a profile for untrusted repositories (hooks off,
//!   `GIT_*` scrubbed, system config skipped); see the [`guide::security`] guide.
//!
//! # Recipes
//!
//! Read state — depend on the trait so the same code takes a real client or a mock:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_git::{Git, GitApi};
//! # async fn demo() -> Result<(), processkit::Error> {
//! let git = Git::new();
//! let dir = Path::new(".");
//! let branch = git.current_branch(dir).await?;        // the checked-out branch
//! let dirty = !git.status(dir).await?.is_empty();     // any uncommitted change?
//! # let _ = (branch, dirty); Ok(()) }
//! ```
//!
//! Mutate through the builder specs — `fetch` retries transient network failures:
//!
//! ```no_run
//! use std::path::Path;
//! use vcs_git::{CommitPaths, Git, GitApi, GitPush};
//! # async fn demo(git: &Git) -> Result<(), processkit::Error> {
//! let dir = Path::new(".");
//! git.fetch(dir).await?;
//! git.commit_paths(dir, CommitPaths::new(["src/a.rs"], "wip")).await?;
//! git.push(dir, GitPush::branch("feature").set_upstream()).await?;
//! # Ok(()) }
//! ```
//!
//! # Testing
//!
//! Two seams: enable the **`mock`** feature for a `mockall`-generated
//! `MockGitApi` (stub whole methods), or inject a
//! [`ScriptedRunner`](processkit::testing::ScriptedRunner) with [`Git::with_runner`] to
//! exercise the *real* argv-building and parsing against canned output. The
//! cross-cutting testing patterns live in
//! [vcs-testkit's guide](https://docs.rs/vcs-testkit/latest/vcs_testkit/guide/testing/).
//!
//! # Safety
//!
//! Every caller value placed in a bare positional argv slot is refused before
//! spawning if it is empty or starts with `-` (git would parse it as a flag);
//! flag-value slots (`-b <name>`) are consumed verbatim and don't need it. For
//! eager validation at an input boundary, the [`RefName`] / [`RevSpec`] newtypes
//! validate up front. Paths always go through `--` / pathspec.
//!
//! # In-depth guide
//!
//! Beyond this page, this crate ships a full how-to guide — rendered on docs.rs
//! from `docs/`. See the [`guide`] module (and its
//! [`security`](crate::guide::security) / [`conflicts`](crate::guide::conflicts)
//! sub-guides).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use processkit::Command;
// Re-export the processkit types that appear in this crate's public API, so
// consumers needn't depend on processkit directly — incl. `ProcessRunner` (the
// `with_runner`/`Git<R>` seam) and the `JobRunner` default. (`Error`/`Result`/
// `ProcessResult`/`ProcessRunner` are in scope here too via this `pub use`.)
pub use processkit::{Error, JobRunner, ProcessResult, ProcessRunner, Result};
// Re-exported so a consumer can name the token for `default_cancel_on` without
// taking a direct `processkit` dependency.
pub use processkit::CancellationToken;

pub mod conflict;
mod parse;
pub use parse::{BlameLine, Branch, BranchStatus, Commit, StatusEntry, Worktree};
// The git-format diff model + parser and the version type are shared with
// `vcs-jj` (identical output) — re-exported so `vcs_git::FileDiff`,
// `vcs_git::parse_diff`, `vcs_git::GitVersion`, … still resolve.
pub use vcs_diff::{
    ChangeKind, DiffLine, DiffSpec, DiffStat, FileDiff, Hunk, Version as GitVersion, parse_diff,
};
// The error classifiers live in the shared plumbing crate — re-exported so
// `vcs_git::is_merge_conflict`, … still resolve.
use vcs_cli_support::git_credential_helper;
pub use vcs_cli_support::{
    Credential, CredentialProvider, CredentialRequest, CredentialService, EnvToken, RetryPolicy,
    Secret, StaticCredential, is_lock_contention, is_merge_conflict, is_nothing_to_commit,
    is_transient_fetch_error, provider_fn,
};

/// Name of the underlying CLI binary this crate drives.
pub const BINARY: &str = "git";

/// Options for [`GitApi::worktree_add`] (`git worktree add`).
///
/// `#[non_exhaustive]`, so build it through [`WorktreeAdd::checkout`] /
/// [`WorktreeAdd::create_branch`] rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WorktreeAdd {
    /// Filesystem path for the new worktree.
    pub path: PathBuf,
    /// Create and check out this new branch (`-b <name>`); `None` checks out an
    /// existing ref.
    pub new_branch: Option<String>,
    /// The commit/branch to base the worktree on; `None` defaults to `HEAD`.
    pub commitish: Option<String>,
    /// Register the worktree without populating its files (`--no-checkout`) — the
    /// caller fills the working tree itself (e.g. a copy-on-write clone).
    pub no_checkout: bool,
}

impl WorktreeAdd {
    /// A worktree at `path` checking out an existing `commitish` (e.g. a branch):
    /// `git worktree add <path> <commitish>`.
    pub fn checkout(path: impl Into<PathBuf>, commitish: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            new_branch: None,
            commitish: Some(commitish.into()),
            no_checkout: false,
        }
    }

    /// A worktree at `path` creating a new branch `name` based on `commitish`:
    /// `git worktree add -b <name> <path> <commitish>`.
    pub fn create_branch(
        path: impl Into<PathBuf>,
        name: impl Into<String>,
        commitish: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            new_branch: Some(name.into()),
            commitish: Some(commitish.into()),
            no_checkout: false,
        }
    }

    /// Register the worktree without checking out its files (`--no-checkout`),
    /// for a caller that populates the working tree itself.
    pub fn no_checkout(mut self) -> Self {
        self.no_checkout = true;
        self
    }
}

/// Options for [`GitApi::push`] (`git push`).
///
/// `#[non_exhaustive]`, so build it through [`GitPush::branch`] /
/// [`GitPush::refspec`] rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct GitPush {
    /// Remote to push to (defaults to `origin`).
    pub remote: String,
    /// The refspec — a bare branch name, or `local:remote_branch`.
    pub refspec: String,
    /// Set the pushed branch as the upstream (`-u`).
    pub set_upstream: bool,
}

impl GitPush {
    /// Push branch `name` to `origin` under the same name (`git push origin <name>`).
    pub fn branch(name: impl Into<String>) -> Self {
        Self {
            remote: "origin".to_string(),
            refspec: name.into(),
            set_upstream: false,
        }
    }

    /// Push `local` to a differently-named `remote_branch`
    /// (`git push origin <local>:<remote_branch>`).
    pub fn refspec(local: impl AsRef<str>, remote_branch: impl AsRef<str>) -> Self {
        Self {
            remote: "origin".to_string(),
            refspec: format!("{}:{}", local.as_ref(), remote_branch.as_ref()),
            set_upstream: false,
        }
    }

    /// Push to a non-default remote.
    pub fn remote(mut self, remote: impl Into<String>) -> Self {
        self.remote = remote.into();
        self
    }

    /// Record the pushed branch as the local branch's upstream (`-u`).
    pub fn set_upstream(mut self) -> Self {
        self.set_upstream = true;
        self
    }
}

/// Options for [`GitApi::clone_repo`] (`git clone`).
///
/// `#[non_exhaustive]`, so build it through [`CloneSpec::new`] and the chained
/// setters rather than a struct literal.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct CloneSpec {
    /// Check out this branch instead of the remote's default (`--branch`).
    pub branch: Option<String>,
    /// Shallow-clone to this many commits (`--depth`). git silently ignores
    /// the flag for a plain local-path source (warns, still clones fully);
    /// use a `file://` URL to shallow-clone locally.
    pub depth: Option<u32>,
    /// Create a bare repository (`--bare`).
    pub bare: bool,
}

impl CloneSpec {
    /// A plain full clone of the remote's default branch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check out `branch` instead of the remote's default (`--branch`).
    pub fn branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// Shallow-clone to `depth` commits (`--depth`); see the field doc for the
    /// local-path caveat.
    pub fn depth(mut self, depth: u32) -> Self {
        self.depth = Some(depth);
        self
    }

    /// Clone as a bare repository (`--bare`).
    pub fn bare(mut self) -> Self {
        self.bare = true;
        self
    }
}

/// Options for [`GitApi::commit_paths`] (`git commit --only`).
///
/// `#[non_exhaustive]`, so build it through [`CommitPaths::new`] and the chained
/// setters rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CommitPaths {
    /// The exact paths whose working-tree content to commit (`--only -- <paths>`).
    pub paths: Vec<PathBuf>,
    /// The commit message (`-m`).
    pub message: String,
    /// Amend the previous commit instead of creating a new one (`--amend`).
    pub amend: bool,
}

impl CommitPaths {
    /// Commit exactly `paths`' working-tree content with `message`
    /// (`git commit -m <message> --only -- <paths>`).
    pub fn new(
        paths: impl IntoIterator<Item = impl Into<PathBuf>>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            paths: paths.into_iter().map(Into::into).collect(),
            message: message.into(),
            amend: false,
        }
    }

    /// Amend the previous commit instead of creating a new one (`--amend`).
    pub fn amend(mut self) -> Self {
        self.amend = true;
        self
    }
}

/// Options for [`GitApi::merge_commit`] (`git merge` that commits the result).
///
/// `#[non_exhaustive]`, so build it through [`MergeCommit::branch`] and the
/// chained setters rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MergeCommit {
    /// The branch to merge in.
    pub branch: String,
    /// Always create a merge commit, even when a fast-forward was possible
    /// (`--no-ff`).
    pub no_ff: bool,
    /// The merge commit message (`-m`); `None` takes the default message
    /// non-interactively (`--no-edit`).
    pub message: Option<String>,
}

impl MergeCommit {
    /// Merge `name` taking the default merge message non-interactively
    /// (`git merge --no-edit <name>`).
    pub fn branch(name: impl Into<String>) -> Self {
        Self {
            branch: name.into(),
            no_ff: false,
            message: None,
        }
    }

    /// Always create a merge commit, even when a fast-forward was possible
    /// (`--no-ff`).
    pub fn no_ff(mut self) -> Self {
        self.no_ff = true;
        self
    }

    /// Use `m` as the merge commit message (`-m`).
    pub fn message(mut self, m: impl Into<String>) -> Self {
        self.message = Some(m.into());
        self
    }
}

/// Options for [`GitApi::merge_no_commit`] (`git merge --no-commit`).
///
/// `#[non_exhaustive]`, so build it through [`MergeNoCommit::branch`] and the
/// chained setters rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MergeNoCommit {
    /// The branch to merge in.
    pub branch: String,
    /// Stage the squashed result without recording `MERGE_HEAD` (`--squash`);
    /// takes precedence over `no_ff` (git rejects the pair).
    pub squash: bool,
    /// Always record a real (abortable) merge, even when a fast-forward was
    /// possible (`--no-ff`).
    pub no_ff: bool,
}

impl MergeNoCommit {
    /// Merge `name` but stop before committing (`git merge --no-commit <name>`).
    pub fn branch(name: impl Into<String>) -> Self {
        Self {
            branch: name.into(),
            squash: false,
            no_ff: false,
        }
    }

    /// Stage the squashed result without recording `MERGE_HEAD` (`--squash`).
    pub fn squash(mut self) -> Self {
        self.squash = true;
        self
    }

    /// Always record a real (abortable) merge, even when a fast-forward was
    /// possible (`--no-ff`).
    pub fn no_ff(mut self) -> Self {
        self.no_ff = true;
        self
    }
}

/// Options for [`GitApi::tag_create_annotated`] (`git tag -a`).
///
/// `#[non_exhaustive]`, so build it through [`AnnotatedTag::new`] and the chained
/// setter rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AnnotatedTag {
    /// The tag name.
    pub name: String,
    /// The tag message (`-m`).
    pub message: String,
    /// The revision to tag (`<rev>`); `None` tags `HEAD`.
    pub rev: Option<String>,
}

impl AnnotatedTag {
    /// An annotated tag `name` with `message` at `HEAD`
    /// (`git tag -a <name> -m <message>`).
    pub fn new(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            message: message.into(),
            rev: None,
        }
    }

    /// Tag `r` instead of `HEAD`.
    pub fn rev(mut self, r: impl Into<String>) -> Self {
        self.rev = Some(r.into());
        self
    }
}

/// A pre-validated git reference name (branch/tag/remote), for callers that
/// accept names from untrusted input (UIs, bots, agents) and want to fail
/// early with a clear error. The dir-taking methods stay `&str` — they apply
/// the same flag-injection guard internally — so this type is **optional**
/// up-front validation, not a required wrapper.
///
/// Rules follow the load-bearing core of `git check-ref-format`: non-empty,
/// no leading `-` or `.`, no `..`, no control characters or space, none of
/// `~ ^ : ? * [ \`, no trailing `/` or `.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RefName(String);

impl RefName {
    /// Validate `name` as a reference name.
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        let bad = name.is_empty()
            || name.starts_with('-')
            || name.starts_with('.')
            || name.ends_with('/')
            || name.ends_with(".lock")
            || name.contains("..")
            || name
                .chars()
                .any(|c| c.is_control() || " ~^:?*[\\".contains(c));
        if bad {
            return Err(Error::Spawn {
                program: BINARY.to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid git reference name: {name:?}"),
                ),
            });
        }
        Ok(RefName(name))
    }

    /// The validated name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RefName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A pre-validated revision/range expression (`HEAD~2`, `main..feature`).
/// Deliberately *minimal* — git's revision grammar is too rich to validate
/// here — it only guarantees the expression is non-empty and cannot be parsed
/// as a flag (no leading `-`), matching the internal guard the dir-taking
/// methods apply anyway. Optional up-front validation for untrusted input.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RevSpec(String);

impl RevSpec {
    /// Validate `rev` as a revision/range expression (non-empty, no leading `-`).
    pub fn new(rev: impl Into<String>) -> Result<Self> {
        let rev = rev.into();
        reject_flag_like("revision", &rev)?;
        Ok(RevSpec(rev))
    }

    /// The validated expression.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RevSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the installed `git` binary supports, probed via
/// [`GitApi::capabilities`]. A value type — the client holds no state, so
/// probe once and keep the result (callers cache it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct GitCapabilities {
    /// The binary's parsed version.
    pub version: GitVersion,
}

/// The oldest git major this crate is written against. Validated on 2.54;
/// expected to work from ≥ 2.30 — but only the *major* is hard-gated, because
/// a false "unsupported" on an untested-but-fine 2.2x would be worse than the
/// argv error git itself would give. (Contrast vcs-jj, whose floor is precise:
/// its parsers were empirically validated against one jj release.)
const MIN_SUPPORTED_MAJOR: u64 = 2;

impl GitCapabilities {
    /// Whether the binary meets the supported floor (major ≥ 2).
    pub fn is_supported(&self) -> bool {
        self.version.major >= MIN_SUPPORTED_MAJOR
    }

    /// Error unless [`is_supported`](Self::is_supported) — a clear "needs git
    /// ≥ 2, found 1.9.5" instead of a cryptic argv failure later.
    pub fn ensure_supported(&self) -> Result<()> {
        if self.is_supported() {
            return Ok(());
        }
        Err(Error::Spawn {
            program: BINARY.to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!(
                    "vcs-git requires git >= {MIN_SUPPORTED_MAJOR} (validated on 2.54), \
                     found {}",
                    self.version
                ),
            ),
        })
    }
}

/// The Git operations this crate exposes — the interface consumers code against
/// and mock in tests.
///
/// **Injection safety:** every method that places a caller-supplied name,
/// revision, range, remote, or URL in a positional argv slot rejects a value
/// that is empty or begins with `-` (it would be parsed as a flag) with an
/// [`Error::Spawn`] *before* spawning. Flag-value slots (`-m <msg>`,
/// `--branch <b>`), filesystem path arguments (`--`-separated pathspecs, plus
/// worktree paths and clone destinations — typed `Path`, caller-trusted), and
/// the `run`/`run_raw` escape hatches are not guarded. For eager validation at
/// an input boundary, see [`RefName`] / [`RevSpec`].
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait::async_trait]
pub trait GitApi: Send + Sync {
    /// Run `git <args>` **in the process's current directory**, returning trimmed
    /// stdout (throws on a non-zero exit). A raw escape hatch for unmodelled commands
    /// — you supply the whole argv, so target a specific repo with `-C <dir>` in the
    /// args. The `at(dir)` bound view does **not** re-bind `run`/`run_args` (they and
    /// the other `run*` hatches stay process-cwd, unlike every modelled `GitAt`
    /// method); pass `-C dir` explicitly if you need the bound repo (M15).
    async fn run(&self, args: &[String]) -> Result<String>;
    /// Like [`GitApi::run`] but never errors on a non-zero exit — returns the
    /// captured [`ProcessResult`].
    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>>;
    /// Installed Git version (`git --version`).
    async fn version(&self) -> Result<String>;
    /// The installed binary's parsed version, as [`GitCapabilities`]
    /// (`git --version`). A value type — probe once and keep it; an
    /// unrecognisable version string is an [`Error::Parse`].
    async fn capabilities(&self) -> Result<GitCapabilities>;
    /// Working-tree status (`git status --porcelain=v1 -z`).
    async fn status(&self, dir: &Path) -> Result<Vec<StatusEntry>>;
    /// Raw porcelain status text (`git status --porcelain=v1`) — the unparsed
    /// counterpart of [`status`](GitApi::status), mirroring `vcs_jj` `status_text`.
    async fn status_text(&self, dir: &Path) -> Result<String>;
    /// Like [`status`](GitApi::status) but ignoring untracked files
    /// (`git status --porcelain=v1 -z --untracked-files=no`) — "is the *tracked*
    /// tree dirty", staged or not.
    async fn status_tracked(&self, dir: &Path) -> Result<Vec<StatusEntry>>;
    /// A combined branch + working-tree snapshot in **one** spawn
    /// (`git status --porcelain=v2 --branch -z`): HEAD, branch, upstream,
    /// ahead/behind, and change counts — the data a prompt/status-bar needs
    /// without N round-trips. See [`BranchStatus`].
    async fn branch_status(&self, dir: &Path) -> Result<BranchStatus>;
    /// Paths with unresolved merge conflicts, repo-relative with `/` separators
    /// (`git diff --name-only --diff-filter=U -z`). Empty when there are none.
    async fn conflicted_files(&self, dir: &Path) -> Result<Vec<String>>;
    /// Current branch name, or `None` on a **detached HEAD**
    /// (`git symbolic-ref --quiet --short HEAD`). Returns the branch name for a
    /// normal branch **and for an unborn branch** (a fresh `init`/`clone` before the
    /// first commit); `None` only when HEAD is detached. Mirrors
    /// [`JjApi::current_bookmark`](../vcs_jj/trait.JjApi.html#tymethod.current_bookmark)'s
    /// `Option` shape, so cross-backend code treats "no named branch/bookmark" the
    /// same way on both wrappers.
    async fn current_branch(&self, dir: &Path) -> Result<Option<String>>;
    /// Local branches, current one flagged (`git branch`).
    async fn branches(&self, dir: &Path) -> Result<Vec<Branch>>;
    /// Up to `max` commits reachable from `revspec`, newest first
    /// (`git log <revspec>`). Pass `"HEAD"` for the current branch's history, or
    /// a range like `"main..HEAD"` / `"origin/main..HEAD"` to scope it. Mirrors
    /// [`JjApi::log`](../vcs_jj/trait.JjApi.html#tymethod.log)'s revset argument,
    /// so cross-backend code uses one signature. The `revspec` is guarded against
    /// being parsed as a flag.
    async fn log(&self, dir: &Path, revspec: &str, max: usize) -> Result<Vec<Commit>>;
    /// Resolve a revision to a full hash (`git rev-parse --verify <rev>`). `--verify`
    /// requires `rev` to name exactly one object, so a non-revision (e.g. a filename)
    /// errors instead of being echoed back as a fake id.
    async fn rev_parse(&self, dir: &Path, rev: &str) -> Result<String>;
    /// Resolve a revision to its abbreviated hash (`git rev-parse --short <rev>`) —
    /// e.g. to label a detached HEAD.
    async fn rev_parse_short(&self, dir: &Path, rev: &str) -> Result<String>;
    /// Initialise a repository (`git init`).
    async fn init(&self, dir: &Path) -> Result<()>;
    /// Stage `paths` (`git add -- <paths>`).
    async fn add(&self, dir: &Path, paths: &[PathBuf]) -> Result<()>;
    /// Commit staged changes (`git commit -m`).
    async fn commit(&self, dir: &Path, message: &str) -> Result<()>;
    /// Create a branch without switching to it (`git branch <name>`).
    async fn create_branch(&self, dir: &Path, name: &str) -> Result<()>;
    /// Switch to a branch or revision (`git checkout <reference>`).
    async fn checkout(&self, dir: &Path, reference: &str) -> Result<()>;
    /// Check out a commit as a detached HEAD (`git checkout --detach <commit>`).
    async fn checkout_detach(&self, dir: &Path, commit: &str) -> Result<()>;
    /// Commit exactly the spec's paths' working-tree content, ignoring the index
    /// (`git commit [--amend] -m <message> --only -- <paths>`); see [`CommitPaths`].
    async fn commit_paths(&self, dir: &Path, spec: CommitPaths) -> Result<()>;
    /// The last commit's full message (`git log -1 --format=%B`) — e.g. to
    /// pre-fill an amend.
    async fn last_commit_message(&self, dir: &Path) -> Result<String>;
    /// Whether `HEAD` is unborn — a fresh repo with no commits yet
    /// (`git rev-parse --verify -q HEAD`, exit-code mapped).
    async fn is_unborn(&self, dir: &Path) -> Result<bool>;
    /// Whether the working tree has no unstaged modifications to **tracked** files
    /// (`git diff --quiet`). Untracked files are *not* counted — this is not a full
    /// "is the working tree clean?" check; use [`status`](GitApi::status) for that.
    async fn diff_is_empty(&self, dir: &Path) -> Result<bool>;

    // --- Discovery / identity ------------------------------------------------

    /// The repository's common git directory (`rev-parse --git-common-dir`) —
    /// stable across linked worktrees.
    async fn common_dir(&self, dir: &Path) -> Result<PathBuf>;
    /// This worktree's git directory (`rev-parse --git-dir`).
    async fn git_dir(&self, dir: &Path) -> Result<PathBuf>;
    /// Resolve a revision to a commit hash, peeling tags
    /// (`rev-parse --verify <rev>^{commit}`).
    async fn resolve_commit(&self, dir: &Path, rev: &str) -> Result<String>;
    /// The remote's default branch from `symbolic-ref refs/remotes/origin/HEAD`
    /// (short name only); `None` when `origin/HEAD` is unset.
    async fn remote_head_branch(&self, dir: &Path) -> Result<Option<String>>;
    /// Whether a local branch exists (`show-ref --verify --quiet refs/heads/<name>`).
    async fn branch_exists(&self, dir: &Path, name: &str) -> Result<bool>;
    /// Whether `origin` has `name`, without fetching (`ls-remote origin
    /// refs/heads/<name>` — the fully-qualified ref, so `foo` can't tail-match
    /// `bar/foo`). Runs with `GIT_TERMINAL_PROMPT=0` and a 10s timeout so a missing
    /// credential or a flaky network can't hang the call.
    async fn remote_branch_exists(&self, dir: &Path, name: &str) -> Result<bool>;
    /// A remote's URL (`remote get-url <remote>`).
    async fn remote_url(&self, dir: &Path, remote: &str) -> Result<String>;
    /// The current branch's upstream, e.g. `Some("origin/main")`
    /// (`rev-parse --abbrev-ref --symbolic-full-name @{u}`); `None` when unset.
    async fn upstream(&self, dir: &Path) -> Result<Option<String>>;
    /// Branch names on `remote`, without fetching
    /// (`ls-remote --heads <remote>`).
    async fn remote_branches(&self, dir: &Path, remote: &str) -> Result<Vec<String>>;

    // --- Branches ------------------------------------------------------------

    /// Whether `branch` is fully merged into `target` (`branch --merged <target>`).
    async fn is_merged(&self, dir: &Path, branch: &str, target: &str) -> Result<bool>;
    /// Set `branch`'s upstream to `upstream` (e.g. `origin/main`)
    /// (`branch --set-upstream-to=<upstream> <branch>`).
    async fn set_upstream(&self, dir: &Path, branch: &str, upstream: &str) -> Result<()>;
    /// Delete a local branch (`branch -d`, or `-D` when `force`).
    async fn delete_branch(&self, dir: &Path, name: &str, force: bool) -> Result<()>;
    /// Rename a local branch (`branch -m <old> <new>`).
    async fn rename_branch(&self, dir: &Path, old: &str, new: &str) -> Result<()>;
    /// Count commits in a range (`rev-list --count <range>`).
    async fn rev_list_count(&self, dir: &Path, range: &str) -> Result<usize>;
    /// Whether a diff range is empty (`diff --quiet <range>`).
    async fn diff_range_is_empty(&self, dir: &Path, range: &str) -> Result<bool>;
    /// Aggregate change stats for a range (`diff --shortstat <range>`). Named to
    /// match `vcs_jj::JjApi::diff_stat`.
    async fn diff_stat(&self, dir: &Path, range: &str) -> Result<DiffStat>;
    /// Raw git-format unified diff text for `spec`
    /// (`diff <spec> --no-color --no-ext-diff -M`) — stable machine output, returned
    /// **verbatim** (a trailing blank context line is preserved, so the last hunk
    /// stays in sync with its `@@` line count for a re-parse/re-apply).
    async fn diff_text(&self, dir: &Path, spec: DiffSpec) -> Result<String>;
    /// Parsed per-file unified diff for `spec`, layered on [`diff_text`](GitApi::diff_text).
    async fn diff(&self, dir: &Path, spec: DiffSpec) -> Result<Vec<FileDiff>>;

    // --- In-progress state ---------------------------------------------------

    /// Whether the index has no staged changes (`diff --cached --quiet`).
    async fn staged_is_empty(&self, dir: &Path) -> Result<bool>;
    /// Whether a rebase is in progress (a `rebase-merge`/`rebase-apply` dir exists
    /// under the git dir).
    async fn is_rebase_in_progress(&self, dir: &Path) -> Result<bool>;
    /// Whether a merge is in progress (a `MERGE_HEAD` exists under the git dir).
    async fn is_merge_in_progress(&self, dir: &Path) -> Result<bool>;

    // --- Mutations -----------------------------------------------------------

    /// Fetch from the default remote (`fetch --quiet`), with `GIT_TERMINAL_PROMPT=0`.
    /// Transient (network) failures are retried (3 attempts, 500 ms backoff).
    async fn fetch(&self, dir: &Path) -> Result<()>;
    /// Fetch from a *named* remote (`fetch --quiet <remote>`), with
    /// `GIT_TERMINAL_PROMPT=0`. Transient failures are retried like
    /// [`fetch`](GitApi::fetch).
    async fn fetch_from(&self, dir: &Path, remote: &str) -> Result<()>;
    /// Fetch a single branch from `origin` into its remote-tracking ref
    /// (`fetch --quiet origin refs/heads/<b>:refs/remotes/origin/<b>`), with
    /// `GIT_TERMINAL_PROMPT=0`. Transient failures are retried (3×, 500 ms).
    async fn fetch_branch(&self, dir: &Path, branch: &str) -> Result<()>;
    /// Push to a remote (`push [-u] <remote> <refspec>`); see [`GitPush`].
    async fn push(&self, dir: &Path, spec: GitPush) -> Result<()>;
    /// Stage a branch's changes without committing (`merge --squash <branch>`).
    async fn merge_squash(&self, dir: &Path, branch: &str) -> Result<()>;
    /// Merge a branch (`merge [--no-ff] [-m <msg> | --no-edit] <branch>`); with no
    /// message it takes the default merge message non-interactively (`--no-edit`).
    /// See [`MergeCommit`].
    async fn merge_commit(&self, dir: &Path, spec: MergeCommit) -> Result<()>;
    /// Merge a branch but stop before committing, so the result can be inspected
    /// (`merge --no-commit [--squash | --no-ff] <branch>`). With `no_ff` (and not
    /// `squash`) git records `MERGE_HEAD`, so the in-progress merge is abortable
    /// via [`merge_abort`](GitApi::merge_abort) — the dry-run pattern. With
    /// `squash`, git stages the squashed result but records **no** `MERGE_HEAD`,
    /// so it is *not* an abortable merge: undo it with
    /// [`reset_merge`](GitApi::reset_merge) / [`reset_hard`](GitApi::reset_hard),
    /// not `merge_abort`. See [`MergeNoCommit`].
    async fn merge_no_commit(&self, dir: &Path, spec: MergeNoCommit) -> Result<()>;
    /// Abort an in-progress merge (`merge --abort`).
    async fn merge_abort(&self, dir: &Path) -> Result<()>;
    /// Finish a merge after resolving conflicts (`commit --no-edit`).
    async fn merge_continue(&self, dir: &Path) -> Result<()>;
    /// Undo an in-progress (or just-staged) merge: `reset --merge` resets the
    /// index and the merge-touched working-tree files back to `HEAD` and drops
    /// `MERGE_HEAD`, **discarding the merge's changes** while keeping unrelated
    /// unstaged edits. Use it after `merge_squash` / `merge_no_commit(squash)`,
    /// where there is no `MERGE_HEAD` for `merge_abort` to act on.
    async fn reset_merge(&self, dir: &Path) -> Result<()>;
    /// Hard-reset the working tree to a revision (`reset --hard <rev>`).
    async fn reset_hard(&self, dir: &Path, rev: &str) -> Result<()>;
    /// Rebase the current branch onto `onto` (`rebase <onto>`); the editor is
    /// suppressed (`GIT_EDITOR=true`) so it never hangs a headless caller.
    async fn rebase(&self, dir: &Path, onto: &str) -> Result<()>;
    /// Abort an in-progress rebase (`rebase --abort`).
    async fn rebase_abort(&self, dir: &Path) -> Result<()>;
    /// Continue a rebase after resolving conflicts (`rebase --continue`); the
    /// editor is suppressed (`GIT_EDITOR=true`) so the message-confirm never hangs.
    async fn rebase_continue(&self, dir: &Path) -> Result<()>;
    /// Stash the working tree (`stash push`, `--include-untracked` when asked) —
    /// e.g. to save state before a copy-on-write restore.
    async fn stash_push(&self, dir: &Path, include_untracked: bool) -> Result<()>;
    /// Restore the most recent stash and drop it (`stash pop`).
    async fn stash_pop(&self, dir: &Path) -> Result<()>;

    // --- Worktrees -----------------------------------------------------------

    /// List worktrees (`worktree list --porcelain`).
    async fn worktree_list(&self, dir: &Path) -> Result<Vec<Worktree>>;
    /// Add a worktree (`worktree add [-b <branch>] <path> [<commitish>]`).
    async fn worktree_add(&self, dir: &Path, spec: WorktreeAdd) -> Result<()>;
    /// Remove a worktree (`worktree remove [--force] <path>`).
    async fn worktree_remove(&self, dir: &Path, path: &Path, force: bool) -> Result<()>;
    /// Move a worktree (`worktree move <from> <to>`).
    async fn worktree_move(&self, dir: &Path, from: &Path, to: &Path) -> Result<()>;
    /// Prune stale worktree admin entries (`worktree prune`).
    async fn worktree_prune(&self, dir: &Path) -> Result<()>;

    // --- Clone / tags / inspection --------------------------------------------

    /// Clone `url` into `dest` (`git clone <url> <dest>` + [`CloneSpec`] flags).
    /// Runs without a working directory — pass an **absolute** `dest`.
    async fn clone_repo(&self, url: &str, dest: &Path, spec: CloneSpec) -> Result<()>;
    /// Create a lightweight tag at `rev` (`tag <name> [<rev>]`; `None` = HEAD).
    async fn tag_create(&self, dir: &Path, name: &str, rev: Option<String>) -> Result<()>;
    /// Create an annotated tag (`tag -a <name> -m <message> [<rev>]`); see
    /// [`AnnotatedTag`].
    async fn tag_create_annotated(&self, dir: &Path, spec: AnnotatedTag) -> Result<()>;
    /// Tag names, sorted by git's default ordering (`tag --list`).
    async fn tag_list(&self, dir: &Path) -> Result<Vec<String>>;
    /// Delete a tag (`tag -d <name>`).
    async fn tag_delete(&self, dir: &Path, name: &str) -> Result<()>;
    /// A file's content at a revision (`git show <rev>:<path>`). `path` is
    /// repo-relative; backslashes are normalised to `/` (git requires it).
    /// Content is decoded **lossily** — binary files come back mangled rather
    /// than erroring — and returned **verbatim**: the blob's trailing newline(s)
    /// are preserved (not trimmed), so a read-modify-write round-trip is byte-exact.
    async fn show_file(&self, dir: &Path, rev: &str, path: &str) -> Result<String>;
    /// The value of a config key, or `None` when unset (`config --get <key>`,
    /// whose exit 1 covers both "unset" and "no such section" — git doesn't
    /// distinguish). A multi-valued key errors; read those via `run`.
    async fn config_get(&self, dir: &Path, key: &str) -> Result<Option<String>>;
    /// Set a config key in the repository's local config (`config <key> <value>`).
    ///
    /// **Trusted-input sink.** `key` is guarded against a flag-shape, but this
    /// writes whatever key/value it's given — including code-execution keys like
    /// `core.sshCommand` or `filter.<drv>.clean`. Never wire untrusted input into
    /// it; a `harden()`ed client does *not* protect against config *you* write.
    async fn config_set(&self, dir: &Path, key: &str, value: &str) -> Result<()>;
    /// Add a remote (`remote add <name> <url>`).
    async fn remote_add(&self, dir: &Path, name: &str, url: &str) -> Result<()>;
    /// Change a remote's URL (`remote set-url <name> <url>`).
    async fn remote_set_url(&self, dir: &Path, name: &str, url: &str) -> Result<()>;
    /// Per-line authorship of `path` (`blame --line-porcelain [<rev>] -- <path>`;
    /// `None` = the working tree's HEAD).
    async fn blame(&self, dir: &Path, path: &str, rev: Option<String>) -> Result<Vec<BlameLine>>;

    // --- Sequencer -------------------------------------------------------------

    /// Apply a commit onto the current branch (`cherry-pick <rev>`). A conflict
    /// surfaces as an error classified by [`is_merge_conflict`].
    async fn cherry_pick(&self, dir: &Path, rev: &str) -> Result<()>;
    /// Revert a commit with the default message (`revert --no-edit <rev>`).
    async fn revert(&self, dir: &Path, rev: &str) -> Result<()>;
    /// Skip the current patch of a paused rebase (`rebase --skip`). Mainly for
    /// the `apply` backend's "nothing to commit" stop — the default `merge`
    /// backend auto-drops emptied patches on `--continue`.
    async fn rebase_skip(&self, dir: &Path) -> Result<()>;
}

vcs_cli_support::managed_client! {
    /// The real Git client. Generic over the [`ProcessRunner`] so tests can inject a
    /// fake process executor; [`Git::new`] uses the real job-backed runner.
    ///
    /// Wraps a [`ManagedClient`](vcs_cli_support::ManagedClient): enable lock-contention retry with
    /// [`with_retry`](Git::with_retry) (opt-in; off by default).
    ///
    /// **Every** client (not just [`hardened`](Git::hardened)) scrubs the inherited
    /// repo-**redirector** environment variables below, so a `GIT_DIR` (etc.) leaking
    /// from the parent process — e.g. running inside a git hook, which exports
    /// `GIT_DIR`/`GIT_INDEX_FILE` — can't silently redirect commands at a *different*
    /// repository than the bound `dir`. (`harden()` additionally scrubs the
    /// command-hook vars and pins hooks/fsmonitor/sshCommand off.)
    pub struct Git => BINARY, scrub_env = [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
    ]
}

impl<R: ProcessRunner> Git<R> {
    /// Retry **whole-repo lock-contention** failures (another process holds the
    /// repo's `index.lock`) per `policy` — opt-in, off by default. Safe even for
    /// mutating commands: that lock is acquired before any write, so a failure is
    /// pre-execution (git never ran) and a retry can't double-apply. Per-ref lock
    /// failures are *not* retried (a multi-ref op can fail a ref lock mid-way). See
    /// [`RetryPolicy`] and [`is_lock_contention`].
    pub fn with_retry(mut self, policy: RetryPolicy) -> Self {
        self.core = self.core.with_retry(policy);
        self
    }

    /// Supply credentials for **HTTPS** remote operations (`fetch`/`push`/`clone`/
    /// `ls-remote`) via a [`CredentialProvider`] — opt-in, off by default (ambient
    /// git credential helpers / SSH agent). When the provider yields a credential,
    /// each remote op runs with an inline `credential.helper` that feeds the secret
    /// from an environment variable, so the token never appears in `argv`. Local
    /// operations are unaffected. This covers HTTPS only — an **SSH** remote ignores
    /// the helper and authenticates via the ambient SSH agent, as before.
    #[must_use]
    pub fn with_credentials(mut self, provider: Arc<dyn CredentialProvider>) -> Self {
        self.core = self.core.with_credentials(provider);
        self
    }

    /// Convenience for the common case: authenticate HTTPS remotes with a single
    /// static `token` (a personal-access token; the default username
    /// `x-access-token` is used). Shorthand for
    /// `with_credentials(Arc::new(StaticCredential::token(token)))`. For a specific
    /// username, build a [`Credential::userpass`] and use
    /// [`with_credentials`](Git::with_credentials).
    #[must_use]
    pub fn with_token(self, token: impl Into<Secret>) -> Self {
        self.with_credentials(Arc::new(StaticCredential::token(token)))
    }

    /// Convenience: read the HTTPS token from environment variable `var` at request
    /// time; if `var` is unset/empty, fall back to ambient auth. Shorthand for
    /// `with_credentials(Arc::new(EnvToken::new(var)))`.
    #[must_use]
    pub fn with_env_token(self, var: impl Into<String>) -> Self {
        self.with_credentials(Arc::new(EnvToken::new(var)))
    }

    /// Resolve HTTPS credentials for a remote op into the leading `-c` config args
    /// (an inline `credential.helper`) and the secret env to set on the command.
    /// Both are empty when no provider is configured — ambient git auth, unchanged.
    /// The secret lives only in the returned env, never in the args.
    ///
    /// `expect_host` scopes the helper to a host (the secret is released only for
    /// that host, so a redirect/submodule to another host can't extract it).
    /// Callers that know the operation's target host — e.g. `clone` from its URL —
    /// pass it; the others pass `None` (the helper is ungated, as before).
    async fn remote_credentials(
        &self,
        expect_host: Option<&str>,
    ) -> Result<(Vec<String>, Vec<(String, Secret)>)> {
        match self
            .core
            .resolve_credential(CredentialService::Git, None)
            .await?
        {
            Some(cred) => {
                let helper = git_credential_helper(&cred, expect_host);
                Ok((helper.config_args, helper.env))
            }
            None => Ok((Vec::new(), Vec::new())),
        }
    }
}

/// Set each secret environment variable on `cmd` (the values from
/// [`Git::remote_credentials`]). A no-op when `envs` is empty.
fn apply_secret_env(cmd: Command, envs: &[(String, Secret)]) -> Command {
    envs.iter()
        .fold(cmd, |cmd, (name, value)| cmd.env(name, value.expose()))
}

#[async_trait::async_trait]
impl<R: ProcessRunner> GitApi for Git<R> {
    async fn run(&self, args: &[String]) -> Result<String> {
        self.core.run(args).await
    }

    async fn run_raw(&self, args: &[String]) -> Result<ProcessResult<String>> {
        self.core.output_string(args).await
    }

    async fn version(&self) -> Result<String> {
        self.core.run(["--version"]).await
    }

    async fn capabilities(&self) -> Result<GitCapabilities> {
        let raw = self.version().await?;
        let version = parse::parse_git_version(&raw).ok_or_else(|| Error::Parse {
            program: BINARY.to_string(),
            message: format!("unrecognisable `git --version` output: {raw:?}"),
        })?;
        Ok(GitCapabilities { version })
    }

    async fn status(&self, dir: &Path) -> Result<Vec<StatusEntry>> {
        self.core
            .parse(
                self.core
                    .command_in(dir, ["status", "--porcelain=v1", "-z"]),
                parse::parse_porcelain,
            )
            .await
    }

    async fn status_text(&self, dir: &Path) -> Result<String> {
        self.core
            .run(self.core.command_in(dir, ["status", "--porcelain=v1"]))
            .await
    }

    async fn branch_status(&self, dir: &Path) -> Result<BranchStatus> {
        // `GIT_OPTIONAL_LOCKS=0`: skip the opportunistic index refresh-write a
        // `status` may otherwise persist. This is the snapshot/poll primitive —
        // a filesystem watcher re-querying through it must not have the query
        // itself dirty `.git/index` and re-trigger the watch (verified: with
        // optional locks off, a re-query writes nothing).
        self.core
            .parse(
                self.core
                    .command_in(dir, ["status", "--porcelain=v2", "--branch", "-z"])
                    .env("GIT_OPTIONAL_LOCKS", "0"),
                parse::parse_porcelain_v2,
            )
            .await
    }

    async fn status_tracked(&self, dir: &Path) -> Result<Vec<StatusEntry>> {
        self.core
            .parse(
                self.core.command_in(
                    dir,
                    ["status", "--porcelain=v1", "-z", "--untracked-files=no"],
                ),
                parse::parse_porcelain,
            )
            .await
    }

    async fn conflicted_files(&self, dir: &Path) -> Result<Vec<String>> {
        // `-z` keeps special-character paths literal (no C-style quoting).
        self.core
            .parse(
                self.core
                    .command_in(dir, ["diff", "--name-only", "--diff-filter=U", "-z"]),
                parse::parse_nul_paths,
            )
            .await
    }

    async fn current_branch(&self, dir: &Path) -> Result<Option<String>> {
        // `symbolic-ref --quiet --short HEAD` is the one command that answers all
        // three head states correctly in a single spawn: it prints the branch name
        // (exit 0) for a normal **and an unborn** branch (a fresh `init`/`clone`
        // before the first commit — where `rev-parse --abbrev-ref HEAD` instead
        // *errors* with exit 128), and `--quiet` makes a detached HEAD a silent
        // exit 1 (HEAD isn't a symbolic ref) rather than a `fatal:`. So map exit
        // 0 → `Some(branch)`, exit 1 → `None` (detached), and anything else (e.g.
        // not a repository, exit 128) stays a real error.
        let res = self
            .core
            .output_string(
                self.core
                    .command_in(dir, ["symbolic-ref", "--quiet", "--short", "HEAD"]),
            )
            .await?;
        match res.code() {
            Some(0) => Ok(Some(res.stdout().trim().to_string())),
            Some(1) => Ok(None), // detached HEAD: no named branch
            _ => {
                let _ = res.ensure_success()?;
                Ok(None) // unreachable: a non-zero exit always errors above
            }
        }
    }

    async fn branches(&self, dir: &Path) -> Result<Vec<Branch>> {
        // `--no-column` + `--no-color`: `column.ui = always` would columnate
        // several names onto one line and `color.{ui,branch} = always` would inject
        // ANSI escapes — both even when piped, corrupting the line parser and the
        // returned names.
        self.core
            .parse(
                self.core
                    .command_in(dir, ["branch", "--no-column", "--no-color"]),
                parse::parse_branches,
            )
            .await
    }

    async fn log(&self, dir: &Path, revspec: &str, max: usize) -> Result<Vec<Commit>> {
        reject_flag_like("revspec", revspec)?;
        let n = format!("-n{max}");
        self.core
            .parse(
                self.core.command_in(
                    dir,
                    [
                        "log",
                        revspec,
                        n.as_str(),
                        "-z",
                        "--format=%H%x1f%h%x1f%an%x1f%aI%x1f%s",
                    ],
                ),
                parse::parse_log,
            )
            .await
    }

    async fn rev_parse(&self, dir: &Path, rev: &str) -> Result<String> {
        reject_flag_like("revision", rev)?;
        // `--verify`: without it, `git rev-parse Makefile` echoes the *filename* back
        // as a fake object id (exit 0), so a caller resolving an untrusted revision
        // could get a non-hash. `--verify` requires `rev` to name exactly one object,
        // erroring otherwise — a valid revision still resolves to the same full hash
        // (M13; matches `rev_parse_short`/`resolve_commit`, which already `--verify`).
        self.core
            .run(self.core.command_in(dir, ["rev-parse", "--verify", rev]))
            .await
    }

    async fn rev_parse_short(&self, dir: &Path, rev: &str) -> Result<String> {
        reject_flag_like("revision", rev)?;
        self.core
            .run(self.core.command_in(dir, ["rev-parse", "--short", rev]))
            .await
    }

    async fn init(&self, dir: &Path) -> Result<()> {
        self.core
            .run_unit(self.core.command_in(dir, ["init"]))
            .await
    }

    async fn add(&self, dir: &Path, paths: &[PathBuf]) -> Result<()> {
        // `--` separates the pathspecs so a path can never be read as an option.
        let mut command = self.core.command_in(dir, ["add", "--"]);
        for path in paths {
            command = command.arg(path);
        }
        self.core.run_unit(command).await
    }

    async fn commit(&self, dir: &Path, message: &str) -> Result<()> {
        // C locale: a failure's output feeds `is_nothing_to_commit`.
        self.core
            .run_unit(c_locale(
                self.core.command_in(dir, ["commit", "-m", message]),
            ))
            .await
    }

    async fn create_branch(&self, dir: &Path, name: &str) -> Result<()> {
        reject_flag_like("branch name", name)?;
        self.core
            .run_unit(self.core.command_in(dir, ["branch", name]))
            .await
    }

    async fn checkout(&self, dir: &Path, reference: &str) -> Result<()> {
        reject_flag_like("reference", reference)?;
        // The trailing `--` marks the end of revisions with no pathspecs
        // following, so git resolves `reference` as a ref *only*. Without it a
        // `reference` that doesn't name a ref but names a tracked path silently
        // falls into pathspec mode and restores that path from the index,
        // discarding unstaged edits (verified: `git checkout notes.txt` →
        // "Updated 1 path", exit 0; `git checkout notes.txt --` → hard error).
        self.core
            .run_unit(self.core.command_in(dir, ["checkout", reference, "--"]))
            .await
    }

    async fn checkout_detach(&self, dir: &Path, commit: &str) -> Result<()> {
        reject_flag_like("commit", commit)?;
        self.core
            .run_unit(self.core.command_in(dir, ["checkout", "--detach", commit]))
            .await
    }

    async fn commit_paths(&self, dir: &Path, spec: CommitPaths) -> Result<()> {
        // `--only -- <paths>` commits exactly these paths' working-tree content
        // regardless of the index; `--` keeps a path from being read as an option.
        // C locale: a failure's output feeds `is_nothing_to_commit`.
        let mut command = c_locale(self.core.command_in(dir, ["commit"]));
        if spec.amend {
            command = command.arg("--amend");
        }
        command = command.arg("-m").arg(spec.message).arg("--only").arg("--");
        for path in &spec.paths {
            command = command.arg(path);
        }
        self.core.run_unit(command).await
    }

    async fn last_commit_message(&self, dir: &Path) -> Result<String> {
        self.core
            .run(self.core.command_in(dir, ["log", "-1", "--format=%B"]))
            .await
    }

    async fn is_unborn(&self, dir: &Path) -> Result<bool> {
        // `rev-parse --verify -q HEAD` resolves HEAD quietly: 0 = a commit exists
        // (not unborn), 1 = no commit yet (unborn). `probe` maps those to a bool
        // and surfaces anything else (e.g. 128, not a repo) as `Error::Exit`.
        Ok(!self
            .core
            .probe(
                self.core
                    .command_in(dir, ["rev-parse", "--verify", "-q", "HEAD"]),
            )
            .await?)
    }

    async fn diff_is_empty(&self, dir: &Path) -> Result<bool> {
        // `git diff --quiet` is an exit-code answer: 0 = clean (empty), 1 = dirty;
        // `probe` errors on any other code / timeout / signal.
        self.core
            .probe(self.core.command_in(dir, ["diff", "--quiet"]))
            .await
    }

    async fn common_dir(&self, dir: &Path) -> Result<PathBuf> {
        Ok(PathBuf::from(
            self.core
                .run(self.core.command_in(dir, ["rev-parse", "--git-common-dir"]))
                .await?,
        ))
    }

    async fn git_dir(&self, dir: &Path) -> Result<PathBuf> {
        Ok(PathBuf::from(
            self.core
                .run(self.core.command_in(dir, ["rev-parse", "--git-dir"]))
                .await?,
        ))
    }

    async fn resolve_commit(&self, dir: &Path, rev: &str) -> Result<String> {
        reject_flag_like("revision", rev)?;
        // `^{commit}` peels an annotated tag down to the commit it points at.
        let spec = format!("{rev}^{{commit}}");
        self.core
            .run(
                self.core
                    .command_in(dir, ["rev-parse", "--verify", spec.as_str()]),
            )
            .await
    }

    async fn remote_head_branch(&self, dir: &Path) -> Result<Option<String>> {
        // `--quiet` makes an *unset* origin/HEAD a silent **exit 1** (no `fatal:`
        // on stderr); that's "no default branch", not an error. Map exit 0 → the
        // branch, exit 1 → `None`, and anything else (a real failure like "not a
        // repository" exit 128, or a timeout/signal with no exit code) surfaces via
        // `ensure_success` — mirroring `config_get`, rather than swallowing it.
        let res = self
            .core
            .output_string(
                self.core
                    .command_in(dir, ["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"]),
            )
            .await?;
        match res.code() {
            Some(0) => {
                // "refs/remotes/origin/main" → "main"; strip the whole ref prefix so
                // a slashed default branch (e.g. "release/v2") survives intact.
                let out = res.stdout().trim();
                Ok(Some(
                    out.strip_prefix("refs/remotes/origin/")
                        .unwrap_or(out)
                        .to_string(),
                ))
            }
            Some(1) => Ok(None), // unset origin/HEAD
            _ => {
                let _ = res.ensure_success()?;
                Ok(None) // unreachable: a non-zero/no-code exit always errors above
            }
        }
    }

    async fn branch_exists(&self, dir: &Path, name: &str) -> Result<bool> {
        let refname = format!("refs/heads/{name}");
        // `show-ref --verify --quiet` is an exit-code answer: 0 = exists, 1 = not.
        self.core
            .probe(
                self.core
                    .command_in(dir, ["show-ref", "--verify", "--quiet", refname.as_str()]),
            )
            .await
    }

    async fn remote_branch_exists(&self, dir: &Path, name: &str) -> Result<bool> {
        // No credential prompt, bounded wait: a missing helper or a flaky network
        // must not hang the call. `output_string` reports a timeout as a flagged result
        // (non-zero exit) rather than erroring, so an unreachable remote reads as
        // "absent" (`false`) — the best-effort answer a probe wants. A genuine
        // spawn failure (no `git`) still surfaces as an error.
        //
        // Query the *fully-qualified* ref: `ls-remote origin <name>` tail-matches
        // path components, so a bare `foo` would also match `refs/heads/bar/foo`.
        // `refs/heads/<name>` matches only the exact branch.
        let refname = format!("refs/heads/{name}");
        let (pre, envs) = self.remote_credentials(None).await?;
        let mut args: Vec<String> = pre;
        args.extend(["ls-remote", "origin", refname.as_str()].map(String::from));
        let cmd = apply_secret_env(
            self.core
                .command_in(dir, &args)
                .env("GIT_TERMINAL_PROMPT", "0")
                .timeout(Duration::from_secs(10)),
            &envs,
        );
        let res = self.core.output_string(cmd).await?;
        Ok(res.code() == Some(0) && !res.stdout().trim().is_empty())
    }

    async fn remote_url(&self, dir: &Path, remote: &str) -> Result<String> {
        reject_flag_like("remote name", remote)?;
        self.core
            .run(self.core.command_in(dir, ["remote", "get-url", remote]))
            .await
    }

    async fn upstream(&self, dir: &Path) -> Result<Option<String>> {
        // `@{u}` resolves the configured upstream; with no upstream the command
        // exits **128** — but so does a genuine failure (detached HEAD, not a repo),
        // and git gives them all the same exit code, so a *non-zero exit* maps to
        // `None` (the documented "no upstream"). A **timeout/signal** (no exit code
        // at all), however, is a real failure and must surface — not be reported as
        // "no upstream" — so it goes through `ensure_success` like the other
        // exit-code-mapping sites.
        let res = self
            .core
            .output_string(self.core.command_in(
                dir,
                ["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
            ))
            .await?;
        match res.code() {
            Some(0) => {
                let name = res.stdout().trim();
                Ok((!name.is_empty()).then(|| name.to_string()))
            }
            Some(_) => Ok(None), // any non-zero exit ⇒ no upstream configured
            None => {
                let _ = res.ensure_success()?; // timeout/signal ⇒ a real error
                Ok(None) // unreachable: ensure_success errors on a no-code outcome
            }
        }
    }

    async fn remote_branches(&self, dir: &Path, remote: &str) -> Result<Vec<String>> {
        reject_flag_like("remote name", remote)?;
        // `GIT_TERMINAL_PROMPT=0`: a remote needing credentials must fail fast,
        // never block on an interactive auth prompt. A provider, if set, supplies
        // the credential via an inline helper (token kept out of argv).
        let (pre, envs) = self.remote_credentials(None).await?;
        let mut args: Vec<String> = pre;
        args.extend(["ls-remote", "--heads", remote].map(String::from));
        let cmd = apply_secret_env(
            self.core
                .command_in(dir, &args)
                .env("GIT_TERMINAL_PROMPT", "0"),
            &envs,
        );
        self.core.parse(cmd, parse::parse_ls_remote_heads).await
    }

    async fn is_merged(&self, dir: &Path, branch: &str, target: &str) -> Result<bool> {
        reject_flag_like("branch", branch)?;
        reject_flag_like("target", target)?;
        // `--no-column` + `--no-color`: under `column.ui = always` git would pack
        // several names per line and under `color.{ui,branch} = always` it would
        // inject ANSI escapes — both even when piped, so the marker-stripping
        // compare below would never match (a false "not merged").
        let out = self
            .core
            .run(self.core.command_in(
                dir,
                ["branch", "--merged", target, "--no-column", "--no-color"],
            ))
            .await?;
        // Each line is a fixed 2-column marker (`  `/`* `/`+ `) then the name;
        // drop exactly those two columns rather than trimming a char class (which
        // would over-strip a name that legitimately began with the marker char).
        Ok(out
            .lines()
            .filter_map(|line| line.get(2..))
            .any(|b| b == branch))
    }

    async fn set_upstream(&self, dir: &Path, branch: &str, upstream: &str) -> Result<()> {
        reject_flag_like("branch name", branch)?;
        let flag = format!("--set-upstream-to={upstream}");
        self.core
            .run_unit(self.core.command_in(dir, ["branch", flag.as_str(), branch]))
            .await
    }

    async fn delete_branch(&self, dir: &Path, name: &str, force: bool) -> Result<()> {
        reject_flag_like("branch name", name)?;
        let flag = if force { "-D" } else { "-d" };
        self.core
            .run_unit(self.core.command_in(dir, ["branch", flag, name]))
            .await
    }

    async fn rename_branch(&self, dir: &Path, old: &str, new: &str) -> Result<()> {
        reject_flag_like("branch name", old)?;
        reject_flag_like("branch name", new)?;
        self.core
            .run_unit(self.core.command_in(dir, ["branch", "-m", old, new]))
            .await
    }

    async fn rev_list_count(&self, dir: &Path, range: &str) -> Result<usize> {
        reject_flag_like("range", range)?;
        self.core
            .try_parse(
                self.core.command_in(dir, ["rev-list", "--count", range]),
                |s| {
                    s.trim().parse::<usize>().map_err(|e| Error::Parse {
                        program: BINARY.to_string(),
                        message: e.to_string(),
                    })
                },
            )
            .await
    }

    async fn diff_range_is_empty(&self, dir: &Path, range: &str) -> Result<bool> {
        reject_flag_like("range", range)?;
        // `diff --quiet <range>`: 0 = empty range, 1 = has changes.
        self.core
            .probe(self.core.command_in(dir, ["diff", "--quiet", range]))
            .await
    }

    async fn diff_stat(&self, dir: &Path, range: &str) -> Result<DiffStat> {
        reject_flag_like("range", range)?;
        // `LC_ALL=C`: git's `--shortstat` summary ("N file(s) changed, …") is
        // gettext-translated, but `parse_shortstat` keys on the English
        // "file"/"insertion"/"deletion" — without C locale a non-English git
        // returns an all-zero `DiffStat` rather than the real counts.
        self.core
            .parse(
                c_locale(self.core.command_in(dir, ["diff", "--shortstat", range])),
                parse::parse_shortstat,
            )
            .await
    }

    async fn diff_text(&self, dir: &Path, spec: DiffSpec) -> Result<String> {
        // The target is a single positional arg: `HEAD` for the working tree, or
        // the caller's revision/range. `-M` enables rename detection; `--no-color`
        // / `--no-ext-diff` keep the output stable and machine-parseable.
        let target = match spec {
            DiffSpec::WorkingTree => {
                // On an unborn repo `HEAD` doesn't resolve (`git diff HEAD` errors);
                // diff against the empty tree so a pre-first-commit working tree
                // still yields its additions instead of a hard failure.
                if self.is_unborn(dir).await? {
                    EMPTY_TREE.to_string()
                } else {
                    "HEAD".to_string()
                }
            }
            DiffSpec::Rev(rev) => {
                reject_flag_like("revision", &rev)?;
                rev
            }
        };
        // The explicit prefixes pin the `a/`…`b/` form the shared parser extracts
        // paths from — a user's `diff.noprefix` / `diff.mnemonicPrefix` config
        // would otherwise change the headers and make every file silently vanish
        // from the parse. (Command-line prefixes override both config options.)
        // `run_untrimmed`: trimming the diff would drop a trailing blank context
        // line, desyncing the last hunk from its `@@` line count for a consumer
        // that re-applies or re-parses it (H7).
        self.core
            .run_untrimmed(self.core.command_in(
                dir,
                [
                    "diff",
                    target.as_str(),
                    "--no-color",
                    "--no-ext-diff",
                    "-M",
                    "--src-prefix=a/",
                    "--dst-prefix=b/",
                ],
            ))
            .await
    }

    async fn diff(&self, dir: &Path, spec: DiffSpec) -> Result<Vec<FileDiff>> {
        let text = self.diff_text(dir, spec).await?;
        Ok(parse_diff(&text))
    }

    async fn staged_is_empty(&self, dir: &Path) -> Result<bool> {
        // `diff --cached --quiet`: 0 = nothing staged, 1 = staged changes.
        self.core
            .probe(self.core.command_in(dir, ["diff", "--cached", "--quiet"]))
            .await
    }

    async fn is_rebase_in_progress(&self, dir: &Path) -> Result<bool> {
        let git_dir = self.resolved_git_dir(dir).await?;
        Ok(git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists())
    }

    async fn is_merge_in_progress(&self, dir: &Path) -> Result<bool> {
        Ok(self
            .resolved_git_dir(dir)
            .await?
            .join("MERGE_HEAD")
            .exists())
    }

    async fn fetch(&self, dir: &Path) -> Result<()> {
        // `GIT_TERMINAL_PROMPT=0` so a remote needing credentials fails fast
        // rather than blocking on an interactive prompt — matching the other
        // remote ops (`fetch_branch`, `push`, `remote_branch_exists`).
        // Fetch is idempotent, so `retry` replays it on a transient failure
        // (DNS/timeout/dropped connection); a non-transient error fails at once.
        // C locale: the retry decision classifies the failure's message.
        // Leading `-c` credential.helper (+ secret env) when a provider is set.
        let (pre, envs) = self.remote_credentials(None).await?;
        let mut args: Vec<String> = pre;
        args.extend(["fetch", "--quiet"].map(String::from));
        let cmd = apply_secret_env(
            c_locale(self.core.command_in(dir, &args))
                .env("GIT_TERMINAL_PROMPT", "0")
                // On a per-client timeout, terminate gracefully (then hard-kill
                // after a grace window) so a timed-out fetch closes cleanly.
                .timeout_grace(FETCH_TIMEOUT_GRACE)
                .retry(FETCH_ATTEMPTS, FETCH_BACKOFF, is_transient_fetch_error),
            &envs,
        );
        self.core.run_unit(cmd).await
    }

    async fn fetch_from(&self, dir: &Path, remote: &str) -> Result<()> {
        // A leading-`-` remote is a bare positional here — and a flag like
        // `--upload-pack=<cmd>` would run an arbitrary local program for a
        // local/ext transport, so this guard is load-bearing for security.
        reject_flag_like("remote", remote)?;
        // Same containment as `fetch` (prompt off, C locale, transient retry,
        // optional credential helper), with the remote named explicitly.
        let (pre, envs) = self.remote_credentials(None).await?;
        let mut args: Vec<String> = pre;
        args.extend(["fetch", "--quiet", remote].map(String::from));
        let cmd = apply_secret_env(
            c_locale(self.core.command_in(dir, &args))
                .env("GIT_TERMINAL_PROMPT", "0")
                .timeout_grace(FETCH_TIMEOUT_GRACE)
                .retry(FETCH_ATTEMPTS, FETCH_BACKOFF, is_transient_fetch_error),
            &envs,
        );
        self.core.run_unit(cmd).await
    }

    async fn fetch_branch(&self, dir: &Path, branch: &str) -> Result<()> {
        let refspec = format!("refs/heads/{branch}:refs/remotes/origin/{branch}");
        let (pre, envs) = self.remote_credentials(None).await?;
        let mut args: Vec<String> = pre;
        args.extend(["fetch", "--quiet", "origin", refspec.as_str()].map(String::from));
        let cmd = apply_secret_env(
            c_locale(self.core.command_in(dir, &args))
                .env("GIT_TERMINAL_PROMPT", "0")
                .timeout_grace(FETCH_TIMEOUT_GRACE)
                .retry(FETCH_ATTEMPTS, FETCH_BACKOFF, is_transient_fetch_error),
            &envs,
        );
        self.core.run_unit(cmd).await
    }

    async fn push(&self, dir: &Path, spec: GitPush) -> Result<()> {
        reject_flag_like("remote", &spec.remote)?;
        reject_flag_like("refspec", &spec.refspec)?;
        // M16: `reject_flag_like` catches a leading `-`/empty/NUL, but not the refspec
        // metacharacters that silently change what a push *does* — a leading `+`
        // (force-push, overwriting the remote non-fast-forward) or an extra `:` (push
        // to an unexpected remote ref). A valid refspec here is `branch` or
        // `local:remote_branch` (the single `:` is API-constructed by
        // `GitPush::refspec`), so allow at most one `:` and no leading `+` on either
        // side. A caller who genuinely needs a force-push must do it explicitly via
        // `run(["push", "--force", …])`, not smuggle a `+` through a branch name.
        let sides: Vec<&str> = spec.refspec.split(':').collect();
        if sides.len() > 2 || sides.iter().any(|s| s.starts_with('+')) {
            return Err(processkit::Error::Spawn {
                program: BINARY.to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "push refspec {:?} contains a force (`+`) or multi-ref (`:`) \
                         metacharacter — pass a plain branch or `local:remote`, or use \
                         `run([\"push\", …])` for a force-push",
                        spec.refspec
                    ),
                ),
            });
        }
        let (pre, envs) = self.remote_credentials(None).await?;
        let mut args: Vec<String> = pre;
        args.push("push".to_string());
        if spec.set_upstream {
            args.push("-u".to_string());
        }
        args.push(spec.remote.clone());
        args.push(spec.refspec.clone());
        let cmd = apply_secret_env(
            self.core
                .command_in(dir, &args)
                .env("GIT_TERMINAL_PROMPT", "0")
                // On a per-client timeout, terminate gracefully (then hard-kill
                // after a grace window) so a timed-out push releases its lock and
                // doesn't leave the remote ref half-updated. No-op without a
                // deadline (matches `fetch`).
                .timeout_grace(FETCH_TIMEOUT_GRACE),
            &envs,
        );
        self.core.run_unit(cmd).await
    }

    async fn merge_squash(&self, dir: &Path, branch: &str) -> Result<()> {
        reject_flag_like("branch", branch)?;
        // C locale: a conflict's output feeds `is_merge_conflict` (same reason as
        // `merge_commit`/`merge_no_commit`). `--squash` never commits, so no editor.
        self.core
            .run_unit(c_locale(
                self.core.command_in(dir, ["merge", "--squash", branch]),
            ))
            .await
    }

    async fn merge_commit(&self, dir: &Path, spec: MergeCommit) -> Result<()> {
        reject_flag_like("branch", &spec.branch)?;
        let mut args: Vec<&str> = vec!["merge"];
        if spec.no_ff {
            args.push("--no-ff");
        }
        if let Some(msg) = spec.message.as_deref() {
            args.push("-m");
            args.push(msg);
        } else {
            // No message → take the default merge message non-interactively
            // instead of opening `$EDITOR` (which would hang a headless caller).
            args.push("--no-edit");
        }
        args.push(&spec.branch);
        // C locale: a conflict's output feeds `is_merge_conflict`.
        self.core
            .run_unit(c_locale(self.core.command_in(dir, args)))
            .await
    }

    async fn merge_no_commit(&self, dir: &Path, spec: MergeNoCommit) -> Result<()> {
        reject_flag_like("branch", &spec.branch)?;
        let mut args: Vec<&str> = vec!["merge", "--no-commit"];
        // `--squash` and `--no-ff` are mutually exclusive (git rejects the pair);
        // a squash never fast-forwards anyway, so it takes precedence.
        if spec.squash {
            args.push("--squash");
        } else if spec.no_ff {
            args.push("--no-ff");
        }
        args.push(&spec.branch);
        // C locale: a conflict's output feeds `is_merge_conflict`.
        self.core
            .run_unit(c_locale(self.core.command_in(dir, args)))
            .await
    }

    async fn merge_abort(&self, dir: &Path) -> Result<()> {
        self.core
            .run_unit(c_locale(self.core.command_in(dir, ["merge", "--abort"])))
            .await
    }

    async fn merge_continue(&self, dir: &Path) -> Result<()> {
        // `--no-edit` already reuses the prepared MERGE_MSG; `no_editor` is a
        // headless backstop so a commit hook re-opening the editor can't hang.
        // C locale: the failure output feeds the classifiers (a still-conflicted
        // tree reports "nothing to commit"-adjacent / conflict messages).
        self.core
            .run_unit(no_editor(c_locale(
                self.core.command_in(dir, ["commit", "--no-edit"]),
            )))
            .await
    }

    async fn reset_merge(&self, dir: &Path) -> Result<()> {
        self.core
            .run_unit(self.core.command_in(dir, ["reset", "--merge"]))
            .await
    }

    async fn reset_hard(&self, dir: &Path, rev: &str) -> Result<()> {
        reject_flag_like("revision", rev)?;
        self.core
            .run_unit(self.core.command_in(dir, ["reset", "--hard", rev]))
            .await
    }

    async fn rebase(&self, dir: &Path, onto: &str) -> Result<()> {
        reject_flag_like("rebase target", onto)?;
        // Force a no-op editor so a rebase that would open `$EDITOR` (reword, or
        // the message-confirm on `--continue`) never hangs a headless caller.
        // C locale: a conflict's output feeds `is_merge_conflict`.
        self.core
            .run_unit(no_editor(c_locale(
                self.core.command_in(dir, ["rebase", onto]),
            )))
            .await
    }

    async fn rebase_abort(&self, dir: &Path) -> Result<()> {
        self.core
            .run_unit(c_locale(self.core.command_in(dir, ["rebase", "--abort"])))
            .await
    }

    async fn rebase_continue(&self, dir: &Path) -> Result<()> {
        self.core
            .run_unit(no_editor(c_locale(
                self.core.command_in(dir, ["rebase", "--continue"]),
            )))
            .await
    }

    async fn stash_push(&self, dir: &Path, include_untracked: bool) -> Result<()> {
        let mut command = self.core.command_in(dir, ["stash", "push"]);
        if include_untracked {
            command = command.arg("--include-untracked");
        }
        self.core.run_unit(command).await
    }

    async fn stash_pop(&self, dir: &Path) -> Result<()> {
        // C locale: a conflicting `stash pop` emits git's merge-machinery
        // `CONFLICT (...)` output, which feeds `is_merge_conflict` (e.g. via
        // `switch_with_stash`) — a translated message would defeat it.
        self.core
            .run_unit(c_locale(self.core.command_in(dir, ["stash", "pop"])))
            .await
    }

    async fn worktree_list(&self, dir: &Path) -> Result<Vec<Worktree>> {
        self.core
            .parse(
                self.core
                    .command_in(dir, ["worktree", "list", "--porcelain"]),
                parse::parse_worktree_porcelain,
            )
            .await
    }

    async fn worktree_add(&self, dir: &Path, spec: WorktreeAdd) -> Result<()> {
        if let Some(name) = spec.new_branch.as_deref() {
            reject_flag_like("branch name", name)?;
        }
        if let Some(commitish) = spec.commitish.as_deref() {
            reject_flag_like("commit-ish", commitish)?;
        }
        let mut command = self.core.command_in(dir, ["worktree", "add"]);
        if let Some(name) = spec.new_branch.as_deref() {
            command = command.arg("-b").arg(name);
        }
        if spec.no_checkout {
            command = command.arg("--no-checkout");
        }
        command = command.arg(&spec.path);
        if let Some(commitish) = spec.commitish.as_deref() {
            command = command.arg(commitish);
        }
        self.core.run_unit(command).await
    }

    async fn worktree_remove(&self, dir: &Path, path: &Path, force: bool) -> Result<()> {
        let mut command = self.core.command_in(dir, ["worktree", "remove"]);
        if force {
            command = command.arg("--force");
        }
        command = command.arg(path);
        self.core.run_unit(command).await
    }

    async fn worktree_move(&self, dir: &Path, from: &Path, to: &Path) -> Result<()> {
        let command = self
            .core
            .command_in(dir, ["worktree", "move"])
            .arg(from)
            .arg(to);
        self.core.run_unit(command).await
    }

    async fn worktree_prune(&self, dir: &Path) -> Result<()> {
        self.core
            .run_unit(self.core.command_in(dir, ["worktree", "prune"]))
            .await
    }

    async fn clone_repo(&self, url: &str, dest: &Path, spec: CloneSpec) -> Result<()> {
        // A leading-`-` url is a bare positional — `git clone --upload-pack=<cmd>`
        // would run an arbitrary local program. A real URL never leads with `-`,
        // so this guard has no false positives.
        reject_flag_like("url", url)?;
        // No working directory: clone creates `dest` itself, so `dest` should
        // be absolute (a relative path would resolve against this process' cwd).
        // Leading `-c` credential.helper (+ secret env) when a provider is set,
        // scoped to the clone URL's host so a cross-host redirect/submodule during
        // the clone can't extract the token (the URL is often externally supplied).
        let (pre, envs) = self
            .remote_credentials(vcs_cli_support::https_host(url).as_deref())
            .await?;
        let mut initial: Vec<String> = pre;
        initial.push("clone".to_string());
        let mut command = self.core.command(&initial);
        if let Some(branch) = spec.branch.as_deref() {
            command = command.arg("--branch").arg(branch);
        }
        if let Some(depth) = spec.depth {
            command = command.arg("--depth").arg(depth.to_string());
        }
        if spec.bare {
            command = command.arg("--bare");
        }
        let command = apply_secret_env(
            command
                .arg(url)
                .arg(dest)
                .env("GIT_TERMINAL_PROMPT", "0")
                // On a per-client timeout, terminate gracefully (then hard-kill after
                // a grace window). No-op without a deadline (matches `fetch`).
                .timeout_grace(FETCH_TIMEOUT_GRACE),
            &envs,
        );

        // R7: git populates `dest` incrementally, so a failed clone (timeout, network,
        // auth) can leave a **partial, non-empty** `dest` that blocks a retry with
        // "destination path already exists and is not empty". `timeout_grace` alone
        // can't prevent it — Windows' job-kill is atomic (no graceful tier) and the
        // Unix grace is too short to delete a multi-GB partial. So clean it ourselves.
        //
        // Only clean a `dest` we could have *created*: absent, or an empty directory.
        // git refuses to clone into a **non-empty** existing dir, so a non-empty `dest`
        // means the failure was that refusal and the caller's data is untouched — never
        // delete that. (A best-effort blocking remove on the error path; a partial clone
        // may be large, but this path is rare.)
        let cleanable = match std::fs::read_dir(dest) {
            Err(_) => true,                              // absent/unreadable → clone creates it
            Ok(mut entries) => entries.next().is_none(), // an empty directory
        };
        let result = self.core.run_unit(command).await;
        if result.is_err() && cleanable {
            let _ = std::fs::remove_dir_all(dest);
        }
        result
    }

    async fn tag_create(&self, dir: &Path, name: &str, rev: Option<String>) -> Result<()> {
        reject_flag_like("tag name", name)?;
        if let Some(rev) = rev.as_deref() {
            reject_flag_like("revision", rev)?;
        }
        let mut args = vec!["tag", name];
        if let Some(rev) = rev.as_deref() {
            args.push(rev);
        }
        self.core.run_unit(self.core.command_in(dir, args)).await
    }

    async fn tag_create_annotated(&self, dir: &Path, spec: AnnotatedTag) -> Result<()> {
        reject_flag_like("tag name", &spec.name)?;
        if let Some(rev) = spec.rev.as_deref() {
            reject_flag_like("revision", rev)?;
        }
        let mut args = vec!["tag", "-a", &spec.name, "-m", &spec.message];
        if let Some(rev) = spec.rev.as_deref() {
            args.push(rev);
        }
        self.core.run_unit(self.core.command_in(dir, args)).await
    }

    async fn tag_list(&self, dir: &Path) -> Result<Vec<String>> {
        // `--no-column`: a user's `column.ui = always` would pack several tags
        // onto one line even when piped, corrupting the one-per-line split.
        let out = self
            .core
            .run(self.core.command_in(dir, ["tag", "--list", "--no-column"]))
            .await?;
        Ok(out.lines().map(str::to_string).collect())
    }

    async fn tag_delete(&self, dir: &Path, name: &str) -> Result<()> {
        reject_flag_like("tag name", name)?;
        self.core
            .run_unit(self.core.command_in(dir, ["tag", "-d", name]))
            .await
    }

    async fn show_file(&self, dir: &Path, rev: &str, path: &str) -> Result<String> {
        // A leading-`-` rev makes the whole `<rev>:<path>` token start with `-`,
        // so git would parse it as a flag — guard it before building the spec.
        reject_flag_like("revision", rev)?;
        // git rejects backslash separators in the `<rev>:<path>` spec ("exists
        // on disk, but not in <rev>") — normalise for Windows callers. Only on
        // Windows: on Unix a backslash is a legal filename byte, and rewriting
        // it would make a literal `a\b.txt` unresolvable.
        #[cfg(windows)]
        let path = path.replace('\\', "/");
        let spec = format!("{rev}:{path}");
        // `run_untrimmed`: a blob's trailing newline(s) are part of its content —
        // trimming them corrupts a read-modify-write round-trip (H7).
        self.core
            .run_untrimmed(self.core.command_in(dir, ["show", spec.as_str()]))
            .await
    }

    async fn config_get(&self, dir: &Path, key: &str) -> Result<Option<String>> {
        reject_flag_like("config key", key)?;
        let res = self
            .core
            .output_string(self.core.command_in(dir, ["config", "--get", key]))
            .await?;
        match res.code() {
            // Exit 1 = unset (git lumps "no such key/section" in here too).
            Some(1) => Ok(None),
            // Strip only git's trailing line terminator (`\n`, or `\r\n`), not all
            // trailing whitespace: a config value can legitimately end in spaces or
            // a tab (e.g. a templated prefix), and `--get` returns a single line, so
            // it never itself ends in a newline.
            Some(0) => Ok(Some(
                res.stdout().trim_end_matches(['\r', '\n']).to_string(),
            )),
            _ => {
                let _ = res.ensure_success()?;
                Ok(None) // unreachable: a non-zero exit always errors above.
            }
        }
    }

    async fn config_set(&self, dir: &Path, key: &str, value: &str) -> Result<()> {
        reject_flag_like("config key", key)?;
        self.core
            .run_unit(self.core.command_in(dir, ["config", key, value]))
            .await
    }

    async fn remote_add(&self, dir: &Path, name: &str, url: &str) -> Result<()> {
        reject_flag_like("remote name", name)?;
        reject_flag_like("url", url)?;
        self.core
            .run_unit(self.core.command_in(dir, ["remote", "add", name, url]))
            .await
    }

    async fn remote_set_url(&self, dir: &Path, name: &str, url: &str) -> Result<()> {
        reject_flag_like("remote name", name)?;
        reject_flag_like("url", url)?;
        self.core
            .run_unit(self.core.command_in(dir, ["remote", "set-url", name, url]))
            .await
    }

    async fn blame(&self, dir: &Path, path: &str, rev: Option<String>) -> Result<Vec<BlameLine>> {
        let mut args = vec!["blame", "--line-porcelain"];
        if let Some(rev) = rev.as_deref() {
            // A standalone positional rev with a leading `-` would be any blame
            // flag (`-s`, `--reverse`, `-L…`) — guard before the `--`.
            reject_flag_like("revision", rev)?;
            args.push(rev);
        }
        args.push("--");
        args.push(path);
        self.core
            .parse(
                self.core.command_in(dir, args),
                parse::parse_blame_porcelain,
            )
            .await
    }

    async fn cherry_pick(&self, dir: &Path, rev: &str) -> Result<()> {
        reject_flag_like("revision", rev)?;
        // No editor opens non-interactively, but keep the headless backstop.
        // C locale: a conflict's output feeds `is_merge_conflict`.
        self.core
            .run_unit(no_editor(c_locale(
                self.core.command_in(dir, ["cherry-pick", rev]),
            )))
            .await
    }

    async fn revert(&self, dir: &Path, rev: &str) -> Result<()> {
        reject_flag_like("revision", rev)?;
        self.core
            .run_unit(no_editor(c_locale(
                self.core.command_in(dir, ["revert", "--no-edit", rev]),
            )))
            .await
    }

    async fn rebase_skip(&self, dir: &Path) -> Result<()> {
        self.core
            .run_unit(no_editor(c_locale(
                self.core.command_in(dir, ["rebase", "--skip"]),
            )))
            .await
    }
}

// --- Internal helpers --------------------------------------------------------
//
// The error classifiers (`is_merge_conflict`/`is_nothing_to_commit`/
// `is_transient_fetch_error`), the fetch-retry policy, and the argv injection
// guard now live in the shared `vcs-cli-support` crate (re-exported at the top of
// this module); what remains here is git-specific.

/// Git's well-known empty-tree object id — a stable stand-in for `HEAD` when
/// diffing the working tree of an unborn (no-commits-yet) repository. Public so a
/// caller can diff/stat a pre-first-commit working tree against it directly.
pub const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Total attempts / fixed backoff for a transient-retried `fetch` — the shared
/// policy from `vcs-cli-support`, aliased so the retry call sites read locally.
const FETCH_ATTEMPTS: u32 = vcs_cli_support::FETCH_ATTEMPTS;
const FETCH_BACKOFF: Duration = vcs_cli_support::FETCH_BACKOFF;
const FETCH_TIMEOUT_GRACE: Duration = vcs_cli_support::FETCH_TIMEOUT_GRACE;

/// Point git's editor at a no-op so any command that would open `$EDITOR`
/// (a rebase reword, the message-confirm on `rebase --continue`) succeeds
/// non-interactively instead of hanging a headless caller.
fn no_editor(cmd: processkit::Command) -> processkit::Command {
    cmd.env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
}

/// Force the C locale on a command whose output feeds the error classifiers
/// (`is_merge_conflict`, `is_nothing_to_commit`, `is_transient_fetch_error`):
/// they match untranslated English substrings, and a localized git would emit
/// translated messages, silently turning a classified failure (conflict /
/// clean-tree / transient) into an unclassified one.
fn c_locale(cmd: processkit::Command) -> processkit::Command {
    cmd.env("LC_ALL", "C")
}

/// Injection guard for bare positional argv slots — delegates to the shared
/// [`vcs_cli_support::reject_flag_like`], naming this crate's binary so the
/// ~45 call sites stay `reject_flag_like(what, value)`.
fn reject_flag_like(what: &str, value: &str) -> Result<()> {
    vcs_cli_support::reject_flag_like(BINARY, what, value)
}

impl<R: ProcessRunner> Git<R> {
    /// Run `git <args>` over string slices — `git.run_args(&["status", "-s"])`
    /// without allocating a `Vec<String>`. Inherent (not on the object-safe
    /// trait), so it can take `&[&str]`; forwards to the same path as
    /// [`GitApi::run`].
    pub async fn run_args(&self, args: &[&str]) -> Result<String> {
        self.core.run(args).await
    }

    /// Like [`run_args`](Git::run_args) but never errors on a non-zero exit
    /// (mirrors [`GitApi::run_raw`]).
    pub async fn run_raw_args(&self, args: &[&str]) -> Result<ProcessResult<String>> {
        self.core.output_string(args).await
    }

    /// Bind this client to `dir`, returning a [`GitAt`] handle whose methods omit
    /// the `dir` argument: `git.at(dir).status()` runs [`status`](GitApi::status)
    /// against `dir`. The dir-taking [`GitApi`] methods stay on [`Git`] for
    /// driving many directories (e.g. linked worktrees) from one client.
    pub fn at<'a>(&'a self, dir: &'a Path) -> GitAt<'a, R> {
        GitAt { git: self, dir }
    }

    /// Harden this client for driving repositories it didn't create: running
    /// `git` inside an untrusted checkout executes that repository's hooks and
    /// honours its config — arbitrary code execution by default. The profile
    /// (applied to **every** command this client runs):
    ///
    /// **⚠ Requires git ≥ 2.31.** The hook / `fsmonitor` / `sshCommand` pins ride
    /// git's env-based config (`GIT_CONFIG_COUNT`), which older git **silently
    /// ignores** — so on git < 2.31 `harden()` still scrubs the environment and
    /// turns prompts off, but repo-local hooks/fsmonitor/sshCommand are **not**
    /// disabled (no error is raised). There is **no built-in 2.31 gate yet**
    /// ([`capabilities().ensure_supported()`](GitCapabilities::ensure_supported)
    /// only checks the major version, so it passes on 2.0–2.30). Before relying on
    /// `harden()` against a fully untrusted repo on a host you don't control, check
    /// the version yourself — `Git::new().capabilities().await?.version` exposes
    /// `major`/`minor` — and require ≥ 2.31, or add an OS-level sandbox. (A
    /// machine-checked minor-version floor is tracked for a future release; see
    /// `docs/audit-2026-07.md` H3.)
    ///
    /// - **Disables hooks** — `core.hooksPath=/dev/null` pinned via git's
    ///   env-based config (`GIT_CONFIG_COUNT`/`KEY_n`/`VALUE_n`, git ≥ 2.31;
    ///   verified to suppress hooks on Windows too) — and `core.fsmonitor`
    ///   (a config-driven daemon launch). Env-config overrides even the
    ///   *repo-local* `.git/config` for the keys it names, so these pins beat a
    ///   poisoned `.git/config`.
    /// - **Neutralizes `core.sshCommand`** (pinned empty) — the config-key twin of
    ///   the scrubbed `GIT_SSH_COMMAND`, an arbitrary program git would run for the
    ///   SSH transport. Empty is falsy to git, so the default `ssh` (ambient
    ///   `~/.ssh/config`/agent) still works; only the repo's override is dropped.
    /// - **Removes inherited repo redirectors** so a poisoned parent
    ///   environment can't point commands at another repository: `GIT_DIR`,
    ///   `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_COMMON_DIR`,
    ///   `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`,
    ///   `GIT_NAMESPACE`, `GIT_CEILING_DIRECTORIES`, `GIT_CONFIG_PARAMETERS`,
    ///   `GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM`. (The first seven are also
    ///   scrubbed by *every* client — see the type-level doc — not just here.)
    /// - **Removes inherited command hooks** that make git spawn an arbitrary
    ///   program from the *environment* (a second code-execution path besides
    ///   repo hooks): `GIT_SSH_COMMAND`/`GIT_SSH` (transport), `GIT_ASKPASS`
    ///   (credential prompt), `GIT_EXTERNAL_DIFF` (diff driver), `GIT_PAGER`,
    ///   `GIT_EDITOR`/`GIT_SEQUENCE_EDITOR`, `GIT_PROXY_COMMAND` (a program for a
    ///   `git://` connection), `GIT_EXEC_PATH` (relocates git's own sub-commands),
    ///   and `GIT_TEMPLATE_DIR` (seeds hooks/config on `init`/`clone`). It also drops
    ///   the pathspec-mode vars (`GIT_LITERAL_PATHSPECS` / `GIT_GLOB_PATHSPECS` /
    ///   `GIT_NOGLOB_PATHSPECS` / `GIT_ICASE_PATHSPECS`), which silently change which
    ///   paths a command matches. The library's own auth seam
    ///   ([`with_credentials`](Git::with_credentials)) injects credentials via a
    ///   git `credential.helper` / token env, **not** these variables, so it keeps
    ///   working through a hardened client; an operator who deliberately relies on
    ///   an ambient `GIT_SSH_COMMAND`/`GIT_ASKPASS` should inject it per-call
    ///   instead of inheriting it into an untrusted-repo run.
    /// - **Skips system config** (`GIT_CONFIG_NOSYSTEM=1`) and keeps terminal
    ///   prompts off everywhere (`GIT_TERMINAL_PROMPT=0`).
    ///
    /// **Residual repo-local-config vectors (NOT neutralized).** `harden()` closes
    /// the *hooks*, `fsmonitor`, `core.sshCommand`, and the env redirector/command-
    /// hook paths — but a few **repo-local `.git/config` / `.gitattributes`** keys
    /// still run an arbitrary program and are not pinned: `filter.<drv>.clean`/
    /// `smudge` (run on any working-tree materialization — `checkout`, `stash pop`,
    /// `worktree add`), and `diff.<drv>.textconv` / `diff.external` (run when a diff
    /// is produced; [`diff_text`](GitApi::diff_text) defends itself with
    /// `--no-ext-diff`, but other diff/blame reads do not). So for a **fully
    /// untrusted** repo, do not materialize its working tree or run diffs through a
    /// hardened client without an OS-level sandbox — `harden()` is hardening, not a
    /// sandbox.
    ///
    /// What it does NOT do beyond that: sandbox the git binary itself, or stop the
    /// repo's *content* from being malicious. In a **colocated jj repo**, git hooks
    /// only run when *git* commands run — harden the `Git` client; `Jj` needs
    /// no equivalent (jj has no repo-local hooks; see the vcs-jj docs).
    ///
    /// Chainable — `Git::with_runner(rec).harden()` works in tests; use
    /// [`Git::hardened()`](Git::hardened) for the common case.
    pub fn harden(self) -> Self {
        let removed = [
            // Repo redirectors — point git at another repo/index/object store.
            // (`GIT_DIR`…`GIT_NAMESPACE` are also scrubbed by *every* client via the
            // `managed_client!` `scrub_env`; re-listed here so the hardened profile is
            // self-contained and its double-removal is harmless.)
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_NAMESPACE",
            "GIT_CEILING_DIRECTORIES",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
            // Command hooks — make git spawn an arbitrary program from the env.
            "GIT_SSH_COMMAND",
            "GIT_SSH",
            "GIT_ASKPASS",
            "GIT_EXTERNAL_DIFF",
            "GIT_PAGER",
            "GIT_EDITOR",
            "GIT_SEQUENCE_EDITOR",
            // More env command-hooks (M14): `GIT_PROXY_COMMAND` runs an arbitrary
            // program for a `git://` connection; `GIT_EXEC_PATH` relocates where git
            // finds its own sub-commands (so `git-<x>` becomes attacker-chosen);
            // `GIT_TEMPLATE_DIR` seeds hooks/config into a repo on `init`/`clone`.
            "GIT_PROXY_COMMAND",
            "GIT_EXEC_PATH",
            "GIT_TEMPLATE_DIR",
            // Pathspec interpretation (M14) — not code-execution, but they silently
            // change which paths a command matches, so pin deterministic behavior.
            "GIT_LITERAL_PATHSPECS",
            "GIT_GLOB_PATHSPECS",
            "GIT_NOGLOB_PATHSPECS",
            "GIT_ICASE_PATHSPECS",
        ];
        let mut hardened = self;
        for key in removed {
            hardened = hardened.default_env_remove(key);
        }
        hardened
            .default_env("GIT_CONFIG_NOSYSTEM", "1")
            .default_env("GIT_TERMINAL_PROMPT", "0")
            // Env-config (`GIT_CONFIG_COUNT`/`KEY_n`/`VALUE_n`) overrides even the
            // *repo-local* `.git/config` for the keys it names — so these pins beat
            // a poisoned `.git/config`, which `GIT_CONFIG_NOSYSTEM` (system) and the
            // scrubbed `GIT_CONFIG_GLOBAL` (global) do not reach.
            .default_env("GIT_CONFIG_COUNT", "3")
            .default_env("GIT_CONFIG_KEY_0", "core.hooksPath")
            // `/dev/null` as the hooks dir disables hooks on every platform,
            // Windows included: git looks for `<hooksPath>/<hook-name>`, and no
            // such file can exist under `/dev/null` (it is not a directory), so the
            // lookup always misses and no hook runs. A literal POSIX path is fine
            // on Windows here — it is used as a path *prefix* to probe, never
            // opened — and it reads unambiguously as "nowhere."
            .default_env("GIT_CONFIG_VALUE_0", "/dev/null")
            .default_env("GIT_CONFIG_KEY_1", "core.fsmonitor")
            .default_env("GIT_CONFIG_VALUE_1", "false")
            // Neutralize a repo-local `core.sshCommand` (an arbitrary program git
            // runs for the SSH transport on fetch/push/clone) — the config-key twin
            // of the scrubbed `GIT_SSH_COMMAND` env var. An empty value is falsy to
            // git, so it falls back to the default `ssh` (ambient `~/.ssh/config` /
            // agent still work); only the repo's override is dropped.
            .default_env("GIT_CONFIG_KEY_2", "core.sshCommand")
            .default_env("GIT_CONFIG_VALUE_2", "")
    }

    /// Switch to `branch`, carrying uncommitted changes (tracked *and*
    /// untracked) across via the stash: `stash push -u` → `checkout` →
    /// `stash pop`. A clean tree skips the stash round-trip entirely — a
    /// `stash push` there would save nothing and the later pop would pop an
    /// older, unrelated stash.
    ///
    /// Failure behaviour:
    /// - `checkout` fails (atomic — the working copy stays on the original
    ///   branch): the stash is popped back to restore the original state, and
    ///   the checkout error is returned. If that restoring pop *also* fails,
    ///   the changes stay safe in the stash (`git stash list`).
    /// - `stash pop` on the target branch conflicts: the error is returned with
    ///   the target branch checked out; git keeps the stash entry, so the
    ///   changes can be resolved or re-applied manually.
    ///
    /// Inherent (not on the object-safe trait): a composed operation, not a 1:1
    /// CLI verb — mock the underlying `status`/`stash_*`/`checkout` instead.
    pub async fn switch_with_stash(&self, dir: &Path, branch: &str) -> Result<()> {
        // Untracked-inclusive guard to match `stash push -u`: "dirty" must mean
        // the same thing to the guard and to the stash.
        if self.status(dir).await?.is_empty() {
            return self.checkout(dir, branch).await;
        }
        self.stash_push(dir, true).await?;
        match self.checkout(dir, branch).await {
            Ok(()) => self.stash_pop(dir).await,
            Err(err) => {
                // A failed checkout is atomic — we are still on the original
                // branch, so popping restores the exact pre-call state. If the
                // pop fails too, the stash entry is preserved for the caller.
                let _ = self.stash_pop(dir).await;
                Err(err)
            }
        }
    }

    /// `git_dir` resolved to an absolute path — `rev-parse --git-dir` may report
    /// it relative to `dir` (e.g. `.git`), which the filesystem probes need joined.
    async fn resolved_git_dir(&self, dir: &Path) -> Result<PathBuf> {
        let git_dir = PathBuf::from(
            self.core
                .run(self.core.command_in(dir, ["rev-parse", "--git-dir"]))
                .await?,
        );
        Ok(if git_dir.is_absolute() {
            git_dir
        } else {
            dir.join(git_dir)
        })
    }
}

impl Git {
    /// A hardened real (job-backed) client — `Git::new().harden()`; see
    /// [`harden`](Git::harden) for what the profile does.
    pub fn hardened() -> Self {
        Self::new().harden()
    }
}

/// A [`Git`] client with a working directory bound, so calls drop the leading
/// `dir` argument — `git.at(dir).status()` is `git.status(dir)`. Construct one
/// with [`Git::at`] (or, through the facade, `vcs_core::Repo::git_at`). Cheap to
/// copy: it only borrows the client and the path.
pub struct GitAt<'a, R: ProcessRunner = processkit::JobRunner> {
    git: &'a Git<R>,
    dir: &'a Path,
}

// Hand-written rather than derived: the view only holds two references, so it is
// `Copy` for *every* runner. `#[derive(Copy)]` would add a spurious `R: Copy`
// bound that the real default `JobRunner` doesn't satisfy, silently dropping
// `Copy` on the production `Repo::git_at()` handle.
impl<R: ProcessRunner> Clone for GitAt<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<R: ProcessRunner> Copy for GitAt<'_, R> {}

// Generate [`GitAt`] forwarders from a method list: `bare` methods forward
// verbatim, `dir` methods inject `self.dir` as the first argument. The shared
// macro lives in `vcs-cli-support` (see `vcs_cli_support::at_forwarders!`).
vcs_cli_support::at_forwarders! {
    GitAt, git, "Git",
    bare {
        fn run(args: &[String]) -> Result<String>;
        fn run_raw(args: &[String]) -> Result<ProcessResult<String>>;
        fn run_args(args: &[&str]) -> Result<String>;
        fn run_raw_args(args: &[&str]) -> Result<ProcessResult<String>>;
        fn version() -> Result<String>;
        fn capabilities() -> Result<GitCapabilities>;
        fn clone_repo(url: &str, dest: &Path, spec: CloneSpec) -> Result<()>;
    }
    dir {
        fn status() -> Result<Vec<StatusEntry>>;
        fn status_text() -> Result<String>;
        fn status_tracked() -> Result<Vec<StatusEntry>>;
        fn branch_status() -> Result<BranchStatus>;
        fn conflicted_files() -> Result<Vec<String>>;
        fn current_branch() -> Result<Option<String>>;
        fn branches() -> Result<Vec<Branch>>;
        fn log(revspec: &str, max: usize) -> Result<Vec<Commit>>;
        fn rev_parse(rev: &str) -> Result<String>;
        fn rev_parse_short(rev: &str) -> Result<String>;
        fn init() -> Result<()>;
        fn add(paths: &[PathBuf]) -> Result<()>;
        fn commit(message: &str) -> Result<()>;
        fn create_branch(name: &str) -> Result<()>;
        fn checkout(reference: &str) -> Result<()>;
        fn checkout_detach(commit: &str) -> Result<()>;
        fn commit_paths(spec: CommitPaths) -> Result<()>;
        fn last_commit_message() -> Result<String>;
        fn is_unborn() -> Result<bool>;
        fn diff_is_empty() -> Result<bool>;
        fn common_dir() -> Result<PathBuf>;
        fn git_dir() -> Result<PathBuf>;
        fn resolve_commit(rev: &str) -> Result<String>;
        fn remote_head_branch() -> Result<Option<String>>;
        fn branch_exists(name: &str) -> Result<bool>;
        fn remote_branch_exists(name: &str) -> Result<bool>;
        fn remote_url(remote: &str) -> Result<String>;
        fn upstream() -> Result<Option<String>>;
        fn remote_branches(remote: &str) -> Result<Vec<String>>;
        fn is_merged(branch: &str, target: &str) -> Result<bool>;
        fn set_upstream(branch: &str, upstream: &str) -> Result<()>;
        fn delete_branch(name: &str, force: bool) -> Result<()>;
        fn rename_branch(old: &str, new: &str) -> Result<()>;
        fn rev_list_count(range: &str) -> Result<usize>;
        fn diff_range_is_empty(range: &str) -> Result<bool>;
        fn diff_stat(range: &str) -> Result<DiffStat>;
        fn diff_text(spec: DiffSpec) -> Result<String>;
        fn diff(spec: DiffSpec) -> Result<Vec<FileDiff>>;
        fn staged_is_empty() -> Result<bool>;
        fn is_rebase_in_progress() -> Result<bool>;
        fn is_merge_in_progress() -> Result<bool>;
        fn fetch() -> Result<()>;
        fn fetch_from(remote: &str) -> Result<()>;
        fn fetch_branch(branch: &str) -> Result<()>;
        fn push(spec: GitPush) -> Result<()>;
        fn merge_squash(branch: &str) -> Result<()>;
        fn merge_commit(spec: MergeCommit) -> Result<()>;
        fn merge_no_commit(spec: MergeNoCommit) -> Result<()>;
        fn merge_abort() -> Result<()>;
        fn merge_continue() -> Result<()>;
        fn reset_merge() -> Result<()>;
        fn reset_hard(rev: &str) -> Result<()>;
        fn rebase(onto: &str) -> Result<()>;
        fn rebase_abort() -> Result<()>;
        fn rebase_continue() -> Result<()>;
        fn stash_push(include_untracked: bool) -> Result<()>;
        fn stash_pop() -> Result<()>;
        fn switch_with_stash(branch: &str) -> Result<()>;
        fn worktree_list() -> Result<Vec<Worktree>>;
        fn worktree_add(spec: WorktreeAdd) -> Result<()>;
        fn worktree_remove(path: &Path, force: bool) -> Result<()>;
        fn worktree_move(from: &Path, to: &Path) -> Result<()>;
        fn worktree_prune() -> Result<()>;
        fn tag_create(name: &str, rev: Option<String>) -> Result<()>;
        fn tag_create_annotated(spec: AnnotatedTag) -> Result<()>;
        fn tag_list() -> Result<Vec<String>>;
        fn tag_delete(name: &str) -> Result<()>;
        fn show_file(rev: &str, path: &str) -> Result<String>;
        fn config_get(key: &str) -> Result<Option<String>>;
        fn config_set(key: &str, value: &str) -> Result<()>;
        fn remote_add(name: &str, url: &str) -> Result<()>;
        fn remote_set_url(name: &str, url: &str) -> Result<()>;
        fn blame(path: &str, rev: Option<String>) -> Result<Vec<BlameLine>>;
        fn cherry_pick(rev: &str) -> Result<()>;
        fn revert(rev: &str) -> Result<()>;
        fn rebase_skip() -> Result<()>;
    }
}

/// Synchronous, best-effort helpers for contexts that cannot `.await` — chiefly
/// a `Drop` guard. They shell out through `std::process` directly (no async, no
/// job-containment), so reserve them for short-lived cleanup.
pub mod blocking {
    use std::path::Path;
    use std::process::Command;

    /// Remove a worktree synchronously (`git worktree remove [--force] <path>`).
    pub fn worktree_remove(dir: &Path, path: &Path, force: bool) -> std::io::Result<()> {
        let mut cmd = Command::new(super::BINARY);
        cmd.current_dir(dir).args(["worktree", "remove"]);
        if force {
            cmd.arg("--force");
        }
        cmd.arg(path);
        let status = cmd.status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "`git worktree remove` exited with {status}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use processkit::testing::{RecordingRunner, Reply, ScriptedRunner};

    #[test]
    fn binary_name_is_git() {
        assert_eq!(BINARY, "git");
    }

    // Compile-time guard: the bound view must stay `Copy` for the *default*
    // `JobRunner` (the production `Repo::git_at()` handle), not just for the
    // `&RecordingRunner` the other tests use. A derived `Copy` would regress this.
    #[allow(dead_code)]
    fn bound_view_is_copy_for_default_runner() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<GitAt<'static, processkit::JobRunner>>();
    }

    // The bound view (`git.at(dir)`) must produce byte-identical argv to the
    // dir-taking call (`git.method(dir, …)`) — the forwarder injects `self.dir`
    // in the right place and nothing else changes.
    #[tokio::test]
    async fn bound_view_matches_dir_taking_calls() {
        let dir = Path::new("/repo");
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);

        // A method with trailing args (dir injected first).
        git.merge_commit(dir, MergeCommit::branch("feat").no_ff())
            .await
            .unwrap();
        git.at(dir)
            .merge_commit(MergeCommit::branch("feat").no_ff())
            .await
            .unwrap();
        // A method taking a path arg after dir.
        git.worktree_remove(dir, Path::new("/wt"), true)
            .await
            .unwrap();
        git.at(dir)
            .worktree_remove(Path::new("/wt"), true)
            .await
            .unwrap();
        // One of the new query methods.
        git.conflicted_files(dir).await.unwrap();
        git.at(dir).conflicted_files().await.unwrap();
        // One of the §4 additions.
        git.tag_delete(dir, "v1").await.unwrap();
        git.at(dir).tag_delete("v1").await.unwrap();

        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), calls[1].args_str());
        assert_eq!(calls[2].args_str(), calls[3].args_str());
        assert_eq!(calls[4].args_str(), calls[5].args_str());
        assert_eq!(calls[6].args_str(), calls[7].args_str());
        // The bound calls also carried the bound dir as their working directory.
        assert_eq!(calls[1].cwd.as_deref(), Some(dir));
        assert_eq!(calls[3].cwd.as_deref(), Some(dir));
    }

    // Hermetic: the real status() command-building + porcelain parsing run
    // against a scripted runner — no `git` binary needed, so this runs on CI.
    #[tokio::test]
    async fn status_parses_scripted_output() {
        // `-z` output: NUL-delimited records, raw paths.
        let git = Git::with_runner(
            ScriptedRunner::new().on(["git", "status"], Reply::ok(" M a.rs\0?? b.rs\0")),
        );
        let entries = git.status(Path::new(".")).await.expect("status");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].code, " M");
        assert_eq!(entries[1].path, "b.rs");
    }

    // `status_tracked` is `status` minus untracked files — same parser, extra flag.
    #[tokio::test]
    async fn status_tracked_excludes_untracked_flag() {
        let rec = RecordingRunner::replying(Reply::ok(" M a.rs\0"));
        let git = Git::with_runner(&rec);
        let entries = git.status_tracked(Path::new(".")).await.expect("status");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].code, " M");
        assert_eq!(
            rec.only_call().args_str(),
            ["status", "--porcelain=v1", "-z", "--untracked-files=no"]
        );
    }

    // `branch_status` builds the porcelain v2 + branch + -z argv and parses the
    // combined header/entry output in one call.
    #[tokio::test]
    async fn branch_status_builds_v2_branch_args_and_parses() {
        let out = concat!(
            "# branch.oid abc\0",
            "# branch.head main\0",
            "# branch.upstream origin/main\0",
            "# branch.ab +1 -0\0",
            "1 .M N... 100644 100644 100644 1 2 a.rs\0",
            "? new.txt\0",
        );
        let rec = RecordingRunner::replying(Reply::ok(out));
        let git = Git::with_runner(&rec);
        let s = git
            .branch_status(Path::new("."))
            .await
            .expect("branch_status");
        assert_eq!(
            rec.only_call().args_str(),
            ["status", "--porcelain=v2", "--branch", "-z"]
        );
        // The poll primitive must not itself write the index (and re-trigger a
        // filesystem watcher re-querying through it).
        assert!(rec.only_call().envs.iter().any(|(k, v)| {
            k.to_str() == Some("GIT_OPTIONAL_LOCKS")
                && v.as_deref().and_then(|o| o.to_str()) == Some("0")
        }));
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind), (Some(1), Some(0)));
        assert_eq!(s.tracked_changes, 1);
        assert_eq!(s.untracked, 1);
        assert!(s.is_dirty());
    }

    // `conflicted_files` lists unmerged paths NUL-delimited (no quoting).
    #[tokio::test]
    async fn conflicted_files_builds_args_and_parses_nul_list() {
        let rec = RecordingRunner::replying(Reply::ok("a.rs\0sub/spaced name.rs\0"));
        let git = Git::with_runner(&rec);
        let paths = git
            .conflicted_files(Path::new("."))
            .await
            .expect("conflicted_files");
        assert_eq!(paths, ["a.rs", "sub/spaced name.rs"]);
        assert_eq!(
            rec.only_call().args_str(),
            ["diff", "--name-only", "--diff-filter=U", "-z"]
        );
    }

    #[tokio::test]
    async fn rev_parse_short_builds_short_flag() {
        let rec = RecordingRunner::replying(Reply::ok("a1b2c3d\n"));
        let git = Git::with_runner(&rec);
        let out = git.rev_parse_short(Path::new("/r"), "HEAD").await.unwrap();
        assert_eq!(out, "a1b2c3d");
        assert_eq!(rec.only_call().args_str(), ["rev-parse", "--short", "HEAD"]);
    }

    // M13: `rev_parse` passes `--verify` so a non-revision (a filename) errors
    // instead of being echoed back as a fake object id.
    #[tokio::test]
    async fn rev_parse_verifies_the_revision() {
        let rec = RecordingRunner::replying(Reply::ok("deadbeef\n"));
        let git = Git::with_runner(&rec);
        let out = git.rev_parse(Path::new("/r"), "HEAD").await.unwrap();
        assert_eq!(out, "deadbeef");
        assert_eq!(
            rec.only_call().args_str(),
            ["rev-parse", "--verify", "HEAD"]
        );
    }

    // A non-zero exit surfaces as a structured `Error::Exit`.
    #[tokio::test]
    async fn nonzero_exit_is_structured_error() {
        let git = Git::with_runner(
            ScriptedRunner::new().on(["git", "status"], Reply::fail(128, "not a git repository")),
        );
        match git.status(Path::new(".")).await.unwrap_err() {
            Error::Exit { code, stderr, .. } => {
                assert_eq!(code, 128);
                assert!(stderr.contains("not a git repository"), "{stderr}");
            }
            other => panic!("expected Exit, got {other:?}"),
        }
    }

    // diff_is_empty maps the raw exit code itself: 0 → clean, 1 → dirty, and
    // anything else is a real failure surfaced as Error::Exit.
    #[tokio::test]
    async fn diff_is_empty_maps_exit_codes() {
        let clean =
            Git::with_runner(ScriptedRunner::new().on(["git", "diff", "--quiet"], Reply::ok("")));
        assert!(clean.diff_is_empty(Path::new(".")).await.unwrap());

        let dirty = Git::with_runner(
            ScriptedRunner::new().on(["git", "diff", "--quiet"], Reply::fail(1, "")),
        );
        assert!(!dirty.diff_is_empty(Path::new(".")).await.unwrap());

        let broken = Git::with_runner(ScriptedRunner::new().on(
            ["git", "diff", "--quiet"],
            Reply::fail(128, "fatal: not a repo"),
        ));
        assert!(matches!(
            broken.diff_is_empty(Path::new(".")).await.unwrap_err(),
            Error::Exit { code: 128, .. }
        ));
    }

    // `add` must insert `--` before the pathspecs so a path can never be parsed
    // as an option. No fallback rule: the run only matches if `add --` was built.
    #[tokio::test]
    async fn add_inserts_pathspec_separator() {
        let git = Git::with_runner(ScriptedRunner::new().on(["git", "add", "--"], Reply::ok("")));
        git.add(Path::new("."), &[PathBuf::from("f.rs")])
            .await
            .expect("add should build `add -- <paths>`");
    }

    #[tokio::test]
    async fn worktree_list_parses_porcelain() {
        let git = Git::with_runner(ScriptedRunner::new().on(
            ["git", "worktree", "list"],
            Reply::ok("worktree /repo\nHEAD abc\nbranch refs/heads/main\n"),
        ));
        let wts = git.worktree_list(Path::new(".")).await.expect("list");
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].branch.as_deref(), Some("main"));
        assert_eq!(wts[0].head.as_deref(), Some("abc"));
    }

    // The new-branch worktree must build `worktree add -b <name> <path> <base>`,
    // in that exact order; only the full argv is scripted (no fallback).
    #[tokio::test]
    async fn worktree_add_builds_branch_path_and_base() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.worktree_add(
            Path::new("/repo"),
            WorktreeAdd::create_branch("/wt", "feature", "main"),
        )
        .await
        .expect("worktree add");
        assert_eq!(
            rec.only_call().args_str(),
            ["worktree", "add", "-b", "feature", "/wt", "main"]
        );
    }

    #[tokio::test]
    async fn worktree_remove_passes_force_then_path() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.worktree_remove(Path::new("/repo"), Path::new("/wt"), true)
            .await
            .expect("remove");
        assert_eq!(
            rec.only_call().args_str(),
            ["worktree", "remove", "--force", "/wt"]
        );
    }

    // `--no-checkout` must land between `-b <name>` and the path.
    #[tokio::test]
    async fn worktree_add_no_checkout_inserts_flag() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.worktree_add(
            Path::new("/repo"),
            WorktreeAdd::checkout("/wt", "main").no_checkout(),
        )
        .await
        .expect("worktree add");
        assert_eq!(
            rec.only_call().args_str(),
            ["worktree", "add", "--no-checkout", "/wt", "main"]
        );
    }

    #[tokio::test]
    async fn checkout_detach_builds_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.checkout_detach(Path::new("."), "abc123")
            .await
            .expect("detach");
        assert_eq!(
            rec.only_call().args_str(),
            ["checkout", "--detach", "abc123"]
        );
    }

    // current_branch reads `symbolic-ref --quiet --short HEAD`: exit 0 → the branch
    // name (a normal *or* unborn branch), exit 1 → None (detached HEAD), and any
    // other non-zero (e.g. not a repository) stays a real error.
    #[tokio::test]
    async fn current_branch_reads_symbolic_ref_with_exit_mapping() {
        // A normal branch (exit 0) — and the argv is pinned.
        let rec = RecordingRunner::replying(Reply::ok("feature/x\n"));
        let on_branch = Git::with_runner(&rec);
        assert_eq!(
            on_branch.current_branch(Path::new(".")).await.unwrap(),
            Some("feature/x".to_string())
        );
        assert_eq!(
            rec.only_call().args_str(),
            ["symbolic-ref", "--quiet", "--short", "HEAD"]
        );
        // An unborn branch also exits 0 with the branch name (the bug this fixes:
        // the old `rev-parse --abbrev-ref HEAD` errored with exit 128 here).
        let unborn = Git::with_runner(
            ScriptedRunner::new().on(["git", "symbolic-ref"], Reply::ok("main\n")),
        );
        assert_eq!(
            unborn.current_branch(Path::new(".")).await.unwrap(),
            Some("main".to_string())
        );
        // A detached HEAD exits 1 silently → None.
        let detached =
            Git::with_runner(ScriptedRunner::new().on(["git", "symbolic-ref"], Reply::fail(1, "")));
        assert_eq!(detached.current_branch(Path::new(".")).await.unwrap(), None);
        // Any other non-zero (not a repository, exit 128) is a real error.
        let not_repo = Git::with_runner(ScriptedRunner::new().on(
            ["git", "symbolic-ref"],
            Reply::fail(128, "fatal: not a git repository"),
        ));
        assert!(not_repo.current_branch(Path::new(".")).await.is_err());
    }

    // Partial amend commit must build `commit --amend -m <msg> --only -- <paths>`.
    #[tokio::test]
    async fn commit_paths_builds_only_amend_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.commit_paths(
            Path::new("."),
            CommitPaths::new([PathBuf::from("a.rs"), PathBuf::from("b.rs")], "msg").amend(),
        )
        .await
        .expect("commit_paths");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "commit", "--amend", "-m", "msg", "--only", "--", "a.rs", "b.rs"
            ]
        );
    }

    // is_unborn maps the rev-parse exit code: 0 → has commits (false), 1 →
    // unborn (true), anything else is a structured error.
    #[tokio::test]
    async fn is_unborn_maps_exit_codes() {
        let born =
            Git::with_runner(ScriptedRunner::new().on(["git", "rev-parse"], Reply::ok("abc\n")));
        assert!(!born.is_unborn(Path::new(".")).await.unwrap());
        let unborn =
            Git::with_runner(ScriptedRunner::new().on(["git", "rev-parse"], Reply::fail(1, "")));
        assert!(unborn.is_unborn(Path::new(".")).await.unwrap());
        let broken = Git::with_runner(
            ScriptedRunner::new().on(["git", "rev-parse"], Reply::fail(128, "boom")),
        );
        assert!(matches!(
            broken.is_unborn(Path::new(".")).await.unwrap_err(),
            Error::Exit { code: 128, .. }
        ));
    }

    #[tokio::test]
    async fn log_builds_revspec_and_format() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.log(Path::new("."), "main..HEAD", 5).await.expect("log");
        assert_eq!(
            rec.only_call().args_str(),
            [
                "log",
                "main..HEAD",
                "-n5",
                "-z",
                "--format=%H%x1f%h%x1f%an%x1f%aI%x1f%s"
            ]
        );
    }

    #[tokio::test]
    async fn stash_push_adds_include_untracked() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.stash_push(Path::new("."), true).await.expect("stash");
        assert_eq!(
            rec.only_call().args_str(),
            ["stash", "push", "--include-untracked"]
        );
    }

    // `diff_text` for the working tree must build `diff HEAD` plus the stable
    // machine-output flags, in order.
    #[tokio::test]
    async fn diff_text_builds_working_tree_args() {
        // The `rev-parse` unborn probe replies exit 0 (HEAD resolves), so the diff
        // targets HEAD. The probe is the first call; the diff is the last.
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.diff_text(Path::new("."), DiffSpec::WorkingTree)
            .await
            .expect("diff_text");
        assert_eq!(
            rec.calls().last().unwrap().args_str(),
            [
                "diff",
                "HEAD",
                "--no-color",
                "--no-ext-diff",
                "-M",
                // Pin the parser's `a/`…`b/` headers against a user's
                // `diff.noprefix`/`diff.mnemonicPrefix` config.
                "--src-prefix=a/",
                "--dst-prefix=b/",
            ]
        );
    }

    // On an unborn repo the working-tree diff targets the empty tree instead of
    // the unresolvable `HEAD`, so it returns additions rather than erroring. The
    // diff rule only matches the empty-tree argv, so a `HEAD` target would miss it.
    #[tokio::test]
    async fn diff_text_working_tree_uses_empty_tree_when_unborn() {
        let git = Git::with_runner(
            ScriptedRunner::new()
                .on(["git", "rev-parse"], Reply::fail(1, "")) // unborn: HEAD doesn't resolve
                .on(["git", "diff", EMPTY_TREE], Reply::ok("EMPTY")),
        );
        let out = git
            .diff_text(Path::new("."), DiffSpec::WorkingTree)
            .await
            .expect("diff_text");
        assert_eq!(out, "EMPTY");
    }

    // Hermetic: real diff() arg-building (`Rev`) + the ported parser against
    // canned git-format output.
    #[tokio::test]
    async fn diff_parses_scripted_output() {
        let out = "diff --git a/m b/m\n--- a/m\n+++ b/m\n@@ -1 +1 @@\n-a\n+b\n";
        let git = Git::with_runner(ScriptedRunner::new().on(["git", "diff"], Reply::ok(out)));
        let files = git
            .diff(Path::new("."), DiffSpec::Rev("HEAD~1".into()))
            .await
            .expect("diff");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "m");
        assert_eq!(files[0].change, ChangeKind::Modified);
    }

    #[tokio::test]
    async fn branch_exists_maps_exit_codes() {
        let yes = Git::with_runner(ScriptedRunner::new().on(["git", "show-ref"], Reply::ok("")));
        assert!(yes.branch_exists(Path::new("."), "main").await.unwrap());
        let no =
            Git::with_runner(ScriptedRunner::new().on(["git", "show-ref"], Reply::fail(1, "")));
        assert!(!no.branch_exists(Path::new("."), "nope").await.unwrap());
    }

    // The full ref prefix is stripped but a slashed default branch survives; an
    // unset origin/HEAD (non-zero exit) is `None`, not an error.
    #[tokio::test]
    async fn remote_head_branch_strips_prefix_and_keeps_slashes() {
        let simple = Git::with_runner(ScriptedRunner::new().on(
            ["git", "symbolic-ref"],
            Reply::ok("refs/remotes/origin/main\n"),
        ));
        assert_eq!(
            simple
                .remote_head_branch(Path::new("."))
                .await
                .unwrap()
                .as_deref(),
            Some("main")
        );

        let slashed = Git::with_runner(ScriptedRunner::new().on(
            ["git", "symbolic-ref"],
            Reply::ok("refs/remotes/origin/release/v2\n"),
        ));
        assert_eq!(
            slashed
                .remote_head_branch(Path::new("."))
                .await
                .unwrap()
                .as_deref(),
            Some("release/v2")
        );

        let unset =
            Git::with_runner(ScriptedRunner::new().on(["git", "symbolic-ref"], Reply::fail(1, "")));
        assert!(
            unset
                .remote_head_branch(Path::new("."))
                .await
                .unwrap()
                .is_none()
        );
    }

    // remote_branch_exists must pass `GIT_TERMINAL_PROMPT=0` and treat empty
    // stdout as "absent".
    #[tokio::test]
    async fn remote_branch_exists_sets_env_and_reads_stdout() {
        let rec = RecordingRunner::replying(Reply::ok("abc123\trefs/heads/main\n"));
        let git = Git::with_runner(&rec);
        assert!(
            git.remote_branch_exists(Path::new("/repo"), "main")
                .await
                .unwrap()
        );
        let call = rec.only_call();
        assert!(call.envs.iter().any(|(k, v)| {
            k.to_str() == Some("GIT_TERMINAL_PROMPT")
                && v.as_deref().and_then(|o| o.to_str()) == Some("0")
        }));
        // Exact-ref query — a bare `main` would tail-match `bar/main`.
        assert_eq!(call.args_str(), ["ls-remote", "origin", "refs/heads/main"]);

        let empty = Git::with_runner(ScriptedRunner::new().on(["git", "ls-remote"], Reply::ok("")));
        assert!(
            !empty
                .remote_branch_exists(Path::new("."), "x")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn diff_stat_parses_counts() {
        let git = Git::with_runner(ScriptedRunner::new().on(
            ["git", "diff", "--shortstat"],
            Reply::ok(" 2 files changed, 5 insertions(+), 1 deletion(-)\n"),
        ));
        let stat = git.diff_stat(Path::new("."), "main..HEAD").await.unwrap();
        assert_eq!(
            (stat.files_changed, stat.insertions, stat.deletions),
            (2, 5, 1)
        );
    }

    #[tokio::test]
    async fn status_text_returns_raw_porcelain() {
        let git = Git::with_runner(ScriptedRunner::new().on(
            ["git", "status", "--porcelain=v1"],
            Reply::ok(" M a.rs\n?? b.rs\n"),
        ));
        let text = git.status_text(Path::new(".")).await.expect("status_text");
        assert!(text.contains(" M a.rs") && text.contains("?? b.rs"));
    }

    #[tokio::test]
    async fn run_args_forwards_str_slices() {
        let git =
            Git::with_runner(ScriptedRunner::new().on(["git", "status", "-s"], Reply::ok("ok\n")));
        assert_eq!(git.run_args(&["status", "-s"]).await.unwrap(), "ok");
    }

    #[tokio::test]
    async fn merge_commit_builds_no_ff_and_message() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.merge_commit(
            Path::new("/r"),
            MergeCommit::branch("feature").no_ff().message("merge it"),
        )
        .await
        .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["merge", "--no-ff", "-m", "merge it", "feature"]
        );
    }

    // No message → `--no-edit` (default message, non-interactive) instead of `$EDITOR`.
    #[tokio::test]
    async fn merge_commit_without_message_uses_no_edit() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.merge_commit(Path::new("/r"), MergeCommit::branch("feature"))
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["merge", "--no-edit", "feature"]
        );
    }

    // rebase/rebase_continue force a no-op editor so a headless caller never hangs.
    #[tokio::test]
    async fn rebase_suppresses_editor() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.rebase(Path::new("/r"), "main").await.unwrap();
        let call = rec.only_call();
        assert_eq!(call.args_str(), ["rebase", "main"]);
        assert!(call.envs.iter().any(|(k, v)| {
            k.to_str() == Some("GIT_EDITOR")
                && v.as_deref().and_then(|o| o.to_str()) == Some("true")
        }));
    }

    #[tokio::test]
    async fn push_builds_set_upstream_remote_refspec() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.push(
            Path::new("/r"),
            GitPush::refspec("feat", "feature").set_upstream(),
        )
        .await
        .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["push", "-u", "origin", "feat:feature"]
        );
    }

    // The common bare-branch push: `push origin <branch>` (no `-u`), with prompts
    // off so a credential-needing remote fails fast instead of hanging.
    #[tokio::test]
    async fn push_bare_branch_builds_origin_branch_prompt_off() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.push(Path::new("/r"), GitPush::branch("feature"))
            .await
            .unwrap();
        let call = rec.only_call();
        assert_eq!(call.args_str(), ["push", "origin", "feature"]);
        assert!(call.envs.iter().any(|(k, v)| {
            k.to_str() == Some("GIT_TERMINAL_PROMPT")
                && v.as_deref().and_then(|o| o.to_str()) == Some("0")
        }));
    }

    // M16: a `+` (force-push) or an extra `:` (multi-ref) smuggled into a branch name
    // is refused before spawning — force-pushing must be explicit via `run`.
    #[tokio::test]
    async fn push_rejects_force_and_multiref_metacharacters() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        for bad in ["+main", "+main:main", "a:b:c"] {
            assert!(
                git.push(Path::new("/r"), GitPush::branch(bad))
                    .await
                    .is_err(),
                "{bad:?} must be rejected"
            );
        }
        // A legitimate `local:remote` refspec still works (one `:`, no `+`).
        assert!(
            git.push(Path::new("/r"), GitPush::refspec("main", "prod"))
                .await
                .is_ok()
        );
        assert!(
            rec.calls()
                .iter()
                .all(|c| c.args_str().last().unwrap() != "+main")
        );
    }

    // `.remote()` swaps the remote token in place.
    #[tokio::test]
    async fn push_remote_override_swaps_remote() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.push(
            Path::new("/r"),
            GitPush::branch("feature").remote("upstream"),
        )
        .await
        .unwrap();
        assert_eq!(rec.only_call().args_str(), ["push", "upstream", "feature"]);
    }

    // With a credential provider, a remote op gets a leading `-c credential.helper`
    // pair (the secret referenced by env-var NAME) plus the secret in the env — and
    // the token value never appears in argv. Covers push (mutating) and fetch.
    #[tokio::test]
    async fn with_credentials_injects_helper_and_secret_env_for_remote_ops() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec)
            .with_credentials(Arc::new(StaticCredential::token("ghp_secret123")));
        git.push(Path::new("/r"), GitPush::branch("feature"))
            .await
            .unwrap();
        let call = rec.only_call();
        let args = call.args_str();
        // A leading helper-reset + inline helper precede the subcommand.
        assert_eq!(args[0], "-c", "config flag leads the argv");
        assert!(
            args.iter().any(|a| a == "credential.helper="),
            "inherited helpers are cleared first: {args:?}"
        );
        assert!(
            args.iter()
                .any(|a| a.contains("credential.helper=!f()")
                    && a.contains("VCS_TOOLKIT_GIT_PASSWORD")),
            "inline helper references the secret by env-var name: {args:?}"
        );
        assert!(
            args.contains(&"push".to_string()) && args.contains(&"feature".to_string()),
            "the real subcommand still runs: {args:?}"
        );
        // The secret value is NEVER in argv.
        assert!(
            !args.iter().any(|a| a.contains("ghp_secret123")),
            "secret leaked into argv: {args:?}"
        );
        // The secret lives in the env, under the helper's var name.
        let pw = call
            .envs
            .iter()
            .find(|(k, _)| k.to_str() == Some("VCS_TOOLKIT_GIT_PASSWORD"))
            .and_then(|(_, v)| v.as_ref())
            .and_then(|v| v.to_str());
        assert_eq!(pw, Some("ghp_secret123"), "secret carried in env");
    }

    // Without a provider, remote ops are byte-identical to before — no `-c`
    // credential helper, no secret env (ambient git auth, unchanged).
    #[tokio::test]
    async fn default_client_injects_no_credential_helper() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.push(Path::new("/r"), GitPush::branch("feature"))
            .await
            .unwrap();
        let call = rec.only_call();
        assert_eq!(
            call.args_str(),
            ["push", "origin", "feature"],
            "no credential `-c` args without a provider"
        );
        assert!(
            !call
                .envs
                .iter()
                .any(|(k, _)| k.to_str() == Some("VCS_TOOLKIT_GIT_PASSWORD")),
            "no secret env without a provider"
        );
    }

    // `clone_repo` builds its argv via a different path (`command()` + `.arg()`
    // chaining, not `command_in` + extend), so verify the `-c` credential args
    // still LEAD it and the real clone flags/url/dest follow the subcommand.
    #[tokio::test]
    async fn with_credentials_clone_puts_config_flags_before_subcommand() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git =
            Git::with_runner(&rec).with_credentials(Arc::new(StaticCredential::token("s3cr3t")));
        git.clone_repo(
            "https://example.com/r.git",
            Path::new("/dest"),
            CloneSpec::default().branch("main"),
        )
        .await
        .unwrap();
        let call = rec.only_call();
        let args = call.args_str();
        assert_eq!(args[0], "-c", "config flags lead the clone argv");
        let clone_at = args
            .iter()
            .position(|a| a == "clone")
            .expect("clone present");
        // Only credential `-c` flags precede the `clone` subcommand.
        assert!(
            args[..clone_at]
                .iter()
                .all(|a| a == "-c" || a.starts_with("credential.helper")),
            "only credential -c flags precede `clone`: {args:?}"
        );
        // The real clone flags/url/dest follow the subcommand.
        let tail = &args[clone_at..];
        assert!(tail.iter().any(|a| a == "--branch") && tail.iter().any(|a| a == "main"));
        assert!(tail.iter().any(|a| a == "https://example.com/r.git"));
        assert!(
            !args.iter().any(|a| a.contains("s3cr3t")),
            "secret not in argv"
        );
        // H5: clone scopes the helper to the URL's host (in env, never argv), so a
        // cross-host redirect/submodule during the clone can't extract the token.
        let host = call
            .envs
            .iter()
            .find(|(k, _)| k.to_str() == Some("VCS_TOOLKIT_GIT_HOST"))
            .and_then(|(_, v)| v.as_ref())
            .and_then(|v| v.to_str());
        assert_eq!(
            host,
            Some("example.com"),
            "the clone URL's host scopes the credential helper"
        );
        // The host scoping travels in env; the credential `-c` flags that precede
        // `clone` must not bake the host into the helper config.
        assert!(
            args[..clone_at].iter().all(|a| !a.contains("example.com")),
            "host stays in env, not the credential config args: {:?}",
            &args[..clone_at]
        );
    }

    // A `Credential::userpass` username threads through to the helper's env on a
    // remote op (here `fetch`) — the non-default-username path, end-to-end.
    #[tokio::test]
    async fn with_credentials_userpass_threads_username_through_env() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec).with_credentials(Arc::new(StaticCredential::new(
            Credential::userpass("alice", "s3cr3t"),
        )));
        git.fetch(Path::new("/r")).await.unwrap();
        let call = rec.only_call();
        let user = call
            .envs
            .iter()
            .find(|(k, _)| k.to_str() == Some("VCS_TOOLKIT_GIT_USERNAME"))
            .and_then(|(_, v)| v.as_ref())
            .and_then(|v| v.to_str());
        assert_eq!(user, Some("alice"), "userpass username reaches the env");
        assert_eq!(call.args_str()[0], "-c", "helper `-c` leads fetch too");
        assert!(call.args_str().contains(&"fetch".to_string()));
    }

    // No-provider is byte-identical for the read/clone arg-construction paths too,
    // not only `push` (fetch uses `command_in`+extend; clone uses `command`+chain).
    #[tokio::test]
    async fn default_client_no_helper_on_fetch_and_clone() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        Git::with_runner(&rec).fetch(Path::new("/r")).await.unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["fetch", "--quiet"],
            "fetch unchanged without a provider"
        );

        let rec = RecordingRunner::replying(Reply::ok(""));
        Git::with_runner(&rec)
            .clone_repo(
                "https://example.com/r.git",
                Path::new("/dest"),
                CloneSpec::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str()[0],
            "clone",
            "clone leads with the subcommand (no `-c`) without a provider"
        );
    }

    // The `with_token` convenience drives the same HTTPS credential.helper path as
    // `with_credentials` (secret in env, helper `-c` leads, not in argv).
    #[tokio::test]
    async fn with_token_convenience_authenticates_https_remote() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec).with_token("ghp_conv");
        git.fetch(Path::new("/r")).await.unwrap();
        let call = rec.only_call();
        assert_eq!(call.args_str()[0], "-c", "helper `-c` leads");
        let pw = call
            .envs
            .iter()
            .find(|(k, _)| k.to_str() == Some("VCS_TOOLKIT_GIT_PASSWORD"))
            .and_then(|(_, v)| v.as_ref())
            .and_then(|v| v.to_str());
        assert_eq!(pw, Some("ghp_conv"), "secret carried in env");
        assert!(
            !call.args_str().iter().any(|a| a.contains("ghp_conv")),
            "secret not in argv"
        );
    }

    #[tokio::test]
    async fn upstream_maps_unset_to_none() {
        let set = Git::with_runner(
            ScriptedRunner::new().on(["git", "rev-parse"], Reply::ok("origin/main\n")),
        );
        assert_eq!(
            set.upstream(Path::new(".")).await.unwrap().as_deref(),
            Some("origin/main")
        );
        // No upstream configured exits 128 (indistinguishable from a real failure by
        // code, since git uses 128 for both) → None.
        let unset =
            Git::with_runner(ScriptedRunner::new().on(["git", "rev-parse"], Reply::fail(128, "")));
        assert!(unset.upstream(Path::new(".")).await.unwrap().is_none());
        // A timeout (no exit code) is a real failure — it must surface, not read as
        // "no upstream".
        let timed_out =
            Git::with_runner(ScriptedRunner::new().on(["git", "rev-parse"], Reply::timeout()));
        assert!(timed_out.upstream(Path::new(".")).await.is_err());
    }

    // remote_head_branch maps the `symbolic-ref --quiet` exit code: 0 → the branch
    // (ref prefix stripped), 1 → None (unset origin/HEAD), and anything else (a real
    // failure / timeout) surfaces rather than being swallowed as "no default branch".
    #[tokio::test]
    async fn remote_head_branch_maps_exit_codes() {
        let set = Git::with_runner(ScriptedRunner::new().on(
            ["git", "symbolic-ref"],
            Reply::ok("refs/remotes/origin/release/v2\n"),
        ));
        assert_eq!(
            set.remote_head_branch(Path::new("."))
                .await
                .unwrap()
                .as_deref(),
            Some("release/v2"),
            "the full ref prefix is stripped, slashes preserved"
        );
        let unset =
            Git::with_runner(ScriptedRunner::new().on(["git", "symbolic-ref"], Reply::fail(1, "")));
        assert!(
            unset
                .remote_head_branch(Path::new("."))
                .await
                .unwrap()
                .is_none()
        );
        // A real failure (exit 128, not the silent --quiet exit 1) surfaces.
        let err = Git::with_runner(ScriptedRunner::new().on(
            ["git", "symbolic-ref"],
            Reply::fail(128, "fatal: not a git repository"),
        ));
        assert!(err.remote_head_branch(Path::new(".")).await.is_err());
        // A timeout surfaces too.
        let timed_out =
            Git::with_runner(ScriptedRunner::new().on(["git", "symbolic-ref"], Reply::timeout()));
        assert!(timed_out.remote_head_branch(Path::new(".")).await.is_err());
    }

    #[tokio::test]
    async fn set_upstream_builds_branch_flag() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.set_upstream(Path::new("/r"), "feat", "origin/feature")
            .await
            .unwrap();
        assert_eq!(
            rec.only_call().args_str(),
            ["branch", "--set-upstream-to=origin/feature", "feat"]
        );
    }

    #[tokio::test]
    async fn remote_branches_parses_ls_remote() {
        let git = Git::with_runner(ScriptedRunner::new().on(
            ["git", "ls-remote"],
            Reply::ok("aaa\trefs/heads/main\nbbb\trefs/heads/feat/x\n"),
        ));
        let branches = git.remote_branches(Path::new("."), "origin").await.unwrap();
        assert_eq!(branches, ["main", "feat/x"]);
    }

    #[tokio::test]
    async fn delete_branch_force_uses_capital_d() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.delete_branch(Path::new("/r"), "old", true)
            .await
            .unwrap();
        assert_eq!(rec.only_call().args_str(), ["branch", "-D", "old"]);
    }

    // `branch --merged` marks the current branch with `*` and a branch checked out
    // in another worktree with `+`; both must still match after marker stripping.
    #[tokio::test]
    async fn is_merged_strips_branch_markers() {
        let git = Git::with_runner(ScriptedRunner::new().on(
            ["git", "branch", "--merged"],
            Reply::ok("  main\n* feature\n+ wt-branch\n"),
        ));
        for name in ["main", "feature", "wt-branch"] {
            assert!(
                git.is_merged(Path::new("."), name, "main").await.unwrap(),
                "{name} should be reported merged"
            );
        }
        assert!(
            !git.is_merged(Path::new("."), "absent", "main")
                .await
                .unwrap()
        );
    }

    // `fetch` must disable the credential prompt so it fails fast (never hangs) on
    // a remote needing auth — matching the other remote ops.
    #[tokio::test]
    async fn fetch_disables_terminal_prompt() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.fetch(Path::new("/r")).await.unwrap();
        let call = rec.only_call();
        assert_eq!(call.args_str(), ["fetch", "--quiet"]);
        assert!(call.envs.iter().any(|(k, v)| {
            k.to_str() == Some("GIT_TERMINAL_PROMPT")
                && v.as_deref().and_then(|o| o.to_str()) == Some("0")
        }));
    }

    // A transient failure (DNS/network) is retried up to FETCH_ATTEMPTS times.
    #[tokio::test]
    async fn fetch_retries_transient_failures() {
        let rec = RecordingRunner::replying(Reply::fail(
            128,
            "fatal: unable to access: Could not resolve host: example.com",
        ));
        let git = Git::with_runner(&rec);
        assert!(git.fetch(Path::new("/r")).await.is_err());
        assert_eq!(rec.calls().len(), FETCH_ATTEMPTS as usize);
    }

    // R6 (a `fetch` timeout is NOT retried) is pinned at the unit level by
    // `vcs_cli_support`'s `classifies_nothing_to_commit_and_transient_fetch`
    // (`is_transient_fetch_error(&Timeout) == false`); together with
    // `fetch_retries_transient_failures` above (the loop retries exactly what the
    // predicate accepts) that proves the timeout is terminal for the fetch-retry. A
    // faithful end-to-end timeout is awkward to simulate hermetically (a paused-clock
    // `Reply::pending()` doesn't auto-fire the per-command deadline), so it isn't
    // duplicated here.

    // Opt-in lock-contention retry: a mutation that fails because another process
    // holds `index.lock` is retried and succeeds — the command never ran, so the
    // retry is safe. `RetryPolicy::none().attempts(3)` keeps the backoff at zero so
    // the test never sleeps.
    #[tokio::test]
    async fn with_retry_retries_lock_contention_on_a_mutation() {
        let rec = RecordingRunner::new(ScriptedRunner::new().on_sequence(
            ["git", "commit"],
            [
                Reply::fail(
                    128,
                    "fatal: Unable to create '/r/.git/index.lock': File exists.",
                ),
                Reply::ok(""),
            ],
        ));
        let git = Git::with_runner(&rec).with_retry(RetryPolicy::none().attempts(3));
        git.commit(Path::new("/r"), "msg")
            .await
            .expect("retried past the lock");
        assert_eq!(rec.calls().len(), 2, "one retry after the lock failure");
    }

    // Retry is off by default — the same lock failure propagates without `with_retry`.
    #[tokio::test]
    async fn default_client_does_not_retry_lock_contention() {
        let rec = RecordingRunner::new(ScriptedRunner::new().on_sequence(
            ["git", "commit"],
            [
                Reply::fail(
                    128,
                    "fatal: Unable to create '/r/.git/index.lock': File exists.",
                ),
                Reply::ok(""),
            ],
        ));
        let git = Git::with_runner(&rec);
        assert!(git.commit(Path::new("/r"), "msg").await.is_err());
        assert_eq!(rec.calls().len(), 1, "no retry without with_retry");
    }

    // Even with retry on, a real (non-lock) failure is returned immediately — only
    // lock contention is retried, so a genuine error is never silently repeated.
    #[tokio::test]
    async fn with_retry_does_not_retry_a_real_failure() {
        let rec = RecordingRunner::new(ScriptedRunner::new().on_sequence(
            ["git", "commit"],
            [
                Reply::fail(1, "error: pathspec 'x' did not match"),
                Reply::ok(""),
            ],
        ));
        let git = Git::with_runner(&rec).with_retry(RetryPolicy::none().attempts(3));
        assert!(git.commit(Path::new("/r"), "msg").await.is_err());
        assert_eq!(rec.calls().len(), 1, "a non-lock failure is not retried");
    }

    // A non-transient failure fails fast — no retry.
    #[tokio::test]
    async fn fetch_does_not_retry_permanent_failures() {
        let rec = RecordingRunner::replying(Reply::fail(1, "fatal: couldn't find remote ref"));
        let git = Git::with_runner(&rec);
        assert!(git.fetch(Path::new("/r")).await.is_err());
        assert_eq!(rec.calls().len(), 1);
    }

    // Client-level cancellation (processkit 0.8 `cancellation` feature) on a
    // *retried* op: a `fetch` built on a client with `default_cancel_on(token)`
    // parks until the token fires, then surfaces `Error::Cancelled` — and because
    // cancellation is **terminal** (not transient), the fetch-retry does NOT
    // replay it (one spawn, not FETCH_ATTEMPTS). Hermetic via `Reply::pending()`
    // on a paused clock.
    #[tokio::test(start_paused = true)]
    async fn fetch_cancels_and_does_not_retry() {
        use processkit::CancellationToken;
        let token = CancellationToken::new();
        let rec =
            RecordingRunner::new(ScriptedRunner::new().on(["git", "fetch"], Reply::pending()));
        let git = Git::with_runner(&rec).default_cancel_on(token.clone());
        let call = git.fetch(Path::new("/r"));
        tokio::pin!(call);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(3600), &mut call)
                .await
                .is_err(),
            "fetch must park until the token fires"
        );
        token.cancel();
        assert!(matches!(call.await.unwrap_err(), Error::Cancelled { .. }));
        assert_eq!(
            rec.calls().len(),
            1,
            "cancellation is terminal — the fetch-retry must not replay it"
        );
    }

    // The injection guard: a flag-shaped value in any exposed positional slot
    // must be refused BEFORE anything spawns.
    #[tokio::test]
    async fn flag_like_positionals_are_rejected_before_spawning() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        let dir = Path::new("/r");

        assert!(git.checkout(dir, "-evil").await.is_err());
        assert!(git.create_branch(dir, "--force").await.is_err());
        assert!(git.delete_branch(dir, "-D", false).await.is_err());
        assert!(git.rename_branch(dir, "ok", "-bad").await.is_err());
        assert!(
            git.merge_commit(dir, MergeCommit::branch("-evil"))
                .await
                .is_err()
        );
        assert!(
            git.merge_no_commit(dir, MergeNoCommit::branch("-evil").no_ff())
                .await
                .is_err()
        );
        assert!(git.merge_squash(dir, "-evil").await.is_err());
        assert!(git.rebase(dir, "-i").await.is_err());
        assert!(git.cherry_pick(dir, "-n").await.is_err());
        assert!(git.revert(dir, "-evil").await.is_err());
        assert!(git.tag_create(dir, "-d", None).await.is_err());
        assert!(
            git.tag_create(dir, "ok", Some("-evil".into()))
                .await
                .is_err()
        );
        assert!(git.tag_delete(dir, "-evil").await.is_err());
        assert!(git.remote_add(dir, "-evil", "url").await.is_err());
        assert!(git.remote_set_url(dir, "-evil", "url").await.is_err());
        assert!(git.set_upstream(dir, "-evil", "origin/x").await.is_err());
        assert!(git.log(dir, "-evil", 5).await.is_err());
        assert!(git.rev_list_count(dir, "-evil").await.is_err());
        assert!(git.diff_stat(dir, "-evil").await.is_err());
        assert!(git.diff_range_is_empty(dir, "-evil").await.is_err());
        assert!(
            git.diff_text(dir, DiffSpec::Rev("-evil".into()))
                .await
                .is_err()
        );
        assert!(git.rev_parse(dir, "-evil").await.is_err());
        assert!(git.rev_parse_short(dir, "-evil").await.is_err());
        assert!(git.resolve_commit(dir, "-evil").await.is_err());
        assert!(git.reset_hard(dir, "-evil").await.is_err());
        assert!(git.checkout_detach(dir, "-evil").await.is_err());
        assert!(git.config_set(dir, "-evil", "v").await.is_err());
        assert!(
            git.push(dir, GitPush::branch("-evil")).await.is_err(),
            "refspec guard"
        );
        // Embedded-token-prefix and standalone-rev positionals:
        assert!(git.show_file(dir, "-evil", "f.txt").await.is_err());
        assert!(git.blame(dir, "f.txt", Some("-s".into())).await.is_err());
        assert!(git.remote_url(dir, "-evil").await.is_err());
        assert!(git.remote_branches(dir, "-evil").await.is_err());
        assert!(git.fetch_from(dir, "--upload-pack=x").await.is_err());
        // URL positionals (a leading-`-` url is an RCE-class flag injection).
        assert!(
            git.clone_repo("--upload-pack=x", Path::new("/d"), CloneSpec::new())
                .await
                .is_err()
        );
        assert!(git.remote_add(dir, "ok", "--upload-pack=x").await.is_err());
        assert!(git.remote_set_url(dir, "ok", "-evil").await.is_err());
        assert!(git.is_merged(dir, "-evil", "main").await.is_err());
        assert!(git.config_get(dir, "-evil").await.is_err());
        assert!(
            git.worktree_add(
                dir,
                WorktreeAdd::create_branch(Path::new("/wt"), "-evil", "HEAD")
            )
            .await
            .is_err()
        );
        // Empty values are refused too.
        assert!(git.checkout(dir, "").await.is_err());

        assert!(
            rec.calls().is_empty(),
            "nothing may spawn: {:?}",
            rec.calls()
        );

        // …and legitimate values still pass through unchanged (with the trailing
        // `--` that keeps a path-like ref out of pathspec mode — C2).
        git.checkout(dir, "feature/x").await.expect("checkout");
        assert_eq!(rec.only_call().args_str(), ["checkout", "feature/x", "--"]);
    }

    // The hardened profile lands its env pairs/removals on EVERY command, and
    // composes with per-command env like GIT_TERMINAL_PROMPT.
    #[tokio::test]
    async fn harden_applies_env_profile_to_every_command() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec).harden();
        git.status(Path::new("/r")).await.expect("status");
        git.fetch(Path::new("/r")).await.expect("fetch");

        for call in rec.calls() {
            let has = |k: &str, v: &str| {
                call.envs.iter().any(|(key, val)| {
                    key.to_str() == Some(k) && val.as_deref().and_then(|o| o.to_str()) == Some(v)
                })
            };
            let removed = |k: &str| {
                call.envs
                    .iter()
                    .any(|(key, val)| key.to_str() == Some(k) && val.is_none())
            };
            assert!(has("GIT_CONFIG_NOSYSTEM", "1"), "{:?}", call.args_str());
            assert!(has("GIT_CONFIG_COUNT", "3"));
            assert!(has("GIT_CONFIG_KEY_0", "core.hooksPath"));
            assert!(has("GIT_CONFIG_VALUE_0", "/dev/null"));
            assert!(has("GIT_CONFIG_KEY_1", "core.fsmonitor"));
            // The repo-local core.sshCommand kill-switch (pinned empty).
            assert!(has("GIT_CONFIG_KEY_2", "core.sshCommand"));
            assert!(has("GIT_CONFIG_VALUE_2", ""));
            assert!(has("GIT_TERMINAL_PROMPT", "0"));
            assert!(removed("GIT_DIR"), "GIT_DIR scrubbed");
            assert!(removed("GIT_CONFIG_GLOBAL"), "global config scrubbed");
            // Command-hook env vectors are scrubbed too.
            assert!(removed("GIT_SSH_COMMAND"), "GIT_SSH_COMMAND scrubbed");
            assert!(removed("GIT_ASKPASS"), "GIT_ASKPASS scrubbed");
            assert!(removed("GIT_EXTERNAL_DIFF"), "GIT_EXTERNAL_DIFF scrubbed");
            assert!(removed("GIT_PAGER"), "GIT_PAGER scrubbed");
            // M14: the additional code-execution vectors + pathspec-mode vars.
            assert!(removed("GIT_PROXY_COMMAND"), "GIT_PROXY_COMMAND scrubbed");
            assert!(removed("GIT_EXEC_PATH"), "GIT_EXEC_PATH scrubbed");
            assert!(removed("GIT_TEMPLATE_DIR"), "GIT_TEMPLATE_DIR scrubbed");
            assert!(
                removed("GIT_ICASE_PATHSPECS"),
                "GIT_ICASE_PATHSPECS scrubbed"
            );
        }
    }

    // H4: EVERY git client (not just `harden()`) scrubs the repo-**redirector** env
    // vars, so a `GIT_DIR`/`GIT_INDEX_FILE` leaking from the parent (e.g. running
    // inside a git hook, which exports them) can't silently retarget a command at a
    // different repository than the bound `dir`. The command-hook scrubs and config
    // pins stay `harden()`-only.
    #[tokio::test]
    async fn default_client_scrubs_repo_redirector_env() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec); // NOT hardened
        git.status(Path::new("/r")).await.expect("status");
        let call = rec.only_call();
        let removed = |k: &str| {
            call.envs
                .iter()
                .any(|(key, val)| key.to_str() == Some(k) && val.is_none())
        };
        let has_key = |k: &str| call.envs.iter().any(|(key, _)| key.to_str() == Some(k));
        for var in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_NAMESPACE",
        ] {
            assert!(removed(var), "{var} must be scrubbed on the default client");
        }
        // `harden()`-only surface is absent on a plain client.
        assert!(
            !has_key("GIT_SSH_COMMAND"),
            "command-hook scrub is harden()-only"
        );
        assert!(
            !has_key("GIT_CONFIG_NOSYSTEM"),
            "config pins are harden()-only"
        );
    }

    // RefName/RevSpec accept/reject tables.
    #[test]
    fn ref_name_and_rev_spec_validate() {
        for ok in ["main", "feature/x", "v1.2.3", "a-b_c"] {
            assert!(RefName::new(ok).is_ok(), "{ok}");
        }
        for bad in [
            "", "-evil", ".hidden", "a..b", "a b", "a~b", "a^b", "a:b", "a?b", "a*b", "a[b",
            "a\\b", "end/", "x.lock",
        ] {
            assert!(RefName::new(bad).is_err(), "{bad:?} must be rejected");
        }
        assert!(RevSpec::new("HEAD~2").is_ok());
        assert!(RevSpec::new("main..feature").is_ok());
        assert!(RevSpec::new("-evil").is_err());
        assert!(RevSpec::new("").is_err());
    }

    // capabilities parses real-world version shapes (incl. the Windows build
    // trailer) and gates on the major floor only.
    #[tokio::test]
    async fn capabilities_parse_and_gate_versions() {
        let gh = Git::with_runner(ScriptedRunner::new().on(
            ["git", "--version"],
            Reply::ok("git version 2.54.0.windows.1\n"),
        ));
        let caps = gh.capabilities().await.expect("capabilities");
        assert_eq!(caps.version.to_string(), "2.54.0");
        assert!(caps.is_supported());
        caps.ensure_supported().expect("supported");

        // Two-part versions parse (patch defaults to 0); an ancient major fails
        // the gate with a clear message.
        let old = Git::with_runner(
            ScriptedRunner::new().on(["git", "--version"], Reply::ok("git version 1.9\n")),
        );
        let caps = old.capabilities().await.expect("capabilities");
        assert_eq!(
            caps.version,
            GitVersion {
                major: 1,
                minor: 9,
                patch: 0
            }
        );
        let err = caps.ensure_supported().expect_err("unsupported");
        // The message must name the floor and the found version.
        let Error::Spawn { source, .. } = &err else {
            panic!("expected Spawn, got {err:?}");
        };
        let message = source.to_string();
        assert!(message.contains(">= 2"), "names the floor: {message}");
        assert!(
            message.contains("1.9.0"),
            "names the found version: {message}"
        );

        // Garbage output is a parse error, not a silent zero version.
        let garbage = Git::with_runner(
            ScriptedRunner::new().on(["git", "--version"], Reply::ok("not a version")),
        );
        assert!(matches!(
            garbage.capabilities().await.unwrap_err(),
            Error::Parse { .. }
        ));
    }

    // clone_repo is dir-less and appends only the requested flags.
    #[tokio::test]
    async fn clone_repo_builds_flags_and_runs_dirless() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.clone_repo(
            "https://example.com/r.git",
            Path::new("/dest"),
            CloneSpec::new().branch("main").depth(1).bare(),
        )
        .await
        .expect("clone");
        let call = rec.only_call();
        assert_eq!(
            call.args_str(),
            [
                "clone",
                "--branch",
                "main",
                "--depth",
                "1",
                "--bare",
                "https://example.com/r.git",
                "/dest"
            ]
        );
        assert_eq!(call.cwd, None, "clone runs without a working directory");

        let bare = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&bare);
        git.clone_repo("u", Path::new("/d"), CloneSpec::new())
            .await
            .expect("clone");
        assert_eq!(bare.only_call().args_str(), ["clone", "u", "/d"]);
    }

    // R7: a failed clone cleans a `dest` it could have *created* (absent or empty) so
    // a retry isn't blocked by "destination already exists and is not empty" — but it
    // must NEVER delete a non-empty pre-existing dir (git would have refused, so the
    // caller's data is untouched). Scripted-fail clone + real temp dirs (only the fs
    // cleanup is real; nothing spawns).
    #[tokio::test]
    async fn clone_failure_cleans_only_a_dest_it_could_have_created() {
        use vcs_testkit::TempDir;
        let tmp = TempDir::new("r7-clone");
        let git = Git::with_runner(ScriptedRunner::new().on(
            ["git", "clone"],
            Reply::fail(
                128,
                "fatal: could not read Username for 'https://x': prompts disabled",
            ),
        ));

        // A non-empty caller dir must survive a failed clone.
        let occupied = tmp.path().join("occupied");
        std::fs::create_dir(&occupied).unwrap();
        std::fs::write(occupied.join("keep.txt"), b"caller data").unwrap();
        assert!(
            git.clone_repo("https://x/r", &occupied, CloneSpec::new())
                .await
                .is_err()
        );
        assert!(
            occupied.join("keep.txt").exists(),
            "a non-empty caller dir must survive a failed clone"
        );

        // An empty dest we could have populated is removed on failure.
        let empty = tmp.path().join("empty");
        std::fs::create_dir(&empty).unwrap();
        assert!(
            git.clone_repo("https://x/r", &empty, CloneSpec::new())
                .await
                .is_err()
        );
        assert!(
            !empty.exists(),
            "an empty dest is cleaned so a retry isn't blocked"
        );

        // A pre-existing FILE at `dest` must survive (read_dir errs → cleanable, but
        // remove_dir_all refuses a non-dir). Pins that a future "also remove a file"
        // change can't slip in unnoticed.
        let file_dest = tmp.path().join("a-file");
        std::fs::write(&file_dest, b"caller file").unwrap();
        assert!(
            git.clone_repo("https://x/r", &file_dest, CloneSpec::new())
                .await
                .is_err()
        );
        assert!(
            file_dest.exists() && std::fs::read(&file_dest).unwrap() == b"caller file",
            "a caller's file at dest must survive a failed clone"
        );

        // A symlink `dest` → an EMPTY dir the caller owns: `read_dir` follows the
        // link and sees empty, so `cleanable` is true and `remove_dir_all` DOES run on
        // the link — it must unlink only the symlink, never delete THROUGH it. The
        // target dir (and a sibling sentinel) must survive.
        #[cfg(unix)]
        {
            let target = tmp.path().join("link-target"); // stays empty → cleanable path
            std::fs::create_dir(&target).unwrap();
            let sentinel = tmp.path().join("sibling.txt");
            std::fs::write(&sentinel, b"untouched").unwrap();
            let link = tmp.path().join("a-symlink");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(
                git.clone_repo("https://x/r", &link, CloneSpec::new())
                    .await
                    .is_err()
            );
            assert!(
                target.exists() && sentinel.exists(),
                "a failed clone must unlink at most the symlink, never delete through it"
            );
        }
    }

    #[tokio::test]
    async fn tag_methods_build_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.tag_create(Path::new("/r"), "v1", None).await.unwrap();
        git.tag_create(Path::new("/r"), "v1", Some("abc".into()))
            .await
            .unwrap();
        git.tag_create_annotated(Path::new("/r"), AnnotatedTag::new("v2", "notes"))
            .await
            .unwrap();
        git.tag_delete(Path::new("/r"), "v1").await.unwrap();
        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), ["tag", "v1"]);
        assert_eq!(calls[1].args_str(), ["tag", "v1", "abc"]);
        assert_eq!(calls[2].args_str(), ["tag", "-a", "v2", "-m", "notes"]);
        assert_eq!(calls[3].args_str(), ["tag", "-d", "v1"]);
    }

    #[tokio::test]
    async fn tag_list_splits_lines() {
        let git = Git::with_runner(
            ScriptedRunner::new().on(["git", "tag", "--list"], Reply::ok("v1\nv2.0\n")),
        );
        assert_eq!(git.tag_list(Path::new(".")).await.unwrap(), ["v1", "v2.0"]);
    }

    // The line-parsed list commands must pass `--no-column`: a user's
    // `column.ui = always` would pack several names per line, and
    // `color.{ui,branch} = always` would inject ANSI escapes — both even when
    // piped. Branch listings disable both; `git tag` isn't colorized, so it only
    // needs `--no-column`.
    #[tokio::test]
    async fn list_commands_disable_column_and_color() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.branches(Path::new(".")).await.unwrap();
        git.is_merged(Path::new("."), "b", "main").await.unwrap();
        git.tag_list(Path::new(".")).await.unwrap();
        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), ["branch", "--no-column", "--no-color"]);
        assert_eq!(
            calls[1].args_str(),
            ["branch", "--merged", "main", "--no-column", "--no-color"]
        );
        assert_eq!(calls[2].args_str(), ["tag", "--list", "--no-column"]);
    }

    // Commands whose failure output feeds the error classifiers must force the
    // C locale — a translated message would defeat the substring matching.
    #[tokio::test]
    async fn classified_commands_force_c_locale() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.commit(Path::new("."), "msg").await.unwrap();
        git.merge_commit(Path::new("."), MergeCommit::branch("b"))
            .await
            .unwrap();
        git.merge_squash(Path::new("."), "b").await.unwrap();
        git.merge_no_commit(Path::new("."), MergeNoCommit::branch("b"))
            .await
            .unwrap();
        git.cherry_pick(Path::new("."), "abc").await.unwrap();
        git.stash_pop(Path::new(".")).await.unwrap();
        git.fetch(Path::new(".")).await.unwrap();
        for call in rec.calls() {
            assert!(
                call.envs.iter().any(|(k, v)| {
                    k.to_str() == Some("LC_ALL")
                        && v.as_deref().and_then(|o| o.to_str()) == Some("C")
                }),
                "{:?} should force LC_ALL=C",
                call.args_str()
            );
        }
    }

    // The `<rev>:<path>` spec requires forward slashes — Windows callers may
    // hand in backslashes. The normalisation is Windows-only.
    #[cfg(windows)]
    #[tokio::test]
    async fn show_file_normalises_path_separators() {
        let rec = RecordingRunner::replying(Reply::ok("content\n"));
        let git = Git::with_runner(&rec);
        let out = git
            .show_file(Path::new("/r"), "HEAD", "sub\\dir\\f.txt")
            .await
            .expect("show_file");
        // The blob's trailing newline is preserved verbatim (H7) — not trimmed.
        assert_eq!(out, "content\n");
        assert_eq!(rec.only_call().args_str(), ["show", "HEAD:sub/dir/f.txt"]);
    }

    // H7: content verbs return git's output byte-for-byte — the round-trip-corrupting
    // cases are multiple trailing newlines and a missing final newline.
    #[tokio::test]
    async fn content_verbs_preserve_exact_trailing_bytes() {
        for raw in ["a\nb\n\n", "no-final-newline", "trailing spaces   \n"] {
            let rec = RecordingRunner::replying(Reply::ok(raw));
            let git = Git::with_runner(&rec);
            let out = git
                .show_file(Path::new("/r"), "HEAD", "f.txt")
                .await
                .expect("show_file");
            assert_eq!(out, raw, "show_file returns bytes verbatim");
        }
        // diff_text is verbatim too (its trailing blank context line must survive so
        // the last hunk stays in sync with its `@@` count).
        let diff = "diff --git a/f b/f\n@@ -1,2 +1,2 @@\n-x\n+y\n \n";
        let rec = RecordingRunner::replying(Reply::ok(diff));
        let git = Git::with_runner(&rec);
        assert_eq!(
            git.diff_text(Path::new("/r"), DiffSpec::Rev("HEAD".into()))
                .await
                .expect("diff_text"),
            diff
        );
    }

    // On Unix a backslash is a legal filename byte — the spec must pass through
    // verbatim so a literal `a\b.txt` stays resolvable.
    #[cfg(not(windows))]
    #[tokio::test]
    async fn show_file_keeps_backslashes_on_unix() {
        let rec = RecordingRunner::replying(Reply::ok("content\n"));
        let git = Git::with_runner(&rec);
        git.show_file(Path::new("/r"), "HEAD", "sub\\dir\\f.txt")
            .await
            .expect("show_file");
        assert_eq!(rec.only_call().args_str(), ["show", "HEAD:sub\\dir\\f.txt"]);
    }

    // config --get: exit 0 → Some(value), exit 1 → None (unset), other → error.
    #[tokio::test]
    async fn config_get_maps_exit_codes() {
        let set = Git::with_runner(
            ScriptedRunner::new().on(["git", "config", "--get"], Reply::ok("Alice\n")),
        );
        assert_eq!(
            set.config_get(Path::new("."), "user.name").await.unwrap(),
            Some("Alice".to_string())
        );
        // Only git's trailing newline (here `\r\n`) is stripped — a value's own
        // trailing spaces are preserved (they can be meaningful).
        let spaced = Git::with_runner(
            ScriptedRunner::new().on(["git", "config", "--get"], Reply::ok("prefix:  \r\n")),
        );
        assert_eq!(
            spaced.config_get(Path::new("."), "x.y").await.unwrap(),
            Some("prefix:  ".to_string())
        );
        let unset = Git::with_runner(
            ScriptedRunner::new().on(["git", "config", "--get"], Reply::fail(1, "")),
        );
        assert_eq!(
            unset.config_get(Path::new("."), "user.name").await.unwrap(),
            None
        );
        // A multi-valued key (exit 2) or worse is a real error.
        let multi = Git::with_runner(ScriptedRunner::new().on(
            ["git", "config", "--get"],
            Reply::fail(2, "multiple values"),
        ));
        assert!(
            multi
                .config_get(Path::new("."), "remote.all")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn blame_builds_rev_before_pathspec_separator() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.blame(Path::new("/r"), "src/lib.rs", Some("HEAD~1".into()))
            .await
            .unwrap();
        git.blame(Path::new("/r"), "src/lib.rs", None)
            .await
            .unwrap();
        let calls = rec.calls();
        assert_eq!(
            calls[0].args_str(),
            ["blame", "--line-porcelain", "HEAD~1", "--", "src/lib.rs"]
        );
        assert_eq!(
            calls[1].args_str(),
            ["blame", "--line-porcelain", "--", "src/lib.rs"]
        );
    }

    // revert must never open an editor: --no-edit plus the env backstop.
    #[tokio::test]
    async fn sequencer_methods_suppress_editors() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.revert(Path::new("/r"), "abc").await.unwrap();
        git.cherry_pick(Path::new("/r"), "abc").await.unwrap();
        git.rebase_skip(Path::new("/r")).await.unwrap();
        let calls = rec.calls();
        assert_eq!(calls[0].args_str(), ["revert", "--no-edit", "abc"]);
        assert_eq!(calls[1].args_str(), ["cherry-pick", "abc"]);
        assert_eq!(calls[2].args_str(), ["rebase", "--skip"]);
        for call in &calls {
            assert!(
                call.envs
                    .iter()
                    .any(|(k, _)| k.to_str() == Some("GIT_EDITOR")),
                "editor suppressed on {:?}",
                call.args_str()
            );
        }
    }

    // harden() scrubs GIT_EDITOR/GIT_SEQUENCE_EDITOR from the inherited
    // environment, but a sequencer command sets its own `GIT_EDITOR=true` per call
    // (no_editor). `command_in` applies the client-level removal eagerly, so the env
    // list ends up `[…, (GIT_EDITOR, None), …, (GIT_EDITOR, "true")]` — and at spawn
    // each op is applied in order (processkit's `Command` does `env_remove` then
    // `env`), so the LAST write wins. The effective value MUST be `true`, else a
    // hardened `revert`/`cherry-pick`/`rebase` would lose its no-op editor and hang
    // a headless caller. Pin that effective-precedence (fold in spawn order).
    #[tokio::test]
    async fn hardened_sequencer_keeps_its_no_op_editor() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec).harden();
        git.revert(Path::new("/r"), "abc").await.unwrap();
        let call = rec.only_call();
        // Resolve each editor var the way the OS does: last op for the key wins.
        let effective = |var: &str| {
            call.envs
                .iter()
                .rfind(|(k, _)| k.to_str() == Some(var))
                .and_then(|(_, v)| v.as_deref())
                .and_then(|v| v.to_str())
        };
        // Both no-op editors must survive harden()'s scrub (symmetric precedence),
        // else a hardened sequencer command hangs a headless caller.
        assert_eq!(
            effective("GIT_EDITOR"),
            Some("true"),
            "the per-command no-op editor must survive harden()'s scrub"
        );
        assert_eq!(
            effective("GIT_SEQUENCE_EDITOR"),
            Some("true"),
            "the per-command no-op sequence editor must survive harden()'s scrub"
        );
    }

    #[tokio::test]
    async fn remote_add_and_set_url_build_args() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.remote_add(Path::new("/r"), "up", "https://x/y.git")
            .await
            .unwrap();
        git.remote_set_url(Path::new("/r"), "up", "https://x/z.git")
            .await
            .unwrap();
        let calls = rec.calls();
        assert_eq!(
            calls[0].args_str(),
            ["remote", "add", "up", "https://x/y.git"]
        );
        assert_eq!(
            calls[1].args_str(),
            ["remote", "set-url", "up", "https://x/z.git"]
        );
    }

    // Dirty tree: stash -u → checkout → pop, in that order.
    #[tokio::test]
    async fn switch_with_stash_round_trips_dirty_tree() {
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["git", "status"], Reply::ok(" M a.rs\0"))
                .on(["git", "stash", "push"], Reply::ok(""))
                .on(["git", "checkout"], Reply::ok(""))
                .on(["git", "stash", "pop"], Reply::ok("")),
        );
        let git = Git::with_runner(&rec);
        git.switch_with_stash(Path::new("/r"), "feature")
            .await
            .expect("switch");
        let calls = rec.calls();
        assert_eq!(calls.len(), 4);
        assert_eq!(
            calls[1].args_str(),
            ["stash", "push", "--include-untracked"]
        );
        assert_eq!(calls[2].args_str(), ["checkout", "feature", "--"]);
        assert_eq!(calls[3].args_str(), ["stash", "pop"]);
    }

    // A clean tree skips the stash round-trip — a no-op `stash push` would make
    // the later pop grab an older, unrelated stash.
    #[tokio::test]
    async fn switch_with_stash_skips_stash_on_clean_tree() {
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["git", "status"], Reply::ok(""))
                .on(["git", "checkout"], Reply::ok("")),
        );
        let git = Git::with_runner(&rec);
        git.switch_with_stash(Path::new("/r"), "feature")
            .await
            .expect("switch");
        let calls = rec.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|c| c.args_str()[0] != "stash"));
    }

    // A failed checkout pops the stash back (we are still on the original
    // branch) and surfaces the checkout error.
    #[tokio::test]
    async fn switch_with_stash_restores_on_checkout_failure() {
        let rec = RecordingRunner::new(
            ScriptedRunner::new()
                .on(["git", "status"], Reply::ok(" M a.rs\0"))
                .on(["git", "stash", "push"], Reply::ok(""))
                .on(
                    ["git", "checkout"],
                    Reply::fail(1, "error: pathspec 'nope'"),
                )
                .on(["git", "stash", "pop"], Reply::ok("")),
        );
        let git = Git::with_runner(&rec);
        let err = git
            .switch_with_stash(Path::new("/r"), "nope")
            .await
            .expect_err("checkout error must surface");
        assert!(matches!(err, Error::Exit { .. }));
        let calls = rec.calls();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[3].args_str(), ["stash", "pop"], "restoring pop ran");
    }

    // `fetch_from` names the remote, keeps the prompt off, and shares the
    // transient retry.
    #[tokio::test]
    async fn fetch_from_builds_args_and_retries() {
        let rec = RecordingRunner::replying(Reply::ok(""));
        let git = Git::with_runner(&rec);
        git.fetch_from(Path::new("/r"), "upstream")
            .await
            .expect("fetch_from");
        let call = rec.only_call();
        assert_eq!(call.args_str(), ["fetch", "--quiet", "upstream"]);
        assert!(call.envs.iter().any(|(k, v)| {
            k.to_str() == Some("GIT_TERMINAL_PROMPT")
                && v.as_deref().and_then(|o| o.to_str()) == Some("0")
        }));

        let failing = RecordingRunner::replying(Reply::fail(128, "fatal: Connection timed out"));
        let git = Git::with_runner(&failing);
        assert!(git.fetch_from(Path::new("/r"), "upstream").await.is_err());
        assert_eq!(failing.calls().len(), FETCH_ATTEMPTS as usize);
    }

    // The consumer-facing mock seam: a function depending on `&dyn GitApi` is
    // tested with a generated mock.
    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn consumer_mocks_the_interface() {
        async fn on_branch(git: &dyn GitApi, want: &str) -> bool {
            git.current_branch(Path::new(".")).await.unwrap().as_deref() == Some(want)
        }
        let mut mock = MockGitApi::new();
        mock.expect_current_branch()
            .returning(|_| Ok(Some("main".to_string())));
        assert!(on_branch(&mock, "main").await);
    }
}

// Long-form how-to guides, rendered from this crate's docs/*.md on docs.rs.
#[doc = include_str!("../docs/git.md")]
#[allow(rustdoc::broken_intra_doc_links)]
pub mod guide {
    #[doc = include_str!("../docs/security.md")]
    #[allow(rustdoc::broken_intra_doc_links)]
    pub mod security {}
    #[doc = include_str!("../docs/conflicts.md")]
    #[allow(rustdoc::broken_intra_doc_links)]
    pub mod conflicts {}
}
