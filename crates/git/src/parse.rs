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
    /// Path the status applies to (the *new* path for a rename/copy). A
    /// [`PathBuf`] built from the raw `-z` bytes (no C-quoting to undo, even for
    /// paths with spaces), so a filename whose bytes are not valid UTF-8 (legal on
    /// Unix) is carried losslessly and can be fed straight back into `add` /
    /// `commit_paths` — decoding it through `String::from_utf8_lossy` would
    /// substitute `U+FFFD` and address a different file.
    pub path: PathBuf,
    /// For a rename/copy, the original path; `None` otherwise. Named to match
    /// `vcs_jj::ChangedPath::old_path` so cross-backend code reads the rename
    /// source the same way on both wrappers.
    pub old_path: Option<PathBuf>,
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

/// One entry from `git stash list`, parsed via
/// `--format=%gd%x1f%H%x1f%gs -z`: the stash's position, the stashed commit's
/// hash, and its label split into an optional branch and the rest of the
/// message.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StashEntry {
    /// The stash's position in the list (`stash@{<index>}`'s `<index>`), most
    /// recent first (`0`) — the numeral [`crate::GitApi::stash_apply`] /
    /// [`crate::GitApi::stash_drop`] take.
    pub index: usize,
    /// The stashed commit's full object id (`%H`).
    pub hash: String,
    /// The branch checked out when the stash was pushed, from git's default
    /// `"WIP on <branch>: …"` / `stash push -m`'s `"On <branch>: …"` label;
    /// `None` when git recorded no branch (a detached HEAD, `"(no branch)"`).
    pub branch: Option<String>,
    /// The rest of the label: the default `<abbrev-sha> <subject>` when
    /// `stash push` was given no `-m`, or the caller's message verbatim when
    /// it was.
    pub message: String,
}

/// A worktree from `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Worktree {
    /// Absolute path to the worktree. A [`PathBuf`] built from the raw
    /// `worktree list --porcelain` bytes (via [`vcs_diff::path_from_bytes`]), so a
    /// worktree whose directory name is not valid UTF-8 (legal on Unix) is carried
    /// losslessly instead of being flattened to `U+FFFD` — the same platform-correct
    /// type `StatusEntry::path` uses, and what the facade's `WorktreeInfo.path`
    /// forwards.
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
///
/// Consumes **raw bytes** (not a lossily-decoded `&str`): the path is part of the
/// payload and, on Unix, need not be valid UTF-8 — decoding through
/// `String::from_utf8_lossy` first would corrupt it to `U+FFFD` and break the
/// round-trip back into `add`/`commit_paths`. The two-byte status code is ASCII;
/// only the path bytes are carried losslessly (via [`vcs_diff::path_from_bytes`]).
pub(crate) fn parse_porcelain(output: &[u8]) -> Vec<StatusEntry> {
    let mut entries = Vec::new();
    let mut records = output.split(|&b| b == 0).filter(|rec| !rec.is_empty());
    while let Some(rec) = records.next() {
        // "XY path": two status-code bytes, then a space at index 2, then the raw
        // path bytes. Require the separating space (git's porcelain always emits
        // it) so a malformed/short record — e.g. one whose leading bytes are a
        // multibyte char, where index 2 is not the space — is skipped, not turned
        // into a garbage entry.
        let (Some(code), Some(&b' ')) = (rec.get(..2), rec.get(2)) else {
            continue;
        };
        let path = &rec[3..];
        // A rename/copy carries its source path as the immediately following NUL
        // record; consume it. The `R`/`C` can sit in EITHER status column — the index
        // column (`R ` staged rename) or the worktree column (` R` worktree rename) —
        // so check both. Missing the ` R`/` C` case left the source record as a
        // phantom entry with a garbage `code`/`path` (M11).
        let old_path = if matches!(code, [b'R' | b'C', _] | [_, b'R' | b'C']) {
            records.next().map(vcs_diff::path_from_bytes)
        } else {
            None
        };
        entries.push(StatusEntry {
            // The status code is always 2 ASCII bytes, so this decode is exact.
            code: String::from_utf8_lossy(code).into_owned(),
            path: vcs_diff::path_from_bytes(path),
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
#[doc(hidden)]
pub fn parse_porcelain_v2(output: &str) -> BranchStatus {
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
///
/// Consumes **raw bytes** and yields [`PathBuf`]s (via
/// [`vcs_diff::path_from_bytes`]) so a non-UTF-8 conflicted/diff path survives
/// losslessly rather than being flattened to `U+FFFD` by a `&str` decode.
pub(crate) fn parse_nul_paths(output: &[u8]) -> Vec<PathBuf> {
    output
        .split(|&b| b == 0)
        .filter(|path| !path.is_empty())
        .map(vcs_diff::path_from_bytes)
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

/// Parse `git stash list -z --format=%gd%x1f%H%x1f%gs` output into
/// [`StashEntry`] records: NUL-separated entries (robust to a multi-line
/// message), `\x1f`-separated fields — the same framing [`parse_log`] uses. A
/// record whose selector isn't the expected `stash@{<n>}` shape (unexpected
/// git output) is skipped rather than turned into a garbage entry.
pub(crate) fn parse_stash_list(output: &str) -> Vec<StashEntry> {
    output
        .split('\0')
        .filter(|rec| !rec.is_empty())
        .filter_map(|rec| {
            let mut fields = rec.split('\u{1f}');
            let selector = fields.next()?;
            let hash = fields.next()?.to_string();
            let subject = fields.next().unwrap_or("");
            let index: usize = selector
                .strip_prefix("stash@{")?
                .strip_suffix('}')?
                .parse()
                .ok()?;
            let (branch, message) = parse_stash_subject(subject);
            Some(StashEntry {
                index,
                hash,
                branch,
                message,
            })
        })
        .collect()
}

/// Split a `git stash` reflog subject (`%gs`) into the branch it names and the
/// rest of the message. git's default label is `WIP on <branch>: <subject>`;
/// an explicit `stash push -m <msg>` instead records `On <branch>: <msg>`. A
/// detached-HEAD stash names the placeholder `(no branch)`, reported here as
/// `None` rather than that literal string. A subject matching neither shape
/// (an unrecognized or hand-crafted reflog entry) is returned whole as the
/// message, with no branch.
fn parse_stash_subject(subject: &str) -> (Option<String>, String) {
    let Some(rest) = subject
        .strip_prefix("WIP on ")
        .or_else(|| subject.strip_prefix("On "))
    else {
        return (None, subject.to_string());
    };
    match rest.split_once(": ") {
        Some((branch, message)) => {
            let branch = (branch != "(no branch)").then(|| branch.to_string());
            (branch, message.to_string())
        }
        None => (None, rest.to_string()),
    }
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
///
/// Consumes **raw bytes** (not a lossily-decoded `&str`): the `worktree <path>`
/// value is a filesystem path that, on Unix, need not be valid UTF-8, so its bytes
/// are carried losslessly (via [`vcs_diff::path_from_bytes`]) — a `String` decode
/// would substitute `U+FFFD` and make `Worktree.path` name a *different* directory,
/// the same defect the status/diff surface already avoids. The labels and the
/// text-typed values (`HEAD` sha, `branch` ref) are ASCII, so they still decode as
/// `String`.
///
/// This parses the **newline-framed** porcelain (no `-z`): git only grew
/// `worktree list --porcelain -z` in 2.36, above this crate's git-support floor
/// (2.31), and requesting `-z` there would hard-fail the listing. Newline framing
/// already covers the non-UTF-8 case this task targets — a path byte is never `\n`
/// — so only a worktree path containing a *literal newline* stays out of scope,
/// exactly as before this change.
pub(crate) fn parse_worktree_porcelain(output: &[u8]) -> Vec<Worktree> {
    let mut worktrees = Vec::new();
    let mut current: Option<Worktree> = None;
    let flush = |current: &mut Option<Worktree>, out: &mut Vec<Worktree>| {
        if let Some(wt) = current.take() {
            out.push(wt);
        }
    };
    for line in output.split(|&b| b == b'\n') {
        if line.is_empty() {
            flush(&mut current, &mut worktrees);
            continue;
        }
        // `label value`, split on the FIRST ASCII space (the path itself may hold
        // spaces); a valueless attribute (`bare`/`detached`/`locked`) has none.
        let (label, value) = match line.iter().position(|&b| b == b' ') {
            Some(i) => (&line[..i], Some(&line[i + 1..])),
            None => (line, None),
        };
        match label {
            // A new record begins; flush any record not closed by a blank line.
            b"worktree" => {
                flush(&mut current, &mut worktrees);
                current = Some(Worktree {
                    // Raw path bytes → `PathBuf`, lossless on Unix.
                    path: value.map(vcs_diff::path_from_bytes).unwrap_or_default(),
                    branch: None,
                    head: None,
                    bare: false,
                    detached: false,
                    locked: false,
                });
            }
            b"HEAD" => {
                if let Some(wt) = current.as_mut() {
                    wt.head = value.map(|v| String::from_utf8_lossy(v).into_owned());
                }
            }
            b"branch" => {
                if let Some(wt) = current.as_mut() {
                    // Value is a full ref (`refs/heads/main`); expose the short name.
                    wt.branch = value.map(|v| {
                        let full = String::from_utf8_lossy(v);
                        full.strip_prefix("refs/heads/")
                            .unwrap_or(&full)
                            .to_string()
                    });
                }
            }
            b"bare" => {
                if let Some(wt) = current.as_mut() {
                    wt.bare = true;
                }
            }
            b"detached" => {
                if let Some(wt) = current.as_mut() {
                    wt.detached = true;
                }
            }
            b"locked" => {
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

/// One path `git clean` would remove (`-n`, dry run) or removed (`-f`,
/// forced), from a `Would remove <path>` / `Removing <path>` output line.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CleanEntry {
    /// The path, decoded from git's C-quoting (unquoted the same way this
    /// crate unquotes any other git porcelain path) and stripped of the
    /// directory-entry trailing `/` when [`is_dir`](Self::is_dir) is set.
    pub path: PathBuf,
    /// Whether this entry names a whole untracked **directory** (`-d`), from
    /// git's trailing `/` on directory entries, rather than a single file.
    pub is_dir: bool,
}

/// Parse `git clean -n`/`-f` output: one line per candidate/removed path,
/// `Would remove <path>` (dry run, `-n`) or `Removing <path>` (forced, `-f`,
/// unless `-q`). Any other line — e.g. `Skipping repository <path>` for a
/// nested untracked `.git`, or a `warning:`/`fatal:` line — names neither a
/// delete candidate nor a deleted path, so it is ignored rather than
/// mis-parsed as one.
///
/// `git clean` has no `-z`/NUL machine framing, so this parses newline-framed
/// text (`str::lines` also strips a CRLF `\r`, so Windows output parses
/// identically); a path needing escaping is C-quoted like any other git
/// porcelain path — see [`unquote_clean_path`].
pub(crate) fn parse_clean_output(output: &str) -> Vec<CleanEntry> {
    output
        .lines()
        .filter_map(|line| {
            let rest = line
                .strip_prefix("Would remove ")
                .or_else(|| line.strip_prefix("Removing "))?;
            let mut decoded = unquote_clean_path(rest);
            let is_dir = decoded.last() == Some(&b'/');
            if is_dir {
                decoded.pop();
            }
            Some(CleanEntry {
                path: vcs_diff::path_from_bytes(&decoded),
                is_dir,
            })
        })
        .collect()
}

/// Decode a `git clean` path the same way git quotes any other porcelain
/// path: wrapped in double quotes and C-escaped when it holds a control byte,
/// a `"`, a `\`, or — with the default `core.quotePath=true` — any non-ASCII
/// byte (e.g. `é` → `\303\251`, pure ASCII in the quoted form, so decoding
/// `git clean`'s stdout as `&str` first never corrupts a quoted path; only an
/// *unquoted* non-UTF-8 byte, which requires `core.quotePath=false` plus a
/// non-UTF-8 filename, stays out of scope). An unquoted path (no leading `"`)
/// is returned unchanged. Mirrors the diff-header path-unquoting `vcs_diff`
/// uses internally for `git diff`'s `a/`/`b/` headers; kept as a small local
/// copy since `git clean` has no `-z` machine framing to prefer instead, and
/// the two crates' quoting rules are otherwise unrelated.
fn unquote_clean_path(s: &str) -> Vec<u8> {
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
/// deletions; no changes yields an empty string → all zeros). Delegates to the
/// shared [`DiffStat::parse`] (also used by `vcs_jj::parse::parse_diff_stat`),
/// which both crates' callers force the **C locale** for — see `c_locale` at
/// the `shortstat`/`diff --stat` call sites.
pub(crate) fn parse_shortstat(output: &str) -> DiffStat {
    DiffStat::parse(output)
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

/// One configured Git remote, as listed by `git remote -v`.
///
/// Git emits one row for each fetch and push URL. `parse_remotes` coalesces
/// those rows to one remote name and prefers its fetch URL.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Remote {
    /// Configured remote name (for example, `origin`).
    pub name: String,
    /// The remote's fetch URL.
    pub url: String,
}

/// Parse `git remote -v` output into one row per configured remote.
///
/// The normal format is `<name> <url> (fetch)` followed by a matching `(push)`
/// row. Rows with a name and URL but no recognised direction are tolerated as a
/// fallback, while a recognised fetch row always replaces an earlier fallback
/// or push URL. Malformed/blank rows are ignored rather than aborting a whole
/// listing because a future Git display-format change should remain diagnosable
/// without making the configured remotes disappear behind a parser error.
pub(crate) fn parse_remotes(output: &str) -> Vec<Remote> {
    let mut remotes: Vec<(Remote, bool)> = Vec::new();

    for line in output.lines() {
        let mut fields = line.split_whitespace();
        let (Some(name), Some(url)) = (fields.next(), fields.next()) else {
            continue;
        };
        let is_fetch = matches!(fields.next(), Some("(fetch)"));

        if let Some((remote, has_fetch)) =
            remotes.iter_mut().find(|(remote, _)| remote.name == name)
        {
            if is_fetch && !*has_fetch {
                remote.url = url.to_string();
                *has_fetch = true;
            }
        } else {
            remotes.push((
                Remote {
                    name: name.to_string(),
                    url: url.to_string(),
                },
                is_fetch,
            ));
        }
    }

    remotes.into_iter().map(|(remote, _)| remote).collect()
}

/// One submodule declared in the superproject's `.gitmodules`, parsed from the
/// machine-unambiguous `git config --file .gitmodules --list -z` source rather
/// than a hand-rolled text scan of the ini-style file.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Submodule {
    /// The subsection name — the quoted key in `[submodule "<name>"]`. Usually
    /// equal to [`path`](Self::path), but git allows the two to differ (a
    /// renamed submodule keeps its original section name), so it is captured
    /// separately.
    pub name: String,
    /// `submodule.<name>.path` — the repo-relative mount point of the submodule.
    /// A [`PathBuf`] built from the raw config bytes (via
    /// [`vcs_diff::path_from_bytes`]), so a path that is not valid UTF-8 (legal
    /// on Unix) is carried losslessly, matching [`StatusEntry::path`].
    pub path: PathBuf,
    /// `submodule.<name>.url` — the upstream the submodule is fetched from.
    /// Empty when the entry declares no `url` (a malformed `.gitmodules`).
    pub url: String,
    /// `submodule.<name>.branch`, the tracked branch for
    /// `git submodule update --remote`; `None` when unset.
    pub branch: Option<String>,
}

/// The sync state of a submodule, from the one-character prefix in the
/// `git submodule status` output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubmoduleState {
    /// Initialized, and the checked-out commit matches the commit the
    /// superproject records for it — no prefix (a leading space) in
    /// `git submodule status`.
    Current,
    /// Not initialized (`-` prefix): the working tree is absent, so
    /// `git submodule update --init` is needed before the submodule can be used.
    Uninitialized,
    /// The currently checked-out submodule commit does **not** match the commit
    /// recorded in the superproject's index (`+` prefix) — the working submodule
    /// is out of sync with the recorded gitlink.
    RevisionMismatch,
    /// The submodule has unresolved merge conflicts (`U` prefix).
    Conflict,
}

/// One entry from `git submodule status`: the checked-out commit, the mount
/// path, and the sync [`state`](Self::state) derived from the line's leading
/// prefix character.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SubmoduleStatus {
    /// The repo-relative mount path of the submodule. A [`PathBuf`] built from
    /// the raw bytes (via [`vcs_diff::path_from_bytes`]), lossless on Unix.
    pub path: PathBuf,
    /// The submodule commit `git submodule status` reports: the checked-out
    /// commit when initialized, or the commit the superproject records (the
    /// gitlink) when uninitialized. Full object id (40-hex SHA-1 or 64-hex
    /// SHA-256).
    pub sha: String,
    /// The sync state from the line's prefix character.
    pub state: SubmoduleState,
    /// The trailing `git describe` of the submodule HEAD (the `(…)` suffix) —
    /// e.g. `heads/main`, a tag, or an abbreviated sha; `None` for an
    /// uninitialized submodule, which has no such suffix.
    pub describe: Option<String>,
}

/// Parse `git config --file .gitmodules --list -z` output into the declared
/// submodules, preserving `.gitmodules` file order.
///
/// The `-z` framing makes each record `key\nvalue`, records separated by NUL —
/// robust against a value containing `=` (which the non-`-z` `key=value` form
/// would mis-split) or whitespace. Only `submodule.<name>.<attr>` keys are
/// consumed; `<name>` is everything between `submodule.` and the final `.`
/// (`rsplit_once`), so a subsection name that itself contains dots or slashes
/// (e.g. `libs/sub`) is recovered intact while the trailing `<attr>`
/// (`path`/`url`/`branch`, lowercased by git) is read off the end.
pub(crate) fn parse_gitmodules_config(output: &[u8]) -> Vec<Submodule> {
    let mut subs: Vec<Submodule> = Vec::new();
    for record in output.split(|&b| b == 0).filter(|r| !r.is_empty()) {
        // `key\nvalue`: split on the FIRST newline (the value may itself contain
        // newlines under `-z`, though path/url/branch never do). A record with no
        // newline is a bare valueless key → empty value.
        let (key_bytes, value_bytes) = match record.iter().position(|&b| b == b'\n') {
            Some(i) => (&record[..i], &record[i + 1..]),
            None => (record, &b""[..]),
        };
        // Keys are ASCII config identifiers; a lossy decode is exact for them.
        let key = String::from_utf8_lossy(key_bytes);
        let Some(rest) = key.strip_prefix("submodule.") else {
            continue;
        };
        // `<name>.<attr>` — the attr is the final dot-component; the name is
        // everything before it (and may contain dots/slashes itself).
        let Some((name, attr)) = rest.rsplit_once('.') else {
            continue;
        };
        // Find-or-insert by name, preserving first-seen (file) order.
        let sub = match subs.iter_mut().find(|s| s.name == name) {
            Some(existing) => existing,
            None => {
                subs.push(Submodule {
                    name: name.to_string(),
                    path: PathBuf::new(),
                    url: String::new(),
                    branch: None,
                });
                subs.last_mut().expect("just pushed")
            }
        };
        match attr {
            "path" => sub.path = vcs_diff::path_from_bytes(value_bytes),
            "url" => sub.url = String::from_utf8_lossy(value_bytes).into_owned(),
            "branch" => sub.branch = Some(String::from_utf8_lossy(value_bytes).into_owned()),
            // Other keys (update/ignore/shallow/…) intentionally not captured;
            // `#[non_exhaustive]` leaves room to add them later.
            _ => {}
        }
    }
    subs
}

/// Parse `git submodule status` output into typed entries.
///
/// Each line is `<prefix><sha> <path>[ (<describe>)]`, where `<prefix>` is a
/// single status character (a space, `-`, `+`, or `U`; see [`SubmoduleState`]).
/// `git submodule status` has no `-z`/NUL framing, so the path is separated
/// from the optional trailing ` (<describe>)` heuristically: when the line ends
/// in `)`, the last ` (` opens the describe suffix and everything before it is
/// the path; otherwise the whole remainder after the sha is the path. A line
/// whose leading byte is not one of the four known status characters is skipped
/// as unrecognized rather than mis-parsed.
pub(crate) fn parse_submodule_status(output: &[u8]) -> Vec<SubmoduleStatus> {
    let mut entries = Vec::new();
    for line in output.split(|&b| b == b'\n') {
        // Trim a trailing CR so CRLF-framed output (Windows) parses identically.
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let state = match line[0] {
            b' ' => SubmoduleState::Current,
            b'-' => SubmoduleState::Uninitialized,
            b'+' => SubmoduleState::RevisionMismatch,
            b'U' => SubmoduleState::Conflict,
            // No recognized prefix — skip rather than fold the first byte into
            // the sha and emit a corrupt entry.
            _ => continue,
        };
        let rest = &line[1..];
        // `<sha> <path>…`: the sha runs up to the first space.
        let Some(sp) = rest.iter().position(|&b| b == b' ') else {
            continue;
        };
        let sha = String::from_utf8_lossy(&rest[..sp]).into_owned();
        let tail = &rest[sp + 1..];
        // Split off a trailing ` (<describe>)` suffix, if present.
        let (path_bytes, describe) = match tail.last() {
            Some(b')') => match tail
                .windows(2)
                .rposition(|w| w == b" (")
                .filter(|&i| i + 2 < tail.len())
            {
                Some(i) => (
                    &tail[..i],
                    Some(String::from_utf8_lossy(&tail[i + 2..tail.len() - 1]).into_owned()),
                ),
                None => (tail, None),
            },
            _ => (tail, None),
        };
        entries.push(SubmoduleStatus {
            path: vcs_diff::path_from_bytes(path_bytes),
            sha,
            state,
            describe,
        });
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_parses_codes_and_paths() {
        // NUL-delimited records; the path with a space stays raw (no quoting).
        let got = parse_porcelain(b" M src/lib.rs\0?? new file.txt\0A  added.rs\0");
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

    // A path whose bytes are not valid UTF-8 (legal on Unix) survives byte-for-byte
    // through `parse_porcelain` — the load-bearing property for the status→add
    // round-trip. `0xFF` is never valid UTF-8; the old `from_utf8_lossy` path would
    // have replaced it with U+FFFD and named a different file.
    #[cfg(unix)]
    #[test]
    fn porcelain_preserves_non_utf8_path_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let got = parse_porcelain(b" M caf\xff.txt\0");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path.as_os_str().as_bytes(), b"caf\xff.txt");
    }

    #[test]
    fn porcelain_parses_rename_with_old_path() {
        // `R  new\0old\0` — the source path is the next NUL record.
        let got = parse_porcelain(b"R  new.rs\0old.rs\0 M other.rs\0");
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
        let got = parse_porcelain(b" R new.rs\0old.rs\0 M other.rs\0");
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
        assert!(parse_porcelain(b"\0  \0X\0").is_empty());
    }

    // A record whose leading char is multibyte has no space at index 2, so it is
    // skipped (git's porcelain always emits `XY<space>path`). `𝓁` is 4 bytes, so
    // the byte at index 2 is a continuation byte, not the separating space.
    #[test]
    fn porcelain_skips_non_ascii_status_records() {
        assert!(parse_porcelain("𝓁abc\0".as_bytes()).is_empty());
        // A well-formed record alongside the garbage still parses.
        let entries = parse_porcelain("𝓁abc\0 M a.rs\0".as_bytes());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, std::path::Path::new("a.rs"));
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
            parse_nul_paths(b"a.rs\0sub/with space.rs\0"),
            [PathBuf::from("a.rs"), PathBuf::from("sub/with space.rs")]
        );
        assert!(parse_nul_paths(b"").is_empty());
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
        let got = parse_worktree_porcelain(input.as_bytes());
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].path, PathBuf::from("/repo"));
        assert_eq!(got[0].branch.as_deref(), Some("main"));
        assert_eq!(got[0].head.as_deref(), Some("abc123"));
        assert!(got[1].detached && got[1].branch.is_none());
        assert!(got[2].bare && got[2].head.is_none());
    }

    // A worktree whose directory name is not valid UTF-8 (legal on Unix) survives
    // byte-for-byte through `parse_worktree_porcelain`, so the facade's
    // `WorktreeInfo.path` addresses the SAME directory. `0xFF` is never valid UTF-8;
    // the old `&str` (`from_utf8_lossy`) parse would have replaced it with U+FFFD.
    #[cfg(unix)]
    #[test]
    fn worktrees_preserve_non_utf8_path_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let got = parse_worktree_porcelain(b"worktree /repo/wt-caf\xff\nHEAD abc123\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path.as_os_str().as_bytes(), b"/repo/wt-caf\xff");
        assert_eq!(got[0].head.as_deref(), Some("abc123"));
    }

    #[test]
    fn worktrees_parse_last_record_without_trailing_blank() {
        // The final record may not be followed by a blank line.
        let got = parse_worktree_porcelain(b"worktree /only\nHEAD aaa\nbranch refs/heads/x\n");
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

    #[test]
    fn gitmodules_config_parses_z_framed_records() {
        // `-z` layout: `key\nvalue\0` per record. Two attributes per submodule,
        // in `.gitmodules` order, and a subsection name containing a slash.
        let out = b"submodule.libs/sub.path\nlibs/sub\0\
                    submodule.libs/sub.url\n../sub\0\
                    submodule.libs/sub.branch\nmain\0\
                    submodule.second.path\nsecond\0\
                    submodule.second.url\n../sub\0";
        let got = parse_gitmodules_config(out);
        assert_eq!(
            got,
            vec![
                Submodule {
                    name: "libs/sub".into(),
                    path: "libs/sub".into(),
                    url: "../sub".into(),
                    branch: Some("main".into()),
                },
                Submodule {
                    name: "second".into(),
                    path: "second".into(),
                    url: "../sub".into(),
                    branch: None,
                },
            ]
        );
    }

    #[test]
    fn gitmodules_config_keeps_value_with_equals_and_ignores_non_submodule_keys() {
        // A value containing `=` survives (the non-`-z` `key=value` split would
        // corrupt it); a non-`submodule.*` key is ignored.
        let out = b"submodule.x.url\nhttps://h/r?a=b\0\
                    core.autocrlf\nfalse\0\
                    submodule.x.path\nx\0";
        let got = parse_gitmodules_config(out);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].url, "https://h/r?a=b");
        assert_eq!(got[0].path, PathBuf::from("x"));
    }

    #[test]
    fn gitmodules_config_empty_is_no_submodules() {
        assert!(parse_gitmodules_config(b"").is_empty());
    }

    #[test]
    fn remotes_empty_output_is_empty() {
        assert!(parse_remotes("\n \t\r\n").is_empty());
    }

    #[test]
    fn remotes_one_remote_prefers_fetch_url() {
        assert_eq!(
            parse_remotes(
                "origin\thttps://example.test/fetch.git (fetch)\norigin\thttps://example.test/push.git (push)\n"
            ),
            vec![Remote {
                name: "origin".into(),
                url: "https://example.test/fetch.git".into(),
            }]
        );
    }

    #[test]
    fn remotes_multiple_rows_dedupe_and_tolerate_malformed_output() {
        assert_eq!(
            parse_remotes(
                "origin ssh://example.test/push.git (push)\n\
                 upstream https://example.test/upstream.git (fetch)\r\n\
                 malformed-only-name\n\
                 origin https://example.test/fetch.git (fetch)\n\
                 upstream https://example.test/upstream-push.git (push)\n",
            ),
            vec![
                Remote {
                    name: "origin".into(),
                    url: "https://example.test/fetch.git".into(),
                },
                Remote {
                    name: "upstream".into(),
                    url: "https://example.test/upstream.git".into(),
                },
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn gitmodules_config_preserves_non_utf8_path_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let out = b"submodule.s.path\ncaf\xff/sub\0submodule.s.url\n../sub\0";
        let got = parse_gitmodules_config(out);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path.as_os_str().as_bytes(), b"caf\xff/sub");
    }

    #[test]
    fn submodule_status_parses_all_prefix_states() {
        // One line per state: current (space), revision-mismatch (+), conflict
        // (U), uninitialized (-, no describe suffix).
        let out = b" 833caa0 libs/sub (heads/main)\n\
                    +530fd06 plus/mod (530fd06)\n\
                    U000aaaa conf/mod (heads/topic)\n\
                    -deadbee minus/mod\n";
        let got = parse_submodule_status(out);
        assert_eq!(got.len(), 4);

        assert_eq!(got[0].state, SubmoduleState::Current);
        assert_eq!(got[0].sha, "833caa0");
        assert_eq!(got[0].path, PathBuf::from("libs/sub"));
        assert_eq!(got[0].describe.as_deref(), Some("heads/main"));

        assert_eq!(got[1].state, SubmoduleState::RevisionMismatch);
        assert_eq!(got[1].path, PathBuf::from("plus/mod"));
        assert_eq!(got[1].describe.as_deref(), Some("530fd06"));

        assert_eq!(got[2].state, SubmoduleState::Conflict);
        assert_eq!(got[2].path, PathBuf::from("conf/mod"));

        assert_eq!(got[3].state, SubmoduleState::Uninitialized);
        assert_eq!(got[3].sha, "deadbee");
        assert_eq!(got[3].path, PathBuf::from("minus/mod"));
        assert_eq!(got[3].describe, None);
    }

    #[test]
    fn submodule_status_handles_spaced_path_and_crlf() {
        // A path containing a space is kept whole (the ` (describe)` suffix is
        // split off from the END), and a CRLF line terminator parses identically.
        let out = b" abc123 dir with space/sub (v1.0)\r\n";
        let got = parse_submodule_status(out);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, PathBuf::from("dir with space/sub"));
        assert_eq!(got[0].describe.as_deref(), Some("v1.0"));
    }

    #[test]
    fn submodule_status_without_describe_keeps_full_path() {
        // No trailing `(...)`: the whole remainder after the sha is the path.
        let out = b" abc123 libs/no-describe\n";
        let got = parse_submodule_status(out);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, PathBuf::from("libs/no-describe"));
        assert_eq!(got[0].describe, None);
    }

    #[test]
    fn submodule_status_empty_is_no_entries() {
        assert!(parse_submodule_status(b"").is_empty());
    }

    #[test]
    fn stash_list_parses_default_and_custom_labels() {
        // Entry 0: `stash push -m "my label"` on `feature`. Entry 1: a plain
        // `stash push` (no `-m`), whose default label embeds the abbrev sha +
        // subject of the commit stashed on top of.
        let out = concat!(
            "stash@{0}\u{1f}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\u{1f}",
            "On feature: my label\0",
            "stash@{1}\u{1f}bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\u{1f}",
            "WIP on feature: f1c02c2 init\0",
        );
        let got = parse_stash_list(out);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].index, 0);
        assert_eq!(got[0].hash, "a".repeat(40));
        assert_eq!(got[0].branch.as_deref(), Some("feature"));
        assert_eq!(got[0].message, "my label");
        assert_eq!(got[1].index, 1);
        assert_eq!(got[1].branch.as_deref(), Some("feature"));
        assert_eq!(got[1].message, "f1c02c2 init");
    }

    #[test]
    fn stash_list_detached_head_has_no_branch() {
        let out = "stash@{0}\u{1f}cccccccccccccccccccccccccccccccccccccccc\u{1f}\
                    On (no branch): detached label\0";
        let got = parse_stash_list(out);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].branch, None);
        assert_eq!(got[0].message, "detached label");
    }

    #[test]
    fn stash_list_empty_is_no_entries() {
        assert!(parse_stash_list("").is_empty());
    }

    #[test]
    fn stash_list_skips_a_record_with_an_unrecognized_selector() {
        // A malformed/foreign selector (not `stash@{<n>}`) must be skipped, not
        // turned into a garbage entry with index 0.
        let out = "not-a-selector\u{1f}deadbeef\u{1f}subject\0";
        assert!(parse_stash_list(out).is_empty());
    }

    #[test]
    fn clean_output_parses_dry_run_files_and_directories() {
        let out = "Would remove junk.txt\nWould remove sub/\n";
        let got = parse_clean_output(out);
        assert_eq!(
            got,
            vec![
                CleanEntry {
                    path: PathBuf::from("junk.txt"),
                    is_dir: false,
                },
                CleanEntry {
                    path: PathBuf::from("sub"),
                    is_dir: true,
                },
            ]
        );
    }

    #[test]
    fn clean_output_parses_forced_removals() {
        let out = "Removing junk.txt\nRemoving sub/\n";
        let got = parse_clean_output(out);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].path, PathBuf::from("junk.txt"));
        assert!(!got[0].is_dir);
        assert_eq!(got[1].path, PathBuf::from("sub"));
        assert!(got[1].is_dir);
    }

    #[test]
    fn clean_output_unquotes_c_quoted_paths() {
        // `é` under the default `core.quotePath=true` is octal-escaped
        // (`\303\251`); the directory's trailing `/` sits INSIDE the quotes.
        let out = "Would remove \"caf\\303\\251.txt\"\nWould remove \"w\\303\\251ird dir/\"\n";
        let got = parse_clean_output(out);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].path, PathBuf::from("café.txt"));
        assert!(!got[0].is_dir);
        assert_eq!(got[1].path, PathBuf::from("wéird dir"));
        assert!(got[1].is_dir);
    }

    #[test]
    fn clean_output_ignores_unrecognized_lines() {
        // `Skipping repository …` (a nested untracked `.git`) names neither a
        // delete candidate nor a deleted path.
        let out = "Skipping repository sub/nested\nWould remove real.txt\n";
        let got = parse_clean_output(out);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, PathBuf::from("real.txt"));
    }

    #[test]
    fn clean_output_empty_is_no_entries() {
        assert!(parse_clean_output("").is_empty());
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
            let _ = parse_porcelain(s.as_bytes());
            let _ = parse_porcelain_v2(&s);
            let _ = parse_log(&s);
            let _ = parse_branches(&s);
            let _ = parse_worktree_porcelain(s.as_bytes());
            let _ = parse_blame_porcelain(&s);
            let _ = parse_shortstat(&s);
            let _ = parse_ls_remote_heads(&s);
            let _ = parse_remotes(&s);
            let _ = parse_nul_paths(s.as_bytes());
            let _ = parse_git_version(&s);
            let _ = parse_stash_list(&s);
            let _ = parse_clean_output(&s);
        }

        // The byte parsers must also never panic on *arbitrary bytes* — the actual
        // shape of a `-z` stream carrying a non-UTF-8 path, which the `String`
        // generator above can never produce.
        #[test]
        fn byte_parsers_never_panic_on_arbitrary_bytes(b in any::<Vec<u8>>()) {
            let _ = parse_porcelain(&b);
            let _ = parse_nul_paths(&b);
            let _ = parse_worktree_porcelain(&b);
        }

        // …and on structure-biased input that reaches the parsing branches.
        #[test]
        fn parsers_never_panic_on_structured_text(s in structured_doc()) {
            let _ = parse_porcelain(s.as_bytes());
            let _ = parse_porcelain_v2(&s);
            let _ = parse_log(&s);
            let _ = parse_blame_porcelain(&s);
            let _ = parse_gitmodules_config(s.as_bytes());
            let _ = parse_submodule_status(s.as_bytes());
            let _ = parse_stash_list(&s);
            let _ = parse_clean_output(&s);
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
