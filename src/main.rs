//! geectl — Geeko Controller, a CLI for openSUSE packaging workflows.
//!
//! Non-interactive is the only mode: no prompts, no editor, no pager.
//! Missing required input fails fast with a structured error naming the
//! missing flag. Stdout carries only the result payload (JSON by default);
//! every diagnostic, warning, and error goes to stderr. There is exactly
//! one exit path: the error kind's exit code.

use clap::Parser;
use geectl::auth::{
    load_dotenv, obs_alias_vars, same_api_url, AuthProvider, Credentials, EnvGiteaAuth, EnvObsAuth,
    GiteaSshKey,
};
use geectl::cli::{Cli, Command, ConfigCommand, Format};
use geectl::config;
use geectl::debug::Debug;
use geectl::error::{ErrorKind, GeectlError};
use geectl::obs::OBS_API_BASE;
use geectl::oscrc;
use serde_json::json;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

fn run(
    cli: Cli,
    debug: &Debug,
    format: Format,
    format_source: &'static str,
) -> Result<String, GeectlError> {
    let (payload, text) = match cli.command {
        Some(Command::Version) | None => {
            let payload = json!({
                "name": "geectl",
                "version": env!("CARGO_PKG_VERSION"),
            });
            let text = format!("geectl {}", env!("CARGO_PKG_VERSION"));
            (payload, text)
        }
        Some(Command::Request { command }) => geectl::cli_request::run(*command, debug, cli.live)?,
        Some(Command::Build { command }) => geectl::cli_build::run_build(debug, command, cli.live)?,
        Some(Command::LocalBuild { command }) => {
            geectl::cli_local_build::run_local_build(debug, command, cli.live)?
        }
        Some(Command::Config { command }) => match command {
            ConfigCommand::Show => {
                let payload = config_show_payload(format, format_source)?;
                let text = render_config_show_text(&payload);
                (payload, text)
            }
            ConfigCommand::Init => {
                let payload = config_init_payload()?;
                let text = render_config_init_text(&payload);
                (payload, text)
            }
            ConfigCommand::ImportOscrc {
                oscrc,
                apiurl,
                alias,
                force,
            } => {
                let payload = config_import_oscrc_payload(
                    oscrc.as_deref(),
                    apiurl.as_deref(),
                    alias.as_deref(),
                    force,
                    cli.live,
                )?;
                let text = render_config_import_oscrc_text(&payload);
                (payload, text)
            }
        },
        Some(Command::Remote { command }) => {
            geectl::cli_remote::run_remote(command, debug, cli.live)?
        }
        Some(Command::Branch { command }) => {
            geectl::cli_branch::run_branch(*command, debug, cli.live)?
        }
        Some(Command::Maintenance { command }) => {
            let (payload, text) = geectl::maintenance::run(command, debug, cli.live)?;
            (payload, text)
        }
        Some(Command::Service { command }) => {
            geectl::cli_service::run_service(command, debug, cli.live)?
        }
        Some(Command::Gitea { command }) => geectl::cli_gitea::run_gitea(command, debug, cli.live)?,
        Some(Command::PullRequest { command }) => {
            geectl::cli_pull_request::run_pull_request(command, debug, cli.live)?
        }
        Some(Command::Person { command }) => {
            geectl::cli_person::run_person(command, debug, cli.live)?
        }
        Some(Command::Meta { command }) => geectl::cli_meta::run_meta(command, debug, cli.live)?,
        Some(Command::Changelog { command }) => {
            geectl::cli_changelog::run_changelog(command, debug, cli.live)?
        }
        Some(Command::Checkout {
            project,
            package,
            rev,
        }) => {
            geectl::cli_working_copy::checkout(&project, &package, rev.as_deref(), debug, cli.live)?
        }
        Some(Command::Update { rev }) => {
            geectl::cli_working_copy::update(rev.as_deref(), debug, cli.live)?
        }
        Some(Command::Status) => geectl::cli_working_copy::status(debug)?,
        Some(Command::Diff) => geectl::cli_working_copy::diff(debug)?,
        Some(Command::Add { files }) => geectl::cli_working_copy::add(&files, debug, cli.live)?,
        Some(Command::Rm { files }) => geectl::cli_working_copy::rm(&files, debug, cli.live)?,
        Some(Command::Addremove) => geectl::cli_working_copy::addremove(debug, cli.live)?,
        Some(Command::Commit { message, noservice }) => {
            geectl::cli_working_copy::commit(&message, noservice, debug, cli.live)?
        }
        Some(Command::Pull) => geectl::cli_working_copy::pull(debug, cli.live)?,
        Some(Command::Resolve) => geectl::cli_working_copy::resolve(debug, cli.live)?,
        Some(Command::Prjconf { command }) => {
            geectl::cli_prjconf::run_prjconf(command, debug, cli.live)?
        }
    };
    Ok(match format {
        Format::Json => payload.to_string(),
        Format::Text => text,
    })
}

/// Resolve the output format: the `--format` flag wins, then the
/// `GEECTL_FORMAT` environment variable, then the config file's
/// `format` key, then `json`. Returns the format and where it came
/// from (`"flag"` / `"environment"` / `"config"` / `"default"`) for
/// `config show`. Resolving loads the config file, so a malformed
/// config or an invalid `GEECTL_FORMAT` fails even `version` —
/// fail-fast on misconfiguration, by design — unless the flag is
/// given, which returns before the config is read.
fn resolve_format(flag: Option<Format>) -> Result<(Format, &'static str), GeectlError> {
    if let Some(format) = flag {
        return Ok((format, "flag"));
    }
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    // effective_config validates the value, so this match is
    // exhaustive by construction; the fallback is only for the type,
    // never expected to fire.
    let format = match effective.config.format.as_deref() {
        Some("json") => Format::Json,
        Some("text") => Format::Text,
        _ => Format::Json,
    };
    Ok((format, effective.format_source))
}

/// Resolve the debug channel from the `--debug` / `--debug-body`
/// flags and the `GEECTL_DEBUG` environment variable. The flag wins
/// over the environment; `--debug-body` implies `--debug`. Only the
/// exact value `1` enables the channel via the environment; unset,
/// empty, or anything else disables it. Returns `(enabled, bodies)`.
fn resolve_debug(debug_flag: bool, debug_body_flag: bool, env: Option<&str>) -> (bool, bool) {
    let enabled = debug_flag || debug_body_flag || env == Some("1");
    (enabled, debug_body_flag)
}

/// Collect argv without panicking on non-UTF-8 arguments:
/// `std::env::args()` panics on them, so go through `args_os()`
/// and convert lossily (an invalid sequence becomes U+FFFD rather
/// than crashing the run before it starts).
fn lossy_argv(args: impl Iterator<Item = std::ffi::OsString>) -> Vec<String> {
    args.map(|arg| arg.to_string_lossy().into_owned()).collect()
}

/// The startup line for the debug channel: `geectl <version>` plus
/// the arguments as invoked (skipping argv[0], the binary path).
/// There is no secret-valued flag today; a future one must be masked
/// here, by construction, before it reaches this line.
fn startup_line(argv: &[String]) -> String {
    let mut parts = vec!["geectl".to_string(), env!("CARGO_PKG_VERSION").to_string()];
    parts.extend(argv.iter().skip(1).cloned());
    parts.join(" ")
}

/// The OBS auth kind for the debug config line: the per-URL
/// resolution (alias, else plain for the default) — the plain-only
/// provider would report `none` for an alias-only setup while every
/// command authenticates fine, the same staleness
/// [`obs_auth_status`] fixes for `config show`. `basic`, `bearer`,
/// `ssh`, `none`, or `error`. Never a username, never a value — the
/// channel is stricter than `config show`.
fn obs_auth_kind(url: &str, instances: &[(String, String)]) -> &'static str {
    match EnvObsAuth::credentials_for_url(url, url, instances) {
        Ok(Credentials::None) => "none",
        Ok(Credentials::Basic { .. }) => "basic",
        Ok(Credentials::Bearer { .. }) => "bearer",
        Ok(Credentials::SshKey { .. }) => "ssh",
        Err(_) => "error",
    }
}

/// The Gitea auth kind for the debug config line. Gitea has two
/// providers, so the kind prefers bearer, then ssh, then error,
/// else none: a bearer token wins over a key, and a key wins over a
/// misconfiguration in the other provider. The bearer is the one
/// the configured URL would use — the matching alias's variables
/// when `gitea_url` is named in `[instances]`, the plain variables
/// otherwise — so an alias-only setup does not read as
/// unauthenticated here.
fn gitea_auth_kind(gitea_url: &str, instances: &[(String, String)]) -> &'static str {
    let bearer = EnvGiteaAuth::credentials_for_url(gitea_url, gitea_url, instances);
    let key = GiteaSshKey.credentials();
    if matches!(bearer, Ok(Credentials::Bearer { .. })) {
        "bearer"
    } else if matches!(key, Ok(Credentials::SshKey { .. })) {
        "ssh"
    } else if bearer.is_err() || key.is_err() {
        "error"
    } else {
        "none"
    }
}

/// The config line for the debug channel: the effective OBS and
/// Gitea base hosts with their auth kinds. None on any config-load
/// failure, so a debug run never fails for want of a config line —
/// including `version`, which does not otherwise touch config.
fn debug_config_line() -> Option<String> {
    let path = config::config_path().ok()?;
    let file_config = config::load_config_at(&path).ok()?;
    let effective = config::effective_config(&file_config).ok()?;
    // effective_config resolves env > file > default, so both URLs
    // are present by construction; the fallback is only for the
    // type, never expected to fire.
    let obs_url = effective.config.obs_url.as_deref().unwrap_or("");
    let gitea_url = effective.config.gitea_url.as_deref().unwrap_or("");
    Some(format!(
        "config: obs {} (auth: {}), gitea {} (auth: {})",
        geectl::debug::url_host(obs_url),
        obs_auth_kind(obs_url, &file_config.instances),
        geectl::debug::url_host(gitea_url),
        gitea_auth_kind(gitea_url, &file_config.instances)
    ))
}

/// The `config show` payload: the effective endpoint URLs with the
/// source of each value (environment / config / default), the
/// effective output format with its source (flag / environment /
/// config / default), the named-instance table, and the credential
/// status for each provider. The `[build]` tunables ride along too,
/// with their source (config / default / error): the effective values
/// a local build would use, or `null` with source `error` when the
/// dir cannot be resolved. A
/// status is never a secret: Basic auth shows the username only,
/// Bearer shows "configured" only, and an SSH key shows its path.
fn config_show_payload(
    format: Format,
    format_source: &'static str,
) -> Result<serde_json::Value, GeectlError> {
    // The config path is resolved once and passed to the load: a
    // second `config_path()` call here could diverge from the one
    // inside `load_config()` if the environment changed in between.
    // The URL sources come from `effective_config` itself — they are
    // computed with the values, not re-derived by re-reading the
    // environment.
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    // The `[build]` tunables come from the file config, NOT from
    // `effective_config`: it hardcodes both to `None`, and a local
    // build reads the file config plus the defaults instead. The
    // payload carries the effective value either way, so `config
    // show` reports what a build would actually use. A dir that
    // cannot be resolved, or is not valid UTF-8, reports `null`
    // with source `error` instead of failing the whole command.
    // A configured relative dir is absolutized the same way the
    // build absolutizes it.
    let (build_cache_dir, build_cache_dir_source) = match &file_config.build_cache_dir {
        Some(dir) => {
            let absolute =
                std::path::absolute(dir).unwrap_or_else(|_| std::path::PathBuf::from(dir));
            match absolute.to_str() {
                Some(dir) => (json!(dir), "config"),
                None => (json!(null), "error"),
            }
        }
        None => match geectl::local_build::default_cache_dir() {
            Ok(dir) => match dir.to_str() {
                Some(dir) => (json!(dir), "default"),
                None => (json!(null), "error"),
            },
            Err(_) => (json!(null), "error"),
        },
    };
    let (build_cache_cap_gib, build_cache_cap_gib_source) = match file_config.build_cache_cap_gib {
        Some(cap) => (cap, "config"),
        None => (geectl::local_build::DEFAULT_CACHE_CAP_GIB, "default"),
    };
    let obs_url = effective.config.obs_url.as_deref().unwrap_or(OBS_API_BASE);
    // The effective Gitea URL is the configured default for the
    // single configured URL, so `gitea_auth` reports the
    // credentials that URL would actually use — the matching
    // alias's variables when the URL is named in `[instances]`,
    // the plain variables otherwise — not the plain-only status.
    // (Computed before the payload: the borrow of the effective
    // URL ends here, and the payload moves it below.)
    let gitea_url = effective.config.gitea_url.as_deref().unwrap_or("");
    let gitea_auth = credentials_status(EnvGiteaAuth::credentials_for_url(
        gitea_url,
        gitea_url,
        &effective.config.instances,
    ));
    let mut instances_json = serde_json::Map::new();
    for (alias, url) in &effective.config.instances {
        instances_json.insert(alias.clone(), json!(url));
    }
    Ok(json!({
        "config_file": path.to_string_lossy(),
        "obs_url": effective.config.obs_url,
        "obs_url_source": effective.obs_url_source,
        "gitea_url": effective.config.gitea_url,
        "gitea_url_source": effective.gitea_url_source,
        "format": format.as_str(),
        "format_source": format_source,
        "build_cache_dir": build_cache_dir,
        "build_cache_dir_source": build_cache_dir_source,
        "build_cache_cap_gib": build_cache_cap_gib,
        "build_cache_cap_gib_source": build_cache_cap_gib_source,
        "instances": instances_json,
        "obs_auth": obs_auth_status(obs_url, &effective.config.instances),
        "gitea_auth": gitea_auth,
        "gitea_ssh_key": auth_status(&GiteaSshKey),
    }))
}

/// The OBS credential status for one URL: the matching instance
/// alias's variables win, else the plain variables when it is the
/// configured default — so `config show` reports what the commands
/// would actually use, instead of the plain variables only.
fn obs_auth_status(url: &str, instances: &[(String, String)]) -> serde_json::Value {
    credentials_status(EnvObsAuth::credentials_for_url(url, url, instances))
}

/// The credential status as JSON: `{"status": "configured"}` plus,
/// for Basic auth, the username (never the password); for an SSH key,
/// the path; for a misconfiguration, `{"status": "error", "message":
/// ...}` naming the variable, never the value.
fn auth_status(provider: &dyn AuthProvider) -> serde_json::Value {
    credentials_status(provider.credentials())
}

/// The credential status as JSON for an already-resolved result
/// — the same shape as [`auth_status`], for credentials resolved
/// per URL instead of through a provider.
fn credentials_status(credentials: Result<Credentials, GeectlError>) -> serde_json::Value {
    match credentials {
        Ok(Credentials::None) => json!({"status": "not_configured"}),
        Ok(Credentials::Basic { username, .. }) => {
            json!({"status": "configured", "username": username})
        }
        Ok(Credentials::Bearer { .. }) => json!({"status": "configured"}),
        Ok(Credentials::SshKey { path }) => {
            json!({"status": "configured", "path": path.to_string_lossy()})
        }
        Err(e) => json!({
            "status": "error",
            "code": e.kind().code(),
            "message": e.context,
            "remediation": e.kind().remediation(),
        }),
    }
}

/// The `config init` payload: the path created. The file itself is
/// written by `write_config_template`, which refuses to overwrite.
fn config_init_payload() -> Result<serde_json::Value, GeectlError> {
    let path = config::config_path()?;
    config::write_config_template(&path)?;
    Ok(json!({
        "config_file": path.to_string_lossy(),
        "created": true,
        "note": "Edit the endpoint URLs as needed. Credentials are never stored here — use the GEECTL_* environment variables.",
    }))
}

fn render_config_show_text(payload: &serde_json::Value) -> String {
    [
        format!(
            "config file: {}",
            payload["config_file"].as_str().unwrap_or("")
        ),
        format!(
            "obs_url: {} ({})",
            payload["obs_url"].as_str().unwrap_or(""),
            payload["obs_url_source"].as_str().unwrap_or("")
        ),
        format!(
            "gitea_url: {} ({})",
            payload["gitea_url"].as_str().unwrap_or(""),
            payload["gitea_url_source"].as_str().unwrap_or("")
        ),
        format!(
            "format: {} ({})",
            payload["format"].as_str().unwrap_or(""),
            payload["format_source"].as_str().unwrap_or("")
        ),
        format!(
            "build_cache_dir: {} ({})",
            payload["build_cache_dir"].as_str().unwrap_or("-"),
            payload["build_cache_dir_source"].as_str().unwrap_or("")
        ),
        format!(
            "build_cache_cap_gib: {} GiB ({})",
            payload["build_cache_cap_gib"].as_u64().unwrap_or(0),
            payload["build_cache_cap_gib_source"].as_str().unwrap_or("")
        ),
        format!(
            "instances: {}",
            render_instances_text(&payload["instances"])
        ),
        format!("obs_auth: {}", render_auth_text(&payload["obs_auth"])),
        format!("gitea_auth: {}", render_auth_text(&payload["gitea_auth"])),
        format!(
            "gitea_ssh_key: {}",
            render_auth_text(&payload["gitea_ssh_key"])
        ),
    ]
    .join("\n")
}

/// The `[instances]` table as text: `alias -> url` pairs sorted by
/// alias, or `none` when the table is empty.
fn render_instances_text(instances: &serde_json::Value) -> String {
    match instances.as_object() {
        Some(map) if !map.is_empty() => map
            .iter()
            .map(|(alias, url)| format!("{alias} -> {}", url.as_str().unwrap_or("")))
            .collect::<Vec<_>>()
            .join(", "),
        _ => "none".to_string(),
    }
}

fn render_auth_text(auth: &serde_json::Value) -> String {
    match auth["status"].as_str().unwrap_or("") {
        "configured" => {
            if let Some(user) = auth.get("username").and_then(|v| v.as_str()) {
                format!("configured (username {user})")
            } else if let Some(path) = auth.get("path").and_then(|v| v.as_str()) {
                format!("configured ({path})")
            } else {
                "configured".to_string()
            }
        }
        "error" => format!("error: {}", auth["message"].as_str().unwrap_or("")),
        _ => "not_configured".to_string(),
    }
}

fn render_config_init_text(payload: &serde_json::Value) -> String {
    format!("created {}", payload["config_file"].as_str().unwrap_or(""))
}

/// The `config import-oscrc` payload: the oscrc sections found
/// (apiurl + username, never a password), the selected section, the
/// alias and URL written (or planned), and the env lines to set. The
/// username is reported because it is not a secret (`config show`
/// reports usernames too); the password is never stored, never
/// printed, and never read into a value — only its variable name is
/// reported.
fn config_import_oscrc_payload(
    oscrc: Option<&Path>,
    apiurl: Option<&str>,
    alias: Option<&str>,
    force: bool,
    live: bool,
) -> Result<serde_json::Value, GeectlError> {
    let oscrc_path = match oscrc {
        Some(path) => path.to_path_buf(),
        None => oscrc::default_oscrc_path()?,
    };
    let text = oscrc::read_oscrc(&oscrc_path)?;
    let sections = oscrc::parse_oscrc(&text, &oscrc_path)?;
    let importable: Vec<oscrc::OscrcSection> = sections
        .iter()
        .filter(|s| s.username.is_some())
        .cloned()
        .collect();
    let selected = match apiurl {
        Some(wanted) => {
            let found = sections
                .iter()
                .find(|s| same_api_url(&s.apiurl, wanted).unwrap_or(false));
            match found {
                None => {
                    return Err(GeectlError::new(
                        ErrorKind::ConfigError,
                        format!(
                            "{}: no oscrc section for {wanted}; found: {}",
                            oscrc_path.display(),
                            list_apiurls(&sections),
                        ),
                    ));
                }
                Some(section) if section.username.is_none() => {
                    return Err(GeectlError::new(
                        ErrorKind::ConfigError,
                        format!(
                            "{}: oscrc section {wanted} has no username; add a `user` line to it or pick another section",
                            oscrc_path.display()
                        ),
                    ));
                }
                Some(section) => section.clone(),
            }
        }
        None => match importable.as_slice() {
            [only] => only.clone(),
            [] => {
                return Err(GeectlError::new(
                    ErrorKind::ConfigError,
                    format!(
                        "{}: no oscrc section carries a username; nothing to import",
                        oscrc_path.display()
                    ),
                ));
            }
            _ => {
                return Err(GeectlError::new(
                    ErrorKind::ConfigError,
                    format!(
                        "{}: more than one oscrc section carries a username; pass --apiurl to pick one: {}",
                        oscrc_path.display(),
                        list_apiurls(&importable),
                    ),
                ));
            }
        },
    };
    // The selection guarantees a username: every path above returns
    // an importable section or errors.
    let username = selected.username.clone().unwrap_or_default();
    let alias = match alias {
        Some(alias) => alias.to_string(),
        None => config::alias_from_url(&selected.apiurl)?,
    };
    let (username_var, password_var, _, _, _) = obs_alias_vars(&alias);
    let config_path = config::config_path()?;
    let outcome = if live {
        config::add_instance(&config_path, &alias, &selected.apiurl, force)?
    } else {
        config::check_add_instance(&config_path, &alias, &selected.apiurl, force)?
    };
    let outcome_str = match outcome {
        config::AddInstance::Added => "added",
        config::AddInstance::Replaced => "replaced",
        config::AddInstance::AlreadyPresent => "already_present",
    };
    let mut sections_json = Vec::new();
    for section in &sections {
        sections_json.push(json!({
            "apiurl": section.apiurl,
            "username": section.username,
        }));
    }
    Ok(json!({
        "config_file": config_path.to_string_lossy(),
        "oscrc_file": oscrc_path.to_string_lossy(),
        "live": live,
        "written": live && outcome != config::AddInstance::AlreadyPresent,
        "outcome": outcome_str,
        "alias": alias,
        "apiurl": selected.apiurl,
        "username": username,
        "username_var": username_var,
        "password_var": password_var,
        "set_username": format!("{username_var}={username}"),
        "sections": sections_json,
    }))
}

/// The apiurls of oscrc sections, comma-separated, for error
/// messages. Apiurls are not secrets.
fn list_apiurls(sections: &[oscrc::OscrcSection]) -> String {
    sections
        .iter()
        .map(|s| s.apiurl.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_config_import_oscrc_text(payload: &serde_json::Value) -> String {
    let sections = payload["sections"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|s| {
                    let apiurl = s["apiurl"].as_str().unwrap_or("");
                    match s["username"].as_str() {
                        Some(user) => format!("{apiurl} (username {user})"),
                        None => format!("{apiurl} (no username)"),
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let outcome = payload["outcome"].as_str().unwrap_or("");
    let live = payload["live"].as_bool().unwrap_or(false);
    let action = match (live, outcome) {
        (true, "added") => "added".to_string(),
        (true, "replaced") => "replaced".to_string(),
        (true, "already_present") => "already present".to_string(),
        (false, "added") => "would add (dry run; pass --live to write)".to_string(),
        (false, "replaced") => "would replace (dry run; pass --live to write)".to_string(),
        (false, "already_present") => "already present".to_string(),
        _ => outcome.to_string(),
    };
    [
        format!("oscrc: {}", payload["oscrc_file"].as_str().unwrap_or("")),
        format!("found: {sections}"),
        format!(
            "import: {} -> {} (username {}), {action} in {}",
            payload["alias"].as_str().unwrap_or(""),
            payload["apiurl"].as_str().unwrap_or(""),
            payload["username"].as_str().unwrap_or(""),
            payload["config_file"].as_str().unwrap_or("")
        ),
        format!(
            "set: export {}",
            payload["set_username"].as_str().unwrap_or("")
        ),
        format!(
            "set: export {}=... (set this yourself; geectl never stores the password)",
            payload["password_var"].as_str().unwrap_or("")
        ),
    ]
    .join("\n")
}

/// Write a payload to stdout, terminated with a trailing newline
/// (an empty payload writes nothing).
/// A closed pipe (`BrokenPipe`, e.g. the output piped to `head`) is
/// not an error — the consumer just stopped reading. Any other write
/// failure is an internal error. `println!` would panic on either;
/// this is the single stdout writer.
fn write_payload(payload: &str) -> Result<(), GeectlError> {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    match write!(handle, "{}", terminated(payload)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(GeectlError::new(ErrorKind::InternalError, e.to_string())),
    }
}

/// Terminate a payload with a trailing newline: a payload that does
/// not end with one gains one; an already-terminated payload is
/// returned untouched (so a `cat` file ending in two newlines keeps
/// both — verbatim wins over tidiness). JSON payloads never end with
/// a newline. An empty payload stays empty, so `remote cat` of an
/// empty file prints zero bytes instead of one newline.
fn terminated(payload: &str) -> String {
    if payload.is_empty() || payload.ends_with('\n') {
        payload.to_string()
    } else {
        format!("{payload}\n")
    }
}

/// Write an error to stderr, in the selected format: the JSON error
/// payload by default, `error:` + `remediation:` lines in text mode.
/// A failure to write to stderr is ignored: there is nowhere else to
/// report it, and panicking while already handling an error helps no
/// one.
fn write_error(err: &GeectlError, format: Format) {
    let stderr = std::io::stderr();
    let mut handle = stderr.lock();
    let rendered = match format {
        Format::Json => err.to_json().to_string(),
        Format::Text => render_error_text(err),
    };
    let _ = writeln!(handle, "{rendered}");
}

/// The text rendering of an error: `error: <message>` plus
/// `remediation: <...>` lines. The message is the error's context
/// when it names the failure, else the kind's one-sentence message.
/// The exit code is unchanged.
fn render_error_text(err: &GeectlError) -> String {
    let message = if err.context.is_empty() {
        err.kind.message().to_string()
    } else {
        err.context.clone()
    };
    format!(
        "error: {message}
remediation: {}",
        err.kind.remediation()
    )
}

fn main() -> ExitCode {
    // The Windows main thread carries a 1 MiB stack, and the CLI
    // parse in an unoptimized build needs more than that (measured
    // 1.0-1.5 MiB for `--version` on arm64 debug, over 1 MiB on
    // x86_64 debug). Run the real
    // entry on a thread with an explicit stack; 32 MiB is virtual
    // reservation only, committed on demand. The thread is named
    // `main` so panic reports keep naming it, and a panic resumes
    // on the joining thread so the exit behaviour is unchanged.
    // A thread that cannot start leaves no other way to run: run
    // inline on the current stack instead of failing outright.
    const STACK_SIZE: usize = 32 * 1024 * 1024;
    match std::thread::Builder::new()
        .name("main".to_string())
        .stack_size(STACK_SIZE)
        .spawn(real_main)
    {
        Ok(child) => match child.join() {
            Ok(code) => code,
            Err(payload) => std::panic::resume_unwind(payload),
        },
        Err(err) => {
            eprintln!("geectl: cannot start the main thread ({err}); running inline");
            real_main()
        }
    }
}

fn real_main() -> ExitCode {
    // Parse errors go through the single error path, not clap's
    // default (usage on stderr, exit 2, no JSON). Help and version are
    // the exception: they print clap's text to stdout and exit 0.
    // The argv is collected once, lossily, for the debug startup line
    // below. Parsing itself uses the raw `OsString` argv, so a
    // non-UTF-8 argument still gets clap's invalid-UTF-8 error instead
    // of being silently replaced by U+FFFD and sent on as a value.
    let argv = lossy_argv(std::env::args_os());
    let cli = match Cli::try_parse_from(std::env::args_os()) {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind as ClapErrorKind;
            match e.kind() {
                ClapErrorKind::DisplayHelp | ClapErrorKind::DisplayVersion => {
                    // Help/version text is the payload here. It already
                    // ends with a newline, so trim it before
                    // write_payload adds exactly one. A write failure
                    // is handled like the normal path (InternalError +
                    // its exit code), not swallowed: ENOSPC/EIO must
                    // not exit 0.
                    match write_payload(e.render().to_string().trim_end()) {
                        Ok(()) => return ExitCode::from(0),
                        Err(err) => {
                            // No parsed CLI here, so no selected format:
                            // parse-time errors are always JSON.
                            write_error(&err, Format::Json);
                            return ExitCode::from(err.exit_code() as u8);
                        }
                    }
                }
                _ => {
                    // No `cli` exists yet — the format flag itself may
                    // be the parse failure — so this stays JSON.
                    let err = GeectlError::new(ErrorKind::InvalidInput, e.to_string());
                    write_error(&err, Format::Json);
                    return ExitCode::from(err.exit_code() as u8);
                }
            }
        }
    };
    // The .env file loads at startup, before any command runs, and
    // only with explicit opt-in (`GEECTL_DOTENV=1` in the real
    // environment; see `load_dotenv`): without it the file is never
    // read. A missing file is fine, but a malformed, unreadable, or
    // group/other-readable file that carries `GEECTL_*` keys fails
    // fast as a config error — including for `version`, like a
    // malformed config file (see `resolve_format`). The loaded values
    // supply `GEECTL_DEBUG` below, and every later `snapshot_env`
    // call reuses the same loaded snapshot; the load itself never
    // prints a value.
    let dotenv = match load_dotenv() {
        Ok(dotenv) => dotenv,
        Err(err) => {
            write_error(&err, Format::Json);
            return ExitCode::from(err.exit_code() as u8);
        }
    };
    let dotenv_loaded = dotenv.is_some();
    // The debug channel: one backend install for the process,
    // then the startup and config lines on stderr. Off by default,
    // so without `--debug` / `GEECTL_DEBUG=1` this is a no-op.
    // Debug never fails a run: a config line that cannot be built
    // is skipped, and a stderr write failure is ignored like any
    // other diagnostic write. The backend installs only when the
    // channel is on: installing it unconditionally would raise the
    // global max level for every run, and ureq's own `debug!`
    // records would print on the first real HTTP command.
    // GEECTL_DEBUG comes from the real environment first, else the
    // .env file — the same precedence `snapshot_env` applies
    // everywhere else (main reads it before any snapshot exists).
    let debug_env = std::env::var("GEECTL_DEBUG").ok().or_else(|| {
        dotenv
            .as_ref()
            .and_then(|map| map.get("GEECTL_DEBUG").cloned())
    });
    let (debug_enabled, debug_bodies) =
        resolve_debug(cli.debug, cli.debug_body, debug_env.as_deref());
    if debug_enabled {
        geectl::debug::install();
    }
    let debug = Debug::new(debug_enabled, debug_bodies);
    if debug.enabled() {
        debug.emit(&startup_line(&argv));
        if dotenv_loaded {
            // The file name only, never a key or a value.
            debug.emit("loaded .env");
        }
        if let Some(line) = debug_config_line() {
            debug.emit(&line);
        }
    }
    // The format is resolved before `run` moves `cli`: the error
    // path needs it to render in the selected format. A resolution
    // failure (malformed config, invalid GEECTL_FORMAT) is a JSON
    // error — no format was selected yet.
    let (format, format_source) = match resolve_format(cli.format) {
        Ok(resolved) => resolved,
        Err(err) => {
            write_error(&err, Format::Json);
            return ExitCode::from(err.exit_code() as u8);
        }
    };
    let code = match run(cli, &debug, format, format_source) {
        Ok(payload) => match write_payload(&payload) {
            Ok(()) => ExitCode::from(0),
            Err(err) => {
                write_error(&err, format);
                ExitCode::from(err.exit_code() as u8)
            }
        },
        Err(err) => {
            write_error(&err, format);
            ExitCode::from(err.exit_code() as u8)
        }
    };
    // The ssh masters a command's git operations shared are closed
    // here, after the command ran and before the process exits.
    geectl::working_copy::close_ssh_control_masters();
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_payload_is_json_with_name_and_version() {
        let cli = Cli::parse_from(["geectl", "version"]);
        let payload = run(cli, &Debug::disabled(), Format::Json, "default").unwrap();
        let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(value["name"], "geectl");
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn no_subcommand_prints_version_payload() {
        let cli = Cli::parse_from(["geectl"]);
        assert!(run(cli, &Debug::disabled(), Format::Json, "default").is_ok());
    }

    #[test]
    fn resolve_format_flag_wins_without_config() {
        // The flag short-circuits: no config file is read at all.
        let (format, source) = resolve_format(Some(Format::Text)).unwrap();
        assert_eq!(format, Format::Text);
        assert_eq!(source, "flag");
    }

    #[test]
    fn config_show_payload_reports_format_and_source() {
        let payload = config_show_payload(Format::Text, "config").unwrap();
        assert_eq!(payload["format"], "text");
        assert_eq!(payload["format_source"], "config");
        let text = render_config_show_text(&payload);
        assert!(text.contains("format: text (config)"), "{text}");
    }

    #[test]
    fn text_error_renders_message_context_and_remediation() {
        // Unified rendering (the request group owns write_error):
        // the context names the failure when present, else the kind's
        // message; there is no separate `context:` line.
        let err = GeectlError::new(ErrorKind::InvalidInput, "bad name".to_string());
        let text = render_error_text(&err);
        assert!(text.starts_with("error: bad name"));
        assert!(text.contains("remediation: fix the input and retry"));
    }

    #[test]
    fn text_error_omits_empty_context() {
        let err = GeectlError::new(ErrorKind::NotFound, String::new());
        let text = render_error_text(&err);
        assert!(!text.contains("context:"));
        assert!(text.contains("error: the requested resource does not exist"));
    }

    #[test]
    fn terminated_ensures_a_trailing_newline() {
        assert_eq!(terminated("abc"), "abc\n");
        assert_eq!(terminated("abc\n"), "abc\n");
        // Already-terminated payloads are untouched, even with more
        // than one trailing newline: verbatim wins over tidiness.
        assert_eq!(terminated("abc\n\n"), "abc\n\n");
        assert_eq!(terminated(""), "");
    }

    #[test]
    fn error_text_uses_context_when_present() {
        let with_context = GeectlError::new(ErrorKind::ConfigError, "missing credentials");
        let text = render_error_text(&with_context);
        assert!(text.starts_with("error: missing credentials"), "{text}");
        assert!(text.contains("remediation: "), "{text}");
        let bare = GeectlError::new(ErrorKind::ConfigError, "");
        assert_eq!(
            render_error_text(&bare),
            "error: the configuration is invalid\nremediation: check the configuration (flag, environment variable, or config file) and retry"
        );
    }

    #[test]
    fn build_subcommand_parses() {
        let cli = Cli::parse_from([
            "geectl",
            "build",
            "results",
            "openSUSE:Factory",
            "--code",
            "failed",
        ]);
        assert!(matches!(cli.command, Some(Command::Build { .. })));
    }

    #[test]
    fn error_text_renders_code_message_and_remediation() {
        // Unified rendering (the request group owns write_error):
        // the context names the failure in the `error:` line itself.
        let err = GeectlError::new(
            ErrorKind::ConfigError,
            "build reads need GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD",
        );
        let text = render_error_text(&err);
        assert!(text.starts_with("error: build reads need "));
        assert!(text.contains("GEECTL_OBS_USERNAME"));
        assert!(text.contains("remediation: check the configuration"));
    }

    #[test]
    fn resolve_debug_flag_enables_without_env() {
        assert_eq!(resolve_debug(true, false, None), (true, false));
    }

    #[test]
    fn resolve_debug_env_one_enables_without_flag() {
        assert_eq!(resolve_debug(false, false, Some("1")), (true, false));
    }

    #[test]
    fn resolve_debug_env_other_values_disable_without_flag() {
        assert_eq!(resolve_debug(false, false, None), (false, false));
        assert_eq!(resolve_debug(false, false, Some("")), (false, false));
        assert_eq!(resolve_debug(false, false, Some("0")), (false, false));
        assert_eq!(resolve_debug(false, false, Some("yes")), (false, false));
    }

    #[test]
    fn resolve_debug_flag_wins_over_env() {
        assert_eq!(resolve_debug(true, false, Some("0")), (true, false));
    }

    #[test]
    fn resolve_debug_body_implies_debug() {
        assert_eq!(resolve_debug(false, true, None), (true, true));
        assert_eq!(resolve_debug(false, true, Some("0")), (true, true));
    }

    #[test]
    fn resolve_debug_flag_without_body_prints_no_bodies() {
        assert_eq!(resolve_debug(true, false, Some("1")), (true, false));
    }

    #[test]
    fn startup_line_joins_the_arguments() {
        assert_eq!(
            startup_line(&[
                "geectl".to_string(),
                "--debug".to_string(),
                "version".to_string()
            ]),
            format!("geectl {} --debug version", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn startup_line_without_arguments_has_no_trailing_space() {
        assert_eq!(
            startup_line(&["geectl".to_string()]),
            format!("geectl {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    #[cfg(unix)]
    fn lossy_argv_replaces_non_utf8_arguments() {
        use std::os::unix::ffi::OsStringExt;
        let argv = lossy_argv(
            vec![
                std::ffi::OsString::from("geectl"),
                std::ffi::OsString::from_vec(vec![0xff, 0xfe]),
            ]
            .into_iter(),
        );
        assert_eq!(argv[0], "geectl");
        assert!(argv[1].contains('\u{FFFD}'), "argv[1]: {:?}", argv[1]);
    }
}
