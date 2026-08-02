//! The plumbing behind the two conflict tools —
//! [`repo_conflict_regions`](crate::VcsMcpServer::repo_conflict_regions) and
//! [`repo_resolve_conflict`](crate::VcsMcpServer::repo_resolve_conflict): the
//! repo-containment path guard, the JSON wire shapes, and the git/jj dispatch
//! over `vcs_git::conflict` / `vcs_jj::conflict`. Crate-internal
//! (`pub(crate)`); the public surface is the two `#[tool]` methods in
//! `repo_tools`.
//!
//! # Why the working copy, not `repo_show_file`
//!
//! Conflict markers are **materialized in the working copy**, and on git they
//! exist *only* there: a conflicted path has no stage-0 index entry, so
//! `git show :<path>` fails outright and `git show HEAD:<path>` returns `HEAD`'s
//! clean version with no markers at all (verified against git 2.55). Routing
//! these tools through the `show_file` facade would therefore report "no
//! conflicts" for a file `repo_conflicts` lists as conflicted — a silent, badly
//! wrong answer. On jj `jj file show -r @` *does* return the materialized markers
//! (verified against jj 0.38, byte-identical to the file on disk), but reading
//! the working copy on both backends keeps one semantics, and — decisively —
//! makes the read tool report exactly the bytes the resolve tool will overwrite.
//!
//! Because neither tool's read spawns a backend command, the read tool records no
//! jj operation and is honestly `readOnlyHint` (see `repo_info`'s precedent);
//! `repo_resolve_conflict` still spawns (its conflicted-path check and, on git,
//! the finalizing `git add`) and is write-gated and `destructiveHint`.

use std::path::{Component, Path, PathBuf};

use rmcp::ErrorData;
use rmcp::model::CallToolResult;
use vcs_core::processkit::{Error as VcsError, ErrorReason};
use vcs_core::{BackendKind, vcs_git, vcs_jj};

use crate::output::ok_json;
use crate::params::ConflictSideArg;

/// An agent-supplied path, validated to stay inside the repository.
pub(crate) struct RepoPath {
    /// Repo-relative — the shape `Repo::conflicted_files`/`mark_resolved` use.
    pub(crate) rel: PathBuf,
    /// Absolute, for the filesystem read/write.
    pub(crate) abs: PathBuf,
}

/// Resolve an agent-supplied repo-relative `path` against the repository `root`,
/// refusing anything that could address a file outside it.
///
/// Every path component must be [`Component::Normal`]: that rejects an absolute
/// path, a Windows drive prefix (`C:\…`), a bare root (`/…`), `.`, and — the one
/// that matters — any `..` traversal. Both `/` and `\` are accepted as separators
/// on Windows (`Path::components` splits on both there), while on Unix a
/// backslash stays an ordinary filename character, as it must.
///
/// This is the guard the *facade* would otherwise provide: every other `repo_*`
/// tool reaches the filesystem through a `git`/`jj` subprocess run inside the
/// repo, which confines the path for us. These two touch the filesystem directly,
/// so the confinement has to be explicit. (A symlink *inside* the repo pointing
/// out of it is not caught here — the same exposure git itself has when it
/// materializes a conflicted working tree.)
pub(crate) fn repo_path(root: &Path, path: &str) -> Result<RepoPath, ErrorData> {
    let refuse = |why: &str| {
        Err(ErrorData::invalid_params(
            format!("path {path:?} {why}; pass a repo-relative path, as repo_conflicts reports"),
            None,
        ))
    };
    if path.is_empty() {
        return refuse("is empty");
    }
    let mut rel = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => rel.push(part),
            Component::ParentDir => return refuse("contains a `..` component"),
            Component::CurDir => return refuse("contains a `.` component"),
            Component::RootDir | Component::Prefix(_) => return refuse("is absolute"),
        }
    }
    if rel.as_os_str().is_empty() {
        return refuse("names no file");
    }
    Ok(RepoPath {
        abs: root.join(&rel),
        rel,
    })
}

/// Read the working-copy file as text.
///
/// The conflict parsers work on `&str`, so a file whose bytes are not valid UTF-8
/// is refused with a clear message rather than lossily decoded — the same
/// fail-closed stance [`ok_json`] takes on the way out. A missing file is
/// client-actionable too, so both land as `invalid_params`.
pub(crate) async fn read_working_copy(p: &RepoPath) -> Result<String, ErrorData> {
    tokio::fs::read_to_string(&p.abs).await.map_err(|e| {
        let rel = p.rel.display();
        ErrorData::invalid_params(
            match e.kind() {
                std::io::ErrorKind::NotFound => {
                    format!("no such file in the working copy: {rel}")
                }
                std::io::ErrorKind::InvalidData => format!(
                    "{rel} is not valid UTF-8; the conflict model is text-only, so its \
                     markers cannot be parsed (read the raw bytes another way)"
                ),
                _ => format!("cannot read {rel} from the working copy: {e}"),
            },
            None,
        )
    })
}

/// Map a `vcs-git`/`vcs-jj` conflict-model error into an MCP error. A malformed
/// conflict region (`Parse`) and a refused resolution (an `InvalidInput` spawn
/// error — "this region records no base", "Side(3) does not exist") are both
/// about the *caller's* file/argument, so they are client-facing invalid params;
/// anything else is internal.
fn conflict_err(e: VcsError) -> ErrorData {
    if vcs_cli_support::is_invalid_input(&e) || matches!(e.reason(), ErrorReason::Parse { .. }) {
        ErrorData::invalid_params(e.to_string(), None)
    } else {
        ErrorData::internal_error(e.to_string(), None)
    }
}

/// One conflict region on the wire.
///
/// `number`/`total` are **positional** — computed from the parsed region list, so
/// the "conflict N of M" counter is present identically on both backends (git's
/// grammar has no counter of its own). `region` is the backend's own parsed type,
/// serialized as-is: git 2/3-way marker sides and jj n-way diff/snapshot sections
/// are genuinely different shapes and are not flattened into one lossy union. On
/// jj the nested region additionally carries the file's *own* `number`/`total` as
/// jj wrote them, so both readings are available.
#[derive(serde::Serialize)]
pub(crate) struct WireRegion<R: serde::Serialize> {
    number: usize,
    total: usize,
    region: R,
}

/// [`repo_conflict_regions`](crate::VcsMcpServer::repo_conflict_regions)'s JSON
/// shape. `backend` tags which of the two region shapes `regions` carries.
#[derive(serde::Serialize)]
pub(crate) struct ConflictRegionsOut<'a, R: serde::Serialize> {
    backend: &'static str,
    path: &'a str,
    conflict_count: usize,
    regions: Vec<WireRegion<R>>,
}

impl<'a, R: serde::Serialize> ConflictRegionsOut<'a, R> {
    fn new(backend: &'static str, path: &'a str, regions: Vec<R>) -> Self {
        let total = regions.len();
        Self {
            backend,
            path,
            conflict_count: total,
            regions: regions
                .into_iter()
                .enumerate()
                .map(|(i, region)| WireRegion {
                    number: i + 1,
                    total,
                    region,
                })
                .collect(),
        }
    }
}

/// Parse `content` with the backend's conflict grammar and encode its regions.
///
/// A file with no conflict markers yields an empty `regions` list, **not** an
/// error — symmetric with `repo_conflicts`, which reports `[]` on a clean tree.
pub(crate) fn regions_json(
    kind: BackendKind,
    path: &str,
    content: &str,
) -> Result<CallToolResult, ErrorData> {
    match kind {
        BackendKind::Git => {
            let segments = vcs_git::conflict::parse_conflicts(content).map_err(conflict_err)?;
            let regions = segments
                .into_iter()
                .filter_map(|s| match s {
                    vcs_git::conflict::ConflictSegment::Conflict(region) => Some(*region),
                    vcs_git::conflict::ConflictSegment::Text(_) => None,
                })
                .collect();
            ok_json(&ConflictRegionsOut::new("git", path, regions))
        }
        BackendKind::Jj => {
            let segments = vcs_jj::conflict::parse_conflicts(content).map_err(conflict_err)?;
            let regions = segments
                .into_iter()
                .filter_map(|s| match s {
                    vcs_jj::conflict::JjConflictSegment::Conflict(region) => Some(*region),
                    vcs_jj::conflict::JjConflictSegment::Text(_) => None,
                })
                .collect();
            ok_json(&ConflictRegionsOut::new("jj", path, regions))
        }
        // `BackendKind` is `#[non_exhaustive]`: a backend added later would have
        // its own marker grammar, and guessing one of the existing two could
        // mis-parse a file. Fail loudly instead.
        other => Err(unknown_backend(other)),
    }
}

/// Refuse a backend this crate has no conflict grammar for.
fn unknown_backend(kind: BackendKind) -> ErrorData {
    ErrorData::internal_error(
        format!(
            "no conflict-marker grammar is implemented for the {} backend",
            kind.as_str()
        ),
        None,
    )
}

/// What a resolution did, for the tool's JSON result.
pub(crate) struct Resolved {
    /// The whole file's content with every region replaced by the chosen side.
    pub(crate) content: String,
    /// How many conflict regions were replaced.
    pub(crate) conflicts_resolved: usize,
}

/// Parse `content`, map `side` onto the backend's resolution domain, and render
/// the resolved file.
///
/// Every refusal here happens **before** the caller writes anything: a side the
/// backend doesn't have (`side` on git), a `theirs` that would be ambiguous
/// (a jj conflict with more than two sides), an `index` supplied without
/// `side = "side"`, a file whose markers don't parse, or a resolution the region
/// can't satisfy (no recorded base). Never a silent no-op, never a panic.
pub(crate) fn resolve_content(
    kind: BackendKind,
    content: &str,
    side: ConflictSideArg,
    index: Option<usize>,
) -> Result<Resolved, ErrorData> {
    let refuse = |why: String| ErrorData::invalid_params(why, None);
    if index.is_some() && side != ConflictSideArg::Side {
        return Err(refuse(format!(
            "`index` applies only to side=\"side\"; drop it (side={side:?} already names \
             the side exactly)"
        )));
    }
    match kind {
        BackendKind::Git => {
            use vcs_git::conflict::{ConflictSegment, ResolutionSide};
            let chosen = match side {
                ConflictSideArg::Ours => ResolutionSide::Ours,
                ConflictSideArg::Base => ResolutionSide::Base,
                ConflictSideArg::Theirs => ResolutionSide::Theirs,
                // git's three sides are named, so an index would be a second,
                // redundant spelling — and 'the 3rd side' has no git meaning.
                ConflictSideArg::Side => {
                    return Err(refuse(
                        "side=\"side\" is jj-only (jj conflicts can have any number of \
                         sides); on git use \"ours\", \"base\", or \"theirs\""
                            .to_string(),
                    ));
                }
            };
            let segments = vcs_git::conflict::parse_conflicts(content).map_err(conflict_err)?;
            let conflicts_resolved = segments
                .iter()
                .filter(|s| matches!(s, ConflictSegment::Conflict(_)))
                .count();
            Ok(Resolved {
                content: vcs_git::conflict::resolve(&segments, chosen).map_err(conflict_err)?,
                conflicts_resolved,
            })
        }
        BackendKind::Jj => {
            use vcs_jj::conflict::{JjConflictSegment, JjResolution};
            let segments = vcs_jj::conflict::parse_conflicts(content).map_err(conflict_err)?;
            let regions: Vec<_> = segments
                .iter()
                .filter_map(|s| match s {
                    JjConflictSegment::Conflict(region) => Some(&**region),
                    JjConflictSegment::Text(_) => None,
                })
                .collect();
            let chosen = match side {
                // jj records sides in file order; the first is the "ours" analogue.
                ConflictSideArg::Ours => JjResolution::Side(0),
                ConflictSideArg::Base => JjResolution::Base,
                ConflictSideArg::Theirs => {
                    // "Theirs" is only unambiguous for a 2-sided conflict. For an
                    // n-way one, `Side(1)` would be the *middle* side — silently
                    // not what "theirs" means — so refuse and make the caller say
                    // which side it wants.
                    if let Some(region) = regions.iter().find(|r| r.sides().len() != 2) {
                        return Err(refuse(format!(
                            "conflict {} of this file has {} sides, so \"theirs\" is \
                             ambiguous on jj; use side=\"side\" with an explicit 0-based \
                             `index` (repo_conflict_regions lists the sides in order)",
                            region.number,
                            region.sides().len()
                        )));
                    }
                    JjResolution::Side(1)
                }
                ConflictSideArg::Side => JjResolution::Side(index.ok_or_else(|| {
                    refuse(
                        "side=\"side\" needs a 0-based `index` naming which side to keep"
                            .to_string(),
                    )
                })?),
            };
            Ok(Resolved {
                content: vcs_jj::conflict::resolve(&segments, chosen).map_err(conflict_err)?,
                conflicts_resolved: regions.len(),
            })
        }
        // See `regions_json`: never guess another backend's grammar.
        other => Err(unknown_backend(other)),
    }
}
