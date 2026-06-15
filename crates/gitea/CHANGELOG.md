# Changelog — vcs-gitea

All notable changes to the `vcs-gitea` crate are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
This crate is versioned and published independently of the other workspace
crates; tag releases as `vcs-gitea-v<version>`.

## [Unreleased]

### Added
- `pr_comment(dir, number, body)` — add a comment to a pull request,
  returning the command's output (`tea comment <index> <body>`). Gitea PRs
  and issues share the `index` space and the same `tea comment` subcommand
  hits both. The `body` is a bare positional, so it is argv-guarded with
  `reject_flag_like` (a leading `-` or empty value is rejected before any
  process spawns) — the first such guard in this crate.
- `pr_edit(dir, number, PrEdit)` — edit a pull request's title and/or
  description (`tea pr edit <index> [--title <title>] [--description <body>]`).
  A new `PrEdit` builder (`new()`, `.title(..)`, `.body(..)`) carries the
  optional fields; absent flags are not emitted. An empty string is treated
  as a real value (tea clears the field on `--title ""` / `--description ""`),
  not as `None`. The trait methods are **defaulted** to `Error::Unsupported`
  so external implementers keep compiling when the crate bumps — only the
  `Gitea` concrete impl and the regenerated `MockGiteaApi` override them.
- `vcs-cli-support` added as a direct dependency (for `reject_flag_like`,
  needed by `pr_comment`).

### Changed
- Documented that **Gitea authentication is ambient**: unlike the new
  `vcs-github`/`vcs-gitlab` per-operation `with_credentials` token providers,
  `tea` has no non-interactive per-invocation token mechanism (it authenticates
  from `tea login add` only), so `Gitea` offers no credential injection.
  `vcs-cli-support`'s `CredentialService::Gitea` is reserved for if/when `tea`
  gains env-token support.
- Bumped `processkit` to **0.11.0** (from 0.9.1), a major breaking release ahead
  of processkit's 1.0 freeze. Breaking for downstream via the re-exported
  `processkit::Error`: `Error::Timeout`/`Signalled` now carry partial
  `stdout`/`stderr`, `Error::Signalled`/`NotFound`/`CassetteMiss` are first-class
  variants, the blanket `From<io::Error>` is gone, and `Invocation::cwd` is now
  `Option<PathBuf>`.

### Removed
- The **`cancellation`** feature — cancellation is always available now
  (processkit 0.10 made it core), so the `cli_client!`-generated
  `default_cancel_on(token)` and the re-exported `CancellationToken` no longer sit
  behind a feature. Downstream that enabled `vcs-gitea/cancellation` should drop it.

### Fixed
- `pr_view` reports a *possible page-miss* when its `tea pr list --limit 999`
  listing fills the cap and the requested number isn't found — distinguishing
  "this PR exists but is beyond the page" from a flat "no such PR" on a very large
  repo (instead of an indistinguishable not-found either way).
- `Release` JSON parsing gained `serde` **aliases** for the cleaned/camelCase/raw
  key forms (`tag_name`/`tag-name`/`tagName`/`Tag-Name`, `published_at`/…) alongside
  tea's current quirky `toSnakeCase` keys (`tag-_name`, `published _at`), so a
  future `tea` that fixes its header casing doesn't silently break release parsing.
- The typed single-issue view (`tea issues <n>`) tolerates a JSON `null`
  `body`/`url` (an issue with no description) instead of failing the whole parse —
  `#[serde(default)]` only covered an absent key, not a present `null`.

## [0.1.0] - 2026-06-08

### Added
- Initial release: `GiteaApi` trait + `Gitea` client wrapping the `tea` CLI,
  mirroring `vcs-github`'s shape (async, `#[non_exhaustive]` DTOs, the structured
  `processkit::Error`, the `mock` feature → `MockGiteaApi`, and the
  `Gitea::with_runner` scripted-runner seam).
- The **lean pull-request lifecycle** `tea` supports: `auth_status` (a non-empty
  `login list`), `pr_list` (`PullRequest`), `pr_view` (synthesized by listing
  with `--state all` and filtering by number — `tea` has no single-PR view),
  `pr_create(PrCreate)`, `pr_merge(number, MergeStrategy)`
  (`--style merge|rebase|squash`), and `pr_close`.
- **Issues and releases**: `issue_list` (`Vec<Issue>`), `issue_view(number)` (the
  first-class `tea issues <n>` single-issue view), `issue_create(title, body)`,
  and `release_list` (`Vec<Release>`). No `release_view` — `tea releases` always
  lists.
- Raw escape hatches `run`/`run_raw` (+ inherent `run_args`/`run_raw_args`), and
  a `Gitea::at(dir)` → `GiteaAt` bound view mirroring every repo-scoped method.

### Notes
- Deliberately narrower than `vcs-github`/`vcs-gitlab`: `tea` exposes no
  current-repo view, no draft toggle, no PR-checks command, and no single-release
  view, so `repo_view`, `pr_mark_ready`, `pr_checks`, and `release_view` are
  absent (the `vcs-forge` facade reports them as `Unsupported` for the Gitea
  backend).
- **`tea --output json` is modeled, not the Gitea REST API.** Its **list**
  commands emit tea's print-*table* (a JSON array of string-maps; snake-cased
  column-header keys that can contain spaces/slashes; **all values strings**; no
  `html_url`, no nested branch objects), and its **detail** view (`issues <n>`) a
  separate *typed* object. The parsers select columns with `--fields` and
  string-parse the `index`. Consequences: a PR's merge state rides the `state`
  column (`"merged"`), and a `Release` carries **no web URL** (`tea releases`
  exposes only a tar/zip download URL, not surfaced). **This contract is derived
  by reading tea's source (`gitea.com/gitea/tea` `main`; the `PullFields`/
  `IssueFields` sets confirmed identical on the released v0.14.1), not validated
  end-to-end** — confirm it against a live `tea` (the `#[ignore]` integration
  tests in `tests/cli.rs`) before the first release.

### Changed
- Bumped `processkit` to **0.8** — the re-exported `Error`/`ProcessResult` carry
  through 0.8 (`Error` still `#[non_exhaustive]` with `NotReady`/`Unsupported` and
  feature-gated `Cancelled`/`ResourceLimit`; `Error::Exit` Display gained a
  stderr-tail suffix; `Command` is `#[must_use]`). **Breaking** for consumers that
  match the re-exported types exhaustively, or that bump their own direct
  `processkit` separately — caret `"0.7"` does not span 0.8, so bump together.
- Internal: the `CliClient` verbs the wrapper bodies call were renamed to one
  shared vocabulary (`text`→`run`, `capture`→`output`, `unit`→`run_unit`,
  `code`→`exit_code`); no public-API or built-argv change.
- New off-by-default **`cancellation`** feature: pulls in processkit's
  `cancellation`, so `cli_client!` emits `default_cancel_on(token)` on the client —
  build a cancellable client (every command it runs dies when the token fires) and
  pass it through the facade. No new vcs-* API; `CancellationToken` is re-exported
  from `processkit`.
- `auth_status` tolerates a non-zero `tea login list` exit (e.g. no config file
  yet) and reports `false` instead of erroring, matching its "reports the bool,
  must not error" contract.
- `pr_create` doc: tea prints a textual summary (no URL) and has no flag to
  shape the create output — documented instead of implied parity with gh/glab.
- `pr_create` now takes a `PrCreate` spec
  (`PrCreate::new(title, body).head(…).base(…)`) instead of positional
  `title, body, head, base` arguments, mirroring `vcs-git`'s `GitPush` builder
  style. The built argv is unchanged.

### Fixed
- `pr_list` passes `--limit 100` (tea's default page of 30 silently truncated
  larger sets), and `pr_view` — which lists and filters by number — uses
  `--limit 999`, so a PR beyond the first page is no longer a false "not found"
  (PRs beyond 999 still are; documented).

[Unreleased]: https://github.com/ZelAnton/vcs-toolkit-rs/compare/vcs-gitea-v0.1.0...HEAD
[0.1.0]: https://github.com/ZelAnton/vcs-toolkit-rs/releases/tag/vcs-gitea-v0.1.0
