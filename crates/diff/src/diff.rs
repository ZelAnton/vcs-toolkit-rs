//! The unified-diff model and parser, shared by `vcs-git` and `vcs-jj`.
//!
//! `git diff` and `jj diff --git` emit byte-identical git-format unified diffs,
//! so a single parser serves both. Pure functions over arbitrary text — no
//! process execution.

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
    pub path: String,
    /// For a rename, the original path (forward-slash normalised); `None` otherwise.
    pub old_path: Option<String>,
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
    let mut new_path = None;
    let mut minus_path = None;
    let mut rename_to = None;
    let mut rename_from = None;
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
            rename_to = Some(p.trim_end().to_string());
        } else if let Some(p) = line.strip_prefix("rename from ") {
            rename_from = Some(p.trim_end().to_string());
        } else if let Some(p) = line.strip_prefix("+++ b/") {
            new_path = Some(p.trim_end().to_string());
        } else if let Some(p) = line.strip_prefix("--- a/") {
            minus_path = Some(p.trim_end().to_string());
        }
    }
    if let Some(done) = current.take() {
        hunks.push(done);
    }

    let normalize = |p: String| p.replace('\\', "/");
    // A rename keeps its old path so a caller can record the deletion too.
    let old_path = if rename_to.is_some() {
        kind = ChangeKind::Renamed;
        rename_from.map(normalize)
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
        path: normalize(path),
        old_path,
        hunks,
        raw: section.to_string(),
    })
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
/// (e.g. binary files): the `b/<new>` of the `diff --git` header. Ambiguous only
/// when a path contains the literal `" b/"`, which binary-with-spaces makes rare.
fn header_b_path(section: &str) -> Option<String> {
    let first = section.lines().next()?;
    let s = first.strip_prefix("diff --git ")?;
    let idx = s.find(" b/")?;
    let path = s[idx + 1..].strip_prefix("b/").unwrap_or("");
    // A `diff --git a/x b/` with no path after `b/` yields nothing, not an empty
    // path — so a malformed header drops the section instead of an empty FileDiff.
    (!path.is_empty()).then(|| path.to_string())
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
        let kinds: Vec<_> = files.iter().map(|f| (f.path.as_str(), f.change)).collect();
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
        assert_eq!(rename.old_path.as_deref(), Some("old/f.txt"));
    }

    #[test]
    fn diff_handles_space_paths() {
        // git appends a trailing tab to `+++`/`---` paths containing spaces; the
        // path must survive intact (the `diff --git` header is ambiguous here).
        let full = "diff --git a/a b/c.txt b/a b/c.txt\n--- a/a b/c.txt\t\n+++ b/a b/c.txt\t\n@@ -1 +1 @@\n-x\n+y\n";
        let files = parse_diff(full);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "a b/c.txt");
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
        assert_eq!(files[0].path, "real.txt");
        // A mode-only change (no +++/---/rename, no hunks) still keeps its path via
        // the header fallback — the path-resolution change must not drop it.
        let mode_only = "diff --git a/f.sh b/f.sh\nold mode 100644\nnew mode 100755\n";
        let files = parse_diff(mode_only);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "f.sh");
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
