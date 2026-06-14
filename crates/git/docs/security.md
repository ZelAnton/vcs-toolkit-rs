# Security & hardening guide

These wrappers shell out to the real `git` / `jj` / `gh` — so the threats are the
CLI's, not a reimplemented protocol's: a caller-supplied string that smuggles a
flag into argv, and a repository you didn't create whose hooks and config run
arbitrary code the moment you touch it. Two layers answer those, both **on by
default or one call away**:

- **Injection guards** — automatic, in every typed method. Nothing to opt into.
- **`Git::hardened()`** — one constructor for the untrusted-repo case.

Pre-validation at your input boundary (the [newtypes](#validating-newtypes-eager-at-your-input-boundary))
is the optional third layer, for failing fast on bad input *before* it reaches a
method.

A separate, opt-in concern is **supplying** a credential rather than guarding
against one — see [Credential provisioning](#credential-provisioning-opt-in) below.

---

## Injection guards (automatic)

Every exposed positional argument — branch/tag/bookmark names, revisions,
revsets, ranges, remotes, operation ids, clone/fetch endpoints — is checked
**before anything spawns**: a value that is empty or begins with `-` is refused,
because `git`/`jj` would parse a leading-`-` string as a *flag* rather than the
name you meant. That is the whole attack: a caller string like
`--upload-pack=/bin/evil` in a remote slot, or `--config=core.pager=…` in a
revset slot, would otherwise run an arbitrary program. The guard makes the
smuggle impossible at the argv level, so it holds regardless of how the value
reached you.

A rejected argument surfaces as a spawn-side **`processkit::Error::Spawn`** —
the same variant a missing binary produces — carrying the program name and an
`InvalidInput` IO source describing the rejected value. (It is raised *instead*
of spawning, not by the child.)

```rust,ignore
# use vcs_git::{Git, GitApi};
# async fn demo(git: &Git, repo: &std::path::Path) {
// A caller-supplied branch name that starts with `-`:
let err = git.checkout(repo, "--upload-pack=/bin/evil").await.unwrap_err();
assert!(matches!(err, processkit::Error::Spawn { .. })); // never spawned
# }
```

What is **not** guarded, by design:

- **Flag-value slots** (`-m <msg>`, `--branch <b>`, `-r <revset>`) — the CLI
  itself rejects a dash-value there with a clear error rather than misparsing it.
- **Filesystem path arguments** — `--`-separated pathspecs, worktree paths,
  clone destinations. These are typed `Path` and caller-trusted; git's `--`
  separator keeps even a `-dash.txt` literal.
- **The `run` / `run_raw` escape hatches** — you build the whole argv, so you own
  its safety.

One hard rule on top: **never compose commands through a shell**
(`sh -c "git … | grep …"`) — that reopens the entire injection surface the
guards close. If output composition is ever genuinely needed, processkit 0.7's
`Command::pipe` chains commands in one kill-on-close group with no shell in
between; until then, parse in-process like the wrappers do.

## Validating newtypes (eager, at your input boundary)

The guards above run inside each method. When you take a name or revision from a
UI, bot, or agent and want to reject it *at the boundary* — before it flows
through your code — validate it up front with a newtype. These are **optional**:
method signatures stay `&str` and guard internally either way; the newtypes are
for early, explicit validation, not a required wrapper.

When to reach for one — a short decision note:

- **`&str` straight through** when the value is program-internal (a constant, a
  name you just listed from the repo): the in-method guard at the spawn edge is
  the only check needed.
- **`RefName` / `RevSpec` / `RevsetExpr`** when the value crosses a trust
  boundary *early* and an invalid one should fail with context at intake (an
  HTTP handler, an MCP/agent tool argument, config parsing) rather than three
  layers down at spawn time — validate once, then pass `.as_str()` everywhere.

`vcs-git`:

```text
pub fn RefName::new(name: impl Into<String>) -> Result<Self>  // signature shape
pub fn RevSpec::new(rev:  impl Into<String>) -> Result<Self>
```

- **`RefName`** follows the load-bearing core of `git check-ref-format`:
  non-empty; no leading `-` or `.`; no `..`; no control characters or space; none
  of `~ ^ : ? * [ \`; no trailing `/` or `.lock`.
- **`RevSpec`** is deliberately minimal — git's revision grammar is too rich to
  validate here — so it only guarantees non-empty and no leading `-`, matching
  the internal guard.

`vcs-jj`:

- **`RevsetExpr`** mirrors `RevSpec`: non-empty, no leading `-`. (jj's revset
  grammar is likewise too rich to validate further.)

```rust,ignore
use vcs_git::RefName;

# fn demo() -> Result<(), processkit::Error> {
let name = RefName::new("feature/login")?;   // Ok — validated once, here
assert!(RefName::new("-x").is_err());        // leading `-`
assert!(RefName::new("a..b").is_err());      // `..`
assert!(RefName::new("").is_err());          // empty
// Pass the inner &str to any method:
# async fn use_it(git: &vcs_git::Git, repo: &std::path::Path, name: RefName)
#   -> Result<(), processkit::Error> {
git.checkout(repo, name.as_str()).await?;
# Ok(()) }
# Ok(()) }
```

A rejected newtype returns the same `Error::Spawn { program, source }` shape the
in-method guard uses — so a value that passes `RefName::new` will never be
rejected later for flag-shape.

## `Git::hardened()`

Running `git` inside a repository you didn't create is **arbitrary code
execution by default**: git fires that repo's hooks and honours its config on
ordinary commands. The hardened profile neutralises that, applying the same
settings to **every** command the client runs:

- **Disables hooks** — `core.hooksPath=/dev/null`, pinned through git's
  env-based config (`GIT_CONFIG_COUNT` / `GIT_CONFIG_KEY_n` / `GIT_CONFIG_VALUE_n`;
  verified to suppress hooks on Windows too) — and `core.fsmonitor=false` (a
  config-driven daemon launch).
- **Scrubs repo-redirecting `GIT_*` variables** so a poisoned parent environment
  can't point a command at another repository: `GIT_DIR`, `GIT_WORK_TREE`,
  `GIT_INDEX_FILE`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`,
  `GIT_NAMESPACE`, `GIT_CEILING_DIRECTORIES`, `GIT_CONFIG_PARAMETERS`,
  `GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM`.
- **Skips system config** (`GIT_CONFIG_NOSYSTEM=1`) and keeps terminal prompts
  off everywhere (`GIT_TERMINAL_PROMPT=0`).

```rust,ignore
use vcs_git::Git;

let git = Git::hardened();        // == Git::new().harden()
// Every command this client runs carries the profile above.
```

It is chainable, so it composes with a runner in tests
(`Git::with_runner(rec).harden()`) and with a deadline
(`Git::hardened().default_timeout(…)`).

What it does **not** do: sandbox the git binary itself, or stop the repo's
*content* from being malicious.

**jj needs no equivalent.** jj has no repo-local hooks, and its config comes from
the user/repo TOML files jj itself trusts — there is deliberately no
`Jj::hardened()`. In a **colocated** repo the risk lives entirely on the git
side (git hooks fire only when *git* commands run there), so harden the `Git`
client you point at it and leave `Jj` plain.

## Untrusted file content

The conflicted-file parsers treat their input as arbitrary bytes: `vcs_git::conflict`
and `vcs_jj::conflict` turn marker soup into structured regions and **never
panic** on malformed or hostile input — a bad file is an `Error::Parse`, not a
crash. This is property-tested for panic-freedom on arbitrary input, alongside a
byte-exact `render(parse(x)) == x` roundtrip. See the [conflicts guide](https://docs.rs/vcs-git/latest/vcs_git/guide/conflicts/).

## Credential provisioning (opt-in)

By default the toolkit holds **no** secrets — every backend authenticates through
its CLI's own ambient credential system (git credential helpers, the SSH agent,
`gh`/`glab` logins). When you instead want to supply a secret *per operation* (a CI
job's short-lived token, a vault lookup, per-account routing), attach a
`CredentialProvider` with `Git::with_credentials(...)` (and `GitHub`/`GitLab` have
the same method). It is **opt-in** — without a provider, behaviour is unchanged.

The security properties that make this safe:

- **The secret never reaches `argv`.** For git HTTPS, the provider's token is fed
  through an inline `credential.helper` that reads it from an environment variable
  *by name*; only the variable name appears in the command line, never the value.
  (The forges inject `GH_TOKEN`/`GITLAB_TOKEN` as environment variables, also not
  `argv`.) `argv` is broadly observable (`ps`, `/proc/<pid>/cmdline`); the process
  environment is same-user only — the right channel for a secret.
- **It is never persisted.** The git helper answers only git's `get` action (never
  `store`/`erase`), so the token is never written to a credential cache or to any
  config file. It lives only in the child process's environment, for that one call.
- **It can't leak through logs.** Secrets are wrapped in `Secret`, which redacts
  itself in `Debug`/`Display`; processkit's own command/error formatting shows
  environment-variable *names* only, never values.
- **HTTPS only.** git invokes a credential helper for HTTP(S) auth only, so an SSH
  remote ignores it and falls through to the ambient SSH agent, as before.

`vcs-gitea` and `vcs-jj` stay ambient-only: `tea` has no per-invocation token
mechanism, and jj's in-process git backend offers no per-operation override.

## See also

- [git guide](https://docs.rs/vcs-git/latest/vcs_git/guide/) — the full `GitApi` surface and the hardened profile in context.
- [`vcs-cli-support` credentials module](https://docs.rs/vcs-cli-support/latest/vcs_cli_support/credentials/) — the `CredentialProvider` seam and `git_credential_helper`.
- [jj guide](https://docs.rs/vcs-jj/latest/vcs_jj/guide/) — why there is no `Jj::hardened()`, and the colocated-repo story.
- [Process model & errors](https://docs.rs/vcs-core/latest/vcs_core/guide/process_model/) — `Error::Spawn` and the other
  variants the guards raise, plus containment and observability.
