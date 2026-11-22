//! The centralized error store.
//!
//! Every error geectl can produce is defined here, in one place, with a
//! stable code, a human message, a remediation, and an exit code. There is
//! exactly one exit path (the binary maps the error kind to its exit code);
//! no code anywhere else invents an exit code or a free-text error identity.
//!
//! The store exists so an agent can distinguish programmatically between
//! not-found, auth-failed, server-error, and invalid-input — the same
//! distinction the exit code and the JSON payload both carry. The error
//! path itself is tested: every kind must render to JSON and to Display
//! without panicking.

use serde::Serialize;
use std::fmt;

/// The kinds of error geectl can produce. One variant per failure mode;
/// new failure modes get a new variant here, never a free-text string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The requested resource does not exist (OBS project/package/request).
    NotFound,
    /// Authentication failed (bad credentials, expired token, 401/403).
    AuthFailed,
    /// The server returned an error (5xx, or a 4xx that is the server's
    /// fault, not the caller's).
    ServerError,
    /// The caller's input is invalid (bad package/project name, bad flag).
    InvalidInput,
    /// A resource with that name already exists (a fork, a repo). A
    /// distinct, actionable state: reuse the existing resource, or
    /// pick another name — not an invalid input and not a server
    /// error.
    AlreadyExists,
    /// A request exceeded its bounded timeout.
    Timeout,
    /// A network error that is not a timeout (DNS, connection refused).
    NetworkError,
    /// TLS verification failed. There is no option to disable verification.
    TlsError,
    /// The configuration is invalid or a required secret is missing.
    ConfigError,
    /// An internal error (a bug). The last resort, never the default.
    InternalError,
    /// A local build failed (non-zero engine exit).
    BuildFailed,
    /// A local build exceeded its bounded timeout.
    BuildTimeout,
    /// A build of the same package is already running.
    BuildInProgress,
    /// The build pulls dependencies from a foreign `home:` project.
    UntrustedProject,
    /// The package is multibuild and no flavor was given.
    MultibuildFlavorRequired,
    /// The requested architecture is not the host architecture.
    UnsupportedArch,
    /// The `build` engine binary is not installed.
    NoBuildEngine,
    /// The build image could not be pulled or resolved.
    ImageUnavailable,
    /// A cached package failed its hdrmd5 check after re-download.
    CacheCorrupt,
    /// A destructive operation was refused: the build root is in use.
    BuildRootInUse,
    /// The directory is not a geectl build root (no marker file).
    NotAGeectlBuildRoot,
}

impl ErrorKind {
    /// Stable machine-readable code for this kind.
    pub fn code(self) -> &'static str {
        match self {
            ErrorKind::NotFound => "not_found",
            ErrorKind::AuthFailed => "auth_failed",
            ErrorKind::ServerError => "server_error",
            ErrorKind::InvalidInput => "invalid_input",
            ErrorKind::AlreadyExists => "already_exists",
            ErrorKind::Timeout => "timeout",
            ErrorKind::NetworkError => "network_error",
            ErrorKind::TlsError => "tls_error",
            ErrorKind::ConfigError => "config_error",
            ErrorKind::InternalError => "internal_error",
            ErrorKind::BuildFailed => "build_failed",
            ErrorKind::BuildTimeout => "build_timeout",
            ErrorKind::BuildInProgress => "build_in_progress",
            ErrorKind::UntrustedProject => "untrusted_project",
            ErrorKind::MultibuildFlavorRequired => "multibuild_flavor_required",
            ErrorKind::UnsupportedArch => "unsupported_arch",
            ErrorKind::NoBuildEngine => "no_build_engine",
            ErrorKind::ImageUnavailable => "image_unavailable",
            ErrorKind::CacheCorrupt => "cache_corrupt",
            ErrorKind::BuildRootInUse => "build_root_in_use",
            ErrorKind::NotAGeectlBuildRoot => "not_a_geectl_build_root",
        }
    }

    /// What happened, in one sentence.
    pub fn message(self) -> &'static str {
        match self {
            ErrorKind::NotFound => "the requested resource does not exist",
            ErrorKind::AuthFailed => "authentication failed",
            ErrorKind::ServerError => "the server returned an error",
            ErrorKind::InvalidInput => "the input is invalid",
            ErrorKind::AlreadyExists => "the resource already exists",
            ErrorKind::Timeout => "the request timed out",
            ErrorKind::NetworkError => "a network error occurred",
            ErrorKind::TlsError => "TLS verification failed",
            ErrorKind::ConfigError => "the configuration is invalid",
            ErrorKind::InternalError => "an internal error occurred",
            ErrorKind::BuildFailed => "the local build failed",
            ErrorKind::BuildTimeout => "the local build timed out",
            ErrorKind::BuildInProgress => "a build of this package is already running",
            ErrorKind::UntrustedProject => "the build pulls dependencies from an untrusted project",
            ErrorKind::MultibuildFlavorRequired => {
                "the package is multibuild and no flavor was given"
            }
            ErrorKind::UnsupportedArch => "the requested architecture is not the host architecture",
            ErrorKind::NoBuildEngine => "the build engine is not installed",
            ErrorKind::ImageUnavailable => "the build image could not be pulled or resolved",
            ErrorKind::CacheCorrupt => "a cached package failed its hdrmd5 check",
            ErrorKind::BuildRootInUse => "the build root is in use",
            ErrorKind::NotAGeectlBuildRoot => "the directory is not a geectl build root",
        }
    }

    /// What the caller should do about it.
    pub fn remediation(self) -> &'static str {
        match self {
            ErrorKind::NotFound => "check the project, package, or request name and retry",
            ErrorKind::AuthFailed => {
                "check the credentials (environment variables or mounted secret file) and retry"
            }
            ErrorKind::ServerError => "retry later; if it persists, the server is at fault",
            ErrorKind::InvalidInput => "fix the input and retry; see --help for the expected form",
            ErrorKind::AlreadyExists => {
                "reuse the existing resource, or pick another name and retry"
            }
            ErrorKind::Timeout => "retry; if it persists, the server may be slow or unreachable",
            ErrorKind::NetworkError => "check the network and the server address, then retry",
            ErrorKind::TlsError => {
                "do not disable verification; check the server certificate and the trust store"
            }
            ErrorKind::ConfigError => {
                "check the configuration (flag, environment variable, or config file) and retry"
            }
            ErrorKind::InternalError => "this is a bug; report it with the error code",
            ErrorKind::BuildFailed => {
                "read the build log at the recorded path; fix the failure and retry, passing --clean after a failure"
            }
            ErrorKind::BuildTimeout => {
                "raise the timeout for big packages; the partial log is preserved at the recorded path"
            }
            ErrorKind::BuildInProgress => {
                "wait for the running build to finish, then retry"
            }
            ErrorKind::UntrustedProject => {
                "only the caller's own home: project and non-home: projects are trusted; a foreign home: project needs the user's decision"
            }
            ErrorKind::MultibuildFlavorRequired => {
                "pass one flavor; one invocation builds one flavor"
            }
            ErrorKind::UnsupportedArch => {
                "build locally only for the host architecture; other architectures go to the server"
            }
            ErrorKind::NoBuildEngine => {
                "install the openSUSE build package (zypper install build) or put the build script on PATH"
            }
            ErrorKind::ImageUnavailable => {
                "check the network and the image name, then retry"
            }
            ErrorKind::CacheCorrupt => {
                "the file is re-downloaded once, then refused; delete the cache file and retry"
            }
            ErrorKind::BuildRootInUse => {
                "wait for the running build, then retry the destructive operation"
            }
            ErrorKind::NotAGeectlBuildRoot => {
                "pass a directory created by a geectl build (it carries the marker file)"
            }
        }
    }

    /// The process exit code for this kind. Distinct per kind, so a caller
    /// can implement retry-on-transient vs fail-fast-on-input policy.
    pub fn exit_code(self) -> i32 {
        match self {
            ErrorKind::InternalError => 1,
            ErrorKind::InvalidInput => 2,
            ErrorKind::NotFound => 3,
            ErrorKind::AuthFailed => 4,
            ErrorKind::Timeout => 5,
            ErrorKind::ServerError => 6,
            ErrorKind::NetworkError => 7,
            ErrorKind::TlsError => 8,
            ErrorKind::ConfigError => 9,
            ErrorKind::AlreadyExists => 10,
            ErrorKind::BuildFailed => 11,
            ErrorKind::BuildTimeout => 12,
            ErrorKind::BuildInProgress => 13,
            ErrorKind::UntrustedProject => 14,
            ErrorKind::MultibuildFlavorRequired => 15,
            ErrorKind::UnsupportedArch => 16,
            ErrorKind::NoBuildEngine => 17,
            ErrorKind::ImageUnavailable => 18,
            ErrorKind::CacheCorrupt => 19,
            ErrorKind::BuildRootInUse => 20,
            ErrorKind::NotAGeectlBuildRoot => 21,
        }
    }

    /// Every kind, for the exhaustive render test.
    pub fn all() -> &'static [ErrorKind] {
        &[
            ErrorKind::NotFound,
            ErrorKind::AuthFailed,
            ErrorKind::ServerError,
            ErrorKind::InvalidInput,
            ErrorKind::AlreadyExists,
            ErrorKind::Timeout,
            ErrorKind::NetworkError,
            ErrorKind::TlsError,
            ErrorKind::ConfigError,
            ErrorKind::InternalError,
            ErrorKind::BuildFailed,
            ErrorKind::BuildTimeout,
            ErrorKind::BuildInProgress,
            ErrorKind::UntrustedProject,
            ErrorKind::MultibuildFlavorRequired,
            ErrorKind::UnsupportedArch,
            ErrorKind::NoBuildEngine,
            ErrorKind::ImageUnavailable,
            ErrorKind::CacheCorrupt,
            ErrorKind::BuildRootInUse,
            ErrorKind::NotAGeectlBuildRoot,
        ]
    }
}

/// A geectl error: a kind from the store plus the context of this instance
/// (what resource, what request). The context is data, never a second
/// error identity — callers match on `kind`, not on the context string.
#[derive(Debug, Clone)]
pub struct GeectlError {
    pub kind: ErrorKind,
    /// Context for this instance (e.g. the URL, the package name). May be
    /// empty. Never contains a credential. A malformed server response
    /// may carry a bounded, redacted preview of what was received
    /// (see `debug::format_body_preview`), so the failure is
    /// diagnosable; never an unbounded document.
    /// `to_json()` omits it when empty; there is no serde field attribute
    /// here because `Serialize` is implemented manually (see below).
    pub context: String,
}

impl GeectlError {
    pub fn new(kind: ErrorKind, context: impl Into<String>) -> Self {
        GeectlError {
            kind,
            context: context.into(),
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn exit_code(&self) -> i32 {
        self.kind.exit_code()
    }

    /// The JSON payload written to stdout on failure. The shape is stable:
    /// `{"error": {"code": ..., "message": ..., "remediation": ...,
    /// "context": ...}}`. `context` is omitted when empty.
    pub fn to_json(&self) -> serde_json::Value {
        let mut error = serde_json::json!({
            "code": self.kind.code(),
            "message": self.kind.message(),
            "remediation": self.kind.remediation(),
        });
        if !self.context.is_empty() {
            error["context"] = serde_json::Value::String(self.context.clone());
        }
        serde_json::json!({ "error": error })
    }
}

/// `Serialize` delegates to [`GeectlError::to_json`], so the serde
/// form is the same stable shape as the JSON payload on stdout
/// (`{"error": {"code", "message", "remediation", "context"}}`). A
/// derived `Serialize` would instead emit `{"kind", "context"}` and
/// diverge from the contract.
impl Serialize for GeectlError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_json().serialize(serializer)
    }
}

impl fmt::Display for GeectlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.context.is_empty() {
            write!(f, "{}: {}", self.kind.code(), self.kind.message())
        } else {
            write!(
                f,
                "{}: {} ({})",
                self.kind.code(),
                self.kind.message(),
                self.context
            )
        }
    }
}

impl std::error::Error for GeectlError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_renders_without_panicking() {
        // The error path must work when everything else is already broken.
        for kind in ErrorKind::all() {
            let err = GeectlError::new(*kind, "test context");
            let _ = err.to_string();
            let json = err.to_json();
            assert_eq!(json["error"]["code"], kind.code());
            assert_eq!(json["error"]["message"], kind.message());
            assert_eq!(json["error"]["remediation"], kind.remediation());
            assert_eq!(json["error"]["context"], "test context");
            assert!(kind.exit_code() > 0);
        }
    }

    #[test]
    fn empty_context_is_omitted_from_json() {
        let err = GeectlError::new(ErrorKind::NotFound, "");
        let json = err.to_json();
        assert!(json["error"].get("context").is_none());
        assert_eq!(
            err.to_string(),
            "not_found: the requested resource does not exist"
        );
    }

    #[test]
    fn serialize_matches_to_json() {
        // The serde form must be the same shape as to_json(): a
        // derived Serialize would emit {"kind", "context"} instead.
        let err = GeectlError::new(ErrorKind::NotFound, "test context");
        let via_serde: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&err).unwrap()).unwrap();
        assert_eq!(via_serde, err.to_json());
        // And the empty-context omission holds in both forms.
        let err = GeectlError::new(ErrorKind::NotFound, "");
        let via_serde: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&err).unwrap()).unwrap();
        assert_eq!(via_serde, err.to_json());
    }

    #[test]
    fn exit_codes_are_distinct() {
        let mut codes: Vec<i32> = ErrorKind::all().iter().map(|k| k.exit_code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), ErrorKind::all().len());
    }
}
