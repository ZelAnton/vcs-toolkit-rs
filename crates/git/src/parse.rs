//! Pure parsers for git's machine-readable output. No process execution, so the
//! tests here are hermetic and run on CI.

use std::path::PathBuf;

/// One entry from `git status --porcelain=v1 -z` (`XY <path>`, NUL-delimited).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
pub struct Branch {
    /// Branch name.
    pub name: String,
    /// Whether this is the checked-out branch (the `*` marker).
    pub current: bool,
}

/// A worktree from `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Worktree {
    /// Absolute path to the worktree.
    pub path: PathBuf,
    /// Short branch name (`refs/heads/` stripped); `None` when detached or bare.
    pub branch: Option<String>,
    /// The checked-out commit (`HEAD <sha>`); `None` for a bare entry.
    pub head: Option<String>,
    /// The main worktree of a bare repository.
    pub bare: bool,
    /// Checked out at a detached HEAD (no branch).
    pub detached: bool,
    /// Locked against pruning.
    pub locked: bool,
}

/// Aggregate line/file counts from `git diff --shortstat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct DiffStat {
    /// Number of files changed.
    pub files_changed: usize,
    /// Lines added (`insertions(+)`).
    pub insertions: usize,
    /// Lines removed (`deletions(-)`).
    pub deletions: usize,
}

/// How a file changed in a unified diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
/// leading ` `/`+`/`-` marker.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Parse a NUL-delimited path list (e.g. `git diff --name-only -z`): one
/// repo-relative path per record, `/` separators, no quoting.
pub(crate) fn parse_nul_paths(output: &str) -> Vec<String> {
    output
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect()
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

/// Parse `git worktree list --porcelain`: records separated by a blank line,
/// each a set of `label [value]` lines — `worktree <path>`, `HEAD <sha>`,
/// `branch refs/heads/<name>`, plus the valueless attributes `bare` / `detached`
/// / `locked`. Unknown labels (e.g. `prunable`) are ignored.
pub(crate) fn parse_worktree_porcelain(output: &str) -> Vec<Worktree> {
    let mut worktrees = Vec::new();
    let mut current: Option<Worktree> = None;
    let flush = |current: &mut Option<Worktree>, out: &mut Vec<Worktree>| {
        if let Some(wt) = current.take() {
            out.push(wt);
        }
    };
    for line in output.lines() {
        if line.is_empty() {
            flush(&mut current, &mut worktrees);
            continue;
        }
        let (label, value) = match line.split_once(' ') {
            Some((l, v)) => (l, Some(v)),
            None => (line, None),
        };
        match label {
            // A new record begins; flush any record not closed by a blank line.
            "worktree" => {
                flush(&mut current, &mut worktrees);
                current = Some(Worktree {
                    path: PathBuf::from(value.unwrap_or("")),
                    branch: None,
                    head: None,
                    bare: false,
                    detached: false,
                    locked: false,
                });
            }
            "HEAD" => {
                if let Some(wt) = current.as_mut() {
                    wt.head = value.map(str::to_string);
                }
            }
            "branch" => {
                if let Some(wt) = current.as_mut() {
                    // Value is a full ref (`refs/heads/main`); expose the short name.
                    wt.branch =
                        value.map(|v| v.strip_prefix("refs/heads/").unwrap_or(v).to_string());
                }
            }
            "bare" => {
                if let Some(wt) = current.as_mut() {
                    wt.bare = true;
                }
            }
            "detached" => {
                if let Some(wt) = current.as_mut() {
                    wt.detached = true;
                }
            }
            "locked" => {
                if let Some(wt) = current.as_mut() {
                    wt.locked = true;
                }
            }
            _ => {}
        }
    }
    flush(&mut current, &mut worktrees);
    worktrees
}

/// One line of `git blame --line-porcelain` output: who last touched the line
/// and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BlameLine {
    /// Full hash of the commit that last changed the line.
    pub commit: String,
    /// Line number in that commit's version of the file (1-based).
    pub orig_line: u32,
    /// Line number in the blamed version of the file (1-based).
    pub final_line: u32,
    /// Author name of that commit.
    pub author: String,
    /// Author timestamp as a unix epoch (seconds).
    pub author_time: i64,
    /// Author timezone offset, e.g. `+0200`.
    pub author_tz: String,
    /// The line's content (without the trailing newline).
    pub content: String,
}

/// Parse `git blame --line-porcelain` output. Every line gets a header
/// (`<40-hex sha> <orig> <final> [<group count>]`), a full set of `tag value`
/// metadata lines (`author`, `author-time`, …, optional `boundary`), then the
/// content prefixed with a literal TAB.
pub(crate) fn parse_blame_porcelain(output: &str) -> Vec<BlameLine> {
    let mut lines = Vec::new();
    let mut current: Option<BlameLine> = None;
    for line in output.lines() {
        // Content line: closes the current record.
        if let Some(content) = line.strip_prefix('\t') {
            if let Some(mut entry) = current.take() {
                entry.content = content.to_string();
                lines.push(entry);
            }
            continue;
        }
        let (label, value) = match line.split_once(' ') {
            Some((l, v)) => (l, v),
            None => (line, ""),
        };
        // Header: a 40-hex sha followed by line numbers (and an optional group
        // count, which only appears on a group's first line).
        if label.len() == 40 && label.bytes().all(|b| b.is_ascii_hexdigit()) {
            let mut nums = value.split(' ');
            let orig = nums.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            let fin = nums.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            current = Some(BlameLine {
                commit: label.to_string(),
                orig_line: orig,
                final_line: fin,
                author: String::new(),
                author_time: 0,
                author_tz: String::new(),
                content: String::new(),
            });
            continue;
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        match label {
            "author" => entry.author = value.to_string(),
            "author-time" => entry.author_time = value.parse().unwrap_or(0),
            "author-tz" => entry.author_tz = value.to_string(),
            // committer*/summary/filename/previous/boundary intentionally not
            // captured — `#[non_exhaustive]` leaves room to add them later.
            _ => {}
        }
    }
    lines
}

/// Parse `git diff --shortstat`, e.g. ` 3 files changed, 12 insertions(+), 4
/// deletions(-)`. Any clause may be absent (a pure-insertion diff omits
/// deletions; no changes yields an empty string → all zeros).
pub(crate) fn parse_shortstat(output: &str) -> DiffStat {
    let mut stat = DiffStat::default();
    for part in output.split(',') {
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

/// Parse `git ls-remote --heads <remote>` output — `<sha>\trefs/heads/<name>`
/// per line — into the bare branch names.
pub(crate) fn parse_ls_remote_heads(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let (_sha, refname) = line.split_once('\t')?;
            refname
                .trim()
                .strip_prefix("refs/heads/")
                .map(str::to_string)
        })
        .collect()
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
    let path = rename_to
        .or(new_path)
        .or(minus_path)
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
    Some(s[idx + 1..].strip_prefix("b/").unwrap_or("").to_string())
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

    // --line-porcelain repeats the full metadata for every line; the group
    // count appears only on a group's first header, and `boundary` is a
    // valueless tag — both must parse.
    #[test]
    fn blame_line_porcelain_parses_headers_and_metadata() {
        let sha_a = "a".repeat(40);
        let sha_b = "b".repeat(40);
        let out = format!(
            "{sha_a} 1 1 2\nauthor Alice\nauthor-mail <a@x>\nauthor-time 1717500000\n\
             author-tz +0200\ncommitter Alice\nsummary first\nboundary\nfilename f.txt\n\
             \tline one\n\
             {sha_a} 2 2\nauthor Alice\nauthor-mail <a@x>\nauthor-time 1717500000\n\
             author-tz +0200\ncommitter Alice\nsummary first\nfilename f.txt\n\
             \tline two\n\
             {sha_b} 1 3 1\nauthor Bob\nauthor-mail <b@x>\nauthor-time 1717600000\n\
             author-tz -0500\ncommitter Bob\nsummary second\nfilename f.txt\n\
             \t\n"
        );
        let lines = parse_blame_porcelain(&out);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].commit, sha_a);
        assert_eq!(lines[0].orig_line, 1);
        assert_eq!(lines[0].final_line, 1);
        assert_eq!(lines[0].author, "Alice");
        assert_eq!(lines[0].author_time, 1717500000);
        assert_eq!(lines[0].author_tz, "+0200");
        assert_eq!(lines[0].content, "line one");
        // Second line of the same group: header without a group count.
        assert_eq!(lines[1].final_line, 2);
        assert_eq!(lines[1].content, "line two");
        // A different commit, and an empty content line stays empty.
        assert_eq!(lines[2].commit, sha_b);
        assert_eq!(lines[2].author, "Bob");
        assert_eq!(lines[2].content, "");
    }

    #[test]
    fn blame_ignores_garbage_and_empty_input() {
        assert!(parse_blame_porcelain("").is_empty());
        assert!(parse_blame_porcelain("not a header\n\torphan content\n").is_empty());
    }

    #[test]
    fn nul_paths_split_and_keep_special_characters() {
        assert_eq!(
            parse_nul_paths("a.rs\0sub/with space.rs\0"),
            ["a.rs", "sub/with space.rs"]
        );
        assert!(parse_nul_paths("").is_empty());
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

    #[test]
    fn worktrees_parse_branch_detached_and_bare() {
        let input = "worktree /repo\nHEAD abc123\nbranch refs/heads/main\n\
                     \nworktree /repo/wt\nHEAD def456\ndetached\n\
                     \nworktree /repo/bare\nbare\n";
        let got = parse_worktree_porcelain(input);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].path, PathBuf::from("/repo"));
        assert_eq!(got[0].branch.as_deref(), Some("main"));
        assert_eq!(got[0].head.as_deref(), Some("abc123"));
        assert!(got[1].detached && got[1].branch.is_none());
        assert!(got[2].bare && got[2].head.is_none());
    }

    #[test]
    fn worktrees_parse_last_record_without_trailing_blank() {
        // The final record may not be followed by a blank line.
        let got = parse_worktree_porcelain("worktree /only\nHEAD aaa\nbranch refs/heads/x\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].branch.as_deref(), Some("x"));
    }

    #[test]
    fn shortstat_parses_all_clauses() {
        let got = parse_shortstat(" 3 files changed, 12 insertions(+), 4 deletions(-)\n");
        assert_eq!(
            got,
            DiffStat {
                files_changed: 3,
                insertions: 12,
                deletions: 4
            }
        );
    }

    #[test]
    fn shortstat_tolerates_missing_clauses_and_empty() {
        // Pure-insertion diff omits deletions; no changes yields all zeros.
        let only_ins = parse_shortstat(" 1 file changed, 2 insertions(+)\n");
        assert_eq!(only_ins.insertions, 2);
        assert_eq!(only_ins.deletions, 0);
        assert_eq!(parse_shortstat(""), DiffStat::default());
    }

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
