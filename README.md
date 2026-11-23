<img src="https://raw.githubusercontent.com/scarabeusiv/geectl/main/docs/assets/logo.svg"
     align="right" width="130" alt="geectl logo">

# geectl

**Geeko Controller — a CLI for openSUSE packaging workflows.**

geectl drives openSUSE packaging workflows from the command line, built for
automation and scripting, with human-readable text output via `--format text`. It talks to the OBS (Open Build Service) API
directly (primary goal) and, as a secondary goal, to the src.opensuse.org git
workflow (Gitea), hiding the backend choice.

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

- **Primary goal: OBS.** The operations the packaging skill needs, against
  the OBS API.
- **Secondary goal: git flow, seamlessly.** Integrating the src.opensuse.org
  / Gitea workflow so the agent does not have to know whether it is talking
  to OBS directly or via git. This is **not** initial work and not on the roadmap.

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
- **Credentials from environment variables or mounted secret files only.**
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
| OBS | username + password (Basic auth; OBS has no API key) | `GEECTL_OBS_USERNAME` + `GEECTL_OBS_PASSWORD`, or `GEECTL_OBS_PASSWORD_FILE` |
| Gitea | API token (Bearer) | `GEECTL_GITEA_TOKEN`, or `GEECTL_GITEA_TOKEN_FILE` — per instance, `GEECTL_GITEA_TOKEN_<ALIAS>` (see below) |
| Gitea | SSH private key (git operations) | `GEECTL_GITEA_SSH_KEY` |

The `*_FILE` variables name a file whose whole content is the secret
(trailing newlines are trimmed); they are how a mounted secret (e.g.
a container secret) is used. Setting both a variable and its `_FILE`
form is an error, as is a half-configured pair. A malformed value fails
at startup as a config error naming the variable, never the value.

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
github-actions, weekly, no grouping, limit 100, no auto-merge) proposes
the updates; the repo stays green by merging them promptly, not by
relaxing the checks.

## License

Apache-2.0. See [LICENSE](LICENSE).
