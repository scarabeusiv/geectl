# Temporal OBS instance — live tests

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

There is no published prebuilt frontend server image — the OBS project
builds it from source, and so do we. The build worker is omitted: no
test builds a package, and the worker needs a privileged container.

The instance tracks OBS master (`obs/server/unstable`). That is
deliberate: the temporal job exists to catch drift as OBS changes, so it
tracks the newest OBS, not a stable release. The job is independent and
non-gating, so a master breakage blocks nothing — it just needs fixing.

In CI the frontend build reuses a BuildKit GitHub Actions cache
(`docker-compose.cache.yml`, loaded only when `TEMPORAL_CACHE` is
set), so repeat runs skip the gem install when the OBS source has
not changed. The cache is content-addressed, so drift detection is
unaffected. Only the frontend is cached: db, cache and backend are
published images pulled in seconds, and caching them would pin a
`latest` image and hide drift.

## Run it locally

```sh
bash tests/temporal/provision.sh   # clones OBS (depth 1), builds, bootstraps, waits
GEECTL_TEMPORAL_URL=http://127.0.0.1:3000 cargo test --locked --test temporal
bash tests/temporal/teardown.sh    # removes containers and volumes
```

`provision.sh` clones the OBS source to `tests/temporal/obs` by
default (override with `OBS_SOURCE_DIR`). The first run is slow: the
OBS checkout plus the frontend image build take several minutes. Later
runs reuse both. Set `COMPOSE="podman compose"` if you use podman
instead of docker.

## The instance's credentials

The bootstrap seeds the Admin user `Admin` / `opensuse` — the OBS
development default, published in the OBS repo's `src/api/db/seeds.rb`.
It is not a real credential: the instance is loopback-only, created
fresh per run, and destroyed after. Nothing secret is ever committed
or leaves the machine.

## The harness

`tests/temporal.rs` is env-gated on `GEECTL_TEMPORAL_URL`: without it,
the tests skip and `cargo test` stays green. With it, they assert on
live server state through geectl's own HTTP client:

- `GET /about` returns a real OBS (title + non-empty revision)
- a missing project maps to geectl's `NotFound`
- the project list at `GET /source` returns a `<directory>` of `<entry>` elements

The instance serves plain HTTP on localhost. That is the single
sanctioned exception to geectl's https-only rule (see `tests/temporal.rs`):
loopback-only, ephemeral instance, ephemeral credentials, and the
bootstrap admin credentials only for the mutation tests. The mutation
assertion (create a project through geectl, read it back) runs live
(`temporal_create_project_shows_up_on_server`); the submit-request
tests live in `tests/temporal_obs.rs`.
