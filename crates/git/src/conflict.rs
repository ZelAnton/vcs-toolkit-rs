//! Typed model of git conflict markers — parse a conflicted file's *content*
//! into structured regions and write a chosen resolution back. Pure functions
//! (no subprocess), so everything here is hermetic.
//!
//! Handles git's three `merge.conflictStyle`s with one grammar: `merge`
//! (2-way: ours/theirs), `diff3` (3-way: ours/base/theirs), and `zdiff3`
//! (same markers as diff3 — the common affixes are already outside the
//! region). Marker length is variable (`merge.conflictMarkerSize`, default 7)
//! and is detected per region. Lines are kept verbatim (including `\r\n` and
//! a missing trailing newline), so [`render`] is a byte-exact roundtrip.
//!
//! jj note: files materialized with jj's `ui.conflict-marker-style = "git"`
//! use this exact grammar (with jj's own labels) and parse here; jj's native
//! `diff`/`snapshot` styles live in `vcs_jj::conflict`.

use processkit::{Error, Result};

use crate::BINARY;

/// Which side of a conflict a resolution keeps.
///
/// Intentionally **exhaustive** (no `#[non_exhaustive]`): a git conflict has
/// exactly these three sides — the domain is closed, so `#[non_exhaustive]` would
/// buy no future variant while forcing a wildcard arm on any caller that matches
/// this (callers usually *construct* it to pass to [`resolve`]) and wrongly
/// signalling a fourth side could appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionSide {
    /// The `<<<<<<<` side (typically `HEAD`).
    Ours,
    /// The `|||||||` base (diff3/zdiff3 only).
    Base,
    /// The `>>>>>>>` side (the merged-in branch).
    Theirs,
}

/// One conflicted region: the lines of each side plus the verbatim marker
/// lines (kept so rendering is byte-exact).
///
/// All line vectors store lines **with** their original endings; the last
/// line of a file may have none.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConflictRegion {
    /// Label after the `<<<<<<<` marker (e.g. `HEAD`); empty when absent.
    pub ours_label: String,
    /// Label after the `|||||||` marker; `None` for 2-way conflicts.
    pub base_label: Option<String>,
    /// Label after the `>>>>>>>` marker (e.g. the branch name).
    pub theirs_label: String,
    /// The `<<<<<<<`-side lines.
    pub ours: Vec<String>,
    /// The base lines (`diff3`/`zdiff3`); `None` for 2-way conflicts.
    pub base: Option<Vec<String>>,
    /// The `>>>>>>>`-side lines.
    pub theirs: Vec<String>,
    /// The marker run length (7 unless `merge.conflictMarkerSize` raised it).
    pub marker_len: usize,
    // Verbatim marker lines, for byte-exact rendering.
    marker_ours: String,
    marker_base: Option<String>,
    marker_sep: String,
    marker_end: String,
}

/// A conflicted file as a sequence of plain-text runs and conflict regions —
/// the shape that keeps [`render`] a byte-exact roundtrip.
///
/// Intentionally **exhaustive**: a file is text-or-conflict, and consumers match
/// every segment in the resolve/render loop this crate exists to serve, so the
/// closed enum stays ergonomic. Field-level evolution rides [`ConflictRegion`],
/// which *is* `#[non_exhaustive]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictSegment {
    /// Lines outside any conflict (verbatim, endings included).
    Text(Vec<String>),
    /// One conflicted region (boxed — much larger than a text run).
    Conflict(Box<ConflictRegion>),
}

/// Does `content` contain a line that looks like a conflict-start marker?
/// Cheap pre-check before a full [`parse_conflicts`].
pub fn has_conflict_markers(content: &str) -> bool {
    content
        .split_inclusive('\n')
        .any(|line| marker_run(line, '<').is_some_and(|n| n >= 7))
}

/// The length of the leading `ch` run when `line` is a marker line for it:
/// the run must be followed by a space + label, or end the line.
fn marker_run(line: &str, ch: char) -> Option<usize> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    let n = trimmed.chars().take_while(|&c| c == ch).count();
    if n == 0 {
        return None;
    }
    let rest = &trimmed[n..];
    (rest.is_empty() || rest.starts_with(' ')).then_some(n)
}

/// The label after an `n`-char marker run (empty when none).
fn marker_label(line: &str, n: usize) -> String {
    line.trim_end_matches(['\r', '\n'])[n..]
        .trim_start()
        .to_string()
}

fn parse_error(message: String) -> Error {
    Error::parse(BINARY, message)
}

/// Parse a conflicted file's content into text/conflict segments.
///
/// Errors with [`Error::Parse`] only on a genuinely malformed *region*: a
/// `<<<<<<<`-opened region missing its `=======` separator or `>>>>>>>`
/// terminator. A `=======`/`>>>>>>>` run **outside** any region is treated as
/// ordinary content (a Markdown/RST underline, a divider, a quoted email), so a
/// file with no real conflict — or a real conflict alongside marker-like content
/// — parses cleanly.
pub fn parse_conflicts(content: &str) -> Result<Vec<ConflictSegment>> {
    let mut segments = Vec::new();
    let mut text: Vec<String> = Vec::new();
    let mut lines = content.split_inclusive('\n').peekable();

    while let Some(line) = lines.next() {
        // A region starts at a `<<<<<<<`-run of length ≥ 7. A `=======` / `>>>>>>>`
        // run *outside* a region is ordinary content — a Markdown/RST setext
        // underline (`=========`), a `=======` divider banner, a deep `>>>>>>>`
        // email quote — NOT a malformed conflict, so it is kept verbatim as text
        // (a real conflict is delimited by a `<<<<<<<` opener; the region loops
        // below consume the `=`/`>` markers that belong to it). A genuinely broken
        // region (an opener with no separator/terminator) is still caught inside
        // those loops.
        let Some(n) = marker_run(line, '<').filter(|&n| n >= 7) else {
            text.push(line.to_string());
            continue;
        };
        if !text.is_empty() {
            segments.push(ConflictSegment::Text(std::mem::take(&mut text)));
        }

        let marker_ours = line.to_string();
        let ours_label = marker_label(line, n);
        let mut ours = Vec::new();
        let mut base: Option<Vec<String>> = None;
        let mut marker_base = None;
        let mut base_label = None;

        // Ours, until the base marker (diff3) or the separator.
        let marker_sep = loop {
            let Some(line) = lines.next() else {
                return Err(parse_error(format!(
                    "unterminated conflict (no ======= after {:?})",
                    marker_ours.trim_end()
                )));
            };
            // Only the FIRST `|`-run is the diff3 base marker; a later matching
            // line is base *content* (a region has exactly one base marker — a
            // repeated one used to overwrite it and lose a line on render).
            if base.is_none() && marker_run(line, '|') == Some(n) {
                base_label = Some(marker_label(line, n));
                marker_base = Some(line.to_string());
                base = Some(Vec::new());
                continue;
            }
            if marker_run(line, '=') == Some(n) {
                break line.to_string();
            }
            match &mut base {
                Some(base_lines) => base_lines.push(line.to_string()),
                None => ours.push(line.to_string()),
            }
        };

        // Theirs, until the end marker.
        let mut theirs = Vec::new();
        let marker_end = loop {
            let Some(line) = lines.next() else {
                return Err(parse_error(format!(
                    "unterminated conflict (no >>>>>>> after {:?})",
                    marker_ours.trim_end()
                )));
            };
            if marker_run(line, '>') == Some(n) {
                break line.to_string();
            }
            theirs.push(line.to_string());
        };
        let theirs_label = marker_label(&marker_end, n);

        segments.push(ConflictSegment::Conflict(Box::new(ConflictRegion {
            ours_label,
            base_label,
            theirs_label,
            ours,
            base,
            theirs,
            marker_len: n,
            marker_ours,
            marker_base,
            marker_sep,
            marker_end,
        })));
    }
    if !text.is_empty() {
        segments.push(ConflictSegment::Text(text));
    }
    Ok(segments)
}

/// Re-render segments verbatim — the byte-exact inverse of
/// [`parse_conflicts`].
pub fn render(segments: &[ConflictSegment]) -> String {
    let mut out = String::new();
    for segment in segments {
        match segment {
            ConflictSegment::Text(lines) => lines.iter().for_each(|l| out.push_str(l)),
            ConflictSegment::Conflict(region) => {
                out.push_str(&region.marker_ours);
                region.ours.iter().for_each(|l| out.push_str(l));
                if let Some(marker) = &region.marker_base {
                    out.push_str(marker);
                    if let Some(base) = &region.base {
                        base.iter().for_each(|l| out.push_str(l));
                    }
                }
                out.push_str(&region.marker_sep);
                region.theirs.iter().for_each(|l| out.push_str(l));
                out.push_str(&region.marker_end);
            }
        }
    }
    out
}

/// Produce the file content with every conflict resolved to `side`.
///
/// Errors with a clear message when `side` is [`ResolutionSide::Base`] and a
/// region has no base (2-way `merge` style records none).
pub fn resolve(segments: &[ConflictSegment], side: ResolutionSide) -> Result<String> {
    let mut out = String::new();
    for segment in segments {
        match segment {
            ConflictSegment::Text(lines) => lines.iter().for_each(|l| out.push_str(l)),
            ConflictSegment::Conflict(region) => {
                let chosen = match side {
                    ResolutionSide::Ours => &region.ours,
                    ResolutionSide::Theirs => &region.theirs,
                    ResolutionSide::Base => region.base.as_ref().ok_or_else(|| {
                        Error::spawn(
                            BINARY,
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                "cannot resolve to Base: this conflict records no base \
                                 (2-way `merge` style; use diff3/zdiff3)",
                            ),
                        )
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

    const MERGE_2WAY: &str =
        "line 1\n<<<<<<< HEAD\nmain line 2\n=======\nfeature line 2\n>>>>>>> feature\nline 3\n";
    const DIFF3: &str = "line 1\n<<<<<<< HEAD\nmain line 2\n||||||| 0b025ce\nline 2\n=======\nfeature line 2\n>>>>>>> feature\nline 3\n";

    #[test]
    fn parses_two_way_merge_style() {
        let segments = parse_conflicts(MERGE_2WAY).expect("parse");
        assert_eq!(segments.len(), 3);
        let ConflictSegment::Conflict(region) = &segments[1] else {
            panic!("expected a conflict, got {segments:?}");
        };
        assert_eq!(region.ours_label, "HEAD");
        assert_eq!(region.theirs_label, "feature");
        assert_eq!(region.ours, ["main line 2\n"]);
        assert_eq!(region.theirs, ["feature line 2\n"]);
        assert!(region.base.is_none());
        assert_eq!(region.marker_len, 7);
    }

    #[test]
    fn parses_diff3_with_base() {
        let segments = parse_conflicts(DIFF3).expect("parse");
        let ConflictSegment::Conflict(region) = &segments[1] else {
            panic!("expected a conflict");
        };
        assert_eq!(region.base_label.as_deref(), Some("0b025ce"));
        assert_eq!(region.base.as_deref(), Some(&["line 2\n".to_string()][..]));
    }

    // Proptest-found regression (seed committed in proptest-regressions/): a
    // SECOND `|`-run line inside a diff3 region is base *content*, not a
    // replacement base marker — the overwrite used to drop a line on render,
    // breaking the byte-exact roundtrip.
    #[test]
    fn repeated_base_marker_line_is_base_content() {
        let s = "<<<<<<<< HEAD\n|||||||| base\n|||||||| base\n========\n>>>>>>>> branché\n";
        let segments = parse_conflicts(s).expect("parse");
        let ConflictSegment::Conflict(region) = &segments[0] else {
            panic!("expected a conflict, got {segments:?}");
        };
        assert_eq!(
            region.base.as_deref(),
            Some(&["|||||||| base\n".to_string()][..]),
            "the second |-run line is content of the base section"
        );
        assert_eq!(render(&segments), s, "roundtrip must be byte-exact");
    }

    // Roundtrip must be byte-exact — including CRLF, custom marker sizes,
    // and a conflict at EOF with no trailing newline.
    #[test]
    fn render_roundtrips_exactly() {
        let crlf = "a\r\n<<<<<<< HEAD\r\nours\r\n=======\r\ntheirs\r\n>>>>>>> b\r\nz\r\n";
        let wide = "<<<<<<<<<<<<<<< HEAD\nours\n===============\ntheirs\n>>>>>>>>>>>>>>> b\n";
        let eof = "x\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> b";
        for sample in [MERGE_2WAY, DIFF3, crlf, wide, eof] {
            let segments = parse_conflicts(sample).expect("parse");
            assert_eq!(render(&segments), sample, "roundtrip");
        }
        // The wide sample detected the larger marker run.
        let segments = parse_conflicts(wide).unwrap();
        let ConflictSegment::Conflict(region) = &segments[0] else {
            panic!()
        };
        assert_eq!(region.marker_len, 15);
    }

    #[test]
    fn resolve_takes_one_side_everywhere() {
        let two = format!("{MERGE_2WAY}between\n{MERGE_2WAY}");
        let segments = parse_conflicts(&two).expect("parse");
        assert_eq!(
            resolve(&segments, ResolutionSide::Ours).unwrap(),
            "line 1\nmain line 2\nline 3\nbetween\nline 1\nmain line 2\nline 3\n"
        );
        assert_eq!(
            resolve(&segments, ResolutionSide::Theirs).unwrap(),
            "line 1\nfeature line 2\nline 3\nbetween\nline 1\nfeature line 2\nline 3\n"
        );
        // No base recorded in merge style → Base resolution is refused.
        assert!(resolve(&segments, ResolutionSide::Base).is_err());

        let diff3 = parse_conflicts(DIFF3).expect("parse");
        assert_eq!(
            resolve(&diff3, ResolutionSide::Base).unwrap(),
            "line 1\nline 2\nline 3\n"
        );
    }

    #[test]
    fn empty_sides_and_clean_files_parse() {
        // One side deleted everything.
        let deletion = "<<<<<<< HEAD\n=======\nkept\n>>>>>>> b\n";
        let segments = parse_conflicts(deletion).expect("parse");
        assert_eq!(resolve(&segments, ResolutionSide::Ours).unwrap(), "");
        // A file without conflicts is one text segment.
        let clean = parse_conflicts("just\ntext\n").expect("parse");
        assert_eq!(clean.len(), 1);
        assert!(!has_conflict_markers("just\ntext\n"));
        assert!(has_conflict_markers(MERGE_2WAY));
    }

    #[test]
    fn malformed_files_are_parse_errors() {
        // Only a genuinely broken *region* (an opener with no separator/terminator)
        // is an error.
        for bad in [
            "<<<<<<< HEAD\nours\n",                  // no separator
            "<<<<<<< HEAD\nours\n=======\ntheirs\n", // no terminator
        ] {
            assert!(
                matches!(parse_conflicts(bad), Err(Error::Parse { .. })),
                "{bad:?} must fail"
            );
        }
    }

    // A `=======`/`>>>>>>>` run outside any region is ordinary content (Markdown
    // underline, divider, quoted email), not a malformed conflict — parsed as text,
    // never an error, and round-trips byte-exact. (H6)
    #[test]
    fn marker_like_content_outside_a_region_is_text() {
        for content in [
            "Heading\n=======\nbody\n",          // RST/Markdown setext underline
            "a\n=======================\nb\n",   // divider banner
            ">>>>>>> deep email quote\nreply\n", // quoted email
            "code: a <<<<<<< b\n",               // marker run not at line start
        ] {
            let segments = parse_conflicts(content).expect("parses as text, no error");
            assert!(
                segments
                    .iter()
                    .all(|s| matches!(s, ConflictSegment::Text(_))),
                "{content:?} must be all text, got {segments:?}"
            );
            assert_eq!(render(&segments), content, "round-trips byte-exact");
        }
    }
}

// Property-based fuzzing. The marker grammar slices on marker-run lengths and
// must never panic on a hostile file (a real conflicted file from a git we
// don't control), and `render(parse(x)?) == x` must hold byte-for-byte — the
// regression net for the marker-detection / byte-offset logic.
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// A line drawn from the conflict-marker vocabulary plus multibyte text,
    /// with variable marker-run lengths (7..16) and CRLF, so a joined document
    /// reaches the marker-slicing branches with adversarial content.
    fn conflict_line() -> impl Strategy<Value = String> {
        prop_oneof![
            (7usize..16).prop_map(|n| format!("{} HEAD\n", "<".repeat(n))),
            (7usize..16).prop_map(|n| format!("{}\n", "=".repeat(n))),
            (7usize..16).prop_map(|n| format!("{} branché\n", ">".repeat(n))),
            (7usize..16).prop_map(|n| format!("{} base\n", "|".repeat(n))),
            "[a-zé<>=|]{0,14}\r?\n", // text incl. marker-ish chars + multibyte + CRLF
            Just("\n".to_string()),
        ]
    }

    fn conflict_doc() -> impl Strategy<Value = String> {
        prop::collection::vec(conflict_line(), 0..30).prop_map(|lines| lines.concat())
    }

    proptest! {
        #[test]
        fn parse_never_panics_on_arbitrary_text(s in any::<String>()) {
            let _ = has_conflict_markers(&s);
            // Whatever arbitrary text happens to parse must also round-trip
            // byte-exact — the load-bearing invariant, asserted on this generator
            // too (not just the structured one below).
            if let Ok(segments) = parse_conflicts(&s) {
                prop_assert_eq!(render(&segments), s);
            }
        }

        #[test]
        fn parse_never_panics_on_structured_text(s in conflict_doc()) {
            let _ = parse_conflicts(&s);
        }

        // The load-bearing invariant: whenever the file parses, re-rendering is
        // byte-exact.
        #[test]
        fn render_roundtrips_whatever_parses(s in conflict_doc()) {
            if let Ok(segments) = parse_conflicts(&s) {
                prop_assert_eq!(render(&segments), s);
            }
        }

        // A marker-free file is one Text segment that renders back unchanged.
        #[test]
        fn marker_free_files_are_a_single_text_segment(s in "[a-zé \t\r\n]{0,80}") {
            prop_assume!(!has_conflict_markers(&s));
            let segments = parse_conflicts(&s).expect("no markers → Ok");
            prop_assert_eq!(render(&segments), s);
        }
    }
}
