# later: .gitignore-aware working-tree filtering in vcs-watch

> **Status:** open idea (later). From the 2026-06-09 development sweep. ROADMAP §6.6
> lists `.gitignore`-aware working-tree filtering as additive future work for `vcs-watch`.

## Candidate

`vcs-watch`'s opt-in working-tree watch scope currently sees every filesystem event under
the tree. Make it honor `.gitignore` (and `.git/info/exclude`, nested ignores) so churn
in ignored paths (`target/`, `node_modules/`, build artifacts) doesn't wake the watcher.

*Cost: moderate · Value: small*

**The win:** less wasted re-query work and quieter event streams for consumers that
enable the working-tree scope.

**Critical assessment:** gitignore semantics are fiddly (precedence, negation,
nested/`**` patterns, per-directory files) — doing it *correctly* is real work, and doing
it *wrong* silently drops events the consumer wanted. Crucially, the **marginal value is
small**: vcs-watch's design already re-queries `snapshot()` and **diffs** against prior
state, so an event in an ignored path that doesn't change tracked state is already a
**no-op** downstream. So this trims wasted re-query CPU, not spurious events.

**Revisit:** if a consumer reports the working-tree scope is too noisy/expensive on a repo
with heavy ignored-path churn — otherwise the diff-based no-op behavior is sufficient.
