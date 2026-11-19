# Design choices

This document records the design choices that shape geectl, and why each
one was made. The rules here are load-bearing: they are what keep geectl
usable by agents, and they were written down before the first feature so
later work does not have to rediscover them. Several of them come from a
survey of existing CLI implementations, where the failure mode was found
in real code — the rule states what geectl does instead, and the
rationale states the failure it prevents.

## Interaction model

### Non-interactive is the only mode

No prompts, no editor, no pager, no input primitive anywhere in the
codebase. Missing required input fails fast with a structured error
naming the missing flag. A lint/grep check in CI fails the build if a
prompt primitive appears.

Rationale: every prompt is a hang site for an agent, and the failure
mode is silence, not an error — a caller cannot distinguish "waiting for
input" from "slow server". When prompts are scattered through command
implementations rather than going through one gate, any new command can
silently add a new hang site. The only way to prevent that is to have no
input primitive at all.

### Never read a secret from the terminal

Secrets come only from environment variables or mounted secret files (or
a config file for humans, never prompted). Nothing in geectl touches
`/dev/tty`, and there is no `getpass`-style prompt.

Rationale: a terminal read bypasses stdin redirection, so the identical
command fails fast in one environment (no controlling terminal) and
hangs forever in another (a terminal session). Environment-dependent
hanging is the worst kind of failure for an agent, because it cannot be
reproduced from the caller's side.

### Trust is a configuration decision, never a runtime prompt

Certificate trust and project trust are decided explicitly in a config
file or a dedicated `trust` subcommand — never mid-request, never
"continue anyways". If a trust store is supported, its files are created
0600 from the first byte. A certificate that fails verification fails
the request with a structured error naming the host and the failure.

Rationale: a trust decision made mid-request, under time pressure, by
whoever happens to be at the keyboard (or by an agent that cannot
answer) is not a decision at all. "Continue anyways" next to "trust
permanently" invites the wrong keypress, and the persisted decision then
silently changes the security posture of every later run.

## HTTP and API

### Every request carries a bounded timeout

Every request has a bounded connect timeout and a bounded read timeout,
unconditional on library version, as documented defaults (overridable by
explicit config, never by accident). A test asserts a hung server returns
a structured timeout error within the bound.

Rationale: without a bound, a hung server hangs the CLI indefinitely,
and an agent cannot distinguish "server is slow" from "server is gone".
A timeout that only exists on some library versions is worse than none
in one respect: it passes tests on the version that has it and silently
drops the bound on the version that does not.

### Retry only idempotent methods

Retry only GET, HEAD, PUT, DELETE. POST is never retried. If a POST must
be retried for some reason, it requires an explicit idempotency key sent
with the request. The retry policy (methods, statuses, count, backoff)
is a single documented constant, not per-call-site improvisation.

Rationale: a POST that timed out *after the server processed it* (or
that got a 500 after creating state) is retried, creating a duplicate.
Retrying a mutation is only safe with an idempotency key. This is the
class of bug that is invisible in happy-path tests and shows up only on
flaky networks — i.e. in production.

### Auth refresh happens before the body is consumed

When a request is retried after a 401, the body must be byte-identical
or the retry is refused. Prefer: obtain/refresh credentials before
sending any mutating request.

Rationale: whether a replayed body is intact must not depend on the
*type* of the body object (seekable stream vs file vs in-memory buffer),
which is invisible at the call site. A mutation retried with a truncated
body can create partial state on the server while the caller sees a
success.

### One URL builder, one encoder per context

Path segments are encoded with an empty safe set except `:` (the OBS
`project:package` separator) — `/` is always encoded, never passed
through, and segments are never stripped or trimmed. Query values use
the query encoder, where `+` is encoded because a query string decodes
`+` as a space. No raw-string query path.

Rationale: a single shared URL builder with a permissive safe set means
every caller inherits the injection surface, and two copies of the
builder (one per backend) can drift apart. A value containing `/` alters
the path structure, and values flowing into segments include
server-derived data — so this is a second-order injection surface, not
just a caller bug. Encoding must be a property of the builder.

### Parse server XML with a hardened parser

XML is parsed with a streaming parser with entity expansion disabled and
explicit limits on document size, nesting depth, and element count.
Never include raw document content in an error message; report the parse
position and the expected element instead.

Rationale: a compromised or malicious server response — or a proxy error
page that happens to be XML — can DoS the client with entity expansion
or deep nesting. A parse error that attaches a snippet of the offending
document can itself leak document content into logs.

## Auth and credentials

### No credentials in config files, in any form

Credentials come from environment variables or mounted secret files
only. There is no plaintext fallback and no obfuscation option. If a
config file contains a credential-shaped value, geectl refuses to run
and names the variable/file to use instead.

Rationale: the secure option must not require the user to know it exists
and opt in twice while the insecure option is what happens by default
when a config is written programmatically. Obfuscation that ships its own
decoder is worse than plaintext in one respect: it *looks* protected.

### Credential files are created 0600 from the first byte

Any file that can contain a credential is created with mode 0600 from
the first byte, never chmod-after-write. The temporary file for an
atomic rename gets the same mode.

Rationale: the secure state must not be reached only at the last step of
a multi-step sequence — any interruption in between leaves the secret
exposed, and a process killed mid-write leaves a world-readable file
behind. `chmod` after the fact is not a substitute for creating the file
with the right mode.

### No option to disable TLS verification

Verification is always on. A server whose certificate fails verification
produces a structured error; there is no "continue anyways".

Rationale: a single config line that silently turns off certificate
verification is a persistent, silent downgrade — nothing in the output
marks the connection as unverified, and the option lives in the same
file as the password, so anyone who can edit the config can strip TLS
without the operator noticing.

## Output and errors

### Stdout is only the result payload

Stdout carries only the result payload (JSON by default, one uniform
`--format` across every command). Every diagnostic, progress line,
warning, and error goes to stderr. Verbosity never changes stdout
content.

Rationale: a consumer cannot rely on stdout being parseable if any
command may print human text there, and verbosity changes the stdout
content. A per-command opt-in never converges — three different
machine-readable conventions mean each command is a separate parsing
project. It has to be the default for all commands from the first one.

### Sanitization is structural, not opt-in

JSON output means server text is data, never terminal input, so there is
nothing to sanitize in machine output. Human-readable output is rendered
from parsed data through one renderer that escapes by construction. No
raw print of server text anywhere.

Rationale: a security property that depends on every call site
remembering to apply it will eventually be missed at exactly the call
site that prints attacker-controlled text.

### One exit path, exit codes per error kind

The centralized error store defines the exit code per error kind, and
there is exactly one exit path. No scattered exits, no string exit
values.

Rationale: an agent cannot distinguish "not found" from "auth failed"
from "server error" from "invalid input" by exit code when everything
maps to 1, and the free-text messages are the only signal — which are
not stable across versions. A caller that wants to retry on a transient
server error and fail fast on bad input cannot implement that policy
without distinct codes.

### Errors are structured values; the error path has its own test

Errors are structured values from the central store (kind, code,
message, remediation, exit code). The server's error body is parsed with
the same hardened XML parser as everything else; if it does not parse,
the raw status plus a truncated body excerpt are reported as data, never
scraped with string splits. Every error kind renders without panicking
— the error path has its own test.

Rationale: there is no stable error identity for a caller to match on
when errors are free text, and the error *reporting* path itself can fail
on unexpected input. Error handling is the one path that must work when
everything else is already broken.

## Concurrency and shared state

### Shared resources are locked or unique per invocation

geectl does not run builds in v1, but any shared mutable resource it
does own (state files, caches, the queue) is either locked with `flock`
for the duration of the operation or unique per invocation. A
destructive operation (`clean`, `wipe`, `reset`) refuses when the
resource is in use, and says so in a structured error.

Rationale: the destructive operation is the default-adjacent one — a
clean from one worker destroys another worker's in-flight work, and the
failure surfaces as files vanishing mid-operation, not as a clear error.

### State writes: unique temp file, fsync, rename, fsync the directory

State writes use a unique temp file (`O_EXCL`, random suffix), fsync the
file, then rename, then fsync the directory. Any file that can be written
by concurrent processes is also guarded by `flock` around the
read-modify-write. A stale temp file is never silently reused.

Rationale: the atomic-rename pattern only gives atomicity for a single
writer; with two writers it gives silent last-writer-wins data loss.
Without fsync the rename does not even guarantee durability on crash.

### Session stores: lock every read-modify-write, never swallow corruption

If geectl keeps a session store at all, the lock covers every
read-modify-write, the save is atomic (temp + fsync + rename), and a
corrupt store is a structured error, never a silent swallow. Prefer no
persistent session store: re-authenticate per invocation from
env-provided credentials, or keep the session in memory only.

Rationale: locking that only covers the first load is worse than no
locking in one respect — a reader assumes the store is safe, but the
least contentious moment is the only one covered. Silently swallowing
corruption turns a detectable failure (corrupt session store) into
mysterious re-authentication.

## Configuration

### One config file, explicit precedence, `config show`

One config file location. Explicit precedence, documented in one place:
flag > environment variable > config file > built-in default. A `config
show` command prints the effective configuration with the source of each
value. If two config files are found, that is an error, not a warning.

Rationale: an agent that writes one config location while another exists
gets its settings silently ignored, and the only signal is a warning
that a non-interactive caller may never see. Silent precedence is a
misconfiguration that looks like a server problem.

## Patterns adopted

These are patterns from existing implementations that geectl adopts,
not avoids:

- **Atomic rename for store writes** — write-temp-then-rename is the
  right shape; geectl keeps the shape and adds O_EXCL, fsync, and
  locking.
- **Download verification by default** — verify by default, skip only
  explicitly, for anything geectl downloads.
- **No shell invocation** — external programs are invoked by argv, never
  via a shell string. geectl never builds a shell command from user or
  server data.
- **Non-2xx raises at the transport layer** — the HTTP layer converts any
  non-2xx status into an error before any caller sees the body, so
  callers never have to remember to check the status (and geectl then
  maps the status to the structured error store).
- **One function turning a response into dicts with stable field names**
  — geectl generalizes this to every command with one uniform
  `--format`.
- **Fail-closed defaults** — when a decision cannot be made
  non-interactively, the operation aborts instead of prompting. The
  fail-closed default is right; the flaw is only the interactive prompt
  path.
