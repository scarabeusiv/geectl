//! Working-copy CLI leaves (`geectl checkout`, `update`, `status`,
//! `diff`, `add`, `rm`, `addremove`, `commit`, `pull`, `resolve`).
//!
//! Top-level leaves, not a group: the working copy is the agent's
//! checkout, and the leaves read like verbs. Each leaf operates on
//! the checkout in the current directory, except `checkout`, which
//! creates `<project>/<package>/` under the current directory.
//! The agent checks out, moves into the checkout, and works there:
//! `status`, `diff`, `add`, `rm`, `addremove`, `commit`, `pull`,
//! and `resolve` take no path.
//!
//! Coordinate header (design rule 1): every text rendering opens
//! with one header line echoing the query coordinates
//! (`project: p package: k`). `diff` is a content command and stays
//! verbatim (design rule 4): its text IS the diff. The group's
//! mutations follow the CLI-wide rule: dry-run by default, the
//! global `--live` performs them, and the text opens with the
//! coordinate header, then `dry_run:`, then the result line and the
//! plan.

use geectl::auth::{Credentials, EnvGiteaAuth, EnvObsAuth};
use geectl::config::{self, DEFAULT_GITEA_URL};
use geectl::debug::Debug;
use geectl::error::{ErrorKind, GeectlError};
use geectl::http::{HttpClient, Timeout, UreqClient};
use geectl::obs::{DryRun, ObsClient, RevisionSpec, OBS_API_BASE};
use geectl::working_copy::{
    self, Backend, CommitReport, PullReport, StatusCode, UpdateReport, WorkingCopy,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The resolved endpoints: the OBS base URL, the config file's
/// instances, and the effective config (for the commit service
/// gate).
struct Endpoints {
    obs_base: String,
    instances: Vec<(String, String)>,
    config: config::Config,
}

fn endpoints() -> Result<Endpoints, GeectlError> {
    let path = config::config_path()?;
    let file_config = config::load_config_at(&path)?;
    let effective = config::effective_config(&file_config)?;
    Ok(Endpoints {
        obs_base: effective
            .config
            .obs_url
            .clone()
            .unwrap_or_else(|| OBS_API_BASE.to_string()),
        instances: file_config.instances,
        config: effective.config,
    })
}

/// Whether the leaf needs OBS credentials: `pull` reads the
/// authed expanded routes (`directory_listing_expanded`,
/// `download_source_expanded_to_file`) and `commit` mutates; the
/// rest read public routes. A leaf that needs credentials and
/// does not ask for them 401s — `pull` did, until this table
/// said so. The single source of truth for the credential
/// wiring, pinned by the `leaf_credential_wiring` test; read
/// only through `obs_client`, which is the single choke point.
fn needs_obs_credentials(leaf: &str) -> bool {
    matches!(leaf, "pull" | "commit")
}

/// An OBS client on `http`: the per-instance OBS credentials for
/// a leaf that needs them, when live. The leaf name is the only
/// input — `obs_client` does the `needs_obs_credentials` lookup
/// itself, so the wiring is pinned by the
/// `leaf_credential_wiring` test by construction. The CLI-wide
/// rule is that a dry-run resolves no credentials, except `pull`:
/// its dry-run plan reads the authed expanded routes, and the
/// plan is read-only, so it resolves credentials in dry-run too
/// (a dry-run pull without them 401s on exactly the servers the
/// credential fix was for).
fn obs_client<'a>(
    http: &'a dyn HttpClient,
    endpoints: &Endpoints,
    leaf: &str,
    live: bool,
    debug: &Debug,
) -> Result<ObsClient<'a>, GeectlError> {
    let credentials = if needs_obs_credentials(leaf) && (live || leaf == "pull") {
        EnvObsAuth::credentials_for_url(
            &endpoints.obs_base,
            &endpoints.obs_base,
            &endpoints.instances,
        )?
    } else {
        Credentials::None
    };
    ObsClient::with_base_and_credentials(http, &endpoints.obs_base, credentials)
        .map(|client| client.with_debug(debug.clone()))
}

/// The Gitea credentials for the configured instance: the token
/// and the instance URL it belongs to. `None` when no token is
/// configured: a public git-backed checkout needs none, and the
/// push is what fails closed without one. The URL scopes the token
/// (see `working_copy::GiteaAuth`): git only sends it to that
/// instance.
fn gitea_auth(endpoints: &Endpoints) -> Result<Option<(String, String)>, GeectlError> {
    let url = endpoints
        .config
        .gitea_url
        .clone()
        .unwrap_or_else(|| DEFAULT_GITEA_URL.to_string());
    match EnvGiteaAuth::credentials_for_url(&url, &url, &endpoints.instances)? {
        Credentials::Bearer { token } => Ok(Some((token, url))),
        _ => Ok(None),
    }
}

/// The configured Gitea instance as a `working_copy::GiteaAuth`,
/// when a token is set.
fn gitea_auth_ref(auth: &Option<(String, String)>) -> Option<working_copy::GiteaAuth<'_>> {
    auth.as_ref().map(|(token, url)| working_copy::GiteaAuth {
        token: token.as_str(),
        url: url.as_str(),
    })
}

/// The current directory: the checkout root for every leaf but
/// `checkout`.
fn current_dir() -> Result<PathBuf, GeectlError> {
    std::env::current_dir().map_err(|e| {
        GeectlError::new(
            ErrorKind::InternalError,
            format!("failed to read the current directory: {e}"),
        )
    })
}

/// The working copy in the current directory. `live` decides
/// whether the record may be healed from the root commit: a
/// dry-run plan must not rewrite `geectl.base-*` config, so it
/// opens read-only.
fn open_working_copy(live: bool) -> Result<WorkingCopy, GeectlError> {
    let dir = current_dir()?;
    if live {
        working_copy::open(&dir)
    } else {
        working_copy::open_read(&dir)
    }
}

/// The dry-run plan, when dry-run is on.
fn plan_of(dry_run: &DryRun) -> Option<Vec<String>> {
    if dry_run.enabled() {
        Some(dry_run.plan())
    } else {
        None
    }
}

/// The backend name for JSON.
fn backend_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Obs => "obs",
        Backend::Git => "git",
    }
}

/// The `status` letter for one state, in `git status --porcelain`
/// vocabulary.
fn status_letter(code: StatusCode) -> &'static str {
    match code {
        StatusCode::Added => "A",
        StatusCode::Modified => "M",
        StatusCode::Deleted => "D",
        StatusCode::Unknown => "?",
        StatusCode::Missing => "!",
        StatusCode::Conflicted => "C",
        StatusCode::Renamed => "R",
    }
}

/// The `status` name for one state, for JSON.
fn status_name(code: StatusCode) -> &'static str {
    match code {
        StatusCode::Added => "added",
        StatusCode::Modified => "modified",
        StatusCode::Deleted => "deleted",
        StatusCode::Unknown => "unknown",
        StatusCode::Missing => "missing",
        StatusCode::Conflicted => "conflicted",
        StatusCode::Renamed => "renamed",
    }
}

fn with_header(header: &str, text: String) -> String {
    format!("{header}\n{text}")
}

/// Combine a mutation payload and its text lines: `dry_run` joins
/// the payload and goes right after the header line; the plan, when
/// present, joins the payload and follows the header and result
/// lines verbatim.
fn mutation_result(
    mut payload: Value,
    header: String,
    result_line: String,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    payload["dry_run"] = json!(dry_run);
    let mut lines = vec![header, format!("dry_run: {dry_run}"), result_line];
    if let Some(plan) = plan {
        payload["plan"] = json!(plan);
        lines.extend(plan);
    }
    (payload, lines.join("\n"))
}

/// Check out package `package` in project `project` into
/// `<project>/<package>/` under the current directory.
pub fn checkout(
    project: &str,
    package: &str,
    rev: Option<&str>,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let endpoints = endpoints()?;
    let client = obs_client(&http, &endpoints, "checkout", live, debug)?;
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    // A dry-run checkout clones nothing: resolving the Gitea
    // token here would resolve credentials for a plan.
    let auth = if live { gitea_auth(&endpoints)? } else { None };
    let auth = gitea_auth_ref(&auth);
    let rev = rev.map(RevisionSpec::parse).transpose()?;
    let dir = current_dir()?;
    let target = working_copy::checkout_target(&dir, project, package)?;
    let wc = working_copy::checkout(
        &client,
        project,
        package,
        rev.as_ref(),
        &dir,
        auth.as_ref(),
        &dry_run,
    )?;
    Ok(checkout_result(
        project,
        package,
        &target,
        wc.as_ref(),
        dry_run.enabled(),
        plan_of(&dry_run),
    ))
}

/// The `checkout` result: the checkout record when live, the target
/// path when dry-run.
fn checkout_result(
    project: &str,
    package: &str,
    target: &Path,
    wc: Option<&WorkingCopy>,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let header = format!("project: {project} package: {package}");
    match wc {
        Some(wc) => {
            let payload = json!({
                "project": project,
                "package": package,
                "dir": wc.root.to_string_lossy(),
                "backend": backend_name(wc.backend),
                "base_rev": wc.base_rev,
                "base_srcmd5": wc.base_srcmd5,
            });
            let result_line = format!(
                "checked out {project}/{package} into {} ({} base {})",
                wc.root.display(),
                backend_name(wc.backend),
                wc.base_rev
            );
            mutation_result(payload, header, result_line, dry_run, plan)
        }
        None => {
            let payload = json!({
                "project": project,
                "package": package,
                "dir": target.to_string_lossy(),
            });
            let result_line = format!(
                "dry-run: would check out {project}/{package} into {}",
                target.display()
            );
            mutation_result(payload, header, result_line, dry_run, plan)
        }
    }
}

/// Update the working copy in the current directory to the latest
/// base (or to `--rev`).
pub fn update(
    rev: Option<&str>,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let endpoints = endpoints()?;
    let client = obs_client(&http, &endpoints, "update", live, debug)?;
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    // A dry-run update fetches nothing: resolving the Gitea
    // token here would resolve credentials for a plan.
    let auth = if live { gitea_auth(&endpoints)? } else { None };
    let auth = gitea_auth_ref(&auth);
    let mut wc = open_working_copy(live)?;
    let report = working_copy::update(&mut wc, &client, rev, auth.as_ref(), &dry_run)?;
    Ok(update_result(
        &wc,
        report.as_ref(),
        dry_run.enabled(),
        plan_of(&dry_run),
    ))
}

/// The `update` result: the old and new base, and the conflicts a
/// stopped rebase left.
fn update_result(
    wc: &WorkingCopy,
    report: Option<&UpdateReport>,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let header = format!("project: {} package: {}", wc.project, wc.package);
    match report {
        Some(report) => {
            let payload = json!({
                "project": wc.project,
                "package": wc.package,
                "updated": report.updated,
                "old_base_rev": report.old_base_rev,
                "new_base_rev": report.new_base_rev,
                "old_base_srcmd5": report.old_base_srcmd5,
                "new_base_srcmd5": report.new_base_srcmd5,
                "conflicts": report.conflicts,
                "stash_remaining": report.stash_remaining,
            });
            let mut result_line = if !report.conflicts.is_empty() {
                format!(
                    "update stopped with conflicts in: {}",
                    report.conflicts.join(", ")
                )
            } else if report.updated {
                format!(
                    "updated {}/{} to base {}",
                    wc.project, wc.package, report.new_base_rev
                )
            } else {
                format!(
                    "{}/{} is already at base {}",
                    wc.project, wc.package, report.new_base_rev
                )
            };
            if report.stash_remaining {
                result_line.push_str(
                    "; stashed changes remain in the stash (resolve the conflicts, then run resolve to restore them)",
                );
            }
            mutation_result(payload, header, result_line, dry_run, plan)
        }
        None => {
            let payload = json!({
                "project": wc.project,
                "package": wc.package,
            });
            let result_line = format!("dry-run: would update {}/{}", wc.project, wc.package);
            mutation_result(payload, header, result_line, dry_run, plan)
        }
    }
}

/// Show the working copy's state against the base.
pub fn status(debug: &Debug) -> Result<(Value, String), GeectlError> {
    let _ = debug;
    let wc = open_working_copy(false)?;
    let entries = working_copy::status(&wc)?;
    Ok(status_result(&wc, &entries))
}

/// The `status` result: the entries, one `{letter} {path}` line
/// each.
fn status_result(wc: &WorkingCopy, entries: &[working_copy::StatusEntry]) -> (Value, String) {
    let payload = json!({
        "project": wc.project,
        "package": wc.package,
        "entries": entries.iter().map(|entry| json!({
            "path": entry.path,
            "status": status_name(entry.code),
        })).collect::<Vec<_>>(),
    });
    let text = if entries.is_empty() {
        "no changes".to_string()
    } else {
        entries
            .iter()
            .map(|entry| format!("{} {}", status_letter(entry.code), entry.path))
            .collect::<Vec<_>>()
            .join("\n")
    };
    (
        payload,
        with_header(
            &format!("project: {} package: {}", wc.project, wc.package),
            text,
        ),
    )
}

/// Show the diff of the worktree and the index against the base.
/// A content command: the text IS the diff.
pub fn diff(debug: &Debug) -> Result<(Value, String), GeectlError> {
    let _ = debug;
    let wc = open_working_copy(false)?;
    let diff = working_copy::diff(&wc)?;
    Ok(diff_result(&wc, &diff))
}

/// The `diff` result: the diff verbatim, or `no changes` when the
/// diff is empty.
fn diff_result(wc: &WorkingCopy, diff: &str) -> (Value, String) {
    let payload = json!({
        "project": wc.project,
        "package": wc.package,
        "diff": diff,
    });
    let text = if diff.trim().is_empty() {
        "no changes".to_string()
    } else {
        diff.to_string()
    };
    (payload, text)
}

/// Stage `files` for commit.
pub fn add(files: &[String], debug: &Debug, live: bool) -> Result<(Value, String), GeectlError> {
    let _ = debug;
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    let wc = open_working_copy(live)?;
    working_copy::add(&wc, files, &dry_run)?;
    Ok(add_result(&wc, files, dry_run.enabled(), plan_of(&dry_run)))
}

/// The `add` result: the staged files.
fn add_result(
    wc: &WorkingCopy,
    files: &[String],
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let header = format!("project: {} package: {}", wc.project, wc.package);
    let payload = json!({
        "project": wc.project,
        "package": wc.package,
        "files": files,
    });
    let result_line = if dry_run {
        format!(
            "dry-run: would stage {} in {}/{}",
            files.join(", "),
            wc.project,
            wc.package
        )
    } else {
        format!(
            "staged {} in {}/{}",
            files.join(", "),
            wc.project,
            wc.package
        )
    };
    mutation_result(payload, header, result_line, dry_run, plan)
}

/// Remove `files` from the working copy.
pub fn rm(files: &[String], debug: &Debug, live: bool) -> Result<(Value, String), GeectlError> {
    let _ = debug;
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    let wc = open_working_copy(live)?;
    working_copy::rm(&wc, files, &dry_run)?;
    Ok(rm_result(&wc, files, dry_run.enabled(), plan_of(&dry_run)))
}

/// The `rm` result: the removed files.
fn rm_result(
    wc: &WorkingCopy,
    files: &[String],
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let header = format!("project: {} package: {}", wc.project, wc.package);
    let payload = json!({
        "project": wc.project,
        "package": wc.package,
        "files": files,
    });
    let result_line = if dry_run {
        format!(
            "dry-run: would remove {} from {}/{}",
            files.join(", "),
            wc.project,
            wc.package
        )
    } else {
        format!(
            "removed {} from {}/{}",
            files.join(", "),
            wc.project,
            wc.package
        )
    };
    mutation_result(payload, header, result_line, dry_run, plan)
}

/// Stage new files and worktree deletions.
pub fn addremove(debug: &Debug, live: bool) -> Result<(Value, String), GeectlError> {
    let _ = debug;
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    let wc = open_working_copy(live)?;
    working_copy::addremove(&wc, &dry_run)?;
    Ok(addremove_result(&wc, dry_run.enabled(), plan_of(&dry_run)))
}

/// The `addremove` result.
fn addremove_result(wc: &WorkingCopy, dry_run: bool, plan: Option<Vec<String>>) -> (Value, String) {
    let header = format!("project: {} package: {}", wc.project, wc.package);
    let payload = json!({
        "project": wc.project,
        "package": wc.package,
    });
    let result_line = if dry_run {
        format!(
            "dry-run: would stage new files and worktree deletions in {}/{}",
            wc.project, wc.package
        )
    } else {
        format!(
            "staged new files and worktree deletions in {}/{}",
            wc.project, wc.package
        )
    };
    mutation_result(payload, header, result_line, dry_run, plan)
}

/// Commit the staged changes.
pub fn commit(
    message: &str,
    noservice: bool,
    debug: &Debug,
    live: bool,
) -> Result<(Value, String), GeectlError> {
    // Refused before anything is resolved: an empty message
    // aborts the git commit and sends an empty comment on the
    // OBS path, and neither needs a client or credentials first.
    if message.trim().is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "commit needs a non-empty --message",
        ));
    }
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let endpoints = endpoints()?;
    let client = obs_client(&http, &endpoints, "commit", live, debug)?;
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    // A dry-run commit pushes nothing: resolving the Gitea
    // token here would resolve credentials for a plan.
    let auth = if live { gitea_auth(&endpoints)? } else { None };
    let auth = gitea_auth_ref(&auth);
    let mut wc = open_working_copy(live)?;
    let report = working_copy::commit(
        &mut wc,
        &client,
        &endpoints.config,
        message,
        noservice,
        auth.as_ref(),
        &dry_run,
    )?;
    Ok(commit_result(
        &wc,
        report.as_ref(),
        dry_run.enabled(),
        plan_of(&dry_run),
    ))
}

/// The `commit` result: the new revision or the pushed head, the
/// files sent, and the open requests the commit landed under.
fn commit_result(
    wc: &WorkingCopy,
    report: Option<&CommitReport>,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let header = format!("project: {} package: {}", wc.project, wc.package);
    match report {
        Some(report) => {
            let payload = json!({
                "project": wc.project,
                "package": wc.package,
                "rev": report.rev,
                "srcmd5": report.srcmd5,
                "pushed_head": report.pushed_head,
                "open_requests": report.open_requests,
                "files": report.files,
            });
            let mut result_line = match (report.rev, &report.pushed_head) {
                (Some(rev), _) => {
                    format!("committed {}/{} (revision {rev})", wc.project, wc.package)
                }
                (None, Some(head)) => format!(
                    "committed {}/{} (pushed head {head})",
                    wc.project, wc.package
                ),
                (None, None) => format!("committed {}/{}", wc.project, wc.package),
            };
            if !report.open_requests.is_empty() {
                result_line.push_str(&format!(
                    "; open requests: {}",
                    report.open_requests.join(", ")
                ));
            }
            mutation_result(payload, header, result_line, dry_run, plan)
        }
        None => {
            let payload = json!({
                "project": wc.project,
                "package": wc.package,
            });
            let result_line = format!("dry-run: would commit {}/{}", wc.project, wc.package);
            mutation_result(payload, header, result_line, dry_run, plan)
        }
    }
}

/// Pull a linked package to the link target's latest revision.
pub fn pull(debug: &Debug, live: bool) -> Result<(Value, String), GeectlError> {
    let http = UreqClient::new(Timeout::default())?.with_debug(debug.clone());
    let endpoints = endpoints()?;
    // `pull` resolves credentials even in dry-run (see
    // `obs_client`): its plan reads the authed expanded routes,
    // and the plan is read-only.
    let client = obs_client(&http, &endpoints, "pull", live, debug)?;
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    let mut wc = open_working_copy(live)?;
    let report = working_copy::pull(&mut wc, &client, &dry_run)?;
    Ok(pull_result(
        &wc,
        report.as_ref(),
        dry_run.enabled(),
        plan_of(&dry_run),
    ))
}

/// The `pull` result: the old and new link revisions, and the
/// conflicts a stopped rebase left.
fn pull_result(
    wc: &WorkingCopy,
    report: Option<&PullReport>,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let header = format!("project: {} package: {}", wc.project, wc.package);
    match report {
        Some(report) => {
            let payload = json!({
                "project": wc.project,
                "package": wc.package,
                "updated": report.updated,
                "old_baserev": report.old_baserev,
                "new_baserev": report.new_baserev,
                "old_base_rev": report.old_base_rev,
                "new_base_rev": report.new_base_rev,
                "conflicts": report.conflicts,
                "stash_remaining": report.stash_remaining,
            });
            let mut result_line = if !report.conflicts.is_empty() {
                format!(
                    "pull stopped with conflicts in: {}",
                    report.conflicts.join(", ")
                )
            } else if report.updated {
                format!(
                    "pulled {}/{} to baserev {}",
                    wc.project, wc.package, report.new_baserev
                )
            } else {
                format!(
                    "{}/{} is already at baserev {}",
                    wc.project, wc.package, report.new_baserev
                )
            };
            if report.stash_remaining {
                result_line.push_str(
                    "; stashed changes remain in the stash (resolve the conflicts, then run resolve to restore them)",
                );
            }
            mutation_result(payload, header, result_line, dry_run, plan)
        }
        None => {
            let payload = json!({
                "project": wc.project,
                "package": wc.package,
            });
            let result_line = format!("dry-run: would pull {}/{}", wc.project, wc.package);
            mutation_result(payload, header, result_line, dry_run, plan)
        }
    }
}

/// Stage the resolved files and continue a stopped rebase.
pub fn resolve(debug: &Debug, live: bool) -> Result<(Value, String), GeectlError> {
    let _ = debug;
    let dry_run = DryRun::new(!live).with_debug(debug.clone());
    let mut wc = open_working_copy(live)?;
    let resolved = working_copy::resolve(&mut wc, &dry_run)?;
    Ok(resolve_result(
        &wc,
        resolved.as_deref(),
        dry_run.enabled(),
        plan_of(&dry_run),
    ))
}

/// The `resolve` result: the files that were resolved.
fn resolve_result(
    wc: &WorkingCopy,
    resolved: Option<&[String]>,
    dry_run: bool,
    plan: Option<Vec<String>>,
) -> (Value, String) {
    let header = format!("project: {} package: {}", wc.project, wc.package);
    match resolved {
        Some(resolved) => {
            let payload = json!({
                "project": wc.project,
                "package": wc.package,
                "resolved": resolved,
            });
            let result_line = if resolved.is_empty() {
                format!("nothing to resolve in {}/{}", wc.project, wc.package)
            } else {
                format!(
                    "resolved {} in {}/{}",
                    resolved.join(", "),
                    wc.project,
                    wc.package
                )
            };
            mutation_result(payload, header, result_line, dry_run, plan)
        }
        None => {
            let payload = json!({
                "project": wc.project,
                "package": wc.package,
            });
            let result_line = format!("dry-run: would resolve {}/{}", wc.project, wc.package);
            mutation_result(payload, header, result_line, dry_run, plan)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geectl::working_copy::StatusEntry;

    fn wc() -> WorkingCopy {
        WorkingCopy {
            root: PathBuf::from("/tmp/home_alice/foo"),
            project: "home:alice".to_string(),
            package: "foo".to_string(),
            apiurl: "https://api.opensuse.org".to_string(),
            backend: Backend::Obs,
            base_rev: "1".to_string(),
            base_srcmd5: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        }
    }

    #[test]
    fn checkout_result_live_carries_the_record() {
        let dir = PathBuf::from("/tmp");
        let wc = wc();
        let target = dir.join("home_alice").join("foo");
        let (_, text) = checkout_result("home:alice", "foo", &target, Some(&wc), false, None);
        assert!(text.contains("project: home:alice package: foo"));
        assert!(text.contains("dry_run: false"));
        assert!(text.contains(&format!(
            "checked out home:alice/foo into {} (obs base 1)",
            wc.root.display()
        )));
    }

    #[test]
    fn checkout_result_dry_run_carries_the_target_and_the_plan() {
        let dir = PathBuf::from("/tmp");
        // The target names the sanitized directory (`:` becomes
        // `_`); the separators are the platform's.
        let target = dir.join("home_alice").join("foo");
        let target_text = target.display().to_string();
        let (payload, text) = checkout_result(
            "home:alice",
            "foo",
            &target,
            None,
            true,
            Some(vec![format!("checkout home:alice/foo into {target_text}")]),
        );
        assert_eq!(payload["dry_run"], json!(true));
        assert_eq!(payload["dir"], json!(target_text));
        assert_eq!(
            payload["plan"],
            json!([format!("checkout home:alice/foo into {target_text}")])
        );
        assert!(text.contains(&format!(
            "dry-run: would check out home:alice/foo into {target_text}"
        )));
    }

    #[test]
    fn update_result_names_the_new_base_and_the_conflicts() {
        let report = UpdateReport {
            updated: true,
            old_base_rev: "1".to_string(),
            new_base_rev: "2".to_string(),
            old_base_srcmd5: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            new_base_srcmd5: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            conflicts: Vec::new(),
            stash_remaining: false,
        };
        let (_, text) = update_result(&wc(), Some(&report), false, None);
        assert!(text.contains("updated home:alice/foo to base 2"));
        let conflicted = UpdateReport {
            updated: false,
            old_base_rev: "1".to_string(),
            new_base_rev: "2".to_string(),
            old_base_srcmd5: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            new_base_srcmd5: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            conflicts: vec!["foo.spec".to_string()],
            stash_remaining: false,
        };
        let (_, text) = update_result(&wc(), Some(&conflicted), false, None);
        assert!(text.contains("update stopped with conflicts in: foo.spec"));
        let current = UpdateReport {
            updated: false,
            old_base_rev: "1".to_string(),
            new_base_rev: "1".to_string(),
            old_base_srcmd5: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            new_base_srcmd5: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            conflicts: Vec::new(),
            stash_remaining: false,
        };
        let (_, text) = update_result(&wc(), Some(&current), false, None);
        assert!(text.contains("home:alice/foo is already at base 1"));
    }

    #[test]
    fn status_result_renders_one_letter_line_per_entry() {
        let entries = vec![
            StatusEntry {
                path: "foo.spec".to_string(),
                code: StatusCode::Modified,
            },
            StatusEntry {
                path: "new.txt".to_string(),
                code: StatusCode::Added,
            },
            StatusEntry {
                path: "gone.txt".to_string(),
                code: StatusCode::Missing,
            },
        ];
        let (payload, text) = status_result(&wc(), &entries);
        assert_eq!(payload["entries"][0]["status"], json!("modified"));
        assert_eq!(payload["entries"][1]["status"], json!("added"));
        assert_eq!(payload["entries"][2]["status"], json!("missing"));
        assert!(
            text.contains("project: home:alice package: foo\nM foo.spec\nA new.txt\n! gone.txt")
        );
        let (_, text) = status_result(&wc(), &[]);
        assert!(text.contains("no changes"));
    }

    #[test]
    fn diff_result_keeps_the_diff_verbatim() {
        let (_, text) = diff_result(&wc(), "--- a/foo.spec\n+++ b/foo.spec\n");
        assert_eq!(text, "--- a/foo.spec\n+++ b/foo.spec\n");
        let (payload, text) = diff_result(&wc(), "");
        assert_eq!(payload["diff"], json!(""));
        assert_eq!(text, "no changes");
    }

    #[test]
    fn commit_result_names_the_revision_and_the_open_requests() {
        let report = CommitReport {
            rev: Some(2),
            srcmd5: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
            pushed_head: None,
            open_requests: vec!["42".to_string()],
            files: vec!["foo.spec".to_string()],
        };
        let (payload, text) = commit_result(&wc(), Some(&report), false, None);
        assert_eq!(payload["rev"], json!(2));
        assert!(text.contains("committed home:alice/foo (revision 2); open requests: 42"));
        let pushed = CommitReport {
            rev: None,
            srcmd5: None,
            pushed_head: Some("abcdef1234567890abcdef1234567890abcdef12".to_string()),
            open_requests: Vec::new(),
            files: vec!["foo.spec".to_string()],
        };
        let (_, text) = commit_result(&wc(), Some(&pushed), false, None);
        assert!(text.contains(
            "committed home:alice/foo (pushed head abcdef1234567890abcdef1234567890abcdef12)"
        ));
    }

    #[test]
    fn pull_result_names_the_new_baserev() {
        let report = PullReport {
            updated: true,
            old_baserev: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            new_baserev: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            old_base_rev: "1".to_string(),
            new_base_rev: "2".to_string(),
            conflicts: Vec::new(),
            stash_remaining: false,
        };
        let (_, text) = pull_result(&wc(), Some(&report), false, None);
        assert!(text.contains("pulled home:alice/foo to baserev bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"));
    }

    #[test]
    fn resolve_result_names_the_resolved_files() {
        let (_, text) = resolve_result(&wc(), Some(&["foo.spec".to_string()]), false, None);
        assert!(text.contains("resolved foo.spec in home:alice/foo"));
        let (_, text) = resolve_result(&wc(), Some(&[]), false, None);
        assert!(text.contains("nothing to resolve in home:alice/foo"));
    }

    #[test]
    fn leaf_credential_wiring_pins_which_leaves_need_credentials() {
        // `pull` reads the authed expanded routes and `commit`
        // mutates: without credentials both 401. `checkout` and
        // `update` read public routes only. `obs_client` is the
        // single choke point for the table, so this pins the
        // wiring: a leaf names itself there, and the table
        // decides.
        assert!(needs_obs_credentials("pull"));
        assert!(needs_obs_credentials("commit"));
        assert!(!needs_obs_credentials("checkout"));
        assert!(!needs_obs_credentials("update"));
        assert!(!needs_obs_credentials("status"));
    }

    #[test]
    fn update_result_names_a_remaining_stash() {
        let report = UpdateReport {
            updated: false,
            old_base_rev: "1".to_string(),
            new_base_rev: "2".to_string(),
            old_base_srcmd5: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            new_base_srcmd5: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            conflicts: vec!["foo.spec".to_string()],
            stash_remaining: true,
        };
        let (payload, text) = update_result(&wc(), Some(&report), false, None);
        assert_eq!(payload["stash_remaining"], json!(true));
        assert!(text.contains("stashed changes remain in the stash"));
    }

    #[test]
    fn commit_rejects_an_empty_message_before_any_resolution() {
        let err = commit("", false, &Debug::disabled(), false).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let err = commit("   ", false, &Debug::disabled(), false).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn mutation_result_carries_dry_run_and_plan_in_order() {
        let (payload, text) = mutation_result(
            json!({"project": "home:alice", "package": "foo"}),
            "project: home:alice package: foo".to_string(),
            "staged foo.spec in home:alice/foo".to_string(),
            true,
            Some(vec!["stage foo.spec".to_string()]),
        );
        assert_eq!(payload["dry_run"], json!(true));
        assert_eq!(payload["plan"], json!(["stage foo.spec"]));
        assert_eq!(
            text,
            "project: home:alice package: foo\ndry_run: true\nstaged foo.spec in home:alice/foo\nstage foo.spec"
        );
    }
}
