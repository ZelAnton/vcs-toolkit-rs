//! Pure parsers for git's machine-readable output. No process execution, so the
//! tests here are hermetic and run on CI.
//!
//! The git-format unified-diff model + parser and the version type live in the
//! shared [`vcs_diff`] crate (`git diff` and `jj diff --git` are byte-identical);
//! this module keeps only the git-specific parsers (porcelain, log, blame, …).

use std::path::PathBuf;

use vcs_diff::DiffStat;

/// One entry from `git status --porcelain=v1 -z` (`XY <path>`, NUL-delimited).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StatusEntry {
    /// Two-character status code, e.g. `" M"`, `"??"`, `"A "`, `"R "`.
    pub code: String,
    /// Path the status applies to (the *new* path for a rename/copy). Raw bytes
    /// from `-z` — no C-quoting/escaping to undo, even for paths with spaces.
    pub path: String,
    /// For a rename/copy, the original path; `None` otherwise. Named to match
    /// `vcs_jj::ChangedPath::old_path` so cross-backend code reads the rename
    /// source the same way on both wrappers.
    pub old_path: Option<String>,
}

/// A combined branch + working-tree snapshot from `git status --porcelain=v2
/// --branch -z`: HEAD, branch, upstream tracking, ahead/behind, and change
/// counts — everything a prompt/status-bar needs, in **one** process spawn.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct BranchStatus {
    /// The HEAD commit's full object id (`# branch.oid`); `None` on an unborn
    /// repo (git reports `(initial)`). Truncate for display.
    pub head: Option<String>,
    /// Current branch name (`# branch.head`); `None` when detached.
    pub branch: Option<String>,
    /// Upstream tracking branch (`# branch.upstream`); `None` when unset.
    pub upstream: Option<String>,
    /// Commits ahead of the upstream (`# branch.ab +A`); `None` when no upstream.
    pub ahead: Option<usize>,
    /// Commits behind the upstream (`# branch.ab -B`); `None` when no upstream.
    pub behind: Option<usize>,
    /// Count of changed *tracked* entries — modified/added/deleted/renamed/copied
    /// and unmerged (the `1`/`2`/`u` records).
    pub tracked_changes: usize,
    /// Count of untracked files (the `?` records).
    pub untracked: usize,
    /// Count of unmerged (conflicted) entries (the `u` records; also in
    /// `tracked_changes`).
    pub conflicts: usize,
}

impl BranchStatus {
    /// Whether the working tree has any change at all — tracked or untracked.
    pub fn is_dirty(&self) -> bool {
        self.tracked_changes > 0 || self.untracked > 0
    }
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

/// Parse `git status --porcelain=v1 -z` output: NUL-delimited records, raw
/// (unquoted) paths. A rename/copy entry is followed by its source path as the
/// next NUL record (e.g. `R  new\0old\0`).
pub(crate) fn parse_porcelain(output: &str) -> Vec<StatusEntry> {
    let mut entries = Vec::new();
    let mut records = output.split('\0').filter(|rec| !rec.is_empty());
    while let Some(rec) = records.next() {
        // "XY path": two status-code chars, a space, then the path. Real git
        // codes are ASCII, but slice via `get` so a malformed record (a
        // multibyte char where the code/space belong) is skipped, not a panic.
        let (Some(code), Some(path)) = (rec.get(..2), rec.get(3..)) else {
            continue;
        };
        // A rename/copy carries its source path as the immediately following NUL
        // record; consume it. The `R`/`C` can sit in EITHER status column — the index
        // column (`R ` staged rename) or the worktree column (` R` worktree rename) —
        // so check both. Missing the ` R`/` C` case left the source record as a
        // phantom entry with a garbage `code`/`path` (M11).
        let old_path = if matches!(code.as_bytes(), [b'R' | b'C', _] | [_, b'R' | b'C']) {
            records.next().map(str::to_string)
        } else {
            None
        };
        entries.push(StatusEntry {
            code: code.to_string(),
            path: path.to_string(),
            old_path,
        });
    }
    entries
}

/// Parse `git status --porcelain=v2 --branch -z` output into a [`BranchStatus`].
///
/// Records are NUL-terminated: `# branch.*` header lines first, then entry lines
/// (`1`/`2` changed, `u` unmerged, `?` untracked, `!` ignored). A `2` (rename/copy)
/// entry stores its original path as the *next* NUL record, so that record is
/// consumed and skipped. Everything is `strip_prefix`/compare based — no byte
/// indexing — so arbitrary bytes never panic (proven by proptest).
pub(crate) fn parse_porcelain_v2(output: &str) -> BranchStatus {
    let mut status = BranchStatus::default();
    let mut records = output.split('\0');
    while let Some(rec) = records.next() {
        if let Some(rest) = rec.strip_prefix("# branch.oid ") {
            // `(initial)` marks an unborn repo (no commits yet).
            status.head = (rest != "(initial)").then(|| rest.to_string());
        } else if let Some(rest) = rec.strip_prefix("# branch.head ") {
            status.branch = (rest != "(detached)").then(|| rest.to_string());
        } else if let Some(rest) = rec.strip_prefix("# branch.upstream ") {
            status.upstream = Some(rest.to_string());
        } else if let Some(rest) = rec.strip_prefix("# branch.ab ") {
            // `+<ahead> -<behind>`.
            let mut parts = rest.split(' ');
            status.ahead = parts
                .next()
                .and_then(|t| t.strip_prefix('+'))
                .and_then(|n| n.parse().ok());
            status.behind = parts
                .next()
                .and_then(|t| t.strip_prefix('-'))
                .and_then(|n| n.parse().ok());
        } else if rec.starts_with("1 ") {
            status.tracked_changes += 1;
        } else if rec.starts_with("2 ") {
            status.tracked_changes += 1;
            // The rename/copy original path is the next NUL record; consume it so
            // it isn't mis-read as another entry.
            records.next();
        } else if rec.starts_with("u ") {
            status.tracked_changes += 1;
            status.conflicts += 1;
        } else if rec.starts_with("? ") {
            status.untracked += 1;
        }
        // `! ` (ignored) and other `# ` headers contribute nothing.
    }
    status
}

/// Parse `git --version` output (`git version 2.54.0.windows.1`) into the shared
/// [`vcs_diff::Version`]: the first dotted-numeric token wins; non-numeric
/// trailers (`.windows.1`, `-rc1`) are ignored; a missing patch reads as `0`.
pub(crate) fn parse_git_version(raw: &str) -> Option<vcs_diff::Version> {
    vcs_diff::parse_dotted_version(raw)
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
/// (`<sha> <orig> <final> [<group count>]`, where `<sha>` is a 40-hex SHA-1 or a
/// 64-hex SHA-256 object id), a full set of `tag value` metadata lines (`author`,
/// `author-time`, …, optional `boundary`), then the content prefixed with a literal
/// TAB.
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
        // Header: a commit sha followed by line numbers (and an optional group
        // count, which only appears on a group's first line). Accept both SHA-1
        // (40 hex) and SHA-256 (64 hex) object ids — a SHA-256 repo would otherwise
        // never match, so `blame` would silently return an empty `Vec`.
        if (label.len() == 40 || label.len() == 64) && label.bytes().all(|b| b.is_ascii_hexdigit())
        {
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
                    old_path: None,
                },
                StatusEntry {
                    code: "??".into(),
                    path: "new file.txt".into(),
                    old_path: None,
                },
                StatusEntry {
                    code: "A ".into(),
                    path: "added.rs".into(),
                    old_path: None,
                },
            ]
        );
    }

    #[test]
    fn porcelain_parses_rename_with_old_path() {
        // `R  new\0old\0` — the source path is the next NUL record.
        let got = parse_porcelain("R  new.rs\0old.rs\0 M other.rs\0");
        assert_eq!(
            got,
            vec![
                StatusEntry {
                    code: "R ".into(),
                    path: "new.rs".into(),
                    old_path: Some("old.rs".into()),
                },
                StatusEntry {
                    code: " M".into(),
                    path: "other.rs".into(),
                    old_path: None,
                },
            ]
        );
    }

    // M11: a rename/copy in the WORKTREE column (` R`/` C`, not just the index `R `)
    // must also consume its source record — otherwise the source became a phantom
    // entry with a garbage code/path.
    #[test]
    fn porcelain_parses_worktree_rename_in_the_y_column() {
        // ` R new\0old\0` — space in X, R in Y (a worktree rename).
        let got = parse_porcelain(" R new.rs\0old.rs\0 M other.rs\0");
        assert_eq!(
            got,
            vec![
                StatusEntry {
                    code: " R".into(),
                    path: "new.rs".into(),
                    old_path: Some("old.rs".into()),
                },
                StatusEntry {
                    code: " M".into(),
                    path: "other.rs".into(),
                    old_path: None,
                },
            ],
            "the source record must be consumed, not left as a phantom entry"
        );
    }

    #[test]
    fn porcelain_ignores_blank_and_short_records() {
        assert!(parse_porcelain("\0  \0X\0").is_empty());
    }

    // Regression (found by proptest): a record whose leading char is multibyte
    // must be skipped, not panic on a non-char-boundary slice. `𝓁` is 4 bytes,
    // so byte index 2 lands inside it.
    #[test]
    fn porcelain_skips_non_ascii_status_records() {
        assert!(parse_porcelain("𝓁abc\0").is_empty());
        // A well-formed record alongside the garbage still parses.
        let entries = parse_porcelain("𝓁abc\0 M a.rs\0");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "a.rs");
    }

    #[test]
    fn porcelain_v2_parses_branch_and_change_counts() {
        // The rename's original path (`1 trap.rs`) is the next NUL record; it must
        // be CONSUMED, not counted as a fourth `1 …` change.
        let out = concat!(
            "# branch.oid abcdef1234567890\0",
            "# branch.head main\0",
            "# branch.upstream origin/main\0",
            "# branch.ab +2 -1\0",
            "1 .M N... 100644 100644 100644 1111 2222 a.rs\0",
            "2 R. N... 100644 100644 100644 3333 4444 R100 new.rs\0",
            "1 trap.rs\0",
            "u UU N... 100644 100644 100644 100644 5 6 7 conflict.rs\0",
            "? untracked.txt\0",
            "! ignored.txt\0",
        );
        let s = parse_porcelain_v2(out);
        assert_eq!(s.head.as_deref(), Some("abcdef1234567890"));
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind), (Some(2), Some(1)));
        assert_eq!(
            s.tracked_changes, 3,
            "1 + 2(rename) + u; the trap is consumed"
        );
        assert_eq!(s.untracked, 1);
        assert_eq!(s.conflicts, 1);
        assert!(s.is_dirty());
    }

    #[test]
    fn porcelain_v2_handles_unborn_detached_and_no_upstream() {
        // Unborn repo: `(initial)` oid, no ab line, clean tree.
        let s = parse_porcelain_v2("# branch.oid (initial)\0# branch.head main\0");
        assert_eq!(s.head, None);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream, None);
        assert_eq!((s.ahead, s.behind), (None, None));
        assert!(!s.is_dirty());

        // Detached HEAD, no upstream tracking.
        let s = parse_porcelain_v2("# branch.oid deadbeef\0# branch.head (detached)\0");
        assert_eq!(s.head.as_deref(), Some("deadbeef"));
        assert_eq!(s.branch, None);
        assert_eq!(s.upstream, None);
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

    // A SHA-256 repository emits 64-hex commit ids; the header must still be
    // recognised (the old `len()==40`-only check made `blame` return an empty Vec).
    #[test]
    fn blame_recognises_sha256_object_ids() {
        let sha = "c".repeat(64);
        let out = format!(
            "{sha} 1 1 1\nauthor Carol\nauthor-mail <c@x>\nauthor-time 1717700000\n\
             author-tz +0000\ncommitter Carol\nsummary s\nfilename f.txt\n\
             \tline\n"
        );
        let lines = parse_blame_porcelain(&out);
        assert_eq!(
            lines.len(),
            1,
            "a SHA-256 blame must parse, not drop to empty"
        );
        assert_eq!(lines[0].commit, sha);
        assert_eq!(lines[0].author, "Carol");
        assert_eq!(lines[0].content, "line");
    }

    #[test]
    fn git_version_parses_real_world_shapes() {
        // The Windows build trailer (`.windows.1`) is extra dotted components
        // beyond the patch; an `-rc1` suffix rides on the patch itself.
        let v = parse_git_version("git version 2.54.0.windows.1").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (2, 54, 0));
        let v = parse_git_version("git version 2.41.0-rc1").unwrap();
        assert_eq!((v.major, v.minor, v.patch), (2, 41, 0));
        let v = parse_git_version("git version 2.54").unwrap();
        assert_eq!(v.patch, 0, "missing patch defaults to 0");
        assert!(parse_git_version("no digits here").is_none());
        assert!(parse_git_version("git version unknowable").is_none());
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
        assert_eq!(got, DiffStat::new(3, 12, 4));
    }

    #[test]
    fn shortstat_tolerates_missing_clauses_and_empty() {
        // Pure-insertion diff omits deletions; no changes yields all zeros.
        let only_ins = parse_shortstat(" 1 file changed, 2 insertions(+)\n");
        assert_eq!(only_ins.insertions, 2);
        assert_eq!(only_ins.deletions, 0);
        assert_eq!(parse_shortstat(""), DiffStat::default());
    }
}

// Property-based fuzzing: the parsers are pure functions over *arbitrary* CLI
// text (a git on the user's machine we don't control), so the load-bearing
// invariant is "never panic, whatever the bytes". These feed both unconstrained
// Unicode and structure-biased inputs (real delimiters: NUL, tab, unit
// separator, `diff --git`, `@@` hunks, rename braces) so the fuzzer reaches the
// byte-offset branches, not just the early returns.
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// A line drawn from git's structural vocabulary plus multibyte text, so a
    /// joined document exercises the porcelain/diff/blame branches.
    fn structured_line() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("diff --git a/f b/f\n".to_string()),
            Just("--- a/f\n".to_string()),
            Just("+++ b/f\n".to_string()),
            Just("@@ -1,2 +3,4 @@ ctx\n".to_string()),
            Just("@@ -1 +1 @@\n".to_string()),
            Just("rename from {old => new}.rs\n".to_string()),
            Just("R100\told\tnew\n".to_string()),
            Just(format!("{}\n", "a".repeat(40))), // a 40-hex-ish blame header
            "[-+ ]?[a-zé\t]{0,12}\n",              // diff body / text incl. multibyte
            "[ MARD?]{0,2} [a-zé/]{0,8}\0",        // porcelain-ish NUL record
        ]
    }

    fn structured_doc() -> impl Strategy<Value = String> {
        prop::collection::vec(structured_line(), 0..40).prop_map(|lines| lines.concat())
    }

    proptest! {
        // Panic-freedom on completely arbitrary input.
        #[test]
        fn parsers_never_panic_on_arbitrary_text(s in any::<String>()) {
            let _ = parse_porcelain(&s);
            let _ = parse_porcelain_v2(&s);
            let _ = parse_log(&s);
            let _ = parse_branches(&s);
            let _ = parse_worktree_porcelain(&s);
            let _ = parse_blame_porcelain(&s);
            let _ = parse_shortstat(&s);
            let _ = parse_ls_remote_heads(&s);
            let _ = parse_nul_paths(&s);
            let _ = parse_git_version(&s);
        }

        // …and on structure-biased input that reaches the parsing branches.
        #[test]
        fn parsers_never_panic_on_structured_text(s in structured_doc()) {
            let _ = parse_porcelain(&s);
            let _ = parse_porcelain_v2(&s);
            let _ = parse_log(&s);
            let _ = parse_blame_porcelain(&s);
        }

        // porcelain v2 header/entry lines (with the `2`-consumes-next-record path)
        // must never panic on arbitrary NUL-joined records.
        #[test]
        fn porcelain_v2_never_panics(records in prop::collection::vec(
            prop_oneof![
                Just("# branch.oid (initial)".to_string()),
                Just("# branch.head main".to_string()),
                Just("# branch.ab +1 -2".to_string()),
                "1 [.MADRCU]{2} [a-zé /]{0,10}".prop_map(|s| s),
                "2 R\\. .* R100 [a-zé /]{0,8}".prop_map(|s| s),
                "u UU [a-zé /]{0,8}".prop_map(|s| s),
                "\\? [a-zé /]{0,8}".prop_map(|s| s),
                "[a-zé0-9# ]{0,12}".prop_map(|s| s),
            ],
            0..20,
        ).prop_map(|r| r.join("\0"))) {
            let _ = parse_porcelain_v2(&records);
        }
    }
}
