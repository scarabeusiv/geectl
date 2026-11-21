# Disposable Gitea instance — live tests

The live Gitea tests (`tests/gitea.rs`) run against a real, temporary
Gitea instance. This directory holds the provisioning for it: the
container setup, the bootstrap, and the teardown.

## What it is

A throwaway Gitea: a single container (`gitea/gitea:1.24.7`, pinned),
SQLite, loopback-only. No separate database, no backend/frontend split
— that is the whole point: Gitea is one container where OBS needs four
services and a source checkout.

The image is pinned to a stable release, not `:latest`. The pin is
bumped by hand (it lives in `IMAGE` in `provision.sh`); the live tests
in the gate prove a new version still accepts geectl's auth. A moving
tag would let a Gitea release break the per-PR gate for reasons
unrelated to geectl. (Dependabot's `docker` ecosystem only scans
Dockerfiles and compose files, and this setup uses plain container
commands, so it cannot track the pin.)

## Run it locally

```sh
CONTAINER=podman bash tests/gitea/provision.sh   # starts Gitea, creates the user + token
bash tests/gitea/populate-fixtures.sh            # creates the pool/systemd mirror fixture
GEECTL_GITEA_URL=http://127.0.0.1:3001 GEECTL_GITEA_TOKEN_FILE=tests/gitea/.gitea-token cargo test --locked --test gitea
CONTAINER=podman bash tests/gitea/teardown.sh    # removes the container, volume, and token file
```

`CONTAINER` selects the runtime: `docker` by default (CI runners),
`podman` on the Mac. Plain container commands are used, not compose:
Gitea is a single container, so compose adds nothing — and
`podman compose` on the Mac is only a wrapper around an external
provider (`docker-compose` / `podman-compose`), neither of which is
installed, so compose does not work there at all.

`provision.sh` creates the test user (`geectl`, admin) and an API token
with the minimal scopes the tests need
(`read:user,read:repository,write:repository,write:user,write:organization` — repo creation
needs `write:user` as well as `write:repository`; `write:organization` for the `pool` org
the fixtures create and the cross-org fork-refusal test). The token is written
to a 0600 file (`.gitea-token`, gitignored) and never printed. The
instance is destroyed after the run, so the token never outlives it.

## Timing

Measured cold on the Mac (2026-11-20):

- image pull (cold): ~7s
- container start to `/api/healthz`: ~3s
- user + token creation: ~2s
- live test run: ~5s

Total provisioning + test is ~20s, which is why this runs in the
per-PR gate (the `test` job) rather than on a schedule like the OBS
live job. The OBS instance is slow on purpose (source checkout +
frontend build, minutes); Gitea is a single prebuilt image.

## The harness

`tests/gitea.rs` is env-gated on `GEECTL_GITEA_URL` (and
`GEECTL_GITEA_TOKEN` / `GEECTL_GITEA_TOKEN_FILE`): without them, the
tests skip and `cargo test` stays green. With them, they assert on live
server state through geectl's own HTTP client, with geectl's own
`EnvGiteaAuth` (Bearer token):

- `GET /api/v1/user` returns the provisioned user (the token is
  accepted and the response is valid — this is the auth validation
  the unit tests cannot do)
- `POST /api/v1/user/repos` creates a repository (201)
- `GET /api/v1/repos/{owner}/{repo}` reads it back (server state
  shows the creation)
- `GET /api/v1/user/repos` lists it
- a missing repository maps to geectl's `NotFound`

The instance serves plain HTTP on localhost. That is the single
sanctioned exception to geectl's https-only rule (see `tests/gitea.rs`):
loopback-only, ephemeral instance, ephemeral credentials.
