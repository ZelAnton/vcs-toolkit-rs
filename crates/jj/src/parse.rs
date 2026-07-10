//! Pure parsers for jj output. No process execution, so these tests are
//! hermetic and run on CI.
//!
//! The git-format unified-diff model + parser and the version type live in the
//! shared [`vcs_diff`] crate (`jj diff --git` and `git diff` are byte-identical for
//! ASCII paths — they differ only in non-ASCII filename rendering, which the shared
//! parser decodes); this module keeps only the jj-specific parsers (changes,
//! bookmarks, op log, …).

use std::path::PathBuf;

use vcs_diff::{DiffStat, path_from_bytes};

/// A jj change, parsed from a `\t`-delimited template row.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Change {
    /// Short change id (`change_id.short()`).
    pub change_id: String,
    /// Short commit id (`commit_id.short()`).
    pub commit_id: String,
    /// `true` when the change makes no file modifications.
    pub empty: bool,
    /// First line of the description (empty for an undescribed change).
    pub description: String,
}

/// A jj bookmark, parsed from `jj bookmark list` output.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Bookmark {
    /// Bookmark name.
    pub name: String,
    /// **Full** commit id the bookmark points at — a stable identifier that can
    /// be cross-referenced against a `RepoSnapshot.head` / git oid, not a
    /// display-truncated prefix (T-041). Empty when the bookmark has no single
    /// normal target (a conflicted bookmark, which is still *present*).
    pub target: String,
}

/// A bookmark from `jj bookmark list -a` — local *or* remote-tracking.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BookmarkRef {
    /// Bookmark name.
    pub name: String,
    /// The remote it lives on (e.g. `origin`/`git`); `None` for a local bookmark.
    pub remote: Option<String>,
    /// **Full** commit id it points at (empty for a conflicted bookmark) — a
    /// stable cross-referenceable id, not a display prefix (T-041).
    pub target: String,
    /// Whether this remote-tracking bookmark is tracked (`false` for locals).
    pub tracked: bool,
}

/// A workspace from `jj workspace list` (rendered with `WORKSPACE_TEMPLATE`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Workspace {
    /// Workspace name (`default` for the main one).
    pub name: String,
    /// **Full** commit id of the workspace's working-copy commit — the identity
    /// the facade's `WorktreeInfo.commit` carries so it can be compared against a
    /// `RepoSnapshot.head`; not a display-truncated prefix (T-041).
    pub commit: String,
    /// Local bookmarks pointing at that commit (empty when none).
    pub bookmarks: Vec<String>,
}

/// One entry from `jj diff --summary`: a single-letter status (`M`/`A`/`D`/…)
/// and the (forward-slash-normalised) path it applies to — the *new* path for a
/// rename/copy, with the original on [`old_path`](ChangedPath::old_path).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChangedPath {
    /// Status letter (`M` modified, `A` added, `D` deleted, `R` renamed,
    /// `C` copied).
    pub status: char,
    /// The path the status applies to — the *new* path for a rename/copy. A
    /// [`PathBuf`] built from the raw `jj diff --summary` bytes, so a non-UTF-8
    /// filename (legal on Unix) survives losslessly instead of being flattened to
    /// `U+FFFD` — the same platform-correct type `vcs_git::StatusEntry::path` uses.
    pub path: PathBuf,
    /// For a rename (`R`) or copy (`C`), the original path; `None` otherwise.
    pub old_path: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Machine-template framing/escaping contract (T-041)
// ---------------------------------------------------------------------------
//
// jj templates render into a byte stream we parse back into typed rows, so the
// framing has to be *unambiguous* even for exotic names/descriptions (spaces,
// commas, tabs, quotes, newlines — all of which jj permits somewhere: a git
// bookmark name can carry a comma, a workspace name a tab/newline, a description
// a tab). The single contract every machine template below obeys:
//
//   * **Rows** are separated by a literal `\n`; **fields** within a row by a
//     literal `\t`.
//   * A field that can hold arbitrary user text (a description, a bookmark or
//     workspace *name*, an op-log user) is rendered through jj's `.escape_json()`
//     — a standard JSON string literal (`"…"` with `\t`/`\n`/`\r`/`\"`/`\\`/`\uXXXX`
//     escapes; raw UTF-8 otherwise, verified on jj 0.42). An escaped field can
//     therefore never contain a literal `\t` or `\n`, so the tab/newline framing
//     stays unambiguous, and [`decode_json_field`] recovers the exact original.
//   * A **list** field (a commit's/workspace's local bookmark names) is the
//     `.escape_json()` of each element joined by a single space. Bookmark names
//     can never contain a space (a git-ref rule jj enforces), so the space-joined
//     JSON strings split back apart cleanly ([`decode_name_list`]).
//   * Structurally-constrained fields — hex ids, `0`/`1` and `true`/`false`
//     flags, a `%:z` RFC-3339 timestamp, a remote name (no whitespace by git-ref
//     rule) — are rendered raw; they cannot contain a separator.
//
// The lone documented exception is [`ANNOTATE_TEMPLATE`]: it streams raw file
// *content* (one source line per row) as the sole trailing field, which cannot
// contain a `\n` (rows are line-split) and whose interior tabs are preserved by a
// single `split_once('\t')` — escaping every source line would be wasteful and
// buys nothing there.
//
// **Full vs short ids.** Identity/cross-reference fields carry the *full* commit
// id ([`Bookmark::target`], [`BookmarkRef::target`], [`Workspace::commit`], and
// the snapshot head) so they can be matched against a git oid / `RepoSnapshot.head`
// without a short-prefix collision. The one deliberately *short* surface is the
// history-display [`Change`] (`jj log`'s own abbreviation), which is never used as
// a cross-reference key.

/// Template used by the change commands: tab-separated, one change per line. The
/// change/commit ids stay `.short()` — [`Change`] is the history-display row, not
/// an identity key — while the free-text description is `.escape_json()`-framed so
/// a tab/quote in it round-trips (see the framing contract above).
pub(crate) const CHANGE_TEMPLATE: &str = "change_id.short() ++ \"\\t\" ++ commit_id.short() ++ \"\\t\" ++ if(empty, \"true\", \"false\") ++ \"\\t\" ++ description.first_line().escape_json() ++ \"\\n\"";

/// `jj workspace list -T` template: `"<name>"\t<full-commit>\t<bookmarks>`, where
/// the name is `.escape_json()`-framed (a workspace name may hold a tab/newline),
/// the commit is the **full** id (identity, see the contract), and the bookmarks
/// are the space-joined `.escape_json()` of each local bookmark name.
pub(crate) const WORKSPACE_TEMPLATE: &str = "name.escape_json() ++ \"\\t\" ++ target.commit_id() ++ \"\\t\" ++ target.local_bookmarks().map(|b| b.name().escape_json()).join(\" \") ++ \"\\n\"";

/// `jj log -T` template rendering a commit's local bookmark names as space-joined
/// `.escape_json()` strings (so a name with a comma survives — the old comma-join
/// mangled it). Drives `current_bookmark`/`trunk` via [`first_bookmark_name`].
pub(crate) const BOOKMARKS_TEMPLATE: &str =
    "local_bookmarks.map(|b| b.name().escape_json()).join(\" \")";

/// `jj bookmark list -a -T` template:
/// `<present 1/0>\t"<name>"\t<remote>\t<tracked 1/0>\t<full-commit>`, one row per
/// local *and* remote-tracking bookmark. `present` gates out a locally-deleted
/// **tombstone** (a `bookmark delete` still shown because a remote tracks it), and
/// the name is `.escape_json()`-framed. `remote` is raw (a remote name carries no
/// whitespace).
pub(crate) const BOOKMARK_ALL_TEMPLATE: &str = "if(present, \"1\", \"0\") ++ \"\\t\" ++ name.escape_json() ++ \"\\t\" ++ remote ++ \"\\t\" ++ if(tracked, \"1\", \"0\") ++ \"\\t\" ++ if(normal_target, normal_target.commit_id(), \"\") ++ \"\\n\"";

/// `jj bookmark list -T` template (no `-a`):
/// `<present 1/0>\t<remote>\t"<name>"\t<full-commit>`, one row per bookmark ref.
/// Machine-parsed in place of jj's human-readable default, which interleaves the
/// change id, description, and indented remote-tracking lines that drift with jj's
/// display format. `present` + an empty `remote` let [`parse_bookmarks`] keep only
/// *live local* bookmarks: a locally-deleted **tombstone** renders a `present=0`
/// local row (dropped) plus a `present=1` `remote=<r>` row (dropped as non-local),
/// so it never masquerades as an existing branch — while a *conflicted* bookmark,
/// which is `present=1` with an empty target, is correctly kept (T-041).
pub(crate) const BOOKMARK_LIST_TEMPLATE: &str = "if(present, \"1\", \"0\") ++ \"\\t\" ++ remote ++ \"\\t\" ++ name.escape_json() ++ \"\\t\" ++ if(normal_target, normal_target.commit_id(), \"\") ++ \"\\n\"";

/// `jj log -T` template: `"1"` when the commit has a conflict, else `"0"`.
pub(crate) const CONFLICT_TEMPLATE: &str = "if(conflict, \"1\", \"0\")";

/// `jj log -T` template emitting one short commit id per line — for counting a
/// revset.
pub(crate) const COUNT_TEMPLATE: &str = "commit_id.short() ++ \"\\n\"";

/// `jj log -T` template for [`reachable_bookmarks`](crate::JjApi::reachable_bookmarks):
/// the commit's local bookmark names as space-joined `.escape_json()` strings
/// (so a comma/quote in a name round-trips), then a tab, then the **full** commit
/// id (identity — see the framing contract).
pub(crate) const REACHABLE_BOOKMARKS_TEMPLATE: &str = "local_bookmarks.map(|b| b.name().escape_json()).join(\" \") ++ \"\\t\" ++ commit_id ++ \"\\n\"";

/// Parse `jj --version` output (`jj 0.38.0`) into the shared
/// [`vcs_diff::Version`]: the first dotted-numeric token wins; non-numeric
/// trailers (`-dev`, build hashes) are ignored; a missing patch reads as `0`.
pub(crate) fn parse_jj_version(raw: &str) -> Option<vcs_diff::Version> {
    vcs_diff::parse_dotted_version(raw)
}

/// `jj evolog -T` template. Evolog renders in a *commit* context where the
/// bare keywords (`change_id`, …) don't exist — the `commit.` method form is
/// required. Columns mirror [`CHANGE_TEMPLATE`] (`.escape_json()`-framed
/// description included), so [`parse_changes`] reads it.
pub(crate) const EVOLOG_TEMPLATE: &str = "commit.change_id().short() ++ \"\\t\" ++ commit.commit_id().short() ++ \"\\t\" ++ if(commit.empty(), \"true\", \"false\") ++ \"\\t\" ++ commit.description().first_line().escape_json() ++ \"\\n\"";

/// `jj op log -T` template: `id\t"<user>"\t<start-time>\t"<description>"`, one row
/// per operation. The user and description are `.escape_json()`-framed (either can
/// hold a tab); the id is short (what `op restore`/`op undo` accept) and the
/// timestamp is a separator-free `%:z` RFC-3339.
pub(crate) const OP_TEMPLATE: &str = "id.short() ++ \"\\t\" ++ user.escape_json() ++ \"\\t\" ++ time.start().format(\"%Y-%m-%dT%H:%M:%S%:z\") ++ \"\\t\" ++ description.first_line().escape_json() ++ \"\\n\"";

/// `jj op log -T` template for the rollback **divergence probe**: `id\tparent-count`,
/// one row per operation, newest first. A parent count `>= 2` marks a "reconcile
/// divergent operations" merge — the fingerprint jj records when a *concurrent* jj
/// process advanced the operation log, so a rollback walking this can refuse to
/// revert that foreign work (see `Jj::rollback_to`). Kept minimal (no user/time)
/// because the probe only needs the ancestry shape.
pub(crate) const OP_PARENTS_TEMPLATE: &str = "id.short() ++ \"\\t\" ++ parents.len() ++ \"\\n\"";

/// `jj file annotate -T` template: `change-id\tcontent`. Annotate emits one row
/// per source line and separates them itself — no trailing `\n` here, or every
/// row would be double-spaced. `content` is the framing contract's one documented
/// raw (un-escaped) field: it is the sole trailing column, a source line can't
/// hold a `\n`, and an interior tab is preserved by [`parse_annotate`]'s single
/// `split_once('\t')`.
pub(crate) const ANNOTATE_TEMPLATE: &str = "commit.change_id().short() ++ \"\\t\" ++ content";

/// One entry of `jj op log` (an operation-log row).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Operation {
    /// Short operation id — what `op restore`/`op undo` take.
    pub id: String,
    /// The OS-level `user@host` that ran the operation (not the configured
    /// jj author).
    pub user: String,
    /// Start timestamp, RFC 3339 (`%Y-%m-%dT%H:%M:%S` with a **colon** offset, e.g.
    /// `2026-06-05T10:00:00+02:00`) — parseable by a strict RFC-3339 reader, matching
    /// `vcs-git`'s `%aI` dates (jj's `%z` would emit `+0200`, which strict parsers
    /// reject).
    pub time: String,
    /// First line of the operation description, e.g. `new empty commit`.
    pub description: String,
}

/// One line of `jj file annotate` output: which change last touched it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AnnotationLine {
    /// Short change id of the change that introduced the line.
    pub change_id: String,
    /// Line number in the annotated file (1-based).
    pub line: u32,
    /// The line's content (the raw bytes jj reports for the line, with only
    /// the `\n` row separator removed; a trailing `\r` from a CRLF-terminated
    /// source file is preserved, not stripped).
    pub content: String,
}

/// Decode a single JSON string literal as emitted by a jj template's
/// `.escape_json()` — e.g. `"a\tb"` → `a⇥b`, `"co,mma"` → `co,mma`. This is the
/// inverse of the framing contract's per-field escaping.
///
/// Lenient by design (these parsers must never panic on unexpected jj output): a
/// field that is *not* a `"…"` literal is returned verbatim (so a hex id, flag, or
/// legacy raw field passes through unchanged), and a truncated or malformed escape
/// simply stops decoding rather than erroring. Only the escapes jj's `escape_json`
/// actually emits are recognised (`\" \\ \/ \b \f \n \r \t \uXXXX`); any other
/// backslash pair is passed through as its second char.
fn decode_json_field(field: &str) -> String {
    let mut chars = field.chars();
    // A JSON string starts with a quote; anything else is returned as-is.
    if chars.next() != Some('"') {
        return field.to_string();
    }
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => break, // closing quote — ignore any trailing bytes
            '\\' => match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                Some('b') => out.push('\u{0008}'),
                Some('f') => out.push('\u{000C}'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('u') => {
                    // `\uXXXX` — up to four hex digits (jj only escapes control
                    // chars this way, so the BMP scalar always builds a `char`).
                    let mut code: u32 = 0;
                    for _ in 0..4 {
                        match chars.next().and_then(|h| h.to_digit(16)) {
                            Some(d) => code = code * 16 + d,
                            None => break,
                        }
                    }
                    if let Some(ch) = char::from_u32(code) {
                        out.push(ch);
                    }
                }
                Some(other) => out.push(other),
                None => break,
            },
            other => out.push(other),
        }
    }
    out
}

/// Decode a space-joined list of `.escape_json()` names (the framing contract's
/// list field) back into the individual names. Splitting on the space is exact
/// because a bookmark name can never contain one (a git-ref rule jj enforces), so
/// each token is one whole JSON string literal.
fn decode_name_list(field: &str) -> Vec<String> {
    field
        .split(' ')
        .filter(|tok| !tok.is_empty())
        .map(decode_json_field)
        .collect()
}

/// The first name of a [`BOOKMARKS_TEMPLATE`] render (space-joined `.escape_json()`
/// names), decoded; `None` when the commit carries no local bookmark. Drives
/// `current_bookmark`/`trunk`.
pub(crate) fn first_bookmark_name(rendered: &str) -> Option<String> {
    decode_name_list(rendered.trim()).into_iter().next()
}

/// Parse rows produced by [`OP_TEMPLATE`].
pub(crate) fn parse_operations(output: &str) -> Vec<Operation> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            // The user and description are `.escape_json()`-framed (no literal tab
            // inside), so the four columns split cleanly; `splitn(4)` is belt-and-
            // braces should a future column ever carry one.
            let mut fields = line.splitn(4, '\t');
            let id = fields.next()?.to_string();
            let user = decode_json_field(fields.next()?);
            let time = fields.next()?.to_string();
            let description = decode_json_field(fields.next().unwrap_or(""));
            Some(Operation {
                id,
                user,
                time,
                description,
            })
        })
        .collect()
}

/// Parse rows produced by [`OP_PARENTS_TEMPLATE`] into `(op-id, parent-count)`
/// pairs, newest first — the input to the rollback divergence walk. A row whose
/// parent-count is missing or unparsable is read as `0` parents (it cannot be the
/// divergence merge the probe looks for, so a malformed row never spuriously trips
/// the "foreign concurrency" signal); the id is always kept so the walk can still
/// locate the captured pre-operation.
pub(crate) fn parse_op_parents(output: &str) -> Vec<(String, usize)> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.splitn(2, '\t');
            let id = fields.next().unwrap_or("").to_string();
            let parents = fields
                .next()
                .and_then(|s| s.trim().parse::<usize>().ok())
                .unwrap_or(0);
            (id, parents)
        })
        .collect()
}

/// Parse rows produced by [`ANNOTATE_TEMPLATE`]: one row per source line, the
/// 1-based line number is the row index.
///
/// Splits on `\n` (not [`str::lines`]) so a trailing `\r` belonging to a
/// CRLF-terminated source line stays in the content instead of being stripped.
/// The empty final segment left by a trailing newline carries no tab, so the
/// `split_once('\t')?` filter drops it and the line numbering stays exact.
pub(crate) fn parse_annotate(output: &str) -> Vec<AnnotationLine> {
    output
        .split('\n')
        .enumerate()
        .filter_map(|(idx, line)| {
            let (change_id, content) = line.split_once('\t')?;
            Some(AnnotationLine {
                change_id: change_id.to_string(),
                // Saturating: a >4 billion-line file would silently wrap a raw
                // `as u32`. Such input is not realistic, but truncation never is.
                line: u32::try_from(idx + 1).unwrap_or(u32::MAX),
                content: content.to_string(),
            })
        })
        .collect()
}

/// Parse rows produced by [`CHANGE_TEMPLATE`].
pub(crate) fn parse_changes(output: &str) -> Vec<Change> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            // The description is `.escape_json()`-framed, so it holds no literal
            // tab; `splitn(4)` still isolates it as the trailing column, then
            // `decode_json_field` restores any tab/quote/backslash it carried.
            let mut fields = line.splitn(4, '\t');
            let change_id = fields.next()?.to_string();
            let commit_id = fields.next()?.to_string();
            let empty = fields.next()? == "true";
            let description = decode_json_field(fields.next().unwrap_or(""));
            Some(Change {
                change_id,
                commit_id,
                empty,
                description,
            })
        })
        .collect()
}

/// Parse rows produced by [`BOOKMARK_LIST_TEMPLATE`]:
/// `<present 1/0>\t<remote>\t"<name>"\t<full-commit>`. Yields only **live local**
/// bookmarks — a locally-deleted *tombstone* (`present=0`, or the `present=1`
/// remote-tracking row that surfaces beside it) is filtered out, so a deleted
/// bookmark no longer masquerades as an existing branch in `local_branches` /
/// `branch_exists` (T-041). A *conflicted* bookmark (`present=1`, empty target) is
/// kept — it is present, just without a single normal target. A row with an empty
/// name contributes nothing.
pub(crate) fn parse_bookmarks(output: &str) -> Vec<Bookmark> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let present = fields.next()? == "1";
            let remote = fields.next().unwrap_or("");
            let name = decode_json_field(fields.next().unwrap_or(""));
            let target = fields.next().unwrap_or("").to_string();
            // A tombstone (`present=0`) or a remote-tracking row (non-empty
            // `remote`) is not an existing local branch; drop both. An empty name
            // never yields a bookmark.
            if !present || !remote.is_empty() || name.is_empty() {
                return None;
            }
            Some(Bookmark { name, target })
        })
        .collect()
}

/// Parse rows produced by [`BOOKMARK_ALL_TEMPLATE`]:
/// `<present 1/0>\t"<name>"\t<remote>\t<tracked 1/0>\t<full-commit>` per
/// local/remote bookmark. A locally-deleted **tombstone** (`present=0`) row is
/// dropped so it can't look like a live local bookmark; its remote-tracking
/// counterpart (`present=1`) is still reported. A row whose name field is empty
/// contributes nothing (mirrors [`parse_bookmarks`]).
pub(crate) fn parse_bookmarks_all(output: &str) -> Vec<BookmarkRef> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let present = fields.next()? == "1";
            let name = decode_json_field(fields.next().unwrap_or(""));
            let remote = fields.next().unwrap_or("");
            let tracked = fields.next() == Some("1");
            let target = fields.next().unwrap_or("").to_string();
            if !present || name.is_empty() {
                return None;
            }
            Some(BookmarkRef {
                name,
                remote: (!remote.is_empty()).then(|| remote.to_string()),
                target,
                tracked,
            })
        })
        .collect()
}

/// Parse rows produced by [`REACHABLE_BOOKMARKS_TEMPLATE`]:
/// `"<name>"[ "<name>"…]\t<full-commit>` (names `.escape_json()`-framed). A commit
/// with several bookmarks yields one [`Bookmark`] per name, all sharing that
/// commit as the target. A row with no bookmark names (empty first field)
/// contributes nothing.
pub(crate) fn parse_reachable_bookmarks(output: &str) -> Vec<Bookmark> {
    let mut out = Vec::new();
    for line in output.lines().filter(|l| !l.is_empty()) {
        let mut fields = line.splitn(2, '\t');
        let names = fields.next().unwrap_or("");
        let target = fields.next().unwrap_or("");
        for name in decode_name_list(names) {
            out.push(Bookmark {
                name,
                target: target.to_string(),
            });
        }
    }
    out
}

/// Parse `jj resolve --list` output: each line is a conflicted path left-aligned
/// in a column, then a run of spaces, then a human conflict description. Take the
/// path (the text before the first 2-space gap), forward-slash normalised (jj
/// emits the OS-native separator here, like `--summary`).
///
/// Consumes **raw bytes** and yields [`PathBuf`]s (via [`path_from_bytes`]) so a
/// non-UTF-8 conflicted path survives losslessly, mirroring the git backend's
/// `conflicted_files`.
pub(crate) fn parse_resolve_list(output: &[u8]) -> Vec<PathBuf> {
    output
        .split(|&b| b == b'\n')
        .filter_map(|line| {
            // The path is the bytes before the first 2-space column gap.
            let cut = find_subslice(line, b"  ").unwrap_or(line.len());
            let path = line[..cut].trim_ascii();
            if path.is_empty() {
                return None;
            }
            Some(path_from_bytes(&normalize_slashes(path)))
        })
        .collect()
}

/// Build a workspace-root [`PathBuf`] from the raw stdout of `jj workspace root`.
///
/// Reads the path from **raw bytes** (not a lossily-decoded `String`) so a
/// workspace root that is not valid UTF-8 (legal on Unix) survives byte-for-byte
/// instead of collapsing to `U+FFFD` — matching the byte-faithful status/diff
/// surface, and what the facade's `WorktreeInfo.path` forwards. jj prints the
/// absolute root path followed by a single line terminator (`\n`, or `\r\n` on
/// Windows, where the path is UTF-8 anyway); strip **only** that terminator — not
/// arbitrary trailing whitespace like `str::trim_end` — so a root path that
/// legitimately ends in a space/tab on Unix is preserved.
pub(crate) fn workspace_root_from_bytes(stdout: &[u8]) -> PathBuf {
    let end = stdout
        .iter()
        .rposition(|&b| b != b'\n' && b != b'\r')
        .map_or(0, |i| i + 1);
    path_from_bytes(&stdout[..end])
}

/// Normalise `\` path separators to `/` on raw bytes — jj's `--summary` /
/// `resolve --list` emit the OS-native separator (backslashes on Windows), which
/// the unified DTO reports forward-slash across backends/platforms.
fn normalize_slashes(path: &[u8]) -> Vec<u8> {
    path.iter()
        .map(|&b| if b == b'\\' { b'/' } else { b })
        .collect()
}

/// Byte-slice `find`: the index of the first occurrence of `needle` in `hay`.
fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Parse rows produced by [`WORKSPACE_TEMPLATE`]:
/// `"<name>"\t<full-commit>\t<bookmarks>`, where the name is `.escape_json()`-framed
/// and the bookmarks are space-joined `.escape_json()` names (and may be empty).
pub(crate) fn parse_workspaces(output: &str) -> Vec<Workspace> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            // The name is `.escape_json()`-framed (no literal tab even when it
            // holds one), so the three columns split cleanly.
            let mut fields = line.split('\t');
            let name = decode_json_field(fields.next()?);
            let commit = fields.next().unwrap_or("").to_string();
            let bookmarks = decode_name_list(fields.next().unwrap_or(""));
            Some(Workspace {
                name,
                commit,
                bookmarks,
            })
        })
        .collect()
}

/// Parse `jj diff --summary`: each line is `<status-letter> <path>`. For a rename
/// (`R`) or copy (`C`) jj renders the path as `prefix{old => new}suffix` rather than
/// a plain path, so those are expanded into the real new path (and the old path is
/// captured on [`ChangedPath::old_path`]). Paths are forward-slash normalised —
/// jj's `--summary` uses the OS-native separator, unlike its `--git` diff (and git
/// itself), so this keeps the unified DTO consistent across backends/platforms.
/// Consumes **raw bytes** (not a lossily-decoded `&str`): the path is part of the
/// payload and, on Unix, need not be valid UTF-8 — the status letter and the
/// `{old => new}` rename framing are ASCII, so they parse byte-wise while the path
/// bytes are carried losslessly (via [`path_from_bytes`]).
pub(crate) fn parse_diff_summary(output: &[u8]) -> Vec<ChangedPath> {
    output
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            // The status letter is a single ASCII byte, followed by the separating
            // space; the remainder is the raw path bytes.
            let status = *line.first()? as char;
            if line.get(1) != Some(&b' ') {
                return None;
            }
            let raw = &line[2..];
            if raw.is_empty() {
                return None;
            }
            let (old_path, path) = if matches!(status, 'R' | 'C') {
                let (old, new) = expand_rename(raw);
                let (old, new) = (normalize_slashes(&old), normalize_slashes(&new));
                // A non-brace `R`/`C` path (malformed — jj always renders renames
                // with the `{old => new}` form) expands to `old == new`; don't
                // report that as a self-rename, so `old_path != path` stays a
                // reliable "is this a real rename?" test for consumers.
                (
                    (old != new).then(|| path_from_bytes(&old)),
                    path_from_bytes(&new),
                )
            } else {
                (None, path_from_bytes(&normalize_slashes(raw)))
            };
            Some(ChangedPath {
                status,
                path,
                old_path,
            })
        })
        .collect()
}

/// Expand jj's rename/copy path form `prefix{left => right}suffix` into
/// `(old, new)` full byte paths. Falls back to `(raw, raw)` when the brace/arrow
/// form isn't present, so a plain path is returned unchanged. `{`, `}`, and ` => `
/// are ASCII, so the byte offsets are exact even for a non-UTF-8 surrounding path.
fn expand_rename(raw: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let plain = || (raw.to_vec(), raw.to_vec());
    let (Some(open), Some(close)) = (
        raw.iter().position(|&b| b == b'{'),
        raw.iter().position(|&b| b == b'}'),
    ) else {
        return plain();
    };
    if open >= close {
        return plain();
    }
    let Some(rel) = find_subslice(&raw[open..close], b" => ") else {
        return plain();
    };
    let arrow = open + rel;
    let prefix = &raw[..open];
    let left = &raw[open + 1..arrow];
    let right = &raw[arrow + 4..close];
    let suffix = &raw[close + 1..];
    (
        [prefix, left, suffix].concat(),
        [prefix, right, suffix].concat(),
    )
}

/// Parse the summary footer of `jj diff --stat`, e.g. `4 files changed, 157
/// insertions(+), 137 deletions(-)` (same shape as git's `--shortstat`). The
/// footer is the last line mentioning "changed"; no such line → all zeros.
pub(crate) fn parse_diff_stat(output: &str) -> DiffStat {
    let summary = output
        .lines()
        .rev()
        .find(|line| line.contains("changed"))
        .unwrap_or("");
    let mut stat = DiffStat::default();
    for part in summary.split(',') {
        let part = part.trim();
        let n = part
            .split_whitespace()
            .next()
            .and_then(|tok| tok.parse().ok())
            .unwrap_or(0);
        if part.contains("file") {
            stat.files_changed = n;
        } else if part.contains("insertion") {
            stat.insertions = n;
        } else if part.contains("deletion") {
            stat.deletions = n;
        }
    }
    stat
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jj_version_parses_real_world_shapes() {
        let v = parse_jj_version("jj 0.38.0").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (0, 38, 0));
        let v = parse_jj_version("jj 0.39.0-dev+abc123").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (0, 39, 0));
        let v = parse_jj_version("jj 1.2").unwrap();
        assert_eq!(v.patch, 0, "missing patch defaults to 0");
        // Ordering drives the supported-floor gate.
        assert!(parse_jj_version("jj 0.37.9").unwrap() < parse_jj_version("jj 0.38.0").unwrap());
        assert!(parse_jj_version("jj").is_none());
    }

    #[test]
    fn operations_split_tab_fields() {
        // RFC-3339 colon offset (`%:z`), user + description `.escape_json()`-framed.
        let out = "abc123\t\"user@host\"\t2026-06-05T10:00:00+02:00\t\"new empty commit\"\n\
                   def456\t\"user@host\"\t2026-06-05T09:59:00+02:00\t\"describe commit\\twith tab\"\n";
        let ops = parse_operations(out);
        assert_eq!(ops.len(), 2);
        assert_eq!(ops[0].id, "abc123");
        assert_eq!(ops[0].user, "user@host");
        assert_eq!(ops[0].time, "2026-06-05T10:00:00+02:00");
        assert_eq!(ops[0].description, "new empty commit");
        // A literal tab in the description survives (splitn keeps the tail).
        assert_eq!(ops[1].description, "describe commit\twith tab");
    }

    #[test]
    fn op_parents_reads_id_and_parent_count() {
        // Newest first: a 2-parent reconcile merge, then two single-parent ops.
        let out = "merge9\t2\nmine01\t1\npre000\t1\n";
        let rows = parse_op_parents(out);
        assert_eq!(
            rows,
            vec![
                ("merge9".to_string(), 2),
                ("mine01".to_string(), 1),
                ("pre000".to_string(), 1),
            ]
        );
        // A short/malformed row (no parent-count column) keeps its id and reads as
        // 0 parents, so it can never spuriously look like the divergence merge.
        let short = parse_op_parents("abc123\n");
        assert_eq!(short, vec![("abc123".to_string(), 0)]);
        assert!(parse_op_parents("").is_empty());
    }

    #[test]
    fn annotate_rows_carry_line_numbers() {
        let out = "kxoyzabc\tfn main() {\nkxoyzabc\t}\nqlmnopqr\t// added later";
        let lines = parse_annotate(out);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].change_id, "kxoyzabc");
        assert_eq!(lines[0].line, 1);
        assert_eq!(lines[0].content, "fn main() {");
        assert_eq!(lines[2].change_id, "qlmnopqr");
        assert_eq!(lines[2].line, 3);
        assert!(parse_annotate("").is_empty());
    }

    // A CRLF-terminated source line keeps its `\r` in the content (the old
    // `.lines()` split silently stripped it), and a trailing newline does not
    // add a phantom row or perturb the 1-based line numbering.
    #[test]
    fn annotate_preserves_cr_and_ignores_trailing_newline() {
        let out = "kxoyzabc\tfn main() {\r\nkxoyzabc\t}\r\n";
        let lines = parse_annotate(out);
        assert_eq!(lines.len(), 2, "no phantom row from the trailing newline");
        assert_eq!(lines[0].content, "fn main() {\r", "CR preserved");
        assert_eq!((lines[1].line, lines[1].content.as_str()), (2, "}\r"));
    }

    // EVOLOG_TEMPLATE renders the same columns as CHANGE_TEMPLATE, so the rows
    // flow through parse_changes unchanged.
    #[test]
    fn evolog_rows_parse_as_changes() {
        let out = "kz\t38\tfalse\t\"feat: parser\"\nkz\t12\ttrue\t\"\"\n";
        let changes = parse_changes(out);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].description, "feat: parser");
        assert!(changes[1].empty);
    }

    #[test]
    fn changes_split_tab_fields() {
        let input = "kztuxlro\t38e00654\tfalse\t\"feat: stuff\"\nqpvuntsm\t6ecf997f\ttrue\t\"\"\n";
        let got = parse_changes(input);
        assert_eq!(got.len(), 2);
        assert_eq!(
            got[0],
            Change {
                change_id: "kztuxlro".into(),
                commit_id: "38e00654".into(),
                empty: false,
                description: "feat: stuff".into(),
            }
        );
        // Undescribed, empty change.
        assert!(got[1].empty);
        assert_eq!(got[1].description, "");
    }

    // A literal tab inside the (first-line) description round-trips: the template
    // `.escape_json()`-frames it as `\t` inside the quoted field, and
    // `decode_json_field` restores the real tab.
    #[test]
    fn changes_keep_tab_in_description() {
        let got = parse_changes("kztuxlro\t38e00654\tfalse\t\"col1\\tcol2\"\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].description, "col1\tcol2");
    }

    // A commit carrying several bookmarks fans out to one entry each, all sharing
    // the commit; a bookmark-less row contributes nothing.
    #[test]
    fn reachable_bookmarks_fan_out_per_name() {
        let got = parse_reachable_bookmarks("\"main\" \"feat\"\tabc123\n\tdef456\n");
        assert_eq!(
            got,
            vec![
                Bookmark {
                    name: "main".into(),
                    target: "abc123".into()
                },
                Bookmark {
                    name: "feat".into(),
                    target: "abc123".into()
                },
            ]
        );
    }

    // The JSON-string decoder inverts every escape jj's `escape_json` emits, and
    // passes a non-quoted field through verbatim (defensive for hex/flag columns
    // and any legacy raw output).
    #[test]
    fn decode_json_field_reverses_escapes() {
        assert_eq!(decode_json_field("\"plain\""), "plain");
        assert_eq!(decode_json_field("\"co,mma\""), "co,mma");
        assert_eq!(decode_json_field("\"a\\tb\""), "a\tb");
        assert_eq!(decode_json_field("\"line\\ntwo\""), "line\ntwo");
        assert_eq!(decode_json_field("\"q\\\"q\""), "q\"q");
        assert_eq!(decode_json_field("\"back\\\\slash\""), "back\\slash");
        assert_eq!(decode_json_field("\"\\u0009tab\""), "\ttab"); // \uXXXX control
        assert_eq!(decode_json_field("\"caf\u{00e9}\""), "caf\u{00e9}"); // raw UTF-8
        assert_eq!(decode_json_field("\"\""), ""); // empty
        // A non-quoted field is returned as-is (a hex id, a flag, or a truncated row).
        assert_eq!(decode_json_field("f5d07685"), "f5d07685");
        assert_eq!(decode_json_field(""), "");
    }

    // The space-joined name list splits back exactly (bookmark names never hold a
    // space), decoding each element; an empty field is an empty list.
    #[test]
    fn decode_name_list_splits_and_decodes() {
        assert_eq!(decode_name_list("\"main\" \"feat\""), vec!["main", "feat"]);
        assert_eq!(decode_name_list("\"co,mma\""), vec!["co,mma"]);
        assert!(decode_name_list("").is_empty());
        // `first_bookmark_name` takes the decoded head, or `None` when absent.
        assert_eq!(
            first_bookmark_name("\"co,mma\" \"main\""),
            Some("co,mma".to_string())
        );
        assert_eq!(first_bookmark_name(""), None);
        assert_eq!(first_bookmark_name("\n"), None);
    }

    // Exotic workspace names — jj permits a tab or newline in a workspace name,
    // and the template `.escape_json()`-frames it so the row still splits on the
    // literal tab and the name round-trips (the old raw `name` stored the escaped
    // form verbatim). A comma-carrying bookmark name likewise survives (the old
    // comma-join mangled it).
    #[test]
    fn workspaces_round_trip_exotic_names() {
        // `"ta\tb"` = a workspace name holding a real tab; the framed field's only
        // literal tabs are the two column separators.
        let input = "\"ta\\tb\"\tc0ffee\t\"co,mma\" \"pl/ain\"\n";
        let got = parse_workspaces(input);
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0].name, "ta\tb",
            "the interior tab is decoded, not split on"
        );
        assert_eq!(got[0].commit, "c0ffee");
        assert_eq!(
            got[0].bookmarks,
            vec!["co,mma".to_string(), "pl/ain".to_string()]
        );
    }

    // Identity ids are the FULL commit id, so two commits that share a short prefix
    // stay distinct — a short-prefix key would collide and cross-reference wrongly.
    #[test]
    fn full_ids_disambiguate_a_shared_short_prefix() {
        let a = "abcdef0123456789abcdef0123456789abcdef01";
        let b = "abcdef0123456789ffffffffffffffffffffffff"; // same 16-char prefix
        let bms = parse_bookmarks(&format!("1\t\t\"one\"\t{a}\n1\t\t\"two\"\t{b}\n"));
        assert_eq!(bms[0].target, a);
        assert_eq!(bms[1].target, b);
        assert_ne!(bms[0].target, bms[1].target, "full ids must not collide");
        // The same holds for the workspace commit (the WorktreeInfo.commit source).
        let ws = parse_workspaces(&format!("\"w1\"\t{a}\t\n\"w2\"\t{b}\t\n"));
        assert_ne!(ws[0].commit, ws[1].commit);
    }

    #[test]
    fn resolve_list_extracts_paths_before_description() {
        let got = parse_resolve_list(
            b"src/a.rs    2-sided conflict\nb.txt    2-sided conflict including 1 deletion\n",
        );
        assert_eq!(got, vec![PathBuf::from("src/a.rs"), PathBuf::from("b.txt")]);
        assert!(parse_resolve_list(b"").is_empty());
        // OS-native backslash separators (Windows) are normalised to `/`.
        assert_eq!(
            parse_resolve_list(b"sub\\c.txt    2-sided conflict\n"),
            vec![PathBuf::from("sub/c.txt")]
        );
    }

    // A non-UTF-8 conflicted path (legal on Unix) survives byte-for-byte.
    #[cfg(unix)]
    #[test]
    fn resolve_list_preserves_non_utf8_path_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let got = parse_resolve_list(b"caf\xff.txt    2-sided conflict\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].as_os_str().as_bytes(), b"caf\xff.txt");
    }

    #[test]
    fn workspace_root_strips_only_the_trailing_line_terminator() {
        // jj prints the root path then one `\n` (a `\r\n` on Windows).
        assert_eq!(
            workspace_root_from_bytes(b"/repo/ws\n"),
            PathBuf::from("/repo/ws")
        );
        assert_eq!(
            workspace_root_from_bytes(b"/repo/ws\r\n"),
            PathBuf::from("/repo/ws")
        );
        // No terminator at all is fine, and all-empty yields an empty path.
        assert_eq!(
            workspace_root_from_bytes(b"/repo/ws"),
            PathBuf::from("/repo/ws")
        );
        assert_eq!(workspace_root_from_bytes(b"\n"), PathBuf::new());
    }

    // A workspace root whose bytes are not valid UTF-8 (legal on Unix) survives
    // byte-for-byte, so the facade's `WorktreeInfo.path` names the SAME directory;
    // a trailing space (a legal path byte) is kept — only the `\n` is stripped.
    #[cfg(unix)]
    #[test]
    fn workspace_root_preserves_non_utf8_and_trailing_space() {
        use std::os::unix::ffi::OsStrExt;
        let got = workspace_root_from_bytes(b"/repo/ws-caf\xff \n");
        assert_eq!(got.as_os_str().as_bytes(), b"/repo/ws-caf\xff ");
    }

    #[test]
    fn bookmarks_parse_name_and_commit_from_template() {
        // Rows produced by BOOKMARK_LIST_TEMPLATE:
        // `<present>\t<remote>\t"<name>"\t<full-commit>`. Two live local bookmarks.
        let input = "1\t\t\"main\"\tf5d07685\n1\t\t\"feature\"\tdeadbeef\n";
        let got = parse_bookmarks(input);
        assert_eq!(
            got,
            vec![
                Bookmark {
                    name: "main".into(),
                    target: "f5d07685".into()
                },
                Bookmark {
                    name: "feature".into(),
                    target: "deadbeef".into()
                },
            ]
        );
    }

    // The tombstone fix (T-041): a locally-deleted bookmark that a remote still
    // tracks renders a `present=0` local row PLUS a `present=1` remote-tracking
    // row — neither may be reported as a live local branch. A *conflicted*
    // bookmark (`present=1`, empty target) IS live and must be kept; an empty name
    // contributes nothing. An exotic name with a comma round-trips via escaping.
    #[test]
    fn bookmarks_filter_tombstones_but_keep_conflicted() {
        let input = concat!(
            "1\t\t\"live\"\tf5d07685\n",       // live local → kept
            "0\t\t\"tomb\"\t\n",               // deleted local tombstone → dropped
            "1\torigin\t\"tomb\"\tdeadbeef\n", // its remote-tracking row → dropped
            "1\t\t\"conflicted\"\t\n",         // present, no single target → kept
            "1\t\t\"co,mma\"\tcafef00d\n",     // comma in name → decoded intact
            "1\t\t\"\"\t\n",                   // empty name → dropped
        );
        let got = parse_bookmarks(input);
        assert_eq!(
            got,
            vec![
                Bookmark {
                    name: "live".into(),
                    target: "f5d07685".into()
                },
                Bookmark {
                    name: "conflicted".into(),
                    target: String::new()
                },
                Bookmark {
                    name: "co,mma".into(),
                    target: "cafef00d".into()
                },
            ],
            "only live LOCAL bookmarks survive; the tombstone never looks alive"
        );
    }

    // `parse_bookmarks_all` drops a row whose name field is empty and a
    // locally-deleted `present=0` tombstone, matching `parse_bookmarks` — no
    // phantom `BookmarkRef { name: "" }` or ghost-local leaks through. Rows:
    // `<present>\t"<name>"\t<remote>\t<tracked>\t<full-commit>`.
    #[test]
    fn bookmarks_all_drops_empty_name_and_tombstone_rows() {
        let input = concat!(
            "1\t\"main\"\t\t1\tf5d07685\n",       // live local
            "1\t\"\"\torigin\t1\tdeadbeef\n",     // empty name → dropped
            "1\t\"feat\"\torigin\t0\tcafef00d\n", // remote-tracking
            "0\t\"gone\"\t\t0\t\n",               // deleted local tombstone → dropped
        );
        let got = parse_bookmarks_all(input);
        assert_eq!(
            got,
            vec![
                BookmarkRef {
                    name: "main".into(),
                    remote: None,
                    target: "f5d07685".into(),
                    tracked: true,
                },
                BookmarkRef {
                    name: "feat".into(),
                    remote: Some("origin".into()),
                    target: "cafef00d".into(),
                    tracked: false,
                },
            ],
            "the empty-name and tombstone rows must contribute nothing"
        );
    }

    #[test]
    fn workspaces_split_tab_fields_and_bookmarks() {
        let input = "\"default\"\te2aa3420\t\"main\" \"feature\"\n\"ws1\"\t12345678\t\n";
        let got = parse_workspaces(input);
        assert_eq!(got.len(), 2);
        assert_eq!(
            got[0],
            Workspace {
                name: "default".into(),
                commit: "e2aa3420".into(),
                bookmarks: vec!["main".into(), "feature".into()],
            }
        );
        // No bookmarks → empty vec, not [""].
        assert!(got[1].bookmarks.is_empty());
    }

    #[test]
    fn diff_summary_splits_status_and_path() {
        let got = parse_diff_summary(b"M src/lib.rs\nA new file.txt\nD gone.rs\n");
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].status, 'M');
        assert_eq!(got[1].path, PathBuf::from("new file.txt"));
        assert!(got[1].old_path.is_none());
        assert_eq!(got[2].status, 'D');
    }

    // A non-UTF-8 summary path (legal on Unix) survives byte-for-byte.
    #[cfg(unix)]
    #[test]
    fn diff_summary_preserves_non_utf8_path_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let got = parse_diff_summary(b"M caf\xff.txt\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path.as_os_str().as_bytes(), b"caf\xff.txt");
    }

    // jj renders a rename/copy path as `prefix{old => new}suffix` (verified against
    // jj 0.38); it must be expanded into the real new path with the old path
    // captured — not stored raw. A plain `M`/`A`/`D` path is left untouched.
    #[test]
    fn diff_summary_expands_rename_and_copy() {
        let got =
            parse_diff_summary(b"R {old.rs => new.rs}\nC sub/{a.rs => b.rs}\nM lit{eral}.rs\n");
        assert_eq!(got[0].status, 'R');
        assert_eq!(got[0].path, PathBuf::from("new.rs"));
        assert_eq!(
            got[0].old_path.as_deref(),
            Some(PathBuf::from("old.rs").as_path())
        );
        assert_eq!(got[1].path, PathBuf::from("sub/b.rs"));
        assert_eq!(
            got[1].old_path.as_deref(),
            Some(PathBuf::from("sub/a.rs").as_path())
        );
        // A literal `{...}` in a non-rename path (no ` => `) is not mis-expanded.
        assert_eq!(got[2].path, PathBuf::from("lit{eral}.rs"));
        assert!(got[2].old_path.is_none());
    }

    // jj `--summary` emits OS-native separators (backslashes on Windows); paths are
    // normalised to forward slashes to match the `--git` diff and the git backend.
    #[test]
    fn diff_summary_normalises_backslash_separators() {
        let got = parse_diff_summary(b"M deep\\nested\\f.rs\nR win\\{a.rs => b.rs}\n");
        assert_eq!(got[0].path, PathBuf::from("deep/nested/f.rs"));
        assert_eq!(got[1].path, PathBuf::from("win/b.rs"));
        assert_eq!(
            got[1].old_path.as_deref(),
            Some(PathBuf::from("win/a.rs").as_path())
        );
    }

    #[test]
    fn diff_stat_parses_footer_among_per_file_lines() {
        let input = "README.md | 10 +++---\n\
                     src/lib.rs | 4 +-\n\
                     4 files changed, 157 insertions(+), 137 deletions(-)\n";
        assert_eq!(parse_diff_stat(input), DiffStat::new(4, 157, 137));
        assert_eq!(parse_diff_stat(""), DiffStat::default());
    }
}

// Property-based fuzzing: pure parsers over arbitrary jj output must never
// panic, with special attention to `expand_rename` (byte-offset arithmetic on
// `{old => new}` braces) and the templated tab-row parsers.
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// jj's structural vocabulary: `diff --summary` letters, brace renames
    /// (incl. multibyte around the braces), template tab-rows, and diff text.
    fn structured_line() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("M src/a.rs\n".to_string()),
            Just("R sub\\{old.rs => new.rs}\n".to_string()),
            Just("C {a => b}.rs\n".to_string()),
            "[A-Z] \\{[a-zé]{0,6} => [a-zé]{0,6}\\}\n", // rename braces + multibyte
            "[a-zé]{0,8}\t[a-zé]{0,8}\t(true|false)\t[a-zé\t]{0,10}\n", // change row
            "[a-zé]{0,8}\t[a-zé@]{0,8}\t[01]\t[a-zé]{0,8}\n", // bookmark row
            "[-+ ]?[a-zé]{0,10}\n",                     // diff body
        ]
    }

    fn structured_doc() -> impl Strategy<Value = String> {
        prop::collection::vec(structured_line(), 0..40).prop_map(|lines| lines.concat())
    }

    /// A standard JSON string encoder — the reference for what jj's `escape_json`
    /// emits (verified byte-for-byte against jj 0.42). Round-tripping arbitrary
    /// text through this and back through [`decode_json_field`] proves the framing
    /// decoder inverts the real template output for names/descriptions with any
    /// mix of spaces, commas, tabs, quotes, backslashes, and newlines.
    fn json_encode(s: &str) -> String {
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{0008}' => out.push_str("\\b"),
                '\u{000C}' => out.push_str("\\f"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    proptest! {
        // The framing decoder inverts a standard JSON-string encode for ANY text —
        // the round-trip the machine templates rely on for names/descriptions.
        #[test]
        fn json_field_round_trips(s in any::<String>()) {
            prop_assert_eq!(decode_json_field(&json_encode(&s)), s);
        }

        // A full change row (id/flag columns raw, description `.escape_json()`-framed)
        // round-trips through `parse_changes`: the description recovers exactly even
        // with tabs/quotes/backslashes, and the structural columns are untouched.
        #[test]
        fn change_row_round_trips(desc in any::<String>()) {
            // jj's `first_line()` yields a single line; mirror that for a realistic
            // fixture (the framing still handles an embedded newline, but a real row
            // never carries one here).
            let first: String = desc.split(['\n', '\r']).next().unwrap_or("").to_string();
            let row = format!("chg12345678\tcmt87654321\tfalse\t{}\n", json_encode(&first));
            let got = parse_changes(&row);
            prop_assert_eq!(got.len(), 1);
            prop_assert_eq!(got[0].change_id.as_str(), "chg12345678");
            prop_assert_eq!(got[0].commit_id.as_str(), "cmt87654321");
            prop_assert!(!got[0].empty);
            prop_assert_eq!(&got[0].description, &first);
        }

        // A space-joined list of escaped bookmark names round-trips (names never
        // contain a space, so the join is reversible). Uses a comma/slash/dot
        // alphabet — the exotic-but-space-free shapes a git-imported name can take.
        #[test]
        fn name_list_round_trips(names in prop::collection::vec("[a-z,./-]{1,8}", 0..6)) {
            let field = names.iter().map(|n| json_encode(n)).collect::<Vec<_>>().join(" ");
            prop_assert_eq!(decode_name_list(&field), names);
        }

        #[test]
        fn parsers_never_panic_on_arbitrary_text(s in any::<String>()) {
            let _ = parse_changes(&s);
            let _ = parse_operations(&s);
            let _ = parse_annotate(&s);
            let _ = parse_bookmarks(&s);
            let _ = parse_bookmarks_all(&s);
            let _ = parse_reachable_bookmarks(&s);
            let _ = parse_resolve_list(s.as_bytes());
            let _ = parse_workspaces(&s);
            let _ = parse_diff_summary(s.as_bytes());
            let _ = parse_diff_stat(&s);
            let _ = parse_jj_version(&s);
            let _ = expand_rename(s.as_bytes());
        }

        // The byte parsers must also never panic on *arbitrary bytes* — the actual
        // shape of jj machine output carrying a non-UTF-8 path.
        #[test]
        fn byte_parsers_never_panic_on_arbitrary_bytes(b in any::<Vec<u8>>()) {
            let _ = parse_resolve_list(&b);
            let _ = parse_diff_summary(&b);
            let _ = expand_rename(&b);
            let _ = workspace_root_from_bytes(&b);
        }

        #[test]
        fn parsers_never_panic_on_structured_text(s in structured_doc()) {
            let _ = parse_diff_summary(s.as_bytes());
            let _ = parse_changes(&s);
            let _ = parse_bookmarks_all(&s);
        }

        // expand_rename returns the raw verbatim for a non-brace input (its
        // documented identity for the no-rename case).
        #[test]
        fn expand_rename_is_identity_without_braces(s in "[a-zé/ ]{0,20}") {
            prop_assume!(!s.contains('{') && !s.contains('}'));
            let bytes = s.into_bytes();
            prop_assert_eq!(expand_rename(&bytes), (bytes.clone(), bytes));
        }
    }
}
