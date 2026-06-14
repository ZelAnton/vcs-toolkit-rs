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
use processkit::Result;

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
        match std::env::var(&self.var) {
            Ok(value) if !value.is_empty() => Ok(Some(match &self.username {
                Some(user) => Credential::userpass(user.clone(), value),
                None => Credential::token(value),
            })),
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
        (self.0)(request)
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
/// The username/secret must not contain a newline: git's credential protocol is
/// line-based, so an embedded `\n` is read as the end of the value (git truncates
/// there). Real tokens and usernames never contain one.
#[must_use]
pub fn git_credential_helper(cred: &Credential) -> GitCredentialHelper {
    let username = cred.username().unwrap_or(DEFAULT_GIT_USERNAME).to_string();
    // Reference the values by env-var NAME inside the snippet, so `argv` never
    // carries the secret. Respond only to git's `get` action; ignore store/erase.
    let helper = format!(
        "!f() {{ test \"$1\" = get && printf 'username=%s\\npassword=%s\\n' \
         \"${GIT_USERNAME_VAR}\" \"${GIT_PASSWORD_VAR}\"; }}; f"
    );
    GitCredentialHelper {
        config_args: vec![
            "-c".to_string(),
            "credential.helper=".to_string(),
            "-c".to_string(),
            format!("credential.helper={helper}"),
        ],
        env: vec![
            (GIT_USERNAME_VAR.to_string(), Secret::new(username)),
            (GIT_PASSWORD_VAR.to_string(), cred.secret().clone()),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn git_credential_helper_keeps_secret_out_of_argv() {
        let cred = Credential::userpass("alice", "s3cr3t");
        let h = git_credential_helper(&cred);
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
        let h = git_credential_helper(&Credential::token("t"));
        let user = h
            .env
            .iter()
            .find(|(k, _)| k == "VCS_TOOLKIT_GIT_USERNAME")
            .unwrap();
        assert_eq!(user.1.expose(), DEFAULT_GIT_USERNAME);
    }

    #[test]
    fn git_credential_helper_is_immune_to_shell_metacharacters() {
        // A hostile username/secret must stay inert: they're carried as env
        // VALUES, and the helper snippet references them only by env-var NAME
        // (double-quoted), so the user-controlled bytes never enter the argv.
        let cred = Credential::userpass("$(rm -rf /); x", "tok'; echo pwned");
        let h = git_credential_helper(&cred);
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
