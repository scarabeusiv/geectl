#!/usr/bin/env bash
# Provision a disposable Gitea instance for geectl's live tests.
#
# What it does:
#   1. Removes any stale instance (a previous run that was not torn
#      down): user and token creation below fail if they already exist,
#      so every run starts from a fresh volume.
#   2. Starts the Gitea container (single container, SQLite) and waits
#      for `GET /api/healthz`.
#   3. Creates the test user (`geectl`, admin) and an API token for it,
#      with the scopes the live tests need. The token is written to a
#      0600 file (`.gitea-token`, gitignored) — it is never printed, so
#      it never lands in a log or a shell history.
#   4. Prints the base URL and the test command. In CI (`GITHUB_ENV`
#      set), the URL and token are exported for the test step instead
#      (the token is masked first, so it never appears in a log).
#
# Plain container commands, not compose: Gitea is a single container,
# so compose adds nothing — and `podman compose` on the Mac is only a
# wrapper around an external provider (`docker-compose` /
# `podman-compose`), neither of which is installed, so compose does not
# work there at all. `CONTAINER` selects the runtime: `docker` by
# default (CI runners), `podman` on the Mac.
#
# The image is pinned to a stable release (1.24.7), bumped by hand
# when a new stable Gitea lands — the pin lives in `IMAGE` below, and
# the live tests in the gate prove the new version still accepts
# geectl's auth. It is not `:latest`: a moving tag would let a Gitea
# release break the per-PR gate for reasons unrelated to geectl.
# (Dependabot's `docker` ecosystem only scans Dockerfiles and compose
# files, and this setup uses plain container commands, so it cannot
# track the pin.)
#
# The instance is loopback-only and ephemeral: the credentials are
# created fresh per run and destroyed with the instance (see
# `teardown.sh`). Nothing secret is ever committed or leaves the
# machine.
#
# Usage:
#   bash tests/gitea/provision.sh
#   (defaults: port 3001, user `geectl`, container `docker`)
#   GITEA_PORT=3002 CONTAINER=podman bash tests/gitea/provision.sh
#
# Exit code is non-zero if any step fails. The instance is left running
# on failure for inspection; `tests/gitea/teardown.sh` removes it.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTAINER="${CONTAINER:-docker}"
CONTAINER_NAME="geectl-gitea-live"
VOLUME_NAME="geectl-gitea-data"
IMAGE="gitea/gitea:1.24.7"
GITEA_PORT="${GITEA_PORT:-3001}"
# A port is 1-65535: anything else (a hostname, a path, an empty
# string) would be interpolated into the container's -p mapping and
# the base URL below. Digits-only and at most 5 digits first, so the
# arithmetic comparison cannot overflow on a long digit string.
case "$GITEA_PORT" in
    ''|*[!0-9]*)
        echo "GITEA_PORT must be a port number (1-65535); got '$GITEA_PORT'" >&2
        exit 1
        ;;
esac
if [ "${#GITEA_PORT}" -gt 5 ] || [ "$GITEA_PORT" -lt 1 ] || [ "$GITEA_PORT" -gt 65535 ]; then
    echo "GITEA_PORT must be a port number (1-65535); got '$GITEA_PORT'" >&2
    exit 1
fi
BASE_URL="http://127.0.0.1:${GITEA_PORT}"
GITEA_USER="${GITEA_USER:-geectl}"
GITEA_PASSWORD="${GITEA_PASSWORD:-geectl-live-test}"
GITEA_EMAIL="${GITEA_EMAIL:-geectl@test.invalid}"
TOKEN_FILE="$SCRIPT_DIR/.gitea-token"
READINESS_TIMEOUT_SECONDS="${READINESS_TIMEOUT_SECONDS:-120}"

# A stale instance (previous run, no teardown) would make user/token
# creation fail, so start from a fresh volume every time.
$CONTAINER stop "$CONTAINER_NAME" >/dev/null 2>&1 || true
$CONTAINER rm "$CONTAINER_NAME" >/dev/null 2>&1 || true
$CONTAINER volume rm "$VOLUME_NAME" >/dev/null 2>&1 || true

echo "Starting Gitea ($IMAGE) ..."
# Pull the image first, with retries: Docker Hub rate-limits anonymous
# pulls from shared GitHub runner IPs (the rust-deny job documents the
# same 429s), and a transient 429 must not fail the gate. `run` below
# would pull implicitly and fail.
for attempt in 1 2 3; do
    if $CONTAINER pull "$IMAGE"; then
        break
    fi
    if [ "$attempt" -eq 3 ]; then
        echo "could not pull $IMAGE after 3 attempts" >&2
        exit 1
    fi
    sleep 10
done
$CONTAINER run -d --name "$CONTAINER_NAME" --rm \
    -p "127.0.0.1:${GITEA_PORT}:3000" \
    -e USER_UID=1000 -e USER_GID=1000 \
    -e GITEA__security__INSTALL_LOCK=true \
    -e GITEA__database__DB_TYPE=sqlite3 \
    -e "GITEA__server__ROOT_URL=http://127.0.0.1:${GITEA_PORT}/" \
    -v "${VOLUME_NAME}:/data" \
    "$IMAGE"

echo "Waiting for Gitea at $BASE_URL/api/healthz ..."
deadline=$((SECONDS + READINESS_TIMEOUT_SECONDS))
until curl --max-time 5 -fsS -o /dev/null "$BASE_URL/api/healthz" >/dev/null 2>&1; do
    if [ "$SECONDS" -ge "$deadline" ]; then
        echo "Gitea did not come up within ${READINESS_TIMEOUT_SECONDS}s" >&2
        $CONTAINER logs "$CONTAINER_NAME" | tail -50 >&2
        exit 1
    fi
    sleep 2
done
echo "Gitea is up."

# The test user (admin, so it can create repositories) and its token.
# Scopes are the minimal set the live tests need: read the user and
# the repo list, create a repository, read it back. (Repo creation
# needs `write:user` as well as `write:repository` — `write:repository`
# alone returns 403.)
echo "Creating the test user and token ..."
$CONTAINER exec -u git "$CONTAINER_NAME" \
    gitea admin user create \
    --username "$GITEA_USER" \
    --password "$GITEA_PASSWORD" \
    --email "$GITEA_EMAIL" \
    --admin --must-change-password=false
TOKEN_OUTPUT="$($CONTAINER exec -u git "$CONTAINER_NAME" \
    gitea admin user generate-access-token \
    --username "$GITEA_USER" \
    --token-name geectl-live \
    --scopes read:user,read:repository,write:repository,write:user)"
TOKEN="$(printf '%s' "$TOKEN_OUTPUT" | sed -n 's/^Access token was successfully created: //p')"
if [ -z "$TOKEN" ]; then
    # The raw output is withheld on purpose: it may contain the
    # token (e.g. if the output format changed just enough for the
    # extraction to miss it).
    echo "could not extract the access token from the generate-access-token output" >&2
    exit 1
fi
# The token file is 0600 from the first byte (umask), and holds only
# the token (no trailing newline). Remove it first: a pre-existing
# file keeps its mode on truncate, so a stale 0644 file would stay
# readable. The chmod after the write is belt-and-braces.
rm -f "$TOKEN_FILE"
(umask 077 && printf '%s' "$TOKEN" > "$TOKEN_FILE")
chmod 600 "$TOKEN_FILE"

echo "Disposable Gitea instance is up."
echo "Base URL: $BASE_URL"
if [ -n "${GITHUB_ENV:-}" ]; then
    # In CI, export the URL and token for the test step. Mask the
    # token first so it never appears in a log.
    echo "::add-mask::$TOKEN"
    {
        echo "GEECTL_GITEA_URL=$BASE_URL"
        echo "GEECTL_GITEA_TOKEN=$TOKEN"
    } >> "$GITHUB_ENV"
    echo "URL and token exported to \$GITHUB_ENV (token masked, not printed)."
else
    echo "Run the live tests with:"
    echo "  GEECTL_GITEA_URL=$BASE_URL GEECTL_GITEA_TOKEN_FILE=$TOKEN_FILE cargo test --locked --test gitea"
fi
