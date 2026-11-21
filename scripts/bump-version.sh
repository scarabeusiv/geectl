#!/usr/bin/env bash
# Move every release-version surface to a new version in one step.
#
# Usage: scripts/bump-version.sh X.Y.Z
#
# The version surfaces in this repo are exactly two: the [package]
# version in Cargo.toml and the geectl entry in Cargo.lock (README.md
# and docs/ carry no version string — verified by grepping for the
# current version before writing this script). The script rewrites
# both, then verifies the result, so a half-applied bump fails here
# rather than riding along in a version-bump PR.
set -euo pipefail

# No leading zeros: Cargo rejects them, and only after the tree is edited.
semver='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'

new="${1:-}"
if ! [[ "$new" =~ $semver ]]; then
  echo "usage: $0 X.Y.Z" >&2
  exit 2
fi

cd "$(git rev-parse --show-toplevel)"

# Standalone robustness: the tree must be clean before the bump, and
# any failure after the first edit restores the two version surfaces.
# (In the release workflow the checkout is fresh, so the precondition
# is a no-op there; it protects a developer running the script by hand.)
if [ -n "$(git status --porcelain)" ]; then
  echo "refusing to bump: the tree has uncommitted changes" >&2
  exit 1
fi
trap 'git checkout -- Cargo.toml Cargo.lock 2>/dev/null || true' ERR

# Tolerant [package] version read: spaces around `=` and a trailing
# comment are allowed. A workspace-inherited version
# (`version.workspace = true`) yields nothing and fails closed below.
manifest_version() {
  sed -n '/^\[package\]/,/^\[/s/^[[:space:]]*version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$1" | head -1
}

old=$(manifest_version Cargo.toml)
if ! [[ "$old" =~ $semver ]]; then
  echo "no X.Y.Z [package] version found in Cargo.toml (got '$old')" >&2
  exit 1
fi

# Numeric X.Y.Z comparison (10# keeps leading zeros decimal); portable,
# unlike sort -V.
IFS=. read -r o1 o2 o3 <<<"$old"
IFS=. read -r n1 n2 n3 <<<"$new"
if ((10#$n1 != 10#$o1)); then
  forward=$((10#$n1 > 10#$o1))
elif ((10#$n2 != 10#$o2)); then
  forward=$((10#$n2 > 10#$o2))
else
  forward=$((10#$n3 > 10#$o3))
fi
if [ "$forward" -ne 1 ]; then
  echo "refusing to move $old to $new: the version only moves forward" >&2
  echo "(a half-applied earlier run is undone with: git checkout -- Cargo.toml Cargo.lock)" >&2
  exit 1
fi

# In-place edit without GNU sed -i; `cat >` keeps the file's mode.
subst() { # sed-script file
  local tmp
  tmp=$(mktemp)
  sed "$1" "$2" >"$tmp"
  cat "$tmp" >"$2"
  rm -f "$tmp"
}

old_re=${old//./\\.}

subst "/^\[package\]/,/^\[/s/^\([[:space:]]*version[[:space:]]*=[[:space:]]*\)\"$old_re\".*/\1\"$new\"/" Cargo.toml

# -p geectl re-resolves only the local package's lock entry;
# dependencies stay put. Anything more (a newer cargo rewriting the
# lock format, say) does not belong in a version bump.
cargo update -p geectl --quiet
# Compare against HEAD, not the worktree: a staged pre-existing edit
# would otherwise evade the check. Both surfaces must move exactly
# one line each (the version line); anything more does not belong in
# a version bump.
toml_diff=$(git diff HEAD --numstat -- Cargo.toml | cut -f1-2)
lock_diff=$(git diff HEAD --numstat -- Cargo.lock | cut -f1-2)
if [ "$toml_diff" != "$(printf '1\t1')" ] || [ "$lock_diff" != "$(printf '1\t1')" ]; then
  echo "version surfaces changed beyond one line each (Cargo.toml=$toml_diff Cargo.lock=$lock_diff):" >&2
  git diff HEAD -- Cargo.toml Cargo.lock >&2
  exit 1
fi

# Verify both surfaces carry the new version, and that no other
# tracked file still carries the old one.
manifest=$(manifest_version Cargo.toml)
locked=$(sed -n '/^[[:space:]]*name[[:space:]]*=[[:space:]]*"geectl"/,/^[[:space:]]*version[[:space:]]*=/s/^[[:space:]]*version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' Cargo.lock | head -1)
if [ "$manifest" != "$new" ] || [ "$locked" != "$new" ]; then
  echo "version mismatch after bump: Cargo.toml=$manifest Cargo.lock=$locked (expected $new)" >&2
  exit 1
fi
# git grep exits 1 on no match (clean) and 0 on a match (stale), but
# 2-plus on a real error (bad pathspec, corrupt index): only exit 1
# is tolerated, anything else fails loudly.
stale_file=$(mktemp)
stale_rc=0
git grep -F -l "$old" -- ':!Cargo.lock' ':!Cargo.toml' >"$stale_file" || stale_rc=$?
if [ "$stale_rc" -gt 1 ]; then
  echo "stale-version check failed (git grep exit $stale_rc)" >&2
  exit 1
fi
if [ -s "$stale_file" ]; then
  echo "old version $old still appears in:" >&2
  cat "$stale_file" >&2
  exit 1
fi
rm -f "$stale_file"
