# geectl CLI output — design

Issues: https://github.com/scarabeusiv/geectl/issues/51 (debug channel),
https://github.com/scarabeusiv/geectl/issues/52 (human-readable output)
Status: debug channel implemented (#51 closed); text rendering implemented (PR #83); #52 open.
Depends on: the OBS command surface in the CLI (see §2); #23 (git vs
OBS abstraction — the command tree must not split by backend).

Verified against main `f0c6e3a`: `src/main.rs` is a thin CLI
over the library; global `--format` flag (json default, text alternative)
plus the debug flags (`--debug`, `--debug-body`); text renderers exist
for every command; the debug channel exists (global flags,
`GEECTL_DEBUG=1`, `log` crate facade with a bespoke backend in
`src/debug.rs`); errors go to stderr in the selected format (JSON
payload by default, `error:`/`remediation:` lines in text mode); the
library prints only two `eprintln!` calls, both dry-run warnings (the
`[dry-run]` plan echo in `src/obs/types.rs`, the credential warning in
`src/cli_request.rs`).

## The decision, up front

Two small, separate additions that share one principle: **stdout is the
result payload and nothing else; everything a human needs to read while
debugging or running a command by hand goes to stderr, or to the text
rendering of that payload.**

- **Debug (#51): one channel.** A global `--debug` flag plus a
  `GEECTL_DEBUG=1` environment variable, both feeding a single channel
  in the library — the `log` facade with a bespoke backend — that writes
  human-readable lines to stderr. One level, no `-v` ladder, no `RUST_LOG`.
  It prints what a run did — HTTP attempts, retries and budgets, library
  decision points — never a secret, never a header; bodies only behind
  `--debug-body`.
- **Text (#52): one obligation per command.** Every CLI command returns
  `(payload, text)` — the JSON payload and a human rendering of the same
  data — and the type enforces it: a command without a renderer does
  not compile. Text stays opt-in via `--format text`; there is no
  terminal sniffing. A persistent opt-in comes from config / env
  instead (decision 2).

## 1. The debug channel (#51)

### Mechanism: flag + env, flag wins

- **Global `--debug` flag** (`global = true`, like `--format`): works
  before or after the subcommand, applies to every command.
- **`GEECTL_DEBUG=1` environment variable**: the same effect, for
  scripts and CI where editing every invocation is friction. Only the
  exact value `1` enables it; unset, empty, or anything else disables
  it. No `true`/`yes` parsing — one value, unambiguous.
- **Precedence follows the existing config rule** (flag > env > config >
  default, `docs/architecture.md`): `--debug` always enables,
  regardless of the env var; there is no way to turn debug off from the
  command line once the env var sets it (there is no `--no-debug` —
  unsetting the var is the off switch).
- **No config-file knob.** Debug is a per-invocation concern, not a
  standing preference; a config file that silently turned it on would
  surprise every agent run on that machine. Flag and env only.

### What it prints: the event catalog

One line per event, on stderr, in time order. The catalog is closed:
new event kinds are a design decision, not a quiet addition — that is
what "one coherent channel" means.

- **HTTP attempt** (the bulk of the channel): method, URL path,
  status or error kind, duration, attempt number against the retry
  budget. Example shape:
  `http: GET /source/openSUSE:Factory/geectl -> 200 in 123ms (attempt 1/4)`
  On a retryable failure the line carries the reason and the backoff:
  `http: GET /source/... -> 500 in 210ms (attempt 2/4, retry in 2s: transient status 500)`
  (test `src/http.rs:4390`).
  On exhaustion: `http: GET /source/... failed after 4 attempts (last: 500)`.
  (test `src/http.rs:4395`).
  A body that fails mid-read retries the same way, and emits a second
  line for the same attempt: the status line when the headers arrive,
  then the failure — two events, not two attempts:
  `http: GET /source/... -> network error in 210ms (attempt 1/4, retry in 1s: body read failed)`
  (test `src/http.rs:4423`).
- **Read-after-write retries** (the library's bounded retries, not the
  HTTP policy): each probe plus the remaining budget, e.g.
  `retry: create_package registration probe 1/30s: still not found`.
  (test `src/obs/remote.rs:9617`).
  These are retries with no output elsewhere — a retried
  `change_state` consuming its 60 s budget would otherwise leave no
  visible trace.
- **Library decision points** (the ones that change what happens
  next): the supersede-all filing decision (`decision: filing for foo; superseding open submit request(s) 1398492 with the new request`; test `src/obs/payload.rs:3810`), dedup checks (`decision: open maintenance_incident request(s) 1399001 already target openSUSE:Maintenance for foo; refusing to file`; test `src/obs/payload.rs:3897`), the DryRun guard (`decision: dry-run: mutation refused, plan only: <action>`; test `src/obs/types.rs:1257`),
  endpoint selection (`decision: read via /public/source (no credentials)`; test `src/http.rs:4336` vs
  `decision: read via /source (authenticated)`; test `src/http.rs:4448`), and the accept-state check before a retried `change_state` (`decision: request 1398977 still new, re-issuing
  accepted`; test `src/obs/request.rs:6700`).
- **Service status lines**: `service_state <project>/<package>: <code>` and
  `list_services <project>/<package>: <n> service(s)` — emitted by the services
  reads, unprefixed (the verb names the operation, as with the startup line;
  emit sites `src/obs/services.rs:277,281,307,314`).
- **Startup line**: `geectl <version> request show 1398977` — the invoked
  command line, so a pasted debug transcript is self-describing.
  There are no secret-valued flags, so nothing to redact; any
  secret-valued flag must be masked here by construction, never
  printed and filtered later.
- **Config line at startup**: the effective OBS/Gitea base *hosts*
  and the auth *kind* (`basic`/`bearer`/`ssh`/`none`/`error`) — never a
  username, never a value. (`config show` already reports the
  username as data; debug is stricter than `config show`, because
  debug lines get pasted into bug reports.)

### What it never prints

- **No request or response bodies, no headers.** Bodies can be large
  (up to the 10 MiB cap) and headers carry the credentials. The
  method + URL + status + duration + attempt is the diagnostic.
  Printing bodies is a separate decision with a size cap and
  redaction — not a quiet addition.
- **No secrets, ever.** Credentials travel in headers, never in URLs,
  so the URL is safe to print as-is — but the rule is stated at the
  sink: the sink takes pre-formed lines, never a header map or a
  credential value, so a secret cannot reach it by accident. A test
  asserts a fake credential never appears in any debug line (see
  Test story).
- **No timestamps.** Per-attempt durations are the useful number; wall
  clock adds noise and makes transcripts harder to diff. Correlation
  with server logs is the server's job, not the client's.

### Scrub scope

Bodies behind `--debug-body` are scrubbed for credentials only, by
a simple name-based scan (`redact_body` in `src/debug.rs`): a value
is masked when its field name looks credential-bearing — user,
password, passwd, pwd, token, secret, api key, session id,
`Authorization` header values, access key, private key. That is the whole scrub:
field-name matching, nothing else.

Everything else passes through unscrubbed. OBS exposes the rest of
a body by default, and safeguarding non-credential content is the
server's job, not geectl's. A general arbitrary-content redactor —
CDATA handling, trailing-range rescans, nested-element context
propagation, malformed-input corner hunting — would be brittle by
construction: one tweak of an OBS response shape and it is wrong.
What the simple scan cannot see (a bare array item, a structured
object value, a CDATA section, credential text wrapped in a child
element, the element text of a tag carrying attributes, an
unterminated quote, an empty credential value) it leaves alone — that is
the design, not a gap to chase. (Attribute *values* with credential
names are redacted; only the element text of an attribute-carrying
tag is left alone.)

### Format: human lines, not JSON lines

Human-readable lines on stderr, prefixed by component:
`http: ...`, `retry: ...`, `decision: ...`, `config: ...`. Two
kinds are unprefixed: the startup line (the bare command line) and
the service status lines (the verb names the operation). The
consumer of `--debug` is a human (or an agent reading a failure
report) diagnosing a run — JSON lines would need parsing to read, and
the machine-readable verdict already exists (the JSON error payload
plus the exit code). Forcing heterogeneous events (an HTTP attempt, a
filing decision, a DryRun guard) into one JSON schema is schema work
with no consumer. The payload contract is untouched: debug never
changes stdout (`docs/architecture.md` already states "Verbosity
never changes stdout" — this design implements that line).

### Granularity: one `--debug`, no `-v` ladder

One level that prints everything the channel carries. Leveled
verbosity (`-v`, `-vv`) is preemptive complexity: no evidence any
command is too noisy to read whole, and a ladder would split
the event
catalog across levels with no rule for which event belongs where. If
a command's debug output gets too noisy, that is a new decision with
the noisy command as evidence — not a ladder built in advance.

### Where it lives: the `log` facade with a bespoke backend

- A small `Debug` sink type in the library (`src/debug.rs`): an
  `enabled: bool` plus pre-formed lines. The library's `UreqClient`
  and `ObsClient` take it at construction (builder or constructor
  parameter); the binary constructs it from the flag/env and passes
  it in. The sink is public in the library so other consumers
  (freshcrab) can wire it to their own log.
- **The `log` crate (decision 4).** The emit path goes through the
  `log` facade to a minimal bespoke `log::Log` backend installed by
  `install`, writing the plain human line to stderr. There is no
  `env_logger` and no `RUST_LOG` semantics: the backend accepts only
  records from this module, so another crate's `debug!` records never
  reach the channel. Static linking makes the dependency acceptable.
- **Near-zero cost when off.** The HTTP transport checks `enabled()` before
  formatting; the library decision-point sites format one small string
  each, dropped inside `emit()` when the channel is off. The sink
  takes pre-formed lines, so the check is one bool per event.
- **The two existing prints.** `try_change_state` emits a
  `decision:` line on the channel when it declines an accept
  (`src/obs/request.rs:3205`): the refused-change path (`:3208`)
  propagates the error, so the reason reaches the JSON error
  payload, while the already-in-target-state path (`:3225`)
  returns `Ok(false)` — the boolean outcome is in the payload, so
  nothing is lost. The re-issue `decision:` lines
  emit live in `post_changestate_with_retry` (`:2934`, emits
  `:2962-3002`). The `[dry-run]` plan echo
  (`src/obs/types.rs`, `DryRun::note`) is unconditional stderr
  narration: it is the dry-run feature's output, not a debug print —
  a dry run shows what would happen — and the plan is data
  (`DryRun::plan`). It is documented where it lives.

### Composition with `--format`

Orthogonal: `--format` selects the stdout payload, `--debug`
selects stderr diagnostics. `--format text --debug` prints the text
payload on stdout and debug lines on stderr; `--format json
--debug` prints JSON on stdout and the same debug lines on stderr.
Debug output is identical in both modes.

## 2. Human-readable output (#52)

### The obligation, enforced by the type

Every CLI command returns `(payload, text)` — the JSON payload and a
human rendering of the same data — exactly as `run()` does today for
`version` and `config`. **The pair is the enforcement:** a command
handler must return both arms, so a command without a text renderer
does not compile. No checklist, no review gate, no later pass — the
type is the rule.

Consequences:

- **Text renderers ship in the same PR as the command.** A command
  PR that adds a subcommand without its renderer is uncompilable, so
  a command without a renderer cannot land.
- **The renderer lives in the binary (`src/main.rs`), not the
  library.** The library returns typed data; rendering is a
  presentation concern of the CLI. The library stays
  format-agnostic.
- **Tests for both, in the same PR.** A JSON test (existing pattern)
  plus a text test (render the fixture payload, assert the lines).
  A third test asserts the two agree on identifying fields: every id,
  name, and revision the renderer carries appears in the text
  rendering; fields listed as documented omissions are exempt.
  (See Test story.)

### Shape rules (the renderer contract)

1. **Text renders the same data as the JSON payload** — never extra
   facts, never fewer identifying facts, except for the documented
   omissions below. A summary may omit long free text (a
   description, a comment body); it never silently omits an id, a
   name, a revision, or a state.
   - **Coordinate header:** text starts
     with one header line echoing the query coordinates — the
     required positionals, in the order project, package,
     repository, arch (as applicable), plus `package:`
     when the query is package-scoped. Tables stay as the worked
     examples pin them; the header is the only addition.
   - **Documented omissions:** anything else the text leaves out is
     listed per renderer in the module docs, with the reason —
     never silent. The JSON payload remains the complete record.
   - **Content commands (rule 4) are exempt from the header:**
     their text *is* the content, verbatim; the coordinates live
     in the JSON payload. The exemption covers the content itself;
     a synthetic empty-content line (e.g. `no log`) still carries
     the header.
   Every text rendering opens
   with one header line echoing the query coordinates
   (project/package/repository/arch as applicable), except content
   commands (rule 4), whose text is the content verbatim; tables
   stay as designed; every other omission stands as a DOCUMENTED
   per-renderer omission, not a fix — recorded at the renderer
   (comment + module docs), never by expanding the table to echo the
   full payload.
2. **Single objects render as `key: value` lines**, one fact per
   line — the `config show` precedent. Nested objects flatten with a
   prefix (`action 1: submit ...`); nested lists get one line per
   item.
3. **Lists render as tables**: a header row, space-aligned columns,
   no borders, no box-drawing. Column width is the widest value in
   the printed set. No truncation: a long value makes a wide
   line, and the JSON payload remains the complete record — text never
   silently drops an identifier. (Truncation is a separate decision if a
   real command's output proves unreadable.) A table cell may still
   summarize a nested object in compact form (the `request search`
   actions column shows `submit foo`); the full fields stay in the
   JSON payload and in the corresponding `show` command — that is the
   sanctioned exception to rule 1, not silent dropping.
4. **Content commands render verbatim.** For `cat` (and any future
   command whose payload *is* a file's content), the text rendering is
   the content itself, not `key: value` lines. JSON carries it as a
   string field; text prints it raw.
5. **Empty results print an explicit line** (`no files`, `no
   requests`, `no results`) rather than empty output — empty stdout is
   ambiguous (success with nothing vs a broken pipe).
6. **Deterministic order.** The JSON array order and the text row
   order are identical, always — the renderer never re-sorts.
7. **No reformatting of values.** Timestamps print as the server gave
   them (ISO); never relative ("2h ago" — relative times rot in
   transcripts). Numbers print as the server gave them.
8. **No color, no paging, no spinners.** Non-interactive output
   must stay clean in pipes and logs. Color is a separate decision
   (opt-in, tty-gated) — not this one.
9. **One renderer, escapes by construction** (`docs/architecture.md`):
   text is rendered from parsed data, never a raw print of server
   text — except content commands (§2 rule 4), where the content is
   the parsed body, not a raw socket dump.

### Worked examples (shape, not an exhaustive field list)

Per-command fields are the command author's decision, following the
shape rules above. Six examples pin the shape:

- **`request show <id>`** (single object, `key: value`):
  `id: 1398977`, `state: new`, `creator: ...`, `created: ...`,
  then one line per action (`action 1: submit openSUSE:Factory/foo
  rev 42 -> openSUSE:Factory`), one line per review (`review 1: by
  group:factory-staging state accepted`).
- **`request search` / `search_requests`** (list, table):
  header `ID STATE CREATOR ACTIONS`, one row per request, the actions
  column a compact inline form (`submit foo`). `request list` renders
  the same table under a `list` header; `--since` bounds by history or last-update
  date, `--limit` caps the rows.
- **`list_files`** (list, table): header `NAME SIZE MTIME`, one row
  per file. The md5 is JSON-only: it is a verification value,
  not a human identifier, and a 32-hex column would dominate every
  row. (If a human need for the md5 in text is proven, it is a new
  decision.)
- **`cat <package> <file>`** (content): the file verbatim.
- **`download <package> <file> --output <path>`** (single object +
  confirmation): header `project: … package: … file: …`, then
  `downloaded <file> (<n> bytes) to <path>` — the bytes never ride
  the text rendering. (tests/cli.rs:1242)
- **`build results`** (list + summary, batch 5): a `project:`
  header line, then the table `REPOSITORY ARCH PACKAGE CODE`, then
  a summary line (`1 failed, 3 succeeded`, terms alphabetical) —
  the summary is derived from the same data, not a new fact.

### Opt-in via `--format text` everywhere

**Opt-in via `--format text` everywhere — no terminal sniffing.**
(Decision 1.)

- **Determinism.** The same invocation must produce the same bytes
  in every context. A tty-sniffing default means `geectl request
  show 5` prints text in a terminal and JSON in a pipe — the same
  command, different output. For a tool whose users run it both ways
  (terminal, CI, scripts), that
  nondeterminism is a bug farm: a script that worked interactively
  breaks in CI, a pasted transcript does not reproduce, tests must
  fake a tty.
- **Automation is the primary consumer.** JSON default everywhere
  keeps programmatic parsing unconditional — no "which format did I
  get?" branch.
- **Humans opt in once.** `--format text` is explicit and cheap;
  shell users who want text always alias it
  (`alias geectl='geectl --format text'`). The docs show the alias.
- **The architecture doc already states it:** "JSON by
  default, one uniform `--format` across every command"
  (`docs/architecture.md`) — tty-sniffing would violate "uniform".
- The alternative (text on a tty, JSON otherwise) is what many CLIs
  do — and it is the wrong default here precisely because geectl's
  scope gate asks "how does an agent use this?", and agents need
  determinism more than humans need a saved flag.

### Persistent opt-in without sniffing: config + env

A human who always wants text should not have to type the flag every
time — but the answer is persistence, not detection. **`format` becomes a config-file key and a `GEECTL_FORMAT`
environment variable**, under the existing precedence (flag > env >
config file > built-in default). (Decision 2.)

- The config change is small: `config.rs` gains a `format` field,
  validated (`json`|`text`, anything else is `ConfigError` naming
  the value — a format is not a secret, so naming it is fine), and
  `config show` reports the effective format and its source, like
  the endpoint URLs.
- The risk is stated, not hidden: a human's config that sets `text`
  would break an agent that parses stdout as JSON without passing a
  flag. The mitigation is a rule, not code: **agent invocations pass
  `--format json` explicitly** (the skill's invocations do), and the
  flag beats the config. `config show` makes a stray `text` default
  diagnosable.
- The config/env default stands; if it is ever revoked, the design still stands:
  `--format text` per invocation is the whole mechanism, and the
  alias is the persistent form.

### Errors in text mode

Errors go to stderr as a JSON payload today, in both formats. A
human running `--format text` who hits an error currently gets JSON
on stderr — the annoying experience this issue is about.
**Render the error in the selected format too** — text
mode prints `error: <message>` plus `remediation: <...>` lines on
stderr; JSON mode keeps the JSON payload. The error kind and the
exit code are unchanged; only the rendering varies, in one place
(`write_error` takes the format). (Decision 3.) The alternative
— one JSON error path always — is defensible (one exit path, one
error shape), but it leaves the human-facing half of this issue
unsolved: every failed text-mode run still ends in JSON.

### Dependency: the OBS command surface

The OBS command surface exists in the binary (PR #83): `remote`,
`request`, `branch`, `build`, `maintenance`, `service`, `gitea`.
The library operations behind it (`show_request`,
`search_requests`, `cat`, `list_files`, `list_packages`,
`package_log`, `request_diff`, `change_state`, `branch_package`,
`build` reads, … — 194 public functions across `src/obs/`, of which
the command surface exposes a thin subset). The text renderers ride
on that work:

- **One subcommand per operation (or per operation group), thin
  over the library.** Every leaf command returns `(payload, text)`.
- **The (payload, text) pair is the command handler's return
  type.** Retrofitting a renderer onto an existing command is the
  drift this design prevents.
- **Text never drives new library API.** A renderer that needs a
  field the library does not return is a library gap, filed as
  such — the renderer does not reach around the library into raw
  XML or a second HTTP call.

## Errors (shared)

- The error store (`src/error.rs`, 23 kinds, exit codes 1–23) is
  unchanged. `--debug` adds diagnostics around an error, never a
  new error kind: a failed run with `--debug` ends with the same
  JSON error payload (or the text rendering, decision 3) and the
  same exit code, preceded on stderr by the debug lines that explain
  it.
- Debug lines and the error payload interleave on stderr in time
  order — both are stderr, so ordering is natural. Nothing on
  stdout changes on any failure path.
- A new failure mode for the debug channel itself: none. If stderr
  is closed, debug writes are ignored like the existing `write_error`
  (there is nowhere else to report it). Debug never fails a run.

## Test story

- **PR-level (gating):** fixtures, never a live server.
  - Debug off → no debug lines: the sink writes to a `Vec` in
    tests, not real stderr; assert it stays empty for a scripted
    run with debug disabled.
  - Debug on → the event lines for a scripted HTTP sequence
    (fixture transport): attempt lines with method/URL/status/
    duration/attempt, a retry line with the backoff, an exhaustion
    line. Assert the exact lines for a 500-then-200 sequence and
    for a 404 (no retry, one line).
  - Redaction: a fixture run with a fake credential asserts the
    credential string appears nowhere in the debug output.
  - Flag/env precedence: `--debug` enables with `GEECTL_DEBUG`
    unset; `GEECTL_DEBUG=1` enables without the flag;
    `GEECTL_DEBUG=0` (and empty, and `yes`) disables without the
    flag.
  - Text: every command renders its fixture payload in both
    formats; the identifying-fields agreement test (every id/name/
    revision in the JSON appears in the text); empty-list renders
    the explicit `no <things>` line; `cat` renders verbatim.
  - Error rendering (decision 3): text mode renders
    `error:` + `remediation:` lines; JSON mode keeps the payload;
    exit code identical in both.
- **Live (non-gating):** the live suite already proves the
  operations against a real server; debug adds nothing to it
  except that a red live run can be re-run with `--debug` to
  produce the transcript — that is the channel's purpose.
- **No new scheduled job.** Debug and text are PR-level features;
  the live suite is unchanged.

## Risks

- **Debug lines pasted with secrets.** Mitigated by construction:
  the sink takes pre-formed lines, never headers or bodies, and
  the redaction test asserts a fake credential never appears.
  Residual: a URL carrying a credential as a query parameter
  — the transport guard already forbids credentials over cleartext
  and credentials never go in URLs; a URL with a secret in it
  would be a new bug, and the debug line would surface it.
- **Debug noise drowning the signal.** One level prints
  everything; a chatty command (a search fanning out per-file GETs,
  the `revision_patch` / `project_diff` fan-outs) produces many lines. Mitigated by the one-line-per-event
  shape and by the channel being off by default — it is only read
  when someone asks for it. If a specific command's output proves
  unreadable, that is the evidence for a granularity decision.
- **Text renderers drifting from JSON.** A field added to the
  payload but not the renderer (or vice versa) makes the two
  disagree. Mitigated by the identifying-fields agreement test and
  by the pair return type — the renderer is in the same function as
  the payload, so a diff that touches one and not the other is
  visible in review. Residual: free-text fields omitted from text
  by design (rule 1) — documented per renderer, not silent.
- **Config `format` breaking agent parsing.** A human's config
  setting `text` would change an agent's stdout. Mitigated by the
  rule that agent invocations pass `--format json` explicitly and
  by `config show` reporting the format source. If that mitigation
  proves too weak in practice, decision 2's answer becomes no — the design
  stands without it.
- **Scope creep into a logging framework.** The channel is
  deliberately not leveled, not timestamped, not JSON — one level
  on stderr, through the `log` facade to the minimal bespoke
  backend. Each of those is a separate decision with its own
  justification if a real need appears; this design names them as
  out, so none is added quietly.

## What stays out

- Leveled verbosity (`-v`/`-vv`), `RUST_LOG`, timestamps, JSON
  debug lines, request/response headers in debug output, and
  request/response bodies in default debug output (bodies are
  behind `--debug-body` only, decision 5).
- Color, paging, progress spinners, interactive prompts (the
  binary stays non-interactive; none of these belong in it).
- `env_logger` / `RUST_LOG` semantics.
- Terminal sniffing for the default format.
- Any change to the library's operations: the debug sink observes
  calls, it does not change them; the text renderers present
  payloads, they do not add fields.

## Decisions

### #51 — debug channel

1. **Flag + env, or flag only?** Both —
   `--debug` plus `GEECTL_DEBUG=1`, flag wins. The env var is for scripts and CI
   where editing every invocation is friction; precedence follows
   the existing flag > env rule.
2. **Human lines or JSON lines on stderr?** Human lines — the consumer is a human diagnosing a run, and the
   machine-readable verdict already exists (JSON error payload +
   exit code).
3. **One `--debug` or leveled `-v`?** One level — no evidence any command is too noisy to read whole.
4. **Hand-rolled sink or the `log` crate?** Use the `log` crate — static linking makes the dependency
   acceptable. The channel contract still holds: stderr, human
   lines, one level, off by default.
5. **Request/response bodies in debug output?** Bodies behind a separate flag (e.g. `--debug-body`), size-capped
   and redacted — default debug output never prints bodies, so no
   secrets land in the log by default.

### #52 — human-readable output

1. **Text default on a terminal, or opt-in everywhere?**
   Opt-in via `--format text` everywhere,
   no terminal sniffing — the same invocation produces the same
   bytes in every context, and the agent is the primary consumer.
2. **Persistent opt-in via config file + env (`format` in
   `config.toml`, `GEECTL_FORMAT`)?** Yes — persistence without detection, under the existing
   precedence (flag > env > config > default); agent invocations
   pass `--format json` explicitly.
3. **Errors in text mode: render as text, or keep JSON errors
   always?** Render in the selected format — text mode: `error:` + `remediation:` lines on stderr;
   JSON mode unchanged.
