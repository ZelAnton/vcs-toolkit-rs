# Security policy

## Reporting a vulnerability

Please report security issues **privately** — do not open a public issue for a
vulnerability.

Use GitHub's private vulnerability reporting:
**[Report a vulnerability](https://github.com/ZelAnton/vcs-toolkit-rs/security/advisories/new)**
(Security → Advisories → *Report a vulnerability* on the repository).

Include, as far as you can: the affected crate and version, the platform
(Windows / Linux / macOS), which CLI and version is involved
(`git` / `jj` / `gh` / `glab` / `tea`), a description of the issue, and a minimal
reproduction. You can expect an acknowledgement within a few days; a fix and
coordinated disclosure follow once the issue is confirmed.

## Why these crates are security-relevant

vcs-toolkit **executes the installed `git` / `jj` / `gh` / `glab` / `tea` binaries
as subprocesses**, often against repositories and remotes the caller did not
create. That exposes a few sensitive surfaces, and a bug in any of them is treated
as a security issue, not just a functional one:

- **Argument injection.** Caller-supplied names, revisions, revsets, refspecs, and
  endpoints are placed in argv. Every bare positional is guarded before spawning (a
  value that is empty or begins with `-` is refused), and validating newtypes
  (`RefName` / `RevSpec` / `RevsetExpr`) are offered. A path that lets a caller
  smuggle a flag into a command is a vulnerability.
- **Untrusted repositories.** Running `git` inside a repo you didn't create can
  execute that repo's hooks and honour its config. `Git::hardened()` disables hooks
  and `core.fsmonitor`, scrubs repo-redirecting `GIT_*`, and skips system config; a
  bypass of that profile is a vulnerability.
- **Process containment.** Every command runs inside an OS job (Windows Job Object
  / Linux cgroup v2 / POSIX process group, via
  [`processkit`](https://crates.io/crates/processkit)) so a subprocess tree can't be
  orphaned; a containment escape is a vulnerability.

## Supported versions

The crates are pre-1.0 and **versioned independently**. Only the **latest published
version of each crate** on [crates.io](https://crates.io/) receives security fixes;
please reproduce on the latest releases before reporting.
