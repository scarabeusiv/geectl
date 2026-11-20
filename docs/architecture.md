# Architecture

## Crate shape: lib + bin

geectl is one crate with a library and a binary:

- `src/lib.rs` — the library. The binary is a thin CLI over it, so tools
  can link the same logic instead of shelling out to the binary. The
  library is the core; the binary is one consumer of it.
- `src/main.rs` — the binary. Argument parsing (clap), one exit path,
  stdout carries only the result payload.

The library modules at scaffolding time:

| Module | Contents |
| ------ | -------- |
| `error` | The centralized error store (see below). |
| `url` | The single URL builder (one encoder per context). |
| `http` | The HTTP client trait with a bounded timeout, plus a ureq implementation. |
| `auth` | The credential types and resolution (see below). |
| `config` | The XDG config file (URLs only) and the secure-write primitive. |

Later modules (OBS operations, Gitea operations, XML→JSON mapping) plug
into this shape; they do not change it.

## The error store (`src/error.rs`)

Every error geectl can produce is defined in one place: `ErrorKind`, with
a stable code, a human message, a remediation, and an exit code per kind.
`GeectlError` is a kind plus the context of the instance (what resource,
what request) — callers match on the kind, never on the context string.

Rules:

- There is exactly one exit path: the binary maps the error kind to its
  exit code. No code anywhere else invents an exit code or a free-text
  error identity.
- The error payload is stable JSON:
  `{"error": {"code": ..., "message": ..., "remediation": ...,
  "context": ...}}`. `context` is omitted when empty.
- The error path has its own test: every kind renders to JSON and to
  Display without panicking. Error handling is the one path that must
  work when everything else is already broken.
- The server's error body is parsed with the same hardened XML parser as
  everything else; if it does not parse, the raw status plus a truncated
  body excerpt are reported as data, never scraped with string splits.

## The URL builder (`src/url.rs`)

There is exactly one place that builds URLs, and one encoder per context:

- Path segments are encoded with an empty safe set except `:` (the OBS
  `project:package` separator). `/` is always encoded, never passed
  through, and segments are never stripped or trimmed — a segment
  containing `/` becomes one encoded element (`%2F`), never two path
  elements.
- Query values use the query encoder, where `+` is encoded (`%2B`)
  because a query string decodes `+` as a space.
- No raw-string query path: query pairs go through the encoder.

Encoding is a property of the builder, never of each caller remembering
to encode.

## The HTTP client (`src/http.rs`)

- Every request carries a bounded connect timeout (default 10s) and a
  bounded read timeout (default 30s), unconditional on library version.
  The timeout values are documented defaults, overridable by explicit
  config, never by accident.
- Retry policy, as a single documented constant: only idempotent methods
  are retried (GET, HEAD, PUT, DELETE), up to 3 times with doubling
  backoff. POST is never retried without an explicit idempotency key.
  `post_json` (JSON body, `Content-Type: application/json`) never
  retries at all: a retried mutation can create a duplicate, so every
  non-2xx status is returned immediately.
- Non-2xx statuses are converted to a `GeectlError` at this layer, before
  any caller sees the body, so callers never have to remember to check
  the status. 404 maps to `NotFound`, 401/403 to `AuthFailed`, other
  statuses to `ServerError`.
- Auth refresh happens before the body is consumed, or the body is
  buffered and replayable by construction. A retry after 401 must verify
  the body is byte-identical or refuse to retry.

## Auth (`src/auth.rs`, `src/http.rs`)

Credentials are resolved by providers implementing the `AuthProvider`
trait (`src/auth.rs`): `EnvObsAuth` (OBS Basic from
`GEECTL_OBS_USERNAME` + `GEECTL_OBS_PASSWORD` or
`GEECTL_OBS_PASSWORD_FILE`), `EnvGiteaAuth` (Gitea Bearer from
`GEECTL_GITEA_TOKEN` or `GEECTL_GITEA_TOKEN_FILE`), and `GiteaSshKey`
(the SSH private key from `GEECTL_GITEA_SSH_KEY`). Unconfigured
resolves to `Credentials::None`; a half-configured pair, both secret
sources set, or a malformed value is a `ConfigError` naming the
source, never the value. The SSH key must be a PEM private key in a
file mode 0600 or stricter (unix).

The HTTP client (`src/http.rs`) applies the credentials per request:
Basic as an `Authorization: Basic` header, Bearer as
`Authorization: Bearer`. An SSH key cannot authenticate an HTTP
request and is refused before any network traffic. Credentials are
only sent over `https://`, or to a loopback host (for tests with
fixture credentials) — cleartext http to any other host is refused
before connecting.

Rules that hold:

- Credentials come from environment variables or mounted secret files
  only. There is no plaintext fallback and no obfuscation option.
- No credential is ever stored in a config file, in any form. A
  credential-shaped value in a config file is a refusal to run.
- Secrets are never logged. Any file that can contain a credential is
  created with mode 0600 from the first byte.
- TLS verification is always on. There is no option to disable it, and
  no "continue anyways".

## Output contract

- Stdout is only the result payload (JSON by default, one uniform
  `--format` across every command). Every diagnostic, progress line,
  warning, and error goes to stderr. Verbosity never changes stdout.
- JSON output means server text is data, never terminal input, so there
  is nothing to sanitize in machine output. Human-readable output is
  rendered from parsed data through one renderer that escapes by
  construction. No raw print of server text anywhere.
- There are no prompts, no editor, no pager, and no input primitive
  anywhere. Missing required input fails fast with a structured error
  naming the missing flag.

## Configuration

- One config file: `$XDG_CONFIG_HOME/geectl/config.toml` (default
  `~/.config/geectl/config.toml`). Explicit precedence, documented in
  one place: flag > environment variable > config file > built-in
  default. The built-in defaults are `https://api.opensuse.org` (OBS)
  and `https://src.opensuse.org` (Gitea).
- A `config show` command prints the effective configuration with the
  source of each value. If two config files are found, that is an
  error, not a warning.
- A `config init` command writes the template (endpoints only) and
  refuses to overwrite an existing file.

## Testing

Three layers, by design:

1. **Fixtures per-PR (gating).** Fast, deterministic, recorded responses.
   Every PR runs them; they gate the merge.
2. **Live Gitea tests per-PR (gating).** A disposable Gitea container
   is provisioned in the `test` job, and `tests/gitea.rs` authenticates
   against it with geectl's own `EnvGiteaAuth` and does real API calls
   (read the user, create a repository, read it back, list it),
   asserting on server state. This is the auth validation the fixtures
   cannot do: the unit tests prove geectl sends the right `Authorization`
   header; these prove a real Gitea accepts the token and returns valid
   responses. They gate because they are fast (~20s end to end: a
   single prebuilt image), unlike the OBS temporal job.
3. **Scheduled live tests against a temporal OBS instance (independent
   CI job).** We spin up our own (temporary) OBS instance, connect to
   it, and collect responses. The key assertion is on **server state**:
   whatever change the CLI did must actually show up on the server —
   that proves the CLI did what it claimed, not just that it printed
   the right thing. This runs on a schedule, **not** per-PR. It is an
   independent CI job: when it regresses, it is fixed independently; it
   does not block PRs. It exists to catch drift as the server changes
   underneath us, which fixtures alone cannot do. The OBS instance is
   slow on purpose (source checkout + frontend build, minutes); that is
   why it is scheduled while the Gitea test gates.

### The temporal layer in detail

**Provisioning (`tests/temporal/`).** The instance is the OBS project's
own development container setup, trimmed to what the live tests need:
MariaDB, memcached, the backend, and the frontend (the API). The build
worker is omitted — no test builds a package, and the worker needs a
privileged container. The images are the published OBS master images at
`registry.opensuse.org/obs/server/unstable/containers/containers/openbuildservice/<name>`;
the frontend is built from the OBS source (`src/api`) because there is
no published prebuilt frontend server image. `provision.sh` clones the
OBS source (depth 1), builds the frontend, runs `rake dev:bootstrap`
(which creates the database and seeds the Admin user), and polls the
public `/about` endpoint until the API answers. `teardown.sh` removes
the containers and volumes. The instance tracks OBS master on purpose:
the job exists to catch drift, so it tracks the newest OBS, not a
stable release.

**Schedule.** The `temporal` workflow runs weekly (Mondays 06:00 UTC,
the same slot as `freshness`) and on demand (`workflow_dispatch`), on a
GitHub-hosted runner. It has no `pull_request` trigger and is not a
required status check. It is slow on purpose (source checkout, frontend
build, bootstrap); the timeout is 60 minutes.

**Harness (`tests/temporal.rs`).** The tests are env-gated on
`GEECTL_TEMPORAL_URL`: without it they skip, so `cargo test` stays
green with no instance. With it, they assert on live server state
through geectl's own HTTP client: `GET /about` returns a real OBS
(title + non-empty revision), a missing project maps to geectl's
`NotFound`, and the public project list returns the OBS collection
envelope. The mutation assertions (create a project through geectl,
read it back) are wired up but ignored until the auth provider and a
project-create operation exist — see
`temporal_create_project_shows_up_on_server`.

**Plain HTTP on localhost is the single sanctioned exception to the
https-only URL rule.** The OBS development frontend serves plain HTTP;
there is no TLS to terminate against, and a self-signed certificate
would fail geectl's always-on TLS verification anyway. The exception
is safe because the instance is loopback-only (the harness rejects any
non-loopback URL outright), ephemeral (created fresh per run,
destroyed after), and the credentials are ephemeral (the OBS
development default, never a real credential). This layer runs with no
auth at all; when auth lands, the credentials it sends go to the same
loopback-only ephemeral instance. The harness talks to the instance
through `UreqClient` directly, not `UrlBuilder` (which rightly rejects
`http://` bases) — the exception is confined to that one file.

## Distribution

geectl is distributed as an OBS package with tagged releases, fetched via
zypper. Containers fetch the released version — no building random
hashes in containers.
