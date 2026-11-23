//! OBS service state and service definitions: the two read-only
//! operations the Factory scope filter keeps. A service run is
//! triggered by a commit, never by this library: [`ObsClient::service_state`]
//! reads the run state the server recorded in the directory listing,
//! and [`ObsClient::list_services`] reads the stored `_service` text
//! as typed definitions. Trigger, wait, and merge are out under the
//! Factory scope filter (design §4): each exists only to drive
//! server-side runs, which Factory declines. The commit path is intended to reuse
//! `service_state` as its settlement read.

use super::tag_attr;
use super::DirectoryListing;
use super::ObsClient;
use super::ServiceState;
use crate::debug::format_body_preview;
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
    /// the markup.
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

/// Convert an HTML fragment to plain text: a comment in a stored
/// `_service` file is markup (a commented-out element, a prose
/// note), and the typed view carries its text, not the tags.
/// The render is unwrapped: the width is the stored payload, not
/// terminal output, so a long comment stays one line and a long
/// word is never split. Link footnotes are off
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
    html2text::config::plain_no_decorate()
        .string_from_read(html.as_bytes(), usize::MAX)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Parse a stored `_service` file into typed service definitions.
///
/// The parse follows the server's rule (`osc`'s
/// `Serviceinfo.read`): a service needs a `name` of at least three
/// characters with no `/`, and every parameter needs a `name`. The
/// mode is stored verbatim, including modes this client does not
/// know. A `<link>` element is skipped, not parsed into a service:
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
        let Some(name) = tag_attr(open_tag, "name").map(|value| xml_unescape(&value)) else {
            return Err(malformed());
        };
        if name.chars().count() < 3 || name.contains('/') {
            return Err(malformed());
        }
        let mode = tag_attr(open_tag, "mode")
            .map(|value| xml_unescape(&value))
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
                let Some(param_name) =
                    tag_attr(param_tag, "name").map(|value| xml_unescape(&value))
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
                    xml_unescape(text)
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
            self.authed_directory_listing(project, package)
        } else {
            self.directory_listing(project, package, None)
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
        let listing = self.service_directory_listing(project, package)?;
        if !listing.entries.iter().any(|entry| entry.name == "_service") {
            self.debug.emit(&format!(
                "list_services {project}/{package}: no _service file"
            ));
            return Ok(None);
        }
        let body = self.service_file_body(project, package)?;
        let services = parse_services(&body)?;
        self.debug.emit(&format!(
            "list_services {project}/{package}: {} service(s)",
            services.len()
        ));
        Ok(Some(services))
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
                "<serviceinfo code=\"running\" xsrcmd5=\"aaa\" lsrcmd5=\"bbb\"/>",
            ),
            None,
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.service_state("openSUSE:Factory", "foo").unwrap(),
            Some(ServiceState {
                code: ServiceCode::Running,
                xsrcmd5: Some("aaa".to_string()),
                lsrcmd5: Some("bbb".to_string()),
                error: None,
            })
        );
    }

    #[test]
    fn service_state_failed_carries_error_verbatim() {
        let http = fake_with(
            &dir_with_serviceinfo(
                "<serviceinfo code=\"failed\" xsrcmd5=\"aaa\" lsrcmd5=\"bbb\"><error>service run failed: download_url: 404 Not Found\nsecond line</error></serviceinfo>",
            ),
            None,
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.service_state("openSUSE:Factory", "foo").unwrap(),
            Some(ServiceState {
                code: ServiceCode::Failed,
                xsrcmd5: Some("aaa".to_string()),
                lsrcmd5: Some("bbb".to_string()),
                error: Some(
                    "service run failed: download_url: 404 Not Found\nsecond line".to_string()
                ),
            })
        );
    }

    #[test]
    fn service_state_succeeded() {
        let http = fake_with(
            &dir_with_serviceinfo(
                "<serviceinfo code=\"succeeded\" xsrcmd5=\"aaa\" lsrcmd5=\"bbb\"/>",
            ),
            None,
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.service_state("openSUSE:Factory", "foo").unwrap(),
            Some(ServiceState {
                code: ServiceCode::Succeeded,
                xsrcmd5: Some("aaa".to_string()),
                lsrcmd5: Some("bbb".to_string()),
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
