#!/usr/bin/env bash
# Populate the disposable Gitea instance with the real fixtures the
# live tests exercise — the production layout mirrored from
# code.opensuse.org (see the layout note in src/gitea.rs). This is
# its own step after provisioning, so the live tests never create
# fixtures inline: they assume the mirror exists and fail clearly
# when it does not.
#
# What it creates (exactly what the live tests exercise, no more):
#   - org `pool`
#   - repo `pool/systemd`, default branch `factory`
#
# Idempotent: a re-run against the same instance reuses the existing
# org/repo instead of failing on the duplicate — the fixed names
# cannot be unique per run, and that is the point of the mirror. If
# the repo exists with a different default branch, that is a
# mismatch — fail loudly instead of silently passing.
#
# Inputs (same as the live-test harness):
#   GEECTL_GITEA_URL — base URL of the provisioned instance
#     (default http://127.0.0.1:${GITEA_PORT:-3001}). Must be
#     loopback-only: the token travels in cleartext over plain HTTP,
#     so a non-localhost URL is refused, not silently used.
#   GEECTL_GITEA_TOKEN, or GEECTL_GITEA_TOKEN_FILE (default
#     tests/gitea/.gitea-token, the file provision.sh writes). The
#     token is never printed.
#
# Usage:
#   bash tests/gitea/populate-fixtures.sh
#   GEECTL_GITEA_URL=http://127.0.0.1:3002 bash tests/gitea/populate-fixtures.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BASE_URL="${GEECTL_GITEA_URL:-http://127.0.0.1:${GITEA_PORT:-3001}}"
# Strip every trailing slash: one would leave `//api/v1/…` paths,
# which 404 and abort loudly — but fail on the guard instead.
while [ "${BASE_URL%/}" != "$BASE_URL" ]; do BASE_URL="${BASE_URL%/}"; done

# Loopback-only: the token goes in an Authorization header over
# plain HTTP, so the instance must be local. Anything else is a
# misconfiguration, not a target. Parse the host properly — a
# prefix match is not enough: `http://127.0.0.1.evil.com` and
# `http://127.0.0.1@evil.com` both start with a loopback literal
# but point at evil.com (the Rust harness guards the same forms;
# see `is_loopback_url` in src/http.rs, shared with the transport
# guard). The host must compare exactly after stripping scheme,
# userinfo, port, path, query and fragment: an `@` in the query or
# fragment is not a userinfo separator — `http://evil.com?@127.0.0.1`
# points at evil.com, so the authority ends at the first of `/`, `?`,
# `#` before the userinfo split, exactly like the Rust check.
case "$BASE_URL" in
    http://*) rest="${BASE_URL#http://}" ;;
    *)
        echo "GEECTL_GITEA_URL must be a loopback URL (http://127.0.0.1, http://localhost, or http://[::1])" >&2
        exit 1
        ;;
esac
# A base URL carries no query or fragment: `?x`/`#x` would swallow
# the appended /api/v1/… path (the server would answer the homepage
# instead), and the failure would misdiagnose as a wrong default
# branch. Reject, do not strip — silently rewriting the operator's
# URL would hide the mistake.
case "$rest" in
    *\?*|*#*)
        echo "GEECTL_GITEA_URL must be a plain base URL without a query or fragment" >&2
        exit 1
        ;;
esac
host="${rest%%[/?#]*}"   # drop path, query and fragment
host="${host##*@}"   # drop any userinfo (the last '@' wins)
case "$host" in
    \[*\]) ;;                   # bare [::1]
    \[*\]:*) host="${host%%\]:*}]" ;;   # [::1]:3001 -> [::1]
    *) host="${host%%:*}" ;;     # drop :port
esac
case "$host" in
    127.0.0.1|localhost|'[::1]') ;;
    *)
        echo "GEECTL_GITEA_URL must be a loopback URL (http://127.0.0.1, http://localhost, or http://[::1])" >&2
        exit 1
        ;;
esac

if [ -n "${GEECTL_GITEA_TOKEN:-}" ]; then
    TOKEN="$GEECTL_GITEA_TOKEN"
elif [ -n "${GEECTL_GITEA_TOKEN_FILE:-}" ]; then
    if [ ! -r "$GEECTL_GITEA_TOKEN_FILE" ]; then
        echo "GEECTL_GITEA_TOKEN_FILE is set but not readable: $GEECTL_GITEA_TOKEN_FILE" >&2
        exit 1
    fi
    TOKEN="$(cat "$GEECTL_GITEA_TOKEN_FILE")"
elif [ -f "$SCRIPT_DIR/.gitea-token" ]; then
    TOKEN="$(cat "$SCRIPT_DIR/.gitea-token")"
else
    echo "no Gitea token: set GEECTL_GITEA_TOKEN or GEECTL_GITEA_TOKEN_FILE (provision.sh writes tests/gitea/.gitea-token)" >&2
    exit 1
fi
if [ -z "$TOKEN" ]; then
    echo "Gitea token is empty" >&2
    exit 1
fi

# GET status code only, no body: the token is in the header and the
# body is never printed. Exits non-zero only on a transport failure.
status() {
    curl --max-time 10 -s -o /dev/null -w "%{http_code}" \
        -H "Authorization: token $TOKEN" "$1"
}

# POST a JSON body; on an HTTP error curl -f exits non-zero and prints
# only its own error line — the response body is suppressed, not
# shown. On success the body is discarded (>/dev/null). The token is
# in the header, never in the body.
post_json() {
    curl --max-time 10 -fsS \
        -H "Authorization: token $TOKEN" \
        -H "Content-Type: application/json" \
        -d "$2" "$1" >/dev/null
}

if [ "$(status "$BASE_URL/api/v1/orgs/pool")" = "200" ]; then
    echo "org pool already exists"
else
    post_json "$BASE_URL/api/v1/orgs" '{"username":"pool"}'
    echo "org pool created"
fi

code="$(status "$BASE_URL/api/v1/repos/pool/systemd")"
if [ "$code" = "200" ]; then
    # The repo exists: its default branch must be the mirrored one.
    # A repo of the same name with another default branch is a
    # different fixture, not this one.
    body="$(curl --max-time 10 -fsS -H "Authorization: token $TOKEN" "$BASE_URL/api/v1/repos/pool/systemd")"
    if printf '%s' "$body" | grep -q '"default_branch"[[:space:]]*:[[:space:]]*"factory"'; then
        echo "repo pool/systemd already exists (default branch factory)"
    else
        echo "repo pool/systemd exists with a different default branch; expected factory" >&2
        exit 1
    fi
else
    post_json "$BASE_URL/api/v1/orgs/pool/repos" '{"name":"systemd","private":false,"default_branch":"factory"}'
    echo "repo pool/systemd created (default branch factory)"
fi

echo "Gitea fixtures populated."
