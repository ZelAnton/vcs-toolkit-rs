#!/usr/bin/env bash
#
# scripts/release/lib.test.sh — unit fixtures for the pure release decisions in
# `scripts/release/lib.sh` (the version math, the dependency graph + publish
# order, the `cargo package` verify set, and the SemVer bump comparison the
# release gate makes). Run by `scripts/gate` and by CI (`release-lib` job) so a
# change to the release logic that breaks a decision is caught BEFORE it can
# mis-publish — a semver-breaking patch or a crate packaged out of dependency
# order — on crates.io, which is irreversible.
#
# Pure functions only: no git/cargo/network. `test_manifest_deps_match` is the
# one test that reads real files — it re-derives each crate's in-workspace
# dependencies from its Cargo.toml and asserts `crate_deps` matches, so the map
# can't drift from the manifests.
#
# Usage: bash scripts/release/lib.test.sh   (exit 0 = all passed)

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck source=scripts/release/lib.sh
. "$SCRIPT_DIR/lib.sh"

PASS=0
FAIL=0

# assert_eq <label> <expected> <actual>
assert_eq() {
  local label="$1" expected="$2" actual="$3"
  if [ "$expected" = "$actual" ]; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    printf 'FAIL: %s\n  expected: [%s]\n  actual:   [%s]\n' "$label" "$expected" "$actual" >&2
  fi
}

# assert_ok <label> <cmd...>   — command must succeed (exit 0)
assert_ok() {
  local label="$1"; shift
  if "$@" >/dev/null 2>&1; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    printf 'FAIL: %s (expected success, got exit %d)\n' "$label" "$?" >&2
  fi
}

# assert_fail <label> <cmd...>  — command must fail (non-zero)
assert_fail() {
  local label="$1"; shift
  if "$@" >/dev/null 2>&1; then
    FAIL=$((FAIL + 1))
    printf 'FAIL: %s (expected failure, got success)\n' "$label" >&2
  else
    PASS=$((PASS + 1))
  fi
}

# ---- version math ----------------------------------------------------------
test_next_version() {
  assert_eq "patch bump"            "1.2.4"  "$(next_version 1.2.3 patch)"
  assert_eq "minor bump zeroes patch" "1.3.0" "$(next_version 1.2.3 minor)"
  assert_eq "major bump zeroes rest" "2.0.0"  "$(next_version 1.2.3 major)"
  assert_eq "0.x minor rolls to .10" "0.10.0" "$(next_version 0.9.2 minor)"
  assert_eq "0.x patch"             "0.9.3"  "$(next_version 0.9.2 patch)"
  assert_eq "double-digit patch"    "1.2.10" "$(next_version 1.2.9 patch)"
  assert_fail "reject two-part version"  next_version 1.2 patch
  assert_fail "reject four-part version" next_version 1.2.3.4 patch
  assert_fail "reject prerelease"        next_version 1.2.3-rc1 patch
  assert_fail "reject non-numeric"       next_version 1.2.x patch
  assert_fail "reject unknown bump"      next_version 1.2.3 mega
}

test_is_semver() {
  assert_ok   "0.9.2 is semver"    is_semver 0.9.2
  assert_ok   "10.20.30 is semver" is_semver 10.20.30
  assert_fail "empty not semver"   is_semver ""
  assert_fail "1.2 not semver"     is_semver 1.2
  assert_fail "v-prefixed not semver" is_semver v1.2.3
}

# ---- dependency graph / publish order --------------------------------------
test_release_order_covers_all() {
  assert_eq "release_order lists all 12 crates" "12" "$(release_order | wc -w | tr -d ' ')"
}

# The publish order MUST be topological: every crate's in-workspace deps appear
# strictly before it. This is the property that keeps `cargo publish` from
# uploading a dependent before its dependency is live on crates.io.
test_release_order_is_topological() {
  local order pos=0 name dep ok=1
  order="$(release_order)"
  # position of each crate in the order (1-based)
  index_of() {
    local target="$1" i=0 c
    for c in $order; do
      i=$((i + 1))
      [ "$c" = "$target" ] && { echo "$i"; return 0; }
    done
    echo 0
  }
  for name in $order; do
    local np dp
    np="$(index_of "$name")"
    for dep in $(crate_deps "$name"); do
      dp="$(index_of "$dep")"
      if [ "$dp" -eq 0 ] || [ "$dp" -ge "$np" ]; then
        ok=0
        printf 'FAIL: topo order — %s (pos %s) depends on %s (pos %s)\n' "$name" "$np" "$dep" "$dp" >&2
      fi
    done
  done
  assert_eq "release order is a valid topological sort" "1" "$ok"
}

test_crate_dir() {
  assert_eq "vcs-diff dir"        "crates/diff"        "$(crate_dir vcs-diff)"
  assert_eq "vcs-cli-support dir" "crates/cli-support" "$(crate_dir vcs-cli-support)"
  assert_eq "vcs-mcp dir"         "crates/mcp"         "$(crate_dir vcs-mcp)"
  assert_fail "unknown crate dir" crate_dir vcs-nope
}

# ---- verify package set (the `cargo package` set) --------------------------
oneline() { tr '\n' ' ' | sed 's/ $//'; }

test_package_set_nothing_published() {
  # A leaf verifies alone.
  assert_eq "leaf packages alone" "vcs-diff" "$(package_set 'vcs-diff' '' | oneline)"
  # vcs-git pulls in its two (unpublished) foundational deps, in release order.
  assert_eq "vcs-git + unpublished deps" \
    "vcs-diff vcs-cli-support vcs-git" \
    "$(package_set 'vcs-git' '' | oneline)"
  # First `all` release: nothing published yet -> the whole workspace packages
  # together so the temporary registry can satisfy every in-run internal dep.
  assert_eq "all-release packages full workspace" \
    "vcs-diff vcs-cli-support vcs-git vcs-jj vcs-github vcs-gitlab vcs-gitea vcs-forge vcs-testkit vcs-core vcs-watch vcs-mcp" \
    "$(package_set "$(release_order)" '' | oneline)"
}

test_package_set_published_deps_resolve_from_registry() {
  # Foundational deps already published: a solo wrapper release packages ONLY
  # itself; its deps resolve from crates.io (their real published versions), the
  # same fidelity a real `cargo publish` gets — NOT the local (maybe-ahead) tree.
  assert_eq "solo wrapper, deps published -> itself only" \
    "vcs-git" \
    "$(package_set 'vcs-git' 'vcs-diff vcs-cli-support' | oneline)"
  # Everything but the released crate is published -> package just that crate.
  assert_eq "solo facade, all deps published -> itself only" \
    "vcs-mcp" \
    "$(package_set 'vcs-mcp' 'vcs-diff vcs-cli-support vcs-git vcs-jj vcs-github vcs-gitlab vcs-gitea vcs-forge vcs-core' | oneline)"
  # Mixed: diff+cli-support published, git+jj not -> package core with its
  # unpublished deps (git, jj); the published diff/cli-support stay on crates.io.
  assert_eq "mixed publish state pulls only unpublished deps" \
    "vcs-git vcs-jj vcs-core" \
    "$(package_set 'vcs-core' 'vcs-diff vcs-cli-support' | oneline)"
}

test_verify_package_specs() {
  assert_eq "specs for vcs-git (nothing published)" \
    "-p vcs-diff -p vcs-cli-support -p vcs-git" \
    "$(verify_package_specs 'vcs-git' '')"
  assert_eq "specs for vcs-git (deps published)" \
    "-p vcs-git" \
    "$(verify_package_specs 'vcs-git' 'vcs-diff vcs-cli-support')"
  # First `all` release: every crate selected as a `-p` (== cargo package --workspace).
  assert_eq "specs for full workspace count" "12" \
    "$(verify_package_specs "$(release_order)" '' | tr ' ' '\n' | grep -c '^-p$' | tr -d ' ')"
}

# ---- SemVer bump decision --------------------------------------------------
test_bump_rank_and_sufficiency() {
  assert_eq "rank patch" "0" "$(bump_rank patch)"
  assert_eq "rank minor" "1" "$(bump_rank minor)"
  assert_eq "rank major" "2" "$(bump_rank major)"
  assert_ok   "minor covers minor"  bump_sufficient minor minor
  assert_ok   "major covers minor"  bump_sufficient major minor
  assert_ok   "patch covers patch"  bump_sufficient patch patch
  assert_fail "patch under minor"   bump_sufficient patch minor
  assert_fail "minor under major"   bump_sufficient minor major
  assert_fail "patch under major"   bump_sufficient patch major
}

test_required_bump_from_summary() {
  assert_eq "major required" "major" \
    "$(required_bump_from_summary 'Summary semver requires new major version: 2 major and 0 minor checks failed')"
  assert_eq "minor required (0.x breaking)" "minor" \
    "$(required_bump_from_summary 'Summary semver requires new minor version: 1 major and 0 minor checks failed')"
  assert_eq "patch required (additions)" "patch" \
    "$(required_bump_from_summary 'Summary semver requires new patch version')"
  assert_eq "no update -> patch" "patch" \
    "$(required_bump_from_summary 'Checked 42 items; no semver update required')"
  assert_eq "garbage -> unknown" "unknown" \
    "$(required_bump_from_summary 'error[E0432]: unresolved import; baseline build failed')"
}

# ---- the map is pinned to the real manifests -------------------------------
# Re-derive each crate's in-workspace (non-dev) deps straight from its
# Cargo.toml `[dependencies]` table and assert `crate_deps` matches. This is the
# guard that stops the dependency map from silently rotting when a crate gains
# or drops an internal dependency (which would corrupt publish order and the
# verify closure).
manifest_deps() {
  awk '
    /^\[/ { in_deps = ($0 == "[dependencies]") }
    in_deps && /^vcs-[a-z-]+[[:space:]]*=/ {
      key = $1
      sub(/[[:space:]]*=.*/, "", key)
      print key
    }
  ' "$1/Cargo.toml" | sort -u | tr '\n' ' ' | sed 's/ $//'
}

test_manifest_deps_match() {
  local name dir declared actual
  for name in $(release_order); do
    dir="$REPO_ROOT/$(crate_dir "$name")"
    declared="$(printf '%s\n' $(crate_deps "$name") | sort -u | tr '\n' ' ' | sed 's/ $//')"
    actual="$(manifest_deps "$dir")"
    assert_eq "crate_deps($name) matches $(crate_dir "$name")/Cargo.toml [dependencies]" \
      "$declared" "$actual"
  done
}

# ---- run --------------------------------------------------------------------
test_next_version
test_is_semver
test_release_order_covers_all
test_release_order_is_topological
test_crate_dir
test_package_set_nothing_published
test_package_set_published_deps_resolve_from_registry
test_verify_package_specs
test_bump_rank_and_sufficiency
test_required_bump_from_summary
test_manifest_deps_match

echo
echo "release lib tests: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
