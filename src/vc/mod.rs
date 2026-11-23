//! The `vc` validation/generation layer for `.changes` entries.
//!
//! Validation is deterministic, with no LLM: V1–V8 and V10–V12 are
//! exact checks, V9 a `needs-detail` heuristic. Generation emits the
//! diff-derived skeleton entry (version line, patch and dependency
//! lists, license lines) plus the upstream NEWS section extracted
//! from the source tarball — extraction only, never distillation.
//! The output is a draft for a human to complete: the layer never
//! files anything, and never silently emits a bare version-only
//! entry (see [`generate::Generation::needs_detail`]).
//!
//! The rules are owned by the packaging skill where they already
//! exist — format and content in
//! `skills/opensuse-packaging/references/changelog-rules.md`,
//! mechanics in `changelog-entry.md`, the mechanical gates in
//! `changes-lint.sh` / `changes-guard.sh` / `changes-patches.sh` —
//! and this module ports the mechanical parts.

pub mod changes;
pub mod generate;
pub mod news;
pub mod spec;
pub mod validate;

pub use validate::{Finding, PackageSnapshot, Rule, Skipped, ValidateOptions, ValidationReport};
