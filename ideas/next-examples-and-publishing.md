# next: examples/ and publishing polish

> **Status:** open idea (next). From the 2026-06-09 development sweep. The docs are
> already strong (ProcessKit-grade rustdoc + embedded guides via `include_str!`, a
> task-oriented `docs/cookbook.md`). This is the remaining Rust-publishing-norm polish
> that the roadmap's R6/R7 (community-health files, keywords/categories) don't cover.

## Candidates

### A. `examples/` directories on the lead crates
*Cost: moderate · Value: moderate*

No crate has an `examples/` dir. Add a handful of runnable `examples/*.rs` on the lead
crates (`vcs-core`, `vcs-forge`): prompt-line via `snapshot`, open-a-PR-and-watch-CI,
stash-safe switch. `cargo test` compiles examples, so they cannot rot, and they're more
discoverable from a crates.io page than the (excellent) embedded cookbook.

**Critical:** real **duplication risk** with `docs/cookbook.md`, whose recipes already
cover these flows. Mitigate by keeping the examples thin and pointing their doc-comments
at the cookbook for prose. Worth doing, but it's polish layered on already-good docs —
hence `next`, not `today`.

### B. crates.io page completeness pass
*Cost: trivial · Value: low (after R7)*

Once R7 lands `keywords`/`categories`, do a final per-crate front-page review: confirm
`description` reads well standalone, `readme` renders, the docs.rs badge resolves, and
`homepage`/`documentation` point where intended. A 12-crate spot-check, no code.

## Assessment

(A) is the substance; (B) is a cleanup tail on R7. **Revisit:** (A) right after the
roadmap's R-block, since publishing standards (R6/R7) and examples are the same "look
professional on crates.io" theme; (B) folds into the next release prep.
