# Supported CLI versions

vcs-toolkit-rs drives installed command-line tools; their versions are part of
the runtime contract. The floors below come from each wrapper's
`*Capabilities::is_supported` implementation, not from whichever versions happen
to be installed on a developer machine or a hosted CI runner.

| CLI | Wrapper | Supported floor | Preflight and below-floor result | Real-binary CI coverage |
|---|---|---:|---|---|
| `git` | `vcs-git` | 2.31 | `GitApi::capabilities()` parses `git --version`; `GitCapabilities::ensure_supported()` returns an `Unsupported` process error below 2.31. | Runner-default git on Linux, Windows, and macOS; an additional `ubuntu-22.04` integration leg supplies an older, but not exactly pinned, git. |
| `jj` | `vcs-jj` | 0.38.0 | `JjApi::capabilities()` parses `jj --version`; `JjCapabilities::ensure_supported()` returns an `Unsupported` process error below 0.38.0. | Linux integration matrix: 0.38.0, 0.40.0, 0.42.0; Windows: 0.42.0; weekly drift: the latest stable release resolved at run time. |
| `gh` | `vcs-github` | 2.0.0 | `GitHubApi::capabilities()` parses `gh --version`; `GitHubCapabilities::ensure_supported()` returns an `Unsupported` process error below 2.0.0. | GitHub-hosted runner `gh` is used by authenticated read integration tests; its exact version is not pinned by this repository. |
| `glab` | `vcs-gitlab` | 1.25.0 | `GitLabApi::capabilities()` parses `glab --version`; `GitLabCapabilities::ensure_supported()` returns an `Unsupported` process error below 1.25.0. | The Linux integration and weekly drift lanes install the latest release at run time, best-effort. There is no pinned real-binary floor leg. |
| `tea` | `vcs-gitea` | 0.9.0 | `GiteaApi::capabilities()` parses `tea --version`; `GiteaCapabilities::ensure_supported()` returns an `Unsupported` process error below 0.9.0. | Integration installs latest best-effort; the weekly live-Gitea matrix runs tea 0.9.2 (the available 0.9.x floor representative) and latest. |

All five wrappers also have hermetic version-parser tests for an exact-floor
banner, a below-floor banner, and an unrecognisable banner. Those tests are the
deterministic floor gate; the real-binary lanes catch command/output drift.

## What the gate does

`capabilities()` always runs the CLI's `--version` command and parses a typed
version. A recognised but old version is still returned as a capabilities value:
`is_supported()` is `false`, and `ensure_supported()` turns that into the clear
below-floor error shown in the table. An unrecognisable banner is a parse error;
a missing binary, timeout, or spawn failure remains the corresponding process
error.

For the direct wrapper APIs (`GitApi`, `JjApi`, `GitHubApi`, `GitLabApi`, and
`GiteaApi`), call `capabilities().await?.ensure_supported()?` once during startup
when the application requires a guaranteed floor. Ordinary typed methods do not
all repeat that preflight; without it, an older CLI may instead reject a newer
command or flag in its native error format.

`vcs-forge` adds a stricter boundary around remote mutations:

- `Forge::capabilities()` intersects the CLI's static command surface with its
  parsed version and authentication state. A below-floor or unrecognisable
  version reports `supported: false` and clears every operation flag.
- Mutating facade calls cache one version probe and refuse a *confirmed*
  below-floor CLI with `Error::VersionUnsupported` before the mutation spawns.
  A failed or unparsable probe is fail-open so a changed banner cannot disable an
  otherwise working command; the command's own result remains authoritative.
- Read calls do not use this automatic mutation gate. Consumers that need a hard
  startup guarantee should still use the wrapper preflight above.

The `vcs-core` facade likewise delegates to git/jj and does not silently replace
the explicit wrapper preflight.

## CI coverage

The regular test jobs are hermetic and run on Linux, Windows, and macOS. Their
scripted runners prove parsing, exact argv, and floor/error behavior without
depending on a host CLI version.

The real-binary lanes in [CI](../.github/workflows/ci.yml) add:

- pinned jj floor/mid/top-of-matrix coverage on Linux, plus the floor jj on
  `ubuntu-22.04` for an older runner-default git data point;
- one pinned jj confirmation leg on Windows;
- authenticated runner-provided `gh` reads; and
- best-effort latest `glab` and `tea` smoke tests.

The non-gating [scheduled CLI drift workflow](../.github/workflows/scheduled-cli-drift.yml)
resolves the actual latest jj/glab/tea releases, runs the ignored integration
suites, and reports drift through a tracking issue. Its live-Gitea job separately
checks tea 0.9.2 and latest against a disposable server. "Best-effort" means an
installation outage can skip that CLI's smoke test; the deterministic floor tests
remain mandatory in normal CI.

## Raising a floor

Raise a CLI floor only when the typed surface needs a command/flag/output contract
that cannot be supported safely on the old floor, or when maintaining the old
parser branch would make results ambiguous. In the same change:

1. update the wrapper's `MIN_SUPPORTED` value and exact-floor/below-floor tests;
2. update this matrix, the wrapper guide, and its changelog;
3. move or add a real-binary floor leg where the workflow pins that CLI (jj and
   tea today), and document any remaining runner-default or best-effort gap;
4. run the normal test/docs/public-API gates and the relevant real-binary suite.

The floor is a released compatibility promise, not a recommendation to stay on
that version. Current stable CLIs receive the scheduled drift coverage and are the
preferred choice for new environments.
