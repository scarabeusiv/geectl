//! geectl — Geeko Controller, a CLI for openSUSE packaging workflows.
//!
//! The library is the core of geectl: the binary is a thin CLI over it, so
//! tools (e.g. freshcrab) can link the same logic instead of shelling out.
//!
//! The library is deliberately small at scaffolding time. It carries the
//! pieces the design doc names as load-bearing from the start:
//! - [`error`]: the centralized error store (one definition per error kind,
//!   with code, message, remediation, and exit code).
//! - [`url`]: the single URL builder (one encoder per context: path segments
//!   vs query values).
//! - [`changelog`]: the `.changes` validation/generation layer — the V1–V12
//!   rules over a package snapshot, and the diff-derived skeleton
//!   entry generator with upstream NEWS extraction.
//! - [`http`]: the HTTP client trait with a bounded timeout, plus a ureq
//!   implementation.
//! - [`auth`]: the credential types and resolution — OBS user/pass
//!   (Basic), Gitea token (Bearer), and the Gitea SSH key — resolved
//!   from environment variables or mounted secret files. No
//!   credentials are stored anywhere here.
//! - [`config`]: the XDG config file (endpoint URLs only, never
//!   credentials) and the secure-write primitive.
//! - [`debug`]: the debug channel — one coherent `--debug` stream of
//!   human-readable lines on stderr (HTTP attempts, retries, library
//!   decision points), with bodies only behind `--debug-body`.
//! - [`obs`]: the OBS operations (one module per cluster, `src/obs/`) a tool uses to move a package update
//!   through the distribution — devel-project lookup, devel version,
//!   open-submit-request check, submit-request filing, superseding,
//!   branch cleanup, branch-project and branch-package creation,
//!   file upload, reading service state and service definitions,
//!   and reading and acting on requests (show, log, search,
//!   accept/decline/revoke, reviews, comments), with dry-run
//!   as the default for mutations.
//! - [`pull_request`]: the uniform pull-request surface — one handle and
//!   one read shape for an OBS submit request and a Gitea pull request.
//! - [`xml`]: the minimal XML document editor for the small documents
//!   geectl rewrites in place (a package `_meta`, a `_link`) —
//!   internal only, so XML never leaves the library.
//! - [`gitea`]: the Gitea operations for git-backed packages — repo
//!   read, fork, repo delete — plus the `branch` composite (Gitea
//!   fork + OBS `cmd=fork`) behind the uniform flow. Internal
//!   plumbing, not the user-facing surface: the uniform flow verbs
//!   (`branch`, `submit`, …) are what an agent calls. The module is
//!   public so the live harness can exercise the typed operations;
//!   the `Gitea*` types never appear in a uniform-flow signature.
//! - [`local_build`]: the local-build vertical — one podman build for
//!   quick debug, with the shared pkg cache, per-root sccache,
//!   and the size-capped eviction policy. The server stays the source
//!   of truth; local is for iterating fast.
//! - [`working_copy`]: the git working copy — checkout, update,
//!   status, diff, add, rm, addremove, commit, pull, and resolve
//!   over one OBS package or one git-backed package, through the
//!   `git` binary.

pub mod auth;
pub mod changelog;
pub mod config;
pub mod debug;
pub mod error;
pub mod gitea;
pub mod http;
pub mod local_build;
pub mod obs;
pub mod pull_request;
pub mod url;
pub mod working_copy;

pub(crate) mod xml;
