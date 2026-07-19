//! The unified-diff model and parser, shared by `vcs-git` and `vcs-jj`.
//!
//! `git diff` and `jj diff --git` emit the same git-format unified diff, so a
//! single parser serves both. (They're byte-identical for ASCII paths; they differ
//! only in how a non-ASCII filename is rendered — git's default `core.quotePath`
//! octal-C-quotes it, jj writes raw UTF-8 — and the parser decodes both.) Pure
//! functions over arbitrary text — no process execution.

use std::path::PathBuf;

use crate::pathbytes::path_from_bytes;

/// What a diff call compares — the working tree/copy, or a specific
/// revision/revset (or range).
///
/// Shared by the `vcs-git` and `vcs-jj` wrappers (re-exported as
/// `vcs_git::DiffSpec` / `vcs_jj::DiffSpec`); each backend interprets it against
/// its own CLI (`git diff …` / `jj diff -r …`).
///
/// Deliberately **not** `#[non_exhaustive]`: each backend's `diff` interpreter
/// must handle every variant, so adding one is a (pre-1.0) breaking change that
/// fails the wrappers' exhaustive matches at compile time rather than slipping
/// through a runtime catch-all.
#[derive(Debug, Clone)]
pub enum DiffSpec {
    /// All tracked changes in the working tree/copy vs the last commit — staged
    /// or not, excluding untracked files (`git diff HEAD`; `jj diff -r @`).
    WorkingTree,
    /// A specific revision/revset or range, e.g. `HEAD~1` / `main..HEAD`
    /// (`git diff <rev>`) or `@-` / `main..@` (`jj diff -r <revset>`).
    ///
    /// This crate is intentionally plain data — no I/O, no validation — so
    /// this string is passed through unchecked; guarding it against a
    /// flag-like value (a leading `-`) is each backend wrapper's job, and the
    /// two differ: `vcs-git` runs an inline `reject_flag_like` check (plus a
    /// trailing `--`) before using it, while `vcs-jj` relies on it landing in
    /// `jj`'s `-r <revset>` flag-value slot, which the CLI itself rejects if
    /// dash-prefixed. Don't assume either guarantee from this type alone.
    Rev(String),
}

/// Aggregate line/file counts from a diff stat (`git diff --shortstat`,
/// `jj diff --stat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DiffStat {
    /// Number of files changed.
    pub files_changed: usize,
    /// Lines added (`insertions(+)`).
    pub insertions: usize,
    /// Lines removed (`deletions(-)`).
    pub deletions: usize,
}

impl DiffStat {
    /// Build a [`DiffStat`]. (A constructor, because the struct is
    /// `#[non_exhaustive]` — the parser crates and tests can't use struct-literal
    /// syntax across the crate boundary.)
    pub fn new(files_changed: usize, insertions: usize, deletions: usize) -> Self {
        Self {
            files_changed,
            insertions,
            deletions,
        }
    }

    /// Parse a single `git diff --shortstat` / `jj diff --stat` summary clause,
    /// e.g. ` 3 files changed, 12 insertions(+), 4 deletions(-)`. Any of the
    /// three sub-clauses may be absent (a pure-insertion diff omits `deletions`;
    /// no changes at all yields an empty string → all zeros) — a missing or
    /// unparsable count defaults to `0` rather than erroring, since this is fed
    /// arbitrary CLI text.
    ///
    /// Shared by `vcs_git::parse::parse_shortstat` and
    /// `vcs_jj::parse::parse_diff_stat`, which were previously byte-identical
    /// past their own preprocessing (jj additionally selects the last line
    /// mentioning "changed" before calling this). The keyed-substring matching
    /// ("file"/"insertion"/"deletion") assumes the **English/C-locale** wording
    /// both CLIs emit under the C locale the callers force — see their own
    /// `LC_ALL=C` comments at the call site.
    pub fn parse(summary: &str) -> Self {
        let mut stat = Self::default();
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
}

/// How a file changed in a unified diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ChangeKind {
    /// A new file (`new file mode …`).
    Added,
    /// An existing file's contents changed.
    Modified,
    /// The file was removed (`deleted file mode …`).
    Deleted,
    /// The file was renamed (`rename from …` / `rename to …`).
    Renamed,
}

/// One line inside a [`Hunk`], tagged by its role. The stored text excludes the
/// leading ` `/`+`/`-` marker **and the line terminator** — a CRLF-origin diff's
/// trailing `\r` is stripped along with the `\n`, so reconstruct exact bytes
/// from [`FileDiff::raw`], not from these lines.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DiffLine {
    /// Unchanged context line (leading ` `).
    Context(String),
    /// Added line (leading `+`).
    Added(String),
    /// Removed line (leading `-`).
    Removed(String),
}

/// A single `@@ … @@` hunk within a [`FileDiff`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Hunk {
    /// Start line in the old file (the `-<start>` of the `@@` header).
    pub old_start: usize,
    /// Line count in the old file (defaults to 1 when the `,<count>` is omitted).
    pub old_lines: usize,
    /// Start line in the new file (the `+<start>` of the `@@` header).
    pub new_start: usize,
    /// Line count in the new file (defaults to 1 when the `,<count>` is omitted).
    pub new_lines: usize,
    /// Text after the closing `@@` (the function/section heading); empty when none.
    pub section: String,
    /// The hunk body, one entry per `+`/`-`/` ` line.
    pub lines: Vec<DiffLine>,
}

/// One file's entry in a parsed git-format unified diff (`git diff` or
/// `jj diff --git`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FileDiff {
    /// How the file changed.
    pub change: ChangeKind,
    /// The file's path — the *new* path for a rename — forward-slash normalised.
    ///
    /// A [`PathBuf`] (not a `String`) so a non-UTF-8 filename is carried
    /// losslessly: git C-quotes a non-ASCII path into octal escapes that decode
    /// back to the exact bytes, kept here via [`path_from_bytes`] rather than
    /// substituted with `U+FFFD`. (For jj's raw-UTF-8 `--git` diff a non-UTF-8
    /// path is still subject to the surrounding text layer's decode; the
    /// byte-faithful cross-backend round-trip is the status/conflict path, which
    /// carries `PathBuf` end to end.)
    pub path: PathBuf,
    /// For a rename, the original path (forward-slash normalised); `None` otherwise.
    pub old_path: Option<PathBuf>,
    /// The `@@` hunks; empty for a binary file or a pure rename with no edits.
    pub hunks: Vec<Hunk>,
    /// The verbatim diff section for this file (the `diff --git …` block through
    /// to the next file), for callers that display the raw text.
    pub raw: String,
}

/// Parse a git-format unified diff into one [`FileDiff`] per file. Works on
/// `git diff` and `jj diff --git` output alike. Public so a consumer can parse
/// diff text it obtained by other means.
///
/// Paths are read from the unambiguous single-path lines (`+++ b/…`, `--- a/…`,
/// `rename to …`) rather than the space-ambiguous `diff --git a/… b/…` header,
/// and normalised to forward slashes. Ported from the `vcs-flow-commit` parser.
pub fn parse_diff(diff: &str) -> Vec<FileDiff> {
    diff_sections(diff).filter_map(parse_section).collect()
}

/// Slice a git-format diff into per-file sections (each starts at `diff --git`).
fn diff_sections(full: &str) -> impl Iterator<Item = &str> {
    let mut bounds = Vec::new();
    let mut idx = 0;
    for line in full.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            bounds.push(idx);
        }
        idx += line.len();
    }
    let ends = bounds
        .iter()
        .skip(1)
        .copied()
        .chain(std::iter::once(full.len()));
    bounds
        .clone()
        .into_iter()
        .zip(ends)
        .map(move |(s, e)| &full[s..e])
        .collect::<Vec<_>>()
        .into_iter()
}

/// Determine the [`FileDiff`] for one `diff --git` section: change kind and path
/// from the header lines, plus every `@@` hunk and its body.
fn parse_section(section: &str) -> Option<FileDiff> {
    let mut kind = ChangeKind::Modified;
    // Paths are accumulated as raw bytes (not `String`) so a git C-quoted
    // non-ASCII path decodes to its exact bytes and reaches `path_from_bytes`
    // without a lossy round-trip through `String`.
    let mut new_path: Option<Vec<u8>> = None;
    let mut minus_path: Option<Vec<u8>> = None;
    let mut rename_to: Option<Vec<u8>> = None;
    let mut rename_from: Option<Vec<u8>> = None;
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;

    for line in section.lines() {
        if let Some(hunk) = parse_hunk_header(line) {
            if let Some(done) = current.replace(hunk) {
                hunks.push(done);
            }
            continue;
        }
        if let Some(hunk) = current.as_mut() {
            // Inside a hunk body: classify by the leading marker. `\ No newline at
            // end of file` annotations and any stray blank line are dropped.
            match line.as_bytes().first() {
                Some(b' ') => hunk.lines.push(DiffLine::Context(line[1..].to_string())),
                Some(b'+') => hunk.lines.push(DiffLine::Added(line[1..].to_string())),
                Some(b'-') => hunk.lines.push(DiffLine::Removed(line[1..].to_string())),
                _ => {}
            }
            continue;
        }
        // Header region (before the first `@@`).
        if line.starts_with("new file") {
            kind = ChangeKind::Added;
        } else if line.starts_with("deleted file") {
            kind = ChangeKind::Deleted;
        } else if let Some(p) = line.strip_prefix("rename to ") {
            // `rename to`/`from` carry a *bare* path (no `a/`/`b/`), possibly git-
            // C-quoted when it has a non-ASCII/tab/quote/backslash byte.
            rename_to = Some(unquote_git_path(p.trim_end()));
        } else if let Some(p) = line.strip_prefix("rename from ") {
            rename_from = Some(unquote_git_path(p.trim_end()));
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            // `b/<path>`, or `"b/<path>"` quoted (the `b/` is *inside* the quotes),
            // or `/dev/null` (deleted side). Unquote, then strip the `b/` — a
            // `/dev/null` (no `b/`) yields `None`, leaving `new_path` unset.
            new_path = strip_side_prefix(unquote_git_path(rest.trim_end()), b"b/");
        } else if let Some(rest) = line.strip_prefix("--- ") {
            minus_path = strip_side_prefix(unquote_git_path(rest.trim_end()), b"a/");
        }
    }
    if let Some(done) = current.take() {
        hunks.push(done);
    }

    // A rename keeps its old path so a caller can record the deletion too.
    let old_path = if rename_to.is_some() {
        kind = ChangeKind::Renamed;
        rename_from
    } else {
        None
    };
    // Resolve the path by priority (rename target → `+++ b/` → `--- a/` → the
    // `diff --git` header), skipping any source that is present-but-empty so a
    // malformed `+++ b/`-with-no-path falls through rather than yielding a FileDiff
    // with an empty path. If every source is absent/empty, the section is dropped.
    let path = [rename_to, new_path, minus_path]
        .into_iter()
        .flatten()
        .find(|p| !p.is_empty())
        .or_else(|| header_b_path(section))?;
    Some(FileDiff {
        change: kind,
        path: path_from_bytes(&path),
        old_path: old_path.map(|p| path_from_bytes(&p)),
        hunks,
        raw: section.to_string(),
    })
}

/// Strip a leading `a/` / `b/` (or any) prefix from a raw path, byte-wise;
/// `None` when it is absent (so a `/dev/null` side yields no path).
fn strip_side_prefix(path: Vec<u8>, prefix: &[u8]) -> Option<Vec<u8>> {
    path.strip_prefix(prefix).map(<[u8]>::to_vec)
}

/// Parse a hunk header `@@ -<os>[,<ol>] +<ns>[,<nl>] @@[ <section>]` into an empty
/// [`Hunk`]; `None` for any other line.
fn parse_hunk_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ ")?;
    let (ranges, section) = rest.split_once(" @@")?;
    let mut parts = ranges.split_whitespace();
    let (old_start, old_lines) = parse_hunk_range(parts.next()?.strip_prefix('-')?);
    let (new_start, new_lines) = parse_hunk_range(parts.next()?.strip_prefix('+')?);
    Some(Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
        section: section.strip_prefix(' ').unwrap_or(section).to_string(),
        lines: Vec::new(),
    })
}

/// Parse a `<start>[,<count>]` hunk range; an omitted count means 1 line.
fn parse_hunk_range(range: &str) -> (usize, usize) {
    match range.split_once(',') {
        Some((start, count)) => (start.parse().unwrap_or(0), count.parse().unwrap_or(0)),
        None => (range.parse().unwrap_or(0), 1),
    }
}

/// Fallback path extraction for sections with no `+++`/`---`/`rename` lines
/// (e.g. binary files): the `b/<new>` of the `diff --git` header. Handles both the
/// unquoted `a/<p> b/<p>` form and git's C-quoted `"a/<p>" "b/<p>"` form (a
/// non-ASCII / special-byte path). The unquoted form is ambiguous only when a path
/// contains the literal `" b/"`, which binary-with-spaces makes rare.
fn header_b_path(section: &str) -> Option<Vec<u8>> {
    let first = section.lines().next()?;
    let s = first.strip_prefix("diff --git ")?;
    // Quoted header: the b-side is the last `"b/…"` token (for the binary/mode-only
    // sections this fallback serves, both sides share one path and one quoting).
    let path = if let Some(q) = s.rfind("\"b/") {
        strip_side_prefix(unquote_git_path(&s[q..]), b"b/").unwrap_or_default()
    } else {
        let idx = s.find(" b/")?;
        strip_side_prefix(unquote_git_path(&s[idx + 1..]), b"b/").unwrap_or_default()
    };
    // A `diff --git a/x b/` with no path after `b/` yields nothing, not an empty
    // path — so a malformed header drops the section instead of an empty FileDiff.
    (!path.is_empty()).then_some(path)
}

/// Decode a git **C-quoted** path. git wraps a path in double quotes and C-escapes
/// it when it contains a control byte, a `"`, a `\`, or — with the default
/// `core.quotePath=true` — any non-ASCII (high) byte (e.g. `é` → `\303\251`). A path
/// that is *not* quoted (no leading `"`) is returned unchanged, so callers can apply
/// this unconditionally. Octal escapes decode to raw bytes, so a multi-byte UTF-8
/// filename round-trips; the **raw decoded bytes** are returned (the caller builds
/// a lossless [`PathBuf`] via [`path_from_bytes`]) instead of a lossily-decoded
/// `String` — a non-UTF-8 path would otherwise be corrupted to `U+FFFD` here.
/// Decoding stops at the first unescaped closing quote (trailing bytes are ignored).
fn unquote_git_path(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'"') {
        return bytes.to_vec();
    }
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 1; // skip the opening quote
    while i < bytes.len() {
        match bytes[i] {
            b'"' => break, // unescaped closing quote
            b'\\' if i + 1 < bytes.len() => {
                i += 1;
                match bytes[i] {
                    b'a' => out.push(0x07),
                    b'b' => out.push(0x08),
                    b't' => out.push(b'\t'),
                    b'n' => out.push(b'\n'),
                    b'v' => out.push(0x0b),
                    b'f' => out.push(0x0c),
                    b'r' => out.push(b'\r'),
                    b'"' => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    d @ b'0'..=b'7' => {
                        // Up to 3 octal digits → one byte (`\NNN`, NNN ≤ 0o377).
                        let mut val = u32::from(d - b'0');
                        let mut taken = 0;
                        while taken < 2
                            && i + 1 < bytes.len()
                            && (b'0'..=b'7').contains(&bytes[i + 1])
                        {
                            i += 1;
                            val = val * 8 + u32::from(bytes[i] - b'0');
                            taken += 1;
                        }
                        out.push(val as u8);
                    }
                    other => out.push(other), // unknown escape: keep the byte
                }
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_covers_add_modify_delete_rename() {
        // Add (new), modify (mod), delete (gone), and a directory-changing rename
        // (old/f -> new/f). Ported from the vcs-flow section-parser test.
        let full = concat!(
            "diff --git a/new b/new\n",
            "new file mode 100644\n--- /dev/null\n+++ b/new\n@@ -0,0 +1 @@\n+n\n",
            "diff --git a/mod b/mod\n",
            "--- a/mod\n+++ b/mod\n@@ -1 +1 @@\n-a\n+b\n",
            "diff --git a/gone b/gone\n",
            "deleted file mode 100644\n--- a/gone\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n",
            "diff --git a/old/f.txt b/new/f.txt\n",
            "similarity index 100%\nrename from old/f.txt\nrename to new/f.txt\n",
        );
        let files = parse_diff(full);
        let kinds: Vec<_> = files
            .iter()
            .map(|f| (f.path.to_str().unwrap(), f.change))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("new", ChangeKind::Added),
                ("mod", ChangeKind::Modified),
                ("gone", ChangeKind::Deleted),
                ("new/f.txt", ChangeKind::Renamed),
            ]
        );
        // The rename carries its old path so the deletion is recorded too.
        let rename = files
            .iter()
            .find(|f| f.change == ChangeKind::Renamed)
            .unwrap();
        assert_eq!(
            rename.old_path.as_deref(),
            Some(std::path::Path::new("old/f.txt"))
        );
    }

    #[test]
    fn diff_handles_space_paths() {
        // git appends a trailing tab to `+++`/`---` paths containing spaces; the
        // path must survive intact (the `diff --git` header is ambiguous here).
        let full = "diff --git a/a b/c.txt b/a b/c.txt\n--- a/a b/c.txt\t\n+++ b/a b/c.txt\t\n@@ -1 +1 @@\n-x\n+y\n";
        let files = parse_diff(full);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, std::path::Path::new("a b/c.txt"));
    }

    // git C-quotes a path with a non-ASCII byte (default `core.quotePath=true`).
    // These fixtures are verbatim `git diff` output for a file named `café.txt`
    // (`é` = UTF-8 0xC3 0xA9 = octal \303\251). The parser must unquote them rather
    // than dropping the file. (Captured from real git 2.x.)
    #[test]
    fn diff_unquotes_non_ascii_modify() {
        let full = concat!(
            "diff --git \"a/caf\\303\\251.txt\" \"b/caf\\303\\251.txt\"\n",
            "index 45b983b..b023018 100644\n",
            "--- \"a/caf\\303\\251.txt\"\n",
            "+++ \"b/caf\\303\\251.txt\"\n",
            "@@ -1 +1 @@\n-hi\n+bye\n",
        );
        let files = parse_diff(full);
        assert_eq!(files.len(), 1, "the non-ASCII file must not be dropped");
        assert_eq!(files[0].path, std::path::Path::new("café.txt"));
        assert_eq!(files[0].change, ChangeKind::Modified);
    }

    #[test]
    fn diff_unquotes_non_ascii_rename() {
        let full = concat!(
            "diff --git \"a/caf\\303\\251.txt\" \"b/r\\303\\251sum\\303\\251.txt\"\n",
            "similarity index 100%\n",
            "rename from \"caf\\303\\251.txt\"\n",
            "rename to \"r\\303\\251sum\\303\\251.txt\"\n",
        );
        let files = parse_diff(full);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, std::path::Path::new("résumé.txt"));
        assert_eq!(files[0].change, ChangeKind::Renamed);
        assert_eq!(
            files[0].old_path.as_deref(),
            Some(std::path::Path::new("café.txt"))
        );
    }

    // A binary/mode-only quoted section (no `+++`/`---`/rename lines) resolves its
    // path from the quoted `diff --git` header via `header_b_path`.
    #[test]
    fn diff_unquotes_quoted_header_fallback() {
        let full = concat!(
            "diff --git \"a/caf\\303\\251.bin\" \"b/caf\\303\\251.bin\"\n",
            "index 0000000..1111111 100644\n",
            "Binary files \"a/caf\\303\\251.bin\" and \"b/caf\\303\\251.bin\" differ\n",
        );
        let files = parse_diff(full);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, std::path::Path::new("café.bin"));
    }

    // A path with a literal tab is also C-quoted (`\t`), independent of quotePath.
    #[test]
    fn diff_unquotes_escaped_tab_path() {
        let full = "diff --git \"a/a\\tb.txt\" \"b/a\\tb.txt\"\n--- \"a/a\\tb.txt\"\n+++ \"b/a\\tb.txt\"\n@@ -1 +1 @@\n-x\n+y\n";
        let files = parse_diff(full);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, std::path::Path::new("a\tb.txt"));
    }

    // Git already emits `/` as its path separator. A literal backslash is C-quoted
    // in diff output and must survive decoding unchanged on Unix.
    #[cfg(unix)]
    #[test]
    fn diff_preserves_c_quoted_backslash_path_on_unix() {
        let full = "diff --git \"a/a\\\\b.txt\" \"b/a\\\\b.txt\"\n--- \"a/a\\\\b.txt\"\n+++ \"b/a\\\\b.txt\"\n@@ -1 +1 @@\n-x\n+y\n";
        let files = parse_diff(full);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, std::path::Path::new("a\\b.txt"));
    }

    #[test]
    fn unquote_git_path_decodes_escapes_and_passes_through_plain() {
        // The decoder now yields raw bytes (the caller builds a lossless PathBuf).
        assert_eq!(unquote_git_path("b/plain.txt"), b"b/plain.txt".to_vec()); // not quoted
        assert_eq!(
            unquote_git_path("\"b/caf\\303\\251.txt\""),
            "b/café.txt".as_bytes().to_vec()
        ); // octal → the exact UTF-8 bytes
        assert_eq!(unquote_git_path("\"a\\tb\""), b"a\tb".to_vec()); // \t
        assert_eq!(unquote_git_path("\"a\\\\b\""), b"a\\b".to_vec()); // \\
        assert_eq!(unquote_git_path("\"a\\\"b\""), b"a\"b".to_vec()); // \"
        // A non-UTF-8 octal escape (0xFF) survives byte-for-byte — the whole point.
        assert_eq!(unquote_git_path("\"\\377.bin\""), b"\xff.bin".to_vec());
    }

    #[test]
    fn diff_drops_sections_with_no_resolvable_path() {
        // A header whose `b/` carries no path, and no `+++`/`---`/rename lines:
        // there is no usable path, so the section is dropped (no empty-path FileDiff).
        let bad = "diff --git a/x b/\nbinary files differ\n";
        assert!(parse_diff(bad).is_empty());
        // An empty `+++ b/` (and no `--- a/`) falls through to the header's real
        // `b/<path>` rather than producing an empty path.
        let recover = "diff --git a/real.txt b/real.txt\n+++ b/\nbinary files differ\n";
        let files = parse_diff(recover);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, std::path::Path::new("real.txt"));
        // A mode-only change (no +++/---/rename, no hunks) still keeps its path via
        // the header fallback — the path-resolution change must not drop it.
        let mode_only = "diff --git a/f.sh b/f.sh\nold mode 100644\nnew mode 100755\n";
        let files = parse_diff(mode_only);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, std::path::Path::new("f.sh"));
    }

    #[test]
    fn diff_parses_hunk_ranges_and_body() {
        let full = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,2 +1,3 @@ fn main()\n ctx\n-old\n+new\n+added\n";
        let files = parse_diff(full);
        assert_eq!(files.len(), 1);
        // The verbatim section is preserved for display.
        assert_eq!(files[0].raw, full);
        let hunk = &files[0].hunks[0];
        assert_eq!(
            (
                hunk.old_start,
                hunk.old_lines,
                hunk.new_start,
                hunk.new_lines
            ),
            (1, 2, 1, 3)
        );
        assert_eq!(hunk.section, "fn main()");
        assert_eq!(
            hunk.lines,
            vec![
                DiffLine::Context("ctx".into()),
                DiffLine::Removed("old".into()),
                DiffLine::Added("new".into()),
                DiffLine::Added("added".into()),
            ]
        );
    }

    #[test]
    fn diff_omitted_count_defaults_to_one() {
        // `@@ -3 +3 @@` (no `,count`) means a single line on each side.
        let full = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -3 +3 @@\n-a\n+b\n";
        let hunk = &parse_diff(full)[0].hunks[0];
        assert_eq!((hunk.old_start, hunk.old_lines), (3, 1));
        assert_eq!((hunk.new_start, hunk.new_lines), (3, 1));
    }

    #[test]
    fn diff_stat_parses_all_clauses() {
        let got = DiffStat::parse(" 3 files changed, 12 insertions(+), 4 deletions(-)\n");
        assert_eq!(got, DiffStat::new(3, 12, 4));
    }

    #[test]
    fn diff_stat_tolerates_missing_clauses_and_empty() {
        // Pure-insertion diff omits deletions; no changes yields all zeros.
        let only_ins = DiffStat::parse(" 1 file changed, 2 insertions(+)\n");
        assert_eq!(only_ins.insertions, 2);
        assert_eq!(only_ins.deletions, 0);
        assert_eq!(DiffStat::parse(""), DiffStat::default());
    }
}

// Property-based fuzzing: `parse_diff` is a pure function over *arbitrary* CLI
// text (a git/jj on the user's machine we don't control), so the load-bearing
// invariant is "never panic, whatever the bytes" — the byte-offset slicing in
// `parse_section`/`header_b_path` must stay char-boundary-safe.
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// A line drawn from a git-format diff's structural vocabulary plus multibyte
    /// text, so a joined document reaches the byte-offset branches.
    fn diff_line() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("diff --git a/f b/f\n".to_string()),
            Just("--- a/f\n".to_string()),
            Just("+++ b/f\n".to_string()),
            Just("@@ -1,2 +3,4 @@ ctx\n".to_string()),
            Just("@@ -1 +1 @@\n".to_string()),
            Just("new file mode 100644\n".to_string()),
            Just("deleted file mode 100644\n".to_string()),
            Just("rename from {old => new}.rs\n".to_string()),
            Just("rename to é/r.rs\n".to_string()),
            "[-+ ]?[a-zé\t]{0,12}\n", // diff body / text incl. multibyte
        ]
    }

    fn diff_doc() -> impl Strategy<Value = String> {
        prop::collection::vec(diff_line(), 0..40).prop_map(|lines| lines.concat())
    }

    proptest! {
        // Panic-freedom on completely arbitrary input.
        #[test]
        fn parse_diff_never_panics_on_arbitrary_text(s in any::<String>()) {
            let _ = parse_diff(&s);
        }

        // …and on structure-biased input that reaches the parsing branches.
        #[test]
        fn parse_diff_never_panics_on_structured_text(s in diff_doc()) {
            let _ = parse_diff(&s);
        }

        // parse_diff never invents files it can't render the marker for: every
        // returned FileDiff carries a raw section starting with `diff --git`.
        #[test]
        fn parse_diff_sections_are_well_formed(s in diff_doc()) {
            for file in parse_diff(&s) {
                prop_assert!(file.raw.starts_with("diff --git"));
            }
        }
    }
}

// The optional `serde` feature derives `Serialize` on the public model.
#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use super::*;

    #[test]
    fn diff_stat_and_change_kind_serialize() {
        assert_eq!(
            serde_json::to_value(DiffStat::new(3, 12, 4)).unwrap(),
            serde_json::json!({"files_changed": 3, "insertions": 12, "deletions": 4})
        );
        // Field-less enum variants serialize as their name.
        assert_eq!(
            serde_json::to_value(ChangeKind::Renamed).unwrap(),
            serde_json::json!("Renamed")
        );
    }
}
