//! Typed results from `tea … --output csv` and the positional DSV parsers.
//!
//! # Why CSV, not JSON
//!
//! `tea`'s table-backed **list** commands (`login list`, `pr list`, `issues list`,
//! `releases list`) do **not** support `--output json` on the crate's declared
//! floor. On `tea` 0.9.x the format dispatch (`modules/print/table.go`,
//! `func (t *table) print`) has no `json` case, so `--output json` falls through to
//! the `default` arm and prints `unknown output type 'json', available types are:
//! …` to **stdout with exit code 0** — after which a JSON parser rejects it (empirically
//! seen in the sibling F# port against live `tea` 0.9.2, and confirmed here by reading
//! tea's source at the `v0.9.2` tag). `json` support was only added in `tea` 0.10.0;
//! newer `tea` (≥ 0.10) makes the unknown-format arm exit non-zero instead. So the old
//! `--output json` path was silently broken on `tea` 0.9.x (this crate's floor) while
//! passing against a newer `tea` — exactly the drift the `#[ignore]` real-`tea` tests in
//! `tests/cli.rs` exist to catch, missed because the scheduled lane only ran the latest
//! `tea`.
//!
//! `--output csv` is supported across the **whole** `tea` 0.9+ line (the `csv` arm has
//! existed throughout), so every read op here asks for `csv` and parses the quoted DSV
//! positionally.
//!
//! # tea's CSV wire format (two dialects, one RFC-4180 parser)
//!
//! tea's `outputDsv` changed shape across the versions this crate supports:
//!
//! - **0.9.x–0.13.x (naive):** each field is wrapped in `"` and rows are joined by the
//!   three-character sequence `","`, with **no escaping** — a header line then one line
//!   per row (an empty list is a header-only line, never nothing to parse).
//! - **0.14.x (`encoding/csv`):** proper RFC-4180 — only fields that need it are quoted,
//!   an embedded `"` is doubled (`""`), and a field containing a newline is quoted and
//!   spans physical lines.
//!
//! A single RFC-4180 reader ([`parse_csv_records`]) handles **both**: the naive dialect is
//! itself valid RFC-4180 for values without an embedded `"` (each field is simply always
//! quoted), and a quoted field's internal newline is read as part of the field either way,
//! so a multi-line issue body round-trips. The one value the naive dialect can corrupt is a
//! field containing a literal `"` (unescaped on 0.9.x) — a rare, tea-side limitation of that
//! old format, not something this parser can recover.
//!
//! Parsing is pure, so the unit tests are hermetic; the `#[ignore]` real-`tea` tests in
//! `tests/cli.rs` are the definitive live check that tea's real output still matches these
//! positional column maps.

use processkit::{Error, Result};

use crate::BINARY;

/// Parse `tea --version` output (`tea version 0.9.2` / `🍵 tea version 0.9.2`) into
/// the shared [`vcs_diff::Version`]: the first dotted-numeric token wins, so any
/// build/emoji/commit trailer is ignored. `None` when the banner carries no version
/// token. Reuses the same tolerant parser `vcs-git`/`vcs-jj` gate on, so the CLIs
/// share one version-parsing contract.
pub(crate) fn parse_tea_version(raw: &str) -> Option<vcs_diff::Version> {
    vcs_diff::parse_dotted_version(raw)
}

/// A pull request (`tea pr list --output csv`), flattened from tea's table
/// columns (`index`/`title`/`state`/`head`/`base`/`url`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PullRequest {
    /// PR number (tea's `index` column).
    pub number: u64,
    /// PR title.
    pub title: String,
    /// State, e.g. `"open"`, `"closed"`, `"merged"` — tea folds the merge flag
    /// into this column (a merged PR reads `"merged"`, not `"closed"`).
    pub state: String,
    /// Whether the PR has been merged — derived from `state == "merged"` (tea has
    /// no separate merged column).
    pub merged: bool,
    /// Source (head) branch name — a **flat** branch. tea renders a fork PR's head as
    /// `owner:branch`; the parser strips the `owner:` prefix so this is always the bare
    /// branch (matching GitHub/GitLab; the fork owner isn't modelled).
    pub head_branch: String,
    /// Target (base) branch name (tea's `base` column, a flat branch name).
    pub base_branch: String,
    /// Web URL (tea's `url` column).
    pub url: String,
}

/// An issue (`tea issues list --output csv`). `issue_view` is synthesized by paging
/// this same list (tea's single-issue view renders Markdown and ignores `--output`),
/// so both list and view flatten into this one struct.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Issue {
    /// Issue number (tea's `index`).
    pub number: u64,
    /// Issue title.
    pub title: String,
    /// State, e.g. `"open"`, `"closed"`.
    pub state: String,
    /// Issue body / description.
    pub body: String,
    /// Web URL (tea's `url`).
    pub url: String,
}

/// A release (`tea releases list --output csv`), flattened from tea's fixed
/// release-table columns (`Tag-Name`/`Title`/`Published At`/`Status`/`Tar URL`).
/// **`tea releases` exposes no web-page URL** (only a tar download URL, which we
/// deliberately don't surface), so [`url`](Release::url) is always empty for Gitea —
/// see the field doc.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Release {
    /// Git tag the release points at (tea's `Tag-Name` column).
    pub tag: String,
    /// Release title (tea's `Title` column).
    pub title: String,
    /// Publish timestamp, e.g. `"2023-07-26T13:02:36Z"` (tea's `Published At`
    /// column); empty for an unpublished draft.
    pub published_at: String,
    /// Whether the release is a draft (derived from tea's `Status` column).
    pub draft: bool,
    /// Whether the release is a pre-release (derived from tea's `Status` column).
    pub prerelease: bool,
    /// **Always empty for Gitea.** `tea releases list` has no release-page URL
    /// column (only a tar download URL, intentionally not surfaced here).
    pub url: String,
}

/// Normalise tea's PR **head** column to a flat branch name. For a **fork** PR,
/// tea's `formatPRHead` renders `owner:branch` (and `<marker>:branch` for a deleted
/// fork), unlike the plain branch it renders for a same-repo PR — and unlike
/// GitHub's/GitLab's flat head. Since a git ref can't contain `:`, splitting on the
/// first `:` recovers the branch (the fork owner isn't modelled on the flat DTO,
/// matching the other backends); a same-repo head with no `:` is returned as-is. (M26)
fn strip_fork_owner(head: &str) -> String {
    match head.split_once(':') {
        Some((_owner, branch)) => branch.to_string(),
        None => head.to_string(),
    }
}

/// Parse a tea table cell holding an issue/PR index (a plain number after DSV
/// unquoting, e.g. `4`) into a `u64`, mapping a non-numeric value to [`Error::Parse`].
fn parse_index(value: &str) -> Result<u64> {
    value
        .trim()
        .parse()
        .map_err(|_| Error::parse(BINARY, format!("expected a numeric index, got {value:?}")))
}

/// The message `tea` prints when asked for an `--output` format it does not support.
/// On the crate's floor (`tea` 0.9.x) this goes to **stdout with exit code 0**, so a
/// read op would otherwise treat it as a silently-empty list; detect it and turn it
/// into a loud [`Error::Parse`] instead (a newer `tea` exits non-zero, which the
/// `try_parse`/`ensure_success` layer already surfaces as an error). tea has spelled
/// the prefix with either a leading `'` (0.9/0.10) or `"` (0.14) quote and, in some
/// builds, wrapped the whole message in `"`, so match the version-stable prefix after
/// an optional leading double-quote.
fn reject_unknown_output(output: &str) -> Result<()> {
    let first_line = output.lines().next().unwrap_or("").trim_start();
    let probe = first_line.strip_prefix('"').unwrap_or(first_line);
    if probe.starts_with("unknown output type") {
        return Err(Error::parse(
            BINARY,
            format!("tea rejected the requested --output format (contract drift): {first_line}"),
        ));
    }
    Ok(())
}

/// Split tea's DSV output into records of fields with a single RFC-4180 reader that
/// handles both tea dialects (see the module docs): quoted fields, an embedded `""`
/// escaped quote, and newlines **inside** a quoted field (a multi-line issue body).
/// A record ends at a newline that is not inside quotes; `\r` outside quotes is
/// dropped (CRLF tolerance). The final record needs no trailing newline. Empty input
/// yields no records.
fn parse_csv_records(input: &str) -> Vec<Vec<String>> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    // Whether the current physical span has produced any token yet — distinguishes a
    // genuine trailing record from the position just after a record-terminating `\n`.
    let mut pending = false;

    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => {
                in_quotes = true;
                pending = true;
            }
            ',' => {
                record.push(std::mem::take(&mut field));
                pending = true;
            }
            '\r' => {}
            '\n' => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
                pending = false;
            }
            _ => {
                field.push(c);
                pending = true;
            }
        }
    }
    if pending || !field.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

/// Whether a parsed record is a blank line (no real cells) rather than a data row —
/// skipped so a stray blank or whitespace-only line never becomes a bogus row. A real
/// data row always carries a non-whitespace key column (a numeric index, a tag, a login
/// name), so trimming here never drops genuine data.
fn is_blank_record(record: &[String]) -> bool {
    record.iter().all(|field| field.trim().is_empty())
}

/// Cell at `col` in a positionally-parsed row, or `""` when the row has fewer columns
/// than requested (a trimmed/older tea) — mirrors the previous serde `#[serde(default)]`
/// tolerance for optional trailing columns.
fn cell(row: &[String], col: usize) -> &str {
    row.get(col).map(String::as_str).unwrap_or("")
}

/// The data rows of a DSV table: reject the unknown-output-type sentinel, split into
/// records, drop the header record, and skip blank lines. The header is always present
/// (tea prints it even for an empty list), so an empty or header-only table yields no
/// data rows.
fn data_rows(csv: &str) -> Result<Vec<Vec<String>>> {
    reject_unknown_output(csv)?;
    let mut records = parse_csv_records(csv);
    if records.is_empty() {
        return Ok(Vec::new());
    }
    // Drop the header record; keep the rest, minus any blank line.
    let rows = records.split_off(1);
    Ok(rows.into_iter().filter(|r| !is_blank_record(r)).collect())
}

/// Parse `tea pr list --output csv` into the flattened [`PullRequest`]s. Columns are
/// `index,title,state,head,base,url` (the `--fields` order this crate requests).
pub(crate) fn parse_pr_list(csv: &str) -> Result<Vec<PullRequest>> {
    data_rows(csv)?
        .iter()
        .map(|row| {
            let state = cell(row, 2);
            Ok(PullRequest {
                number: parse_index(cell(row, 0))?,
                title: cell(row, 1).to_string(),
                merged: state.eq_ignore_ascii_case("merged"),
                state: state.to_string(),
                head_branch: strip_fork_owner(cell(row, 3)),
                base_branch: cell(row, 4).to_string(),
                url: cell(row, 5).to_string(),
            })
        })
        .collect()
}

/// Parse `tea issues list --output csv` into the flattened [`Issue`]s. Columns are
/// `index,title,state,body,url` (the `--fields` order this crate requests).
pub(crate) fn parse_issue_list(csv: &str) -> Result<Vec<Issue>> {
    data_rows(csv)?
        .iter()
        .map(|row| {
            Ok(Issue {
                number: parse_index(cell(row, 0))?,
                title: cell(row, 1).to_string(),
                state: cell(row, 2).to_string(),
                body: cell(row, 3).to_string(),
                url: cell(row, 4).to_string(),
            })
        })
        .collect()
}

/// Parse `tea releases list --output csv` into the flattened [`Release`]s. tea's fixed
/// release table has no `--fields` flag; the columns are, in order, `Tag-Name`,
/// `Title`, `Published At`, `Status`, `Tar URL`. A data row with an empty tag is a real
/// parse failure (drift), not a silent empty tag.
pub(crate) fn parse_release_list(csv: &str) -> Result<Vec<Release>> {
    data_rows(csv)?
        .iter()
        .map(|row| {
            let tag = cell(row, 0);
            if tag.is_empty() {
                return Err(Error::parse(
                    BINARY,
                    "release row is missing its tag column".to_string(),
                ));
            }
            let status = cell(row, 3);
            Ok(Release {
                tag: tag.to_string(),
                title: cell(row, 1).to_string(),
                published_at: cell(row, 2).to_string(),
                draft: status.eq_ignore_ascii_case("draft"),
                prerelease: status.eq_ignore_ascii_case("prerelease"),
                // tea's release table carries no web-page URL column.
                url: String::new(),
            })
        })
        .collect()
}

/// Whether at least one login is configured, from `tea login list --output csv` — one
/// data row per login (columns `Name,URL,SSHHost,User,Default`), so a header-only or
/// empty table means "not logged in". The unknown-output-type sentinel is a loud error,
/// not a silent `false`.
pub(crate) fn parse_login_present(csv: &str) -> Result<bool> {
    Ok(!data_rows(csv)?.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // An empty (some tea builds) or header-only (the usual empty-list shape) table is
    // an empty list, not a serde-style error — this is what lets `pr_view`/`issue_view`
    // detect an empty (past-the-end) page as a clean absence.
    #[test]
    fn empty_and_header_only_parse_as_an_empty_list() {
        for blank in ["", "   ", "\n", " \r\n "] {
            assert!(parse_pr_list(blank).unwrap().is_empty());
            assert!(parse_issue_list(blank).unwrap().is_empty());
            assert!(parse_release_list(blank).unwrap().is_empty());
            assert!(!parse_login_present(blank).unwrap());
        }
        // Header-only (tea prints the header even for zero rows).
        let pr_header = r#""index","title","state","head","base","url""#;
        assert!(parse_pr_list(pr_header).unwrap().is_empty());
        let login_header = r#""Name","URL","SSHHost","User","Default""#;
        assert!(!parse_login_present(login_header).unwrap());
    }

    // The unknown-output-type diagnostic tea prints (with exit 0 on 0.9.x) must become a
    // loud parse error — never a silently-empty list that would hide the format regression.
    #[test]
    fn unknown_output_type_is_a_parse_error() {
        // 0.9.x shape (leading `'`, no wrapping quote), 0.10 shape (wrapped in `"`),
        // and 0.14 shape (leading `"` after %q) — all must be rejected.
        for sentinel in [
            "unknown output type 'json', available types are:\n- csv: comma-separated values\n",
            "\"unknown output type 'json', available types are:\n- csv: comma-separated values\n",
            "unknown output type \"json\", available types are: csv, simple, table, tsv, yaml, json",
        ] {
            assert!(matches!(
                parse_pr_list(sentinel).unwrap_err(),
                Error::Parse { .. }
            ));
            assert!(matches!(
                parse_issue_list(sentinel).unwrap_err(),
                Error::Parse { .. }
            ));
            assert!(matches!(
                parse_release_list(sentinel).unwrap_err(),
                Error::Parse { .. }
            ));
            assert!(matches!(
                parse_login_present(sentinel).unwrap_err(),
                Error::Parse { .. }
            ));
        }
    }

    proptest! {
        // The DSV parsers must only ever return Ok/Err on arbitrary or malformed bytes —
        // never panic.
        #[test]
        fn parsers_never_panic_on_arbitrary_input(s in ".*") {
            let _ = parse_pr_list(&s);
            let _ = parse_issue_list(&s);
            let _ = parse_release_list(&s);
            let _ = parse_login_present(&s);
            let _ = parse_index(&s);
            let _ = parse_csv_records(&s);
        }

        // A well-formed table row with arbitrary quoted string cells exercises the row
        // mapping — notably `parse_index` on a non-numeric `index` — which must surface a
        // structured Err, not crash. (Cells can't contain a raw `"` here; that is tea's
        // own naive-dialect limitation, not this parser's.)
        #[test]
        fn pr_list_tolerates_arbitrary_row_values(
            index in "[^\"\r\n]*", title in "[^\"\r\n]*", state in "[^\"\r\n]*",
            head in "[^\"\r\n]*", base in "[^\"\r\n]*", url in "[^\"\r\n]*",
        ) {
            let csv = format!(
                "\"index\",\"title\",\"state\",\"head\",\"base\",\"url\"\n\
                 \"{index}\",\"{title}\",\"{state}\",\"{head}\",\"{base}\",\"{url}\"\n"
            );
            let _ = parse_pr_list(&csv);
        }
    }

    // The low-level RFC-4180 reader: a quoted field may hold the delimiter, an escaped
    // `""` quote, and an embedded newline (a multi-line field), all read intact.
    #[test]
    fn csv_reader_handles_quotes_commas_and_newlines() {
        let input = "\"a\",\"b, still b\",\"has \"\"quote\"\"\",\"line1\nline2\"\n";
        let records = parse_csv_records(input);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0],
            vec![
                "a".to_string(),
                "b, still b".to_string(),
                "has \"quote\"".to_string(),
                "line1\nline2".to_string(),
            ]
        );
    }

    // The naive 0.9.x dialect (every field wrapped, joined by `","`, CRLF line ends) is
    // itself valid RFC-4180 for quote-free values, so the same reader parses it.
    #[test]
    fn csv_reader_handles_the_naive_dialect() {
        let input = "\"index\",\"title\"\r\n\"7\",\"Add X\"\r\n";
        let records = parse_csv_records(input);
        assert_eq!(records.len(), 2);
        assert_eq!(records[1], vec!["7".to_string(), "Add X".to_string()]);
    }

    // `tea pr list --output csv`: columns `index,title,state,head,base,url`.
    #[test]
    fn parses_pr_list_row() {
        let csv = "\"index\",\"title\",\"state\",\"head\",\"base\",\"url\"\n\
                   \"7\",\"Add X\",\"open\",\"feat/x\",\"main\",\"https://gitea/pr/7\"\n";
        let prs = parse_pr_list(csv).expect("parse prs");
        assert_eq!(prs.len(), 1);
        assert_eq!(
            prs[0],
            PullRequest {
                number: 7,
                title: "Add X".into(),
                state: "open".into(),
                merged: false,
                head_branch: "feat/x".into(),
                base_branch: "main".into(),
                url: "https://gitea/pr/7".into(),
            }
        );
    }

    // M26: a fork PR's head is rendered `owner:branch` by tea; the parser strips the
    // `owner:` prefix to a flat branch (a same-repo head has no `:` and is unchanged).
    #[test]
    fn fork_pr_head_strips_owner_prefix() {
        let csv = "\"index\",\"title\",\"state\",\"head\",\"base\",\"url\"\n\
                   \"8\",\"From a fork\",\"open\",\"alice:feature\",\"main\",\"https://gitea/pr/8\"\n\
                   \"9\",\"Same repo\",\"open\",\"topic/y\",\"main\",\"https://gitea/pr/9\"\n";
        let prs = parse_pr_list(csv).expect("parse prs");
        assert_eq!(prs[0].head_branch, "feature", "fork owner stripped");
        assert_eq!(prs[1].head_branch, "topic/y", "same-repo head unchanged");
        // The direct helper: deleted-fork marker prefix also strips to the branch;
        // degenerate inputs (empty, no colon) pass through unchanged.
        assert_eq!(strip_fork_owner("delete:old"), "old");
        assert_eq!(strip_fork_owner("plain"), "plain");
        assert_eq!(strip_fork_owner(""), "");
    }

    // tea folds the merge flag into the `state` column: a merged PR reads
    // `state="merged"`, from which `merged` is derived.
    #[test]
    fn pr_state_merged_derives_the_flag() {
        let csv = "\"index\",\"title\",\"state\",\"head\",\"base\",\"url\"\n\
                   \"9\",\"done\",\"merged\",\"f\",\"main\",\"u\"\n";
        let prs = parse_pr_list(csv).expect("parse prs");
        assert_eq!(prs[0].number, 9);
        assert!(prs[0].merged);
        assert_eq!(prs[0].state, "merged");
    }

    // A non-numeric `index` cell is a real parse failure, not a silent `0` that
    // `pr_view` could then "find".
    #[test]
    fn pr_non_numeric_index_is_a_parse_error() {
        let csv = "\"index\",\"title\",\"state\"\n\"x\",\"t\",\"open\"\n";
        match parse_pr_list(csv).unwrap_err() {
            Error::Parse { .. } => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    // `tea issues list --output csv`: columns `index,title,state,body,url`, and a
    // multi-line body (tea quotes it) round-trips through the reader.
    #[test]
    fn parses_issue_list_row_with_multiline_body() {
        let csv = "\"index\",\"title\",\"state\",\"body\",\"url\"\n\
                   \"12\",\"Bug\",\"open\",\"line1\nline2\",\"https://gitea/issues/12\"\n";
        let issues = parse_issue_list(csv).expect("parse issues");
        assert_eq!(issues.len(), 1);
        assert_eq!(
            issues[0],
            Issue {
                number: 12,
                title: "Bug".into(),
                state: "open".into(),
                body: "line1\nline2".into(),
                url: "https://gitea/issues/12".into(),
            }
        );
    }

    // A column trim (body/url absent) must still parse via the empty-cell default.
    #[test]
    fn issue_list_tolerates_trimmed_columns() {
        let csv = "\"index\",\"title\",\"state\"\n\"4\",\"wip\",\"open\"\n";
        let issues = parse_issue_list(csv).expect("parse issues");
        assert_eq!(issues[0].number, 4);
        assert_eq!(issues[0].body, "");
        assert_eq!(issues[0].url, "");
    }

    // `tea releases list --output csv`: fixed columns `Tag-Name,Title,Published At,
    // Status,Tar URL`, and NO release-page URL (so `url` is empty).
    #[test]
    fn parses_release_list_row() {
        let csv = "\"Tag-Name\",\"Title\",\"Published At\",\"Status\",\"Tar URL\"\n\
                   \"0.1\",\"First\",\"2023-07-26T13:02:36Z\",\"released\",\"https://gitea/0.1.tar.gz\"\n";
        let releases = parse_release_list(csv).expect("parse releases");
        assert_eq!(releases.len(), 1);
        assert_eq!(
            releases[0],
            Release {
                tag: "0.1".into(),
                title: "First".into(),
                published_at: "2023-07-26T13:02:36Z".into(),
                draft: false,
                prerelease: false,
                url: String::new(), // tea exposes no release-page URL
            }
        );
    }

    // A draft release: tea's `Status` column is "draft", and `Published At` is empty.
    #[test]
    fn release_status_drives_draft_flag() {
        let csv = "\"Tag-Name\",\"Title\",\"Published At\",\"Status\",\"Tar URL\"\n\
                   \"v2\",\"Two\",\"\",\"draft\",\"\"\n";
        let releases = parse_release_list(csv).expect("parse releases");
        assert_eq!(releases[0].tag, "v2");
        assert!(releases[0].draft);
        assert_eq!(releases[0].published_at, "");
        assert!(!releases[0].prerelease);
    }

    // A prerelease: `Status` = "prerelease" sets the prerelease flag only.
    #[test]
    fn release_status_drives_prerelease_flag() {
        let csv = "\"Tag-Name\",\"Title\",\"Published At\",\"Status\",\"Tar URL\"\n\
                   \"v3-rc1\",\"RC\",\"2026-01-02T03:04:05Z\",\"prerelease\",\"\"\n";
        let releases = parse_release_list(csv).expect("parse releases");
        assert!(releases[0].prerelease);
        assert!(!releases[0].draft);
    }

    // A release row with an empty tag column is a real parse failure, not a silent
    // empty tag.
    #[test]
    fn release_missing_tag_is_a_parse_error() {
        let csv = "\"Tag-Name\",\"Title\"\n\"\",\"no tag\"\n";
        match parse_release_list(csv).unwrap_err() {
            Error::Parse { .. } => {}
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    // auth_status counts login data rows; a header-only table means "not logged in",
    // one or more rows means "logged in".
    #[test]
    fn login_rows_drive_auth_status() {
        let none = "\"Name\",\"URL\",\"SSHHost\",\"User\",\"Default\"\n";
        assert!(!parse_login_present(none).unwrap());
        let some = "\"Name\",\"URL\",\"SSHHost\",\"User\",\"Default\"\n\
                    \"gitea\",\"https://gitea\",\"\",\"me\",\"true\"\n";
        assert!(parse_login_present(some).unwrap());
    }
}
