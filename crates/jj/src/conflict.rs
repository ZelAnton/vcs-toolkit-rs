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
    /// contributes its new text; base sections are not sides).
    pub fn sides(&self) -> Vec<Vec<String>> {
        self.sections
            .iter()
            .filter_map(|section| match section {
                JjConflictSection::Diff { lines, .. } => Some(apply_diff(lines, false)),
                JjConflictSection::Snapshot { lines, .. } => Some(lines.clone()),
                JjConflictSection::Base { .. } => None,
            })
            .collect()
    }

    /// The base content, when the region records one (a diff section's old
    /// text, or a snapshot-style `-------` section).
    pub fn base(&self) -> Option<Vec<String>> {
        self.sections.iter().find_map(|section| match section {
            JjConflictSection::Diff { lines, .. } => Some(apply_diff(lines, true)),
            JjConflictSection::Base { lines, .. } => Some(lines.clone()),
            JjConflictSection::Snapshot { .. } => None,
        })
    }
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
                if parse_counter(label.trim_end_matches(" ends").trim_end()).is_some()
                    || label.ends_with("ends")
                {
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
                if marker_run(to_line, '\\').is_none() {
                    return Err(parse_error(format!(
                        "diff section: expected a \\\\\\\\\\\\\\\\ `to:` line, got {:?}",
                        to_line.trim_end()
                    )));
                }
                let to_label = marker_label(to_line, marker_run(to_line, '\\').unwrap())
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
