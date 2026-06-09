# later: new backends (extensibility proofs)

> **Status:** open idea (later). From the 2026-06-09 development sweep. The toolkit's
> extension seams (forge `Backend` enum; the git/jj `VcsRepo` facade) are *designed* to
> admit new backends but have never been exercised with a real third one. These are the
> demand-gated exercises that would validate — or refute — the extensibility claims.

## Candidates

### A. A 4th forge (Bitbucket / Forgejo) as an extensibility proof
*Cost: high · Value: high signal, demand-gated*

The forge `Backend` is a closed trio `{GitHub, GitLab, Gitea}`; adding a forge = a new
wrapper crate + an enum arm + the per-method match arms across `Forge`. The **only** way
to validate the "easy to add a forge" claim is to do it once. It would also settle an
open architecture question with evidence: is the per-method match-dispatch boilerplate
(`crates/forge/src/lib.rs`) tolerable, or does a 4th backend finally justify a refactor?
(Note `decisions/wont-do-2026-06.md` W5/W6/W14 reject codegen / a `Backend` trait / a
shared `facade_trait!` crate *at the current scale* — a real 4th backend is exactly the
"new argument" that could reopen them.)

**Critical:** high cost — a whole wrapper crate plus empirically reverse-engineering yet
another CLI's output (the `tea` experience, and the gitea-JSON-shape bug, show how
painful that is). **Zero current demand.** Highest-signal extensibility exercise, but
horizon and demand-gated.

### B. A new VCS backend (hg / pijul) — feasibility spike, not implementation
*Cost: very high · Value: tests the deepest assumption*

`VcsRepo`/`Repo` is closed to git/jj. Spike whether a third VCS model even fits the
facade's deliberately-honest least-common-denominator (§"out of scope" #2: the
merge/op-rollback divergence is *why* the facade stays LCD). Likely outcome is a
**write-up** ("here's why a 3rd backend would / wouldn't fit") rather than code.

**Critical:** the facade's LCD design may simply not stretch to a third model, and there
is no consumer. Pure horizon — a spike/doc, explicitly not an implementation commitment.

## Assessment

Both are demand-gated. (A) is the more likely to ever happen (forges are additive and
self-contained); (B) is closer to a research note. **Revisit:** when a concrete consumer
needs Bitbucket/Forgejo (A), or when someone seriously proposes a non-git/jj VCS (B).
