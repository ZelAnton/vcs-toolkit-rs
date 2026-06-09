# next: forge facade surface

> **Status:** open idea (next). From the 2026-06-09 development sweep. The `vcs-forge`
> facade dispatches a lean PR/MR + issues + releases surface across GitHub/GitLab/Gitea,
> returning `Error::Unsupported` where a backend (chiefly `tea`) can't. These are the
> two surface refinements just below the roadmap cut.

## Candidates

### A. Forge capability introspection — `Forge::capabilities()` / `supports(op)`
*Cost: low–moderate · Value: now-ish*

Today a capability gap surfaces only as a runtime `Error::Unsupported` **on the call**
(`tea` lacks `repo_view`, `pr_checks`, `pr_mark_ready`, `release_view`). An agent / MCP
consumer that wants to *hide* an unsupported button has to trial-and-fail. Add an
up-front `capabilities()` (or `supports(ForgeOp) -> bool`) so a consumer can branch
before calling. Mirrors the already-shipped `GitCapabilities` / `JjCapabilities` pattern
on the clients (§5.3) — a consistent precedent.

**Critical:** the risk is over-engineering a matrix that is currently ~4 static facts. A
simple enum-driven `supports(op) -> bool` (const per backend) is enough; a *dynamic
probe* is rejected (see `decisions/wont-do-2026-06.md` W15). Below the roadmap cut only
because `Error::Unsupported` already behaves correctly — this is ergonomics, not a gap.

### B. Per-forge issue/release field-parity audit
*Cost: moderate (investigation-first) · Value: moderate*

Wave A (§7.2) unified issues + releases into shared DTOs (`ForgeIssue`/`ForgeRelease`)
with `Unsupported` where `tea` can't. Audit whether `glab`/`tea` issue/release **fields**
(labels, assignees, milestone, state filters, draft/prerelease) reach parity with the
`gh` surface — or whether the unified DTO silently drops per-backend data, handing
consumers surprising `None`s. The facade's whole value is the unified DTO; quiet field
loss undermines it.

**Critical:** the work is only justified if the audit finds real holes — so it's an
investigation that *may* spawn a small roadmap item, not committed work itself. Pair
with (A): both touch the forge surface and the same wrapper crates.

## Assessment

(A) is the stronger of the two and the first to reconsider — it composes with the shipped
client-capability pattern and gets more valuable as the forge surface grows. (B) is
audit-gated. **Revisit:** when the forge facade next grows, or when a consumer asks to
branch on forge capability.
