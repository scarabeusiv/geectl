//! The OBS/Gitea arm builders shared by the `pull-request` and
//! `person` commands: the configured base URL and environment
//! credentials for each arm, and the one reason table for an
//! unscoped search's skipped arm.
//!
//! The builders above this resolver keep their own credential
//! policies — a dry-run act fails closed without credentials, a
//! merge's forward carries the absence, an unscoped search skips
//! the arm — but they all resolve (base, credentials) the same
//! way, so a change to the resolution, or to the skipped wording,
//! lands once, not once per command.

use crate::auth::{Credentials, EnvGiteaAuth, EnvObsAuth};
use crate::config::{self, DEFAULT_GITEA_URL};
use crate::error::GeectlError;
use crate::person::PersonBackend;
use crate::pull_request::PullRequestBackend;

/// The configured OBS base URL and the environment credentials
/// resolution for it: the resolution itself, unjudged — the
/// caller decides what a failure means. `Ok(Credentials::None)`
/// is "nothing configured", not an error.
pub fn obs_base_and_credentials(
    instance: Option<&str>,
) -> Result<(String, Result<Credentials, GeectlError>), GeectlError> {
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let base = config::obs_base_url(instance, &file_config)?;
    let resolve = match instance {
        Some(alias) => EnvObsAuth::credentials_for_instance(alias, &file_config.instances),
        None => EnvObsAuth::credentials_for_url(&base, &base, &file_config.instances),
    };
    Ok((base, resolve))
}

/// The configured Gitea base URL and the environment token
/// resolution for it: the same unjudged shape as the OBS arm —
/// the caller decides what a failure, or `Credentials::None`,
/// means.
pub fn gitea_base_and_credentials(
    instance: Option<&str>,
) -> Result<(String, Result<Credentials, GeectlError>), GeectlError> {
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    // Unreachable by construction — effective_config always yields
    // Some(gitea_url) — but kept as a defensive default rather than
    // an unwrap: a config change must not turn into a panic here.
    let base = effective
        .config
        .gitea_url
        .as_deref()
        .unwrap_or(DEFAULT_GITEA_URL)
        .to_string();
    let resolve = match instance {
        Some(alias) => EnvGiteaAuth::credentials_for_instance(alias, &file_config.instances),
        None => EnvGiteaAuth::credentials_for_url(&base, &base, &file_config.instances),
    };
    Ok((base, resolve))
}

/// One arm of an unscoped search, for the shared skipped-arm
/// reason table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    /// OBS.
    Obs,
    /// Gitea.
    Gitea,
}

impl Arm {
    /// Why an unscoped search skipped this arm, for the payload
    /// and the text note: the arm had no credentials configured.
    pub fn skipped_reason(&self) -> &'static str {
        match self {
            Arm::Obs => "no OBS credentials configured (GEECTL_OBS_APIKEY[_FILE] or GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD[_FILE])",
            Arm::Gitea => {
                "no Gitea token configured (GEECTL_GITEA_TOKEN or GEECTL_GITEA_TOKEN_FILE)"
            }
        }
    }
}

impl From<PullRequestBackend> for Arm {
    fn from(backend: PullRequestBackend) -> Self {
        match backend {
            PullRequestBackend::Obs => Arm::Obs,
            PullRequestBackend::Gitea => Arm::Gitea,
        }
    }
}

impl From<PersonBackend> for Arm {
    fn from(backend: PersonBackend) -> Self {
        match backend {
            PersonBackend::Obs => Arm::Obs,
            PersonBackend::Gitea => Arm::Gitea,
        }
    }
}
