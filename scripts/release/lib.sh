# shellcheck shell=bash
#
# scripts/release/lib.sh — the single source of truth for the pure release
# DECISIONS the `.github/workflows/release.yml` gate makes: which directory
# holds a crate, each crate's in-workspace (non-dev) dependencies, the publish
# ORDER (dependencies before dependents), the per-crate next-version math, the
# set of crates that must be `cargo package`d to verify a release, and the
# SemVer bump comparison used to reject an insufficient bump.
#
# WHY a sourced library (not inline YAML): the same logic drove three separate
# inline `bash` blocks in release.yml (a DIR map, a DEPS map, the version math),
# so it drifted — e.g. the old verify step's DEPS map omitted `vcs-diff` as a
# direct dependency of vcs-github/vcs-gitlab/vcs-forge. Centralising it here
# means release.yml and the unit tests (`scripts/release/lib.test.sh`) exercise
# the EXACT same code, and `test_manifest_deps_match` pins the dependency map to
# the real Cargo.toml manifests so it can't silently rot again.
#
# Every function here is PURE: inputs come from arguments, the only output is on
# stdout, and nothing touches git, cargo, the network, or the filesystem. That
# is what makes the decisions unit-testable with fixtures. The impure inputs
# (current versions, published tags, cargo-semver-checks output) are gathered by
# release.yml and passed in.
#
# Source it, don't execute it:  source scripts/release/lib.sh
#
# Kept POSIX-friendly (case statements, no `declare -A`, no `local -n`) so it
# runs the same on the ubuntu release runner and a developer's shell — including
# macOS's bash 3.2, which has no associative arrays — since the same file is run
# by `scripts/gate` locally.

# Packaging convention: every crate named by `release_order` has a local LICENSE
# byte-identical to the workspace root and sets `license-file = "LICENSE"`.
# Cargo consequently includes LICENSE in every .crate; CI checks the package list
# and byte identity before this release logic can publish it.
#
# The 12 publishable crates, in publish ORDER: every crate appears AFTER all of
# its in-workspace dependencies. Foundational crates first (vcs-diff,
# vcs-cli-support — the wrappers/facades depend on them), then the wrappers, the
# vcs-forge/vcs-core facades, and finally vcs-watch/vcs-mcp (which depend on
# vcs-core / vcs-core+vcs-forge). vcs-testkit has no workspace deps (any
# position). `test_release_order_is_topological` proves this ordering is valid
# against `crate_deps`, so a future reorder that breaks it fails the tests.
release_order() {
  echo "vcs-diff vcs-cli-support vcs-git vcs-jj vcs-github vcs-gitlab vcs-gitea vcs-forge vcs-testkit vcs-core vcs-watch vcs-mcp"
}

# crate_dir <name> -> the crate's directory under the workspace root (stdout).
# Returns non-zero for an unknown crate so a typo can't silently resolve to "".
crate_dir() {
  case "$1" in
    vcs-diff)        echo crates/diff ;;
    vcs-cli-support) echo crates/cli-support ;;
    vcs-git)         echo crates/git ;;
    vcs-jj)          echo crates/jj ;;
    vcs-github)      echo crates/github ;;
    vcs-gitlab)      echo crates/gitlab ;;
    vcs-gitea)       echo crates/gitea ;;
    vcs-forge)       echo crates/forge ;;
    vcs-testkit)     echo crates/testkit ;;
    vcs-core)        echo crates/core ;;
    vcs-watch)       echo crates/watch ;;
    vcs-mcp)         echo crates/mcp ;;
    *) echo "crate_dir: unknown crate '$1'" >&2; return 1 ;;
  esac
}

# crate_deps <name> -> the crate's DIRECT in-workspace, non-dev dependencies,
# space-separated (empty for a leaf). Mirrors the real `[dependencies]` tables;
# `test_manifest_deps_match` asserts this against every crate's Cargo.toml so it
# stays in sync. dev-dependencies (e.g. vcs-testkit) are deliberately excluded:
# they don't affect publish order or the packaged crate.
crate_deps() {
  case "$1" in
    vcs-diff|vcs-cli-support|vcs-testkit) echo "" ;;
    vcs-git|vcs-jj)      echo "vcs-diff vcs-cli-support" ;;
    vcs-github|vcs-gitlab) echo "vcs-cli-support vcs-diff" ;;
    vcs-gitea)           echo "vcs-cli-support vcs-diff" ;;
    vcs-forge)           echo "vcs-cli-support vcs-github vcs-gitlab vcs-gitea vcs-diff" ;;
    vcs-core)            echo "vcs-git vcs-jj vcs-diff vcs-cli-support" ;;
    vcs-watch)           echo "vcs-core" ;;
    vcs-mcp)             echo "vcs-core vcs-forge" ;;
    *) echo "crate_deps: unknown crate '$1'" >&2; return 1 ;;
  esac
}

# is_semver <string> -> succeeds iff <string> is exactly three dot-separated
# non-empty numeric components (X.Y.Z). Rejects pre-release/build metadata and
# short/long forms — the workflow only ever versions plain X.Y.Z crates.
is_semver() {
  case "$1" in
    ''|*[!0-9.]*) return 1 ;;
  esac
  local M m p rest
  IFS='.' read -r M m p rest <<EOF
$1
EOF
  [ -n "$M" ] && [ -n "$m" ] && [ -n "$p" ] && [ -z "$rest" ]
}

# next_version <current> <bump> -> the next version (stdout), incrementing the
# chosen field and zeroing the lower ones (major: X+1.0.0, minor: X.Y+1.0,
# patch: X.Y.Z+1). This is literal field math — the SemVer GATE
# (cargo-semver-checks, see release.yml) is what proves the chosen field is a
# *legal* bump for the crate's actual API change. Errors (non-zero) on a
# malformed version or unknown bump so bad input aborts the release loudly.
next_version() {
  local cur="$1" bump="$2" major minor patch rest
  if ! is_semver "$cur"; then
    echo "next_version: not a X.Y.Z version: '$cur'" >&2
    return 2
  fi
  IFS='.' read -r major minor patch rest <<EOF
$cur
EOF
  case "$bump" in
    major) echo "$((major + 1)).0.0" ;;
    minor) echo "${major}.$((minor + 1)).0" ;;
    patch) echo "${major}.${minor}.$((patch + 1))" ;;
    *) echo "next_version: unknown bump '$bump' (want major|minor|patch)" >&2; return 2 ;;
  esac
}

# package_set "<planned>" "<published>" -> the crates that must be handed to a
# single `cargo package` invocation to verify a release of <planned>, one per
# line in `release_order`. Both arguments are space-separated crate lists;
# <published> is the crates that already have a crates.io release (a `<crate>-v*`
# tag).
#
# Rule: every planned crate is included; a transitive in-workspace dependency is
# ALSO included iff it is planned (bumped this run) or NOT published — because
# cargo verify-builds the packaged crate against its temporary registry for the
# packaged siblings and against crates.io for everything else. A published,
# not-in-run dependency must therefore be LEFT OUT so it resolves from crates.io
# (its actual published version), exactly as a real `cargo publish` would — while
# in-run/unpublished deps, which aren't on crates.io, come from the local
# packaged set. Traversal only recurses into a dependency that is itself
# included (a published dep is terminal: crates.io already has its whole tree).
#
# Keeping <published> a plain argument (rather than reading git) keeps this a
# pure, fixture-testable decision; release.yml supplies the published list from
# `git tag`.
_ps_add() {
  local n="$1" d
  case "$_ps_seen" in *" $n "*) return ;; esac
  _ps_seen="$_ps_seen$n "
  for d in $(crate_deps "$n"); do
    case " $_ps_planned " in
      *" $d "*) _ps_add "$d"; continue ;;
    esac
    case " $_ps_published " in
      *" $d "*) : ;;            # published & not in-run -> resolve from crates.io
      *) _ps_add "$d" ;;        # in-run/unpublished -> package it locally
    esac
  done
}

package_set() {
  _ps_planned="$1"
  _ps_published="${2:-}"
  _ps_seen=" "
  local p o
  for p in $_ps_planned; do
    _ps_add "$p"
  done
  for o in $(release_order); do
    case "$_ps_seen" in *" $o "*) printf '%s\n' "$o" ;; esac
  done
}

# verify_package_specs "<planned>" "<published>" -> the `cargo package` package
# selectors (`-p <crate>` ...) for verifying a release of <planned>, on one
# line. Passing the whole set to ONE `cargo package` lets cargo resolve in-run
# internal dependencies from its temporary registry instead of failing (or
# degrading to the old dependency-blind `cargo build`).
verify_package_specs() {
  local spec="" c
  for c in $(package_set "$1" "${2:-}"); do
    spec="$spec -p $c"
  done
  printf '%s\n' "${spec# }"
}

# bump_rank <bump> -> a total order for bump magnitudes: patch<minor<major.
# Used to compare a chosen bump against the minimum a change requires.
bump_rank() {
  case "$1" in
    patch) echo 0 ;;
    minor) echo 1 ;;
    major) echo 2 ;;
    *) echo "bump_rank: unknown bump '$1'" >&2; return 2 ;;
  esac
}

# bump_sufficient <chosen> <required> -> succeeds iff <chosen> is at least as
# large as <required>. The SemVer gate uses this to reject e.g. a `patch`
# release over a change that requires `minor`.
bump_sufficient() {
  local rc rr
  rc="$(bump_rank "$1")" || return 2
  rr="$(bump_rank "$2")" || return 2
  [ "$rc" -ge "$rr" ]
}

# required_bump_from_summary <text> -> the minimum legal bump cargo-semver-checks
# reports, as patch|minor|major, or `unknown` when the text carries no verdict
# (a build/baseline error rather than a SemVer decision). cargo-semver-checks is
# version-aware — for a 0.x crate a breaking change is reported as "requires new
# minor version" (0.x breaking == minor per Cargo's SemVer), an addition as
# "requires new patch version" — so its phrasing already names the correct bump
# field. Matching its stable summary wording keeps us decoupled from exit codes
# and lets the gate report the minimum acceptable bump. `unknown` must be
# treated as a HARD failure by the caller (never a silent pass).
required_bump_from_summary() {
  local text="$1"
  if printf '%s' "$text" | grep -qiE 'requires?[[:space:]]+new[[:space:]]+major[[:space:]]+version'; then
    echo major
  elif printf '%s' "$text" | grep -qiE 'requires?[[:space:]]+new[[:space:]]+minor[[:space:]]+version'; then
    echo minor
  elif printf '%s' "$text" | grep -qiE 'requires?[[:space:]]+new[[:space:]]+patch[[:space:]]+version|no[[:space:]]+semver[[:space:]]+update[[:space:]]+required|no[[:space:]]+version[[:space:]]+bump[[:space:]]+(is[[:space:]]+)?required'; then
    echo patch
  else
    echo unknown
  fi
}
