#!/usr/bin/env bash
# Check that the stability matrix in crates/core/docs/stability.md matches
# every crate's actual `version = "..."` in its `Cargo.toml`, and that every
# crate with a `Cargo.toml` is listed in the matrix. Also check that every
# publish-eligible crate carries the manifest fields its crates.io/docs.rs
# listing depends on (`description`, `readme` pointing at a real file,
# `keywords`, `categories`), so the published-crate showcase can't silently
# regress when a new crate is added.
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
  if ! grep -qE "^[[:space:]]*\|[[:space:]]*\`${name}\`[[:space:]]*\|" "$matrix"; then
    printf '%s: crate missing from the stability matrix: %s\n' \
      "crates/core/docs/stability.md" "$name" >&2
    status=1
  fi
done < <(printf '%s\n' "$repo_root"/crates/*/Cargo.toml)

# Publish-crate manifest fields: every crate.io-published crate must carry the
# fields that make its crates.io/docs.rs listing usable — a missing one is easy
# to overlook when adding a new crate. Skip `publish = false` crates (none
# today, but this keeps the check honest if one is added later).
while IFS= read -r manifest; do
  crate_dir="$(dirname "$manifest")"
  name="$(grep -m1 -E '^name[[:space:]]*=' "$manifest" | sed -E 's/^name[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')"
  [[ -n "$name" ]] || continue

  if grep -qE '^publish[[:space:]]*=[[:space:]]*false' "$manifest"; then
    continue
  fi

  if ! grep -qE '^description[[:space:]]*=[[:space:]]*"[^"]+"' "$manifest"; then
    printf '%s: missing or empty `description`\n' "$manifest" >&2
    status=1
  fi

  readme_rel="$(grep -m1 -E '^readme[[:space:]]*=' "$manifest" | sed -E 's/^readme[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')"
  if [[ -z "$readme_rel" ]]; then
    printf '%s: missing `readme`\n' "$manifest" >&2
    status=1
  elif [[ ! -f "$crate_dir/$readme_rel" ]]; then
    printf '%s: `readme = "%s"` does not exist\n' "$manifest" "$readme_rel" >&2
    status=1
  fi

  if ! grep -qE '^keywords[[:space:]]*=[[:space:]]*\[[[:space:]]*"' "$manifest"; then
    printf '%s: missing or empty `keywords`\n' "$manifest" >&2
    status=1
  fi

  if ! grep -qE '^categories[[:space:]]*=[[:space:]]*\[[[:space:]]*"' "$manifest"; then
    printf '%s: missing or empty `categories`\n' "$manifest" >&2
    status=1
  fi
done < <(printf '%s\n' "$repo_root"/crates/*/Cargo.toml)

if (( status == 0 )); then
  echo "docs-health: stability matrix matches every crate manifest, and every crate carries the required publish-listing fields"
fi

exit "$status"
