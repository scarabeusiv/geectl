//! geectl — Geeko Controller, a machine-first CLI for agents.
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
//! - [`http`]: the HTTP client trait with a bounded timeout, plus a ureq
//!   implementation.
//! - [`auth`]: the credential types and resolution — OBS user/pass
//!   (Basic), Gitea token (Bearer), and the Gitea SSH key — resolved
//!   from environment variables or mounted secret files. No
//!   credentials are stored anywhere here.
//! - [`config`]: the XDG config file (endpoint URLs only, never
//!   credentials) and the secure-write primitive.
//! - [`obs`]: the OBS operations a tool uses to move a package update
//!   through the distribution — devel-project lookup, devel version,
//!   open-submit-request check, submit-request filing, superseding,
//!   branch cleanup, branch-project and branch-package creation,
//!   file upload, and reading and acting on requests (show, log,
//!   search, accept/decline/revoke, reviews, comments), with dry-run
//!   as the default for mutations.
//! - [`xml`]: the minimal XML document editor for the small documents
//!   geectl rewrites in place (a package `_meta`, a `_link`) —
//!   internal only, so XML never leaves the library.

pub mod auth;
pub mod config;
pub mod error;
pub mod http;
pub mod obs;
pub mod url;

pub(crate) mod xml;
