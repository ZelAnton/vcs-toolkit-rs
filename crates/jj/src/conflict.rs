//! Typed model of jj's **materialized** conflict markers — parse a conflicted
//! file's content into structured regions and write a chosen resolution back.
//! Pure functions (no subprocess), so everything here is hermetic.
//!
//! Covers jj's native styles (`ui.conflict-marker-style`): **`diff`** (the
//! 0.38 default — one side rendered as a unified diff against the base) and
//! **`snapshot`** (every side and the base rendered verbatim). Files
//! materialized with the `git` style use git's grammar — parse those with
//! `vcs_git::conflict` instead (documented asymmetry, not an oversight).
//!
//! A region looks like:
//!
//! ```text
//! <<<<<<< conflict 1 of 1
//! %%%%%%% diff from: <change> <commit> "base"
//! \\\\\\\        to: <change> <commit> "side-a"
//! -line 2
//! +main line 2
//! +++++++ <change> <commit> "side-b"
//! feature line 2
//! >>>>>>> conflict 1 of 1 ends
//! ```
//!
//! Lines are kept verbatim (including `\r\n` and a missing trailing newline),
//! so [`render`] is a byte-exact roundtrip.

use processkit::{Error, Result};

use crate::BINARY;

/// One section inside a jj conflict region.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum JjConflictSection {
    /// A `%%%%%%%` section: one side expressed as a unified diff from the
    /// base (`-`/`+`/` `-prefixed lines). The side's content is the diff's
    /// *new* text; the base is its *old* text.
    Diff {
        /// The `diff from:` label (the base's ids/description).
        from_label: String,
        /// The `to:` label (this side's ids/description).
        to_label: String,
        /// Raw diff lines, verbatim.
        lines: Vec<String>,
    },
    /// A `+++++++` section: one side's content, verbatim.
    Snapshot {
        /// The side's ids/description.
        label: String,
        /// The side's lines, verbatim.
        lines: Vec<String>,
    },
    /// A `-------` section (snapshot style): the base's content, verbatim.
    Base {
        /// The base's ids/description.
        label: String,
        /// The base's lines, verbatim.
        lines: Vec<String>,
    },
}

/// One materialized jj conflict region (`<<<<<<< conflict N of M` …
/// `>>>>>>> conflict N of M ends`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct JjConflictRegion {
    /// This region's number within the file (the `N` of `conflict N of M`).
    pub number: u32,
    /// The file's total conflict count (the `M`).
    pub total: u32,
    /// The region's sections, in file order.
    pub sections: Vec<JjConflictSection>,
    // Verbatim marker lines for byte-exact rendering.
    marker_start: String,
    marker_end: String,
    section_markers: Vec<String>,
}

impl JjConflictRegion {
    /// The materialized content of each *side*, in file order (a diff section
    /// contributes its new text; base sections are not sides). Each side is a
    /// list of newline-terminated lines whose concatenation is the side's exact
    /// bytes — including a **missing** terminating newline, which jj encodes and
    /// this reconstruction honors.
    pub fn sides(&self) -> Vec<Vec<String>> {
        let mixed = self.mixed_eol();
        self.sections
            .iter()
            .filter_map(|section| match section {
                JjConflictSection::Diff {
                    to_label, lines, ..
                } => Some(join_sublines(
                    &apply_diff(lines, false),
                    labeled_no_eol(to_label),
                    mixed,
                )),
                JjConflictSection::Snapshot { label, lines } => {
                    Some(join_sublines(lines, labeled_no_eol(label), mixed))
                }
                JjConflictSection::Base { .. } => None,
            })
            .collect()
    }

    /// The base content, when the region records one (a diff section's old
    /// text, or a snapshot-style `-------` section). Honors a missing
    /// terminating newline the same way [`sides`](Self::sides) does.
    pub fn base(&self) -> Option<Vec<String>> {
        let mixed = self.mixed_eol();
        self.sections.iter().find_map(|section| match section {
            JjConflictSection::Diff {
                from_label, lines, ..
            } => Some(join_sublines(
                &apply_diff(lines, true),
                labeled_no_eol(from_label),
                mixed,
            )),
            JjConflictSection::Base { label, lines } => {
                Some(join_sublines(lines, labeled_no_eol(label), mixed))
            }
            JjConflictSection::Snapshot { .. } => None,
        })
    }

    /// Whether jj rendered this region in its explicit trailing-newline mode.
    ///
    /// When every side (and the base) ends in a newline, jj renders each side's
    /// lines verbatim. But as soon as **any** side lacks a terminating newline,
    /// jj switches representation for the *whole* region: the newline-less side
    /// gets a `(no terminating newline)` label marker and its content with no
    /// trailing line, while every newline-terminated side gets an extra trailing
    /// empty line (a bare `\n`, or a ` `/`+`/`-` context line inside a diff) to
    /// mark "this one *does* end in a newline". Detecting the mode lets
    /// [`join_sublines`] undo that extra line so the materialized bytes are exact
    /// (verified against jj 0.42 across all four newline permutations).
    fn mixed_eol(&self) -> bool {
        self.sections.iter().any(|section| match section {
            JjConflictSection::Snapshot { label, .. } | JjConflictSection::Base { label, .. } => {
                labeled_no_eol(label)
            }
            JjConflictSection::Diff {
                from_label,
                to_label,
                ..
            } => labeled_no_eol(from_label) || labeled_no_eol(to_label),
        })
    }
}

/// jj's marker for a side whose content does not end in a newline. It is
/// appended to the section's label, e.g. `… "side-a" (no terminating newline)`.
const NO_EOL_MARKER: &str = "(no terminating newline)";

/// Whether a section label carries jj's [`NO_EOL_MARKER`].
fn labeled_no_eol(label: &str) -> bool {
    label.trim_end().ends_with(NO_EOL_MARKER)
}

/// Re-join a section's rendered sub-lines (already prefix-stripped for a diff)
/// into the side/base's true bytes, undoing jj's trailing-newline encoding.
///
/// - `no_eol` — this role's label carries [`NO_EOL_MARKER`]: the last sub-line's
///   `\n` is jj's display artifact for a side that has no terminating newline;
///   drop it, so the content ends without a newline.
/// - `mixed` — the region is in the explicit trailing-newline mode
///   ([`JjConflictRegion::mixed_eol`]): a newline-terminated side carries one
///   extra trailing empty sub-line marking "has newline"; drop that one `\n`.
///
/// The two are mutually exclusive per section (a no-newline side is annotated,
/// not padded), so at most one correction applies. The result is re-split into
/// newline-terminated lines so callers and [`resolve`] reproduce the bytes by
/// plain concatenation.
fn join_sublines(sublines: &[String], no_eol: bool, mixed: bool) -> Vec<String> {
    let mut content: String = sublines.concat();
    if no_eol || mixed {
        // Remove the trailing line terminator jj added — its display newline
        // (no_eol) or the trailing-empty pad line (mixed). jj emits it with the
        // file's own ending, so strip a full `\r\n` on a CRLF file (else a bare
        // `\n` would leave a stray `\r` and silently corrupt the resolved bytes).
        if content.ends_with("\r\n") {
            content.truncate(content.len() - 2);
        } else if content.ends_with('\n') {
            content.pop();
        }
    }
    content.split_inclusive('\n').map(str::to_string).collect()
}

/// A conflicted file as a sequence of plain-text runs and conflict regions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JjConflictSegment {
    /// Lines outside any conflict (verbatim).
    Text(Vec<String>),
    /// One materialized conflict region (boxed — much larger than a text run).
    Conflict(Box<JjConflictRegion>),
}

/// What [`resolve`] keeps in place of each conflict region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JjResolution {
    /// The N-th side (0-based, file order) — `Side(0)` is the first side.
    Side(usize),
    /// The recorded base.
    Base,
}

/// Materialize a diff section: `old = true` keeps `-`/` ` lines (the base),
/// `old = false` keeps `+`/` ` lines (the side), stripping the prefix char
/// but preserving the line ending.
fn apply_diff(lines: &[String], old: bool) -> Vec<String> {
    let keep = if old { ['-', ' '] } else { ['+', ' '] };
    lines
        .iter()
        .filter_map(|line| {
            let mut chars = line.chars();
            let first = chars.next()?;
            keep.contains(&first).then(|| chars.as_str().to_string())
        })
        .collect()
}

/// The marker run length when `line` starts with a run of `ch` (≥ 7) followed
/// by a space or line end.
fn marker_run(line: &str, ch: char) -> Option<usize> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let n = trimmed.chars().take_while(|&c| c == ch).count();
    let rest = &trimmed[n..];
    (n >= 7 && (rest.is_empty() || rest.starts_with(' '))).then_some(n)
}

fn marker_label(line: &str, n: usize) -> String {
    line.trim_end_matches(['\r', '\n'])[n..]
        .trim_start()
        .to_string()
}

fn parse_error(message: String) -> Error {
    Error::Parse {
        program: BINARY.to_string(),
        message,
    }
}

/// Parse `conflict N of M` / `conflict N of M ends` headers.
fn parse_counter(label: &str) -> Option<(u32, u32)> {
    let rest = label.strip_prefix("conflict ")?;
    let mut parts = rest.split_whitespace();
    let n = parts.next()?.parse().ok()?;
    let of = parts.next()?;
    let m = parts.next()?.parse().ok()?;
    (of == "of").then_some((n, m))
}

/// Does `content` contain a jj conflict-start marker? Cheap pre-check.
pub fn has_conflict_markers(content: &str) -> bool {
    content.split_inclusive('\n').any(|line| {
        marker_run(line, '<').is_some_and(|n| parse_counter(&marker_label(line, n)).is_some())
    })
}

/// Parse a jj-materialized conflicted file (native `diff`/`snapshot` styles)
/// into text/conflict segments. Errors with [`Error::Parse`] on malformed
/// input (unterminated region, content before the first section marker, a
/// `git`-style file — use `vcs_git::conflict` for those).
pub fn parse_conflicts(content: &str) -> Result<Vec<JjConflictSegment>> {
    let mut segments = Vec::new();
    let mut text: Vec<String> = Vec::new();
    let mut lines = content.split_inclusive('\n');

    while let Some(line) = lines.next() {
        let counter = marker_run(line, '<')
            .map(|n| (n, marker_label(line, n)))
            .and_then(|(n, label)| parse_counter(&label).map(|c| (n, c)));
        let Some((n, (number, total))) = counter else {
            if marker_run(line, '<').is_some() {
                return Err(parse_error(format!(
                    "git-style conflict marker {:?} — parse this file with \
                     vcs_git::conflict (jj's `git` marker style uses git's grammar)",
                    line.trim_end()
                )));
            }
            text.push(line.to_string());
            continue;
        };
        if !text.is_empty() {
            segments.push(JjConflictSegment::Text(std::mem::take(&mut text)));
        }

        let marker_start = line.to_string();
        let mut sections: Vec<JjConflictSection> = Vec::new();
        let mut section_markers: Vec<String> = Vec::new();
        let marker_end = loop {
            let Some(line) = lines.next() else {
                return Err(parse_error(format!(
                    "unterminated jj conflict {number} of {total}"
                )));
            };
            // Section/end markers must match the region's opening run length —
            // jj lengthens ALL of a file's markers together when the content
            // itself contains marker-like runs, so a shorter run is content.
            if marker_run(line, '>') == Some(n) {
                let label = marker_label(line, n);
                // jj's end marker is `conflict N of M ends`; `parse_counter` matches
                // it after trimming the trailing ` ends` (and also a hypothetical
                // `conflict N of M` with no ` ends`). We rely *solely* on that
                // structural check — no loose `ends_with("ends")` fallback, which
                // could wrongly terminate the region on content that happens to be a
                // run of exactly `n` `>` followed by a word ending in "ends".
                if parse_counter(label.trim_end_matches(" ends").trim_end()).is_some() {
                    break line.to_string();
                }
            }
            if let Some(m) = marker_run(line, '%').filter(|&m| m == n) {
                // `%%%%%%% diff from: …` then a `\\\\\\\        to: …` line.
                let from_label = marker_label(line, m)
                    .trim_start_matches("diff from:")
                    .trim()
                    .to_string();
                let Some(to_line) = lines.next() else {
                    return Err(parse_error("diff section missing its `to:` line".into()));
                };
                // The `\\\` run must match the `%%%` run length `m` (== the
                // region's `n`) — jj lengthens every marker in a file together,
                // so a differing run is malformed input, not a valid `to:` line.
                if marker_run(to_line, '\\') != Some(m) {
                    return Err(parse_error(format!(
                        "diff section: expected a {m}-long `\\` `to:` line, got {:?}",
                        to_line.trim_end()
                    )));
                }
                let to_label = marker_label(to_line, m)
                    .trim_start_matches("to:")
                    .trim()
                    .to_string();
                section_markers.push(format!("{line}{to_line}"));
                sections.push(JjConflictSection::Diff {
                    from_label,
                    to_label,
                    lines: Vec::new(),
                });
                continue;
            }
            if let Some(m) = marker_run(line, '+').filter(|&m| m == n) {
                section_markers.push(line.to_string());
                sections.push(JjConflictSection::Snapshot {
                    label: marker_label(line, m),
                    lines: Vec::new(),
                });
                continue;
            }
            if let Some(m) = marker_run(line, '-').filter(|&m| m == n) {
                section_markers.push(line.to_string());
                sections.push(JjConflictSection::Base {
                    label: marker_label(line, m),
                    lines: Vec::new(),
                });
                continue;
            }
            // Content line for the current section.
            match sections.last_mut() {
                Some(
                    JjConflictSection::Diff { lines, .. }
                    | JjConflictSection::Snapshot { lines, .. }
                    | JjConflictSection::Base { lines, .. },
                ) => lines.push(line.to_string()),
                None => {
                    return Err(parse_error(format!(
                        "content before the first section marker in conflict \
                         {number}: {:?}",
                        line.trim_end()
                    )));
                }
            }
        };

        segments.push(JjConflictSegment::Conflict(Box::new(JjConflictRegion {
            number,
            total,
            sections,
            marker_start,
            marker_end,
            section_markers,
        })));
    }
    if !text.is_empty() {
        segments.push(JjConflictSegment::Text(text));
    }
    Ok(segments)
}

/// Re-render segments verbatim — the byte-exact inverse of
/// [`parse_conflicts`].
pub fn render(segments: &[JjConflictSegment]) -> String {
    let mut out = String::new();
    for segment in segments {
        match segment {
            JjConflictSegment::Text(lines) => lines.iter().for_each(|l| out.push_str(l)),
            JjConflictSegment::Conflict(region) => {
                out.push_str(&region.marker_start);
                for (section, marker) in region.sections.iter().zip(&region.section_markers) {
                    out.push_str(marker);
                    let (JjConflictSection::Diff { lines, .. }
                    | JjConflictSection::Snapshot { lines, .. }
                    | JjConflictSection::Base { lines, .. }) = section;
                    lines.iter().for_each(|l| out.push_str(l));
                }
                out.push_str(&region.marker_end);
            }
        }
    }
    out
}

/// Produce the file content with every conflict resolved to `resolution`.
///
/// Errors with a clear message when a region has no such side/base.
pub fn resolve(segments: &[JjConflictSegment], resolution: JjResolution) -> Result<String> {
    let refuse = |what: String| Error::Spawn {
        program: BINARY.to_string(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, what),
    };
    let mut out = String::new();
    for segment in segments {
        match segment {
            JjConflictSegment::Text(lines) => lines.iter().for_each(|l| out.push_str(l)),
            JjConflictSegment::Conflict(region) => {
                let chosen = match resolution {
                    JjResolution::Side(i) => {
                        let sides = region.sides();
                        sides.get(i).cloned().ok_or_else(|| {
                            refuse(format!(
                                "conflict {} has {} side(s); Side({i}) does not exist",
                                region.number,
                                sides.len()
                            ))
                        })?
                    }
                    JjResolution::Base => region.base().ok_or_else(|| {
                        refuse(format!("conflict {} records no base", region.number))
                    })?,
                };
                chosen.iter().for_each(|l| out.push_str(l));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured verbatim from jj 0.38 (default `diff` style).
    const DIFF_STYLE: &str = "line 1\n<<<<<<< conflict 1 of 1\n%%%%%%% diff from: rnxsupvw 638ae425 \"base\"\n\\\\\\\\\\\\\\        to: ozvltnxm 92f2b14f \"side-a\"\n-line 2\n+main line 2\n+++++++ xyrusolp ad268d1f \"side-b\"\nfeature line 2\n>>>>>>> conflict 1 of 1 ends\nline 3\n";

    // Captured verbatim from jj 0.38 (`snapshot` style).
    const SNAPSHOT_STYLE: &str = "line 1\n<<<<<<< conflict 1 of 1\n+++++++ kttusupp 7eedad44 \"side-a\"\nmain line 2\n------- rzkutuko 4fe1246f \"base\"\nline 2\n+++++++ ukuqwwlw 38f5069b \"side-b\"\nfeature line 2\n>>>>>>> conflict 1 of 1 ends\nline 3\n";

    #[test]
    fn parses_diff_style_and_materializes_sides() {
        let segments = parse_conflicts(DIFF_STYLE).expect("parse");
        assert_eq!(segments.len(), 3);
        let JjConflictSegment::Conflict(region) = &segments[1] else {
            panic!("expected a conflict, got {segments:?}");
        };
        assert_eq!((region.number, region.total), (1, 1));
        assert_eq!(region.sections.len(), 2);
        let sides = region.sides();
        assert_eq!(sides.len(), 2);
        assert_eq!(sides[0], ["main line 2\n"], "diff side = applied new text");
        assert_eq!(sides[1], ["feature line 2\n"], "snapshot side verbatim");
        assert_eq!(region.base().unwrap(), ["line 2\n"], "diff old text = base");
    }

    #[test]
    fn parses_snapshot_style() {
        let segments = parse_conflicts(SNAPSHOT_STYLE).expect("parse");
        let JjConflictSegment::Conflict(region) = &segments[1] else {
            panic!("expected a conflict");
        };
        assert_eq!(region.sections.len(), 3);
        let sides = region.sides();
        assert_eq!(sides[0], ["main line 2\n"]);
        assert_eq!(sides[1], ["feature line 2\n"]);
        assert_eq!(region.base().unwrap(), ["line 2\n"]);
        assert!(
            matches!(&region.sections[1], JjConflictSection::Base { label, .. }
                if label.contains("\"base\"")),
        );
    }

    // A content line that is a run of exactly `n` `>` followed by a word ending in
    // "ends" must NOT be mistaken for the region terminator — only `conflict N of M
    // ends` ends the region. (The removed loose `ends_with("ends")` fallback would
    // have wrongly terminated here.) This is malformed input jj wouldn't emit (it
    // lengthens markers past content runs), but the parser must not be fooled by it.
    #[test]
    fn content_run_ending_in_ends_is_not_the_terminator() {
        let input = concat!(
            "line 1\n",
            "<<<<<<< conflict 1 of 1\n",
            "+++++++ side-a\n",
            ">>>>>>> recommends\n", // 7 `>`, ends in "ends" — content, NOT the end marker
            "------- base\n",
            "line 2\n",
            "+++++++ side-b\n",
            "feature line 2\n",
            ">>>>>>> conflict 1 of 1 ends\n", // the real terminator
            "line 3\n",
        );
        let segments = parse_conflicts(input).expect("parse");
        // Three segments — the region did not end early at the `recommends` line.
        assert_eq!(segments.len(), 3);
        let JjConflictSegment::Conflict(region) = &segments[1] else {
            panic!("expected a conflict, got {segments:?}");
        };
        assert_eq!((region.number, region.total), (1, 1));
        assert!(
            region.sides()[0].iter().any(|l| l.contains("recommends")),
            "the `>>>…recommends` content line is part of side-a, not the terminator"
        );
        // And it round-trips byte-for-byte (the content line is stored verbatim).
        assert_eq!(render(&segments), input);
    }

    // The `\\\ to:` run must match the region's marker length. A `%%%%%%%` (7)
    // diff header followed by an 8-long `\` run is malformed (jj lengthens all
    // markers together) and must be rejected, not silently accepted.
    #[test]
    fn diff_section_rejects_mismatched_to_marker_length() {
        let input = concat!(
            "<<<<<<< conflict 1 of 1\n",
            "%%%%%%% diff from: ab cd \"base\"\n",
            "\\\\\\\\\\\\\\\\        to: ef gh \"side\"\n", // 8 backslashes ≠ 7
            "-line\n",
            "+new\n",
            ">>>>>>> conflict 1 of 1 ends\n",
        );
        let err = parse_conflicts(input).unwrap_err();
        assert!(matches!(err, Error::Parse { .. }), "structured parse error");
        assert!(
            err.to_string().contains("to:"),
            "error should point at the malformed `to:` line: {err}"
        );
    }

    #[test]
    fn render_roundtrips_exactly() {
        for sample in [DIFF_STYLE, SNAPSHOT_STYLE] {
            let segments = parse_conflicts(sample).expect("parse");
            assert_eq!(render(&segments), sample, "roundtrip");
        }
        // Conflict at EOF without trailing newline still roundtrips.
        let eof = DIFF_STYLE.trim_end_matches("line 3\n");
        let eof = &eof[..eof.len() - 1]; // drop the final newline of the end marker
        let segments = parse_conflicts(eof).expect("parse");
        assert_eq!(render(&segments), eof);
    }

    #[test]
    fn resolve_picks_sides_and_base() {
        let segments = parse_conflicts(DIFF_STYLE).expect("parse");
        assert_eq!(
            resolve(&segments, JjResolution::Side(0)).unwrap(),
            "line 1\nmain line 2\nline 3\n"
        );
        assert_eq!(
            resolve(&segments, JjResolution::Side(1)).unwrap(),
            "line 1\nfeature line 2\nline 3\n"
        );
        assert_eq!(
            resolve(&segments, JjResolution::Base).unwrap(),
            "line 1\nline 2\nline 3\n"
        );
        assert!(resolve(&segments, JjResolution::Side(2)).is_err());
    }

    // jj switches to an explicit trailing-newline representation the moment ANY
    // side lacks a terminating newline (structures captured verbatim from jj
    // 0.42 across all four permutations). `resolve` must reproduce each side's
    // and the base's exact bytes — including a *missing* final newline — and
    // `render` must still round-trip. Regression for the silent corruption where
    // the phantom trailing/context line became a spurious extra blank line.
    fn check_eol(input: &str, side0: &str, side1: &str, base: &str) {
        let segments = parse_conflicts(input).expect("parse");
        assert_eq!(
            render(&segments),
            input,
            "render must round-trip byte-exact"
        );
        assert_eq!(
            resolve(&segments, JjResolution::Side(0)).unwrap(),
            side0,
            "side 0"
        );
        assert_eq!(
            resolve(&segments, JjResolution::Side(1)).unwrap(),
            side1,
            "side 1"
        );
        assert_eq!(
            resolve(&segments, JjResolution::Base).unwrap(),
            base,
            "base"
        );
    }

    #[test]
    fn resolve_honors_missing_terminating_newline() {
        // side-a (snapshot) lacks the terminating newline; side-b via diff.
        check_eol(
            concat!(
                "line 1\n",
                "<<<<<<< conflict 1 of 1\n",
                "+++++++ aaa 111 \"side-a\" (no terminating newline)\n",
                "main 2\n",
                "%%%%%%% diff from: bbb 222 \"base\"\n",
                "\\\\\\\\\\\\\\        to: ccc 333 \"side-b\"\n",
                "-line 2\n",
                "+feat 2\n",
                " \n",
                ">>>>>>> conflict 1 of 1 ends",
            ),
            "line 1\nmain 2", // side-a: no trailing newline (was corrupted to `main 2\n`)
            "line 1\nfeat 2\n", // side-b: keeps its newline, no phantom blank line
            "line 1\nline 2\n", // base
        );
        // side-b (via diff) lacks the terminating newline; side-a via diff.
        check_eol(
            concat!(
                "line 1\n",
                "<<<<<<< conflict 1 of 1\n",
                "%%%%%%% diff from: bbb 222 \"base\"\n",
                "\\\\\\\\\\\\\\        to: aaa 111 \"side-a\"\n",
                "-line 2\n",
                "+main 2\n",
                " \n",
                "+++++++ ccc 333 \"side-b\" (no terminating newline)\n",
                "feat 2\n",
                ">>>>>>> conflict 1 of 1 ends",
            ),
            "line 1\nmain 2\n",
            "line 1\nfeat 2",
            "line 1\nline 2\n",
        );
        // base lacks the terminating newline (both sides keep theirs).
        check_eol(
            concat!(
                "line 1\n",
                "<<<<<<< conflict 1 of 1\n",
                "%%%%%%% diff from: bbb 222 \"base\" (no terminating newline)\n",
                "\\\\\\\\\\\\\\        to: aaa 111 \"side-a\"\n",
                "-line 2\n",
                "+main 2\n",
                "+\n",
                "+++++++ ccc 333 \"side-b\"\n",
                "feat 2\n",
                "\n",
                ">>>>>>> conflict 1 of 1 ends",
            ),
            "line 1\nmain 2\n",
            "line 1\nfeat 2\n",
            "line 1\nline 2", // base: no trailing newline
        );
        // both sides lack a terminating newline (base keeps one).
        check_eol(
            concat!(
                "line 1\n",
                "<<<<<<< conflict 1 of 1\n",
                "%%%%%%% diff from: bbb 222 \"base\"\n",
                "\\\\\\\\\\\\\\        to: aaa 111 \"side-a\" (no terminating newline)\n",
                "-line 2\n",
                "-\n",
                "+main 2\n",
                "+++++++ ccc 333 \"side-b\" (no terminating newline)\n",
                "feat 2\n",
                ">>>>>>> conflict 1 of 1 ends",
            ),
            "line 1\nmain 2",
            "line 1\nfeat 2",
            "line 1\nline 2\n",
        );
    }

    // CRLF variant (captured byte-for-byte from jj 0.42): every marker and pad
    // line ends in `\r\n`. side-a lacks the terminating newline. The materializer
    // must strip a full `\r\n` terminator — a bare-`\n` pop would leave a stray
    // `\r` and corrupt every resolution on Windows files.
    #[test]
    fn resolve_honors_missing_newline_with_crlf() {
        check_eol(
            concat!(
                "line 1\r\n",
                "<<<<<<< conflict 1 of 1\r\n",
                "+++++++ aaa 111 \"side-a\" (no terminating newline)\r\n",
                "main 2\r\n",
                "%%%%%%% diff from: bbb 222 \"base\"\r\n",
                "\\\\\\\\\\\\\\        to: ccc 333 \"side-b\"\r\n",
                "-line 2\r\n",
                "+feat 2\r\n",
                " \r\n",
                ">>>>>>> conflict 1 of 1 ends",
            ),
            "line 1\r\nmain 2",     // side-a: no trailing newline, no stray \r
            "line 1\r\nfeat 2\r\n", // side-b: full CRLF terminator preserved
            "line 1\r\nline 2\r\n", // base
        );
    }

    #[test]
    fn multi_region_counters_parse() {
        let two = format!(
            "{}middle\n{}",
            DIFF_STYLE,
            DIFF_STYLE
                .replace("conflict 1 of 1", "conflict 2 of 2")
                .replace("line 1\n", "")
                .replace("line 3\n", "")
        );
        let segments = parse_conflicts(&two).expect("parse");
        let counters: Vec<(u32, u32)> = segments
            .iter()
            .filter_map(|s| match s {
                JjConflictSegment::Conflict(r) => Some((r.number, r.total)),
                _ => None,
            })
            .collect();
        assert_eq!(counters, [(1, 1), (2, 2)]);
    }

    #[test]
    fn git_style_and_malformed_are_rejected() {
        let git_style = "<<<<<<< abc 123 \"side-a\"\nx\n||||||| base\ny\n=======\nz\n>>>>>>> def\n";
        let err = parse_conflicts(git_style).unwrap_err();
        assert!(matches!(err, Error::Parse { .. }), "structured parse error");
        // The message must actively steer the caller to the right parser, not just
        // fail — that redirect is the point of rejecting a git-style file here.
        assert!(
            err.to_string().contains("vcs_git::conflict"),
            "git-style error should redirect to vcs_git::conflict: {err}"
        );
        assert!(parse_conflicts("<<<<<<< conflict 1 of 1\nstray content\n").is_err());
        assert!(has_conflict_markers(DIFF_STYLE));
        assert!(!has_conflict_markers(git_style), "git markers aren't jj's");
    }
}

// Property-based fuzzing of the jj conflict grammar: marker-run slicing, the
// `conflict N of M` counters, the `%%%`/`\\\` diff-section continuation, and
// `apply_diff` must never panic on hostile input, and `render(parse(x)?) == x`
// must hold byte-for-byte.
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// Lines from jj's materialized-conflict vocabulary (diff + snapshot
    /// styles) with variable counters/marker lengths and multibyte content.
    fn conflict_line() -> impl Strategy<Value = String> {
        prop_oneof![
            (1u32..3, 1u32..3).prop_map(|(n, m)| format!("<<<<<<< conflict {n} of {m}\n")),
            (1u32..3, 1u32..3).prop_map(|(n, m)| format!(">>>>>>> conflict {n} of {m} ends\n")),
            Just("%%%%%%% diff from: ab cd \"basé\"\n".to_string()),
            Just("\\\\\\\\\\\\\\        to: ef gh \"side\"\n".to_string()),
            Just("+++++++ ij kl \"side-b\"\n".to_string()),
            Just("------- mn op \"base\"\n".to_string()),
            "[-+ ]?[a-zé]{0,10}\n", // diff/content line incl. multibyte
        ]
    }

    fn conflict_doc() -> impl Strategy<Value = String> {
        prop::collection::vec(conflict_line(), 0..30).prop_map(|lines| lines.concat())
    }

    proptest! {
        #[test]
        fn parse_never_panics_on_arbitrary_text(s in any::<String>()) {
            let _ = has_conflict_markers(&s);
            if let Ok(segments) = parse_conflicts(&s) {
                // Whatever arbitrary text parses must round-trip byte-exact — the
                // load-bearing invariant, asserted on this generator too.
                prop_assert_eq!(render(&segments), s.clone());
                // Exercise the materializers on whatever parsed.
                for seg in &segments {
                    if let JjConflictSegment::Conflict(r) = seg {
                        let _ = r.sides();
                        let _ = r.base();
                    }
                }
            }
        }

        #[test]
        fn parse_never_panics_on_structured_text(s in conflict_doc()) {
            let _ = parse_conflicts(&s);
        }

        #[test]
        fn render_roundtrips_whatever_parses(s in conflict_doc()) {
            if let Ok(segments) = parse_conflicts(&s) {
                prop_assert_eq!(render(&segments), s);
            }
        }
    }
}
