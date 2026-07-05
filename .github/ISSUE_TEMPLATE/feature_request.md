---
name: Feature request
about: Suggest a capability or ergonomic improvement
title: ""
labels: enhancement
assignees: ""
---

**The problem / use case**
What are you trying to automate that's hard or impossible today?

**Proposed solution**
What would the API or behavior look like?

```rust
// sketch of the API you'd want
```

**Scope check**
vcs-toolkit is a set of **thin, typed wrappers** over the real
`git` / `jj` / `gh` / `glab` / `tea` binaries — it reflects each tool's exact
behavior rather than reimplementing it, and the facades stay an honest
least-common-denominator. Does this fit that scope? Search the existing issues
first — it may already be planned, parked, or considered-and-declined.

**Alternatives considered**
Other approaches, prior art in other libraries, or workarounds you've tried.
