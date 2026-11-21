#!/usr/bin/env bash
# Tear down the live OBS instance provisioned by `provision.sh`.
#
# Removes the containers and their volumes (`down -v`), so the next
# provision starts from a fresh database. The OBS source checkout is
# left in place for reuse (re-cloning and rebuilding the frontend on
# every run would waste the time this script exists to save); delete
# it by hand if you want a fully clean slate.
#
# Usage:
#   OBS_SOURCE_DIR=/path/to/open-build-service bash tests/live/teardown.sh
#   (defaults to <repo>/tests/live/obs, matching provision.sh)
#   COMPOSE="podman compose" bash tests/live/teardown.sh
#   COMPOSE_PROJECT_NAME=geectl-live-2 OBS_SOURCE_DIR=/path/to/open-build-service-2 bash tests/live/teardown.sh
#   (tears down the second stack: the project name selects its own
#   containers and volumes)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OBS_SOURCE_DIR="${OBS_SOURCE_DIR:-$SCRIPT_DIR/obs}"
# Compose treats a relative bind-mount source without a leading `./`
# as a named volume, so a relative OBS_SOURCE_DIR (the second stack's
# `tests/live/obs-2` in CI) fails provisioning with "refers to
# undefined volume". Resolve it against the current directory here,
# before any compose invocation reads it.
case "$OBS_SOURCE_DIR" in
    /*) ;;
    *) OBS_SOURCE_DIR="$PWD/$OBS_SOURCE_DIR" ;;
esac
COMPOSE="${COMPOSE:-docker compose}"
COMPOSE_FILE="$SCRIPT_DIR/docker-compose.yml"

export OBS_SOURCE_DIR

# The second stack's project name (see provision.sh): it selects
# that stack's containers and volumes for `down -v`.
if [ -n "${COMPOSE_PROJECT_NAME:-}" ]; then
    export COMPOSE_PROJECT_NAME
    echo "Compose project: $COMPOSE_PROJECT_NAME"
fi

$COMPOSE -f "$COMPOSE_FILE" down -v
echo "Live OBS instance torn down (containers and volumes removed)."
echo "OBS source left at $OBS_SOURCE_DIR for reuse."
