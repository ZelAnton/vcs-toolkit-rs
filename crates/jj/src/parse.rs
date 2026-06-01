//! Pure parsers for jj output. No process execution, so these tests are
//! hermetic and run on CI.

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

/// A workspace from `jj workspace list` (rendered with [`WORKSPACE_TEMPLATE`]).
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
/// and the path it applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ChangedPath {
    /// Status letter (`M` modified, `A` added, `D` deleted, …).
    pub status: char,
    /// The path the status applies to.
    pub path: String,
}

/// Aggregate line/file counts from the `jj diff --stat` summary footer.
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

/// Template used by the change commands: tab-separated, one change per line.
pub(crate) const CHANGE_TEMPLATE: &str = "change_id.short() ++ \"\\t\" ++ commit_id.short() ++ \"\\t\" ++ if(empty, \"true\", \"false\") ++ \"\\t\" ++ description.first_line() ++ \"\\n\"";

/// `jj workspace list -T` template: `name\t<commit>\t<bookmarks,comma-joined>`.
pub(crate) const WORKSPACE_TEMPLATE: &str = "name ++ \"\\t\" ++ target.commit_id().short() ++ \"\\t\" ++ target.local_bookmarks().map(|b| b.name()).join(\",\") ++ \"\\n\"";

/// `jj log -T` template rendering a commit's local bookmark names, comma-joined.
/// Drives `current_bookmark`/`trunk`.
pub(crate) const BOOKMARKS_TEMPLATE: &str = "local_bookmarks.map(|b| b.name()).join(\",\")";

/// `jj log -T` template: `"1"` when the commit has a conflict, else `"0"`.
pub(crate) const CONFLICT_TEMPLATE: &str = "if(conflict, \"1\", \"0\")";

/// `jj log -T` template emitting one short commit id per line — for counting a
/// revset.
pub(crate) const COUNT_TEMPLATE: &str = "commit_id.short() ++ \"\\n\"";

/// Parse rows produced by [`CHANGE_TEMPLATE`].
pub(crate) fn parse_changes(output: &str) -> Vec<Change> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut fields = line.split('\t');
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

/// Parse `jj bookmark list` default output. Local bookmark lines look like
/// `name: <change_id> <commit_id> <description>`; remote-tracking lines are
/// indented and skipped.
pub(crate) fn parse_bookmarks(output: &str) -> Vec<Bookmark> {
    output
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with(char::is_whitespace))
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            // Tokens after the name are `<change_id> <commit_id> …`; take the
            // commit id (2nd), falling back to whatever is present.
            let mut tokens = rest.split_whitespace();
            let target = tokens
                .nth(1)
                .or_else(|| rest.split_whitespace().next())
                .unwrap_or("")
                .to_string();
            Some(Bookmark {
                name: name.trim().to_string(),
                target,
            })
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

/// Parse `jj diff --summary`: each line is `<status-letter> <path>`.
pub(crate) fn parse_diff_summary(output: &str) -> Vec<ChangedPath> {
    output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut chars = line.chars();
            let status = chars.next()?;
            // Skip the single separating space; the remainder is the raw path.
            let path = chars.as_str().strip_prefix(' ').unwrap_or(chars.as_str());
            if path.is_empty() {
                return None;
            }
            Some(ChangedPath {
                status,
                path: path.to_string(),
            })
        })
        .collect()
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

    #[test]
    fn bookmarks_parse_name_and_commit_and_skip_remotes() {
        let input = "main: pzlznprr f5d07685 feat(process): job-backed spawn\n  @origin: pzlznprr f5d07685 feat(process)\nfeature: abcd1234 deadbeef wip\n";
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
        assert_eq!(got[2].status, 'D');
    }

    #[test]
    fn diff_stat_parses_footer_among_per_file_lines() {
        let input = "README.md | 10 +++---\n\
                     src/lib.rs | 4 +-\n\
                     4 files changed, 157 insertions(+), 137 deletions(-)\n";
        assert_eq!(
            parse_diff_stat(input),
            DiffStat {
                files_changed: 4,
                insertions: 157,
                deletions: 137
            }
        );
        assert_eq!(parse_diff_stat(""), DiffStat::default());
    }
}
