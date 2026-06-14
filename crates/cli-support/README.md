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
- **`is_merge_conflict` / `is_nothing_to_commit` / `is_transient_fetch_error`**
  — classify a returned `processkit::Error` so callers branch on intent
  ("conflict, resolve it"; "nothing to commit, no-op"; "transient, retry")
  instead of matching on error internals.

The wrapper crates re-export the classifiers (e.g. `vcs_git::is_merge_conflict`)
and call `reject_flag_like` with their own binary name, so you rarely name this
crate directly.

Part of [vcs-toolkit-rs](https://github.com/ZelAnton/vcs-toolkit-rs); used by
`vcs-git`, `vcs-jj`, `vcs-github`, and `vcs-core`.

## License

MIT
