//! Pure parsers for jj output. No process execution, so these tests are
//! hermetic and run on CI.
//!
//! The git-format unified-diff model + parser and the version type live in the
//! shared [`vcs_diff`] crate (`jj diff --git` and `git diff` are byte-identical for
//! ASCII paths — they differ only in non-ASCII filename rendering, which the shared
//! parser decodes); this module keeps only the jj-specific parsers (changes,
//! bookmarks, op log, …).

use vcs_diff::DiffStat;

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
    /// Short id of the commit it points at.
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
    /// Short id of the commit it points at (empty for a conflicted bookmark).
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
    /// Short commit id of the workspace's working-copy commit.
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
    /// The path the status applies to — the *new* path for a rename/copy.
    pub path: String,
    /// For a rename (`R`) or copy (`C`), the original path; `None` otherwise.
    pub old_path: Option<String>,
}

/// Template used by the change commands: tab-separated, one change per line.
pub(crate) const CHANGE_TEMPLATE: &str = "change_id.short() ++ \"\\t\" ++ commit_id.short() ++ \"\\t\" ++ if(empty, \"true\", \"false\") ++ \"\\t\" ++ description.first_line() ++ \"\\n\"";

/// `jj workspace list -T` template: `name\t<commit>\t<bookmarks,comma-joined>`.
pub(crate) const WORKSPACE_TEMPLATE: &str = "name ++ \"\\t\" ++ target.commit_id().short() ++ \"\\t\" ++ target.local_bookmarks().map(|b| b.name()).join(\",\") ++ \"\\n\"";

/// `jj log -T` template rendering a commit's local bookmark names, comma-joined.
/// Drives `current_bookmark`/`trunk`.
pub(crate) const BOOKMARKS_TEMPLATE: &str = "local_bookmarks.map(|b| b.name()).join(\",\")";

/// `jj bookmark list -a -T` template: `name\t<remote>\t<tracked 1/0>\t<commit>`,
/// one row per local *and* remote-tracking bookmark.
pub(crate) const BOOKMARK_ALL_TEMPLATE: &str = "name ++ \"\\t\" ++ remote ++ \"\\t\" ++ if(tracked, \"1\", \"0\") ++ \"\\t\" ++ if(normal_target, normal_target.commit_id().short(), \"\") ++ \"\\n\"";

/// `jj bookmark list -T` template (no `-a`, so local bookmarks only):
/// `name\t<commit>`, one row per local bookmark. Machine-parsed in place of jj's
/// human-readable default, which interleaves the change id, description, and
/// indented remote-tracking lines that drift with jj's display format.
pub(crate) const BOOKMARK_LIST_TEMPLATE: &str =
    "name ++ \"\\t\" ++ if(normal_target, normal_target.commit_id().short(), \"\") ++ \"\\n\"";

/// `jj log -T` template: `"1"` when the commit has a conflict, else `"0"`.
pub(crate) const CONFLICT_TEMPLATE: &str = "if(conflict, \"1\", \"0\")";

/// `jj log -T` template emitting one short commit id per line — for counting a
/// revset.
pub(crate) const COUNT_TEMPLATE: &str = "commit_id.short() ++ \"\\n\"";

/// `jj log -T` template for [`reachable_bookmarks`](crate::JjApi::reachable_bookmarks):
/// the commit's local bookmark names (space-joined; jj names can't contain spaces)
/// then a tab then the short commit id.
pub(crate) const REACHABLE_BOOKMARKS_TEMPLATE: &str =
    "local_bookmarks.map(|b| b.name()).join(\" \") ++ \"\\t\" ++ commit_id.short() ++ \"\\n\"";

/// Parse `jj --version` output (`jj 0.38.0`) into the shared
/// [`vcs_diff::Version`]: the first dotted-numeric token wins; non-numeric
/// trailers (`-dev`, build hashes) are ignored; a missing patch reads as `0`.
pub(crate) fn parse_jj_version(raw: &str) -> Option<vcs_diff::Version> {
    vcs_diff::parse_dotted_version(raw)
}

/// `jj evolog -T` template. Evolog renders in a *commit* context where the
/// bare keywords (`change_id`, …) don't exist — the `commit.` method form is
/// required. Columns mirror [`CHANGE_TEMPLATE`], so [`parse_changes`] reads it.
pub(crate) const EVOLOG_TEMPLATE: &str = "commit.change_id().short() ++ \"\\t\" ++ commit.commit_id().short() ++ \"\\t\" ++ if(commit.empty(), \"true\", \"false\") ++ \"\\t\" ++ commit.description().first_line() ++ \"\\n\"";

/// `jj op log -T` template: `id\tuser\tstart-time\tdescription`, one row per
/// operation.
pub(crate) const OP_TEMPLATE: &str = "id.short() ++ \"\\t\" ++ user ++ \"\\t\" ++ time.start().format(\"%Y-%m-%dT%H:%M:%S%:z\") ++ \"\\t\" ++ description.first_line() ++ \"\\n\"";

/// `jj op log -T` template for the rollback **divergence probe**: `id\tparent-count`,
/// one row per operation, newest first. A parent count `>= 2` marks a "reconcile
/// divergent operations" merge — the fingerprint jj records when a *concurrent* jj
/// process advanced the operation log, so a rollback walking this can refuse to
/// revert that foreign work (see `Jj::rollback_to`). Kept minimal (no user/time)
/// because the probe only needs the ancestry shape.
pub(crate) const OP_PARENTS_TEMPLATE: &str = "id.short() ++ \"\\t\" ++ parents.len() ++ \"\\n\"";

/// `jj file annotate -T` template: `change-id\tcontent`. Annotate emits one row
/// per source line and separates them itself — no trailing `\n` here, or every
/// row would be double-spaced.
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

/// Parse rows produced by [`OP_TEMPLATE`].
pub(crate) fn parse_operations(output: &str) -> Vec<Operation> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            // `splitn(4)` keeps literal tabs inside the description.
            let mut fields = line.splitn(4, '\t');
            Some(Operation {
                id: fields.next()?.to_string(),
                user: fields.next()?.to_string(),
                time: fields.next()?.to_string(),
                description: fields.next().unwrap_or("").to_string(),
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
            // `splitn(4)` so the trailing description keeps any literal tabs it
            // contains rather than being truncated at the first one.
            let mut fields = line.splitn(4, '\t');
            let change_id = fields.next()?.to_string();
            let commit_id = fields.next()?.to_string();
            let empty = fields.next()? == "true";
            let description = fields.next().unwrap_or("").to_string();
            Some(Change {
                change_id,
                commit_id,
                empty,
                description,
            })
        })
        .collect()
}

/// Parse rows produced by [`BOOKMARK_LIST_TEMPLATE`]: `name\t<commit>`, one row
/// per local bookmark. A row with an empty name contributes nothing.
pub(crate) fn parse_bookmarks(output: &str) -> Vec<Bookmark> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let name = fields.next()?.trim();
            if name.is_empty() {
                return None;
            }
            let target = fields.next().unwrap_or("").trim().to_string();
            Some(Bookmark {
                name: name.to_string(),
                target,
            })
        })
        .collect()
}

/// Parse rows produced by [`BOOKMARK_ALL_TEMPLATE`]:
/// `name\t<remote>\t<tracked 1/0>\t<commit>` per local/remote bookmark. A row
/// whose name field is empty contributes nothing (mirrors [`parse_bookmarks`]).
pub(crate) fn parse_bookmarks_all(output: &str) -> Vec<BookmarkRef> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let name = fields.next()?.trim();
            if name.is_empty() {
                return None;
            }
            let remote = fields.next().unwrap_or("");
            let tracked = fields.next() == Some("1");
            let target = fields.next().unwrap_or("").to_string();
            Some(BookmarkRef {
                name: name.to_string(),
                remote: (!remote.is_empty()).then(|| remote.to_string()),
                target,
                tracked,
            })
        })
        .collect()
}

/// Parse rows produced by [`REACHABLE_BOOKMARKS_TEMPLATE`]:
/// `<name>[ <name>…]\t<commit>`. A commit with several bookmarks yields one
/// [`Bookmark`] per name, all sharing that commit as the target. A row with no
/// bookmark names (empty first field) contributes nothing.
pub(crate) fn parse_reachable_bookmarks(output: &str) -> Vec<Bookmark> {
    let mut out = Vec::new();
    for line in output.lines().filter(|l| !l.is_empty()) {
        let mut fields = line.splitn(2, '\t');
        let names = fields.next().unwrap_or("");
        let target = fields.next().unwrap_or("");
        for name in names.split_whitespace() {
            out.push(Bookmark {
                name: name.to_string(),
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
pub(crate) fn parse_resolve_list(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let path = line.split("  ").next().unwrap_or(line).trim();
            (!path.is_empty()).then(|| path.replace('\\', "/"))
        })
        .collect()
}

/// Parse rows produced by [`WORKSPACE_TEMPLATE`]: `name\t<commit>\t<bookmarks>`,
/// where bookmarks are comma-joined (and may be empty).
pub(crate) fn parse_workspaces(output: &str) -> Vec<Workspace> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let name = fields.next()?.to_string();
            let commit = fields.next().unwrap_or("").to_string();
            let bookmarks = fields
                .next()
                .unwrap_or("")
                .split(',')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
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
pub(crate) fn parse_diff_summary(output: &str) -> Vec<ChangedPath> {
    let normalize = |p: String| p.replace('\\', "/");
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut chars = line.chars();
            let status = chars.next()?;
            // Require the single separating space; the remainder is the raw path.
            let raw = chars.as_str().strip_prefix(' ')?;
            if raw.is_empty() {
                return None;
            }
            let (old_path, path) = if matches!(status, 'R' | 'C') {
                let (old, new) = expand_rename(raw);
                let (old, new) = (normalize(old), normalize(new));
                // A non-brace `R`/`C` path (malformed — jj always renders renames
                // with the `{old => new}` form) expands to `old == new`; don't
                // report that as a self-rename, so `old_path != path` stays a
                // reliable "is this a real rename?" test for consumers.
                ((old != new).then_some(old), new)
            } else {
                (None, normalize(raw.to_string()))
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
/// `(old, new)` full paths. Falls back to `(raw, raw)` when the brace/arrow form
/// isn't present, so a plain path is returned unchanged.
fn expand_rename(raw: &str) -> (String, String) {
    let plain = || (raw.to_string(), raw.to_string());
    // `{`, `}`, and ` => ` are ASCII, so these byte offsets land on char
    // boundaries even when the surrounding path is non-ASCII.
    let (Some(open), Some(close)) = (raw.find('{'), raw.find('}')) else {
        return plain();
    };
    if open >= close {
        return plain();
    }
    let Some(rel) = raw[open..close].find(" => ") else {
        return plain();
    };
    let arrow = open + rel;
    let prefix = &raw[..open];
    let left = &raw[open + 1..arrow];
    let right = &raw[arrow + 4..close];
    let suffix = &raw[close + 1..];
    (
        format!("{prefix}{left}{suffix}"),
        format!("{prefix}{right}{suffix}"),
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
        // RFC-3339 colon offset, as the `%:z` template now emits (A10).
        let out = "abc123\tuser@host\t2026-06-05T10:00:00+02:00\tnew empty commit\n\
                   def456\tuser@host\t2026-06-05T09:59:00+02:00\tdescribe commit\twith tab\n";
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
        let out = "kz\t38\tfalse\tfeat: parser\nkz\t12\ttrue\t\n";
        let changes = parse_changes(out);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].description, "feat: parser");
        assert!(changes[1].empty);
    }

    #[test]
    fn changes_split_tab_fields() {
        let input = "kztuxlro\t38e00654\tfalse\tfeat: stuff\nqpvuntsm\t6ecf997f\ttrue\t\n";
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

    // A literal tab inside the (first-line) description must not truncate it:
    // `splitn(4)` keeps the remainder intact.
    #[test]
    fn changes_keep_tab_in_description() {
        let got = parse_changes("kztuxlro\t38e00654\tfalse\tcol1\tcol2\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].description, "col1\tcol2");
    }

    // A commit carrying several bookmarks fans out to one entry each, all sharing
    // the commit; a bookmark-less row contributes nothing.
    #[test]
    fn reachable_bookmarks_fan_out_per_name() {
        let got = parse_reachable_bookmarks("main feat\tabc123\n\tdef456\n");
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

    #[test]
    fn resolve_list_extracts_paths_before_description() {
        let got = parse_resolve_list(
            "src/a.rs    2-sided conflict\nb.txt    2-sided conflict including 1 deletion\n",
        );
        assert_eq!(got, vec!["src/a.rs".to_string(), "b.txt".to_string()]);
        assert!(parse_resolve_list("").is_empty());
        // OS-native backslash separators (Windows) are normalised to `/`.
        assert_eq!(
            parse_resolve_list("sub\\c.txt    2-sided conflict\n"),
            vec!["sub/c.txt".to_string()]
        );
    }

    #[test]
    fn bookmarks_parse_name_and_commit_from_template() {
        // Rows produced by BOOKMARK_LIST_TEMPLATE: `name\t<commit>`.
        let input = "main\tf5d07685\nfeature\tdeadbeef\n";
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
        // A bookmark with no normal target (e.g. conflicted/deleted) → empty
        // commit field, still a row; an empty name contributes nothing.
        let got = parse_bookmarks("conflicted\t\n\tstray\n");
        assert_eq!(
            got,
            vec![Bookmark {
                name: "conflicted".into(),
                target: String::new()
            }]
        );
    }

    // `parse_bookmarks_all` drops a row whose name field is empty, matching its
    // siblings — no phantom `BookmarkRef { name: "" }` leaks through.
    #[test]
    fn bookmarks_all_drops_empty_name_rows() {
        let input = "main\t\t1\tf5d07685\n\torigin\t1\tdeadbeef\nfeat\torigin\t0\tcafef00d\n";
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
            "the empty-name row must contribute nothing"
        );
    }

    #[test]
    fn workspaces_split_tab_fields_and_bookmarks() {
        let input = "default\te2aa3420\tmain,feature\nws1\t12345678\t\n";
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
        let got = parse_diff_summary("M src/lib.rs\nA new file.txt\nD gone.rs\n");
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].status, 'M');
        assert_eq!(got[1].path, "new file.txt");
        assert!(got[1].old_path.is_none());
        assert_eq!(got[2].status, 'D');
    }

    // jj renders a rename/copy path as `prefix{old => new}suffix` (verified against
    // jj 0.38); it must be expanded into the real new path with the old path
    // captured — not stored raw. A plain `M`/`A`/`D` path is left untouched.
    #[test]
    fn diff_summary_expands_rename_and_copy() {
        let got =
            parse_diff_summary("R {old.rs => new.rs}\nC sub/{a.rs => b.rs}\nM lit{eral}.rs\n");
        assert_eq!(got[0].status, 'R');
        assert_eq!(got[0].path, "new.rs");
        assert_eq!(got[0].old_path.as_deref(), Some("old.rs"));
        assert_eq!(got[1].path, "sub/b.rs");
        assert_eq!(got[1].old_path.as_deref(), Some("sub/a.rs"));
        // A literal `{...}` in a non-rename path (no ` => `) is not mis-expanded.
        assert_eq!(got[2].path, "lit{eral}.rs");
        assert!(got[2].old_path.is_none());
    }

    // jj `--summary` emits OS-native separators (backslashes on Windows); paths are
    // normalised to forward slashes to match the `--git` diff and the git backend.
    #[test]
    fn diff_summary_normalises_backslash_separators() {
        let got = parse_diff_summary("M deep\\nested\\f.rs\nR win\\{a.rs => b.rs}\n");
        assert_eq!(got[0].path, "deep/nested/f.rs");
        assert_eq!(got[1].path, "win/b.rs");
        assert_eq!(got[1].old_path.as_deref(), Some("win/a.rs"));
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

    proptest! {
        #[test]
        fn parsers_never_panic_on_arbitrary_text(s in any::<String>()) {
            let _ = parse_changes(&s);
            let _ = parse_operations(&s);
            let _ = parse_annotate(&s);
            let _ = parse_bookmarks(&s);
            let _ = parse_bookmarks_all(&s);
            let _ = parse_reachable_bookmarks(&s);
            let _ = parse_resolve_list(&s);
            let _ = parse_workspaces(&s);
            let _ = parse_diff_summary(&s);
            let _ = parse_diff_stat(&s);
            let _ = parse_jj_version(&s);
            let _ = expand_rename(&s);
        }

        #[test]
        fn parsers_never_panic_on_structured_text(s in structured_doc()) {
            let _ = parse_diff_summary(&s);
            let _ = parse_changes(&s);
            let _ = parse_bookmarks_all(&s);
        }

        // expand_rename returns the raw verbatim for a non-brace input (its
        // documented identity for the no-rename case).
        #[test]
        fn expand_rename_is_identity_without_braces(s in "[a-zé/ ]{0,20}") {
            prop_assume!(!s.contains('{') && !s.contains('}'));
            prop_assert_eq!(expand_rename(&s), (s.clone(), s));
        }
    }
}
