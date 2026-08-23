//! Credential provisioning for the CLI wrappers.
//!
//! Remote operations (a forge API call, a `git`/`jj` fetch or push against an
//! authenticated remote) need a secret the toolkit deliberately does **not**
//! store. By default every backend authenticates through its CLI's *own* ambient
//! credential system (`gh`/`glab` logins, git credential helpers, the SSH agent)
//! — the toolkit holds nothing. This module adds an **opt-in** seam for callers
//! that want to supply a secret *per operation* instead: a CI job minting a
//! short-lived token, an agent acting for different accounts, a vault-backed
//! rotation. You implement (or pick a built-in) [`CredentialProvider`]; the
//! backend resolves it just-in-time and injects the secret through the relevant
//! CLI's *native* non-interactive mechanism — never persisting it.
//!
//! How the secret reaches each CLI (chosen so the value never lands in `argv`,
//! which is broadly observable; only an env-var *name* or a token value in the
//! process environment is used):
//!
//! - **GitHub** (`gh`) → `GH_TOKEN` environment variable.
//! - **GitLab** (`glab`) → `GITLAB_TOKEN` environment variable.
//! - **git** (`fetch`/`push`/`clone`) → an inline `credential.helper` that emits
//!   the secret read from an environment variable *by name* (see
//!   [`git_credential_helper`]); the secret value is never an argument.
//! - **Gitea** (`tea`) and **Jujutsu** (`jj`) — no per-operation injection: `tea`
//!   authenticates only from its stored logins, and `jj`'s in-process git backend
//!   offers no per-invocation credential override. Both stay on ambient auth.
//!
//! Secrets are wrapped in [`Secret`], which redacts itself in `Debug`/`Display`
//! so a stray log line can't leak a token. (It does **not** securely zero memory
//! on drop — that is out of scope; rely on OS-level protections for that.)

use std::fmt;

use async_trait::async_trait;
use processkit::{Error, Result};

/// A secret value — an API token, a password — that **redacts itself** whenever
/// it is formatted, so it can't leak into a log line or an error message. Read
/// the underlying value only at the point of use, via [`expose`](Secret::expose).
///
/// Redaction is the achievable guarantee here; this type does **not** securely
/// scrub its memory on drop.
///
/// Deliberately **not** `PartialEq`/`Eq`: comparing secrets with `String`'s
/// short-circuiting `==` is timing-variable and turns the type into an equality
/// oracle. Compare the [`expose`](Secret::expose)d value explicitly if you must.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    /// Wrap a secret value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the underlying secret. Call this only where the value is actually
    /// needed (e.g. setting an environment variable on a command); don't store
    /// or log the result.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(\"***\")")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// A resolved credential: a [`Secret`] plus an optional username. For a forge
/// token only the secret is used; for git HTTPS the username pairs with the
/// secret as the password (a personal-access token).
///
/// Not `PartialEq`/`Eq` (it holds a [`Secret`], which intentionally is neither).
#[derive(Clone, Debug)]
pub struct Credential {
    username: Option<String>,
    secret: Secret,
}

impl Credential {
    /// A bare token/secret with no username (the forge case, and git HTTPS where
    /// any username is accepted). For git HTTPS a default username
    /// (`x-access-token`, which GitHub/GitLab personal-access tokens accept) is
    /// supplied automatically; use [`userpass`](Credential::userpass) if your host
    /// needs a specific one. Forge token-env injection ignores the username.
    #[must_use]
    pub fn token(secret: impl Into<Secret>) -> Self {
        Self {
            username: None,
            secret: secret.into(),
        }
    }

    /// A username paired with a secret (git HTTPS user/password, where the
    /// password is typically a personal-access token). The username is used only
    /// for **git HTTPS**; forge token-env injection (`GH_TOKEN`/`GITLAB_TOKEN`)
    /// uses only the secret and ignores the username.
    #[must_use]
    pub fn userpass(username: impl Into<String>, secret: impl Into<Secret>) -> Self {
        Self {
            username: Some(username.into()),
            secret: secret.into(),
        }
    }

    /// The username, if one was supplied.
    #[must_use]
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// The secret (token/password).
    #[must_use]
    pub fn secret(&self) -> &Secret {
        &self.secret
    }

    /// Reject values that the line-based Git credential protocol cannot carry.
    ///
    /// The constructors intentionally remain infallible so they can continue to
    /// be used by non-Git credential consumers. Every path that resolves or
    /// materializes a credential for a Git helper calls this shared check before
    /// exposing either field to the helper.
    pub(crate) fn validate(&self) -> Result<()> {
        validate_field(
            "username",
            self.username.as_deref().unwrap_or(DEFAULT_GIT_USERNAME),
        )?;
        validate_field("secret", self.secret.expose())
    }

    /// Apply the helper validation while preserving the ambient-auth rule for an
    /// empty or whitespace-only secret, which is never materialized into a helper.
    pub(crate) fn validate_for_resolution(&self) -> Result<()> {
        validate_field(
            "username",
            self.username.as_deref().unwrap_or(DEFAULT_GIT_USERNAME),
        )?;
        validate_field("secret", self.secret.expose())?;
        Ok(())
    }
}

fn validate_field(field: &str, value: &str) -> Result<()> {
    if value.contains('\r') || value.contains('\n') {
        return Err(Error::spawn(
            "git",
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("credential {field} must not contain CR or LF"),
            ),
        ));
    }
    Ok(())
}

/// Which backend/tool is asking for a credential — lets a provider return
/// different secrets per service. `#[non_exhaustive]`: new backends may be added.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum CredentialService {
    /// A `git` remote operation (fetch/push/clone over HTTPS).
    Git,
    /// A GitHub (`gh`) API operation.
    GitHub,
    /// A GitLab (`glab`) API operation.
    GitLab,
    /// A Gitea (`tea`) API operation. Reserved: `tea` has no per-operation token
    /// mechanism today, so no backend currently emits this — it exists so a
    /// provider can be written against it once `tea` gains support.
    Gitea,
}

/// The context of a credential request: which service, and the remote host if
/// the backend knows it (forge calls often defer host resolution to the CLI, so
/// `host` is frequently `None`). `#[non_exhaustive]`: more context may be added.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct CredentialRequest<'a> {
    /// The backend/tool making the request.
    pub service: CredentialService,
    /// The remote host (e.g. `github.com`), if known.
    pub host: Option<&'a str>,
}

impl<'a> CredentialRequest<'a> {
    /// A request for `service` with no known host.
    #[must_use]
    pub fn new(service: CredentialService) -> Self {
        Self {
            service,
            host: None,
        }
    }

    /// Attach a known remote host.
    #[must_use]
    pub fn with_host(mut self, host: &'a str) -> Self {
        self.host = Some(host);
        self
    }
}

/// Supplies a [`Credential`] for a [`CredentialRequest`], just-in-time. Returning
/// `Ok(None)` means "I have nothing for this request" — the backend then falls
/// back to its ambient CLI auth, exactly as if no provider were configured.
///
/// Implement this for a vault/keychain lookup, per-account routing, or token
/// rotation; for simple cases use [`StaticCredential`], [`EnvToken`], or
/// [`provider_fn`]. The trait is async and dyn-compatible, so a backend can hold
/// an `Arc<dyn CredentialProvider>`.
#[async_trait]
pub trait CredentialProvider: Send + Sync {
    /// Resolve the credential for `request`, or `Ok(None)` to defer to ambient
    /// auth. An `Err` aborts the operation (e.g. the vault was unreachable).
    ///
    /// A returned credential whose secret is **empty** is treated as `None`
    /// (ambient) by the clients — an empty token can't authenticate, and injecting
    /// one would override the ambient login with nothing rather than defer to it.
    async fn credential(&self, request: &CredentialRequest<'_>) -> Result<Option<Credential>>;
}

/// A provider that always yields the same [`Credential`] for every request — the
/// common "use this one token" case.
#[derive(Clone, Debug)]
pub struct StaticCredential(Credential);

impl StaticCredential {
    /// Always supply `credential`.
    #[must_use]
    pub fn new(credential: Credential) -> Self {
        Self(credential)
    }

    /// Always supply a bare token.
    #[must_use]
    pub fn token(secret: impl Into<Secret>) -> Self {
        Self(Credential::token(secret))
    }
}

#[async_trait]
impl CredentialProvider for StaticCredential {
    async fn credential(&self, _request: &CredentialRequest<'_>) -> Result<Option<Credential>> {
        self.0.validate_for_resolution()?;
        Ok(Some(self.0.clone()))
    }
}

/// A provider that reads a bare token from a named **environment variable**, at
/// request time. If the variable is unset/empty it yields `None` (fall back to
/// ambient auth) rather than erroring — handy for "use `$MY_TOKEN` if present".
#[derive(Clone, Debug)]
pub struct EnvToken {
    var: String,
    username: Option<String>,
}

impl EnvToken {
    /// Read the token from environment variable `var`.
    #[must_use]
    pub fn new(var: impl Into<String>) -> Self {
        Self {
            var: var.into(),
            username: None,
        }
    }

    /// Pair the token with a username (for git HTTPS).
    #[must_use]
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }
}

#[async_trait]
impl CredentialProvider for EnvToken {
    async fn credential(&self, _request: &CredentialRequest<'_>) -> Result<Option<Credential>> {
        // Validate the username before an unset/blank variable can defer to
        // ambient auth. The username is still an input to the helper path even
        // when this provider has no secret to materialize.
        validate_field(
            "username",
            self.username.as_deref().unwrap_or(DEFAULT_GIT_USERNAME),
        )?;
        match std::env::var(&self.var) {
            // A set-but-blank (or whitespace-only) variable is treated as unset →
            // `None` (defer to ambient auth), not an empty token that would override
            // the ambient login with nothing.
            Ok(value) => {
                let credential = match &self.username {
                    Some(user) => Credential::userpass(user.clone(), value.clone()),
                    None => Credential::token(value.clone()),
                };
                credential.validate_for_resolution()?;
                if value.trim().is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(credential))
                }
            }
            _ => Ok(None),
        }
    }
}

/// Adapt a synchronous closure into a [`CredentialProvider`]. The closure runs at
/// request time and returns the credential (or `None` to defer to ambient auth).
/// For async sources (a network vault), implement [`CredentialProvider`] directly.
#[must_use]
pub fn provider_fn<F>(f: F) -> FnProvider<F>
where
    F: Fn(&CredentialRequest<'_>) -> Result<Option<Credential>> + Send + Sync,
{
    FnProvider(f)
}

/// A [`CredentialProvider`] backed by a synchronous closure (see [`provider_fn`]).
pub struct FnProvider<F>(F);

impl<F> fmt::Debug for FnProvider<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FnProvider").finish_non_exhaustive()
    }
}

#[async_trait]
impl<F> CredentialProvider for FnProvider<F>
where
    F: Fn(&CredentialRequest<'_>) -> Result<Option<Credential>> + Send + Sync,
{
    async fn credential(&self, request: &CredentialRequest<'_>) -> Result<Option<Credential>> {
        let credential = (self.0)(request)?;
        if let Some(credential) = &credential {
            credential.validate_for_resolution()?;
        }
        Ok(credential)
    }
}

/// The default username git uses when a [`Credential`] supplies none. GitHub (and
/// GitLab) accept any username when the password is a personal-access token, so a
/// fixed placeholder works; `git` still requires *a* username.
const DEFAULT_GIT_USERNAME: &str = "x-access-token";

/// Environment-variable name carrying the username for [`git_credential_helper`].
const GIT_USERNAME_VAR: &str = "VCS_TOOLKIT_GIT_USERNAME";
/// Environment-variable name carrying the secret for [`git_credential_helper`].
const GIT_PASSWORD_VAR: &str = "VCS_TOOLKIT_GIT_PASSWORD";
/// Environment-variable name carrying the *expected host* for
/// [`git_credential_helper`]. When set (non-empty), the helper releases the
/// credential only for a request whose `host` matches — so an HTTP redirect or a
/// submodule fetch to a **different** host never receives the token. Empty →
/// ungated (the helper answers for any host, the pre-host-scoping behavior).
const GIT_HOST_VAR: &str = "VCS_TOOLKIT_GIT_HOST";

/// Extract the `host[:port]` from an HTTPS git URL
/// (`https://[user[:pass]@]host[:port]/…`), matching the scheme ASCII
/// case-insensitively and preserving the authority **verbatim** — including the
/// original host case and port — to scope a credential helper to the host an
/// operation targets. git carries the same `host[:port]` in its credential request
/// and compares it byte-for-byte, so normalizing here would withhold a legitimate
/// credential.
/// Returns `None` for a non-HTTPS URL (an SSH remote never invokes the HTTPS
/// credential helper, so gating it is moot), an IPv6-literal authority, or an
/// unparseable one — in which case the helper stays **ungated**, no worse than
/// before host scoping existed.
#[must_use]
pub fn https_host(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    // The authority ends at the first `/`, `?`, or `#`. Drop any `user:pass@`
    // userinfo, but keep the host **and its port**, with the **original case**:
    // git's credential request carries `host=` verbatim from the URL — it
    // includes the port when one was given (`example.com:8443`) and does not
    // lower-case the host — and the snippet compares it byte-for-byte, so what
    // we scope to must match exactly (stripping the port or normalizing case
    // would withhold a legitimate credential and break auth).
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    // An IPv6 literal (`[::1]:443`) — git formats `host=` for these idiosyncratically;
    // rather than risk withholding a valid credential, stay ungated (return `None`)
    // so auth still works, just without host scoping for that rare case.
    if host_port.is_empty() || host_port.starts_with('[') {
        return None;
    }
    Some(host_port.to_string())
}

/// The pieces needed to authenticate a `git` HTTPS operation with a [`Credential`]
/// **without putting the secret in `argv`**. See [`git_credential_helper`].
///
/// `#[non_exhaustive]`: only [`git_credential_helper`] constructs it, so new fields
/// can be added without breaking callers (who read the fields, never build it).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GitCredentialHelper {
    /// `-c key=value` global options to place **before** the git subcommand. They
    /// reference the secret only by environment-variable *name*, never by value.
    pub config_args: Vec<String>,
    /// Environment variables (name → value) to set on the command. This is where
    /// the actual secret lives — in the child's environment, not its arguments.
    pub env: Vec<(String, Secret)>,
}

/// Build a git `credential.helper` invocation that supplies `cred` over HTTPS
/// while keeping the secret out of `argv` (which is broadly observable). The
/// returned [`config_args`](GitCredentialHelper::config_args) install an inline
/// helper that prints the credential read from two environment variables; the
/// secret value appears only in [`env`](GitCredentialHelper::env), i.e. the child
/// process environment. A leading empty `credential.helper=` first clears any
/// inherited helper so only ours runs.
///
/// The helper is a tiny POSIX-shell snippet: git runs `credential.helper` values
/// that begin with `!` via the shell it ships with (so this works on Windows too,
/// where Git for Windows bundles its own `sh` — it never goes through `cmd.exe`).
/// It applies to **HTTPS remotes only**: git invokes a credential helper just for
/// HTTP(S) user/password auth, so an SSH remote ignores it and falls through to
/// the SSH agent. It is opt-in — built only when a [`CredentialProvider`] yields a
/// credential — so the default path is unchanged. The helper answers only git's
/// `get` action (never `store`/`erase`), so the secret is never written to a
/// credential cache or config; it lives only in the child's environment.
///
/// The username/secret must not contain `\r` or `\n`: git's credential protocol is
/// line-based, so either embedded byte is read as the end of the value (git
/// truncates there, and a username newline can add extra protocol fields). Invalid
/// values return an `InvalidInput` error before the helper is emitted. Real tokens
/// and usernames never contain one.
///
/// `expect_host` scopes the credential to a host: when `Some`, the helper reads
/// git's request (which names the host git is about to authenticate to) and
/// releases the secret only if that host matches — so a cross-host redirect or a
/// submodule fetch to another host can't extract the token. `None` (or an
/// unknown host) leaves the helper ungated. Callers that know the operation's
/// target (e.g. `clone` from its URL) pass [`https_host`] of it.
#[must_use = "handle the helper or its invalid-input error"]
pub fn git_credential_helper(
    cred: &Credential,
    expect_host: Option<&str>,
) -> Result<GitCredentialHelper> {
    cred.validate()?;
    let username = cred.username().unwrap_or(DEFAULT_GIT_USERNAME).to_string();
    // Reference the values by env-var NAME inside the snippet, so `argv` never
    // carries the secret. Respond only to git's `get` action; ignore store/erase.
    // Read git's request from stdin (key=value lines, terminated by a blank line)
    // to learn the host, then release the credential only when:
    //   - the password var is non-empty (`test -n`): if `config_args` is applied
    //     without `env`, the helper emits nothing and git falls through to ambient
    //     auth, rather than overriding it with an empty credential that fails; and
    //   - the host is unscoped (`$…_HOST` empty) or matches the request's host, so
    //     a redirect/submodule to a different host never receives the secret.
    let helper = format!(
        "!f() {{ test \"$1\" = get || return; h=; \
         while IFS= read -r l; do case \"$l\" in \"\") break ;; host=*) h=${{l#host=}} ;; esac; done; \
         test -n \"${GIT_PASSWORD_VAR}\" || return; \
         test -z \"${GIT_HOST_VAR}\" || test \"$h\" = \"${GIT_HOST_VAR}\" || return; \
         printf 'username=%s\\npassword=%s\\n' \
         \"${GIT_USERNAME_VAR}\" \"${GIT_PASSWORD_VAR}\"; }}; f"
    );
    Ok(GitCredentialHelper {
        config_args: vec![
            "-c".to_string(),
            "credential.helper=".to_string(),
            "-c".to_string(),
            format!("credential.helper={helper}"),
        ],
        env: vec![
            (GIT_USERNAME_VAR.to_string(), Secret::new(username)),
            (GIT_PASSWORD_VAR.to_string(), cred.secret().clone()),
            (
                GIT_HOST_VAR.to_string(),
                Secret::new(expect_host.unwrap_or_default()),
            ),
        ],
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::{Command, Output, Stdio};

    use super::*;

    fn credential_fill(helper: &GitCredentialHelper, request: &str) -> Output {
        let mut command = Command::new("git");
        command
            .args(&helper.config_args)
            .args(["credential", "fill"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (name, value) in &helper.env {
            command.env(name, value.expose());
        }

        let mut child = command.spawn().expect("git must be available for tests");
        child
            .stdin
            .take()
            .expect("credential fill stdin")
            .write_all(request.as_bytes())
            .expect("write credential request");
        child.wait_with_output().expect("wait for credential fill")
    }

    #[test]
    fn secret_redacts_in_debug_and_display() {
        let s = Secret::new("hunter2");
        assert_eq!(format!("{s:?}"), "Secret(\"***\")");
        assert_eq!(format!("{s}"), "***");
        // The value is only reachable through `expose`.
        assert_eq!(s.expose(), "hunter2");
        // A Credential's Debug must not leak the secret either.
        let c = Credential::userpass("alice", "hunter2");
        let dbg = format!("{c:?}");
        assert!(!dbg.contains("hunter2"), "secret leaked in Debug: {dbg}");
        assert!(dbg.contains("alice"), "username should be visible: {dbg}");
    }

    #[tokio::test]
    async fn static_and_env_and_fn_providers() {
        let req = CredentialRequest::new(CredentialService::GitHub);

        let s = StaticCredential::token("tok");
        assert_eq!(
            s.credential(&req).await.unwrap().unwrap().secret().expose(),
            "tok"
        );

        // EnvToken: absent → None; present → the token.
        let env = EnvToken::new("VCS_TOOLKIT_TEST_TOKEN_UNSET_XYZ");
        assert!(env.credential(&req).await.unwrap().is_none());

        // provider_fn routes on the request.
        let p = provider_fn(|r: &CredentialRequest<'_>| {
            Ok(match r.service {
                CredentialService::GitHub => Some(Credential::token("gh")),
                _ => None,
            })
        });
        assert_eq!(
            p.credential(&req).await.unwrap().unwrap().secret().expose(),
            "gh"
        );
        let gl = CredentialRequest::new(CredentialService::GitLab);
        assert!(p.credential(&gl).await.unwrap().is_none());
    }

    // EnvToken's present-variable path: a set variable yields the token (the most
    // common "use $CI_TOKEN" provider); the username pairs through `with_username`.
    #[tokio::test]
    async fn env_token_reads_a_present_variable() {
        let req = CredentialRequest::new(CredentialService::Git);
        // A unique name so no other (parallel) test reads or writes it.
        let var = "VCS_TOOLKIT_TEST_ENV_TOKEN_PRESENT_4f2a";
        // SAFETY: edition-2024 requires `unsafe` for env mutation; the name is
        // unique to this test, so there is no concurrent reader of it.
        unsafe { std::env::set_var(var, "tok-from-env") };
        let provider = EnvToken::new(var).with_username("alice");
        let cred = provider
            .credential(&req)
            .await
            .unwrap()
            .expect("present variable yields a credential");
        assert_eq!(cred.secret().expose(), "tok-from-env");
        assert_eq!(cred.username(), Some("alice"));
        // Once removed, it falls back to None (ambient).
        unsafe { std::env::remove_var(var) };
        assert!(provider.credential(&req).await.unwrap().is_none());
    }

    fn invalid_input<T>(result: Result<T>) -> Error {
        match result {
            Ok(_) => panic!("credential unexpectedly accepted CR/LF"),
            Err(error) => {
                assert!(
                    crate::is_invalid_input(&error),
                    "credential rejection must be InvalidInput: {error:?}"
                );
                error
            }
        }
    }

    const CRLF_USERNAMES: &[&str] = &["alice\r", "alice\n", "alice\t\r ", "alice \n\t"];
    const CRLF_SECRETS: &[&str] = &["\r", "\n", "\t\r ", " \n\t"];

    #[test]
    fn git_credential_helper_rejects_cr_and_lf_before_protocol_output() {
        for &bad_username in CRLF_USERNAMES {
            let error = invalid_input(git_credential_helper(
                &Credential::userpass(bad_username, "secret"),
                None,
            ));
            assert!(error.to_string().contains("username"));
        }
        for &bad_secret in CRLF_SECRETS {
            let error = invalid_input(git_credential_helper(
                &Credential::userpass("alice", bad_secret),
                None,
            ));
            assert!(error.to_string().contains("secret"));
        }

        // A valid value still produces the helper, and the values remain in the
        // environment rather than being interpolated into its argv/config text.
        let helper = git_credential_helper(&Credential::userpass("alice", "secret"), None)
            .expect("valid credential");
        assert!(helper.config_args.iter().all(|arg| !arg.contains("secret")));
        assert!(helper.config_args.iter().all(|arg| !arg.contains("alice")));

        for blank_secret in ["", "   ", "\t"] {
            for &bad_username in CRLF_USERNAMES {
                let error = invalid_input(git_credential_helper(
                    &Credential::userpass(bad_username, blank_secret),
                    None,
                ));
                assert!(error.to_string().contains("username"));
            }
        }
    }

    #[tokio::test]
    async fn built_in_and_closure_providers_reject_the_same_cr_and_lf_inputs() {
        let req = CredentialRequest::new(CredentialService::Git);

        for &bad_username in CRLF_USERNAMES {
            invalid_input(
                StaticCredential::new(Credential::userpass(bad_username, "secret"))
                    .credential(&req)
                    .await,
            );
            invalid_input(
                provider_fn(move |_request: &CredentialRequest<'_>| {
                    Ok(Some(Credential::userpass(bad_username, "secret")))
                })
                .credential(&req)
                .await,
            );
        }

        for &bad_secret in CRLF_SECRETS {
            invalid_input(
                StaticCredential::new(Credential::userpass("alice", bad_secret))
                    .credential(&req)
                    .await,
            );
            invalid_input(
                provider_fn(move |_request: &CredentialRequest<'_>| {
                    Ok(Some(Credential::userpass("alice", bad_secret)))
                })
                .credential(&req)
                .await,
            );
        }

        // Username validation must happen before the empty/whitespace-only
        // secret is classified as ambient auth.
        for blank_secret in ["", "   ", "\t"] {
            for &bad_username in CRLF_USERNAMES {
                invalid_input(
                    StaticCredential::new(Credential::userpass(bad_username, blank_secret))
                        .credential(&req)
                        .await,
                );
                invalid_input(
                    provider_fn(move |_request: &CredentialRequest<'_>| {
                        Ok(Some(Credential::userpass(bad_username, blank_secret)))
                    })
                    .credential(&req)
                    .await,
                );
            }
        }

        for blank_secret in ["", "   ", "\t"] {
            assert!(
                StaticCredential::token(blank_secret)
                    .credential(&req)
                    .await
                    .unwrap()
                    .is_some(),
                "plain blank static secret remains a provider result"
            );
            assert!(
                provider_fn(move |_request: &CredentialRequest<'_>| {
                    Ok(Some(Credential::token(blank_secret)))
                })
                .credential(&req)
                .await
                .unwrap()
                .is_some(),
                "plain blank closure secret remains a provider result"
            );
        }

        for (suffix, blank) in [("empty", ""), ("space", "   "), ("tab", "\t")] {
            let var = format!("VCS_TOOLKIT_TEST_ENV_TOKEN_BLANK_{suffix}");
            unsafe { std::env::set_var(&var, blank) };
            assert!(
                EnvToken::new(&var)
                    .credential(&req)
                    .await
                    .unwrap()
                    .is_none(),
                "plain blank environment secret remains ambient"
            );
            unsafe { std::env::remove_var(&var) };
        }

        // Environment-backed secrets are validated before they can reach `printf`.
        for (suffix, bad) in [
            ("cr_only", "\r"),
            ("lf_only", "\n"),
            ("mixed_cr", "\t\r "),
            ("mixed_lf", " \n\t"),
        ] {
            let var = format!("VCS_TOOLKIT_TEST_ENV_TOKEN_CRLF_{suffix}");
            unsafe { std::env::set_var(&var, bad) };
            invalid_input(
                EnvToken::new(&var)
                    .with_username("alice")
                    .credential(&req)
                    .await,
            );
            unsafe { std::env::remove_var(&var) };
        }
        for (suffix, bad) in [
            ("cr", "alice\r"),
            ("lf", "alice\n"),
            ("mixed_cr", "alice\t\r "),
            ("mixed_lf", "alice \n\t"),
        ] {
            let var = format!("VCS_TOOLKIT_TEST_ENV_USERNAME_CRLF_{suffix}");
            unsafe { std::env::set_var(&var, "secret") };
            invalid_input(
                EnvToken::new(&var)
                    .with_username(bad)
                    .credential(&req)
                    .await,
            );
            unsafe { std::env::remove_var(&var) };
        }

        for (suffix, blank_secret) in [("empty", ""), ("space", "   "), ("ws", "\t")] {
            for &bad_username in CRLF_USERNAMES {
                let var = format!("VCS_TOOLKIT_TEST_ENV_USERNAME_BLANK_{suffix}");
                unsafe { std::env::set_var(&var, blank_secret) };
                invalid_input(
                    EnvToken::new(&var)
                        .with_username(bad_username)
                        .credential(&req)
                        .await,
                );
                unsafe { std::env::remove_var(&var) };
            }
        }
    }

    #[test]
    fn git_credential_helper_keeps_secret_out_of_argv() {
        let cred = Credential::userpass("alice", "s3cr3t");
        let h = git_credential_helper(&cred, None).expect("valid credential");
        // The secret value must NOT appear in any config arg (only the env-var name).
        for a in &h.config_args {
            assert!(!a.contains("s3cr3t"), "secret leaked into argv: {a}");
        }
        assert!(
            h.config_args
                .iter()
                .any(|a| a.contains("VCS_TOOLKIT_GIT_PASSWORD"))
        );
        // A leading empty helper clears inherited helpers.
        assert!(h.config_args.iter().any(|a| a == "credential.helper="));
        // The secret + username live in the env, keyed by the helper's var names.
        let pw = h
            .env
            .iter()
            .find(|(k, _)| k == "VCS_TOOLKIT_GIT_PASSWORD")
            .unwrap();
        assert_eq!(pw.1.expose(), "s3cr3t");
        let user = h
            .env
            .iter()
            .find(|(k, _)| k == "VCS_TOOLKIT_GIT_USERNAME")
            .unwrap();
        assert_eq!(user.1.expose(), "alice");
    }

    #[test]
    fn git_credential_helper_defaults_username() {
        let h = git_credential_helper(&Credential::token("t"), None).expect("valid credential");
        let user = h
            .env
            .iter()
            .find(|(k, _)| k == "VCS_TOOLKIT_GIT_USERNAME")
            .unwrap();
        assert_eq!(user.1.expose(), DEFAULT_GIT_USERNAME);
    }

    #[test]
    fn git_credential_helper_scopes_to_expected_host() {
        // Ungated: the host env is present but empty, and the snippet's host
        // check is skipped — the credential is released for any host.
        let ungated =
            git_credential_helper(&Credential::token("t"), None).expect("valid credential");
        let host_env = ungated
            .env
            .iter()
            .find(|(k, _)| k == "VCS_TOOLKIT_GIT_HOST")
            .expect("host env var is always set");
        assert_eq!(host_env.1.expose(), "", "None => empty (ungated) host");

        // Gated: the expected host travels in the env (never argv), and the
        // snippet gates on it — the host value is not baked into the shell text.
        let gated = git_credential_helper(&Credential::token("t"), Some("github.com"))
            .expect("valid credential");
        assert_eq!(
            gated
                .env
                .iter()
                .find(|(k, _)| k == "VCS_TOOLKIT_GIT_HOST")
                .unwrap()
                .1
                .expose(),
            "github.com"
        );
        assert!(
            gated.config_args.iter().all(|a| !a.contains("github.com")),
            "the expected host stays in env, out of argv: {:?}",
            gated.config_args
        );
        // The snippet references the host var by name and reads git's request.
        assert!(
            gated
                .config_args
                .iter()
                .any(|a| a.contains("VCS_TOOLKIT_GIT_HOST") && a.contains("host=")),
            "snippet gates on the request host: {:?}",
            gated.config_args
        );
    }

    #[test]
    fn https_host_accepts_any_ascii_scheme_case_and_keeps_authority_verbatim() {
        assert_eq!(
            https_host("https://github.com/o/r.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            https_host("HTTPS://Git.Example.COM:8443/o/r.git").as_deref(),
            Some("Git.Example.COM:8443"),
            "the scheme is case-insensitive; the host and port are not normalized"
        );
        // Userinfo is stripped, but the port and case are PRESERVED — git's
        // `host=` request carries `host[:port]` verbatim from the URL and matches
        // it case-sensitively, so scoping to a normalized host would withhold the
        // credential and break auth for a non-default port / uppercase host.
        assert_eq!(
            https_host(
                "hTtPs://first@x-access-token:tok@Git.Example.COM:8443/g/file@rev?email=a@b#tail@x"
            )
            .as_deref(),
            Some("Git.Example.COM:8443"),
            "last authority @ drops userinfo; later @ bytes do not affect the host"
        );
        assert_eq!(
            https_host("HtTpS://host.io?email=a@b#tail@x").as_deref(),
            Some("host.io"),
            "authority ends before path, query, or fragment"
        );
    }

    #[test]
    fn https_host_keeps_safe_none_outcomes_for_other_or_unusable_authorities() {
        // Non-HTTPS (SSH) never invokes the helper → no host to scope.
        assert_eq!(https_host("git@github.com:o/r.git"), None);
        assert_eq!(https_host("ssh://git@github.com/o/r"), None);
        assert_eq!(https_host("httpSx://github.com/o/r"), None);
        assert_eq!(https_host("https:github.com/o/r"), None);
        assert_eq!(https_host("https://"), None);
        assert_eq!(https_host("HTTPS:///path"), None);
        assert_eq!(https_host("hTtPs://?query"), None);
        assert_eq!(https_host("HtTpS://#fragment"), None);
        assert_eq!(https_host("HTTPS://user@/path"), None);
        // IPv6 literal → ungated (None) rather than a wrong match that breaks auth.
        assert_eq!(https_host("hTtPs://[::1]:8443/x"), None);
    }

    #[test]
    fn mixed_case_https_host_gates_real_git_credential_requests() {
        let target_url = "hTtPs://Git.Example.COM:8443/org/repo.git";
        let expected_host = https_host(target_url).expect("mixed-case HTTPS host");
        let helper = git_credential_helper(
            &Credential::userpass("alice", "target-only-secret"),
            Some(&expected_host),
        )
        .expect("valid credential");

        let matched = credential_fill(&helper, &format!("url={target_url}\n\n"));
        assert!(
            matched.status.success(),
            "matching host should receive the helper credential: {}",
            String::from_utf8_lossy(&matched.stderr)
        );
        let matched_stdout = String::from_utf8_lossy(&matched.stdout);
        assert!(matched_stdout.contains("username=alice\n"));
        assert!(matched_stdout.contains("password=target-only-secret\n"));

        let mismatched = credential_fill(
            &helper,
            "protocol=https\nhost=redirect.example\npath=org/repo.git\n\n",
        );
        let mismatched_stdout = String::from_utf8_lossy(&mismatched.stdout);
        let mismatched_stderr = String::from_utf8_lossy(&mismatched.stderr);
        assert!(
            !mismatched_stdout.contains("target-only-secret")
                && !mismatched_stderr.contains("target-only-secret"),
            "a different host must never receive or expose the scoped secret"
        );
        assert!(
            !mismatched_stdout.contains("username=alice"),
            "a different host must receive no scoped username either"
        );
    }

    #[test]
    fn git_credential_helper_is_immune_to_shell_metacharacters() {
        // A hostile username/secret must stay inert: they're carried as env
        // VALUES, and the helper snippet references them only by env-var NAME
        // (double-quoted), so the user-controlled bytes never enter the argv.
        let cred = Credential::userpass("$(rm -rf /); x", "tok'; echo pwned");
        let h = git_credential_helper(&cred, Some("github.com")).expect("valid credential");
        for a in &h.config_args {
            assert!(
                !a.contains("rm -rf"),
                "username metachars reached argv: {a}"
            );
            assert!(!a.contains("pwned"), "secret reached argv: {a}");
        }
        // They are preserved verbatim in the env, where the shell only ever
        // expands them as a quoted variable value.
        let user = h
            .env
            .iter()
            .find(|(k, _)| k == "VCS_TOOLKIT_GIT_USERNAME")
            .unwrap();
        assert_eq!(user.1.expose(), "$(rm -rf /); x");
    }
}
