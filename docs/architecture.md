# Architecture

## Crate shape: lib + bin

geectl is one crate with a library and a binary:

`src/lib.rs` — the library. The binary is a thin CLI over it, so tools can link the same logic instead of shelling out to the binary. The library is the core; the binary is one consumer of it.
`src/main.rs` — the binary. Argument parsing (clap), one exit path, stdout carries only the result payload.

The library modules:

| Module | Contents |
| ------ | -------- |
| `error` | The centralized error store (see below). |
| `url` | The single URL builder (one encoder per context). |
| `http` | The HTTP client trait with a bounded timeout, plus a ureq implementation. |
| `auth` | The credential types and resolution (see below). |
| `config` | The XDG config file (endpoint URLs, output-format default, named instances, local-build tunables) and the secure-write primitive. |
| `gitea` | The Gitea operations (repo/branch reads and mutations, fork, repo create/delete). |
| `obs` | The OBS operations (devel lookup, open-SR check, SR filing, superseding, branch cleanup, request read/act — see below). |
| `debug` | The debug channel (one coherent `--debug` stream on stderr). |
| `xml` | Minimal XML document editing for the small documents rewritten in place (`_meta`, `_link`). |


## The error store (`src/error.rs`)

Every error geectl can produce is defined in one place: `ErrorKind`, with
a stable code, a human message, a remediation, and an exit code per kind.
`GeectlError` is a kind plus the context of the instance (what resource,
what request) — the kind is the match key.

Rules:

There is exactly one exit path: the binary maps the error kind to its exit code.
The error payload is stable JSON: `{"error": {"code": ..., "message": ..., "remediation": ..., "context": ...}}`. `context` is omitted when empty.
The error path has its own test: every kind renders to JSON and to Display, panic-free.
The server's error body is parsed with the same hardened XML parser as everything else; if it does not parse, the raw status plus a truncated body excerpt are reported as data.

## The URL builder (`src/url.rs`)

There is exactly one place that builds URLs, and one encoder per context:
Path segments are encoded with an empty safe set except `:` (the OBS `project:package` separator). `/` is always encoded, and segments pass through exactly — a segment containing `/` becomes one encoded element (`%2F`).
Query values use the query encoder, where `+` is encoded (`%2B`) because a query string decodes `+` as a space.
Query pairs go through the encoder.

Encoding is a property of the builder.

## The HTTP client (`src/http.rs`)

Every request carries a bounded connect timeout (default 10s) and a bounded read timeout (default 30s), unconditional on library version. The timeout values are documented defaults, overridable by explicit config.
Retry policy, as a single documented constant: only idempotent methods are retried (GET, HEAD, PUT), up to 3 times with doubling backoff. POST retries only with an explicit idempotency key. DELETE runs as a single attempt. `post_json` (JSON body, `Content-Type: application/json`) returns every non-2xx status immediately on first attempt.
Non-2xx statuses are converted to a `GeectlError` at this layer, before any caller sees the body. 404 maps to `NotFound`, 401/403 to `AuthFailed`, other statuses to `ServerError`.
Auth refresh happens before the body is consumed, or the body is buffered and replayable by construction. A retry after 401 proceeds only when the body is byte-identical.

## Auth (`src/auth.rs`, `src/http.rs`)

OBS credentials — an API token (Bearer) or a username/password
pair (Basic) — come from `EnvObsAuth` (per-URL, via
`credentials_for_url`): the matching
instance alias's variables (`GEECTL_OBS_APIKEY[_FILE]_<ALIAS>` or
`GEECTL_OBS_USERNAME_<ALIAS>` and friends), else the plain
`GEECTL_OBS_APIKEY` or `GEECTL_OBS_APIKEY_FILE`, or
`GEECTL_OBS_USERNAME` + `GEECTL_OBS_PASSWORD` or
`GEECTL_OBS_PASSWORD_FILE` when the URL is the configured default.
OBS login takes a username and password; a token
login fails with a 401 (upstream
https://github.com/openSUSE/open-build-service/issues/20322). Gitea Bearer token credentials come from
`EnvGiteaAuth` the same way (per-URL, via `credentials_for_url`):
the matching alias's variables (`GEECTL_GITEA_TOKEN_<ALIAS>`),
else the plain `GEECTL_GITEA_TOKEN` or
`GEECTL_GITEA_TOKEN_FILE` when the URL is the configured default.
When an alias matches but its variables are unset and the URL is
also the default, the plain variables apply. The default's
credentials apply only to the default instance. Credentials are otherwise
resolved by providers implementing the `AuthProvider` trait
(`src/auth.rs`): `EnvGiteaAuth` (the plain single-URL path) and
`GiteaSshKey` (the SSH private key from `GEECTL_GITEA_SSH_KEY`).
Unconfigured resolves to `Credentials::None`; a half-configured
pair, both credential types set for one instance, both secret
sources set, or a malformed value is a `ConfigError` naming the
source. The SSH key must be a PEM private key in a
file mode 0600 or stricter (unix).

The HTTP client (`src/http.rs`) applies the credentials per request:
Basic as an `Authorization: Basic` header, Bearer as
`Authorization: Bearer`. HTTP requests carry Basic or Bearer
credentials only. Credentials travel over `https://`, or to a
loopback host (for tests with fixture credentials).

Rules that hold:

Credentials come from environment variables or mounted secret files.
Credentials live outside config files. A credential-shaped value in a config file is a startup error.
Logs exclude secret values. Any file that can contain a credential is created with mode 0600 from the first byte.
TLS verification is always on.

## OBS operations (`src/obs.rs`)

The library surface a tool uses to move a package update through the
distribution: `ObsClient` (devel-project lookup, devel version, open
submit-request check, filing, superseding, branch cleanup, branching, copying, linking, repairing
links, link revisions, converting links to branches and detaching them, deleting and restoring
packages and projects, aggregating binaries, releasing, branch-project and branch-package creation,
package creation plain or git-backed, file upload, build status, maintenance incident / change-devel
/ delete filing, maintained-instance listing, maintenance branching, the remote reads listed below,
and reading and acting on
requests), `SrPayload`
(the validated submit-request payload), `SrFiling` (the only filing
path, dry-run by default; `refile` is the file-then-supersede flow),
`DryRun` (the would-do guard), and `CveId` (CVE id parsing plus the
changelog-names-CVE rule). `MaintenanceIncidentPayload`,
`ChangeDevelPayload`, and `DeletePayload` are the validated
maintenance-request payloads; `SrFiling` files them the same way it
files a submit request.
Request reading and acting is the typed `Request` model
(`RequestAction`, `RequestState`, `ReviewState`, `RequestReview`,
`RequestHistory`, `RequestComment`) with its parser and serializer,
`RequestSearch` (the search builder over the server's `match=`
expression), and `ReviewTarget` (who a review is asked of).

Behaviour rules (from the openSUSE packaging guide):

A submit request targets the DEVEL project, or Factory from the recorded devel project and package: a Factory target is verified live at filing against the target package meta.
Regular flow: Home -> devel -> Factory -> updates. Devel projects have maintainers and owners, and they are allowed to skip the home step: Devel -> Factory -> updates. That is the legitimate path — the devel project's maintainers and owners work in devel directly and submit straight to Factory. The develprj check exists because only the devel project may submit to Factory.
Filing supersedes every open submit request for the package from the authenticated user. `SrFiling::file` files the new request first, then supersedes each open one with it; the supersede is unconditional — a second filing at the same head files and supersedes again. The open-request check is also available on its own as `ObsClient::has_open_sr`.
The SR description follows the guide's message format: one headline of at most 160 characters of prose, then `- ` lines for staging, needs, and rebuilds. The library validates the description is non-empty; the headline shape is the caller's responsibility.
A security fix's `.changes` entry must name every CVE it fixes. When the caller supplies CVE ids, `SrPayload::new` requires a `.changes` entry naming each of them.
A lookup that errors (transport, auth, unexpected response) returns an error. Only a successful response that carries no result reads as "none" / "false".

The full lifecycle is file -> supersede -> delete branch, and the
branch dies by default: every payload carries
`opt_sourceupdate="cleanup"`, so the new branch is deleted when its
request is accepted. `SrFiling::refile` files the new request first
and only then supersedes the old one with it, so `superseded_by`
always names a request that exists. Branch deletion
(`ObsClient::delete_package`) is a separate explicit call for the
old branch — accept skips a superseded request, so
server-side cleanup skips it — and the delete runs only when no
request in `new`, `review`, or `declined` state involves the
package as a submit source or target.

The supersede and delete semantics follow the OBS API server code.
Where osc and the API disagree, the API wins:

`superseded_by` is required by the server; the library always sends it.
Superseding applies to `new`, `review`, and `declined` states only. The library leaves the old request's state to the server; its 403 is surfaced as-is.
The file-first ordering in `refile` makes the link real: the server takes `superseded_by` at face value.
A package delete on the API revokes open requests as source and declines them as target. The library blocks the delete instead, naming the blocking requests. `force` overrides the guard, forwarded as `force=1`, which also overrides the server's weak-dependency guard (refuse when other packages use this one as their devel package) — osc's `force` semantics.
If superseding fails after the new request is filed, `refile` returns an error naming the new id.

Branch cleanup is the default: every payload carries
`opt_sourceupdate="cleanup"`, so OBS
deletes the source branch when the request is accepted — and the
source project with it, when that was its last package and it is a
`home:<user>` subproject. The packaging guide requires it
for filings from a `home:` subproject; pr-guard requires it. The accepted values are `update`, `noupdate`, `cleanup`
(`BsRequestAction`); `BsRequestActionSubmit` deletes the source
package on accept when the value is `cleanup`. The
server also defaults to cleanup when no value is given and
the source is the filer's branch project for the target, but the
library sends the value explicitly: the server default only covers
branch projects, and the explicit value is what pr-guard checks.
A forward from a devel project opts out with
`SrPayload::without_cleanup`, which sends `update` instead: accept
with cleanup deletes the devel package.
`ObsClient::delete_package` remains the manual cleanup for the
branches server-side cleanup leaves: a superseded request's branch
and a declined request's branch.

Branch setup is creation plus upload: a fresh branch project
(`ObsClient::create_project`, `PUT /source/<project>/_meta`) and the
branch package in it (`ObsClient::create_package`, `PUT
/source/<project>/<package>/_meta`), then the files the update
carries (`ObsClient::upload_file`, `PUT
/source/<project>/<package>/<filename>` as `application/octet-stream`).
The meta PUTs return `<status code="ok"/>`; the file PUT returns the
backend's `<revision rev="N" .../>` document, and `upload_file`
returns that revision number (or `None` in dry-run). The filename rule
mirrors the server (`PackageService::FileVerifier`: `/` and a leading
`.` are excluded; at least two characters; `_meta` and `_attribute`
are reserved).
All three are DryRun-guarded like the delete: in dry-run the
creation or upload is recorded in the plan only, and is
credential-free.

Branching, copying, and linking are the agent's package-duplication
operations, and they are one operation each whether the package is
OBS-native or git-backed: a link points at the package whatever its
backend, and a git-backed package is created the same way as a plain
one, by naming its repository and branch in `scmsync`
(`ObsClient::create_package_with_options`). `ObsClient::branch_package` branches a
package (`POST /source/<project>/<package>?cmd=branch`, with the
nodevelproject and new-package behaviours); `ObsClient::copy_package`
and `ObsClient::copy_project` copy a package or a whole project
(`?cmd=copy`); `ObsClient::link_package` links a package by composing
its meta from the source meta and PUTting a `_link`;
`ObsClient::repair_link` repairs a broken link the same way, by
editing the `_link` itself — the server repairs links only in its
commit path, which the working-copy commit takes;
`ObsClient::set_link_rev` pins or unpins a link revision, for one
package or a whole project; `ObsClient::link_to_branch` converts a
link into a branch and `ObsClient::detach_branch` detaches one;
`ObsClient::delete_project`, `ObsClient::undelete_package`, and
`ObsClient::undelete_project` delete and restore;
`ObsClient::aggregate_package` collects another package's binaries
under a new name; and `ObsClient::release_package` /
`ObsClient::release_project` release sources and binaries to a
release target as built; `ObsClient::lock` locks a project with
the server's `cmd=lock`, which records the message, or a package
by setting its `lock` flag — the message is sent but not recorded;
`ObsClient::unlock` unlocks one with a required comment. Every one of these is
DryRun-guarded and credentialed like the create and upload calls
above.

The server is ground truth for these operations, and the library is
written against its behaviour. Two principles run through the
implementations: a mistaken assumption fails before the first write
(a non-link package handed to detach, a package that already exists
handed to a plain create, a pinned revision for a whole
project), leaving state untouched (an existing `_aggregate`, an
existing link). The repository mapping of an aggregate is validated
by the server at build time: a bad map builds as broken. Consumers see typed values only. The XML document editor
(`src/xml.rs`) and the response parsers are `pub(crate)` — the
boundary the typed surface is built on.

Remote reads: the read surface for inspecting what the distribution
holds, all typed. The public reads (see Credentials below) run
credential-free. `cat` reads one file at a revision (`rev`);
`download_source_to_file` streams one file to disk at a revision (a number or a srcmd5),
bypassing the transport's 10 MiB body cap and UTF-8 check, so a source tarball
downloads; the
directory listings (`list_files`, `list_packages`) are the server's
own `<directory>` documents. `package_log` reads the package history
(newest first — the server lists oldest first, so the log is
reversed) through `/source`, which has a credentialed route only, so
it reads with the client's credentials. `revision_patch` diffs one
revision against the previous one (revision 1 has nothing to diff
against) with per-file unified diffs. `search_packages` and
`search_projects` match a substring against name, title, and
description — the search is a substring match — and the term is
non-empty and `'`-free.
`owners`, `owners_for_binary`, and `owners_for_package` read the owner
search, `maintainers` filters it to role `maintainer`, and `whois`
reads a person. The credentialed reads require credentials, and an
unexpected response is an error. A 400 or 404 on the package-only
search returns no owners: no owner root project holds the package.
The package search leaves `limit` unset, so it returns every hit the
server reports. A 403 surfaces as `AuthFailed`, including on public
reads: a search-cap 403 on a public read is an auth failure. `project_diff`
classifies every package in either project — identical, differs,
only-in-old, only-in-new — with a per-file unified diff for the
packages that differ, optionally annotated with the open submit
requests involving each differing package in the new project (which
needs credentials).

Reading and acting on a request is the same shape as filing it: the
server is the authority, and the library leaves server decisions to
the server. `show_request` reads one request with its history,
`request_log` reads only the history, `search_requests` runs a
`RequestSearch` against `/search/request` (ids, projects, packages,
source/target, creators, who, states, action types, and the history
flags — the match expression is built in the library's fixed clause order),
and `request_diff` returns the submit diff, plain or wrapped in the
request XML. `change_state` moves a request (`accepted`, `declined`,
`revoked`, `review`, `new`); `accept_request`, `decline_request`,
and `revoke_request` are the named wrappers, and
`try_change_state` is the lenient form: a refused move propagates the error so the decline reason reaches the payload, while a request already in the target state reports `false` (a no-op). `add_review` asks a user, group, project, or
project/package to review (`ReviewTarget` validates the target up
front: at least one of user, group, project, and a package needs its
project); `list_comments` and `add_comment` read and write the
request's comments, with replies threaded under their parent. State
changes, reviews, and comment creation take the `DryRun` guard, like
deleting; the reads run unguarded. Accepting is a maintainer action,
and the boundary is the packaging guide's: an agent accepts its own
requests with explicit authorisation. The server enforces who may
move a request; the library surfaces its refusal as-is. The operations
are backend-agnostic server calls: a request is a server object
whether the package is OBS-native or git-backed.

Maintenance requests cover the other request types an agent files:
maintenance incidents, change-devel, and deletes. They use the same
shape as submit requests — a typed payload, validated at construction
and again on the filing path, filed through `SrFiling` with the DryRun
guard first and a dedup check over open requests — and the same
file-first-then-supersede refile flow where one exists.

A maintenance incident (`MaintenanceIncidentPayload`, `SrFiling::file_maintenance_incident`) carries one action per source package, from one branch project to a maintenance project. Sourceupdate is absent by default — the caller opts in to branch deletion on accept with `with_cleanup` — and the filing pins `source_rev` (`addrevision=1`), so the incident freezes the exact source it was filed against. The server accepts only maintenance or maintenance-incident targets, and the release target must resolve to a maintenance-release project, either through `target_releaseproject` or through the source package's link target — resolved at filing time, to its update instance in both cases (patchinfo packages skip that check). `refile_maintenance_incident` is the same file-first flow as `refile`, exempting the old incident from the dedup.
A change of devel (`ChangeDevelPayload`, `SrFiling::file_change_devel`) moves a package's devel project: the source names the NEW devel package, the target names the package whose devel changes, and on accept the server sets the target package's devel to the source package. All four names are required; the action's fields are the four names.
A delete (`DeletePayload`, `SrFiling::file_delete_request`) deletes a package, a repository, or a whole project. The target is exactly one of the three, and the dedup matches the same target shape.
The dedup search matches on source/target, state (`new` or `review`), and action type (`action/@type`, filtered server-side); the response parser re-checks the type, and narrows further client-side where the search falls short: a delete dedup matches the target shape (a whole-project delete blocks any delete in the project, a package delete only the same package, a repository delete the same repository or a whole-project filing), and an incident dedup matches the release project.
Branching a maintained package (`ObsClient::maintained_instances`, `ObsClient::maintenance_branch`) goes through the branch command (`POST /source?cmd=branch`): the listing is the dry-run report — which projects carrying the attribute hold the package, with the devel package when there is one and where the branch would land (the parser translates `:` to `_` in the reported target package, matching the live branch name) — and the live branch creates the branch project (by default `home:<user>:branches:OBS_Maintained:<package>` — the attribute, with `:` → `_`) and returns where it landed. The attribute defaults to `OBS:Maintained`, the update project attribute to `OBS:UpdateProject`. The live branch is DryRun-guarded like the other mutations. A package whose sources live in scmsync fails with the server's error ("Fork it instead"). Branching and forking stay separate operations.
Refile exists for maintenance incidents. A change-devel or delete request is filed fresh; superseding one is a manual step (`ObsClient::supersede_request`).

The maintenance semantics follow the OBS API server code
(`BsRequestActionMaintenanceIncident`,
`BsRequestActionChangeDevel`, `BsRequestActionDelete`,
`BranchPackage` on open-build-service master).

Build status is the read side of a package's build: what state it
is in, why it is there, and what it produced. The methods are the
same whether the package is OBS-native or git-backed — the build
service serves both through the same `/build` API, one operation for
both. An agent asks them when it needs to know
whether a package built, what blocked it, and what came out:

`results` reads the build matrix for a project (one cell per repository/architecture, with the per-package codes inside); `wait_for_results` polls it until it settles — every cell clean, every cell and package out of waiting — with plain two-second polls and a timeout at the deadline (the caller then re-reads with `results`). The call omits `oldstate`: the server long-polls on an unchanged state, up to five minutes, outrunning the transport's per-read timeout. A multibuild flavor travels as `pkg:flavor` with `multibuild=1` and `locallink=1`; older servers reject `multibuild`, so the read fails there.
`build_log` reads the raw log text (`nostream=1`); `build_status` reads the package's build code (`succeeded`, `failed`, `unresolvable`, ...); `job_status` reads the running job, if any.
`buildinfo` reads the dependencies a build pulls in — and the error blocking a build at startup. An unresolvable build surfaces its missing dependencies as the error, as a comma-separated list (`unresolvable: dep, dep, ...`).
`build_history` lists the completed builds, newest first; `job_history` lists the build jobs that ran in a repository. Match a job-history entry's `srcmd5` against the package's current revision to check a green build is from the current source.
`list_binaries` lists the built files (binary RPMs plus the build artifacts: `_buildenv`, `_statistics`, `rpmlint.log`, the `.src.rpm`, `-debuginfo` and `-debugsource` when built); `download_binary` downloads one by name as raw bytes, capped at 10 MiB by the transport's body limit; `fileinfo` reads its RPM header fields (name, epoch, version, release, arch, source, summary, description, disturl, size, mtime) and dependency lists (plain view: provides, requires, prerequires, conflicts, obsoletes, recommends, supplements, suggests, enhances), with the `fileinfo_ext` view naming the providing/requiring packages per dependency.
`dependson` lists which packages each package depends on for its build; `whatdependson` is the reverse (a rebuild of the package triggers a rebuild of each of them): each entry's `dependencies` are the packages depending on the queried package.
`check_constraints` lists the worker ids satisfying a `_constraints` document. It is a read-only query, exempt from the dry-run guard. None posts an empty body, which the server accepts. The constraints document is passed through as authored: this is the one place a raw XML document crosses the public API.
`list_repositories` reads the project's repositories (names, architectures, the repositories they build against) from the project meta. `add_repository` adds one (name, architectures, and the repositories it builds against, with an optional publish-disable flag); a duplicate name fails; `remove_repository` removes one, tolerating a missing name (it writes only when a stale publish-disable entry for it remains). Both edit the project meta in place and PUT it back, so unknown elements survive the edit; both take the dry-run guard.

All of the build reads are typed, except `check_constraints`, whose
constraints document is passed through as authored, and the parsers
are `pub(crate)`. The trait has `get_bytes` with a default
implementation that returns an error (existing implementors keep
compiling) and `UreqClient` overrides it,
sharing the GET retry loop with `get_with`. The download is capped at 10
MiB by the transport's body limit like every read.

The 404 conventions follow the devel-project precedent: a missing
log reads as `None` (the package is unbuilt, or does not
exist — the server answers 404 for both, so the caller checks
the package exists when the distinction matters); a missing status
reads as `None` too, but through `code="unknown"` with a 200 — the
server only 404s when the package/project does not exist. A
missing binary list reads as an empty list (an unbuilt package
answers 200 with an empty list; only a nonexistent package or
project 404s). A missing job reads as an empty jobstatus element,
so `job_status` returns `None` for it — while a 404 there is an
error (unknown project/package), as it is for the histories, the
buildinfo, the dependency info, a download, and a fileinfo. A
malformed document is a server error.

Credentials: the public reads (devel project, devel version, file
and directory reads (except the deleted-package listing), meta, link target, project diff (unannotated), package diff, revision patch, repository list)
run credential-free; the package log, the top-level project listing, the
maintained-instances listing, the open-SR search, the package and project
search, the owner search, and reading a person, plus the filing POST,
superseding, deleting, creating, and uploading, repository add and
remove, and every `/build` read,
and every request read and act (show, log, search, state change,
review, comment, diff) need OBS credentials; missing credentials fail the call —
except the package log, the top-level project listing, the
deleted-package listing (`list_files` with `include_deleted`), and
the `_project` service reads, which read with whatever credentials the
client carries and surface the server's 401/403. OBS accepts Basic
credentials or a Bearer token. Client construction accepts Basic or
Bearer only. Requests carry credentials over cleartext http only to a
loopback host (the transport guard in `http.rs`).

Dry-run is the default (`DryRun::default()`): every
mutating operation goes through the single guard (`DryRun::allow`)
before any write or API call. Reads run ungated. The library builds
and files the submit-request payload; the caller owns the package
work: branching, bumping, cleaning, building, and gating.

The OBS API base is `https://api.opensuse.org` by default. A custom
base is accepted for a test instance (`ObsClient::with_base`): it must
be `https://` with a host, or `http://` on a loopback host. URLs are
built with the encoders from `url.rs` (one encoder per context); the
base itself is carried as a string: the URL builder rejects
`http://` bases, and the test instance is plain http on loopback.

## Output contract

Stdout is only the result payload (JSON by default, one uniform `--format` across every command). Every diagnostic, progress line, warning, and error goes to stderr. Verbosity leaves stdout unchanged.
The debug channel (`--debug`, or `GEECTL_DEBUG=1`) is the one exception to silence on stderr: human-readable lines prefixed by component (`http:`, `retry:`, `decision:`, `config:`), one level, off by default, identical in both output formats. `--debug-body` also prints request and response bodies, capped at 2048 bytes, redacted, headers excluded; binary bodies are excluded. The channel excludes secret values, and a failure in the channel leaves the run unaffected. The binary wires it through the Debug sink.
JSON output treats server text as data. Human-readable output is rendered from parsed data through one renderer that escapes by construction. Server text is printed only through the renderer.
All input arrives via flags. Missing required input fails fast with a structured error naming the missing flag.

## Configuration

One config file: `$XDG_CONFIG_HOME/geectl/config.toml` (default `~/.config/geectl/config.toml`). Explicit precedence, documented in one place: flag > environment variable > config file > built-in default. The built-in defaults are `https://api.opensuse.org` (OBS) and `https://src.opensuse.org` (Gitea).
A `config show` command prints the effective configuration with the source of each value. Two config files found is an error.
A `config init` command writes the template (endpoints only) and fails when the file exists.

## Testing

Three layers:

1. **Fixtures per-PR (gating).** Fast, deterministic, recorded responses.
   Every PR runs them; they gate the merge.
2. **Live Gitea tests per-PR (gating).** A disposable Gitea container
   is provisioned in the `test` job, and `tests/gitea.rs` authenticates
   against it with geectl's own `EnvGiteaAuth` and does real API calls
   (read the user, create a repository, read it back, list it),
   asserting on server state. They gate because they are fast (a single prebuilt image).
3. **Live tests against a live OBS instance (separate CI job).**
   A temporary OBS instance is spun up and the CLI's changes are
   asserted on **server state**. This runs on every pull request, on every push to main, and on
   a schedule. It is a required status check. It exists to catch drift
   as the server changes underneath us. The OBS
   instance is slow (source checkout + frontend build, minutes); the
   frontend build reuses a BuildKit cache in CI.

### The live layer in detail

**Provisioning (`tests/live/`).** The instance is the OBS project's
own development container setup, trimmed to what the live tests need:
MariaDB, memcached, the backend, and the frontend (the API). The build
worker is omitted from the container set. The images are the published OBS master images at
`registry.opensuse.org/obs/server/unstable/containers/containers/openbuildservice/<name>`;
the frontend is built from the OBS source (`src/api`); the published
images cover the rest. `provision.sh` clones the
OBS source (depth 1, plus its `src/backend/build` submodule), builds
the frontend, runs `rake dev:bootstrap` (which creates the database
and seeds the Admin user), polls the public `/about` endpoint until
the API answers, and probes the backend source server until it
answers. `teardown.sh` removes the containers and volumes. The
instance tracks OBS master.

**Schedule.** The `live` workflow runs on every pull request, on every push to main,
weekly (Mondays 06:00 UTC) and on demand
(`workflow_dispatch`), on a GitHub-hosted runner. It is a required
status check. It is slow (source checkout, frontend build,
bootstrap); the timeout is 60 minutes. The frontend image build reuses
BuildKit's GitHub Actions cache (`tests/live/docker-compose.cache.yml`,
loaded only when `LIVE_CACHE` is set), so repeat runs skip the gem
install for an unchanged OBS source; the cache is
content-addressed, so a changed OBS master still rebuilds the
affected layers.

**Harness (`tests/live.rs`, `tests/live_obs.rs`).** The tests are env-gated on
`GEECTL_LIVE_URL`: the gate skips the tests when it is unset, so
`cargo test` stays green with no instance. With it, they assert on live server state
through geectl's own HTTP client: `GET /about` returns a real OBS
(title + non-empty revision), a missing project maps to geectl's
`NotFound`, and the project list at `GET /source` returns a `<directory>` of
`<entry>` elements. The mutation assertions create a project through geectl and read it
back (`live_create_project_shows_up_on_server`), upload a file
and read it back (`live_upload_file_reads_back_on_server`, in
`tests/live_obs.rs`: the bytes and the revision number), read the
remote state back through the read operations
(`tests/live_obs.rs`: cat at a revision, download at a revision,
directory and project
listings, log newest-first with a range, revision patch, meta, search,
owners, whois, link target, project diff), and file
real submit requests and assert on the server state they leave behind
(`tests/live_obs.rs`: filing, refile, and accept-time cleanup,
plus the request read/act tests — show, log, state changes, reviews,
comments, diff, and search — which read and change live requests
through the library and assert on the server state they find, and the
maintenance tests — incident filing and dedup, incident refile,
incident accept creating the incident project, change-devel accept
re-pointing the devel, delete accept deleting the package, and the
maintained listing plus live branch).

The branch, copy, link, and release operations are covered the same
way: live tests that create the server state through geectl and read
it back (`tests/live_obs.rs`) — a branch that shows up as a
linked package, a copy whose file reads back, a link whose `_link`
and meta read back, a repair that unbreaks the link, a revision
that lands on the link, a conversion and a detach that leave the
expected sources, a delete and a restore verified by re-read, an
aggregate whose `_aggregate` reads back, a release the server
accepts, a git-backed package whose `<scmsync>` reads back. Every
test uses names unique to itself (a process id plus a nanosecond
timestamp): the tests run in parallel against one shared instance.

**Plain HTTP on localhost is the single exception to the
https-only URL rule.** The OBS development frontend serves plain HTTP,
TLS-free, and a self-signed certificate fails geectl's always-on TLS
verification. The instance is
loopback-only (the harness rejects any non-loopback URL outright),
ephemeral (created fresh per run, destroyed after), and the
credentials are ephemeral (the OBS development default). The read
tests run unauthenticated; the mutation tests authenticate with the
bootstrap admin credentials, sent to the same loopback-only ephemeral
instance. The harness talks to the instance
through `UreqClient` directly, bypassing `UrlBuilder` (which rejects
`http://` bases) — the exception is confined to those two files.

## Distribution

geectl is distributed as an OBS package with tagged releases, fetched via
zypper. Containers fetch the released version.
