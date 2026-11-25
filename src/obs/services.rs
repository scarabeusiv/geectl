//! OBS service state, service definitions, and the server-side
//! service trigger: [`ObsClient::service_state`] reads the run state
//! the server recorded in the directory listing,
//! [`ObsClient::list_services`] reads the stored `_service` text
//! as typed definitions, and [`ObsClient::trigger_service`] triggers
//! a server-side run of the package's services. A service run is
//! otherwise triggered by a commit, never by this library. Wait and
//! merge are out under the Factory scope filter (design §4): each
//! exists only to drive server-side runs, which Factory declines.
//! The commit path is intended to reuse `service_state` as its
//! settlement read.

use super::build_url;
use super::parse_status_code;
use super::parse_status_ok;
use super::tag_attr;
use super::valid_obs_name;
use super::DirectoryListing;
use super::DryRun;
use super::ObsClient;
use super::ServiceState;
use crate::debug::{format_body_preview, strip_terminal_controls};
use crate::error::ErrorKind;
use crate::error::GeectlError;
use crate::xml::find_tag_end;
use crate::xml::xml_child_elements;
use crate::xml::xml_direct_comments;
use crate::xml::xml_unescape;

/// A service definition from the stored `_service` file: one
/// `<service>` element, with its mode and parameters as stored.
/// Modes are plain strings, never an enum: a run mode the client
/// does not know is still a definition to report, not a parse
/// failure. (The server rejects unknown modes at upload with 400,
/// so no live round-trip covers one; the parse is unit-tested.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDef {
    /// The `name` attribute: the service binary, e.g.
    /// `download_url`.
    pub name: String,
    /// The `mode` attribute as stored (`manual`, `disabled`,
    /// `trylocal`, `buildtime`, `serveronly`, ...). Empty when the
    /// service carries no mode attribute (the default mode).
    pub mode: String,
    /// The `<param>` children, in file order.
    pub params: Vec<ServiceParam>,
    /// The HTML comments that are direct children of the
    /// `<service>` element, each converted to plain text, in file
    /// order. A comment whose
    /// text is empty after conversion is not listed: the field
    /// carries what a reader of the stored file would see, not
    /// the markup. At most 32 comments are kept per service and
    /// each is at most 4096 characters: comments are annotation,
    /// not structure, so the stored file cannot grow the typed
    /// view without bound.
    pub comments: Vec<String>,
}

/// One `<param>` of a service definition: its name and value,
/// verbatim from the stored file (an empty element is an empty
/// value, not a missing parameter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceParam {
    /// The `name` attribute of the parameter.
    pub name: String,
    /// The element text, unescaped.
    pub value: String,
}

/// Skip the miscellany that may precede a document's root element
/// or follow it: whitespace, comments (`<!-- ... -->`), processing
/// instructions (`<? ... ?>`, including the `<?xml` prolog), and a
/// DOCTYPE (`<!DOCTYPE ...>`). Returns `None` when a comment, PI, or
/// DOCTYPE never terminates: an unterminated header is a malformed
/// file, not an empty one. A DOCTYPE with a well-formed internal
/// subset (a `[...]` holding `>` characters, closed with `]` before the
/// root) is not skipped past — the file fails as malformed instead.
fn skip_document_misc(mut text: &str) -> Option<&str> {
    loop {
        text = text.trim_start();
        if let Some(after) = text.strip_prefix("<!--") {
            let end = after.find("-->")?;
            text = &after[end + 3..];
        } else if let Some(after) = text.strip_prefix("<?") {
            let end = after.find("?>")?;
            text = &after[end + 2..];
        } else if text.starts_with("<!DOCTYPE") {
            let after = &text["<!DOCTYPE".len()..];
            let first = after.chars().next()?;
            if !first.is_whitespace() {
                return Some(text);
            }
            let end = after.find('>')?;
            text = &after[end + 1..];
        } else {
            return Some(text);
        }
    }
}

/// The render width for a comment's HTML-to-text conversion, in
/// columns. Finite, so `html2text` never sizes an allocation or
/// arithmetic by `usize::MAX`; well above any realistic comment,
/// so prose and long words stay unwrapped in practice.
const COMMENT_RENDER_WIDTH: usize = 4096;

/// The most comments kept on one service definition, and the
/// most characters kept per comment. A stored `_service` file is
/// untrusted input, and comments are annotation, not structure:
/// beyond the count further comments are not listed, and a longer
/// comment is truncated to its head.
const MAX_SERVICE_COMMENTS: usize = 32;
const MAX_COMMENT_CHARS: usize = 4096;

/// Convert an HTML fragment to plain text: a comment in a stored
/// `_service` file is markup (a commented-out element, a prose
/// note), and the typed view carries its text, not the tags.
/// The render width is `COMMENT_RENDER_WIDTH`: the stored
/// payload is not terminal output, so a comment within the width
/// stays one line and a long word is never split. Link footnotes
/// are off
/// (`plain_no_decorate`): a comment naming a link keeps the link
/// text, not a footnote block. The output is otherwise
/// `html2text`'s plain text as rendered: links keep their text in
/// brackets, headings keep their `#`, lists keep their `*`. That
/// rendering is the defined conversion. HTML source whitespace
/// collapses: a comment's own line breaks do not survive, only
/// block structure produces lines. The result is trimmed. A
/// conversion failure yields no text and the comment is not
/// listed: the comment is annotation, not structure, so a failed
/// render never fails the parse. (The read itself cannot fail:
/// the input is an in-memory slice.)
fn html_to_plaintext(html: &str) -> String {
    let rendered = html2text::config::plain_no_decorate()
        .string_from_read(html.as_bytes(), COMMENT_RENDER_WIDTH)
        .unwrap_or_default();
    // Terminal control characters are stripped: comment text comes
    // from untrusted `_service` files, and the render is printed
    // raw to the terminal — an entity-decoded escape sequence
    // (`&#27;[2J`, `&#27;]0;title`) or carriage return must not
    // reach it. Newlines survive: they are the render's own line
    // structure.
    let trimmed = strip_terminal_controls(&rendered);
    let trimmed = trimmed.trim();
    if trimmed.chars().count() > MAX_COMMENT_CHARS {
        trimmed.chars().take(MAX_COMMENT_CHARS).collect()
    } else {
        trimmed.to_string()
    }
}

/// Parse a stored `_service` file into typed service definitions.
///
/// The parse follows the server's rule (`osc`'s
/// `Serviceinfo.read`): a service needs a `name` of at least three
/// characters with no `/`, and every parameter needs a `name`. The
/// mode is stored verbatim, including modes this client does not
/// know. Terminal control characters are stripped from the
/// name, the mode, and parameter names and values: the stored
/// file is untrusted input, and the typed values are printed raw
/// to the terminal. A `<link>` element is skipped, not parsed into
/// a service:
/// it names the project the definitions are inherited from. XML
/// comments, a prolog, and a DOCTYPE (without an internal subset,
/// and without `>` in a quoted literal) around the root, and comments
/// and stray text outside a service, are ignored. A comment that
/// is a direct child of a service is converted to plain text
/// and listed on the definition: the stored file's commented-out
/// content is part of what a reader sees, so the typed view
/// carries its text, not the markup.
///
/// A malformed file is a fail-closed [`ErrorKind::ServerError`],
/// never a partial list: a caller that acted on half a `_service`
/// would run the wrong services.
pub(crate) fn parse_services(body: &str) -> Result<Vec<ServiceDef>, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS _service file: not a <services> document; received: {}",
                format_body_preview(body)
            ),
        )
    };
    // The prolog, comments, processing instructions, and a
    // DOCTYPE (without an internal subset, and without `>` in a quoted
    // literal) around the root are skipped: a stored file may carry a
    // header comment, and the parser reads the document, not just
    // the root element.
    let Some(body) = skip_document_misc(body) else {
        return Err(malformed());
    };
    let body = body.strip_prefix('<').ok_or_else(malformed)?;
    // The root element must be `<services>`: the tag name ends at
    // whitespace, `/`, or `>`.
    if !body.starts_with("services") {
        return Err(malformed());
    }
    let after_name = &body["services".len()..];
    let Some(first) = after_name.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    // The open tag ends at the first `>` outside a quoted attribute
    // value: an unterminated root is a malformed file, not an empty
    // list.
    let Some(end) = find_tag_end(after_name) else {
        return Err(malformed());
    };
    let open_tag = &after_name[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<services/>`: the file exists and is empty.
        // Anything after the element must be whitespace, comments,
        // or PIs: a second root is a malformed file, not a second
        // list.
        let Some(trailing) = skip_document_misc(&after_name[end + 1..]) else {
            return Err(malformed());
        };
        if !trailing.is_empty() {
            return Err(malformed());
        }
        return Ok(Vec::new());
    }
    let after_open = &after_name[end + 1..];
    let Some(close_pos) = after_open.find("</services>") else {
        return Err(malformed());
    };
    let inner = &after_open[..close_pos];
    // Anything after the close must be whitespace, comments, or
    // PIs: a second root is a malformed file, not a second list.
    let Some(trailing) = skip_document_misc(&after_open[close_pos + "</services>".len()..]) else {
        return Err(malformed());
    };
    if !trailing.is_empty() {
        return Err(malformed());
    }
    let mut services = Vec::new();
    // Non-service elements are skipped: the stored format carries
    // `<link>` alongside the services, and the view is defined over
    // services only. A malformed child (an unterminated element, a
    // stray closing tag) fails the whole parse.
    for element in xml_child_elements(inner, "service").map_err(|_| malformed())? {
        let Some(end) = find_tag_end(element) else {
            return Err(malformed());
        };
        let open_tag = &element[..end];
        let Some(name) =
            tag_attr(open_tag, "name").map(|value| strip_terminal_controls(&xml_unescape(&value)))
        else {
            return Err(malformed());
        };
        if name.chars().count() < 3 || name.contains('/') {
            return Err(malformed());
        }
        let mode = tag_attr(open_tag, "mode")
            .map(|value| strip_terminal_controls(&xml_unescape(&value)))
            .unwrap_or_default();
        let mut params = Vec::new();
        let mut comments = Vec::new();
        if !open_tag.trim_end().ends_with('/') {
            let Some(inner) = element[end + 1..].strip_suffix("</service>") else {
                return Err(malformed());
            };
            for param in xml_child_elements(inner, "param").map_err(|_| malformed())? {
                let Some(param_end) = find_tag_end(param) else {
                    return Err(malformed());
                };
                let param_tag = &param[..param_end];
                let Some(param_name) = tag_attr(param_tag, "name")
                    .map(|value| strip_terminal_controls(&xml_unescape(&value)))
                else {
                    return Err(malformed());
                };
                let value = if param_tag.trim_end().ends_with('/') {
                    String::new()
                } else {
                    let Some(param_inner) = param[param_end + 1..].strip_suffix("</param>") else {
                        return Err(malformed());
                    };
                    // The value is the text before the first child
                    // element, as in osc's read() (ElementTree
                    // .text): markup inside a parameter is not part
                    // of its value.
                    let text = match param_inner.find('<') {
                        Some(pos) => &param_inner[..pos],
                        None => param_inner,
                    };
                    strip_terminal_controls(&xml_unescape(text))
                };
                params.push(ServiceParam {
                    name: param_name,
                    value,
                });
            }
            // The direct-child comments of this service, in file
            // order, as plain text: a commented-out parameter is
            // still part of the stored definition a reader sees.
            for comment in xml_direct_comments(inner) {
                if comments.len() >= MAX_SERVICE_COMMENTS {
                    break;
                }
                let text = html_to_plaintext(comment);
                if !text.is_empty() {
                    comments.push(text);
                }
            }
        }
        services.push(ServiceDef {
            name,
            mode,
            params,
            comments,
        });
    }
    Ok(services)
}

/// The accepted server-side service trigger: the server answered
/// `<status code="ok"/>` to the `runservice` POST. The trigger only
/// queues the run — the answer says the run was queued, not that it
/// finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceTrigger {
    /// The service the trigger was for, when one was named.
    pub service: Option<String>,
    /// The status code from the server's answer (`"ok"`).
    pub code: String,
}

impl ObsClient<'_> {
    /// The directory listing behind a service read: the public
    /// listing, except for `_project`, which the public routes
    /// cannot address — their access check looks up a Package row,
    /// and `_project` is not one — while the authed source
    /// endpoints resolve it via the project. A client without
    /// credentials gets the server's 401 there, like any other
    /// authenticated read.
    fn service_directory_listing(
        &self,
        project: &str,
        package: &str,
    ) -> Result<DirectoryListing, GeectlError> {
        if package == "_project" {
            self.authed_directory_listing(project, package, &[])
        } else {
            self.directory_listing(project, package, None, false)
        }
    }

    /// The `_service` file body behind `list_services`: the public
    /// `cat`, except for `_project`, which the public file route
    /// likewise cannot address (same Package lookup; the authed
    /// file endpoint resolves it via the project instead).
    fn service_file_body(&self, project: &str, package: &str) -> Result<String, GeectlError> {
        if package == "_project" {
            self.authed_cat(project, package, "_service")
        } else {
            self.cat(project, package, "_service", None)
        }
    }

    /// Read the service run state of package `package` in project
    /// `project`, from the directory listing. `Ok(None)` when the
    /// listing carries no `<serviceinfo>` (no run recorded). The read
    /// is public for real packages: no credentials. A missing package
    /// is [`ErrorKind::NotFound`]; a listing with an unknown service
    /// code is a malformed response ([`ErrorKind::ServerError`]).
    ///
    /// The `_project` package reads the project's own service state
    /// through the authed source endpoints instead: the public
    /// routes cannot address `_project` at all, so a client without
    /// credentials gets the server's 401 there. The live probe
    /// records what the instance returns rather than asserting a
    /// state.
    pub fn service_state(
        &self,
        project: &str,
        package: &str,
    ) -> Result<Option<ServiceState>, GeectlError> {
        let listing = self.service_directory_listing(project, package)?;
        let state = listing.serviceinfo;
        match &state {
            Some(state) => self.debug.emit(&format!(
                "service_state {project}/{package}: {}",
                state.code
            )),
            None => self.debug.emit(&format!(
                "service_state {project}/{package}: no serviceinfo"
            )),
        }
        Ok(state)
    }

    /// Read the stored `_service` file of package `package` in
    /// project `project` as typed service definitions. `Ok(None)`
    /// when the package has no `_service` file; `Ok(Some(vec![]))`
    /// when the file exists and is empty. The read is public for
    /// real packages: no credentials. A missing package is
    /// [`ErrorKind::NotFound`]; a malformed `_service` is a
    /// fail-closed [`ErrorKind::ServerError`], never a partial list.
    ///
    /// The `_project` package reads the project's own `_service`
    /// file through the authed source endpoints instead: the public
    /// routes cannot address `_project` at all, so a client without
    /// credentials gets the server's 401 there.
    pub fn list_services(
        &self,
        project: &str,
        package: &str,
    ) -> Result<Option<Vec<ServiceDef>>, GeectlError> {
        self.list_services_via(false, project, package)
    }

    /// The `_service` definitions behind `trigger_service`'s
    /// `--service` validation: like [`ObsClient::list_services`],
    /// but the read uses the authenticated source endpoints when
    /// the client has credentials — a package that is not publicly
    /// readable 404s on the public path and would misreport as
    /// missing — and the public path otherwise.
    fn list_services_for_trigger(
        &self,
        project: &str,
        package: &str,
    ) -> Result<Option<Vec<ServiceDef>>, GeectlError> {
        self.list_services_via(self.has_auth(), project, package)
    }

    /// The `_service` read shared by [`ObsClient::list_services`]
    /// and the trigger validation: the directory listing, then the
    /// file body when the listing carries a `_service` entry.
    /// `authed` selects the authenticated source endpoints over
    /// the public path.
    fn list_services_via(
        &self,
        authed: bool,
        project: &str,
        package: &str,
    ) -> Result<Option<Vec<ServiceDef>>, GeectlError> {
        let listing = if authed {
            self.authed_directory_listing(project, package, &[])?
        } else {
            self.service_directory_listing(project, package)?
        };
        if !listing.entries.iter().any(|entry| entry.name == "_service") {
            self.debug.emit(&format!(
                "list_services {project}/{package}: no _service file"
            ));
            return Ok(None);
        }
        let body = if authed {
            self.authed_cat(project, package, "_service")?
        } else {
            self.service_file_body(project, package)?
        };
        let services = parse_services(&body)?;
        self.debug.emit(&format!(
            "list_services {project}/{package}: {} service(s)",
            services.len()
        ));
        Ok(Some(services))
    }

    /// Trigger a server-side run of the services of package `package`
    /// in project `project` (`POST
    /// /source/<project>/<package>?cmd=runservice`) and return the
    /// server's `<status code>`. The trigger only queues the run:
    /// the server runs the services asynchronously, so success says
    /// the run was queued, not that it finished; the outcome is read
    /// with `service_state`. Requires credentials; without them the
    /// call fails loudly before any mutation — live, the credentials
    /// check runs before the name check, so a live trigger without
    /// credentials fails as a configuration error instead of
    /// reporting the stored file as not found. In dry-run the
    /// trigger is recorded in the plan and nothing is triggered,
    /// and the result is `None`.
    ///
    /// The server runs the package's whole service set: there is no
    /// per-service server-side run. `service`, when given, names the
    /// service the trigger is for: the name must be defined in the
    /// package's stored `_service` — a name that is not is a
    /// fail-closed [`ErrorKind::NotFound`], before any POST — but it
    /// does not narrow the run. Without it, every service runs. A
    /// package with no `_service` file is likewise
    /// [`ErrorKind::NotFound`]: the server has nothing to run. A
    /// missing package is [`ErrorKind::NotFound`]; any other server
    /// refusal is the transport's error, never success. The name
    /// check runs before the dry-run guard, so a dry-run trigger
    /// with a mistyped service fails instead of reporting a clean
    /// plan; the check reads the stored file through the
    /// authenticated endpoints when the client has credentials.
    /// A stored file that cannot be read at all — a missing
    /// package, a package that is not publicly readable without
    /// credentials — defers the check in dry-run: the plan must
    /// not fail on a read it cannot make. Live still fails on the
    /// read error.
    ///
    /// An agent triggers the services of the package it just changed
    /// so the server re-runs them without waiting for a commit.
    pub fn trigger_service(
        &self,
        project: &str,
        package: &str,
        service: Option<&str>,
        dry_run: &DryRun,
    ) -> Result<Option<ServiceTrigger>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if let Some(service) = service {
            if service.chars().count() < 3 || service.contains('/') {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid service name: {service:?}"),
                ));
            }
        }
        // Live, the credentials check runs before the name
        // check: without credentials the public-path read of a
        // credentials-only package would 404 and mask the
        // configuration error as not-found. Dry-run skips it —
        // the plan needs no credentials.
        if !dry_run.enabled() && !self.has_auth() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot trigger services without OBS credentials (GEECTL_OBS_APIKEY[_FILE] or GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD[_FILE])",
            ));
        }
        // The name check runs before the dry-run guard: a
        // mistyped service fails the dry-run plan instead of
        // passing it and failing only live. A stored file that
        // cannot be read at all defers the check in dry-run —
        // the plan must not fail on a read it cannot make —
        // and fails live on the read error.
        if let Some(service) = service {
            match self.list_services_for_trigger(project, package) {
                Ok(None) => {
                    return Err(GeectlError::new(
                        ErrorKind::NotFound,
                        format!("package {project}/{package} has no _service file"),
                    ));
                }
                Ok(Some(services)) if !services.iter().any(|defined| defined.name == service) => {
                    return Err(GeectlError::new(
                        ErrorKind::NotFound,
                        format!("service {service} is not defined in {project}/{package}"),
                    ));
                }
                Ok(Some(_)) => {}
                Err(_) if dry_run.enabled() => {
                    self.debug.emit(&format!(
                        "trigger_service {project}/{package}: _service unreadable in dry-run, planning without the name check"
                    ));
                }
                Err(err) => return Err(err),
            }
        }
        let action = match service {
            Some(service) => {
                format!("trigger services for {project}/{package} (service {service})")
            }
            None => format!("trigger services for {project}/{package}"),
        };
        if !dry_run.allow(&action) {
            return Ok(None);
        }
        let url = build_url(
            &self.base,
            &["source", project, package],
            &[("cmd", "runservice")],
        );
        let body = self.http.post(&url, "", &self.credentials)?;
        if !parse_status_ok(&body) {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS runservice response for package {project}/{package}"),
            ));
        }
        self.debug.emit(&format!(
            "trigger_service {project}/{package}: ok{}",
            service
                .map(|name| format!(" (service {name})"))
                .unwrap_or_default()
        ));
        Ok(Some(ServiceTrigger {
            service: service.map(str::to_string),
            code: parse_status_code(&body)
                .expect("the status answer parsed ok, so it carries a code"),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::Debug;
    use crate::obs::test_support::{basic_credentials, FakeHttp};
    use crate::obs::ServiceCode;
    use std::sync::{Arc, Mutex};

    const DIRECTORY_URL: &str = "https://api.opensuse.org/public/source/openSUSE:Factory/foo";
    const SERVICE_URL: &str =
        "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_service";
    const AUTHED_DIRECTORY_URL: &str = "https://api.opensuse.org/source/openSUSE:Factory/foo";
    const AUTHED_SERVICE_URL: &str =
        "https://api.opensuse.org/source/openSUSE:Factory/foo/_service";
    const PROJECT_DIRECTORY_URL: &str = "https://api.opensuse.org/source/openSUSE:Factory/_project";
    const PROJECT_SERVICE_URL: &str =
        "https://api.opensuse.org/source/openSUSE:Factory/_project/_service";

    /// A directory listing carrying `_service`, plus `service_body`
    /// as the stored file when present.
    fn fake_with(dir_body: &str, service_body: Option<&str>) -> FakeHttp {
        let http = FakeHttp::new().with_get(DIRECTORY_URL, dir_body);
        match service_body {
            Some(body) => http.with_get(SERVICE_URL, body),
            None => http,
        }
    }

    /// The authed twin of `fake_with`: the directory listing and
    /// stored file behind the authenticated source endpoints.
    fn fake_with_authed(dir_body: &str, service_body: Option<&str>) -> FakeHttp {
        let http = FakeHttp::new().with_get(AUTHED_DIRECTORY_URL, dir_body);
        match service_body {
            Some(body) => http.with_get(AUTHED_SERVICE_URL, body),
            None => http,
        }
    }

    fn captured_lines(lines: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
        lines.lock().unwrap().clone()
    }

    /// Mixed modes (manual, disabled, trylocal, buildtime, default,
    /// serveronly), parameters preserved verbatim including empty
    /// values and an entity.
    const SV_MIXED_ALL: &str = r#"<services>
  <service name="download_url" mode="manual">
    <param name="host">example.com</param>
    <param name="protocol">https</param>
    <param name="query">a&amp;b</param>
  </service>
  <service name="set_version" mode="trylocal">
    <param name="file">version.txt</param>
    <param name="empty"></param>
    <param name="blank"/>
  </service>
  <service name="tar_scm" mode="disabled"/>
  <service name="obs_scm" mode="buildtime">
    <param name="url">https://example.com/repo.git</param>
  </service>
  <service name="format_spec_file">
    <param name="files">*.spec</param>
  </service>
  <service name="run_source_validator" mode="serveronly"/>
</services>
"#;

    /// A mode the client does not know: stored verbatim, never a
    /// parse failure.
    const SV_UNKNOWN_MODE: &str = r#"<services>
  <service name="download_url" mode="bogus-mode">
    <param name="host">example.com</param>
  </service>
</services>
"#;

    /// A project-level `_service`: the `<link>` names the project
    /// the definitions are inherited from and is not a service.
    const SV_PROJECT_LINK: &str = r#"<services>
  <link project="openSUSE:Factory"/>
  <service name="download_url" mode="manual">
    <param name="host">example.com</param>
  </service>
</services>
"#;

    /// The `osc` invalid-`_service` fixture
    /// (`behave/fixtures/pac/_service-set_version-invalid`): the
    /// file is well-formed XML and parses cleanly — its "invalid"
    /// is execution-time (`set_version` cannot read a file named
    /// `invalid`), so the typed view returns the definition and the
    /// fail-closed case is the malformed fixtures below, not this
    /// one. Exact bytes from the fixture.
    const SV_INVALID_OSC: &str = "<services>\n  <service name=\"set_version\" mode=\"manual\">\n    <param name=\"fromfile\">invalid</param>\n  </service>\n</services>\n";

    /// An XML prolog, comments, and stray text around valid
    /// services: ignored, valid parse.
    const SV_PROLOG_COMMENT: &str = r#"<?xml version="1.0"?>
<services>
  <!-- a comment -->
  stray text
  <service name="download_url" mode="manual">
    <param name="host">example.com</param>
  </service>
  <!-- another -->
</services>
"#;

    /// A comment inside a service element, carrying a commented-out
    /// parameter: its text is listed on the service, valid parse.
    /// Mirrors the spec-cleaner
    /// `_service` file, where `obs_scm` carries
    /// `<!-- <param name="changesgenerate">enable</param> -->`.
    const SV_COMMENT_IN_SERVICE: &str = r#"<services>
  <service name="obs_scm" mode="manual">
    <param name="url">https://github.com/openSUSE/spec-cleaner.git</param>
    <param name="scm">git</param>
    <!-- <param name="changesgenerate">enable</param> -->
    <param name="versionformat">@PARENT_TAG@+git.%h</param>
  </service>
</services>
"#;

    /// A directory listing with a `_service` entry, no serviceinfo.
    const DIR_WITH_SERVICE: &str = r#"<directory name="foo"><entry name="_service" md5="aaa" size="1" mtime="1"/><entry name="foo.spec" md5="bbb" size="2" mtime="1"/></directory>"#;

    /// A directory listing with no `_service` entry and no
    /// serviceinfo.
    const DIR_NO_SERVICE: &str = r#"<directory name="foo"><entry name="foo.spec" md5="bbb" size="2" mtime="1"/></directory>"#;

    fn dir_with_serviceinfo(serviceinfo: &str) -> String {
        format!(
            r#"<directory name="foo"><entry name="foo.spec" md5="bbb" size="2" mtime="1"/>{serviceinfo}</directory>"#
        )
    }

    #[test]
    fn mixed_modes_parse_with_params_verbatim() {
        let services = parse_services(SV_MIXED_ALL).unwrap();
        assert_eq!(services.len(), 6);
        assert_eq!(services[0].name, "download_url");
        assert_eq!(services[0].mode, "manual");
        assert_eq!(
            services[0].params,
            vec![
                ServiceParam {
                    name: "host".to_string(),
                    value: "example.com".to_string(),
                },
                ServiceParam {
                    name: "protocol".to_string(),
                    value: "https".to_string(),
                },
                ServiceParam {
                    name: "query".to_string(),
                    value: "a&b".to_string(),
                },
            ]
        );
        assert_eq!(services[1].name, "set_version");
        assert_eq!(services[1].mode, "trylocal");
        assert_eq!(services[1].params[0].value, "version.txt");
        assert_eq!(services[1].params[1].value, "");
        assert_eq!(services[1].params[2].value, "");
        assert_eq!(services[2].name, "tar_scm");
        assert_eq!(services[2].mode, "disabled");
        assert!(services[2].params.is_empty());
        assert_eq!(services[3].mode, "buildtime");
        // No mode attribute: the default mode, stored as empty.
        assert_eq!(services[4].name, "format_spec_file");
        assert_eq!(services[4].mode, "");
        assert_eq!(services[5].mode, "serveronly");
    }

    #[test]
    fn param_value_is_text_before_the_first_child() {
        // osc's read() takes ElementTree .text: markup inside a
        // parameter is not part of its value.
        let services = parse_services(
            "<services><service name=\"download_url\"><param name=\"x\">a<b/>b</param><param name=\"y\"><b/></param></service></services>",
        )
        .unwrap();
        assert_eq!(services[0].params[0].value, "a");
        assert_eq!(services[0].params[1].value, "");
    }

    #[test]
    fn service_param_with_bare_lt_in_value_parses() {
        // A bare `<` in a parameter value is text, not a tag:
        // the file parses (the value itself is the text before
        // the first `<`, by the pre-existing rule).
        let services = parse_services(
            "<services><service name=\"download_url\" mode=\"manual\"><param name=\"x\">a < b</param></service></services>",
        )
        .unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].params.len(), 1);
        assert_eq!(services[0].params[0].value, "a ");
        // Same for `<` followed by a digit: it is text, not a tag.
        let services = parse_services(
            "<services><service name=\"download_url\" mode=\"manual\"><param name=\"x\">a <2 b</param></service></services>",
        )
        .unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].params.len(), 1);
        assert_eq!(services[0].params[0].value, "a ");
        // Same for `<` starting a tag-like span with no `>` of its
        // own: it is text, not a tag.
        let services = parse_services(
            "<services><service name=\"download_url\" mode=\"manual\"><param name=\"x\">a <b c</param></service></services>",
        )
        .unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].params.len(), 1);
        assert_eq!(services[0].params[0].value, "a ");
    }

    #[test]
    fn unknown_mode_is_stored_verbatim() {
        let services = parse_services(SV_UNKNOWN_MODE).unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].mode, "bogus-mode");
    }

    #[test]
    fn project_link_is_skipped_not_a_service() {
        let services = parse_services(SV_PROJECT_LINK).unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "download_url");
    }

    #[test]
    fn osc_invalid_fixture_parses_cleanly() {
        // The fixture's "invalid" is a runtime failure (the service
        // cannot read the file named `invalid`), not a parse failure.
        let services = parse_services(SV_INVALID_OSC).unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "set_version");
        assert_eq!(services[0].mode, "manual");
        assert_eq!(
            services[0].params,
            vec![ServiceParam {
                name: "fromfile".to_string(),
                value: "invalid".to_string(),
            }]
        );
    }

    #[test]
    fn empty_services_is_an_empty_list() {
        assert_eq!(parse_services("<services/>").unwrap(), Vec::new());
        assert_eq!(parse_services("<services></services>").unwrap(), Vec::new());
    }

    #[test]
    fn comments_and_pis_outside_the_root_are_ignored() {
        // A header comment before the root, a prolog plus a comment,
        // a DOCTYPE, and a trailing comment after the close: all
        // ignored, valid parse.
        for body in [
            "<!-- a header comment --><services><service name=\"download_url\" mode=\"manual\"/></services>",
            "<?xml version=\"1.0\" ?><!-- c --><services><service name=\"download_url\" mode=\"manual\"/></services>",
            "<services><service name=\"download_url\" mode=\"manual\"/></services><!-- done -->",
            "<!DOCTYPE services><services><service name=\"download_url\" mode=\"manual\"/></services>",
        ] {
            let services = parse_services(body).unwrap();
            assert_eq!(services.len(), 1);
            assert_eq!(services[0].name, "download_url");
        }
        // An unterminated comment before the root is malformed, not
        // an empty list.
        assert_eq!(
            parse_services("<!-- never closed <services/>")
                .unwrap_err()
                .kind(),
            ErrorKind::ServerError
        );
    }

    #[test]
    fn prolog_comments_and_stray_text_are_ignored() {
        let services = parse_services(SV_PROLOG_COMMENT).unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "download_url");
    }

    #[test]
    fn comment_inside_a_service_becomes_plaintext() {
        // The commented-out parameter is not a parameter: the
        // service parses with the three live parameters only, and
        // the comment's text is listed on the service.
        // `html2text` renders the commented-out element as its
        // text content: the element name and attributes are markup,
        // not text.
        let services = parse_services(SV_COMMENT_IN_SERVICE).unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "obs_scm");
        assert_eq!(services[0].mode, "manual");
        assert_eq!(
            services[0].params,
            vec![
                ServiceParam {
                    name: "url".to_string(),
                    value: "https://github.com/openSUSE/spec-cleaner.git".to_string(),
                },
                ServiceParam {
                    name: "scm".to_string(),
                    value: "git".to_string(),
                },
                ServiceParam {
                    name: "versionformat".to_string(),
                    value: "@PARENT_TAG@+git.%h".to_string(),
                },
            ]
        );
        assert_eq!(services[0].comments, vec!["enable".to_string()]);
    }

    #[test]
    fn comment_only_content_and_comment_before_the_root_parse() {
        // A file carrying only comments besides the services, a
        // service carrying only a comment (its text is listed on
        // the service), and a comment before the root element
        // (listed nowhere): all parse normally.
        let services = parse_services("<services><!-- only a comment --></services>").unwrap();
        assert!(services.is_empty());
        let services = parse_services(
            "<services><service name=\"download_url\" mode=\"manual\"><!-- only a comment --></service></services>",
        )
        .unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "download_url");
        assert!(services[0].params.is_empty());
        assert_eq!(services[0].comments, vec!["only a comment".to_string()]);
        let services = parse_services(
            "<!-- <service name=\"download_url\" mode=\"manual\"/> --><services><service name=\"download_url\" mode=\"manual\"/></services>",
        )
        .unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "download_url");
    }

    #[test]
    fn comment_html_converts_to_plaintext() {
        // Prose markup becomes its text; an empty comment and a
        // comment carrying only markup with no text are not listed;
        // several comments keep file order.
        let services = parse_services(
            r#"<services><service name="download_url" mode="manual"><!-- <p>Generated by foo</p> --><!----><!-- <br/> --><!-- second note --></service></services>"#,
        )
        .unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(
            services[0].comments,
            vec!["Generated by foo".to_string(), "second note".to_string()]
        );
    }

    #[test]
    fn comment_count_and_length_are_capped() {
        // Forty comments on one service: only the first 32 are
        // kept, in file order.
        let mut body = String::from("<services><service name=\"download_url\" mode=\"manual\">");
        for i in 0..40 {
            body.push_str(&format!("<!-- note {i} -->"));
        }
        body.push_str("</service></services>");
        let services = parse_services(&body).unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].comments.len(), 32);
        assert_eq!(services[0].comments[0], "note 0");
        assert_eq!(services[0].comments[31], "note 31");
        // A comment longer than the cap is truncated to its head,
        // not dropped.
        let long = "x".repeat(5000);
        let body = format!(
            "<services><service name=\"download_url\" mode=\"manual\"><!-- {long} --></service></services>"
        );
        let services = parse_services(&body).unwrap();
        assert_eq!(services[0].comments.len(), 1);
        assert_eq!(services[0].comments[0].chars().count(), 4096);
        assert!(services[0].comments[0].chars().all(|c| c == 'x'));
    }

    #[test]
    fn comment_terminal_escapes_are_stripped() {
        // End to end: the stored comment carries no terminal
        // control characters. The filter itself is pinned
        // directly by `strip_terminal_controls_removes_escapes_and_bidi_marks`.
        let services = parse_services(
            r#"<services><service name="download_url" mode="manual"><!-- &#27;[2J&#27;]0;title&#7; --></service></services>"#,
        )
        .unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].comments, vec!["[2J]0;title".to_string()]);
        assert!(services[0].comments[0].chars().all(|c| !c.is_control()));
        let services = parse_services(
            r#"<services><service name="download_url" mode="manual"><!-- line one&#13;line two --></service></services>"#,
        )
        .unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].comments.len(), 1);
        assert!(!services[0].comments[0].contains('\r'));
        assert!(services[0].comments[0].contains("line one"));
        assert!(services[0].comments[0].contains("line two"));
    }

    #[test]
    fn service_names_modes_and_params_have_no_terminal_controls() {
        // The same untrusted file's other fields reach the
        // terminal raw through the service table, so the strip
        // covers the name, the mode, and parameter names and
        // values, not just comments.
        let services = parse_services(
            r#"<services><service name="a&#27;[2Jbc" mode="man&#7;ual"><param name="ho&#27;st">a&#27;[2Jb</param></service></services>"#,
        )
        .unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "a[2Jbc");
        assert_eq!(services[0].mode, "manual");
        assert_eq!(services[0].params.len(), 1);
        assert_eq!(services[0].params[0].name, "host");
        assert_eq!(services[0].params[0].value, "a[2Jb");
        assert!(services[0]
            .params
            .iter()
            .flat_map(|p| p.name.chars().chain(p.value.chars()))
            .all(|c| !c.is_control()));
    }

    #[test]
    fn long_comments_are_stored_unwrapped() {
        // The stored payload is not terminal output: a comment
        // longer than eighty columns keeps its line, and a long
        // unbroken word is never split.
        let long_word = "w".repeat(100);
        let body = format!(
            "<services><service name=\"download_url\" mode=\"manual\"><!-- This is a very long prose comment that keeps going well past eighty columns so that any wrapping at terminal width becomes visible in the stored text --><!-- {long_word} --></service></services>"
        );
        let services = parse_services(&body).unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].comments.len(), 2);
        assert!(!services[0].comments[0].contains('\n'));
        assert!(services[0].comments[0].starts_with("This is a very long prose comment"));
        assert_eq!(services[0].comments[1], long_word);
    }

    #[test]
    fn comment_rich_markup_renders_at_a_finite_width() {
        // A table, preformatted text, nested lists, and a
        // blockquote render without the unbounded-width
        // allocation: the conversion is bounded, and the content
        // still comes through as text.
        let services = parse_services(
            r#"<services><service name="download_url" mode="manual"><!-- <table><tr><td>alpha</td><td>beta</td></tr></table> --><!-- <pre>line1
line2</pre> --><!-- <ul><li>one<ul><li>inner</li></ul></li><li>two</li></ul> --><!-- <blockquote>quoted</blockquote> --></service></services>"#,
        )
        .unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].comments.len(), 4);
        assert!(services[0].comments[0].contains("alpha"));
        assert!(services[0].comments[0].contains("beta"));
        assert!(services[0].comments[1].contains("line1"));
        assert!(services[0].comments[1].contains("line2"));
        assert!(services[0].comments[2].contains("one"));
        assert!(services[0].comments[2].contains("inner"));
        assert!(services[0].comments[2].contains("two"));
        assert!(services[0].comments[3].contains("quoted"));
    }

    #[test]
    fn comment_prose_wraps_within_the_finite_width() {
        // Prose longer than the render width wraps: the stored
        // text carries a newline, which an unbounded width would
        // not produce. The length cap keeps the first 4096
        // characters, so the wrap must land inside them.
        let prose = "word ".repeat(1600);
        let body = format!(
            "<services><service name=\"download_url\" mode=\"manual\"><!-- {prose} --></service></services>"
        );
        let services = parse_services(&body).unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].comments.len(), 1);
        assert!(services[0].comments[0].contains('\n'));
    }

    #[test]
    fn comment_links_keep_text_without_footnotes() {
        // A link in a comment keeps its text, not a footnote
        // block appended to the stored comment.
        let services = parse_services(
            r#"<services><service name="download_url" mode="manual"><!-- see <a href="https://example.com/x">docs</a> for details --></service></services>"#,
        )
        .unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(
            services[0].comments,
            vec!["see [docs] for details".to_string()]
        );
    }

    #[test]
    fn cdata_sections_do_not_fail_the_parse() {
        // A CDATA section inside a service is character data, not
        // a comment: a `>` or `<!--` inside it must not fail the
        // file, and its content is not listed as a comment.
        let services = parse_services(
            r#"<services><service name="download_url" mode="manual"><![CDATA[ a > <!-- ]]><param name="host">example.com</param></service></services>"#,
        )
        .unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].params.len(), 1);
        assert!(services[0].comments.is_empty());
    }

    #[test]
    fn comments_nested_in_children_are_not_service_comments() {
        // A comment inside a parameter, or inside another child
        // element, belongs to that element: only direct-child
        // comments are listed on the service.
        let services = parse_services(
            r#"<services><service name="download_url" mode="manual"><param name="x">v<!-- inner note --></param><extra><!-- deep note --></extra><!-- direct note --></service></services>"#,
        )
        .unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].params.len(), 1);
        assert_eq!(services[0].params[0].name, "x");
        assert_eq!(services[0].params[0].value, "v");
        assert_eq!(services[0].comments, vec!["direct note".to_string()]);
    }

    #[test]
    fn malformed_services_fail_closed() {
        // Unterminated root.
        assert_eq!(
            parse_services("<services><service name=\"download_url\" mode=\"manual\">")
                .unwrap_err()
                .kind(),
            ErrorKind::ServerError
        );
        // Unclosed <service>: the close tag never arrives.
        assert_eq!(
            parse_services("<services><service name=\"download_url\" mode=\"manual\"></services>")
                .unwrap_err()
                .kind(),
            ErrorKind::ServerError
        );
        // A service without a name.
        assert_eq!(
            parse_services("<services><service mode=\"manual\"/></services>")
                .unwrap_err()
                .kind(),
            ErrorKind::ServerError
        );
        // A service name with a slash.
        assert_eq!(
            parse_services("<services><service name=\"bad/name\" mode=\"manual\"/></services>")
                .unwrap_err()
                .kind(),
            ErrorKind::ServerError
        );
        // A service name under three characters.
        assert_eq!(
            parse_services("<services><service name=\"ab\" mode=\"manual\"/></services>")
                .unwrap_err()
                .kind(),
            ErrorKind::ServerError
        );
        // A parameter without a name.
        assert_eq!(
            parse_services(
                "<services><service name=\"download_url\"><param>value</param></service></services>"
            )
            .unwrap_err()
            .kind(),
            ErrorKind::ServerError
        );
        // Not a <services> document at all.
        assert_eq!(
            parse_services("<notservices/>").unwrap_err().kind(),
            ErrorKind::ServerError
        );
        assert_eq!(
            parse_services("").unwrap_err().kind(),
            ErrorKind::ServerError
        );
        // A second root after the close.
        assert_eq!(
            parse_services("<services/><services/>").unwrap_err().kind(),
            ErrorKind::ServerError
        );
    }

    #[test]
    fn service_state_running() {
        let http = fake_with(
            &dir_with_serviceinfo(
                "<serviceinfo code=\"running\" xsrcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" lsrcmd5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"/>",
            ),
            None,
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.service_state("openSUSE:Factory", "foo").unwrap(),
            Some(ServiceState {
                code: ServiceCode::Running,
                xsrcmd5: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                lsrcmd5: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
                error: None,
            })
        );
    }

    #[test]
    fn service_state_failed_carries_error() {
        let http = fake_with(
            &dir_with_serviceinfo(
                "<serviceinfo code=\"failed\" xsrcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" lsrcmd5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"><error>service run failed: download_url: 404 Not Found\nsecond line</error></serviceinfo>",
            ),
            None,
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.service_state("openSUSE:Factory", "foo").unwrap(),
            Some(ServiceState {
                code: ServiceCode::Failed,
                xsrcmd5: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                lsrcmd5: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
                error: Some(
                    "service run failed: download_url: 404 Not Found\nsecond line".to_string()
                ),
            })
        );
    }

    #[test]
    fn service_state_error_strips_terminal_controls() {
        // The error is server text, and the typed value is printed
        // raw to the terminal (state render, working-copy failure
        // message): escape sequences, carriage returns, bells, and
        // bidirectional marks are stripped at parse, while the
        // error's own newlines survive. The entity-decoded form
        // (`&#27;`) is stripped too: the strip runs after unescape.
        let http = fake_with(
            &dir_with_serviceinfo(
                "<serviceinfo code=\"failed\" xsrcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" lsrcmd5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"><error>bad\u{1b}[2Jline\rwith\u{7}bell\nand\u{202e}bidi</error></serviceinfo>",
            ),
            None,
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.service_state("openSUSE:Factory", "foo").unwrap(),
            Some(ServiceState {
                code: ServiceCode::Failed,
                xsrcmd5: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                lsrcmd5: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
                error: Some("bad[2Jlinewithbell\nandbidi".to_string()),
            })
        );
        let http = fake_with(
            &dir_with_serviceinfo(
                "<serviceinfo code=\"failed\"><error>bad&#27;[2Jline</error></serviceinfo>",
            ),
            None,
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .service_state("openSUSE:Factory", "foo")
                .unwrap()
                .and_then(|state| state.error),
            Some("bad[2Jline".to_string())
        );
    }

    #[test]
    fn service_state_succeeded() {
        let http = fake_with(
            &dir_with_serviceinfo(
                "<serviceinfo code=\"succeeded\" xsrcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" lsrcmd5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"/>",
            ),
            None,
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.service_state("openSUSE:Factory", "foo").unwrap(),
            Some(ServiceState {
                code: ServiceCode::Succeeded,
                xsrcmd5: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                lsrcmd5: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
                error: None,
            })
        );
    }

    #[test]
    fn service_state_none_without_serviceinfo() {
        let http = fake_with(DIR_NO_SERVICE, None);
        let client = ObsClient::new(&http);
        assert_eq!(
            client.service_state("openSUSE:Factory", "foo").unwrap(),
            None
        );
    }

    #[test]
    fn service_state_unknown_code_is_malformed() {
        let http = fake_with(&dir_with_serviceinfo("<serviceinfo code=\"weird\"/>"), None);
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .service_state("openSUSE:Factory", "foo")
                .unwrap_err()
                .kind(),
            ErrorKind::ServerError
        );
        let http = fake_with(&dir_with_serviceinfo("<serviceinfo/>"), None);
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .service_state("openSUSE:Factory", "foo")
                .unwrap_err()
                .kind(),
            ErrorKind::ServerError
        );
    }

    #[test]
    fn service_state_unterminated_serviceinfo_is_malformed() {
        // An open tag that never ends.
        let http = fake_with(&dir_with_serviceinfo("<serviceinfo code=\"failed\""), None);
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .service_state("openSUSE:Factory", "foo")
                .unwrap_err()
                .kind(),
            ErrorKind::ServerError
        );
        // An element that never closes.
        let http = fake_with(&dir_with_serviceinfo("<serviceinfo code=\"failed\">"), None);
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .service_state("openSUSE:Factory", "foo")
                .unwrap_err()
                .kind(),
            ErrorKind::ServerError
        );
        // An <error> that never closes is malformed, not a failure
        // without text.
        let http = fake_with(
            &dir_with_serviceinfo("<serviceinfo code=\"failed\"><error>oops</serviceinfo>"),
            None,
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .service_state("openSUSE:Factory", "foo")
                .unwrap_err()
                .kind(),
            ErrorKind::ServerError
        );
        // A self-closing <error/> is a failure without text, not a
        // malformed listing.
        let http = fake_with(
            &dir_with_serviceinfo("<serviceinfo code=\"failed\"><error/></serviceinfo>"),
            None,
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.service_state("openSUSE:Factory", "foo").unwrap(),
            Some(ServiceState {
                code: ServiceCode::Failed,
                xsrcmd5: None,
                lsrcmd5: None,
                error: None,
            })
        );
    }

    #[test]
    fn service_state_second_error_is_malformed() {
        // The server writes at most one <error>: a second one
        // behind a first self-closing <error/> is malformed,
        // whether it never closes, carries text, or is
        // self-closing — it must not be silently dropped behind
        // the first self-closing one.
        for serviceinfo in [
            "<serviceinfo code=\"failed\"><error/><error></serviceinfo>",
            "<serviceinfo code=\"failed\"><error/><error>text</error></serviceinfo>",
            "<serviceinfo code=\"failed\"><error/><error/></serviceinfo>",
        ] {
            let http = fake_with(&dir_with_serviceinfo(serviceinfo), None);
            let client = ObsClient::new(&http);
            assert_eq!(
                client
                    .service_state("openSUSE:Factory", "foo")
                    .unwrap_err()
                    .kind(),
                ErrorKind::ServerError
            );
        }
    }

    #[test]
    fn service_state_missing_package_is_not_found() {
        let http = FakeHttp::new().with_get_error(DIRECTORY_URL, ErrorKind::NotFound);
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .service_state("openSUSE:Factory", "foo")
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
    }

    #[test]
    fn service_state_on_project_uses_the_authed_listing() {
        // `_project` is not a Package row, so the public listing
        // 404s it; the read goes through the authed endpoint.
        let http = FakeHttp::new().with_get(PROJECT_DIRECTORY_URL, DIR_NO_SERVICE);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .service_state("openSUSE:Factory", "_project")
                .unwrap(),
            None
        );
        assert_eq!(
            http.get_calls(),
            vec![(PROJECT_DIRECTORY_URL.to_string(), true)]
        );
    }

    #[test]
    fn list_services_round_trips_the_stored_file() {
        let http = fake_with(DIR_WITH_SERVICE, Some(SV_MIXED_ALL));
        let client = ObsClient::new(&http);
        let services = client
            .list_services("openSUSE:Factory", "foo")
            .unwrap()
            .unwrap();
        assert_eq!(services.len(), 6);
        assert_eq!(services[0].name, "download_url");
        assert_eq!(services[4].name, "format_spec_file");
        assert_eq!(services[4].mode, "");
        // Directory first, then the file: two public reads.
        assert_eq!(
            http.get_calls(),
            vec![
                (DIRECTORY_URL.to_string(), false),
                (SERVICE_URL.to_string(), false),
            ]
        );
    }

    #[test]
    fn list_services_none_without_service_file() {
        let http = fake_with(DIR_NO_SERVICE, None);
        let client = ObsClient::new(&http);
        assert_eq!(
            client.list_services("openSUSE:Factory", "foo").unwrap(),
            None
        );
        // No file, so no cat: one read only.
        assert_eq!(http.get_calls(), vec![(DIRECTORY_URL.to_string(), false)]);
    }

    #[test]
    fn list_services_empty_file_is_an_empty_list() {
        let http = fake_with(DIR_WITH_SERVICE, Some("<services/>"));
        let client = ObsClient::new(&http);
        assert_eq!(
            client.list_services("openSUSE:Factory", "foo").unwrap(),
            Some(Vec::new())
        );
    }

    #[test]
    fn list_services_malformed_file_fails_closed() {
        let http = fake_with(
            DIR_WITH_SERVICE,
            Some("<services><service mode=\"manual\"/></services>"),
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .list_services("openSUSE:Factory", "foo")
                .unwrap_err()
                .kind(),
            ErrorKind::ServerError
        );
    }

    #[test]
    fn list_services_missing_package_is_not_found() {
        let http = FakeHttp::new().with_get_error(DIRECTORY_URL, ErrorKind::NotFound);
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .list_services("openSUSE:Factory", "foo")
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
    }

    #[test]
    fn list_services_on_project_uses_the_authed_endpoints() {
        // Both reads — directory and file — go through the authed
        // endpoints for `_project`; the public routes 404 it.
        let http = FakeHttp::new()
            .with_get(PROJECT_DIRECTORY_URL, DIR_WITH_SERVICE)
            .with_get(PROJECT_SERVICE_URL, SV_PROJECT_LINK);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let services = client
            .list_services("openSUSE:Factory", "_project")
            .unwrap()
            .unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "download_url");
        // Directory first, then the file: two authed reads.
        assert_eq!(
            http.get_calls(),
            vec![
                (PROJECT_DIRECTORY_URL.to_string(), true),
                (PROJECT_SERVICE_URL.to_string(), true),
            ]
        );
    }

    const TRIGGER_URL: &str = "https://api.opensuse.org/source/openSUSE:Factory/foo?cmd=runservice";

    fn authed_client(http: &FakeHttp) -> ObsClient<'_> {
        ObsClient::with_credentials(http, basic_credentials()).unwrap()
    }

    #[test]
    fn trigger_posts_runservice_and_returns_ok() {
        let http = FakeHttp::new().with_post(TRIGGER_URL, "<status code=\"ok\"/>");
        let client = authed_client(&http);
        assert_eq!(
            client
                .trigger_service("openSUSE:Factory", "foo", None, &DryRun::new(false))
                .unwrap(),
            Some(ServiceTrigger {
                service: None,
                code: "ok".to_string(),
            })
        );
        // One authed POST, no reads: the trigger needs no `_service`
        // lookup when no service is named.
        assert_eq!(
            http.post_calls(),
            vec![(TRIGGER_URL.to_string(), String::new(), true)]
        );
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn trigger_with_named_service_validates_against_the_stored_file() {
        let http = fake_with_authed(DIR_WITH_SERVICE, Some(SV_MIXED_ALL))
            .with_post(TRIGGER_URL, "<status code=\"ok\"/>");
        let client = authed_client(&http);
        assert_eq!(
            client
                .trigger_service(
                    "openSUSE:Factory",
                    "foo",
                    Some("download_url"),
                    &DryRun::new(false),
                )
                .unwrap(),
            Some(ServiceTrigger {
                service: Some("download_url".to_string()),
                code: "ok".to_string(),
            })
        );
        // The stored file is read first (authed reads: the client
        // has credentials), then the trigger posts.
        assert_eq!(
            http.get_calls(),
            vec![
                (AUTHED_DIRECTORY_URL.to_string(), true),
                (AUTHED_SERVICE_URL.to_string(), true),
            ]
        );
        assert_eq!(
            http.post_calls(),
            vec![(TRIGGER_URL.to_string(), String::new(), true)]
        );
    }

    #[test]
    fn trigger_with_unknown_service_is_not_found_without_a_post() {
        let http = fake_with_authed(DIR_WITH_SERVICE, Some(SV_MIXED_ALL));
        let client = authed_client(&http);
        assert_eq!(
            client
                .trigger_service(
                    "openSUSE:Factory",
                    "foo",
                    Some("no_such_service"),
                    &DryRun::new(false),
                )
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn trigger_with_named_service_and_no_service_file_is_not_found() {
        let http = fake_with_authed(DIR_NO_SERVICE, None);
        let client = authed_client(&http);
        assert_eq!(
            client
                .trigger_service(
                    "openSUSE:Factory",
                    "foo",
                    Some("download_url"),
                    &DryRun::new(false),
                )
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn trigger_without_service_file_fails_on_the_server_refusal() {
        // No `--service` to validate, so the POST goes out and the
        // server refuses: no service defined is its 404.
        let http = FakeHttp::new().with_post_error(TRIGGER_URL, ErrorKind::NotFound);
        let client = authed_client(&http);
        assert_eq!(
            client
                .trigger_service("openSUSE:Factory", "foo", None, &DryRun::new(false))
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            http.post_calls(),
            vec![(TRIGGER_URL.to_string(), String::new(), true)]
        );
    }

    #[test]
    fn trigger_missing_package_is_not_found() {
        let http = FakeHttp::new().with_post_error(TRIGGER_URL, ErrorKind::NotFound);
        let client = authed_client(&http);
        assert_eq!(
            client
                .trigger_service("openSUSE:Factory", "foo", None, &DryRun::new(false))
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        // The POST went out: the 404 is the server's answer, not a
        // client-side refusal before any request.
        assert_eq!(
            http.post_calls(),
            vec![(TRIGGER_URL.to_string(), String::new(), true)]
        );
    }

    #[test]
    fn trigger_non_ok_answer_is_a_server_error() {
        for body in ["<status code=\"error\"/>", ""] {
            let http = FakeHttp::new().with_post(TRIGGER_URL, body);
            let client = authed_client(&http);
            assert_eq!(
                client
                    .trigger_service("openSUSE:Factory", "foo", None, &DryRun::new(false))
                    .unwrap_err()
                    .kind(),
                ErrorKind::ServerError
            );
        }
    }

    #[test]
    fn trigger_invalid_names_fail_closed() {
        let http = FakeHttp::new().with_post(TRIGGER_URL, "<status code=\"ok\"/>");
        let client = authed_client(&http);
        for (project, package, service) in [
            ("bad/project", "foo", None),
            ("openSUSE:Factory", "bad/package", None),
            ("openSUSE:Factory", "foo", Some("ab")),
            ("openSUSE:Factory", "foo", Some("bad/name")),
            ("openSUSE:Factory", "foo", Some("")),
        ] {
            assert_eq!(
                client
                    .trigger_service(project, package, service, &DryRun::new(false))
                    .unwrap_err()
                    .kind(),
                ErrorKind::InvalidInput,
                "project={project} package={package} service={service:?}"
            );
        }
        assert!(http.post_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn trigger_without_credentials_fails_before_any_request() {
        let http = FakeHttp::new().with_post(TRIGGER_URL, "<status code=\"ok\"/>");
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .trigger_service("openSUSE:Factory", "foo", None, &DryRun::new(false))
                .unwrap_err()
                .kind(),
            ErrorKind::ConfigError
        );
        assert!(http.post_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn trigger_named_service_without_credentials_fails_before_any_request() {
        // Live with a named service and no credentials: the
        // configuration error, not the public-path read's 404
        // for a credentials-only package — and no request at all
        // goes out.
        let http = FakeHttp::new().with_post(TRIGGER_URL, "<status code=\"ok\"/>");
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .trigger_service(
                    "openSUSE:Factory",
                    "foo",
                    Some("download_url"),
                    &DryRun::new(false),
                )
                .unwrap_err()
                .kind(),
            ErrorKind::ConfigError
        );
        assert!(http.post_calls().is_empty());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn trigger_dry_run_records_the_plan_and_posts_nothing() {
        let http = fake_with_authed(DIR_WITH_SERVICE, Some(SV_MIXED_ALL))
            .with_post(TRIGGER_URL, "<status code=\"ok\"/>");
        let client = authed_client(&http);
        let dry_run = DryRun::new(true);
        assert_eq!(
            client
                .trigger_service("openSUSE:Factory", "foo", None, &dry_run)
                .unwrap(),
            None
        );
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: trigger services for openSUSE:Factory/foo".to_string()]
        );
        assert!(http.post_calls().is_empty());
        assert!(http.get_calls().is_empty());
        // The named form plans the name too — the `_service`
        // validation read still runs (read-only, before the
        // dry-run guard), but nothing is posted.
        let dry_run = DryRun::new(true);
        assert_eq!(
            client
                .trigger_service("openSUSE:Factory", "foo", Some("download_url"), &dry_run,)
                .unwrap(),
            None
        );
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: trigger services for openSUSE:Factory/foo (service download_url)"
                    .to_string()
            ]
        );
        assert!(http.post_calls().is_empty());
        assert_eq!(
            http.get_calls(),
            vec![
                (AUTHED_DIRECTORY_URL.to_string(), true),
                (AUTHED_SERVICE_URL.to_string(), true),
            ]
        );
    }

    #[test]
    fn trigger_dry_run_with_unknown_service_is_not_found() {
        // The name check runs before the dry-run guard: a typo
        // fails in dry-run instead of reporting a clean plan, and
        // no plan is recorded.
        let http = fake_with_authed(DIR_WITH_SERVICE, Some(SV_MIXED_ALL));
        let client = authed_client(&http);
        let dry_run = DryRun::new(true);
        assert_eq!(
            client
                .trigger_service("openSUSE:Factory", "foo", Some("no_such_service"), &dry_run,)
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        assert!(dry_run.plan().is_empty());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn trigger_dry_run_with_unreadable_service_file_plans_without_the_check() {
        // The stored file cannot be read (no canned response: the
        // package is missing or not readable): dry-run plans
        // without the name check instead of failing the plan on a
        // read it cannot make. Live still fails on the read error.
        let http = FakeHttp::new();
        let client = authed_client(&http);
        let dry_run = DryRun::new(true);
        assert_eq!(
            client
                .trigger_service("openSUSE:Factory", "foo", Some("download_url"), &dry_run,)
                .unwrap(),
            None
        );
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: trigger services for openSUSE:Factory/foo (service download_url)"
                    .to_string()
            ]
        );
        assert!(http.post_calls().is_empty());
        let err = client
            .trigger_service(
                "openSUSE:Factory",
                "foo",
                Some("download_url"),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NetworkError);
    }

    #[test]
    fn trigger_dry_run_without_credentials_validates_on_the_public_path() {
        // No credentials, so the name check reads the public
        // `_service`: the plan is still recorded, and no POST goes
        // out — the credentials check runs after the dry-run guard.
        let http = fake_with(DIR_WITH_SERVICE, Some(SV_MIXED_ALL));
        let client = ObsClient::new(&http);
        let dry_run = DryRun::new(true);
        assert_eq!(
            client
                .trigger_service("openSUSE:Factory", "foo", Some("download_url"), &dry_run,)
                .unwrap(),
            None
        );
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: trigger services for openSUSE:Factory/foo (service download_url)"
                    .to_string()
            ]
        );
        assert_eq!(
            http.get_calls(),
            vec![
                (DIRECTORY_URL.to_string(), false),
                (SERVICE_URL.to_string(), false),
            ]
        );
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn trigger_debug_channel_carries_decision_lines_only() {
        let (debug, lines) = Debug::captured(true, false);
        let http = fake_with_authed(DIR_WITH_SERVICE, Some(SV_MIXED_ALL))
            .with_post(TRIGGER_URL, "<status code=\"ok\"/>");
        let client = authed_client(&http).with_debug(debug);
        let _ = client.trigger_service("openSUSE:Factory", "foo", None, &DryRun::new(false));
        let _ = client.trigger_service(
            "openSUSE:Factory",
            "foo",
            Some("download_url"),
            &DryRun::new(false),
        );
        assert_eq!(
            captured_lines(&lines),
            vec![
                "trigger_service openSUSE:Factory/foo: ok".to_string(),
                "list_services openSUSE:Factory/foo: 6 service(s)".to_string(),
                "trigger_service openSUSE:Factory/foo: ok (service download_url)".to_string(),
            ]
        );
    }

    #[test]
    fn reads_are_not_dry_run_guarded() {
        // The read operations carry no DryRun parameter — there is
        // no guard to test — so this characterizes the property
        // instead: the reads still send their requests. A dry run
        // suppresses mutations, never reads.
        let http = fake_with(DIR_WITH_SERVICE, Some(SV_MIXED_ALL));
        let client = ObsClient::new(&http);
        assert!(client.service_state("openSUSE:Factory", "foo").is_ok());
        assert!(client.list_services("openSUSE:Factory", "foo").is_ok());
        assert_eq!(
            http.get_calls(),
            vec![
                (DIRECTORY_URL.to_string(), false),
                (DIRECTORY_URL.to_string(), false),
                (SERVICE_URL.to_string(), false),
            ]
        );
    }

    #[test]
    fn debug_channel_carries_decision_lines_only() {
        let (debug, lines) = Debug::captured(true, false);
        let http = fake_with(DIR_WITH_SERVICE, Some(SV_MIXED_ALL));
        let client = ObsClient::new(&http).with_debug(debug);
        let _ = client.service_state("openSUSE:Factory", "foo");
        let _ = client.list_services("openSUSE:Factory", "foo");
        assert_eq!(
            captured_lines(&lines),
            vec![
                "service_state openSUSE:Factory/foo: no serviceinfo".to_string(),
                "list_services openSUSE:Factory/foo: 6 service(s)".to_string(),
            ]
        );
    }

    #[test]
    fn debug_channel_never_prints_service_text_or_error_text() {
        // Even with bodies enabled, the call sites print only the
        // decision line: the _service text and the error text are
        // bodies, printed by the transport layer behind
        // --debug-body, never by the operation itself.
        let (debug, lines) = Debug::captured(true, true);
        let http = fake_with(
            "<directory name=\"foo\"><entry name=\"_service\" md5=\"aaa\" size=\"1\" mtime=\"1\"/><entry name=\"foo.spec\" md5=\"bbb\" size=\"2\" mtime=\"1\"/><serviceinfo code=\"failed\"><error>secret failure text</error></serviceinfo></directory>",
            Some(SV_MIXED_ALL),
        );
        let client = ObsClient::new(&http).with_debug(debug);
        let _ = client.service_state("openSUSE:Factory", "foo");
        let _ = client.list_services("openSUSE:Factory", "foo");
        let lines = captured_lines(&lines);
        assert_eq!(
            lines,
            vec![
                "service_state openSUSE:Factory/foo: failed".to_string(),
                "list_services openSUSE:Factory/foo: 6 service(s)".to_string(),
            ]
        );
        assert!(!lines
            .iter()
            .any(|line| line.contains("secret failure text")));
        assert!(!lines.iter().any(|line| line.contains("download_url")));
    }

    #[test]
    fn debug_disabled_emits_nothing() {
        let (debug, lines) = Debug::captured(false, false);
        let http = fake_with(DIR_WITH_SERVICE, Some(SV_MIXED_ALL));
        let client = ObsClient::new(&http).with_debug(debug);
        let _ = client.service_state("openSUSE:Factory", "foo");
        let _ = client.list_services("openSUSE:Factory", "foo");
        assert!(captured_lines(&lines).is_empty());
    }
}
