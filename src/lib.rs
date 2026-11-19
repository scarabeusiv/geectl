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
//! - [`auth`]: the auth seam — a trait only. The credential design (OBS
//!   user/pass, Gitea token, SSH keys) is done after scaffolding; this trait
//!   is the seam it plugs into. No credentials are stored anywhere here.

pub mod auth;
pub mod error;
pub mod http;
pub mod url;
