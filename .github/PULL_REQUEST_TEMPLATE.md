<!-- Thanks for contributing! Keep the summary focused on *what changed and why*. -->

## What & why

<!-- A real summary of the change and its motivation. Link any related issue. -->

## Checklist

- [ ] `cargo fmt --all`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean
- [ ] `cargo test --workspace --all-features` passes (and `cargo test -- --ignored` if you
      touched a real-binary path)
- [ ] The affected crate's `CHANGELOG.md` `[Unreleased]` is updated when the change is
      user-facing (`Added` / `Changed` / `Fixed`)
- [ ] Docs updated (rustdoc and the `docs/` guide set) if behavior or API changed
- [ ] New dependencies carry a "why" comment in `Cargo.toml`

## Notes for reviewers

<!-- Anything non-obvious: a platform caveat, a CLI-version quirk, a trade-off, a follow-up deferred to ideas/. -->
