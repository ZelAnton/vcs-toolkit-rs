# `decisions/` — settled decisions (won't do / won't change)

This directory holds **closed** decision records: proposals decided *against*, and
designs deliberately confirmed as-is. It is distinct from:

- [`../ROADMAP.md`](../ROADMAP.md) — committed near-term work.
- [`../ideas/`](../ideas/) — **open** proposals to reconsider (`next-` / `later-`).

The split exists so a rejected idea isn't re-derived from scratch, and so the open idea
backlog isn't cluttered with things already settled. A record here is not immutable — a
genuinely **new argument** (a concrete consumer, a changed constraint) can reopen one by
moving its substance back into `ideas/`.

## Contents

- **`wont-do-2026-06.md`** — the consolidated "won't do" register. Migrates the
  "Deliberately out of scope" and "Consciously rejected" lists that used to live inline in
  `ROADMAP.md`, plus the rejections from the 2026-06-09 development sweep. Each carries a
  one-line reason so reviewers don't re-litigate.
