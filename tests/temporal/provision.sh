#!/usr/bin/env bash
# Provision a temporal OBS instance for geectl's scheduled live tests.
#
# What it does:
#   1. Clones the OBS source (depth 1) if it is not already present,
#      and initialises its `src/backend/build` submodule (the Build
#      Perl modules the backend daemons need).
#      The source is needed twice: the backend runs its development
#      start script from the mounted checkout, and the frontend image
#      is built from `src/api`. There is no published prebuilt frontend
#      server image — the OBS project builds it from source, and so do we.
#   2. Starts the database and cache, waits for the database.
#   3. Builds the frontend image (with the BuildKit GitHub Actions
#      cache when TEMPORAL_CACHE is set — see docker-compose.cache.yml).
#   4. Runs `rake dev:bootstrap` in a one-off frontend container. That
#      creates the database and seeds it, including the Admin user
#      (`Admin` / `opensuse` — the OBS development default, published in
#      the OBS repo's `src/api/db/seeds.rb`). The instance is ephemeral
#      and loopback-only; those credentials are never real.
#   5. Starts the backend and frontend, then polls `GET /about` until
#      the API answers, and probes the backend source server until it
#      answers. /about needs only the frontend and the database, while
#      project writes go to the backend — /about alone does not prove
#      a write will work.
#   6. Prints the base URL for the harness (`GEECTL_TEMPORAL_URL`).
#
# The worker is deliberately not started: no test builds a package, and
# the worker needs a privileged container.
#
# Usage:
#   OBS_SOURCE_DIR=/path/to/open-build-service bash tests/temporal/provision.sh
#   (defaults to <repo>/tests/temporal/obs; cloned if missing)
#   COMPOSE="podman compose" bash tests/temporal/provision.sh
#   (defaults to `docker compose`)
#   TEMPORAL_CACHE=1 bash tests/temporal/provision.sh
#   (loads docker-compose.cache.yml, the CI-only BuildKit cache for
#   the frontend image build; local runs leave it unset)
#
# Exit code is non-zero if any step fails. The instance is left running
# on failure for inspection; `tests/temporal/teardown.sh` removes it.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OBS_SOURCE_DIR="${OBS_SOURCE_DIR:-$SCRIPT_DIR/obs}"
OBS_REPO="https://github.com/openSUSE/open-build-service.git"
COMPOSE="${COMPOSE:-docker compose}"
COMPOSE_FILE="$SCRIPT_DIR/docker-compose.yml"
# CI-only frontend build cache (see docker-compose.cache.yml): when
# TEMPORAL_CACHE is set, the cache override file is added to every
# compose invocation. It only affects the frontend build
# (cache_from/cache_to); all other commands behave identically.
# Local runs leave TEMPORAL_CACHE unset and never load the file,
# so podman and plain docker runs are unchanged.
COMPOSE_FILES=(-f "$COMPOSE_FILE")
if [ -n "${TEMPORAL_CACHE:-}" ]; then
    COMPOSE_FILES+=(-f "$SCRIPT_DIR/docker-compose.cache.yml")
    echo "Frontend build cache enabled (TEMPORAL_CACHE is set)."
fi
FRONTEND_PORT="${FRONTEND_PORT:-3000}"
BASE_URL="http://127.0.0.1:${FRONTEND_PORT}"
READINESS_TIMEOUT_SECONDS="${READINESS_TIMEOUT_SECONDS:-600}"

# Fail fast on missing prerequisites: without git the clone fails
# late, and without curl the readiness loop spins until the
# timeout (set -e does not fire inside `until` conditions).
for cmd in git curl; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        echo "required command not found: $cmd" >&2
        exit 1
    fi
done
# The compose command is a word list (e.g. `docker compose`), so
# check the first word.
compose_bin="${COMPOSE%% *}"
if ! command -v "$compose_bin" >/dev/null 2>&1; then
    echo "required command not found: $compose_bin (COMPOSE=$COMPOSE)" >&2
    exit 1
fi

# The OBS source checkout. A depth-1 clone is enough: the backend runs
# from the checkout and the frontend builds from it; history is never
# needed. An interrupted clone leaves a directory with a `.git` but no
# checkout; that is a partial clone, not a reusable one, so remove it
# and re-clone rather than failing on the non-empty destination. Only
# a clone of the OBS repo itself may be removed: an operator-misconfigured
# OBS_SOURCE_DIR could point at an unrelated git checkout, so the
# remote is verified before anything is deleted. A directory that is
# neither a checkout nor a clone is left alone and fails loudly instead
# of being deleted.
if [ ! -d "$OBS_SOURCE_DIR/src/api" ]; then
    if [ -d "$OBS_SOURCE_DIR/.git" ]; then
        origin="$(git -C "$OBS_SOURCE_DIR" remote get-url origin 2>/dev/null || true)"
        if [ "$origin" = "$OBS_REPO" ]; then
            echo "Removing partial OBS clone at $OBS_SOURCE_DIR ..."
            rm -rf -- "$OBS_SOURCE_DIR"
        else
            echo "OBS_SOURCE_DIR $OBS_SOURCE_DIR is a git checkout of ${origin:-<no origin remote>}, not the OBS repo ($OBS_REPO); remove it or point OBS_SOURCE_DIR elsewhere" >&2
            exit 1
        fi
    elif [ -e "$OBS_SOURCE_DIR" ]; then
        echo "OBS_SOURCE_DIR $OBS_SOURCE_DIR exists but is not an OBS checkout (no src/api, no .git); remove it or point OBS_SOURCE_DIR elsewhere" >&2
        exit 1
    fi
    echo "Cloning OBS source (depth 1) into $OBS_SOURCE_DIR ..."
    git clone --depth 1 --single-branch "$OBS_REPO" "$OBS_SOURCE_DIR"
else
    echo "Using existing OBS source at $OBS_SOURCE_DIR"
fi
# The backend runs its Perl daemons from the checkout, and they need
# the Build modules, which live in the `src/backend/build` submodule
# (openSUSE/obs-build): a plain clone leaves that directory empty
# and every backend daemon dies at startup with "Can't locate
# Build.pm". Initialise it whenever the source is a git checkout —
# a fresh clone or a reused one from before this step existed.
if git -C "$OBS_SOURCE_DIR" rev-parse --git-dir >/dev/null 2>&1; then
    echo "Initialising OBS submodules ..."
    git -C "$OBS_SOURCE_DIR" submodule update --init --depth 1
fi
# Log the exact OBS commit so a scheduled failure is actionable: the
# instance tracks OBS master, and the HEAD SHA says which master. A
# valid reuse need not be a git checkout (e.g. an extracted tarball),
# so log the HEAD only when there is one instead of printing git's
# fatal message with an empty SHA.
if head_sha="$(git -C "$OBS_SOURCE_DIR" rev-parse HEAD 2>/dev/null)"; then
    echo "OBS source HEAD: $head_sha"
else
    echo "OBS source HEAD: (not a git checkout)"
fi

export OBS_SOURCE_DIR
export CONTAINER_USERID="$(id -u)"
export FRONTEND_PORT

# The database and cache first: the bootstrap and the backend both need
# the database up, and the frontend will not boot without it.
echo "Starting database and cache ..."
$COMPOSE "${COMPOSE_FILES[@]}" up -d db cache

echo "Waiting for the database ..."
deadline=$((SECONDS + 300))
until $COMPOSE "${COMPOSE_FILES[@]}" exec -T db mysqladmin ping --silent >/dev/null 2>&1; do
    if [ "$SECONDS" -ge "$deadline" ]; then
        echo "database did not come up within 300s" >&2
        $COMPOSE "${COMPOSE_FILES[@]}" logs db | tail -30 >&2
        exit 1
    fi
    sleep 5
done
echo "Database is up."

# The frontend image is built from the OBS source (there is no published
# prebuilt frontend server image). This is the slow step.
echo "Building the frontend image ..."
$COMPOSE "${COMPOSE_FILES[@]}" build frontend

# Bootstrap in a one-off frontend container (not the long-running one):
# it creates the database and seeds it, including the Admin user. It
# runs with the database and cache already up, and without the backend
# (bootstrap touches only the database).
echo "Bootstrapping the OBS database (creates it and seeds the Admin user) ..."
$COMPOSE "${COMPOSE_FILES[@]}" run --rm --no-deps \
    -e RAILS_ENV=development frontend \
    bundle exec rake dev:bootstrap

# Now the long-running services. The database exists, so the frontend
# boots instead of exiting on a missing database.
echo "Starting backend and frontend ..."
$COMPOSE "${COMPOSE_FILES[@]}" up -d backend frontend

# Readiness: the public /about endpoint answers 200 only once the
# frontend can serve, which needs the database. It says nothing about
# the backend (probed separately below).
echo "Waiting for the API at $BASE_URL/about ..."
deadline=$((SECONDS + READINESS_TIMEOUT_SECONDS))
# Explicit 200 check (curl -f without -L returns 0 on a 3xx) and
# per-attempt bounds (a single hung curl must not exceed the
# outer deadline: the deadline is only checked between probes).
until [ "$(curl -sS -o /dev/null -w "%{http_code}" --connect-timeout 5 --max-time 10 "$BASE_URL/about" 2>/dev/null)" = "200" ]; do
    if [ "$SECONDS" -ge "$deadline" ]; then
        echo "API did not come up within ${READINESS_TIMEOUT_SECONDS}s" >&2
        $COMPOSE "${COMPOSE_FILES[@]}" logs frontend | tail -50 >&2
        exit 1
    fi
    sleep 10
done

# Readiness, backend: project writes go to the backend source
# server (bs_srcserver), a separate process from the frontend, and
# they fail until it answers. Its port is not published, so probe it
# from inside the frontend container over the compose network: any
# HTTP response means the source server is listening (a 404 would
# do); only a refused connection means it is not up yet.
echo "Waiting for the backend source server ..."
deadline=$((SECONDS + READINESS_TIMEOUT_SECONDS))
until $COMPOSE "${COMPOSE_FILES[@]}" exec -T frontend ruby -r net/http -e 'begin; Net::HTTP.get_response(URI("http://backend:5352/source")); rescue StandardError; exit(1); end' >/dev/null 2>&1; do
    if [ "$SECONDS" -ge "$deadline" ]; then
        echo "backend did not come up within ${READINESS_TIMEOUT_SECONDS}s" >&2
        $COMPOSE "${COMPOSE_FILES[@]}" logs backend | tail -50 >&2
        exit 1
    fi
    sleep 10
done
echo "Backend is up."

echo "Temporal OBS instance is up."
echo "Base URL: $BASE_URL"
echo "Run the live tests with: GEECTL_TEMPORAL_URL=$BASE_URL cargo test --locked --test temporal"
