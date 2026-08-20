//! Output helpers shared by the tool modules: [`ok_json`] (the fail-closed JSON
//! encoder that refuses a non-UTF-8 path rather than lossily substituting), the
//! [`RepoInfo`] wire shape, and the `vcs-core`/`vcs-forge` → MCP error mappers.
//! Crate-internal (`pub(crate)`); not part of the crate's public API.

use std::path::Path;

use rmcp::ErrorData;
use rmcp::model::{CallToolResult, ContentBlock};
use vcs_forge::vcs_github::is_repo_unavailable;
use vcs_forge::{ForgeAuth, ForgeKind};

use crate::VcsMcpServer;

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

/// Map a `vcs-core` error into an MCP error. Refused input and operations that
/// the selected backend structurally does not support are client-actionable, so
/// surface them as invalid params rather than internal server failures.
pub(crate) fn core_err(e: vcs_core::Error) -> ErrorData {
    // A bad-argument failure — a facade precondition (`Error::Io`/`InvalidInput`)
    // OR the boundary refusal of a flag-like/malformed ref/revision (which the
    // facade now raises as `Error::Vcs` carrying an `InvalidInput` spawn source
    // when it converts a tool string into a validated newtype) — is a client-facing
    // invalid-request, not an internal error. `is_invalid_input` classifies both;
    // `is_unsupported` covers honest backend asymmetries such as Git op-log access.
    if e.is_invalid_input() || e.is_unsupported() {
        ErrorData::invalid_params(e.to_string(), None)
    } else {
        ErrorData::internal_error(e.to_string(), None)
    }
}

/// Map a `vcs-forge` error into an MCP error — an `Unsupported` op, a pre-spawn
/// **version gate** refusal (the installed `gh`/`glab`/`tea` is too old, which the
/// caller can fix by upgrading), or an `InvalidInput` (the facade's pre-spawn
/// refusal path) is a client-facing invalid-request; a forge/network failure is
/// internal.
pub(crate) fn forge_err(e: vcs_forge::Error) -> ErrorData {
    forge_err_with_hint(e, None)
}

/// [`forge_err`] plus an optional trailing diagnostic clause.
///
/// The classification (invalid-params vs internal) and the message body are the
/// ones `forge_err` already produces — the hint is *appended*, never substituted,
/// so the CLI's own bounded one-line diagnostic still reaches the client. `hint`
/// is composed by [`account_hint`] from the identity probe's parsed answer, so
/// nothing derived from the failing command's captured output can enter here.
fn forge_err_with_hint(e: vcs_forge::Error, hint: Option<String>) -> ErrorData {
    let invalid = e.is_unsupported()
        || e.is_version_gated()
        || matches!(e, vcs_forge::Error::InvalidInput(_));
    let message = match hint {
        Some(hint) => format!("{e} — {hint}"),
        None => e.to_string(),
    };
    if invalid {
        ErrorData::invalid_params(message, None)
    } else {
        ErrorData::internal_error(message, None)
    }
}

/// One login rendered for the hint — `` `login` (host) ``, or bare `` `login` ``
/// when the report carried no host for it (which is what an unrecognised report
/// leaves behind; inventing `github.com` there would be a guess).
fn render_account(login: &str, host: Option<&str>) -> String {
    match host {
        Some(host) => format!("`{login}` ({host})"),
        None => format!("`{login}`"),
    }
}

/// The host `gh` reported for `login`, when the account list names it. Read from
/// the list rather than assumed, because `active_account` is only a login and a
/// machine can hold logins on several hosts.
fn host_of<'a>(auth: &'a ForgeAuth, login: &str) -> Option<&'a str> {
    auth.accounts
        .iter()
        .find(|account| account.login == login)
        .map(|account| account.host.as_str())
}

/// The account-selection clause for a failure
/// [`is_repo_unavailable`](vcs_forge::vcs_github::is_repo_unavailable) classified,
/// built **only** from `auth` — the identity probe's parsed answer (logins, hosts)
/// — plus fixed text. The failing command's captured `stdout`/`stderr` is never a
/// source here: `gh auth status` masks tokens and this reads its *parsed* fields
/// rather than its report text, so no captured stream, and no secret one might
/// carry, can ride out through the hint.
///
/// `None` when the probe **contradicts** the guess (the repository is visible to
/// the account in use): the classifier is deliberately wide — an endpoint can 404
/// inside a repository the account sees perfectly well — and telling that caller to
/// switch accounts would send them the wrong way. Everything the clause claims is
/// something the probe actually reported: an unrecognised report format names no
/// account and lists none, rather than asserting there are none.
fn account_hint(auth: &ForgeAuth) -> Option<String> {
    if auth.repo_visible == Some(true) {
        return None;
    }
    // Which identity ran the call. Three honest states: named, session-but-
    // unnamed (an unrecognised report, or logins on several hosts), and no
    // session at all (what gh's exit code 4 means). The named case carries its
    // host when the account list has one for it — same rendering as the other
    // logins below, from the same parsed source.
    let mut hint = match (&auth.active_account, auth.authed) {
        (Some(login), _) => format!(
            "the `gh` account in use is {}",
            render_account(login, host_of(auth, login))
        ),
        (None, Some(false)) => "`gh` reports no logged-in account on this machine".to_string(),
        (None, _) => "`gh` did not report which account it runs as".to_string(),
    };
    // Only the probe's own `Some(false)` justifies this; `None` (not probed —
    // there was no session to probe with) claims nothing.
    if auth.repo_visible == Some(false) {
        hint.push_str(" and this repository is not visible to it");
    }
    // Every other login gh reported, each with its host: the probe does not
    // resolve which host *this* repository belongs to, so filtering the list to
    // "this host" would be a guess — naming the host per account lets the caller
    // do it. Silent when nothing was recognised (an empty list is "unknown", not
    // "there are none" — see `ForgeAuth::accounts`).
    let others: Vec<String> = auth
        .accounts
        .iter()
        .filter(|account| Some(&account.login) != auth.active_account.as_ref())
        .map(|account| render_account(&account.login, Some(&account.host)))
        .collect();
    if !others.is_empty() {
        hint.push_str(&format!("; other logins here: {}", others.join(", ")));
    } else if !auth.accounts.is_empty() {
        hint.push_str("; it is the only login on this machine");
    }
    hint.push_str(
        "; choose the identity explicitly by restarting the server with \
         `--gh-account <login>` or `--gh-token-env <VAR>`",
    );
    if auth.authed == Some(false) {
        hint.push_str(" (or log in with `gh auth login`)");
    }
    Some(hint)
}

impl VcsMcpServer {
    /// Unwrap a forge call's result, mapping a failure through [`forge_err`] —
    /// and, when that failure looks like *the repository is unavailable to the
    /// account `gh` runs as*, appending the account-selection hint
    /// ([`account_hint`]) so the client learns **who** the call ran as instead of
    /// only that GitHub refused to resolve the repository.
    ///
    /// The extra work is strictly on the error path, and only there: a successful
    /// call spawns nothing beyond itself, and a failure that the classifier does
    /// not recognise is mapped exactly as before. When it does recognise one, this
    /// spends the identity probe (`Forge::auth_info`: `gh auth status`, plus one
    /// `gh repo view` when a session exists) — the same probe `forge_info`
    /// exposes, not a second implementation of it. If the probe itself fails,
    /// the original error is returned unchanged: a diagnostic must never replace
    /// the failure it was trying to explain.
    ///
    /// GitHub only. The classifier reads `gh`'s semantics and the hint names
    /// `gh`-specific flags, so a GitLab/Gitea failure — `glab` reports a plain
    /// `404 Not Found` for a hidden project — is mapped with no hint rather than
    /// pointed at a flag that would not help it.
    pub(crate) async fn forge_ok<T>(
        &self,
        result: Result<T, vcs_forge::Error>,
    ) -> Result<T, ErrorData> {
        match result {
            Ok(value) => Ok(value),
            Err(e) => Err(self.forge_error(e).await),
        }
    }

    /// [`forge_ok`](Self::forge_ok)'s error half.
    async fn forge_error(&self, e: vcs_forge::Error) -> ErrorData {
        let Ok(forge) = self.forge() else {
            // Unreachable in practice (a forge error implies a forge), and a
            // plain mapping is the right answer if it ever isn't.
            return forge_err(e);
        };
        if !matches!(forge.kind(), ForgeKind::GitHub) {
            return forge_err(e);
        }
        let repo_unavailable =
            matches!(&e, vcs_forge::Error::Forge(inner) if is_repo_unavailable(inner));
        if !repo_unavailable {
            return forge_err(e);
        }
        match forge.auth_info().await {
            Ok(auth) => forge_err_with_hint(e, account_hint(&auth)),
            Err(_) => forge_err(e),
        }
    }
}
