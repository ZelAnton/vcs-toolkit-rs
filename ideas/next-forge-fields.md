# next: richer forge DTO fields (labels / assignees / author / timestamps)

> **Status:** open idea (next). Spun off from the Tier-2 forge surface wave
> (2026-06-14), which expanded `ForgeRelease` (body/draft/prerelease) and populated
> `ForgeIssue` body/url, but **deferred** the people/metadata fields below. All are
> **additive on the `#[non_exhaustive]` DTOs**, so they can land later without a
> breaking release — which is exactly why they weren't rushed.

## Candidates (from the 2026-06-14 field-parity audit)

| Field | gh | glab | tea | Notes |
|---|---|---|---|---|
| `labels` (PR + issue) | yes (extra `--json`) | yes | **no** | empty on tea; nested JSON (`[{name}]`) to flatten |
| `assignees` (PR + issue) | yes (extra `--json`) | yes | **no** | empty on tea; nested (`[{login/username}]`) |
| `author` (PR/issue/release) | yes (extra `--json`) | yes | **no** | login/username string |
| `created_at` / `updated_at` | yes (extra `--json`) | yes (default) | **no** | ISO-8601 strings |
| `milestone` (PR + issue) | yes (extra `--json`) | yes | **no** | title string |

## Why deferred (not dropped)

- **Uneven support.** Every candidate is gh+glab-capable but **tea can't supply
  them** (its `--fields` print-table has no such columns). That means honest
  `Vec`/`Option` fields that are *always empty on Gitea* — the "surprising empty"
  the facade is supposed to avoid. Acceptable **if documented**, but it's a real
  asymmetry to weigh, not a free win.
- **Cost.** gh needs widened `--json` field lists (a real per-call cost) and the
  github/gitlab parsers need to flatten nested `[{name}]`/`[{login}]` arrays into
  `Vec<String>` — a focused parsing mini-project best done as its own wave.
- **Additive later.** The DTOs are `#[non_exhaustive]`; adding these fields is
  **non-breaking**, so there's no pre-1.0 deadline forcing them now.

## When to pick up

When a consumer (vcs-flow-rs / agent-workspace) actually needs label/assignee-based
PR/issue triage through the facade. Start with **`labels` + `assignees`** (the
highest-demand pair), populate on gh (widen fields + flatten) and glab, document
"empty on Gitea", and add a per-backend mapping test. `author`/timestamps/milestone
follow the same pattern if demand appears.
