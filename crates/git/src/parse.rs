//! Pure parsers for git's machine-readable output. No process execution, so the
//! tests here are hermetic and run on CI.

/// One entry from `git status --porcelain=v1 -z` (`XY <path>`, NUL-delimited).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    /// Two-character status code, e.g. `" M"`, `"??"`, `"A "`, `"R "`.
    pub code: String,
    /// Path the status applies to (the *new* path for a rename/copy). Raw bytes
    /// from `-z` — no C-quoting/escaping to undo, even for paths with spaces.
    pub path: String,
    /// For a rename/copy, the original path; `None` otherwise.
    pub orig_path: Option<String>,
}

/// A commit, parsed from a `\x1f`-delimited `git log` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Full commit hash (`%H`).
    pub hash: String,
    /// Abbreviated commit hash (`%h`).
    pub short_hash: String,
    /// Author name (`%an`).
    pub author: String,
    /// Author date, strict ISO-8601 (`%aI`), e.g. `2026-05-31T10:00:00+00:00`.
    pub date: String,
    /// Subject line (`%s`).
    pub subject: String,
}

/// A local branch from `git branch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    /// Branch name.
    pub name: String,
    /// Whether this is the checked-out branch (the `*` marker).
    pub current: bool,
}

/// Parse `git status --porcelain=v1 -z` output: NUL-delimited records, raw
/// (unquoted) paths. A rename/copy entry is followed by its source path as the
/// next NUL record (e.g. `R  new\0old\0`).
pub(crate) fn parse_porcelain(output: &str) -> Vec<StatusEntry> {
    let mut entries = Vec::new();
    let mut records = output.split('\0').filter(|rec| !rec.is_empty());
    while let Some(rec) = records.next() {
        // "XY path": two ASCII code chars (always ASCII → byte-slicing is safe),
        // a space, then a non-empty path.
        if rec.len() < 4 {
            continue;
        }
        // A rename/copy (R/C in the index column) carries its source path as the
        // immediately following NUL record; consume it.
        let orig_path = if matches!(rec.as_bytes()[0], b'R' | b'C') {
            records.next().map(str::to_string)
        } else {
            None
        };
        entries.push(StatusEntry {
            code: rec[..2].to_string(),
            path: rec[3..].to_string(),
            orig_path,
        });
    }
    entries
}

/// Parse `git log -z --format=%H%x1f%h%x1f%an%x1f%aI%x1f%s` output: commits are
/// NUL-separated (robust to multi-line fields), fields split on the ASCII unit
/// separator.
pub(crate) fn parse_log(output: &str) -> Vec<Commit> {
    output
        .split('\0')
        .filter(|rec| !rec.is_empty())
        .filter_map(|rec| {
            let mut fields = rec.split('\u{1f}');
            Some(Commit {
                hash: fields.next()?.to_string(),
                short_hash: fields.next()?.to_string(),
                author: fields.next()?.to_string(),
                date: fields.next()?.to_string(),
                subject: fields.next().unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// Parse `git branch` output. The first column is the `* `/`  `/`+ ` marker.
pub(crate) fn parse_branches(output: &str) -> Vec<Branch> {
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let current = line.starts_with('*');
            let name = line.get(1..).unwrap_or("").trim();
            // Skip the detached-HEAD pseudo-entry, e.g. "* (HEAD detached at …)".
            if name.is_empty() || name.starts_with('(') {
                return None;
            }
            Some(Branch {
                name: name.to_string(),
                current,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_parses_codes_and_paths() {
        // NUL-delimited records; the path with a space stays raw (no quoting).
        let got = parse_porcelain(" M src/lib.rs\0?? new file.txt\0A  added.rs\0");
        assert_eq!(
            got,
            vec![
                StatusEntry {
                    code: " M".into(),
                    path: "src/lib.rs".into(),
                    orig_path: None,
                },
                StatusEntry {
                    code: "??".into(),
                    path: "new file.txt".into(),
                    orig_path: None,
                },
                StatusEntry {
                    code: "A ".into(),
                    path: "added.rs".into(),
                    orig_path: None,
                },
            ]
        );
    }

    #[test]
    fn porcelain_parses_rename_with_orig_path() {
        // `R  new\0old\0` — the source path is the next NUL record.
        let got = parse_porcelain("R  new.rs\0old.rs\0 M other.rs\0");
        assert_eq!(
            got,
            vec![
                StatusEntry {
                    code: "R ".into(),
                    path: "new.rs".into(),
                    orig_path: Some("old.rs".into()),
                },
                StatusEntry {
                    code: " M".into(),
                    path: "other.rs".into(),
                    orig_path: None,
                },
            ]
        );
    }

    #[test]
    fn porcelain_ignores_blank_and_short_records() {
        assert!(parse_porcelain("\0  \0X\0").is_empty());
    }

    #[test]
    fn log_splits_unit_separated_fields() {
        let input = "abc123\u{1f}abc\u{1f}Ada\u{1f}2026-05-31T10:00:00+00:00\u{1f}Add feature\0\
                     def456\u{1f}def\u{1f}Linus\u{1f}2026-05-30T09:00:00+00:00\u{1f}Fix bug\0";
        let got = parse_log(input);
        assert_eq!(got.len(), 2);
        assert_eq!(
            got[0],
            Commit {
                hash: "abc123".into(),
                short_hash: "abc".into(),
                author: "Ada".into(),
                date: "2026-05-31T10:00:00+00:00".into(),
                subject: "Add feature".into(),
            }
        );
        assert_eq!(got[1].subject, "Fix bug");
    }

    #[test]
    fn log_tolerates_empty_subject() {
        let got = parse_log("h\u{1f}h\u{1f}A\u{1f}2026-05-31T10:00:00+00:00\u{1f}\0");
        assert_eq!(got[0].subject, "");
    }

    #[test]
    fn branches_marks_current_and_skips_detached() {
        let got = parse_branches("* main\n  feature\n  (HEAD detached at abc123)\n");
        assert_eq!(
            got,
            vec![
                Branch {
                    name: "main".into(),
                    current: true
                },
                Branch {
                    name: "feature".into(),
                    current: false
                },
            ]
        );
    }
}
