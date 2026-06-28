# Keep bindable — the language-binding contract

> **Status:** decision record / design confirmed as-is. A Python wrapper
> **`vcs-toolkit-py`** is planned (PyO3, modelled on the existing
> [`processkit`](https://crates.io/crates/processkit) ↔ `processkit-py` pair). This
> records *what makes these crates bindable today* so the shape isn't broken by a
> later change, and *what we deliberately did not add* while preparing for it. It is
> a contract, not a backlog — the binding itself lives in a separate repository and
> is **out of scope** here (see "Architecture" below).

## Why this exists

The toolkit is meant to outlive its first consumers: once the API is frozen it
should bind cleanly into other languages without reshaping the Rust surface. The
audit ahead of that freeze found the crates are **already bindable** — no breaking
change was needed, only a few additive niceties and some signposting. The risk now
is *regression*: an innocuous future change (a non-`Clone` DTO, a closure-only entry
point, an error that hides its cause) that quietly makes the binding harder. The
invariants below are the guard rails.

## Architecture — the crate stays binding-agnostic

The binding is a **separate repo** (maturin + PyO3 + `pyo3-async-runtimes`) that
depends on the *published* crates. It — not this workspace — owns:

- the one tokio runtime, and the blocking + `a`-prefixed async surfaces;
- thin `#[pyclass]` newtypes with **manual getters** (not serde across the boundary);
- the map from [`processkit::Error`] to a host-language exception hierarchy (one
  `map_err`).

So this workspace takes **no** PyO3 / `cdylib` / `py` feature, and adds no
binding-only API. The toolkit's job is to stay *shaped* so that wrapper is thin.
(Cf. `wont-do-2026-06.md` W3: no in-crate blocking API — both consumers run tokio,
and the binding owns its own runtime.)

## The invariants to preserve

Each is load-bearing for a thin wrapper; breaking one pushes complexity into the
binding (or makes a clean binding impossible).

| # | Invariant | Why it matters to a binding |
|---|---|---|
| K1 | **Output DTOs are `Clone`, `#[non_exhaustive]`, with public fields/accessors.** | The wrapper copies fields into a `#[pyclass]` via manual getters; a non-`Clone` or private-field DTO forces lifetime juggling or reflection. `#[non_exhaustive]` lets fields be *added* without a breaking release. |
| K2 | **Builder specs are `Clone` + by-value `self -> Self`.** | A binding builds them imperatively (set field, reassign); chained `self -> Self` maps to that, and `Clone` lets the wrapper keep a template. |
| K3 | **Facades carry `<R: ProcessRunner = JobRunner>`; the public boundary is the concrete `JobRunner` monomorphisation.** | PyO3 wraps a concrete type. The default type param means the binding names `Git` / `Forge` / `Repo` (no generics leak); every public signature resolves to a concrete `…<JobRunner>`. |
| K4 | **Every facade has an object-safe `Send + Sync` trait** (`GitApi` / `JjApi` / `ForgeApi` / `VcsRepo`). | `Send + Sync` is required to hold a handle across the runtime the binding owns; object-safety keeps `&dyn`/`Box<dyn>` available. |
| K5 | **One structured `#[non_exhaustive]` error reachable down to [`processkit::Error`], with boolean `is_*` classifiers.** The high-level facades (`vcs-core`/`vcs-forge`/`vcs-watch`) each define a wrapping enum; the wrapper crates re-export `processkit::Error` directly. | The binding maps errors to exceptions in one place: branch on the `is_*` classifiers (`is_not_found` / `is_transient` / `is_unauthorized` / …) and read the structured `processkit::Error` (`program`/`code`/`stdout`/`stderr`) for exception attributes — no `stderr` string-matching. |
| K6 | **`processkit` is re-exported** from `vcs-core` / `vcs-forge` / `vcs-watch` (and `Secret` from `vcs-forge`). | The binding (and any single-crate consumer) names the wrapped error and the token type without a separate `processkit` dependency or a version-skew hazard. |
| K7 | **Async-only I/O; no blocking *variant* of an async operation.** | The binding owns the runtime and exposes both blocking and async itself; an in-crate blocking variant would be dead weight. The only sync I/O is the `Drop`-context `blocking` cleanup helpers (no runtime available there); trivial non-I/O accessors (`kind`/`root`/`cwd`) are of course sync too. |
| K8 | **No closure / callback in a *facade* entry point a binding must drive.** | A host language can't hand Rust a closure across FFI. The one closure form on the facade surface — `Jj::transaction` (and its `JjAt::transaction` twin) — has the **same effect reachable imperatively** via public primitives (`op_head` + `op_restore`), documented on the method. (The published `vcs-cli-support` plumbing has closure-taking helpers like `retry_async`/`provider_fn`; a binding wraps the facades, not those.) |
| K9 | **The test seam that crosses FFI is the runner seam.** | A binding can't implement a Rust trait or drive `mockall` (`wont-do-2026-06.md` W11); it injects a `processkit::ProcessRunner` (`ScriptedRunner` / `RecordingRunner`) via `with_runner` / `from_*`. Keep those constructors public. See the testing guide's "Testing through a language binding (FFI)" section. |
| K10 | **snake_case throughout; no Rust-only idioms in the surface names.** | Names map 1:1 to the binding without translation. |

## What was added for bindability (additive, already shipped)

Small additive niceties from the prep sweep — no breaking change:

- **`vcs_forge::Error::is_unauthorized()` / `is_rate_limited()`** — auth / rate-limit
  classifiers, so the binding maps them to dedicated exceptions (K5).
- **`vcs_watch::Error::is_transient()` / `is_not_found()` / `processkit_error()`** —
  the classifier family + a flattening accessor over the two-level
  `Vcs(vcs_core::Error::Vcs(_))` nesting (K5); `processkit` promoted to a public dep.
- **`Forge::github_with_token` / `gitlab_with_token`** — explicit-token constructors
  taking `impl Into<Secret>` (a host string coerces), plus the `Secret` re-export
  (K2/K6). Gitea is ambient-login-only (no token override in `tea`).

## What we deliberately did *not* add

Investigated during the prep and found unnecessary — recorded so they aren't
re-derived. (Reopen any with a concrete binding demand.)

| # | Candidate | Verdict |
|---|---|---|
| KB1 | A flattened captured-output DTO (`run_captured`) over `ProcessResult<String>`. | **Already covered.** Every wrapper pairs `run` / `run_args` (flattened `Result<String>`) with `run_raw` / `run_raw_args` (`Result<ProcessResult<String>>`). `ProcessResult` is `processkit`'s type, re-exported everywhere and wrapped by `processkit-py`; a vcs-owned DTO would duplicate `run` and introduce a second, inconsistent type. (Cf. K1/K6.) |
| KB2 | A flat public read view over conflict segments. | **Already covered.** git `ConflictRegion` exposes public `ours`/`base`/`theirs` fields; jj `JjConflictRegion::sides()`/`base()` flatten the `#[non_exhaustive]` section enum (and are what `resolve` itself consumes). Both `ConflictSegment` enums are 2-variant and **not** `#[non_exhaustive]`, so a binding matches them trivially. Only theoretical gap (jj per-side *labels*) is display-only and lossy to flatten — defer until asked. |
| KB3 | A non-closure `Jj::transaction` alternative. | **Doc-only (shipped).** The imperative path (`op_head` + best-effort `op_restore`) already exists on the object-safe `JjApi` and is exactly what the closure wraps; it's now documented on the method and in the crate/testing guides (K8). |
| KB4 | A binding-specific testing API. | **Doc-only (shipped).** The runner seam is the bindable one; documented in the testing guide rather than given new code (K9). |

### Adjacent, deferred (not blocking the binding)

The escape-hatch *accessor shape* — `Repo::git()` / `jj()` return borrows
(`Option<&Git>`), and `Forge` has no typed-client accessor — is slightly awkward to
surface as an independent `#[pyclass]`. It is **binding-layer-solvable** (the clients
are re-exported and directly constructible; the backend is already `Arc`-held), so no
new Rust API is added now. Reopen with a concrete need; tracked alongside
[`../ideas/next-forge-surface.md`](../ideas/next-forge-surface.md).

## See also

- [`../crates/testkit/docs/testing.md`](../crates/testkit/docs/testing.md) — the
  testing guide, incl. the FFI section (K9).
- [`wont-do-2026-06.md`](wont-do-2026-06.md) — W3 (no blocking API), W11 (no mockall
  on facade traits).
- The per-crate `# Testing` sections and CHANGELOGs record the additive items above.
