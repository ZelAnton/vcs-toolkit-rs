---
name: Bug report
about: Report incorrect behavior (a wrong result, a leaked process, a panic, a parse error, …)
title: ""
labels: bug
assignees: ""
---

**What happened**
A clear description of the bug.

**Expected behavior**
What you expected instead.

**Affected crate**
<!-- vcs-git / vcs-jj / vcs-github / vcs-gitlab / vcs-gitea / vcs-core / vcs-forge /
     vcs-watch / vcs-mcp / vcs-diff / vcs-cli-support / vcs-testkit -->

**Reproduction**
A minimal snippet or steps. The smaller, the faster it gets fixed.

```rust
// minimal repro
```

**Environment**
- Crate + version:
- OS + version: <!-- Windows / Linux (distro) / macOS / BSD -->
- Rust version (`rustc --version`):
- Underlying CLI + version, if relevant: <!-- `git --version` / `jj --version` / `gh` / `glab` / `tea` -->
- Relevant feature flags: <!-- mock, tracing, serde, stream, … -->

**Additional context**
Logs (the `tracing` feature, if enabled — but **never paste secrets / tokens / argv / env**),
stack traces, or anything else useful.
