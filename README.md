![geectl logo](docs/assets/logo.svg)

# geectl

**Geeko Controller — a machine-first CLI for agents.**

geectl drives openSUSE packaging workflows from the command line, built for
agents, not for interactive use. It talks to the OBS (Open Build Service) API
directly (primary goal) and, as a secondary goal, to the src.opensuse.org git
workflow (Gitea), hiding the backend choice — the routing is decided on our
side.

It is a pure agentic flow. Human flows already exist and are not disrupted.

[![test](https://github.com/scarabeusiv/geectl/actions/workflows/test.yml/badge.svg)](https://github.com/scarabeusiv/geectl/actions/workflows/test.yml)
[![freshness](https://github.com/scarabeusiv/geectl/actions/workflows/freshness.yml/badge.svg)](https://github.com/scarabeusiv/geectl/actions/workflows/freshness.yml)
[![CodeQL](https://github.com/scarabeusiv/geectl/actions/workflows/codeql.yml/badge.svg)](https://github.com/scarabeusiv/geectl/actions/workflows/codeql.yml)

## Scope gate

Every feature or PR request must answer **"how does an agent use this
functionality?"** No agent use case, no implementation. That is the standing
rule that keeps the scope from ballooning — the full scope is not enumerated
upfront; the gate does the work. There is no 1:1 parity goal with anything:
only the parts needed for management and processing as agents.

- **Primary goal: OBS.** The operations the packaging skill needs, against
  the OBS API.
- **Secondary goal: git flow, seamlessly.** Integrating the src.opensuse.org
  / Gitea workflow so the agent does not have to know whether it is talking
  to OBS directly or via git. This is **not** initial work. If anyone opens
  a PR for it, we review and support it if it bites us, but it is not on
  the roadmap.

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

See [docs/design-choices.md](docs/design-choices.md) for the design
decisions behind these guarantees, including the risks found by reading
another CLI's source and the rules geectl follows instead. See
[docs/architecture.md](docs/architecture.md) for the crate layout.

## Authentication

Credentials come from environment variables or mounted secret files
only — never from a config file, never prompted, never logged.

| Service | Credential | Variables |
| ------- | ---------- | --------- |
| OBS | username + password (Basic auth; OBS has no API key) | `GEECTL_OBS_USERNAME` + `GEECTL_OBS_PASSWORD`, or `GEECTL_OBS_PASSWORD_FILE` |
| Gitea | API token (Bearer) | `GEECTL_GITEA_TOKEN`, or `GEECTL_GITEA_TOKEN_FILE` |
| Gitea | SSH private key (git operations) | `GEECTL_GITEA_SSH_KEY` |

The `*_FILE` variables name a file whose whole content is the secret
(trailing newlines are trimmed); they are how a mounted secret (e.g.
a container secret) is used. Setting both a variable and its `_FILE`
form is an error, as is a half-configured pair. A malformed value fails
at startup as a config error naming the variable, never the value.

The SSH key must be a PEM private key in a file mode 0600 or stricter;
a group/world-readable key is a config error.

The config file (`$XDG_CONFIG_HOME/geectl/config.toml`, default
`~/.config/geectl/config.toml`) holds endpoint URLs only (`obs_url`,
`gitea_url`). A credential-shaped key in it is a refusal to run.
`config show` prints the effective URLs with their source and the
credential status (username only for Basic auth, never a secret value);
`config init` writes the template and refuses to overwrite.

A keyring backend is deferred: it needs a platform secret service and
only serves interactive human use, while agents use environment
variables or mounted secrets. The `AuthProvider` trait is the seam a
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
