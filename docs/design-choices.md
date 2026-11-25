# Design choices

The rules in this document are load-bearing: they keep geectl usable
by agents. Each rule states what geectl does, and the
rationale states the failure it prevents.

## Interaction model

### Non-interactive is the only mode

All input arrives via flags. Missing required input fails fast with a structured error
naming the missing flag.

Rationale: a prompt hangs an agent with silence as the failure mode.

### Secret input comes from the environment

Secrets come from environment variables or mounted secret files. A
config file supplies non-secret settings for humans. A `.env` file in
the working directory supplies environment variables. geectl reads
input from stdin and files, and leaves `/dev/tty` untouched.

Rationale: a terminal read bypasses stdin redirection, so the same
command behaves differently depending on whether a controlling
terminal is present.

## HTTP and API

### Every request carries a bounded timeout

Every request has a bounded connect timeout and a bounded read timeout,
unconditional on library version, as documented defaults (overridable by
explicit config). A test asserts a hung server returns
a structured timeout error within the bound.

Rationale: a bound turns a hung server into a structured timeout error
within the bound, on every library version.

### Retry only idempotent methods

Retry covers GET, HEAD, PUT. A POST retry requires an explicit
idempotency key sent with the request. The retry policy (methods,
statuses, count, backoff) is a single documented constant.

Rationale: idempotency keys make retried mutations safe.

### Auth refresh happens before the body is consumed

A retry after a 401 requires a byte-identical body. Credentials are
obtained or refreshed before any mutating request.

Rationale: a mutation retried with a truncated body creates partial
server state while the caller sees a success.

### One URL builder, one encoder per context

Path segments are encoded with an empty safe set except `:` (the OBS
`project:package` separator) — `/` is always encoded, and segments
keep their exact content. Query values use
the query encoder, where `+` is encoded because a query string decodes
`+` as a space. All query strings go through the encoder.

Rationale: a value containing `/` alters the path structure, and
server-derived data flows into segments.

### Parse server XML with a hardened parser

XML is parsed with a streaming parser that rejects entity expansion
and enforces explicit limits on document size, nesting depth, and
element count. Error messages report the parse position and the
expected element.

Rationale: entity expansion and deep nesting enable denial of
service, and document content in a parse error leaks into logs.

## Auth and credentials

### Credential sources are environment variables and mounted secret files

Credentials come from environment variables (or a `.env` file in the
working directory, which merges into the environment) or mounted secret
files. A config file containing a credential-shaped value stops
geectl at startup, naming the variable or file to use instead.

Rationale: the secure path is the default path for programmatic
config writes.

### Credential files are created 0600 from the first byte

Any file that can contain a credential is created with mode 0600 from
the first byte. The temporary file for an
atomic rename gets the same mode.

Rationale: creation mode protects the secret from the first byte,
including on interruption or a kill mid-write.

### TLS verification is always on

Verification is always on. A server whose certificate fails verification
produces a structured error.

Rationale: verification applies to every connection.

### Where credentials come from: env vars and mounted secret files

OBS takes a username and password (`GEECTL_OBS_USERNAME` +
`GEECTL_OBS_PASSWORD`, or `GEECTL_OBS_PASSWORD_FILE` for the
password); Gitea takes an API token (`GEECTL_GITEA_TOKEN`, or
`GEECTL_GITEA_TOKEN_FILE`) and, for git operations, an SSH private
key (`GEECTL_GITEA_SSH_KEY`). The `*_FILE` form names a file whose
whole content is the secret — the mounted-secret shape. A `.env` file
in the working directory supplies the same variables when the run opts
in with `GEECTL_DOTENV=1` (see
`docs/cli.md`); a real environment variable wins over the same key in
the file, and only `GEECTL_*` keys are honoured. A
half-configured pair, both secret sources set, or a malformed value
fails at startup as a config error naming the variable, for any
command that resolves credentials. The Credentials paragraph in
`docs/architecture.md` states which commands resolve credentials; the
dry-run handling is stated in `docs/cli.md`.

Rationale: startup failure names the misconfigured variable, turning
a misconfiguration into a diagnosis.

### Keyring backend

Credential providers are environment variables and mounted secret
files. The `AuthProvider` trait is the seam a keyring provider plugs into
when a human-facing use case needs it.

## Output and errors

### Stdout is only the result payload

Stdout carries only the result payload (JSON by default, one uniform
`--format` across every command). Every diagnostic, progress line,
warning, and error goes to stderr. Stdout content is identical at
every verbosity level.

Rationale: stdout stays parseable for every command.

### Sanitization is structural

JSON output keeps server text as data. Human-readable output is rendered
from parsed data through one renderer that escapes by construction.
All server text reaches the terminal through the renderer.

Rationale: escaping by construction covers every call site,
including attacker-controlled text.

### One exit path, exit codes per error kind

The centralized error store defines the exit code per error kind, and
there is exactly one exit path. Exit values come from the error store.

Rationale: distinct codes let a caller retry a transient server
error and fail fast on bad input.

### Errors are structured values; the error path has its own test

Errors are structured values from the central store (kind, code,
message, remediation, exit code). The server's error body is parsed with
the same hardened XML parser as everything else; if parsing fails,
the raw status plus a truncated body excerpt are reported as data.
Every error kind renders cleanly — the error path has its own test.

Rationale: structured errors give callers a stable identity to match
on, and the error path works on unexpected input.

## Concurrency and shared state

### Shared resources are locked or unique per invocation

Any shared mutable resource it owns (state files, caches, the
queue) is either locked with `flock`
for the duration of the operation or unique per invocation. A
destructive operation (`clean`, `wipe`, `reset`) on an in-use resource
fails with a structured error.

Rationale: a destructive operation on an in-use resource destroys
in-flight work.

### State writes: unique temp file, fsync, rename, fsync the directory

State writes use a unique temp file (`O_EXCL`, random suffix), fsync the
file, then rename, then fsync the directory. Any file that can be written
by concurrent processes is also guarded by `flock` around the
read-modify-write. Each write creates a fresh temp file.

Rationale: fsync plus rename gives atomicity and durability for a
single writer; locking covers concurrent writers.

### Session stores: lock every read-modify-write, surface corruption as an error

If geectl keeps a session store, the lock covers every
read-modify-write, the save is atomic (temp + fsync + rename), and a
corrupt store is a structured error. The preferred shape is
re-authentication per invocation from env-provided credentials, or an
in-memory-only session.

Rationale: a corrupt store surfaces as a structured error, and the
lock covers every read-modify-write.

## Configuration

### One config file, explicit precedence, `config show`

One config file: `$XDG_CONFIG_HOME/geectl/config.toml` (default
`~/.config/geectl/config.toml`). Explicit precedence, documented in
one place: flag > environment variable > config file > built-in
default. A `.env` file in the working directory supplies environment
variables when the run opts in with `GEECTL_DOTENV=1` — a real
environment variable wins over the same key in the file — and reports
its source as `environment`. A `config show` command
prints the effective configuration
with the source of each value; `config init` writes the template and
refuses to overwrite. Two config files found is an error.

Rationale: one config file gives one effective configuration with a
stated source per value.

## Patterns adopted

Patterns geectl adopts: atomic rename for store writes (write-temp-then-rename plus O_EXCL, fsync, and locking); download verification by default (skipping verification takes an explicit flag); shell-free invocation (external programs are invoked by argv, with user and server data passed as arguments); non-2xx raising at the transport layer (the HTTP layer converts any non-2xx status into an error before any caller sees the body, and geectl maps the status to the structured error store); one function turning a response into dicts with stable field names, generalized to every command with one uniform `--format`; and fail-closed defaults (when a decision cannot be made non-interactively, the operation aborts).
