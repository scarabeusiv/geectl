# Live OBS instance — live tests

The live tests (layer 2 of the testing design) run against a
real, temporary OBS instance — on every pull request, weekly, and on
demand. This directory holds the provisioning for it: the container
setup, the bootstrap, and the teardown.

## What it is

A throwaway OBS built from the OBS project's own development containers:

- `db` — MariaDB (`registry.opensuse.org/obs/server/unstable/.../mariadb`)
- `cache` — memcached (`.../memcached`)
- `backend` — the OBS backend (`.../backend`), running its development
  start script from the mounted OBS source checkout
- `frontend` — the OBS API, built from the OBS source (`src/api`) on top
  of the published `frontend-features` image

There is no published prebuilt frontend server image; the frontend
is built from source. The build worker is omitted: no
test builds a package, and the worker needs a privileged container.

The instance tracks OBS master (`obs/server/unstable`): the live job
catches drift as OBS changes, so it tracks the newest OBS, not a
stable release. The job is independent and non-gating: a master
breakage blocks nothing.

In CI the frontend build reuses a BuildKit GitHub Actions cache
(`docker-compose.cache.yml`, loaded only when `LIVE_CACHE` is
set), so repeat runs skip the gem install when the OBS source has
not changed. The cache is content-addressed, so drift detection is
unaffected. Only the frontend is cached: db, cache and backend are
published images pulled in seconds, and caching them would pin a
`latest` image and hide drift.

## Run it locally

```sh
bash tests/live/provision.sh   # clones OBS (depth 1), builds, bootstraps, waits
GEECTL_LIVE_URL=http://127.0.0.1:3000 cargo test --locked --test live
bash tests/live/teardown.sh    # removes containers and volumes
```

`provision.sh` clones the OBS source to `tests/live/obs` by
default (override with `OBS_SOURCE_DIR`). The first run is slow: the
OBS checkout plus the frontend image build take several minutes. Later
runs reuse both. Set `COMPOSE="podman compose"` if you use podman
instead of docker.

## Two instances (cross-instance copy)

The cross-instance test (`tests/live_cross_instance.rs`, the proof
for `branch copy-package --to-apiurl`) needs a second OBS instance.
Provision it as a second stack — its own compose project, frontend
port, and OBS checkout — alongside the first:

```sh
COMPOSE_PROJECT_NAME=geectl-live-2 FRONTEND_PORT=3001 OBS_SOURCE_DIR=tests/live/obs-2 bash tests/live/provision.sh
GEECTL_LIVE_URL=http://127.0.0.1:3000 GEECTL_LIVE_URL_2=http://127.0.0.1:3001 cargo test --locked --test live_cross_instance
COMPOSE_PROJECT_NAME=geectl-live-2 OBS_SOURCE_DIR=tests/live/obs-2 bash tests/live/teardown.sh
```

The second stack's frontend build is a cache hit (the first stack
already built the image), but its bootstrap runs again against its
own database. The two stacks share nothing but the host. The second
checkout exists because the two backends mount the checkout, and
sharing one between two running backends is unproven.

## The instance's credentials

The bootstrap seeds the Admin user `Admin` / `opensuse` — the OBS
development default, published in the OBS repo's `src/api/db/seeds.rb`.
It is not a real credential: the instance is loopback-only, created
fresh per run, and destroyed after. Nothing secret is ever committed
or leaves the machine.

## The harness

`tests/live.rs` is env-gated on `GEECTL_LIVE_URL`: without it,
the tests skip and `cargo test` stays green. With it, they assert on
live server state through geectl's own HTTP client:

- `GET /about` returns a real OBS (title + non-empty revision)
- a missing project maps to geectl's `NotFound`
- the project list at `GET /source` returns a `<directory>` of `<entry>` elements

The instance serves plain HTTP on localhost. That is the
exception to geectl's https-only rule (see `tests/live.rs`):
loopback-only, ephemeral instance, ephemeral credentials, and the
bootstrap admin credentials only for the mutation tests. The mutation
assertion (create a project through geectl, read it back) runs live
(`live_create_project_shows_up_on_server`); the submit-request
tests live in `tests/live_obs.rs`.

`tests/live_cross_instance.rs` is env-gated on both `GEECTL_LIVE_URL`
and `GEECTL_LIVE_URL_2`: without both, it skips. With both, it copies
a package (text + binary files) from the first instance to the second
through geectl's own `copy_package_to_instance` and asserts on the
second instance's server state — same files, same md5s, byte-identical
reads, carried-over title and description — then copies again (nothing
to copy: the md5 skip) and copies a link package (expanded, no `_link`
file on the target).
