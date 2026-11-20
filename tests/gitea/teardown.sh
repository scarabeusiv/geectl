#!/usr/bin/env bash
# Tear down the disposable Gitea instance provisioned by `provision.sh`.
#
# Stops and removes the container and its volume (so the next provision
# starts from a fresh database), and removes the token file (so no
# credential outlives the instance).
#
# Usage:
#   bash tests/gitea/teardown.sh
#   CONTAINER=podman bash tests/gitea/teardown.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTAINER="${CONTAINER:-docker}"
CONTAINER_NAME="geectl-gitea-live"
VOLUME_NAME="geectl-gitea-data"
TOKEN_FILE="$SCRIPT_DIR/.gitea-token"
# The token must not outlive the instance even if a step below fails
# midway: remove it on any exit, not only on the happy path.
trap 'rm -f "$TOKEN_FILE"' EXIT

# Try the configured runtime first, then the other well-known one:
# the instance may have been provisioned with either (CONTAINER
# defaults to docker, but a local run may have used podman), and
# teardown must not leave it behind.
runtimes=("$CONTAINER")
for candidate in docker podman; do
    if [ "$candidate" != "$CONTAINER" ]; then
        runtimes+=("$candidate")
    fi
done
for runtime in "${runtimes[@]}"; do
    "$runtime" stop "$CONTAINER_NAME" >/dev/null 2>&1 || true
    "$runtime" rm "$CONTAINER_NAME" >/dev/null 2>&1 || true
    "$runtime" volume rm "$VOLUME_NAME" >/dev/null 2>&1 || true
done
rm -f "$TOKEN_FILE"
echo "Disposable Gitea instance torn down (container, volume, and token file removed)."
