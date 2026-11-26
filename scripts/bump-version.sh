#!/usr/bin/env bash
# Move every release-version surface to a new version in one step.
#
# Usage: scripts/bump-version.sh X.Y.Z
#
# The version surfaces in this repo are three: the [package] version
# in Cargo.toml, the geectl entry in Cargo.lock, and the man pages
# under man/, which stamp the version in their `.TH` header
# (README.md and docs/ carry no version string — verified by grepping
# for the current version before writing this script). The script
# rewrites the two manifest surfaces and regenerates the pages, then
# verifies the result, so a half-applied bump fails here rather than
# riding along in a version-bump PR.
set -euo pipefail

# No leading zeros: Cargo rejects them, and only after the tree is edited.
semver='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'

new="${1:-}"
if ! [[ "$new" =~ $semver ]]; then
  echo "usage: $0 X.Y.Z" >&2
  exit 2
fi

cd "$(git rev-parse --show-toplevel)"

# Standalone robustness: the tree must be clean before the bump.
# (In the release workflow the checkout is fresh, so the precondition
# is a no-op there; it protects a developer running the script by hand.)
if [ -n "$(git status --porcelain)" ]; then
  echo "refusing to bump: the tree has uncommitted changes" >&2
  exit 1
fi

# One EXIT cleanup for the whole script: it removes every tempfile
# the script creates and, on any nonzero exit, restores the two
# version surfaces. EXIT (not ERR) is what makes explicit `exit 1`
# validation failures restore too — ERR never fires for an explicit
# exit, so a post-edit drift/mismatch/stale failure used to leave a
# dirty tree behind. Installed after the clean-tree precondition: a
# failure before this point has edited nothing, so there is nothing
# to restore.
subst_tmp=""
stale_tmp=""
gen_tmp=""
cleanup() {
  local rc=$?
  if [ -n "$subst_tmp" ]; then rm -f "$subst_tmp"; fi
  if [ -n "$stale_tmp" ]; then rm -f "$stale_tmp"; fi
  if [ -n "$gen_tmp" ]; then rm -rf "$gen_tmp"; fi
  if [ "$rc" -ne 0 ]; then
    # One file at a time: a single checkout naming a surface that is
    # not in the index (no Cargo.lock yet, say) restores neither.
    git checkout -- Cargo.toml 2>/dev/null || true
    git checkout -- Cargo.lock 2>/dev/null || true
    # The regenerated pages are restored the same way. The tree was
    # clean at entry (see the precondition above), so any untracked
    # file under man/ was created by this run and is removed with it.
    git checkout -- man/ 2>/dev/null || true
    git clean -f man/ 2>/dev/null || true
  fi
}
trap cleanup EXIT

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
  exit 1
fi

# In-place edit without GNU sed -i; `cat >` keeps the file's mode.
subst() { # sed-script file
  local tmp
  tmp=$(mktemp)
  # Registered with the script-wide EXIT cleanup, not a trap of its
  # own (which would replace it): a failure mid-call still removes
  # the tempfile, and so does any later failure.
  subst_tmp="$tmp"
  sed "$1" "$2" >"$tmp"
  cat "$tmp" >"$2"
  rm -f "$tmp"
  subst_tmp=""
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

# The man pages stamp the version in their `.TH` header, so they are
# a version surface too — regenerated, never edited: the generator is
# the single source of truth for their content (the assets-drift job
# fails if the committed pages differ from a fresh generation).
# Generate into a temp dir and copy only the pages over; the
# completions carry no version string and stay untouched. This runs
# after the manifest edits, so the generator builds against the new
# version.
gen_tmp=$(mktemp -d)
cargo run --locked --quiet --features gen --bin geectl-gen -- "$gen_tmp"
cp "$gen_tmp"/man/*.1 man/
# Prune pages the generator no longer produces, mirroring the
# generator pruning of stale pages: a removed leaf leaves no orphan
# page behind.
for page in man/*.1; do
  [ -f "$gen_tmp/man/$(basename "$page")" ] || rm -f "$page"
done

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
stale_tmp=$(mktemp)
stale_rc=0
# The old version must match as a whole token, not a substring: a
# plain -F search also matches longer versions that merely contain
# it (a 10.x release contains the 0.x string). The ERE requires a
# non-digit, non-dot (or a line edge) on both sides, so a v-prefixed
# or quoted occurrence still matches while the longer one does not.
git grep -E -l "(^|[^0-9.])$old_re([^0-9.]|$)" -- ':!Cargo.lock' ':!Cargo.toml' >"$stale_tmp" || stale_rc=$?
if [ "$stale_rc" -gt 1 ]; then
  echo "stale-version check failed (git grep exit $stale_rc)" >&2
  exit 1
fi
if [ -s "$stale_tmp" ]; then
  echo "old version $old still appears in:" >&2
  cat "$stale_tmp" >&2
  exit 1
fi
