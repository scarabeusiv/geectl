<img src="https://raw.githubusercontent.com/scarabeusiv/geectl/main/docs/assets/logo.svg"
     align="right" width="130" alt="geectl logo">

# geectl

**Geeko Controller — a CLI for openSUSE packaging workflows.**

geectl drives openSUSE packaging workflows from the command line, built for
automation and scripting, with human-readable text output via `--format text`.
One operation surface covers the OBS and Git workflows, hiding the
backend choice.

## Usage

See [docs/cli.md](docs/cli.md) for the command reference with real
`--help` output.

[![test](https://github.com/scarabeusiv/geectl/actions/workflows/test.yml/badge.svg)](https://github.com/scarabeusiv/geectl/actions/workflows/test.yml)
[![freshness](https://github.com/scarabeusiv/geectl/actions/workflows/freshness.yml/badge.svg)](https://github.com/scarabeusiv/geectl/actions/workflows/freshness.yml)
[![CodeQL](https://github.com/scarabeusiv/geectl/actions/workflows/codeql.yml/badge.svg)](https://github.com/scarabeusiv/geectl/actions/workflows/codeql.yml)

## Scope gate

Every feature or PR request must answer **"how does an agent use this
functionality?"** No agent use case, no implementation. The full scope is not enumerated
upfront; the gate does the work. There is no 1:1 parity goal with anything:
only the parts needed for management and processing as agents.

- **Both backends, one surface.** The operations the packaging skill needs,
  against the OBS and Git workflows, so the agent does not have to know
  whether it is talking to OBS directly or via git.

## How it differs from osc

geectl is not osc with a new name: one noun per operation family,
explicit flags instead of positionals, no working-copy inference,
JSON by default with opt-in text, and dry-run by default for
mutations. The full list of deliberate deviations, with reasons, is
[docs/cli-swaps.md](docs/cli-swaps.md).

## What geectl guarantees

- **Non-interactive is the only mode.** No prompts, no editor, no pager.
  Missing required input fails fast with a structured error naming the
  missing flag.
- **Stdout is only the result payload** (JSON by default, one uniform
  `--format` across every command). Every diagnostic, warning, and error
  goes to stderr. Verbosity never changes stdout content.
- **Structured errors, one error store.** Every error kind has a code,
  message, remediation, and exit code, defined in one place
  (`src/error.rs`). Agents can distinguish not-found, auth-failed,
  server-error, and invalid-input programmatically.
- **Bounded timeouts on every request.** No unbounded request anywhere.
- **Credentials from environment variables (or a `.env` file in the
  working directory — see below) or mounted secret files only.**
  Never in a config file, never prompted, never logged. There is no
  plaintext fallback and no obfuscation option.
- **No option to disable TLS verification.** Ever.

See [docs/design-choices.md](docs/design-choices.md) for the rationale
behind these guarantees. See
[docs/architecture.md](docs/architecture.md) for the crate layout.

## Authentication

Credentials come from environment variables or mounted secret files
only — never from a config file, never prompted, never logged.

| Service | Credential | Variables |
| ------- | ---------- | --------- |
| OBS | API token (Bearer) or username + password (Basic auth) | `GEECTL_OBS_APIKEY` or `GEECTL_OBS_APIKEY_FILE`, or `GEECTL_OBS_USERNAME` + `GEECTL_OBS_PASSWORD` (or `GEECTL_OBS_PASSWORD_FILE`) |
| Gitea | API token (Bearer) | `GEECTL_GITEA_TOKEN`, or `GEECTL_GITEA_TOKEN_FILE` — per instance, `GEECTL_GITEA_TOKEN_<ALIAS>` (see below) |
| Gitea | SSH private key (git operations) | `GEECTL_GITEA_SSH_KEY` |

The `*_FILE` variables name a file whose whole content is the secret
(trailing newlines are trimmed); they are how a mounted secret (e.g.
a container secret) is used. Setting both a variable and its `_FILE`
form is an error, as is a half-configured pair or setting both a token
and a Basic pair for one instance. A malformed value fails
at startup as a config error naming the variable, never the value.

A `.env` file in the working directory supplies the same variables, when the run opts in with `GEECTL_DOTENV=1` in the real process environment:
geectl loads it at startup and merges it into the environment, with a
real environment variable winning over the same key in the file. Only
`GEECTL_*` keys are honoured, so a project `.env` meant for other tools
cannot change geectl behaviour. A key starting with `GEEC` that is not
a valid `GEECTL_*` key, or a `GEECTL_*` key with anything but ASCII
letters, digits, or `_` after the prefix, is a typo for a geectl
variable and fails as a config error rather than being silently
ignored. The syntax is minimal — `KEY=value`, `#` comments, blank
lines, an optional `export` prefix followed by whitespace (a key glued to `export` is malformed), and
optional single or double quotes — with no variable interpolation and
no multi-line values. A duplicate key keeps its last value, and quoted
values are literal inside the quotes, with no escapes. The file must be
a regular file, at most 64 KiB, and not readable by group or others
(`chmod 600`) on Unix — on non-Unix the mode cannot be checked, and
the opt-in is the guard there. A missing file is fine, and so is a
file that carries no `GEECTL_*` keys, whatever its mode or syntax —
but an unreadable or malformed file that carries `GEECTL_*` keys
fails at startup as a config error naming the file (and the line,
for a malformed line), never the value. `--version` and `--help` print
and exit before the file is loaded, so they never fail on it.
`--debug` reports that a
`.env` file was loaded, never its values. Values from the file report
their source as `environment` in `config show`. No parent directory is
searched, and `.env.*` files are not loaded.

OBS does not issue personal API tokens yet; the client accepts one so
it works when the server ships it — until then a token login fails
with a 401.

Credentials are stored per API URL: a second instance gets one line in
the config file's `[instances]` table (alias → URL) and its own
variables, with the alias uppercased — e.g. `[instances]
myinstance = "https://gitea.example.org"` takes
`GEECTL_GITEA_TOKEN_MYINSTANCE` (or
`GEECTL_GITEA_TOKEN_FILE_MYINSTANCE`). A call resolves its credentials
by the URL it talks to, so the default instance's token is never
forwarded to another instance; one configured URL keeps working
exactly as before. The alias `file` is reserved — it would collide
with the plain `GEECTL_GITEA_TOKEN_FILE` variable.

The SSH key must be a PEM private key in a file mode 0600 or stricter;
a group/world-readable key is a config error.

The config file (`$XDG_CONFIG_HOME/geectl/config.toml`, default
`~/.config/geectl/config.toml`) holds endpoint URLs only (`obs_url`,
`gitea_url`). A credential-shaped key in it is a refusal to run.
`config show` prints the effective URLs with their source and the
credential status (username only for Basic auth, never a secret value);
`config init` writes the template and refuses to overwrite.

There is no keyring backend: it needs a platform secret service, which
only serves interactive use. Credentials come from environment variables
or mounted secrets. The `AuthProvider` trait is the seam a
keyring provider would plug into.

Credentials are only sent over `https://` (or to a loopback host for
tests). There is no option to disable TLS verification, ever.

## Build and test

```sh
cargo test --locked
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo deny check
```

The man pages and shell completions live in `man/` and `completions/`,
generated from the CLI definition by `cargo run --features gen --bin geectl-gen`
and checked in. The `assets-drift` CI job regenerates them and fails on any
difference, so a CLI change regenerates the assets in the same PR.

Requires any recent stable Rust toolchain. The binary is `geectl`; the library is also `geectl`,
so tools can link the same logic instead of shelling out.

```sh
cargo run -- version
cargo run -- --help
```

## Live tests

Beyond the per-PR fixture tests, a `live` workflow runs
live tests against a real (temporary) OBS instance, on every pull
request, weekly on Mondays 06:00 UTC and on demand. It spins up the instance from the OBS
project's own development containers, runs the tests in
`tests/live.rs` against it, and tears it down. The assertions are
on server state — what geectl reads must reflect what the server
actually holds — which is what catches drift as OBS changes underneath
us.

The job is independent and does not gate PRs. The provisioning lives in
`tests/live/` (see its README).

## Distribution

geectl is distributed as an OBS package with tagged releases, fetched via
zypper. Containers fetch the released version — no building random hashes.

## Keeping dependencies fresh

A weekly `freshness` workflow FAILS on any outdated dependency, any
`cargo audit` advisory, any `cargo deny check` failure, any unused
dependency (`cargo machete`), or a stale `Cargo.lock`. Dependabot (cargo +
github-actions, weekly, no grouping except the github/codeql-action init + analyze pair, limit 100, no auto-merge) proposes
the updates; the repo stays green by merging them promptly, not by
relaxing the checks.

## License

Apache-2.0. See [LICENSE](LICENSE).
