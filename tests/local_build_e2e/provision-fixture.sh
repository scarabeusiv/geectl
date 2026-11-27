#!/usr/bin/env bash
# Create the local-build e2e fixture on the provisioned local OBS instance.
#
# What it does:
#   1. Creates the `geectl:e2e` project with a `standard` repository
#      backed by a download-on-demand repository (the Tumbleweed OSS
#      repository), so the scheduler can resolve the base-system dep
#      set for the fixture package.
#   2. Creates the `geectl-e2e-hello` package: a file upload into a package
#      that has no `_meta` yet answers 404, so the package is
#      created before its sources are uploaded.
#   3. Uploads the Tumbleweed build configuration as the project's
#      `_config`: without it the scheduler reports "bad build
#      configuration, no build type defined or detected", because the
#      build type is guessed from the config's `Preinstall: rpm`.
#      The config also defines the base-system dependency closure.
#   4. Uploads the fixture spec as the `geectl-e2e-hello` package's source.
#   5. Waits for the download-on-demand daemon to recheck this
#      repository (a forced recheck, proved by the freshness cookie
#      timestamp passing the fixture creation time), so the buildinfo
#      poll below never resolves from a payload the daemon has not
#      revalidated since the fixture was created.
#   6. Polls the buildinfo until the scheduler resolves it (no
#      `<error>`, at least one `<bdep>`): the first resolution waits
#      on the download-on-demand daemon fetching the repository
#      metadata. The daemon fetches metadata only, never the base
#      RPMs: at download time a binary whose direct download 404s
#      is warmed by asking the server for its versions, which makes
#      the server fetch it, and the download is retried once.
#
# The instance's bootstrap admin credentials (the OBS development
# defaults, ephemeral, never a real credential) authenticate the
# calls. Usage:
#   GEECTL_LIVE_URL=http://127.0.0.1:3000 bash tests/local_build_e2e/provision-fixture.sh
set -euo pipefail

: "${GEECTL_LIVE_URL:?GEECTL_LIVE_URL must be set to the local OBS instance base URL}"
BASE_URL="$GEECTL_LIVE_URL"
ADMIN_USER="${GEECTL_LIVE_OBS_USERNAME:-Admin}"
ADMIN_PASS="${GEECTL_LIVE_OBS_PASSWORD:-opensuse}"
PROJECT="geectl:e2e"
REPO="standard"
PACKAGE="geectl-e2e-hello"
# The fixture repository covers the runner's arch: the test builds for
# the host arch, so the two agree by construction.
ARCH="${FIXTURE_ARCH:-$(uname -m)}"
# The arch flows into the project `_meta` XML, the cookie path,
# and `sh -c` inside the backend container below, so it must be
# a bare arch name: anything else (a slash, a space, a shell
# metacharacter) would escape the intended path.
case "$ARCH" in
    ''|*[!A-Za-z0-9_]*)
        echo "ARCH must match ^[A-Za-z0-9_]+$: got '$ARCH'" >&2
        exit 1
        ;;
esac
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
# The download-on-demand daemon retries a failed repository fetch
# after 600s, so the poll outlasts one failed fetch plus its retry.
POLL_TIMEOUT_SECONDS="${FIXTURE_POLL_TIMEOUT_SECONDS:-900}"
# Compose command resolution: an explicit COMPOSE wins; otherwise
# prefer `podman compose` when podman is present, else fall back to
# `docker compose`. `podman compose` itself needs a provider binary
# (`docker-compose` or `podman-compose`) to parse the file, so
# podman alone is not enough. CI pins `docker compose` in
# local-build-e2e.yml, so it never depends on the runner image's
# toolset. This mirrors tests/live/provision.sh, so the two scripts
# pick the same engine.
if [ -z "${COMPOSE:-}" ]; then
    COMPOSE="docker compose"
    if command -v podman >/dev/null 2>&1; then
        if command -v docker-compose >/dev/null 2>&1 \
            || command -v podman-compose >/dev/null 2>&1; then
            COMPOSE="podman compose"
        fi
    fi
fi

auth=(-u "$ADMIN_USER:$ADMIN_PASS")

echo "Creating fixture project $PROJECT on $BASE_URL ..."
curl --fail-with-body -sS "${auth[@]}" -X PUT -H 'Content-Type: application/xml' \
    --data-binary @- "$BASE_URL/source/$PROJECT/_meta" <<XML
<project name="geectl:e2e">
  <title>geectl local-build e2e fixture</title>
  <description>Fixture project for the local-build end-to-end job: a download-on-demand repository backed by the Tumbleweed OSS repository, so the scheduler can resolve the base-system dep set for the fixture package without any real OBS credential.</description>
  <repository name="standard">
    <download arch="$ARCH" url="https://download.opensuse.org/tumbleweed/repo/oss/" repotype="rpmmd"/>
    <arch>$ARCH</arch>
  </repository>
</project>
XML

echo "Creating fixture package $PACKAGE ..."
curl --fail-with-body -sS "${auth[@]}" -X PUT -H 'Content-Type: application/xml' \
    --data-binary @- "$BASE_URL/source/$PROJECT/$PACKAGE/_meta" <<XML
<package name="$PACKAGE" project="$PROJECT"><title>geectl local-build e2e fixture package</title><description>Fixture package for the local-build end-to-end job.</description></package>
XML

echo "Uploading fixture project config ..."
curl --fail-with-body -sS "${auth[@]}" -X PUT -H 'Content-Type: application/octet-stream' --data-binary @"$SCRIPT_DIR/tumbleweed.conf" \
    "$BASE_URL/source/$PROJECT/_config"

echo "Uploading fixture spec as package $PACKAGE ..."
# application/octet-stream: without an explicit content type curl
# sends application/x-www-form-urlencoded, and Rack then parses the
# spec body as a form and rejects its `%` macros as invalid
# %-encoding before the controller reads the file.
curl --fail-with-body -sS "${auth[@]}" -X PUT -H 'Content-Type: application/octet-stream' --data-binary @"$SCRIPT_DIR/geectl-e2e-hello.spec" \
    "$BASE_URL/source/$PROJECT/$PACKAGE/geectl-e2e-hello.spec"

# Wait for the download-on-demand daemon to recheck this repository
# before the buildinfo poll runs: the scheduler reads the restored
# payload (when the workflow restored the cache) on its first scan,
# so without this wait the buildinfo could resolve from a payload
# the daemon has not yet revalidated against the live repository.
# The wait forces a recheck and proves one ran after this fixture
# was created:
#   - a `.recheck` file in the daemon's dods directory forces a
#     recheck of the matching repository on the daemon's next scan
#     (it is consumed by the scan, so it is rewritten every poll);
#   - the recheck updates the `doddata.cookie` timestamp whether it
#     succeeds or fails, so the cookie epoch passing the fixture
#     creation time proves a recheck ran, not that it succeeded:
#     a failed recheck leaves the last-known-good payload and the
#     daemon retries on its own schedule.
# The daemon scans at most 10s after the scheduler registers the
# repository, and the first metadata fetch takes minutes at most.
DOD_RECHECK_TIMEOUT_SECONDS="${FIXTURE_DOD_RECHECK_TIMEOUT_SECONDS:-1200}"
dod_cookie="/srv/obs/build/$PROJECT/$REPO/$ARCH/:full/doddata.cookie"
compose_file="$REPO_ROOT/tests/live/docker-compose.yml"
created_at="$(date +%s)"
echo "Waiting for the download-on-demand daemon to recheck $PROJECT/$REPO ..."
deadline=$((SECONDS + DOD_RECHECK_TIMEOUT_SECONDS))
while true; do
    # Force the recheck (rewritten: the daemon consumes the file on
    # its next scan, which may run before the scheduler registered
    # this repository).
    printf '%s\n' "$PROJECT/$REPO" | $COMPOSE -f "$compose_file" exec -T backend tee /srv/obs/dods/geectl-e2e.recheck >/dev/null 2>&1 || true
    cookie_epoch="$($COMPOSE -f "$compose_file" exec -T backend cat "$dod_cookie" 2>/dev/null | awk '{print $1}' || true)"
    if [ -n "$cookie_epoch" ] && [ "$cookie_epoch" -gt "$created_at" ] 2>/dev/null; then
        echo "Download-on-demand daemon rechecked $PROJECT/$REPO."
        break
    fi
    if [ "$SECONDS" -ge "$deadline" ]; then
        echo "download-on-demand daemon did not recheck $PROJECT/$REPO within ${DOD_RECHECK_TIMEOUT_SECONDS}s" >&2
        # The daemon rescans only on startup or when the scheduler
        # registers the repository (doddata file plus `.changed` in
        # the dods directory), so show the scheduler's state
        # alongside the daemon log. Never let the diagnostics mask
        # the failure.
        echo "--- fixture diagnostics (backend container) ---" >&2
        $COMPOSE -f "$compose_file" exec -T backend sh -c 'ps aux | grep "[b]s_sched" || echo "scheduler process not running"' >&2 || true
        $COMPOSE -f "$compose_file" exec -T backend sh -c 'tail -40 /srv/obs/log/scheduler_x86_64.crit.log 2>&1' >&2 || true
        $COMPOSE -f "$compose_file" exec -T backend sh -c 'ls -la /srv/obs/dods/ 2>&1' >&2 || true
        $COMPOSE -f "$compose_file" exec -T backend sh -c "ls -lad /srv/obs/build/$PROJECT/$REPO/$ARCH/ 2>&1" >&2 || true
        $COMPOSE -f "$compose_file" exec -T backend sh -c "ls -lad /srv/obs/dod-cache 2>&1" >&2 || true
        $COMPOSE -f "$compose_file" exec -T backend sh -c "ls /srv/obs/build/$PROJECT/$REPO/$ARCH/:full/ 2>&1 | head -20" >&2 || true
        $COMPOSE -f "$compose_file" exec -T backend sh -c "cat /srv/obs/build/$PROJECT/$REPO/$ARCH/:full/doddata.cookie 2>&1" >&2 || true
        $COMPOSE -f "$compose_file" exec -T backend sh -c 'tail -40 /srv/obs/log/bs_dodup.log 2>&1' >&2 || true
        $COMPOSE -f "$compose_file" logs backend --tail 300 2>&1 | grep -iE 'sched|dod' | tail -40 >&2 || true
        exit 1
    fi
    sleep 10
done

echo "Waiting for the buildinfo to resolve ..."
buildinfo_url="$BASE_URL/build/$PROJECT/$REPO/$ARCH/$PACKAGE/_buildinfo"
deadline=$((SECONDS + POLL_TIMEOUT_SECONDS))
body_file="$(mktemp)"
trap 'rm -f "$body_file"' EXIT
attempt=0
while true; do
    attempt=$((attempt + 1))
    # A 404 means the scheduler has not registered the package yet:
    # keep waiting. Auth and server errors fail fast instead of
    # burning the whole timeout.
    http_code="$(curl -sS -o "$body_file" -w '%{http_code}' "${auth[@]}" "$buildinfo_url" || true)"
    case "$http_code" in
        200|404) ;;
        *)
            echo "buildinfo request failed with HTTP $http_code:" >&2
            cat "$body_file" >&2
            exit 1
            ;;
    esac
    last_body="$(cat "$body_file")"
    if [ -n "$last_body" ] && ! printf '%s' "$last_body" | grep -q '<error>' \
        && printf '%s' "$last_body" | grep -q '<bdep'; then
        echo "Buildinfo resolved after $attempt poll(s): $(printf '%s' "$last_body" | grep -o '<bdep' | wc -l) deps."
        break
    fi
    if [ "$SECONDS" -ge "$deadline" ]; then
        echo "buildinfo did not resolve within ${POLL_TIMEOUT_SECONDS}s" >&2
        echo "last buildinfo body:" >&2
        printf '%s\n' "$last_body" >&2
        # The failure is in the instance, not the fixture: show what
        # the download-on-demand daemon did, so the next run does not
        # have to guess. Never let the diagnostics mask the failure.
        compose_file="$REPO_ROOT/tests/live/docker-compose.yml"
        if [ -f "$compose_file" ]; then
            echo "--- fixture diagnostics (backend container) ---" >&2
            $COMPOSE -f "$compose_file" exec -T backend sh -c 'ps aux | grep "[b]s_dodup" || echo "dodup process not running"' >&2 || true
            $COMPOSE -f "$compose_file" exec -T backend sh -c 'ls -la /srv/obs/dods/ 2>&1' >&2 || true
            $COMPOSE -f "$compose_file" exec -T backend sh -c "ls -lad /srv/obs/build/$PROJECT/$REPO/$ARCH/ 2>&1" >&2 || true
            $COMPOSE -f "$compose_file" exec -T backend sh -c "ls -lad /srv/obs/dod-cache 2>&1" >&2 || true
            $COMPOSE -f "$compose_file" exec -T backend sh -c "ls /srv/obs/build/$PROJECT/$REPO/$ARCH/:full/ 2>&1 | head -20" >&2 || true
            $COMPOSE -f "$compose_file" exec -T backend sh -c 'tail -40 /srv/obs/log/bs_dodup.log 2>&1' >&2 || true
        fi
        exit 1
    fi
    if [ $((attempt % 6)) -eq 0 ]; then
        echo "still waiting for the buildinfo ($attempt polls) ..."
    fi
    sleep 10
done

echo "Checking the buildconfig ..."
curl --fail-with-body -sS "${auth[@]}" "$BASE_URL/build/$PROJECT/$REPO/_buildconfig" -o /dev/null
echo "Fixture project $PROJECT is ready."
