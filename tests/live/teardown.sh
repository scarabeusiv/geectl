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
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OBS_SOURCE_DIR="${OBS_SOURCE_DIR:-$SCRIPT_DIR/obs}"
COMPOSE="${COMPOSE:-docker compose}"
COMPOSE_FILE="$SCRIPT_DIR/docker-compose.yml"

export OBS_SOURCE_DIR

$COMPOSE -f "$COMPOSE_FILE" down -v
echo "Live OBS instance torn down (containers and volumes removed)."
echo "OBS source left at $OBS_SOURCE_DIR for reuse."
