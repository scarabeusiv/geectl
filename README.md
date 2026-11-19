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
