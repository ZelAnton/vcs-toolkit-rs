#!/usr/bin/env bash
# Check that the stability matrix in crates/core/docs/stability.md matches
# every crate's actual `version = "..."` in its `Cargo.toml`, and that every
# crate with a `Cargo.toml` is listed in the matrix.
#
# Local Markdown link rot and external-link liveness are already covered by
# the `typos`/`lychee` steps in the `docs-health` CI job (see `ci.yml` and
# `lychee.toml`) across README/CONTRIBUTING/CLAUDE/`docs/**`/per-crate guides/
# CHANGELOGs; this script covers what those tools can't — cross-referencing a
# hand-maintained table against the manifests it describes.

set -u

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
status=0
matrix="$repo_root/crates/core/docs/stability.md"

# Matrix -> manifest: every `| `crate` | version | ... |` row must match the
# crate's actual Cargo.toml version.
matrix_row_re='^[[:space:]]*\|[[:space:]]*`([^`]+)`[[:space:]]*\|[[:space:]]*([^|[:space:]]+)[[:space:]]*\|[^|]*\|[[:space:]]*$'
while IFS= read -r row; do
  if [[ ! "$row" =~ ^[[:space:]]*\| ]] || [[ "$row" != *'`'* ]]; then
    continue
  fi
  if [[ ! "$row" =~ $matrix_row_re ]]; then
    printf '%s: malformed stability matrix crate row: %s\n' \
      "crates/core/docs/stability.md" "$row" >&2
    status=1
    continue
  fi
  crate="${BASH_REMATCH[1]}"
  expected="${BASH_REMATCH[2]}"

  manifest=""
  for candidate in "$repo_root"/crates/*/Cargo.toml; do
    if grep -qE "^name[[:space:]]*=[[:space:]]*\"${crate}\"[[:space:]]*$" "$candidate"; then
      manifest="$candidate"
      break
    fi
  done

  if [[ -z "$manifest" ]]; then
    printf '%s: version matrix names a crate with no Cargo.toml: %s\n' \
      "crates/core/docs/stability.md" "$crate" >&2
    status=1
    continue
  fi

  actual="$(grep -m1 -E '^version[[:space:]]*=' "$manifest" | sed -E 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')"
  if [[ "$actual" != "$expected" ]]; then
    printf '%s: version drift for %s: matrix=%s manifest=%s\n' \
      "crates/core/docs/stability.md" "$crate" "$expected" "$actual" >&2
    status=1
  fi
done < "$matrix"

# Manifest -> matrix: every crate with a Cargo.toml should be listed, so a
# newly-added crate can't silently skip the stability table.
while IFS= read -r manifest; do
  name="$(grep -m1 -E '^name[[:space:]]*=' "$manifest" | sed -E 's/^name[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')"
  [[ -n "$name" ]] || continue
  if ! grep -qE "^\| \`${name}\` \|" "$matrix"; then
    printf '%s: crate missing from the stability matrix: %s\n' \
      "crates/core/docs/stability.md" "$name" >&2
    status=1
  fi
done < <(printf '%s\n' "$repo_root"/crates/*/Cargo.toml)

if (( status == 0 )); then
  echo "docs-health: stability matrix matches every crate manifest"
fi

exit "$status"
