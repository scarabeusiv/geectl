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

$CONTAINER stop "$CONTAINER_NAME" >/dev/null 2>&1 || true
$CONTAINER rm "$CONTAINER_NAME" >/dev/null 2>&1 || true
$CONTAINER volume rm "$VOLUME_NAME" >/dev/null 2>&1 || true
rm -f "$TOKEN_FILE"
echo "Disposable Gitea instance torn down (container, volume, and token file removed)."
