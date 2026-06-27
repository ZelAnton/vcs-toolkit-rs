#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
//! `vcs-diff` — the shared git-format unified-diff model and parser for the
//! [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) workspace.
//!
//! `git diff` and `jj diff --git` emit the same **git-format unified diffs**
//! (byte-identical for ASCII paths; they differ only in non-ASCII filename
//! rendering — git octal-C-quotes by default, jj writes raw UTF-8 — both of which
//! the parser decodes), so `vcs-git` and `vcs-jj` share one model and one parser
//! here rather than each carrying a copy that could silently drift. This is the foundational
//! crate both depend on: **std only**, no async, no subprocess — pure data types
//! and pure functions over text the wrapper crates obtained elsewhere.
//!
//! # The surface
//!
//! - **[`parse_diff`]** — the entry point. Turns git-format diff text into one
//!   [`FileDiff`] per file; the same call serves `git diff` and `jj diff --git`
//!   output alike. Pure and total: arbitrary CLI bytes in, never a panic.
//! - **[`FileDiff`]** — one file's entry: its [`ChangeKind`], the
//!   forward-slash-normalised `path` (and `old_path` for a rename), the
//!   [`Hunk`]s, and the verbatim `raw` section for callers that display text.
//! - **[`Hunk`]** — a single `@@ … @@` block: the old/new line ranges, the
//!   section heading, and a body of **[`DiffLine`]**s
//!   ([`Context`](DiffLine::Context) / [`Added`](DiffLine::Added) /
//!   [`Removed`](DiffLine::Removed)), each with its leading marker and line
//!   terminator stripped.
//! - **[`ChangeKind`]** — how the file changed: [`Added`](ChangeKind::Added) /
//!   [`Modified`](ChangeKind::Modified) / [`Deleted`](ChangeKind::Deleted) /
//!   [`Renamed`](ChangeKind::Renamed).
//! - **[`DiffStat`]** — the aggregate `files_changed`/`insertions`/`deletions`
//!   shape both `git diff --shortstat` and `jj diff --stat` parse into.
//! - **[`DiffSpec`]** — the diff request a wrapper's `diff`/`diff_text` takes
//!   ([`WorkingTree`](DiffSpec::WorkingTree) / [`Rev`](DiffSpec::Rev)). This
//!   crate defines it (so the backends share one type) but has no method that
//!   consumes it.
//! - **[`Version`]** + **[`parse_dotted_version`]** — a numeric
//!   `major.minor.patch` (it `Ord`s, so a caller can gate on a minimum) read
//!   tolerantly from a `<tool> --version` banner.
//!
//! The wrapper crates re-export these (e.g. `vcs_git::FileDiff`,
//! `vcs_git::parse_diff`, `vcs_git::GitVersion`), so consumers rarely name this
//! crate directly.
//!
//! # Recipes
//!
//! Parse a one-file modify diff and read the structured result — pure, so this
//! runs as written:
//!
//! ```rust
//! use vcs_diff::{parse_diff, ChangeKind, DiffLine};
//!
//! let text = "\
//! diff --git a/f b/f
//! --- a/f
//! +++ b/f
//! @@ -1,2 +1,2 @@ fn main()
//!  ctx
//! -old
//! +new
//! ";
//! let files = parse_diff(text);
//! assert_eq!(files.len(), 1);
//! assert_eq!(files[0].change, ChangeKind::Modified);
//! assert_eq!(files[0].path, "f");
//!
//! let hunk = &files[0].hunks[0];
//! assert_eq!((hunk.old_start, hunk.new_start), (1, 1));
//! assert_eq!(hunk.section, "fn main()");
//! assert_eq!(hunk.lines, vec![
//!     DiffLine::Context("ctx".into()),
//!     DiffLine::Removed("old".into()),
//!     DiffLine::Added("new".into()),
//! ]);
//! ```
//!
//! # Features
//!
//! - **`serde`** — derives `serde::Serialize` on every model type
//!   ([`FileDiff`], [`Hunk`], [`DiffLine`], [`ChangeKind`], [`DiffStat`],
//!   [`Version`]) so a caller can emit the parsed diff as JSON.

mod diff;
mod version;

pub use diff::{ChangeKind, DiffLine, DiffSpec, DiffStat, FileDiff, Hunk, parse_diff};
pub use version::{Version, parse_dotted_version};
