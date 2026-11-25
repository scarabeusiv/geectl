# geectl CLI output — design

State: the debug channel exists; text rendering exists for every command.

`src/main.rs` is a thin CLI over the library. Global `--format` flag (JSON default, text alternative) plus the debug flags (`--debug`, `--debug-body`). Errors go to stderr in the selected format (JSON payload by default, `error:`/`remediation:` lines in text mode). The debug channel is the `log` crate facade with a bespoke backend in `src/debug.rs`, fed by the global flags and `GEECTL_DEBUG=1`. One command tree covers both backends (git and OBS).

## The decision, up front

Stdout carries the result payload and nothing else. Everything a human reads while debugging or running a command by hand goes to stderr, or to the text rendering of that payload.

**Debug: one channel.** A global `--debug` flag plus a `GEECTL_DEBUG=1` environment variable, both feeding a single channel in the library — the `log` facade with a bespoke backend — writing human-readable lines to stderr. One level. The channel carries what a run did — HTTP attempts, retries and budgets, library decision points. Bodies print only behind `--debug-body`.
**Text: one obligation per command.** Every CLI command returns `(payload, text)` — the JSON payload and a human rendering of the same data — and the type enforces it. Text stays opt-in via `--format text`. A persistent opt-in comes from config / env instead (decision 2).

## 1. The debug channel

### Mechanism: flag + env, flag wins

**Global `--debug` flag** (`global = true`, like `--format`): works before or after the subcommand, applies to every command.
**`GEECTL_DEBUG=1` environment variable**: the same effect, for scripts and CI where editing every invocation is friction. Only the exact value `1` enables it; unset, empty, or anything else disables it. One value, unambiguous.
**Precedence follows the existing config rule** (flag > env > config > default, `docs/architecture.md`): `--debug` always enables, regardless of the env var; unsetting the var is the off switch.
**Flag and env only.** Debug is a per-invocation concern.

### What it prints: the event catalog

One line per event, on stderr, in time order. The catalog is closed:
the listed event kinds print.

**HTTP attempt** (the bulk of the channel): method, URL path, status or error kind, duration, attempt number against the retry budget. Example shape: `http: GET /source/openSUSE:Factory/geectl -> 200 in 123ms (attempt 1/4)` On a retryable failure the line carries the reason and the backoff: `http: GET /source/... -> 500 in 210ms (attempt 2/4, retry in 2s: transient status 500)` (test `debug_on_records_exhaustion_after_retries`). On exhaustion: `http: GET /source/... failed after 4 attempts (last: 500)`. (test `debug_on_records_exhaustion_after_retries`). A body that fails mid-read retries the same way, and emits a second line for the same attempt: the status line when the headers arrive, then the failure — two events for one attempt: `http: GET /source/... -> network error in 210ms (attempt 1/4, retry in 1s: body read failed)` (test `debug_on_records_body_failure_line_for_severed_read`).
**Read-after-write retries** (the library's bounded retries): each probe plus the remaining budget, e.g. `retry: create_package registration probe 1/30s: still not found`. (test `debug_create_package_probe_reports_each_retry`). A retried `change_state` consuming its 60 s budget leaves its trace in this channel.
**Library decision points** (the ones that change what happens next): the supersede-all filing decision (`decision: filing for foo; superseding open submit request(s) 1234567 with the new request`; test `debug_file_reports_the_supersede_decision`), dedup checks (`decision: open maintenance_incident request(s) 1234567 already target openSUSE:Maintenance for foo; refusing to file`; test `debug_maintenance_incident_reports_the_dedup_refusal`), the DryRun guard (`decision: dry-run: mutation refused, plan only: <action>`; test `debug_dry_run_refusal_reports_the_guard`), endpoint selection (`decision: read via /public/source (no credentials)`; test `debug_on_records_attempt_lines_for_500_then_200` vs `decision: read via /source (authenticated)`; test `debug_endpoint_reports_authenticated_reads`), and the accept-state check before a retried `change_state` (`decision: request 1234567 still new, re-issuing accepted`; test `debug_change_state_retry_reports_probe_and_decision`).
**Service status lines**: `service_state <project>/<package>: <code>` and `list_services <project>/<package>: <n> service(s)` — emitted by the services reads, unprefixed (the verb names the operation, as with the startup line; emit sites `src/obs/services.rs:parse_services,service_directory_listing`).
**Startup line**: `geectl <version> request show 1234567` — the invoked command line, so a pasted debug transcript is self-describing. Secret-valued flags are masked here by construction.
**Config line at startup**: the effective OBS/Gitea base *hosts* and the auth *kind* (`basic`/`bearer`/`ssh`/`none`/`error`) only. (`config show` reports the username as data.)

### Channel boundaries

**Bodies** print behind `--debug-body` only, scrubbed for credentials (see Scrub scope).
**Headers**: the sink takes pre-formed lines, so the channel carries pre-formed lines alone.
**Secrets**: credentials travel in headers, so the URL prints as-is. Debug lines are credential-free by construction, and a test asserts every debug line is credential-free.
**Timestamps**: per-attempt durations are the number the channel carries.

### Scrub scope

Bodies behind `--debug-body` are scrubbed for credentials only, by
a simple name-based scan (`redact_body` in `src/debug.rs`): a value
is masked when its field name looks credential-bearing — user,
password, passwd, pwd, token, secret, api key, session id,
`Authorization` header values, access key, private key. That is the whole scrub:
field-name matching, nothing else.

Everything else passes through: a bare array item, a structured object value, a CDATA section, credential text wrapped in a child element, the element text of a tag carrying attributes, an unterminated quote, an empty credential value. (Attribute *values* with credential names are redacted; only the element text of an attribute-carrying tag passes through.)

### Format: human lines

Human-readable lines on stderr, prefixed by component:
`http: ...`, `retry: ...`, `decision: ...`, `config: ...`. Two
kinds are unprefixed: the startup line (the bare command line) and
the service status lines (the verb names the operation). The
consumer of `--debug` is a human (or an agent reading a failure
report) diagnosing a run. The machine-readable verdict is the JSON error payload
plus the exit code. Stdout carries the payload only; debug writes stderr.

### Granularity: one `--debug`

One level that prints everything the channel carries.

### Where it lives: the `log` facade with a bespoke backend

A small `Debug` sink type in the library (`src/debug.rs`): an `enabled: bool` plus pre-formed lines. The library's `UreqClient` and `ObsClient` take it at construction (builder or constructor parameter); the binary constructs it from the flag/env and passes it in. The sink is public in the library so other consumers (freshcrab) can wire it to their own log.
**The `log` crate (decision 4).** The emit path goes through the `log` facade to a minimal bespoke `log::Log` backend installed by `install`, writing the plain human line to stderr. The backend accepts records from this module.
**Near-zero cost when off.** The HTTP transport checks `enabled()` before formatting; the library decision-point sites format one small string each, dropped inside `emit()` when the channel is off. The sink takes pre-formed lines, so the check is one bool per event.
**The two existing prints.** `try_change_state` emits a `decision:` line on the channel when it declines an accept (`post_changestate_with_retry` in `src/obs/request.rs`): the refused-change path propagates the error, so the reason reaches the JSON error payload, while the already-in-target-state path returns `Ok(false)`, and the boolean outcome is in the payload. The re-issue `decision:` lines emit live in `post_changestate_with_retry`. The `[dry-run]` plan echo (`src/obs/types.rs`, `DryRun::note`) is unconditional stderr narration: it is the dry-run feature's output — a dry run shows what would happen — and the plan is data (`DryRun::plan`).

### Composition with `--format`

Orthogonal: `--format` selects the stdout payload, `--debug`
selects stderr diagnostics. `--format text --debug` prints the text
payload on stdout and debug lines on stderr; `--format json
--debug` prints JSON on stdout and the same debug lines on stderr.
Debug output is identical in both modes.

## 2. Human-readable output

### The obligation, enforced by the type

Every CLI command returns `(payload, text)` — the JSON payload and a
human rendering of the same data — as `run()` does for
`version` and `config`. **The pair is the enforcement:** the type system enforces the renderer: every command handler returns `(payload, text)`.

Consequences:

**Text renderers ship with their command.**
**The renderer lives in the binary (`src/main.rs`).** The library returns typed data; rendering is a presentation concern of the CLI. The library stays format-agnostic.
**Tests for both.** A JSON test (existing pattern) plus a text test (render the fixture payload, assert the lines). A third test asserts the two agree on identifying fields: every id, name, and revision the renderer carries appears in the text rendering; fields listed as documented omissions are exempt. (See Test story.)

### Shape rules (the renderer contract)

1. **Text renders the same data as the JSON payload** — the same identifying facts, except for the documented
   omissions below. A summary may omit long free text (a
   description, a comment body); every id, name, revision, and state in the payload appears in the text.
**Coordinate header:** text starts with one header line echoing the query coordinates — the required positionals, in the order project, package, repository, arch (as applicable), plus `package:` when the query is package-scoped. Tables stay as the worked examples pin them; the header is the only addition.
**Documented omissions:** anything else the text leaves out is listed per renderer in the module docs, with the reason. The JSON payload remains the complete record of the typed state, not of the server's raw bytes (terminal control characters are stripped at parse, per the `ServiceState` error field docs).
**Content commands (rule 4) are exempt from the header:** their text *is* the content, verbatim; the coordinates live in the JSON payload. The exemption covers the content itself; a synthetic empty-content line (e.g. `no log`) still carries the header.
2. **Single objects render as `key: value` lines**, one fact per
   line — the `config show` precedent. Nested objects flatten with a
   prefix (`action 1: submit ...`); nested lists get one line per
   item.
3. **Lists render as tables**: a header row and space-aligned columns.
   Column width is the widest value in
   the printed set. Values print whole: a long value makes a wide
   line, and the JSON payload remains the complete record. A table cell may still
   summarize a nested object in compact form (the `request search`
   actions column shows `submit foo`); the full fields stay in the
   JSON payload and in the corresponding `show` command — that is the
   sanctioned exception to rule 1.
4. **Content commands render verbatim.** For `cat` (and any future
   command whose payload *is* a file's content), the text rendering is
   the content itself. JSON carries it as a
   string field; text prints it raw.
5. **Empty results print an explicit line** (`no files`, `no
   requests`, `no results`).
6. **Deterministic order.** The JSON array order and the text row
   order are identical, always.
7. **Values print as given.** Timestamps print as the server gave
   them (ISO). Numbers print as the server gave them.
8. **Plain output.** Non-interactive output
   must stay clean in pipes and logs.
9. **One renderer, escapes by construction** (`docs/architecture.md`):
   text is rendered from parsed data — except content commands (§2 rule 4), where the content is
   the parsed body.

### Worked examples (shape only)

Per-command fields are the command author's decision, following the
shape rules above. Six examples pin the shape:

**`request show <id>`** (single object, `key: value`): `id: 1234567`, `state: new`, `creator: ...`, `created: ...`, then one line per action (`action 1: submit openSUSE:Factory/foo rev 42 -> openSUSE:Factory`), one line per review (`review 1: by group:factory-staging state accepted`).
**`request search` / `search_requests`** (list, table): header `ID STATE CREATOR ACTIONS`, one row per request, the actions column a compact inline form (`submit foo`). `request list` renders the same table under a `list` header; `--since` bounds by history or last-update date, `--limit` caps the rows.
**`list_files`** (list, table): header `NAME SIZE MTIME`, one row per file. The md5 is JSON-only: it is a verification value.
**`cat <package> <file>`** (content): the file verbatim.
**`download <package> <file> --output <path>`** (single object + confirmation): header `project: … package: … file: …`, then `downloaded <file> (<n> bytes) to <path>` — the file content goes to `--output`, and the text rendering carries the confirmation line. (test `remote_download_text_mode_renders_the_confirmation_line`)
**`build results`** (list + summary): a `project:` header line, then the table `REPOSITORY ARCH PACKAGE CODE`, then a summary line (`1 failed, 3 succeeded`, terms alphabetical) — the summary is derived from the same data. With `--all-instances` the view repeats that shape once per configured instance, each section headed `instance: factory (https://api.example.org)` (test `merged_results_text_sections_per_instance`); an instance that fails to answer renders `instance: local (http://127.0.0.1:3001): error: ...` in place of its table, so the failure stays visible (test `merged_results_text_surfaces_a_failed_instance`).

### Opt-in via `--format text` everywhere

**Opt-in via `--format text` everywhere.**
(Decision 1.)

**Determinism.** The same invocation produces the same bytes in every context.
**Automation is the primary consumer.** JSON default everywhere keeps programmatic parsing unconditional.
**Humans opt in once.** `--format text` is explicit and cheap; shell users who want text always alias it (`alias geectl='geectl --format text'`). The docs show the alias.
**The architecture doc states it:** "JSON by default, one uniform `--format` across every command" (`docs/architecture.md`).

### Persistent opt-in: config + env

**`format` is a config-file key and a `GEECTL_FORMAT`
environment variable**, under the existing precedence (flag > env >
config file > built-in default). (Decision 2.)

`config.rs` carries a `format` field, validated (`json`|`text`, anything else is `ConfigError` naming the value), and `config show` reports the effective format and its source, like the endpoint URLs.
A human's config that sets `text` changes an agent's stdout. **Agent invocations pass `--format json` explicitly** (the skill's invocations do), and the flag beats the config. `config show` reports the format source.
`--format text` per invocation is the whole mechanism, and the alias is the persistent form.

### Errors in text mode

**Render the error in the selected format** — text
mode prints `error: <message>` plus `remediation: <...>` lines on
stderr; JSON mode keeps the JSON payload. The error kind and the
exit code are unchanged; only the rendering varies, in one place
(`write_error` takes the format). (Decision 3.)

### Dependency: the OBS command surface

The OBS command surface exists in the binary: `remote`,
`request`, `branch`, `build`, `maintenance`, `service`, `gitea`.
The library operations behind it (`show_request`,
`search_requests`, `cat`, `list_files`, `list_packages`,
`package_log`, `request_diff`, `change_state`, `branch_package`,
`build` reads, … — the command surface exposes a thin subset). The text renderers ride
on that work:

**One subcommand per operation (or per operation group), thin over the library.** Every leaf command returns `(payload, text)`.
**The (payload, text) pair is the command handler's return type.**
**Text reads the library only.** A renderer needing a field outside the library's return is a library gap, filed as such.

## Errors (shared)

The error store (`src/error.rs`) stands. `--debug` adds diagnostics around an error: a failed run with `--debug` ends with the same JSON error payload (or the text rendering, decision 3) and the same exit code, preceded on stderr by the debug lines that explain it.
Debug lines and the error payload interleave on stderr in time order — both are stderr, so ordering is natural.
If stderr is closed, debug writes are ignored like the existing `write_error`. A run's outcome is independent of the debug channel.

## Test story

**PR-level (gating):** fixtures.
Debug off → the sink writes to a `Vec` in tests; assert it stays empty for a scripted run with debug disabled.
Debug on → the event lines for a scripted HTTP sequence (fixture transport): attempt lines with method/URL/status/ duration/attempt, a retry line with the backoff, an exhaustion line. Assert the exact lines for a 500-then-200 sequence and for a 404 (one line).
Redaction: a fixture run with a fake credential asserts every debug line is credential-free.
Flag/env precedence: `--debug` enables with `GEECTL_DEBUG` unset; `GEECTL_DEBUG=1` enables alone; `GEECTL_DEBUG=0` (and empty, and `yes`) disables alone.
Text: every command renders its fixture payload in both formats; the identifying-fields agreement test (every id/name/ revision in the JSON appears in the text); empty-list renders the explicit `no <things>` line; `cat` renders verbatim.
Error rendering (decision 3): text mode renders `error:` + `remediation:` lines; JSON mode keeps the payload; exit code identical in both.
**Live (gating):** the live suite proves the operations against a real server. A red live run re-runs with `--debug` to produce the transcript.

## Risks

**Debug lines pasted with secrets.** The sink takes pre-formed lines, and the redaction test asserts every debug line is credential-free. A URL carrying a credential as a query parameter is a bug: the transport guard forbids credentials over cleartext, and credentials travel in headers. A debug line printing such a URL surfaces the bug.
**Debug noise drowning the signal.** One level prints everything; a chatty command (a search fanning out per-file GETs, the `revision_patch` / `project_diff` fan-outs) produces many lines. One line per event, and the channel is off by default.
**Text renderers drifting from JSON.** The identifying-fields agreement test and the pair return type pin the two together — the renderer sits in the same function as the payload. Free-text fields omitted from text by design (rule 1) are documented per renderer.
**Scope creep into a logging framework.** The channel is one level on stderr, through the `log` facade to the minimal bespoke backend.

## Decisions

### Debug channel

1. `--debug` plus `GEECTL_DEBUG=1`; flag wins. The env var serves scripts and CI; precedence follows
   the flag > env rule.
2. Human lines on stderr. The consumer is a human diagnosing a run; the machine-readable verdict is the JSON error payload plus
   exit code.
3. One level. One `--debug`.
4. The `log` crate. The channel contract holds: stderr, human
   lines, one level, off by default.
5. Bodies behind a separate flag (`--debug-body`), size-capped
   and redacted.

### Human-readable output

1. Opt-in via `--format text` everywhere.
   The same invocation produces the same
   bytes in every context, and the agent is the primary consumer.
2. Persistent opt-in via config file + env (`format` in
   `config.toml`, `GEECTL_FORMAT`), under the existing
   precedence (flag > env > config > default); agent invocations
   pass `--format json` explicitly.
3. Errors render in the selected format. Text mode: `error:` + `remediation:` lines on stderr;
   JSON mode keeps the JSON payload.
