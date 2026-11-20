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
| `obs` | The OBS operations (devel lookup, open-SR check, SR filing, superseding, branch cleanup — see below). |

Later modules (Gitea operations, XML→JSON mapping) plug into this
shape; they do not change it.

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
  are retried (GET, HEAD, PUT), up to 3 times with doubling
  backoff. POST is never retried without an explicit idempotency key,
  and DELETE is never retried either (single-attempt by design: a
  retried delete cannot tell a lost response from a delete that never
  ran). `post_json` (JSON body, `Content-Type: application/json`) never
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

## OBS operations (`src/obs.rs`)

The library surface a tool uses to move a package update through the
distribution: `ObsClient` (devel-project lookup, devel version, open
submit-request check, filing, superseding, branch cleanup), `SrPayload`
(the validated submit-request payload), `SrFiling` (the only filing
path, dry-run by default; `refile` is the file-then-supersede flow),
`DryRun` (the would-do guard), and `CveId` (CVE id parsing plus the
changelog-names-CVE rule).

Behaviour rules (from the openSUSE packaging guide):

- A submit request targets the DEVEL project, never Factory directly.
  A payload naming `openSUSE:Factory` as its target is refused at
  build time.
- An open submit request for the package must be checked before filing
  (dedup). `SrFiling::file` performs that check itself in live mode
  and refuses to file a duplicate; the check is also available on its
  own as `ObsClient::has_open_sr`.
- The SR description follows the guide's message format: one headline
  of at most 160 characters of prose, then `- ` lines for staging,
  needs, and rebuilds. The library validates the description is
  non-empty; the headline shape is the caller's responsibility.
- A security fix's `.changes` entry must name every CVE it fixes.
  When the caller supplies CVE ids, `SrPayload::new` refuses a
  `.changes` entry that does not name each of them.
- A failed lookup must never read as good news: a lookup that errors
  (transport, auth, unexpected response) returns an error, never a
  silent "not found" or "no open SR". Only a successful response that
  carries no result reads as "none" / "false".

The full lifecycle is file -> supersede -> delete branch, and the
branch dies by default: every payload carries
`opt_sourceupdate="cleanup"`, so the new branch is deleted when its
request is accepted. `SrFiling::refile` files the new request first
and only then supersedes the old one with it, so `superseded_by`
always names a request that exists. Branch deletion
(`ObsClient::delete_package`) is a separate explicit call for the
old branch — a superseded request never reaches accept, so the
server-side cleanup never runs for it — and the delete refuses
while requests in `new`, `review`, or `declined` state involve the
package as a submit source or target.

The supersede and delete semantics are validated against the OBS API
server code, not copied from osc — osc's flow is a reference to check
against, not ground truth. Where osc and the API disagree, the API
wins:

- `superseded_by` is required by the server (a supersede without it
  is refused); the library always sends it.
- Superseding is allowed from `new`, `review`, and `declined`, and
  refused from any final state (`accepted`, `superseded`, `revoked`).
  The library does not pre-check the old request's state; the server
  stays the authority and its 403 is surfaced as-is.
- The server does not verify that `superseded_by` names a real
  request, so the file-first ordering in `refile` is what makes the
  link real.
- A package delete does not fail on open requests: the API revokes
  them as source and declines them as target. The library refuses the
  delete instead, naming the blocking requests, so a branch deletion
  can never silently kill a live submit request. `force` bypasses the
  guard and is forwarded as `force=1`, which also bypasses the
  server's weak-dependency guard (refuse when other packages use this
  one as their devel package) — osc's `force` semantics.
- If superseding fails after the new request is filed, `refile`
  returns an error naming the new id. That is a deliberate divergence
  from osc, which supersedes with `can_fail=True` and only warns: a
  half-done refile passing silently is a failed refile that reads as
  a success.

Branch cleanup is the default: every payload carries
`opt_sourceupdate="cleanup"` (what `osc sr --cleanup` sets), so OBS
deletes the source branch when the request is accepted — and the
source project with it, when that was its last package and it is not
a top-level `home:<user>` project. The packaging guide requires it
for filings from a `home:` subproject; pr-guard refuses the SR
without it. The values and the accept behaviour are validated
against the OBS API server code (`BsRequestAction` accepts only
`update`, `noupdate`, `cleanup`; `BsRequestActionSubmit` deletes
the source package on accept when the value is `cleanup`). The
server also defaults to cleanup itself when no value is given and
the source is the filer's branch project for the target, but the
library sends the value explicitly: the server default only covers
branch projects, and the explicit value is what pr-guard checks.
A forward from a devel project must NOT carry cleanup — accept
would delete the devel package — so that case opts out with
`SrPayload::without_cleanup`, which sends `update` instead.
`ObsClient::delete_package` remains the manual cleanup for the
branches the server-side cleanup never reaches: a superseded
request's branch and a declined request's branch.

Credentials: the public reads (devel project, devel version) need
none; the open-SR search, the filing POST, superseding, and deleting
need OBS Basic and fail loudly without them. OBS accepts Basic only —
a Bearer token or SSH key is refused at client construction. No
credential is stored here, and no request carries one over cleartext
http except to a loopback host (the transport guard in `http.rs`).

Dry-run is the default (`DryRun::default()` files nothing): every
mutating operation goes through the single guard (`DryRun::allow`)
before any write or API call. Reads are not gated. The live executor
that actually branches, bumps, cleans, builds, and runs the package
gates is NOT part of this module: the library builds and files the
submit-request payload; the caller owns the package work.

The OBS API base is `https://api.opensuse.org` by default. A custom
base is accepted for a test instance (`ObsClient::with_base`): it must
be `https://` with a host, or `http://` on a loopback host. URLs are
built with the encoders from `url.rs` (one encoder per context); the
base itself is carried as a string because the URL builder rightly
rejects `http://` bases, and the test instance is plain http on
loopback.

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
OBS source (depth 1, plus its `src/backend/build` submodule — the Build
Perl modules the backend daemons need), builds the frontend, runs `rake
dev:bootstrap` (which creates the database and seeds the Admin user),
polls the public `/about` endpoint until the API answers, and probes
the backend source server until it answers (`/about` needs only the
frontend and the database; project writes go to the backend).
`teardown.sh` removes the containers and volumes. The instance tracks OBS master on purpose:
the job exists to catch drift, so it tracks the newest OBS, not a
stable release.

**Schedule.** The `temporal` workflow runs weekly (Mondays 06:00 UTC,
the same slot as `freshness`) and on demand (`workflow_dispatch`), on a
GitHub-hosted runner. It has no `pull_request` trigger and is not a
required status check. It is slow on purpose (source checkout, frontend
build, bootstrap); the timeout is 60 minutes.

**Harness (`tests/temporal.rs`, `tests/temporal_obs.rs`).** The tests are env-gated on
`GEECTL_TEMPORAL_URL`: without it they skip, so `cargo test` stays
green with no instance. With it, they assert on live server state
through geectl's own HTTP client: `GET /about` returns a real OBS
(title + non-empty revision), a missing project maps to geectl's
`NotFound`, and the project list at `GET /source` returns a `<directory>` of
`<entry>` elements. The mutation assertions create a project through geectl and read it
back (`temporal_create_project_shows_up_on_server`), and file real
submit requests and assert on the server state they leave behind
(`tests/temporal_obs.rs`: filing, refile, and accept-time cleanup).

**Plain HTTP on localhost is the single sanctioned exception to the
https-only URL rule.** The OBS development frontend serves plain HTTP;
there is no TLS to terminate against, and a self-signed certificate
would fail geectl's always-on TLS verification anyway. The exception
is safe because the instance is loopback-only (the harness rejects any
non-loopback URL outright), ephemeral (created fresh per run,
destroyed after), and the credentials are ephemeral (the OBS
development default, never a real credential). The read tests run with no
auth at all; the mutation tests authenticate with the bootstrap admin
credentials, sent to the same loopback-only ephemeral instance. The
harness talks to the instance
through `UreqClient` directly, not `UrlBuilder` (which rightly rejects
`http://` bases) — the exception is confined to those two files.

## Distribution

geectl is distributed as an OBS package with tagged releases, fetched via
zypper. Containers fetch the released version — no building random
hashes in containers.
