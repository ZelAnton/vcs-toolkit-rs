# `ideas/` — open proposals not yet committed

This directory holds **open** development ideas: things worth doing eventually but
not committed to the near-term [`ROADMAP.md`](../ROADMAP.md). Each file is a small
decision record (status header → candidates with cost/value → assessment → revisit
condition).

## The four buckets

A development sweep classifies every candidate into one of four homes:

| Bucket | Meaning | Lives in |
|---|---|---|
| **Today** (сегодня) | Committed; will do | [`../ROADMAP.md`](../ROADMAP.md) → "Active roadmap" |
| **Next** (завтра) | Open; reconsider **first** when the roadmap drains | `ideas/next-*.md` |
| **Later** (потом) | Open; further out, or gated on a concrete consumer | `ideas/later-*.md` |
| **Won't do** | Settled against (or won't change) | [`../decisions/`](../decisions/) |

"Завтра / потом" are hyperbole for ordering, not calendar dates — **next-** items
are simply the first re-examined once committed work is done.

## Filename marker

The horizon is encoded in the **filename prefix**:

- `next-<topic>.md` — reconsider first (high value, just below the cut).
- `later-<topic>.md` — further out, or gated on a concrete consumer / upstream release.

When an idea graduates to committed work, move its substance into `ROADMAP.md` and
either delete the file or leave a one-line pointer. When an idea is rejected
outright, move it to [`../decisions/`](../decisions/).

## Current contents

**Next:**
- `next-forge-surface.md` — forge `capabilities()`/`supports(op)` introspection;
  per-forge issue/release field-parity audit.
- `next-examples-and-publishing.md` — `examples/` dirs on the lead crates
  (CI-compiled), and the remaining crates.io publishing polish.
- `next-mcp-http-transport.md` — an HTTP/SSE transport for `vcs-mcp` (stdio-only today).

**Later:**
- `later-new-backends.md` — a 4th forge (Bitbucket/Forgejo) as an extensibility
  proof; a new-VCS-backend (hg/pijul) feasibility spike.
- `later-upstream-gated.md` — adopt processkit streaming / persistent cat-file
  sessions once they ship **and** a consumer needs them (specs already delivered).
- `later-watch-gitignore.md` — `.gitignore`-aware working-tree filtering in `vcs-watch`.

> This sweep deliberately committed **7** high-conviction items to the roadmap rather
> than padding to a round number — the toolkit is unusually mature for pre-release, so
> the bar for "today" is high. See [`../decisions/wont-do-2026-06.md`](../decisions/wont-do-2026-06.md)
> for what was settled against.
