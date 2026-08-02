//! Command specifications, validated input types, and capability metadata.

use super::*;

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
    pub(super) fn as_arg(self) -> &'static str {
        match self {
            SparseMode::Copy => "copy",
            SparseMode::Full => "full",
            SparseMode::Empty => "empty",
        }
    }
}

/// An exact-path jj fileset (`root-file:"<path>"`), so path metacharacters like `(`,
/// `)`, `|`, `*` are treated literally rather than as fileset operators.
///
/// Build it with [`JjFileset::path`]; the path is **workspace-root-relative** and
/// resolved as such regardless of the command's working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JjFileset(String);

impl JjFileset {
    /// Wrap a workspace-root-relative `path` as an exact-path fileset. Uses jj's
    /// **`root-file:`** anchor (not the cwd-relative `file:`), so the path is
    /// interpreted relative to the workspace root even when the command runs from a
    /// subdirectory (`dir` ≠ root) — a plain `file:` there would silently target a
    /// same-named file under `dir`, or nothing (M2). **On Windows** the caller's `\`
    /// path separators are normalised to jj's forward slash (so `src\a.rs` matches);
    /// **on Unix** `\` is a legitimate filename byte and is left intact — rewriting it
    /// there would corrupt a real path (matching `vcs-git`'s twin, which also gates
    /// the rewrite on Windows). Then `\` and `"` are escaped for the string literal.
    pub fn path(path: impl AsRef<str>) -> Self {
        let path = path.as_ref();
        #[cfg(windows)]
        let normalised = path.replace('\\', "/");
        #[cfg(not(windows))]
        let normalised = path.to_string();
        let escaped = normalised.replace('\\', "\\\\").replace('"', "\\\"");
        JjFileset(format!("root-file:\"{escaped}\""))
    }

    /// The rendered `root-file:"…"` expression.
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
    pub base: RevsetExpr,
    /// Filesystem path for the new workspace.
    pub path: PathBuf,
    /// How to seed the new workspace's sparse patterns (`--sparse-patterns`);
    /// `None` leaves jj's default (inherit from the current workspace).
    pub sparse_patterns: Option<SparseMode>,
}

impl WorkspaceAdd {
    /// A workspace named `name`, based at `base`, materialised at `path`.
    pub fn new(name: impl Into<String>, base: RevsetExpr, path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            base,
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

/// Options for [`JjApi::squash_paths`] (`jj squash --from <from> --into <into>
/// [--use-destination-message] <filesets>`).
///
/// `#[non_exhaustive]`, so build it through [`SquashPaths::new`] and the chained
/// setters rather than a struct literal.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SquashPaths {
    /// Source revision the filesets are squashed out of (`--from`).
    pub from: RevsetExpr,
    /// Destination revision the filesets are squashed into (`--into`).
    pub into: RevsetExpr,
    /// The exact filesets to move; empty squashes the whole `from` change.
    pub filesets: Vec<JjFileset>,
    /// Keep the destination's description rather than combining the two
    /// (`--use-destination-message`).
    pub use_destination_message: bool,
}

impl SquashPaths {
    /// Squash from `from` into `into`, with no filesets selected yet.
    pub fn new(from: RevsetExpr, into: RevsetExpr) -> Self {
        Self {
            from,
            into,
            filesets: Vec::new(),
            use_destination_message: false,
        }
    }

    /// Set the filesets to move (replacing any already added).
    pub fn filesets(mut self, filesets: impl IntoIterator<Item = JjFileset>) -> Self {
        self.filesets = filesets.into_iter().collect();
        self
    }

    /// Keep the destination's description (`--use-destination-message`) instead
    /// of combining the two.
    pub fn use_destination_message(mut self) -> Self {
        self.use_destination_message = true;
        self
    }
}

/// Options for [`JjApi::bookmark_move`] (`jj bookmark move <name> --to <rev>`).
///
/// `#[non_exhaustive]`, so build it through [`BookmarkMove::new`] and the chained
/// [`allow_backwards`](BookmarkMove::allow_backwards) setter rather than a bare
/// `bool` (`bookmark_move(name, to, true)` doesn't say what `true` permits).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BookmarkMove {
    /// The bookmark to move.
    pub name: BookmarkName,
    /// The revision to move it to (`--to`).
    pub to: RevsetExpr,
    /// Allow moving the bookmark to a commit that is not a descendant of its
    /// current target (`--allow-backwards`).
    pub allow_backwards: bool,
}

impl BookmarkMove {
    /// Move bookmark `name` to revision `to`; a backwards move is refused.
    pub fn new(name: BookmarkName, to: RevsetExpr) -> Self {
        Self {
            name,
            to,
            allow_backwards: false,
        }
    }

    /// Allow moving to a commit that is not a descendant of the current target
    /// (`--allow-backwards`).
    pub fn allow_backwards(mut self) -> Self {
        self.allow_backwards = true;
        self
    }
}

/// Options for [`JjApi::squash_into`] (`jj squash --into <rev>`).
///
/// `#[non_exhaustive]`, so build it through [`SquashInto::new`] and the chained
/// [`use_destination_message`](SquashInto::use_destination_message) setter rather
/// than a bare `bool`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SquashInto {
    /// The destination revision the working copy is squashed into (`--into`).
    pub into: RevsetExpr,
    /// Keep the destination's description rather than combining the two
    /// (`--use-destination-message`).
    pub use_destination_message: bool,
}

impl SquashInto {
    /// Squash the working copy into `into`, combining the two descriptions.
    pub fn new(into: RevsetExpr) -> Self {
        Self {
            into,
            use_destination_message: false,
        }
    }

    /// Keep the destination's description (`--use-destination-message`) instead
    /// of combining the two.
    pub fn use_destination_message(mut self) -> Self {
        self.use_destination_message = true;
        self
    }
}

/// Colocation choice for [`JjApi::git_clone`] (`jj git clone
/// --colocate|--no-colocate`).
///
/// The flag is **always** passed explicitly — jj's default flipped across versions
/// and is overridable via `git.colocate` config — so there is deliberately no
/// default: pick [`GitClone::colocated`] or [`GitClone::separate`].
/// `#[non_exhaustive]`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct GitClone {
    /// Create a visible `.git` alongside `.jj` (`--colocate`) rather than a
    /// jj-only checkout (`--no-colocate`).
    pub colocate: bool,
}

impl GitClone {
    /// A colocated clone — a visible `.git` beside `.jj` (`--colocate`).
    pub fn colocated() -> Self {
        Self { colocate: true }
    }

    /// A non-colocated clone — jj-only, no `.git` (`--no-colocate`).
    pub fn separate() -> Self {
        Self { colocate: false }
    }
}

/// The first bookmark name from a [`BOOKMARKS_TEMPLATE`](parse::BOOKMARKS_TEMPLATE)
/// render (space-joined `.escape_json()` names), decoded; `None` when the commit
/// carries no local bookmark. Delegates to [`parse::first_bookmark_name`] so the
/// escaping contract lives in one place.
pub(super) fn first_bookmark(rendered: &str) -> Option<String> {
    parse::first_bookmark_name(rendered)
}

/// Injection guard for bare positional argv slots: a caller-supplied value
/// with a leading `-` is parsed by jj's CLI as a *flag* (verified: `jj edit
/// -evil` → "unexpected argument"), and an empty value changes a command's
/// meaning. Refuse both before anything spawns. Flag-VALUE positions
/// (`-r <revset>`, `-m <msg>`) need no guard — jj itself rejects dash-values
/// there with a clear error rather than misparsing them.
pub(super) fn reject_flag_like(what: &str, value: &str) -> Result<()> {
    vcs_cli_support::reject_flag_like(BINARY, what, value)
}

/// [`reject_flag_like`] for a bare positional **path** argv slot
/// (`workspace add`'s destination path), whose value is typed `PathBuf`
/// rather than `&str` and so may be non-UTF-8 on Unix. Checks a lossy-UTF-8
/// rendering of `path` — `to_string_lossy` never panics, so this never
/// aborts on invalid UTF-8 — for a leading `-` (after trim), emptiness, or an
/// embedded NUL; a leading `-`/NUL/emptiness is always ASCII and so survives
/// the lossy conversion unchanged regardless of any invalid bytes elsewhere
/// in `path`. Only the *check* is lossy: the value actually handed to
/// `Command::arg` stays the original `path`, byte-for-byte, so a legitimate
/// non-UTF-8 path with no leading `-` still reaches the child process
/// unaltered.
pub(super) fn reject_flag_like_path(what: &str, path: &Path) -> Result<()> {
    reject_flag_like(what, &path.to_string_lossy())
}

/// Emptiness guard for a caller-supplied file path that is **wrapped into a
/// larger expression** ([`file_show`](JjApi::file_show)'s
/// [`JjFileset::path`] → `root-file:"<path>"`) instead of occupying a bare argv
/// slot of its own.
///
/// [`reject_flag_like`] is the wrong check for such a slot in both directions: a
/// leading `-` is inert inside the quoted fileset literal (rejecting it would
/// refuse a perfectly legitimate `-dash.txt`), while emptiness — which
/// `reject_flag_like` happens to cover for *bare* positionals — is exactly what
/// silently changes the command's meaning here. `root-file:""` anchors on the
/// **workspace root**: a path that genuinely exists, so jj raises no "No such
/// path" error, yet `file:` is an *exact-file* pattern, so it matches no file at
/// all and `jj file show` exits **0 with empty output** (verified on jj 0.38.0)
/// — the read reports a file that "exists and is empty".
///
/// `vcs_git`'s `show_file` carries the mirror guard with a byte-identical
/// message (only the program name differs), so a cross-backend caller sees ONE
/// error form. Its empty-path degradation differs in shape but not in kind:
/// `git show <rev>:` exits 0 printing the root **tree listing** (verified on git
/// 2.55.0).
///
/// Whitespace-only is refused with the empty string. A name made only of spaces
/// is legal on Unix, but at this boundary it is indistinguishable from the far
/// likelier caller bug (a blank/unset path variable), and refusing it keeps one
/// rule — and one error — across both backends.
///
/// An interior NUL needs no check here: it can only reach `Command::arg`, which
/// already fails the spawn with the same `io::ErrorKind::InvalidInput` this guard
/// raises, so the classification a caller sees is unchanged.
pub(super) fn reject_empty_path(what: &str, path: &str) -> Result<()> {
    if path.trim().is_empty() {
        return Err(Error::spawn(
            BINARY,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{what} {path:?} is empty or whitespace-only — an empty path silently \
                     re-targets the read at the repository root instead of a single file; \
                     refusing before spawning"
                ),
            ),
        ));
    }
    Ok(())
}

/// The working-copy revset `@` as a validated [`RevsetExpr`]. Infallible — `@`
/// is always a valid revset — for the internal helpers that query `@` directly.
pub(super) fn at_revset() -> RevsetExpr {
    RevsetExpr::new("@").expect("`@` is a valid revset")
}

/// Wrap a caller-supplied bookmark/branch/remote name as jj's `exact:` string
/// pattern. jj treats a bare `<NAMES>` / `-b <BOOKMARK>` / `--remote <REMOTE>`
/// argument as a **glob** pattern (verified on 0.42: `bookmark delete '*'`
/// deletes every bookmark; `git push -b '*'` pushes them all), so a name that
/// happens to contain `*`/`?` — or a hostile `"*"` from a UI/bot — would fan the
/// operation out across every matching ref. `exact:` forces a literal match of
/// exactly this name (verified: `exact:foo1` deletes only `foo1`, and a literal
/// `*` in a name is matched verbatim under `exact:`), so these typed methods
/// mutate exactly the one ref the caller named.
pub(super) fn exact(name: &str) -> String {
    format!("exact:{name}")
}

/// Validation guard for the remote segment of jj's positional
/// `<name>@<remote>` bookmark-tracking pattern. jj 0.42 parses an empty remote
/// in `exact:main@` as the empty remote name, warns `No matching remote
/// bookmarks`, and exits successfully without changing anything; whitespace
/// behaves equivalently for that whitespace-only remote name. The legacy parser
/// also splits on the **last** `@`, so `exact:main@origin@backup` is interpreted
/// as bookmark `exact:main@origin` on remote `backup`, not remote
/// `origin@backup`. Refuse both ambiguous forms before spawning.
///
/// Unlike a bare `<NAMES>`/`--remote` slot, the remote segment of this composite
/// form is **not** itself parsed as a string-pattern: an `exact:`/`glob:` prefix
/// on it is taken as part of the *literal* remote name instead of being
/// interpreted (verified on jj 0.42: `bookmark track exact:main@exact:origin`
/// warns "No matching remote bookmarks for names: main@\"exact:origin\"" and
/// tracks nothing — a silent no-op, not an error — and `main@glob:origin` is
/// rejected outright with "remote bookmark must be specified in bookmark@remote
/// form"). The segment is, however, still glob-matched positionally
/// (`main@ori?in` tracks `origin`), so a hostile/glob-bearing remote name must
/// be rejected before spawn rather than wrapped in `exact:`.
pub(super) fn reject_bookmark_track_remote(what: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(Error::spawn(
            BINARY,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{what} must not be empty or whitespace-only — jj would silently match no \
                     remote bookmarks"
                ),
            ),
        ));
    }
    if value.contains('@') {
        return Err(Error::spawn(
            BINARY,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{what} {value:?} contains `@`, which jj's legacy bookmark@remote parser \
                     would split ambiguously"
                ),
            ),
        ));
    }
    if value.contains(['*', '?', '[', ']']) {
        return Err(Error::spawn(
            BINARY,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{what} {value:?} contains a glob metacharacter and could fan out across \
                     remotes — refusing to pass it as a positional argument"
                ),
            ),
        ));
    }
    Ok(())
}

/// Pin `LC_ALL=C` on a command whose failure output is classified by matching
/// **untranslated English substrings** — the transient-fetch markers
/// (`is_transient_fetch_error`). jj's `git fetch` surfaces libc/gai/curl network
/// errors ("Temporary failure in name resolution"), which a localized environment
/// would translate — silently turning a retryable transient failure into an
/// unclassified one that is *not* retried. Mirrors `vcs-git`'s `c_locale`.
pub(super) fn c_locale(cmd: processkit::Command) -> processkit::Command {
    cmd.env("LC_ALL", "C")
}

/// A validated revset expression. Every [`JjApi`] operation that resolves a
/// revision/revset takes a `RevsetExpr` (directly or inside its options struct),
/// so a revset from untrusted input (UIs, bots, agents) is validated once, at
/// construction, and the type is the flag-injection barrier from then on.
/// Deliberately *minimal* — jj's revset grammar is too rich to validate here —
/// it only guarantees the expression is non-empty and cannot be parsed as a flag
/// (no leading `-`). A rejected expression is an [`vcs_cli_support::is_invalid_input`]
/// failure. For a value that must be a bookmark **name** (create/move/delete a
/// bookmark) use [`BookmarkName`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RevsetExpr(String);

impl RevsetExpr {
    /// Validate `revset` (non-empty, no leading `-`).
    pub fn new(revset: impl Into<String>) -> Result<Self> {
        let revset = revset.into();
        reject_flag_like("revset", &revset)?;
        Ok(RevsetExpr(revset))
    }

    /// The validated expression.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RevsetExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for RevsetExpr {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Self::new(s)
    }
}

/// A validated jj bookmark name (jj's equivalent of a git branch). Every
/// [`JjApi`] operation that names a bookmark to create, move, rename, delete,
/// track, fetch, or push takes a `BookmarkName`, so a name from untrusted input
/// is validated once, at construction. jj bookmark names are permissive, so the
/// guarantee is the load-bearing one: non-empty and not flag-shaped (no leading
/// `-`), matching the injection guard these operations applied internally before.
/// The typed methods additionally wrap the name in jj's `exact:` string pattern
/// so a `*`/`?` in a name can never fan the operation out across every bookmark.
/// A rejected name is an [`vcs_cli_support::is_invalid_input`] failure.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BookmarkName(String);

impl BookmarkName {
    /// Validate `name` as a bookmark name (non-empty, no leading `-`).
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        reject_flag_like("bookmark name", &name)?;
        Ok(BookmarkName(name))
    }

    /// The validated name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BookmarkName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for BookmarkName {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Self::new(s)
    }
}

/// What the installed `jj` binary supports, probed via
/// [`JjApi::capabilities`]. A value type — the client holds no state, so probe
/// once and keep the result (callers cache it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct JjCapabilities {
    /// The binary's parsed version.
    pub version: JjVersion,
}

/// The validated jj floor: every parser and flag in this crate was verified
/// empirically against this release. jj's CLI moves fast, so the floor is a full
/// version pinned to a validated release; vcs-git instead gates on the highest
/// version its own argv requires (`2.31`).
const MIN_SUPPORTED: JjVersion = JjVersion {
    major: 0,
    minor: 38,
    patch: 0,
};

impl JjCapabilities {
    /// Whether the binary meets the validated floor (jj ≥ 0.38).
    pub fn is_supported(&self) -> bool {
        self.version >= MIN_SUPPORTED
    }

    /// Error unless [`is_supported`](Self::is_supported) — a clear "needs jj
    /// ≥ 0.38, found 0.35.0" instead of a cryptic argv/template failure later.
    pub fn ensure_supported(&self) -> Result<()> {
        if self.is_supported() {
            return Ok(());
        }
        Err(Error::spawn(
            BINARY,
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!(
                    "vcs-jj requires jj >= {MIN_SUPPORTED} (the validated floor), found {}",
                    self.version
                ),
            ),
        ))
    }
}
