# vcs-cli-support

[![crates.io](https://img.shields.io/crates/v/vcs-cli-support.svg)](https://crates.io/crates/vcs-cli-support) [![docs.rs](https://img.shields.io/docsrs/vcs-cli-support)](https://docs.rs/vcs-cli-support) [![downloads](https://img.shields.io/crates/d/vcs-cli-support.svg)](https://crates.io/crates/vcs-cli-support)

Shared plumbing for the CLI-wrapper crates in
[vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs) — the bits
`vcs-git` / `vcs-jj` / `vcs-github` all need that touch
[`processkit::Error`](https://crates.io/crates/processkit), so they live here
rather than in the std-only `vcs-diff`:

- **`reject_flag_like(program, what, value)`** — the injection guard for bare
  positional argv slots: a leading-`-` or empty value is refused before
  anything spawns, so a caller string can't smuggle a flag into argv.
- **`FETCH_ATTEMPTS` / `FETCH_BACKOFF`** — the transient-retry policy for
  `fetch`.
- **`is_merge_conflict` / `is_nothing_to_commit` / `is_transient_fetch_error` /
  `is_lock_contention`** — classify a returned `processkit::Error` so callers
  branch on intent ("conflict, resolve it"; "nothing to commit, no-op";
  "transient, retry"; "another process holds the lock, retry") instead of
  matching on error internals.
- **`ManagedClient` + `RetryPolicy`** — the `CliClient` wrapper the wrappers hold.
  Adds opt-in **lock-contention retry** (exponential, jittered backoff; safe even
  on mutations since lock failures are pre-execution) and opt-in **credential
  injection**.
- **`credentials` module** — supply a secret *per operation* instead of relying on
  ambient CLI auth: the `CredentialProvider` async trait, `Credential`/`Secret`
  (redacted in `Debug`/`Display`), adapters `StaticCredential` / `EnvToken` /
  `provider_fn`, and `git_credential_helper` (feeds a git HTTPS token via an inline
  helper, keeping it out of `argv`). Off by default → ambient auth, unchanged.

```rust
use std::sync::Arc;
use vcs_cli_support::{CredentialProvider, EnvToken};

// A provider reading a token from $CI_TOKEN at request time:
let provider: Arc<dyn CredentialProvider> = Arc::new(EnvToken::new("CI_TOKEN"));
// Hand it to a backend, e.g. `GitHub::new().with_credentials(provider)`.
```

The wrapper crates re-export the classifiers (e.g. `vcs_git::is_merge_conflict`)
and the credential types (e.g. `vcs_github::CredentialProvider`), and call
`reject_flag_like` with their own binary name, so you rarely name this crate
directly.

Part of [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs); used by
`vcs-git`, `vcs-jj`, `vcs-github`, and `vcs-core`.

## License

MIT
