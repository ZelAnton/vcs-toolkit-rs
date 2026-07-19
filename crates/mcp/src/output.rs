//! Output helpers shared by the tool modules: [`ok_json`] (the fail-closed JSON
//! encoder that refuses a non-UTF-8 path rather than lossily substituting), the
//! [`RepoInfo`] wire shape, and the `vcs-core`/`vcs-forge` → MCP error mappers.
//! Crate-internal (`pub(crate)`); not part of the crate's public API.

use std::path::Path;

use rmcp::ErrorData;
use rmcp::model::{CallToolResult, ContentBlock};

/// Encode a serializable value as a JSON text result.
///
/// **Non-UTF-8 path policy (fail-closed).** Path-bearing DTOs carry a
/// [`PathBuf`](std::path::PathBuf), which serialises to a JSON string only when it
/// is valid UTF-8. A path whose bytes are not valid UTF-8 (possible on Unix) makes
/// serialisation fail, and this returns an **explicit error** rather than emitting
/// the path with `U+FFFD` substitution — so an agent never receives a
/// silently-corrupted path it would feed back into a mutating tool. The ordinary
/// UTF-8 case is unaffected (a plain JSON string). See the crate-level
/// *Non-UTF-8 paths* section.
pub(crate) fn ok_json<T: serde::Serialize>(value: &T) -> Result<CallToolResult, ErrorData> {
    let json = serde_json::to_string_pretty(value).map_err(|e| {
        ErrorData::internal_error(
            format!(
                "failed to serialise the result to JSON: {e} (a filesystem path that is \
                 not valid UTF-8 cannot be represented as a JSON string; it is refused \
                 rather than emitted with U+FFFD substitution)"
            ),
            None,
        )
    })?;
    Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
}

/// [`repo_info`](crate::VcsMcpServer::repo_info)'s JSON shape. `root`/`cwd` are
/// borrowed [`Path`]s — not `to_string_lossy` strings — so that a non-UTF-8
/// root/cwd (legal on Unix) fails serialization in [`ok_json`] the same way
/// every other path-bearing DTO in this crate does, instead of silently
/// substituting `U+FFFD`. See the crate-level *Non-UTF-8 paths* section.
///
/// Deliberately **not** built with `serde_json::json!{}`: that macro resolves
/// a non-literal field to `serde_json::to_value(&expr).unwrap()`, which would
/// **panic** rather than surface a graceful error on a serialization failure
/// (i.e. exactly the non-UTF-8 case this type exists to handle). Passing a
/// concrete `Serialize` struct straight to [`ok_json`] instead runs
/// `serde_json::to_string_pretty`, whose `Err` is already handled there.
#[derive(serde::Serialize)]
pub(crate) struct RepoInfo<'a> {
    pub(crate) backend: &'static str,
    pub(crate) root: &'a Path,
    pub(crate) cwd: &'a Path,
    pub(crate) forge: Option<&'static str>,
}

/// Map a `vcs-core` error into an MCP error. The facade reports a refused
/// *input* (e.g. `commit_paths` with an empty path set) as an
/// `InvalidInput` io error — that's the client's call to fix, so surface it as
/// an invalid-params error rather than an internal one.
pub(crate) fn core_err(e: vcs_core::Error) -> ErrorData {
    // A bad-argument failure — a facade precondition (`Error::Io`/`InvalidInput`)
    // OR the boundary refusal of a flag-like/malformed ref/revision (which the
    // facade now raises as `Error::Vcs` carrying an `InvalidInput` spawn source
    // when it converts a tool string into a validated newtype) — is a client-facing
    // invalid-request, not an internal error. `is_invalid_input` classifies both.
    if e.is_invalid_input() {
        ErrorData::invalid_params(e.to_string(), None)
    } else {
        ErrorData::internal_error(e.to_string(), None)
    }
}

/// Map a `vcs-forge` error into an MCP error — an `Unsupported` op or an
/// `InvalidInput` (the facade's pre-spawn refusal path) is a client-facing
/// invalid-request; a forge/network failure is internal.
pub(crate) fn forge_err(e: vcs_forge::Error) -> ErrorData {
    if e.is_unsupported() || matches!(e, vcs_forge::Error::InvalidInput(_)) {
        ErrorData::invalid_params(e.to_string(), None)
    } else {
        ErrorData::internal_error(e.to_string(), None)
    }
}
