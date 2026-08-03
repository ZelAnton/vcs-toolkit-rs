//! Command specifications, validated revision types, and capability metadata.

use super::*;

/// Options for [`GitApi::sparse_checkout_set`] (`git sparse-checkout set`).
///
/// `#[non_exhaustive]`, so build it through [`SparseCheckoutSet::new`] and the
/// [`non_cone`](SparseCheckoutSet::non_cone) setter rather than a bare boolean
/// that would make the selected matching mode ambiguous at the call site.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SparseCheckoutSet {
    /// Directories in cone mode, or gitignore-style patterns in non-cone mode.
    /// The list must contain at least one non-empty, non-flag-like value.
    pub patterns: Vec<String>,
    /// Whether git should interpret `patterns` as cone directories (`--cone`).
    pub cone: bool,
}

impl SparseCheckoutSet {
    /// Set sparse checkout to `patterns` in cone mode (`--cone`).
    pub fn new(patterns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            patterns: patterns.into_iter().map(Into::into).collect(),
            cone: true,
        }
    }

    /// Interpret the supplied values as gitignore-style patterns
    /// (`--no-cone`) instead of cone directories.
    pub fn non_cone(mut self) -> Self {
        self.cone = false;
        self
    }
}

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
    pub new_branch: Option<RefName>,
    /// The commit/branch to base the worktree on; `None` defaults to `HEAD`.
    pub commitish: Option<RevSpec>,
    /// Register the worktree without populating its files (`--no-checkout`) — the
    /// caller fills the working tree itself (e.g. a copy-on-write clone).
    pub no_checkout: bool,
}

impl WorktreeAdd {
    /// A worktree at `path` checking out an existing `commitish` (e.g. a branch):
    /// `git worktree add <path> <commitish>`.
    pub fn checkout(path: impl Into<PathBuf>, commitish: RevSpec) -> Self {
        Self {
            path: path.into(),
            new_branch: None,
            commitish: Some(commitish),
            no_checkout: false,
        }
    }

    /// A worktree at `path` creating a new branch `name` based on `commitish`:
    /// `git worktree add -b <name> <path> <commitish>`.
    pub fn create_branch(path: impl Into<PathBuf>, name: RefName, commitish: RevSpec) -> Self {
        Self {
            path: path.into(),
            new_branch: Some(name),
            commitish: Some(commitish),
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
    pub fn branch(name: RefName) -> Self {
        Self {
            remote: "origin".to_string(),
            refspec: name.as_str().to_string(),
            set_upstream: false,
        }
    }

    /// Push `local` to a differently-named `remote_branch`
    /// (`git push origin <local>:<remote_branch>`). Both sides are validated
    /// [`RefName`]s, so the single `:` is always the API-inserted separator — a
    /// caller cannot smuggle an extra ref or a force (`+`) through them.
    pub fn refspec(local: &RefName, remote_branch: &RefName) -> Self {
        Self {
            remote: "origin".to_string(),
            refspec: format!("{}:{}", local.as_str(), remote_branch.as_str()),
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

/// The partial-clone object filter for [`GitApi::clone_repo`] (`git clone
/// --filter=<value>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CloneFilter {
    /// Omit blobs until they are needed (`--filter=blob:none`).
    BlobNone,
    /// Omit trees until they are needed (`--filter=tree:0`).
    TreeZero,
}

impl CloneFilter {
    pub(crate) fn cli_value(self) -> &'static str {
        match self {
            Self::BlobNone => "blob:none",
            Self::TreeZero => "tree:0",
        }
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
    /// Use a partial-clone object filter (`--filter=blob:none` or
    /// `--filter=tree:0`).
    pub filter: Option<CloneFilter>,
    /// Limit the clone to the selected branch (`--single-branch`).
    pub single_branch: bool,
    /// Name the remote created by the clone instead of `origin` (`--origin`).
    /// The value is checked before spawning git.
    pub origin: Option<String>,
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

    /// Use a partial clone filter (`--filter=<value>`).
    pub fn filter(mut self, filter: CloneFilter) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Restrict the clone to the selected branch (`--single-branch`).
    pub fn single_branch(mut self) -> Self {
        self.single_branch = true;
        self
    }

    /// Name the clone's remote `name` instead of `origin` (`--origin <name>`).
    /// The value is rejected before spawning if it is empty, flag-like, or
    /// contains an embedded NUL.
    pub fn origin(mut self, name: impl Into<String>) -> Self {
        self.origin = Some(name.into());
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

/// Partial [`MergeCheck`] — names the branch being tested; chain
/// [`into_base`](MergeCheckPartial::into_base) to name the base it must be merged into.
#[derive(Debug, Clone)]
pub struct MergeCheckPartial {
    branch: RefName,
}

impl MergeCheckPartial {
    /// The base commit-ish `branch` should be fully merged **into**.
    pub fn into_base(self, base: RevSpec) -> MergeCheck {
        MergeCheck {
            branch: self.branch,
            base,
        }
    }
}

/// A "is `branch` fully merged into `base`?" check for [`GitApi::is_merged`].
///
/// Built as `MergeCheck::branch(RefName::new("feature")?).into_base(RevSpec::new("main")?)` — the two same-typed
/// refs are named across **two** builder steps, so they can't be silently transposed
/// (a swap would *invert* the answer). `#[non_exhaustive]`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MergeCheck {
    /// The branch/ref being tested for having been merged.
    pub branch: RefName,
    /// The base commit-ish it should be fully merged into.
    pub base: RevSpec,
}

impl MergeCheck {
    /// Name the `branch` to test; chain [`into_base`](MergeCheckPartial::into_base).
    pub fn branch(name: RefName) -> MergeCheckPartial {
        MergeCheckPartial { branch: name }
    }
}

/// Options for [`GitApi::merge_commit`] (`git merge` that commits the result).
///
/// `#[non_exhaustive]`, so build it through [`MergeCommit::branch`] and the
/// chained setters rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MergeCommit {
    /// The commit-ish to merge in.
    pub branch: RevSpec,
    /// Always create a merge commit, even when a fast-forward was possible
    /// (`--no-ff`).
    pub no_ff: bool,
    /// The merge commit message (`-m`); `None` takes the default message
    /// non-interactively (`--no-edit`).
    pub message: Option<String>,
}

impl MergeCommit {
    /// Merge `target` taking the default merge message non-interactively
    /// (`git merge --no-edit <target>`).
    pub fn branch(target: RevSpec) -> Self {
        Self {
            branch: target,
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
    /// The commit-ish to merge in.
    pub branch: RevSpec,
    /// Stage the squashed result without recording `MERGE_HEAD` (`--squash`);
    /// takes precedence over `no_ff` (git rejects the pair).
    pub squash: bool,
    /// Always record a real (abortable) merge, even when a fast-forward was
    /// possible (`--no-ff`).
    pub no_ff: bool,
}

impl MergeNoCommit {
    /// Merge `target` but stop before committing (`git merge --no-commit <target>`).
    pub fn branch(target: RevSpec) -> Self {
        Self {
            branch: target,
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
    pub name: RefName,
    /// The tag message (`-m`).
    pub message: String,
    /// The revision to tag (`<rev>`); `None` tags `HEAD`.
    pub rev: Option<RevSpec>,
}

impl AnnotatedTag {
    /// An annotated tag `name` with `message` at `HEAD`
    /// (`git tag -a <name> -m <message>`).
    pub fn new(name: RefName, message: impl Into<String>) -> Self {
        Self {
            name,
            message: message.into(),
            rev: None,
        }
    }

    /// Tag `r` instead of `HEAD`.
    pub fn rev(mut self, r: RevSpec) -> Self {
        self.rev = Some(r);
        self
    }
}

/// Options for [`GitApi::delete_branch`] (`git branch -d`/`-D`).
///
/// `#[non_exhaustive]`, so build it through [`BranchDelete::new`] and the chained
/// [`force`](BranchDelete::force) setter rather than a struct literal — a bare
/// `bool` at the call site (`delete_branch(name, true)`) doesn't say what `true`
/// means, and this leaves room to add options without a breaking signature change.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BranchDelete {
    /// The local branch name to delete.
    pub name: RefName,
    /// Delete even if not fully merged — `git branch -D` vs `-d`.
    pub force: bool,
}

impl BranchDelete {
    /// Delete branch `name`; not forced (git refuses an unmerged branch).
    pub fn new(name: RefName) -> Self {
        Self { name, force: false }
    }

    /// Delete even if not fully merged (`-D`).
    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }
}

/// Options for [`GitApi::stash_push`] (`git stash push`).
///
/// `#[non_exhaustive]`, so build it through [`StashPush::new`] and the chained
/// [`include_untracked`](StashPush::include_untracked) setter rather than a bare
/// `bool` (`stash_push(dir, true)` doesn't say what `true` selects).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StashPush {
    /// Also stash untracked files (`--include-untracked`).
    pub include_untracked: bool,
}

impl StashPush {
    /// Stash the tracked working-tree changes only.
    pub fn new() -> Self {
        Self::default()
    }

    /// Also stash untracked files (`--include-untracked`).
    pub fn include_untracked(mut self) -> Self {
        self.include_untracked = true;
        self
    }
}

/// How [`Clean`] treats ignored files/directories — the `-x`/`-X` axis of
/// `git clean`, orthogonal to [`directories`](Clean::directories).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum CleanIgnored {
    /// Leave ignored files alone (git's default): only untracked-and-not-ignored
    /// entries are candidates.
    #[default]
    Exclude,
    /// Also remove ignored files/directories (`-x`), in addition to the
    /// ordinary untracked ones.
    Include,
    /// Remove **only** ignored files/directories (`-X`) — untracked-but-not-ignored
    /// entries are left alone.
    Only,
}

/// Options for [`GitApi::clean`] (`git clean`) — deletes untracked files from
/// the working tree.
///
/// **Force is a deliberate, explicit call, never a default.** There is no
/// `Clean::new()` state, nor any other setter, that arms deletion by
/// itself — only the explicit [`force`](Clean::force) call does, the same
/// "no bare `bool`, no implied default" pattern [`BranchDelete::force`] and
/// [`WorktreeRemove::force`] use for their own destructive flag. Independently,
/// [`GitApi::clean`] itself refuses to run at all — before spawning `git` —
/// unless the spec picked **either** [`dry_run`](Clean::dry_run) **or**
/// [`force`](Clean::force): this crate's own guard, so the outcome never
/// depends on whether the caller's `clean.requireForce` git config happens to
/// be (mis)set to `false`. When [`dry_run`](Clean::dry_run) is set, `force` is
/// ignored — dry-run always wins, so it is never possible to accidentally
/// delete while asking for a preview.
///
/// `#[non_exhaustive]`, so build it through [`Clean::new`] and the chained
/// setters rather than a struct literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Clean {
    /// Actually delete rather than merely report (`--force`/`-f`). Ignored
    /// when [`dry_run`](Self::dry_run) is also set.
    pub force: bool,
    /// Report what *would* be deleted, without deleting anything (`--dry-run`/
    /// `-n`). Takes priority over [`force`](Self::force).
    pub dry_run: bool,
    /// Remove whole untracked directories, not just untracked files (`-d`).
    pub directories: bool,
    /// How ignored files/directories are treated; see [`CleanIgnored`].
    pub ignored: CleanIgnored,
}

impl Clean {
    /// A clean spec with neither [`dry_run`](Self::dry_run) nor
    /// [`force`](Self::force) picked yet — passing this as-is to
    /// [`GitApi::clean`] is refused before spawning (see the type docs); chain
    /// one of the two setters before use.
    pub fn new() -> Self {
        Self::default()
    }

    /// Actually delete (`--force`/`-f`) — the one, explicit way to arm
    /// deletion; see the type docs.
    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }

    /// Report what would be deleted without deleting anything (`--dry-run`/`-n`).
    pub fn dry_run(mut self) -> Self {
        self.dry_run = true;
        self
    }

    /// Remove whole untracked directories too (`-d`).
    pub fn directories(mut self) -> Self {
        self.directories = true;
        self
    }

    /// Also remove ignored files/directories (`-x`), in addition to ordinary
    /// untracked ones.
    pub fn include_ignored(mut self) -> Self {
        self.ignored = CleanIgnored::Include;
        self
    }

    /// Remove **only** ignored files/directories (`-X`).
    pub fn only_ignored(mut self) -> Self {
        self.ignored = CleanIgnored::Only;
        self
    }
}

/// Options for [`GitApi::worktree_remove`] (`git worktree remove`).
///
/// `#[non_exhaustive]`, so build it through [`WorktreeRemove::new`] and the chained
/// [`force`](WorktreeRemove::force) setter rather than a struct literal — a bare
/// `bool` (`worktree_remove(path, true)`) doesn't say what `true` means.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WorktreeRemove {
    /// The attached worktree path to remove.
    pub path: PathBuf,
    /// Remove even when the worktree has uncommitted changes (`--force`).
    pub force: bool,
}

impl WorktreeRemove {
    /// Remove the worktree at `path`; not forced (git refuses a dirty one).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            force: false,
        }
    }

    /// Remove even when the worktree has uncommitted changes (`--force`).
    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }
}

/// A `git submodule update` specification — checks out the submodules recorded
/// in the superproject's index to the commits it pins, optionally initializing
/// (`--init`) and recursing (`--recursive`) first, and optionally scoped to
/// specific paths. Built fluently; see [`GitApi::submodule_update`].
///
/// **This is the one submodule verb that materializes and executes a *different*
/// (nested) repository's content** — with `init`, it clones/fetches each
/// submodule from the URL its `.gitmodules` records and checks out its working
/// tree. Treat those nested repos with the same untrusted-repo caution as the
/// superproject; see the submodules section of the security guide.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct SubmoduleUpdate {
    /// Initialize (register + clone) submodules not yet set up (`--init`).
    pub init: bool,
    /// Recurse into nested submodules (`--recursive`).
    pub recursive: bool,
    /// Create a shallow clone with this history depth (`--depth <n>`); `None`
    /// leaves the depth unset (full history).
    pub depth: Option<u32>,
    /// Restrict the update to these repo-relative submodule paths; empty means
    /// every submodule. Passed after a `--` terminator so a path can never be
    /// parsed as a flag.
    pub paths: Vec<String>,
}

impl SubmoduleUpdate {
    /// A plain update (no `--init`/`--recursive`/`--depth`, all submodules).
    pub fn new() -> Self {
        Self::default()
    }

    /// Register and clone submodules that are not yet initialized (`--init`).
    pub fn init(mut self) -> Self {
        self.init = true;
        self
    }

    /// Recurse into nested submodules (`--recursive`).
    pub fn recursive(mut self) -> Self {
        self.recursive = true;
        self
    }

    /// Make a shallow checkout with history `depth` (`--depth <n>`).
    pub fn depth(mut self, depth: u32) -> Self {
        self.depth = Some(depth);
        self
    }

    /// Scope the update to one submodule `path` (repeatable). With no path set,
    /// the update covers every submodule.
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.paths.push(path.into());
        self
    }

    /// Scope the update to several submodule `paths` (appended to any already set).
    pub fn paths(mut self, paths: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.paths.extend(paths.into_iter().map(Into::into));
        self
    }
}

/// A validated git reference name (branch/tag/remote-tracking ref). Every
/// [`GitApi`] operation that names a branch, tag, or ref to **create, delete,
/// rename, or look up by exact name** takes a `RefName` (directly or inside its
/// options struct), so a name from untrusted input (UIs, bots, agents) is
/// validated once, at construction, and the type — not an internal guard — is
/// the argv-injection barrier from then on. For a general commit-ish or range
/// (`checkout`, `reset_hard`, `log`, `diff` ranges, …) use the more permissive
/// [`RevSpec`] instead.
///
/// Rules follow the load-bearing core of `git check-ref-format`: non-empty,
/// no leading `-` or `.`, no `..`, no control characters or space, none of
/// `~ ^ : ? * [ \`, no trailing `/` or `.lock`. A rejected name is an
/// [`vcs_cli_support::is_invalid_input`] failure.
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
            return Err(Error::spawn(
                BINARY,
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid git reference name: {name:?}"),
                ),
            ));
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

/// A validated revision/range expression (`HEAD~2`, `main..feature`). Every
/// [`GitApi`] operation that resolves a general **commit-ish or range** takes a
/// `RevSpec`, so an untrusted revision is validated once, at construction.
/// Deliberately *minimal* — git's revision grammar is too rich to validate
/// here — it only guarantees the expression is non-empty and cannot be parsed
/// as a flag (no leading `-`). For a value that must be a genuine ref **name**
/// (to create/delete/rename a branch or tag) use the stricter [`RefName`]. A
/// rejected expression is an [`vcs_cli_support::is_invalid_input`] failure.
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

impl std::str::FromStr for RefName {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Self::new(s)
    }
}

impl std::str::FromStr for RevSpec {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Self::new(s)
    }
}

/// The typed result of one `git bisect` classification step.
///
/// The `bisect_start`, `bisect_good`, `bisect_bad`, and `bisect_skip` methods
/// all return this value after Git has classified the current checkout. A
/// [`NextCandidate`](Self::NextCandidate) means that Git moved the worktree to
/// the returned revision and the consumer should run its test there. A
/// [`FirstBad`](Self::FirstBad) means that Git finished the search; the
/// returned revision is the first bad commit and no further classification is
/// needed. The consumer owns the test loop and should call
/// [`GitApi::bisect_reset`] when it is done, including on an error or
/// cancellation.
///
/// This type is Git-only. It is deliberately not part of `vcs-core` because
/// Jujutsu has no corresponding bisect command.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BisectStep {
    /// Git checked out another revision for the consumer to test.
    NextCandidate {
        /// The revision Git selected and checked out.
        revision: RevSpec,
    },
    /// Git completed the search and identified the first bad revision.
    FirstBad {
        /// The first bad revision reported by Git.
        revision: RevSpec,
    },
}

impl BisectStep {
    /// The revision Git selected for this step.
    pub fn revision(&self) -> &RevSpec {
        match self {
            Self::NextCandidate { revision } | Self::FirstBad { revision } => revision,
        }
    }

    /// Whether this step completed the bisect search with a first bad commit.
    pub fn is_first_bad(&self) -> bool {
        matches!(self, Self::FirstBad { .. })
    }
}

/// Backward-readable alias for [`BisectStep`]. Prefer `BisectStep` in new code.
pub type BisectResult = BisectStep;

/// What [`GitApi::checkout`] switches to: a validated ref/revision, or git's `-`
/// "previous branch" shortcut.
///
/// `-` is the one place a leading-`-` token is legitimate — it is git's
/// `@{-1}` shorthand, not caller-controlled argv — so it is modelled as a
/// distinct [`Previous`](CheckoutTarget::Previous) variant emitting a fixed
/// literal, rather than punching a hole in [`RevSpec`]'s no-leading-`-`
/// invariant (which the other commit-ish operations rely on).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckoutTarget {
    /// Check out this validated ref or revision.
    Ref(RevSpec),
    /// Check out the previous branch (`git checkout -`).
    Previous,
}

impl CheckoutTarget {
    /// Check out a validated ref/revision.
    pub fn rev(rev: RevSpec) -> Self {
        Self::Ref(rev)
    }

    /// Check out the previous branch (`git checkout -`).
    pub fn previous() -> Self {
        Self::Previous
    }

    /// The single argv token this target expands to.
    pub(super) fn as_arg(&self) -> &str {
        match self {
            Self::Ref(rev) => rev.as_str(),
            Self::Previous => "-",
        }
    }
}

impl From<RevSpec> for CheckoutTarget {
    fn from(rev: RevSpec) -> Self {
        Self::Ref(rev)
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

/// The oldest git this crate is written against — **2.31**, the highest version its
/// own argv actually requires (validated on 2.54). `harden()` pins config through
/// `GIT_CONFIG_COUNT`/`_KEY_n`/`_VALUE_n` (added in **2.31**); `branch_status`/`snapshot`
/// read `status --porcelain=v2` (2.11) and `switch_with_stash` uses `stash push` (2.13),
/// all below 2.31. Gating on the real minor floor makes [`ensure_supported`] catch a
/// too-old git with a clear message rather than letting it pass and then fail later with
/// a cryptic argv error — the M29 fix (the previous gate was major-only, so 2.7 "passed"
/// then broke). (Contrast vcs-jj, whose floor is precise per its empirically-validated
/// parser release.)
const MIN_SUPPORTED_MAJOR: u64 = 2;
const MIN_SUPPORTED_MINOR: u64 = 31;

impl GitCapabilities {
    /// Whether the binary meets the supported floor (git ≥ 2.31).
    pub fn is_supported(&self) -> bool {
        (self.version.major, self.version.minor) >= (MIN_SUPPORTED_MAJOR, MIN_SUPPORTED_MINOR)
    }

    /// Error unless [`is_supported`](Self::is_supported) — a clear "needs git
    /// ≥ 2.31, found 2.7.4" instead of a cryptic argv failure later.
    pub fn ensure_supported(&self) -> Result<()> {
        if self.is_supported() {
            return Ok(());
        }
        Err(Error::spawn(
            BINARY,
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!(
                    "vcs-git requires git >= {MIN_SUPPORTED_MAJOR}.{MIN_SUPPORTED_MINOR} \
                     (validated on 2.54), found {}",
                    self.version
                ),
            ),
        ))
    }
}
