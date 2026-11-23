use super::build_link_meta;
use super::build_url;
use super::element_text;
use super::elements;
use super::filter_history;
use super::meta_envelope;
use super::meta_roles;
use super::parse_search_matches;
use super::parse_status_invoked;
use super::parse_status_ok;
use super::tag_attr;
use super::valid_obs_filename;
use super::valid_obs_name;
use super::AggregateOptions;
use super::CreatePackageOptions;
use super::DeleteProjectOptions;
use super::DirectoryListing;
use super::DryRun;
use super::FilePatch;
use super::FilePatchStatus;
use super::HistoryEntry;
use super::NamedRole;
use super::ObsClient;
use super::Owner;
use super::PackageDiff;
use super::PackageDiffStatus;
use super::PackageMatch;
use super::PackageMeta;
use super::Person;
use super::ProjectMatch;
use super::ProjectMeta;
use super::ReleaseOptions;
use super::Repository;
use super::RevisionPatch;
use super::RevisionRange;
use super::RevisionSpec;
use super::ServiceCode;
use super::ServiceState;
use super::SourceFile;
use super::FACTORY_PROJECT;
use crate::auth::Credentials;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use crate::xml::find_attr;
use crate::xml::find_tag_end;
use crate::xml::scan_tag;
use crate::xml::skip_xml_prolog;
use crate::xml::xml_escape;
use crate::xml::xml_unescape;
use crate::xml::XmlDoc;
use std::path::Path;

/// Releasing to an explicit target needs one source repository and
/// one target repository to be unambiguous, and a target repository
/// needs its target project: refuse the ambiguous combination
/// client-side instead of failing on the server.
fn validate_release_options(options: &ReleaseOptions) -> Result<(), GeectlError> {
    if options.target_repository.is_some() && options.target_project.is_none() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "releasing to a target repository needs a target project",
        ));
    }
    if options.target_project.is_some()
        && (options.repository.is_none() || options.target_repository.is_none())
    {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "releasing to a target project needs a repository and a target repository",
        ));
    }
    Ok(())
}

/// The option suffix for an aggregate plan line: the set options, in
/// the order they appear in [`AggregateOptions`], so a dry-run plan
/// reads what the aggregate would have done.
fn aggregate_option_suffix(options: &AggregateOptions) -> String {
    let mut flags: Vec<String> = Vec::new();
    if options.no_sources {
        flags.push("no sources".to_string());
    }
    if options.disable_publish {
        flags.push("publish disabled".to_string());
    }
    if !options.repository_map.is_empty() {
        flags.push(format!(
            "{} repository mapping{}",
            options.repository_map.len(),
            if options.repository_map.len() == 1 {
                ""
            } else {
                "s"
            }
        ));
    }
    if flags.is_empty() {
        String::new()
    } else {
        format!(" ({})", flags.join(", "))
    }
}

/// The option suffix for a release plan line: the set options, so a
/// dry-run plan reads what the release would have done.
fn release_option_suffix(options: &ReleaseOptions) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(repository) = &options.repository {
        parts.push(format!("repository {repository}"));
    }
    if let Some(architecture) = &options.architecture {
        parts.push(format!("architecture {architecture}"));
    }
    if let Some(target_project) = &options.target_project {
        match &options.target_repository {
            Some(target_repository) => {
                parts.push(format!("to {target_project}/{target_repository}"));
            }
            None => parts.push(format!("to {target_project}")),
        }
    }
    if let Some(set_release) = &options.set_release {
        parts.push(format!("release tag {set_release}"));
    }
    if options.no_delay {
        parts.push("no delay".to_string());
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// A scmsync URL names the branch to sync in its fragment
/// (`https://example.com/user/repo#main`): refuse a value without
/// one — a scmsync with no branch names nothing to check out. The
/// server stores the value as an opaque string and does not validate
/// it, so this client-side check is the only guard.
fn validate_scmsync(scmsync: &str) -> Result<(), GeectlError> {
    let fragment = scmsync.split('#').nth(1);
    match fragment {
        Some(fragment) if !fragment.is_empty() => Ok(()),
        _ => Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "scmsync must include a fragment with the branch name (e.g. https://example.com/user/repo#main)",
        )),
    }
}

/// The option suffix for a package-creation plan line: the set
/// options, so a dry-run plan reads what the creation would have
/// done.
fn create_package_option_suffix(options: &CreatePackageOptions) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(scmsync) = &options.scmsync {
        parts.push(format!("scmsync {scmsync}"));
    }
    if options.force {
        parts.push("force".to_string());
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", parts.join(", "))
    }
}

/// Extract the Version field from a spec file. Returns None if absent.
pub(crate) fn parse_spec_version(spec_text: &str) -> Option<String> {
    for line in spec_text.lines() {
        if let Some(rest) = line.strip_prefix("Version:") {
            let version = rest.trim().to_string();
            if !version.is_empty() {
                return Some(version);
            }
        }
    }
    None
}

/// A package's `<linkinfo>` from an OBS directory listing: the link
/// target, the revisions the link was expanded at, and the expansion
/// error when the link is broken. All fields are optional: a
/// directory without a link carries no linkinfo at all, and a
/// working link carries no error.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LinkInfo {
    /// The project the link points at.
    pub project: Option<String>,
    /// The package the link points at.
    pub package: Option<String>,
    /// The link target's srcmd5 at this expansion.
    pub srcmd5: Option<String>,
    /// The target revision the link is based at (the link's baserev).
    pub baserev: Option<String>,
    /// The linked package's expanded srcmd5.
    pub xsrcmd5: Option<String>,
    /// The linked package's srcmd5 at the last revision where the
    /// link expanded, when the server was asked for it.
    pub lastworking: Option<String>,
    /// The expansion error when the link is broken; absent when the
    /// link expands.
    pub error: Option<String>,
}

/// Read the `<linkinfo>` element from an OBS directory listing.
/// None when the listing carries no linkinfo (the package is not a
/// link). An error when the body is not a `<directory>` document at
/// all — a truncated 200 body must not read as "not a link". The
/// attributes are read from the open tag only, so a linkinfo with
/// `<linked>` children still parses.
pub(crate) fn parse_linkinfo(body: &str) -> Result<Option<LinkInfo>, GeectlError> {
    let body = body.trim();
    // A malformed linkinfo response still fails closed, but the error
    // carries a bounded preview of what was received, so the failure
    // is diagnosable instead of a bare "not a <directory> document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS directory response: not a <directory> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the <directory> element; skip it. The prolog must terminate
    // (`?>`) — an unterminated prolog is malformed, not a missing
    // <directory>.
    let body = match skip_xml_prolog(body) {
        Ok(body) => body,
        Err(()) => return Err(malformed()),
    };
    let Some(after_open) = body.strip_prefix("<directory") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = after_open.find('>') else {
        return Err(malformed());
    };
    if after_open[..end].trim_end().ends_with('/') {
        // Self-closing `<directory .../>`: no children, no linkinfo.
        return Ok(None);
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</directory>") else {
        return Err(malformed());
    };
    if !inner[close + "</directory>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let mut rest = &inner[..close];
    while let Some(pos) = rest.find("<linkinfo") {
        let after_tag = &rest[pos + "<linkinfo".len()..];
        let Some(first) = after_tag.chars().next() else {
            return Err(malformed());
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        // The tag end is scanned quote-aware: a `>` inside an
        // attribute value (the free-form `error` text) must not end
        // the tag, or a broken link would read as unbroken.
        let Some((end, _)) = scan_tag(rest, pos) else {
            return Err(malformed());
        };
        let tag = &rest[pos..end];
        let mut info = LinkInfo::default();
        if let Some(value) = find_attr(tag, "project") {
            info.project = Some(value);
        }
        if let Some(value) = find_attr(tag, "package") {
            info.package = Some(value);
        }
        if let Some(value) = find_attr(tag, "srcmd5") {
            info.srcmd5 = Some(value);
        }
        if let Some(value) = find_attr(tag, "baserev") {
            info.baserev = Some(value);
        }
        if let Some(value) = find_attr(tag, "xsrcmd5") {
            info.xsrcmd5 = Some(value);
        }
        if let Some(value) = find_attr(tag, "lastworking") {
            info.lastworking = Some(value);
        }
        if let Some(value) = find_attr(tag, "error") {
            info.error = Some(value);
        }
        return Ok(Some(info));
    }
    Ok(None)
}

/// The `rev`, `vrev`, and `srcmd5` attributes of an OBS directory
/// listing's root element, each None when the attribute is absent.
pub(crate) type DirectoryRev = (Option<String>, Option<String>, Option<String>);

/// The `rev`, `vrev`, and `srcmd5` attributes of an OBS directory
/// listing's root element. An error when the body is not a
/// `<directory>` document at all; None values when the attributes
/// are absent. A truncated body (no closing tag) is an error, not
/// an empty revision. The `srcmd5` is the expanded content's md5:
/// for an expanded fetch it is the revision to pin, where `rev`
/// is only the revision number.
pub(crate) fn parse_directory_rev(body: &str) -> Result<DirectoryRev, GeectlError> {
    let body = body.trim();
    // A malformed response still fails closed, but the error carries a
    // bounded preview of what was received, so the failure is
    // diagnosable instead of a bare "not a directory document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS directory response: not a <directory> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version=\"1.0\"?>`)
    // before the directory element; skip it. The prolog must
    // terminate (`?>`) — an unterminated prolog is malformed, not a
    // missing directory.
    let body = if let Some(after) = body.strip_prefix("<?xml") {
        let first = after.chars().next();
        if !matches!(first, Some(c) if c.is_whitespace() || c == '?') {
            return Err(malformed());
        }
        let Some(end) = after.find("?>") else {
            return Err(malformed());
        };
        after[end + 2..].trim_start()
    } else {
        body
    };
    let Some(after_open) = body.strip_prefix("<directory") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = after_open.find('>') else {
        return Err(malformed());
    };
    let tag = &after_open[..end];
    let self_closing = tag.ends_with('/');
    let rev = find_attr(tag, "rev");
    let vrev = find_attr(tag, "vrev");
    let srcmd5 = find_attr(tag, "srcmd5");
    let rest = &after_open[end + 1..];
    if self_closing {
        if !rest.trim().is_empty() {
            return Err(malformed());
        }
        return Ok((rev, vrev, srcmd5));
    }
    let Some(close) = rest.find("</directory>") else {
        return Err(malformed());
    };
    if !rest[close + "</directory>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    Ok((rev, vrev, srcmd5))
}

/// The package names in an OBS project directory listing: the
/// `name` of every `<entry>`. An error when the body is not a
/// `<directory>` document, or when an entry carries no name.
pub(crate) fn parse_package_names(body: &str) -> Result<Vec<String>, GeectlError> {
    let body = body.trim();
    // A malformed listing still fails closed, but the error carries a
    // bounded preview of what was received, so the failure is
    // diagnosable instead of a bare "not a directory document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS directory response: not a <directory> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the directory element; skip it. The prolog must
    // terminate (`?>`) — an unterminated prolog is malformed, not a
    // missing directory.
    let body = if let Some(after) = body.strip_prefix("<?xml") {
        let first = after.chars().next();
        if !matches!(first, Some(c) if c.is_whitespace() || c == '?') {
            return Err(malformed());
        }
        let Some(end) = after.find("?>") else {
            return Err(malformed());
        };
        after[end + 2..].trim_start()
    } else {
        body
    };
    let Some(after_open) = body.strip_prefix("<directory") else {
        return Err(malformed());
    };
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = after_open.find('>') else {
        return Err(malformed());
    };
    if after_open[..end].trim_end().ends_with('/') {
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</directory>") else {
        return Err(malformed());
    };
    if !inner[close + "</directory>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let mut names = Vec::new();
    let mut rest = &inner[..close];
    while let Some(pos) = rest.find("<entry") {
        let after_tag = &rest[pos + "<entry".len()..];
        let Some(first) = after_tag.chars().next() else {
            return Err(malformed());
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let Some(end) = after_tag.find('>') else {
            return Err(malformed());
        };
        let tag = &after_tag[..end];
        let Some(name) = find_attr(tag, "name") else {
            return Err(malformed());
        };
        names.push(name);
        rest = &after_tag[end..];
    }
    Ok(names)
}

pub(crate) fn parse_revision(body: &str) -> Option<u64> {
    let pos = body.find("<revision")?;
    let after_tag = &body[pos + "<revision".len()..];
    // The tag name must end here: `<revisions>` is not a revision
    // element. The next character is whitespace, `/`, or `>`.
    let first = after_tag.chars().next()?;
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return None;
    }
    // The revision number is the `rev` attribute of this element, not
    // text after it — and it must be the `rev` attribute, not a longer
    // name that ends in it (`vrev="3"` is not `rev`).
    let end = after_tag.find('>')?;
    let element = &after_tag[..end];
    let mut rest = element;
    while let Some(pos) = rest.find("rev=") {
        if pos > 0 && !rest.as_bytes()[pos - 1].is_ascii_whitespace() {
            rest = &rest[pos + "rev=".len()..];
            continue;
        }
        let after_eq = &rest[pos + "rev=".len()..];
        let quote = after_eq.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let close = after_eq[1..].find(quote)?;
        let value = &after_eq[1..1 + close];
        let number: u64 = value.parse().ok()?;
        if number == 0 {
            return None;
        }
        return Some(number);
    }
    None
}

/// Parse an OBS source directory listing (`<directory>...</directory>`
/// with `<entry name md5 size mtime .../>` children and an optional
/// `<linkinfo project package/>`). The body must be a `<directory>`
/// document: a body that is not one (empty, truncated, or some other
/// document) is an error, never an empty listing — a truncated 200
/// body must not read as "no files". An `<entry>` without a `name`
/// attribute, or with a `size`/`mtime` that is not a number, is a
/// malformed listing, not a skippable entry. Other children (`<devel
/// .../>` and the like) are ignored: this parser reads files and the
/// link target only.
pub(crate) fn parse_directory(body: &str) -> Result<DirectoryListing, GeectlError> {
    let body = body.trim();
    // A malformed listing still fails closed, but the error carries a
    // bounded preview of what was received, so the failure is
    // diagnosable instead of a bare "not a directory document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS directory response: not a <directory> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the directory element; skip it. The prolog must
    // terminate (`?>`) — an unterminated prolog is malformed, not a
    // missing directory.
    let body = if let Some(after) = body.strip_prefix("<?xml") {
        let first = after.chars().next();
        if !matches!(first, Some(c) if c.is_whitespace() || c == '?') {
            return Err(malformed());
        }
        let Some(end) = after.find("?>") else {
            return Err(malformed());
        };
        after[end + 2..].trim_start()
    } else {
        body
    };
    let Some(after_open) = body.strip_prefix("<directory") else {
        return Err(malformed());
    };
    // The tag name must end here: `<directorylisting>` is not a
    // directory element. The next character is whitespace, `/`, or `>`.
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    // The opening tag ends at the first `>`.
    let Some(end) = after_open.find('>') else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    // The listing's srcmd5 pins the revision the entries belong to
    // (see the `srcmd5` field): read it off the opening tag once,
    // before the self-closing early return.
    let srcmd5 = tag_attr(open_tag, "srcmd5").map(|value| xml_unescape(&value));
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<directory .../>`: no children, so no files
        // and no service state.
        return Ok(DirectoryListing {
            entries: Vec::new(),
            linkinfo: None,
            serviceinfo: None,
            srcmd5,
        });
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</directory>") else {
        return Err(malformed());
    };
    // Nothing but whitespace may follow the closing tag.
    if !inner[close + "</directory>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut entries = Vec::new();
    let mut rest = inner;
    while let Some(pos) = rest.find("<entry") {
        let after_tag = &rest[pos + "<entry".len()..];
        // The tag name must end here: `<entryname>` is not an entry
        // element. The next character is whitespace, `/`, or `>`.
        let Some(first) = after_tag.chars().next() else {
            return Err(malformed());
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        // The element ends at the first `>`: an unterminated entry is
        // a malformed listing, not a skippable element.
        let Some(end) = after_tag.find('>') else {
            return Err(malformed());
        };
        let element = &after_tag[..end];
        let name = tag_attr(element, "name").ok_or_else(malformed)?;
        let size = match tag_attr(element, "size") {
            Some(value) => Some(value.parse::<u64>().map_err(|_| malformed())?),
            None => None,
        };
        let mtime = match tag_attr(element, "mtime") {
            Some(value) => Some(value.parse::<u64>().map_err(|_| malformed())?),
            None => None,
        };
        let md5 = tag_attr(element, "md5").map(|value| xml_unescape(&value));
        entries.push(SourceFile {
            name: xml_unescape(&name),
            size,
            mtime,
            md5,
        });
        rest = &after_tag[end..];
    }
    let mut linkinfo = None;
    let mut rest = inner;
    while let Some(pos) = rest.find("<linkinfo") {
        let after_tag = &rest[pos + "<linkinfo".len()..];
        let Some(first) = after_tag.chars().next() else {
            return Err(malformed());
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let Some(end) = after_tag.find('>') else {
            return Err(malformed());
        };
        let element = &after_tag[..end];
        if let (Some(project), Some(package)) =
            (tag_attr(element, "project"), tag_attr(element, "package"))
        {
            // The first linkinfo wins: a listing carries at most one.
            linkinfo = Some((xml_unescape(&project), xml_unescape(&package)));
            break;
        }
        rest = &after_tag[end..];
    }
    // The service run state, when the listing carries one: the
    // first `<serviceinfo>` wins (a listing carries at most one). An
    // unknown `code` is a malformed listing, not a state to guess at.
    let mut serviceinfo = None;
    let mut rest = inner;
    while let Some(pos) = rest.find("<serviceinfo") {
        let after_tag = &rest[pos + "<serviceinfo".len()..];
        // The tag name must end here: `<serviceinformation>` is not a
        // serviceinfo element. The next character is whitespace, `/`,
        // or `>`.
        let Some(first) = after_tag.chars().next() else {
            return Err(malformed());
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        // The open tag ends at the first `>` outside a quoted
        // attribute value: an unterminated serviceinfo is a malformed
        // listing, not a skippable element.
        let Some(end) = find_tag_end(after_tag) else {
            return Err(malformed());
        };
        let element = &after_tag[..end];
        let code = match tag_attr(element, "code").as_deref() {
            Some("running") => ServiceCode::Running,
            Some("failed") => ServiceCode::Failed,
            Some("succeeded") => ServiceCode::Succeeded,
            _ => return Err(malformed()),
        };
        let xsrcmd5 = tag_attr(element, "xsrcmd5").map(|value| xml_unescape(&value));
        let lsrcmd5 = tag_attr(element, "lsrcmd5").map(|value| xml_unescape(&value));
        let error = if element.trim_end().ends_with('/') {
            None
        } else {
            let after_open = &after_tag[end + 1..];
            let Some(close_pos) = after_open.find("</serviceinfo>") else {
                return Err(malformed());
            };
            let inner_si = &after_open[..close_pos];
            let error = element_text(inner_si, "error");
            // An <error> element that never closes is a malformed
            // listing, not a failure without text: element_text
            // returns None for both a self-closing <error/> and an
            // unclosed one, so tell them apart. A self-closing tag
            // (with or without a space before the slash) carries no
            // text and is fine; a paired tag that never closes is
            // not. The first <error> wins, as in element_text, so a
            // second <error> behind a first self-closing <error/>
            // is malformed: the server writes at most one, and an
            // unclosed or text-carrying second one must not be
            // silently dropped behind the first self-closing one.
            if error.is_none() {
                let mut rest = inner_si;
                let mut seen_error = false;
                while let Some(pos) = rest.find("<error") {
                    let after = &rest[pos + "<error".len()..];
                    let Some(first) = after.chars().next() else {
                        return Err(malformed());
                    };
                    if !(first.is_whitespace() || first == '/' || first == '>') {
                        rest = after;
                        continue;
                    }
                    if seen_error {
                        return Err(malformed());
                    }
                    seen_error = true;
                    let Some(gt) = find_tag_end(after) else {
                        return Err(malformed());
                    };
                    if !after[..gt].trim_end().ends_with('/') {
                        return Err(malformed());
                    }
                    rest = &after[gt..];
                }
            }
            error
        };
        serviceinfo = Some(ServiceState {
            code,
            xsrcmd5,
            lsrcmd5,
            error,
        });
        break;
    }
    Ok(DirectoryListing {
        entries,
        linkinfo,
        serviceinfo,
        srcmd5,
    })
}

/// Parse an OBS package history (`<revisionlist>` with `<revision
/// rev vrev>` children carrying `<srcmd5>`, `<version>`, `<time>`,
/// `<user>`, `<comment>`, and `<requestid>` elements). The body must
/// be a `<revisionlist>` document: a body that is not one is an
/// error, never an empty history. A `<revision>` without numeric
/// `rev`/`vrev` attributes, or with a non-numeric `<time>`, is a
/// malformed history. The entries come back in server order (oldest
/// first); the parser preserves it.
pub(crate) fn parse_history(body: &str) -> Result<Vec<HistoryEntry>, GeectlError> {
    let body = body.trim();
    // A malformed history response still fails closed, but the error
    // carries a bounded preview of what was received, so the failure
    // is diagnosable instead of a bare "not a <revisionlist> document". The
    // preview is the debug channel's redacting preview: credential
    // values are masked, binary bodies are never dumped, and the text
    // is capped at DEBUG_BODY_CAP bytes.
    let malformed = || {
        let preview = crate::debug::format_body_preview(body);
        let received = if preview.is_empty() {
            "<empty response>".to_string()
        } else {
            preview
        };
        GeectlError::new(
            ErrorKind::ServerError,
            format!(
                "unexpected OBS history response: not a <revisionlist> document; received: {received}"
            ),
        )
    };
    // The server may emit an XML prolog (`<?xml version="1.0"?>`)
    // before the <revisionlist> element; skip it. The prolog must terminate
    // (`?>`) — an unterminated prolog is malformed, not a missing
    // <revisionlist>.
    let body = match skip_xml_prolog(body) {
        Ok(body) => body,
        Err(()) => return Err(malformed()),
    };
    let Some(after_open) = body.strip_prefix("<revisionlist") else {
        return Err(malformed());
    };
    // The tag name must end here: `<revisionlists>` is not a
    // revision list. The next character is whitespace, `/`, or `>`.
    let Some(first) = after_open.chars().next() else {
        return Err(malformed());
    };
    if !(first.is_whitespace() || first == '/' || first == '>') {
        return Err(malformed());
    }
    let Some(end) = after_open.find('>') else {
        return Err(malformed());
    };
    let open_tag = &after_open[..end];
    if open_tag.trim_end().ends_with('/') {
        // Self-closing `<revisionlist/>`: no revisions.
        return Ok(Vec::new());
    }
    let inner = &after_open[end + 1..];
    let Some(close) = inner.find("</revisionlist>") else {
        return Err(malformed());
    };
    if !inner[close + "</revisionlist>".len()..].trim().is_empty() {
        return Err(malformed());
    }
    let inner = &inner[..close];
    let mut entries = Vec::new();
    let mut rest = inner;
    while let Some(pos) = rest.find("<revision") {
        let after_tag = &rest[pos + "<revision".len()..];
        // The tag name must end here: `<revisions>` is not a
        // revision element. The next character is whitespace, `/`,
        // or `>`.
        let Some(first) = after_tag.chars().next() else {
            return Err(malformed());
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let Some(end) = after_tag.find('>') else {
            return Err(malformed());
        };
        let element = &after_tag[..end];
        let rev = match tag_attr(element, "rev") {
            Some(value) => value.parse::<u64>().map_err(|_| malformed())?,
            None => return Err(malformed()),
        };
        let vrev = match tag_attr(element, "vrev") {
            Some(value) => value.parse::<u64>().map_err(|_| malformed())?,
            None => return Err(malformed()),
        };
        let inner = &after_tag[end + 1..];
        let Some(close) = inner.find("</revision>") else {
            return Err(malformed());
        };
        let block = &inner[..close];
        let time = match element_text(block, "time") {
            Some(value) => Some(value.trim().parse::<u64>().map_err(|_| malformed())?),
            None => None,
        };
        entries.push(HistoryEntry {
            rev,
            vrev,
            srcmd5: element_text(block, "srcmd5"),
            version: element_text(block, "version"),
            time,
            user: element_text(block, "user"),
            comment: element_text(block, "comment"),
            request_id: element_text(block, "requestid"),
        });
        rest = &inner[close + "</revision>".len()..];
    }
    Ok(entries)
}

/// A unified diff of `old` against `new`, in the standard form:
/// `--- <old_path>` / `+++ <new_path>` headers, `@@ -a,b +c,d @@`
/// hunks with three lines of context, `-`/`+`/context lines. The
/// diff is computed on lines (LCS-based); a file that exists on one
/// side only is rendered with `/dev/null` on the missing side. The
/// LCS table is quadratic in the file sizes, so very large files are
/// expensive to diff — the callers diff package text files, which are
/// small. An empty diff (identical contents) is an empty string:
/// there is nothing to show.
pub(crate) fn unified_diff(old_path: &str, old: &str, new_path: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let n = old_lines.len();
    let m = new_lines.len();
    // LCS table: `table[i][j]` is the LCS length of the suffixes from
    // `i` and `j`.
    let mut table = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if old_lines[i] == new_lines[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Op {
        Equal,
        Delete,
        Add,
    }
    let mut ops: Vec<(Op, &str)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old_lines[i] == new_lines[j] {
            ops.push((Op::Equal, old_lines[i]));
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            ops.push((Op::Delete, old_lines[i]));
            i += 1;
        } else {
            ops.push((Op::Add, new_lines[j]));
            j += 1;
        }
    }
    while i < n {
        ops.push((Op::Delete, old_lines[i]));
        i += 1;
    }
    while j < m {
        ops.push((Op::Add, new_lines[j]));
        j += 1;
    }
    if ops.iter().all(|(op, _)| *op == Op::Equal) {
        return String::new();
    }
    // Group into hunks with three lines of context: hunks merge when
    // the equal run between two changes is at most twice the context.
    const CONTEXT: usize = 3;
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    let mut k = 0;
    while k < ops.len() {
        while k < ops.len() && ops[k].0 == Op::Equal {
            k += 1;
        }
        if k >= ops.len() {
            break;
        }
        let hunk_start = k.saturating_sub(CONTEXT);
        let mut end = k;
        while end < ops.len() {
            if ops[end].0 == Op::Equal {
                let run_start = end;
                while end < ops.len() && ops[end].0 == Op::Equal {
                    end += 1;
                }
                if end >= ops.len() {
                    // Trailing equals: only CONTEXT of them belong.
                    end = run_start + CONTEXT.min(end - run_start);
                    break;
                }
                if end - run_start > 2 * CONTEXT {
                    // Gap too wide: the hunk ends CONTEXT after the
                    // last change.
                    end = run_start + CONTEXT;
                    break;
                }
                // Short gap: absorbed, keep going.
            } else {
                end += 1;
            }
        }
        hunks.push((hunk_start, end));
        k = end;
    }
    let mut out = String::new();
    out.push_str(&format!("--- {old_path}\n+++ {new_path}\n"));
    for (start, end) in hunks {
        // Line numbers before the hunk, then the counts within it.
        let (mut old_line, mut new_line) = (0, 0);
        for (op, _) in &ops[..start] {
            match op {
                Op::Equal => {
                    old_line += 1;
                    new_line += 1;
                }
                Op::Delete => old_line += 1,
                Op::Add => new_line += 1,
            }
        }
        let (mut old_count, mut new_count) = (0, 0);
        for (op, _) in &ops[start..end] {
            match op {
                Op::Equal => {
                    old_count += 1;
                    new_count += 1;
                }
                Op::Delete => old_count += 1,
                Op::Add => new_count += 1,
            }
        }
        let fmt_range = |start: usize, count: usize| -> String {
            if count == 0 {
                // An empty range starts at the line before it.
                format!("{start},0")
            } else if count == 1 {
                format!("{}", start + 1)
            } else {
                format!("{},{}", start + 1, count)
            }
        };
        out.push_str(&format!(
            "@@ -{} +{} @@\n",
            fmt_range(old_line, old_count),
            fmt_range(new_line, new_count)
        ));
        for (op, line) in &ops[start..end] {
            let marker = match op {
                Op::Equal => ' ',
                Op::Delete => '-',
                Op::Add => '+',
            };
            out.push(marker);
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Parse a project meta (`<project>` with `title`, `description`,
/// `url?`, `scmsync?`, `devel?`, persons, groups, and
/// `<repository name><arch>*<path project repository/>` children).
/// The body must be a `<project>` document with a `name` attribute:
/// either is missing and the parse is an error, never a meta with
/// no name.
pub(crate) fn parse_project_meta(body: &str) -> Result<ProjectMeta, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS project meta response: not a <project> document",
        )
    };
    let (open_tag, inner) = meta_envelope(body, "project")?;
    let name = tag_attr(&open_tag, "name").ok_or_else(malformed)?;
    let devel = {
        let mut found = None;
        for (element, _) in elements(inner, "devel")? {
            if let Some(project) = tag_attr(&element, "project") {
                found = Some(xml_unescape(&project));
                break;
            }
        }
        found
    };
    let mut repositories = Vec::new();
    for (element, repo_inner) in elements(inner, "repository")? {
        let Some(repo_name) = tag_attr(&element, "name") else {
            continue;
        };
        let mut arch = Vec::new();
        for (_, text) in elements(&repo_inner, "arch")? {
            let arch_name = text.trim().to_string();
            if !arch_name.is_empty() {
                arch.push(xml_unescape(&arch_name));
            }
        }
        let mut paths = Vec::new();
        for (path_element, _) in elements(&repo_inner, "path")? {
            if let (Some(project), Some(repository)) = (
                tag_attr(&path_element, "project"),
                tag_attr(&path_element, "repository"),
            ) {
                paths.push((xml_unescape(&project), xml_unescape(&repository)));
            }
        }
        repositories.push(Repository {
            name: xml_unescape(&repo_name),
            arch,
            paths,
        });
    }
    let (persons, groups) = meta_roles(inner)?;
    Ok(ProjectMeta {
        name: xml_unescape(&name),
        title: element_text(inner, "title"),
        description: element_text(inner, "description"),
        url: element_text(inner, "url"),
        scmsync: element_text(inner, "scmsync"),
        devel,
        persons,
        groups,
        repositories,
    })
}

/// Parse a package meta (`<package>` with `title`, `description`,
/// `url?`, `scmsync?`, `releasename?`, `devel project package?`,
/// `<person userid role/>`, and `<group groupid role/>` children).
/// A `<devel>` without a `package` attribute means the devel package
/// keeps this package's name.
/// The body must be a `<package>` document with `name` and `project`
/// attributes: either is missing and the parse is an error, never a
/// meta with no name.
pub(crate) fn parse_package_meta(body: &str) -> Result<PackageMeta, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::ServerError,
            "unexpected OBS package meta response: not a <package> document",
        )
    };
    let (open_tag, inner) = meta_envelope(body, "package")?;
    let name = tag_attr(&open_tag, "name").ok_or_else(malformed)?;
    let project = tag_attr(&open_tag, "project").ok_or_else(malformed)?;
    let devel = {
        let mut found = None;
        for (element, _) in elements(inner, "devel")? {
            if let Some(devel_project) = tag_attr(&element, "project") {
                // A missing package attribute means the devel package
                // keeps this package's name: the setter omits it in
                // exactly that case (see `set_devel_project`).
                let devel_package = tag_attr(&element, "package").unwrap_or_else(|| name.clone());
                found = Some((xml_unescape(&devel_project), xml_unescape(&devel_package)));
                break;
            }
        }
        found
    };
    let (persons, groups) = meta_roles(inner)?;
    Ok(PackageMeta {
        name: xml_unescape(&name),
        project: xml_unescape(&project),
        title: element_text(inner, "title"),
        description: element_text(inner, "description"),
        url: element_text(inner, "url"),
        scmsync: element_text(inner, "scmsync"),
        releasename: element_text(inner, "releasename"),
        devel,
        persons,
        groups,
    })
}

/// True when `body` is an OBS search collection: it carries a
/// `<collection` tag and a `matches` count. The count is required so
/// a malformed collection cannot read as empty.
fn is_search_collection(body: &str) -> bool {
    body.contains("<collection") && parse_search_matches(body).is_some()
}

/// Parse a package search response (`<collection matches="N">` of
/// `<package name project>` elements with `<title>` and
/// `<description>` children). None when the response is not a
/// collection at all, carries no `matches` count, or says there are
/// matches but none parsed: an unexpected response is an error,
/// never an empty list. An empty collection (with its count) reads
/// as an empty list. A package element without a `name` or `project`
/// attribute is skipped, not an error: only the collection envelope
/// itself is validated here.
pub(crate) fn parse_package_search(body: &str) -> Option<Vec<PackageMatch>> {
    if !is_search_collection(body) {
        return None;
    }
    let matches_count = parse_search_matches(body)?;
    let mut matches = Vec::new();
    for (element, inner) in elements(body, "package").ok()? {
        let (Some(name), Some(project)) =
            (tag_attr(&element, "name"), tag_attr(&element, "project"))
        else {
            continue;
        };
        matches.push(PackageMatch {
            name: xml_unescape(&name),
            project: xml_unescape(&project),
            title: element_text(&inner, "title"),
            description: element_text(&inner, "description"),
        });
    }
    // The count says there are matches but none parsed: the server
    // changed shape underneath us, and an empty list here would hide
    // results.
    if matches_count > 0 && matches.is_empty() {
        return None;
    }
    Some(matches)
}

/// Parse a project search response (`<collection matches="N">` of
/// `<project name>` elements with `<title>` and `<description>`
/// children). None on the same fail-closed terms as
/// [`parse_package_search`].
pub(crate) fn parse_project_search(body: &str) -> Option<Vec<ProjectMatch>> {
    if !is_search_collection(body) {
        return None;
    }
    let matches_count = parse_search_matches(body)?;
    let mut matches = Vec::new();
    for (element, inner) in elements(body, "project").ok()? {
        let Some(name) = tag_attr(&element, "name") else {
            continue;
        };
        matches.push(ProjectMatch {
            name: xml_unescape(&name),
            title: element_text(&inner, "title"),
            description: element_text(&inner, "description"),
        });
    }
    if matches_count > 0 && matches.is_empty() {
        return None;
    }
    Some(matches)
}

/// Validate a search term: non-empty, and without a single quote —
/// the term goes into a single-quoted XPath string literal, where a
/// quote would break out of the literal (XPath has no string
/// escape).
fn validate_search_term(term: &str) -> Result<&str, GeectlError> {
    if term.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "search term must not be empty",
        ));
    }
    if term.contains('\'') {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("search term must not contain a single quote: {term:?}"),
        ));
    }
    Ok(term)
}

/// Parse an owner search response (`<collection>` of `<owner
/// rootproject project package?>` elements with `<person name
/// role/>` and `<group name role/>` children — `name`, not
/// `userid`). None when the response is not a collection at all, or
/// when a `matches` count is present but does not parse: an
/// unexpected response is an error, never an empty list. An owner
/// element without its `rootproject` or `project` attribute is
/// skipped, not an error: only the collection envelope itself is
/// validated here. The `matches` count is not required: the owner
/// search renders a collection either way, and refusing on a missing
/// count would refuse a valid empty answer.
pub(crate) fn parse_owners(body: &str) -> Option<Vec<Owner>> {
    if !body.contains("<collection") {
        return None;
    }
    if body.contains("matches=") && parse_search_matches(body).is_none() {
        return None;
    }
    let mut owners = Vec::new();
    for (element, inner) in elements(body, "owner").ok()? {
        let (Some(root_project), Some(project)) = (
            tag_attr(&element, "rootproject"),
            tag_attr(&element, "project"),
        ) else {
            continue;
        };
        let package = tag_attr(&element, "package").map(|value| xml_unescape(&value));
        let mut persons = Vec::new();
        for (person_element, _) in elements(&inner, "person").ok()? {
            if let (Some(name), Some(role)) = (
                tag_attr(&person_element, "name"),
                tag_attr(&person_element, "role"),
            ) {
                persons.push(NamedRole {
                    name: xml_unescape(&name),
                    role: xml_unescape(&role),
                });
            }
        }
        let mut groups = Vec::new();
        for (group_element, _) in elements(&inner, "group").ok()? {
            if let (Some(name), Some(role)) = (
                tag_attr(&group_element, "name"),
                tag_attr(&group_element, "role"),
            ) {
                groups.push(NamedRole {
                    name: xml_unescape(&name),
                    role: xml_unescape(&role),
                });
            }
        }
        owners.push(Owner {
            root_project: xml_unescape(&root_project),
            project: xml_unescape(&project),
            package,
            persons,
            groups,
        });
    }
    Some(owners)
}

/// Parse a person document (`<person>` with `<login>`, `<email>`,
/// `<realname?>`, `<state>`, and `<globalrole>*` children). None
/// when the body is not a `<person>` document, or when it carries no
/// login, email, or state: a person without an identity is an
/// unexpected response, never a person with empty fields.
pub(crate) fn parse_person(body: &str) -> Option<Person> {
    let (_, inner) = meta_envelope(body, "person").ok()?;
    let token = |tag: &str| -> Option<String> {
        element_text(inner, tag)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    Some(Person {
        login: token("login")?,
        email: token("email")?,
        realname: token("realname"),
        state: token("state")?,
    })
}

/// Validate a login name against the server's format: 2–100
/// characters, from `[\w $\^\-.#*+&'"]` (word characters plus space,
/// `$`, `^`, `-`, `.`, `#`, `*`, `+`, `&`, `'`, `"`).
fn validate_login(login: &str) -> Result<(), GeectlError> {
    let invalid = || {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid OBS login name: {login:?}"),
        )
    };
    let len = login.chars().count();
    if !(2..=100).contains(&len) {
        return Err(invalid());
    }
    let allowed = |c: char| {
        c.is_ascii_alphanumeric()
            || c == '_'
            || matches!(
                c,
                ' ' | '$' | '^' | '-' | '.' | '#' | '*' | '+' | '&' | '\'' | '"'
            )
    };
    if !login.chars().all(allowed) {
        return Err(invalid());
    }
    Ok(())
}

impl<'a> ObsClient<'a> {
    /// Return the devel (project, package) pair for `package` in
    /// project `project`, or None when no devel mapping is
    /// registered. `GET /public/source/<project>/<package>/_meta`; a
    /// public read, no credentials. The pair comes from the package
    /// meta (`<devel project package/>`): the package half can differ
    /// from the queried package name, so only the pair carries it. A
    /// 404 is None as well: the package is not there, so it has no
    /// devel mapping. Any other failed lookup is an error, never None.
    pub fn devel_project(
        &self,
        project: &str,
        package: &str,
    ) -> Result<Option<(String, String)>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        let meta = match self.package_meta(project, package) {
            Ok(meta) => meta,
            // A 404 means the package (or the project itself) is not
            // there, so it has no devel mapping — that is None, not
            // a failed lookup. Any other failure stays an error.
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let Some((devel_project, devel_package)) = meta.devel else {
            return Ok(None);
        };
        // Both halves come from network XML, so they are validated at
        // the source like any other interpolated value: a devel name
        // that is not a valid OBS name is unexpected server data
        // (ServerError), not a caller mistake — the caller-supplied
        // names are what InvalidInput is for.
        for (label, value) in [("project", &devel_project), ("package", &devel_package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::ServerError,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        Ok(Some((devel_project, devel_package)))
    }

    /// Return the devel package's Version of `package`, or None when
    /// the package has no devel mapping or its spec carries no
    /// Version. A failed lookup is an error, never None — including a
    /// missing spec file: the devel package may carry no spec, or the
    /// spec may keep the Factory package name after a rename, so the
    /// `{devel_package}.spec` read can 404 with `NotFound`. The spec is
    /// read from the devel package, not the queried package: the
    /// devel package name can differ from the queried one.
    pub fn devel_version(&self, package: &str) -> Result<Option<String>, GeectlError> {
        // `devel_project` validates both halves at the source, so
        // the names are safe to interpolate here.
        let (project, devel_package) = match self.devel_project(FACTORY_PROJECT, package)? {
            Some(pair) => pair,
            None => return Ok(None),
        };
        let url = build_url(
            &self.base,
            &[
                "public",
                "source",
                &project,
                &devel_package,
                &format!("{devel_package}.spec"),
            ],
            &[],
        );
        let spec = self.http.get_with(&url, &Credentials::None)?;
        Ok(parse_spec_version(&spec))
    }

    /// Delete `project`/`package` from OBS (`DELETE
    /// /source/<project>/<package>`): the manual branch cleanup.
    /// Requests filed through this library clean up on accept by
    /// default (see [`SrPayload`]), so this is for the branches the
    /// server-side cleanup never reaches: a superseded request's
    /// branch, a declined request's branch, and any branch whose
    /// request was filed without the cleanup attribute. Requires
    /// credentials; without them the call fails loudly before any
    /// request. Takes the caller's [`DryRun`]: in dry-run the delete
    /// is recorded in the plan and nothing is deleted, so a dry-run
    /// caller cannot mutate the server by invoking this directly.
    ///
    /// The delete is refused while requests in `new`, `review`, or
    /// `declined` state involve the package as a submit source or
    /// target — unless `force` is set. That refusal is the library's
    /// own guard, and it exists because the API does NOT refuse: a
    /// package delete revokes the requests that have it as source and
    /// declines the ones that have it as target
    /// (`Package#revoke_requests_with_self_as_source` /
    /// `#decline_requests_with_self_as_target`), with the comment
    /// "The package 'X/Y' has been removed". Deleting a branch with a
    /// live submit request would silently kill that request, so the
    /// guard names the blocking requests and refuses instead. The
    /// state set is exactly the set the API touches — osc's
    /// `delete_package` pre-check covers the same set, so here osc and
    /// the API agree; they differ only in mechanism (osc refuses, the
    /// API closes).
    ///
    /// `force` bypasses the guard AND is forwarded as `force=1` to
    /// the API, which also bypasses the server's weak-dependency
    /// guard (`check_weak_dependencies!`: refuse when other packages
    /// use this one as their devel package). That is osc's `force`
    /// semantics, and it is the only way a delete with blocking
    /// requests proceeds — the caller has verified the requests are
    /// abandonable. A review by the package on an open request is NOT
    /// covered by the guard: the delete only obsoletes that review
    /// (`obsolete_reviews_for_self`) without changing the request's
    /// state.
    ///
    /// Success is the `<status code="ok"/>` body: any other body is
    /// a server error, never success. A delete that times out leaves
    /// the state unknown (the package may or may not be gone) — the
    /// caller checks the server state instead of assuming.
    pub fn delete_package(
        &self,
        project: &str,
        package: &str,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        // The dry-run guard: in dry-run the delete is recorded in the
        // plan and nothing is deleted — not even the pre-check search
        // runs, since a dry-run delete has nothing it must read.
        // Without this, a dry-run caller invoking delete directly
        // would mutate the server.
        if !dry_run.allow(&format!("delete package {project}/{package}")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot delete a package without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        if !force {
            let blocking = self.open_requests_for_package(project, package)?;
            if !blocking.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "refusing to delete {project}/{package}: open submit request(s) {} involve it (a delete would revoke them as source / decline them as target); supersede, accept, or decline them first, or pass force",
                        blocking.join(", ")
                    ),
                ));
            }
        }
        let mut query: Vec<(&str, &str)> = Vec::new();
        if force {
            query.push(("force", "1"));
        }
        let url = build_url(&self.base, &["source", project, package], &query);
        let body = self.http.delete(&url, &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS delete response for package {project}/{package}"),
            ))
        }
    }

    /// Create project `project` on OBS (`PUT /source/<project>/_meta`)
    /// with the given title and description. Requires credentials;
    /// without them the call fails loudly before any request. In
    /// dry-run the creation is recorded in the plan and nothing is
    /// created — the guard runs before the credentials check, like
    /// `delete_package`, so planning a creation needs no credentials.
    ///
    /// An agent creates a branch project before filing a submit
    /// request — a fresh project per update, so the accepted branch
    /// dies on accept and never piles up — so project creation
    /// belongs in the library, not in test setup: freshcrab needs
    /// exactly this operation when it migrates its branch creation
    /// onto geectl. Success is the `<status code="ok"/>` body: any
    /// other body is a server error, never success.
    pub fn create_project(
        &self,
        project: &str,
        title: &str,
        description: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        // The dry-run guard: in dry-run the creation is recorded in
        // the plan and nothing is created — not even the credentials
        // check runs. Without this, a dry-run caller invoking create
        // directly would mutate the server.
        if !dry_run.allow(&format!("create project {project}")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot create a project without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let meta = format!(
            "<project name=\"{}\"><title>{}</title><description>{}</description></project>",
            xml_escape(project),
            xml_escape(title),
            xml_escape(description),
        );
        let url = build_url(&self.base, &["source", project, "_meta"], &[]);
        let body = self.http.put(&url, &meta, &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS create-project response for project {project}"),
            ))
        }
    }

    /// PUT `url` with `body`, retrying a not-found while `budget`
    /// lasts: the server's API answers before its backend has
    /// registered a just-created parent, so a PUT that follows a
    /// creation can 404 on a parent that is about to exist. Only a
    /// not-found retries — a 410 maps to the same kind — and the
    /// retry sleeps 500 ms between attempts, so a genuinely missing
    /// parent still fails loudly once the budget runs out instead of
    /// hanging. The PUT is idempotent, so a retry cannot duplicate
    /// anything. Callers pass the budget; [`ObsClient::create_package`]
    /// passes 30 s.
    fn put_with_registration_retry(
        &self,
        url: &str,
        body: &str,
        budget: std::time::Duration,
    ) -> Result<String, GeectlError> {
        let deadline = std::time::Instant::now() + budget;
        let mut probe = 0;
        loop {
            match self.http.put(url, body, &self.credentials) {
                Ok(body) => return Ok(body),
                Err(err)
                    if err.kind() == ErrorKind::NotFound
                        && std::time::Instant::now() < deadline =>
                {
                    probe += 1;
                    self.debug.emit(&format!(
                        "retry: create_package registration probe {probe}/{}: still not found",
                        Self::format_budget(budget)
                    ));
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Create package `package` in project `project` on OBS (`PUT
    /// /source/<project>/<package>/_meta`) with the given title and
    /// description. Requires credentials; without them the call fails
    /// loudly before any request. In dry-run the creation is recorded
    /// in the plan and nothing is created — the guard runs before the
    /// credentials check, like `delete_package`.
    ///
    /// An agent creates the package it submits from — the branch
    /// package — so package creation belongs in the library next to
    /// [`ObsClient::create_project`]: freshcrab needs exactly this
    /// operation when it migrates its branch creation onto geectl.
    /// Success is the `<status code="ok"/>` body: any other body is
    /// a server error, never success.
    ///
    /// The PUT retries a not-found for a bounded 30 s: the server's
    /// API and backend are separate processes, and the API replies to
    /// the project creation before the backend has the project
    /// registered — the immediately following package creation can
    /// then 404 on the just-created project (seen on live run
    /// 44876734097). Only a not-found retries, so anything else still
    /// fails loudly; a genuinely missing project takes about 30 s to
    /// fail instead of failing fast.
    pub fn create_package(
        &self,
        project: &str,
        package: &str,
        title: &str,
        description: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        // The dry-run guard: in dry-run the creation is recorded in
        // the plan and nothing is created — not even the credentials
        // check runs. Without this, a dry-run caller invoking create
        // directly would mutate the server.
        if !dry_run.allow(&format!("create package {project}/{package}")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot create a package without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let meta = format!(
            "<package name=\"{}\" project=\"{}\"><title>{}</title><description>{}</description></package>",
            xml_escape(package),
            xml_escape(project),
            xml_escape(title),
            xml_escape(description),
        );
        let url = build_url(&self.base, &["source", project, package, "_meta"], &[]);
        let body =
            self.put_with_registration_retry(&url, &meta, std::time::Duration::from_secs(30))?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS create-package response for package {project}/{package}"),
            ))
        }
    }

    /// Upload file `filename` with `content` into package `package`
    /// in project `project` on OBS (`PUT
    /// /source/<project>/<package>/<filename>`). Requires credentials;
    /// without them the call fails loudly before any request. In
    /// dry-run the upload is recorded in the plan and nothing is
    /// uploaded — the guard runs before the credentials check, like
    /// `delete_package` — and the result is `None`, like
    /// `SrFiling::file` in dry-run.
    ///
    /// An agent uploads the files it submits — the spec, the
    /// `.changes` entry — into the branch package before filing, so
    /// file upload belongs in the library next to
    /// [`ObsClient::create_project`]: freshcrab needs exactly this
    /// operation when it migrates its branch creation onto geectl.
    /// The content is text (a spec, a `.changes` entry, a service
    /// file); binary content is out of scope for this operation.
    /// Returns the new revision number. Success is the backend's
    /// `<revision rev="N" .../>` document, not a `<status code="ok"/>`:
    /// the API streams the backend's answer back
    /// (`SourcePackageController#update_file` passes the request
    /// through to `bs_srcserver::putfile`, which returns the new
    /// revision), so any other body is a server error, never success.
    pub fn upload_file(
        &self,
        project: &str,
        package: &str,
        filename: &str,
        content: &str,
        dry_run: &DryRun,
    ) -> Result<Option<u64>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if !valid_obs_filename(filename) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS filename: {filename:?}"),
            ));
        }
        // The dry-run guard: in dry-run the upload is recorded in the
        // plan and nothing is uploaded — not even the credentials
        // check runs. Without this, a dry-run caller invoking upload
        // directly would mutate the server.
        if !dry_run.allow(&format!("upload file {project}/{package}/{filename}")) {
            return Ok(None);
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot upload a file without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["source", project, package, filename], &[]);
        let body = self.http.put_file(&url, content, &self.credentials)?;
        match parse_revision(&body) {
            Some(rev) => Ok(Some(rev)),
            None => Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS upload response for file {project}/{package}/{filename}"),
            )),
        }
    }

    /// Upload file `filename` with binary `content` into package
    /// `package` in project `project` on OBS (`PUT
    /// /source/<project>/<package>/<filename>`). The byte counterpart
    /// of [`ObsClient::upload_file`]: same validation, same dry-run
    /// guard, same revision-document success — but the content is
    /// bytes, not text, so a binary file (a tarball, an RPM) uploads
    /// instead of failing the UTF-8 check a text upload would hit.
    /// Binary content is why this exists: the cross-instance copy
    /// downloads each file as bytes and uploads it the same way.
    /// Returns the new revision number, like `upload_file`.
    pub fn upload_file_bytes(
        &self,
        project: &str,
        package: &str,
        filename: &str,
        content: &[u8],
        dry_run: &DryRun,
    ) -> Result<Option<u64>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if !valid_obs_filename(filename) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS filename: {filename:?}"),
            ));
        }
        // The dry-run guard: in dry-run the upload is recorded in the
        // plan and nothing is uploaded — not even the credentials
        // check runs. Without this, a dry-run caller invoking upload
        // directly would mutate the server.
        if !dry_run.allow(&format!("upload file {project}/{package}/{filename}")) {
            return Ok(None);
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot upload a file without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["source", project, package, filename], &[]);
        let body = self.http.put_bytes(&url, content, &self.credentials)?;
        match parse_revision(&body) {
            Some(rev) => Ok(Some(rev)),
            None => Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS upload response for file {project}/{package}/{filename}"),
            )),
        }
    }

    /// GET a public directory listing (`/public/source/...`) and
    /// parse it. A public read: no credentials. The path is the
    /// segments after `/public/source` (a project, or a
    /// project/package pair).
    fn public_directory(
        &self,
        path: &[&str],
        query: &[(&str, &str)],
    ) -> Result<DirectoryListing, GeectlError> {
        let mut segments: Vec<&str> = vec!["public", "source"];
        segments.extend_from_slice(path);
        let url = build_url(&self.base, &segments, query);
        let body = self.http.get_with(&url, &Credentials::None)?;
        parse_directory(&body)
    }

    /// GET an authenticated directory listing (`/source/...`) and
    /// parse it. The authed source endpoints resolve the `_project`
    /// (and `_pattern`) pseudo-packages via the project, where the
    /// public routes cannot address them at all: the public access
    /// check looks up a Package row, and `_project` is not one.
    pub(crate) fn authed_directory_listing(
        &self,
        project: &str,
        package: &str,
    ) -> Result<DirectoryListing, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        let url = build_url(&self.base, &["source", project, package], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_directory(&body)
    }

    /// Read file `file` of package `package` in project `project`
    /// (`GET /public/source/<project>/<package>/<file>`), optionally
    /// at revision `rev` (`?rev=N` for a number, `?rev=<srcmd5>` for
    /// the revision carrying that hash — the backend resolves the
    /// hash). A public read: no credentials. A missing file is an
    /// error, never an empty string.
    pub fn cat(
        &self,
        project: &str,
        package: &str,
        file: &str,
        rev: Option<RevisionSpec>,
    ) -> Result<String, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if !valid_obs_filename(file) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS filename: {file:?}"),
            ));
        }
        if let Some(RevisionSpec::Rev(0)) = rev {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "invalid revision: 0",
            ));
        }
        let mut query: Vec<(&str, &str)> = Vec::new();
        let rev_text;
        if let Some(rev) = rev {
            rev_text = rev.to_string();
            query.push(("rev", &rev_text));
        }
        let url = build_url(
            &self.base,
            &["public", "source", project, package, file],
            &query,
        );
        self.http.get_with(&url, &Credentials::None)
    }

    /// Read file `file` of package `package` in project `project`
    /// via the authenticated source endpoint
    /// (`GET /source/<project>/<package>/<file>`). Same `_project`
    /// reason as `authed_directory_listing`: the public file route
    /// cannot address the pseudo-package. A missing file is an
    /// error, never an empty string. No revision parameter: the
    /// service read needs none.
    pub(crate) fn authed_cat(
        &self,
        project: &str,
        package: &str,
        file: &str,
    ) -> Result<String, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if !valid_obs_filename(file) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS filename: {file:?}"),
            ));
        }
        let url = build_url(&self.base, &["source", project, package, file], &[]);
        self.http.get_with(&url, &self.credentials)
    }

    /// Download file `file` of package `package` in project `project`
    /// (`GET /public/source/<project>/<package>/<file>`), streaming
    /// the body to `dest` instead of buffering it: the binary
    /// counterpart of [`ObsClient::cat`], without the transport's
    /// 10 MiB body cap and without the UTF-8 check, so a source
    /// tarball downloads instead of failing. Same validation and
    /// `?rev=` forms as `cat`: a revision number or a srcmd5
    /// ([`RevisionSpec`]), passed verbatim. A public read: no
    /// credentials. The file is created exclusively, and a failure
    /// mid-stream leaves a partial file for the caller to delete.
    /// Returns the bytes written.
    pub fn download_source_to_file(
        &self,
        project: &str,
        package: &str,
        file: &str,
        rev: Option<&RevisionSpec>,
        dest: &Path,
    ) -> Result<u64, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if !valid_obs_filename(file) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS filename: {file:?}"),
            ));
        }
        if let Some(RevisionSpec::Rev(0)) = rev {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "invalid revision: 0",
            ));
        }
        let mut query: Vec<(&str, &str)> = Vec::new();
        let rev_text;
        if let Some(rev) = rev {
            rev_text = rev.to_string();
            query.push(("rev", &rev_text));
        }
        let url = build_url(
            &self.base,
            &["public", "source", project, package, file],
            &query,
        );
        self.http.get_to_file(&url, &Credentials::None, dest)
    }

    /// GET a public directory listing (`/public/source/...`),
    /// optionally at revision `rev` (`?rev=N` for a number,
    /// `?rev=<srcmd5>` for the revision carrying that hash), and
    /// parse it, returning the full listing: files, link target, and
    /// service state. A public read: no credentials. Shared by the
    /// listing readers that need more than the file entries
    /// ([`ObsClient::service_state`]).
    pub(crate) fn directory_listing(
        &self,
        project: &str,
        package: &str,
        rev: Option<&RevisionSpec>,
    ) -> Result<DirectoryListing, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        let mut query: Vec<(&str, &str)> = Vec::new();
        let rev_text;
        if let Some(rev) = rev {
            rev_text = rev.to_string();
            query.push(("rev", &rev_text));
        }
        self.public_directory(&[project, package], &query)
    }

    /// List the files of package `package` in project `project`
    /// (`GET /public/source/<project>/<package>`), optionally at
    /// revision `rev` (`?rev=N` for a number, `?rev=<srcmd5>` for
    /// the revision carrying that hash — the backend resolves the
    /// hash). A public read: no credentials. The listing carries
    /// each file's name, size, modification time, and md5, in
    /// server order.
    pub fn list_files(
        &self,
        project: &str,
        package: &str,
        rev: Option<&RevisionSpec>,
    ) -> Result<Vec<SourceFile>, GeectlError> {
        if let Some(RevisionSpec::Rev(0)) = rev {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "invalid revision: 0",
            ));
        }
        Ok(self.directory_listing(project, package, rev)?.entries)
    }

    /// List the packages of project `project` (`GET
    /// /public/source/<project>`). A public read: no credentials.
    /// Multibuild flavors (`pkg:flavor1`, `pkg:flavor2`) are separate
    /// entries, exactly as the listing carries them.
    pub fn list_packages(&self, project: &str) -> Result<Vec<String>, GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        let listing = self.public_directory(&[project], &[])?;
        Ok(listing
            .entries
            .into_iter()
            .map(|entry| entry.name)
            .collect())
    }

    /// List the top-level projects (`GET /source`): the names of
    /// every project on the server, sorted. The top-level listing
    /// has no public route, so the read uses the client's
    /// credentials (whatever they are — no fail-closed pre-check: a
    /// client without credentials gets the server's 401 as an error,
    /// like any other authenticated read).
    pub fn list_projects(&self) -> Result<Vec<String>, GeectlError> {
        let url = build_url(&self.base, &["source"], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        let mut names: Vec<String> = parse_directory(&body)?
            .entries
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        names.sort();
        Ok(names)
    }

    /// Read the history of package `package` in project `project`
    /// (`GET /source/<project>/<package>/_history`), optionally
    /// restricted to `range`. The history has no public route, so the
    /// read uses the client's credentials (whatever they are — no
    /// fail-closed pre-check: a client without credentials gets the
    /// server's 401 as an error, like any other authenticated read).
    /// Entries come back newest first: the server lists them oldest
    /// first, so the log is reversed before it is returned; a range
    /// filter preserves the newest-first order.
    pub fn package_log(
        &self,
        project: &str,
        package: &str,
        range: Option<&RevisionRange>,
    ) -> Result<Vec<HistoryEntry>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        let url = build_url(&self.base, &["source", project, package, "_history"], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        let entries = parse_history(&body)?;
        let mut entries = match range {
            None => entries,
            Some(range) => filter_history(entries, range)?,
        };
        // The server lists oldest first; a log reads newest first.
        entries.reverse();
        Ok(entries)
    }

    /// The per-file diff of revision `rev` of package `package` in
    /// project `project` against the previous revision: the files
    /// added, modified, or deleted, each with a unified diff.
    /// Revision 1 of an existing package has no previous revision, so
    /// its patch is empty; revision 1 of a package that does not
    /// exist is an error, not an empty patch.
    /// File contents are read as text; a binary file that differs
    /// fails the call (its contents cannot be rendered as a text
    /// diff). Added and deleted files always appear; a modified file
    /// appears only when its contents actually differ.
    ///
    /// Cost: the patch is one sequential fan-out — two file listings,
    /// then one file read per added or deleted file and two per
    /// changed file, with no concurrency bound and no cap: the client
    /// is synchronous (`&dyn HttpClient` carries no `Sync` bound, and
    /// the test fake is `RefCell`-based), so the fan-out cannot be
    /// parallelized without a transport refactor. A package with many
    /// changed files is that many sequential round-trips. Each item
    /// still fails closed: the first failed read aborts the whole
    /// patch with its error, never a partial list.
    pub fn revision_patch(
        &self,
        project: &str,
        package: &str,
        rev: u64,
    ) -> Result<RevisionPatch, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if rev == 0 {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "invalid revision: 0",
            ));
        }
        if rev == 1 {
            // Revision 1 has no previous revision, so its patch is
            // empty — but only once the package is known to exist: a
            // bare short-circuit would also report an empty patch
            // for a package that is not there at all.
            self.list_files(project, package, Some(&RevisionSpec::Rev(1)))?;
            return Ok(RevisionPatch {
                rev,
                files: Vec::new(),
            });
        }
        let new_files = self.list_files(project, package, Some(&RevisionSpec::Rev(rev)))?;
        let old_files = self.list_files(project, package, Some(&RevisionSpec::Rev(rev - 1)))?;
        let mut patches = Vec::new();
        for new in &new_files {
            match old_files.iter().find(|old| old.name == new.name) {
                None => {
                    // Added: the whole file is new.
                    let content =
                        self.cat(project, package, &new.name, Some(RevisionSpec::Rev(rev)))?;
                    patches.push(FilePatch {
                        filename: new.name.clone(),
                        status: FilePatchStatus::Added,
                        diff: unified_diff("/dev/null", "", &format!("b/{}", new.name), &content),
                    });
                }
                Some(old) if old.md5 != new.md5 => {
                    let old_content = self.cat(
                        project,
                        package,
                        &new.name,
                        Some(RevisionSpec::Rev(rev - 1)),
                    )?;
                    let new_content =
                        self.cat(project, package, &new.name, Some(RevisionSpec::Rev(rev)))?;
                    let diff = unified_diff(
                        &format!("a/{}", new.name),
                        &old_content,
                        &format!("b/{}", new.name),
                        &new_content,
                    );
                    // The md5s differ but the contents may not (a
                    // missing md5 reads as a difference): only a
                    // non-empty diff is a modification.
                    if !diff.is_empty() {
                        patches.push(FilePatch {
                            filename: new.name.clone(),
                            status: FilePatchStatus::Modified,
                            diff,
                        });
                    }
                }
                Some(_) => {}
            }
        }
        for old in &old_files {
            if !new_files.iter().any(|new| new.name == old.name) {
                let content = self.cat(
                    project,
                    package,
                    &old.name,
                    Some(RevisionSpec::Rev(rev - 1)),
                )?;
                patches.push(FilePatch {
                    filename: old.name.clone(),
                    status: FilePatchStatus::Deleted,
                    diff: unified_diff(&format!("a/{}", old.name), &content, "/dev/null", ""),
                });
            }
        }
        patches.sort_by(|a, b| a.filename.cmp(&b.filename));
        Ok(RevisionPatch {
            rev,
            files: patches,
        })
    }

    /// Read the meta of project `project` (`GET
    /// /public/source/<project>/_meta`). A public read: no
    /// credentials. The meta carries the title, description, url,
    /// scmsync, devel project, persons, groups, and repositories —
    /// the same operation regardless of whether the project is a
    /// plain project or a git-backed one: a scmsync project returns
    /// its typed `scmsync` element through this call.
    pub fn project_meta(&self, project: &str) -> Result<ProjectMeta, GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        let url = build_url(&self.base, &["public", "source", project, "_meta"], &[]);
        let body = self.http.get_with(&url, &Credentials::None)?;
        parse_project_meta(&body)
    }

    /// Read the meta of package `package` in project `project` (`GET
    /// /public/source/<project>/<package>/_meta`). A public read: no
    /// credentials. The meta carries the title, description, url,
    /// scmsync, releasename, devel project and package, persons, and
    /// groups — the same operation regardless of whether the package
    /// is a plain package or a git-backed one: a scmsync package
    /// returns its typed `scmsync` element through this call.
    pub fn package_meta(&self, project: &str, package: &str) -> Result<PackageMeta, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        let url = build_url(
            &self.base,
            &["public", "source", project, package, "_meta"],
            &[],
        );
        let body = self.http.get_with(&url, &Credentials::None)?;
        parse_package_meta(&body)
    }

    /// Search packages by substring (`GET
    /// /search/package?match=...`): the match is
    /// `contains(@name,'term') or contains(title,'term') or
    /// contains(description,'term')` — a substring match, not the
    /// exact match osc defaults to. Requires credentials; without
    /// them the search fails loudly before any request. An unexpected
    /// response (no `matches` count) is an error, never an empty
    /// list.
    pub fn search_packages(&self, term: &str) -> Result<Vec<PackageMatch>, GeectlError> {
        let term = validate_search_term(term)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "searching packages requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let match_expr = format!(
            "contains(@name,'{term}') or contains(title,'{term}') or contains(description,'{term}')"
        );
        let url = build_url(
            &self.base,
            &["search", "package"],
            &[("match", &match_expr)],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_package_search(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS package search response for {term:?}"),
            )
        })
    }

    /// Search projects by substring (`GET
    /// /search/project?match=...`): the same substring match as
    /// [`ObsClient::search_packages`], over project names, titles,
    /// and descriptions. Requires credentials; an unexpected response
    /// is an error, never an empty list.
    pub fn search_projects(&self, term: &str) -> Result<Vec<ProjectMatch>, GeectlError> {
        let term = validate_search_term(term)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "searching projects requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let match_expr = format!(
            "contains(@name,'{term}') or contains(title,'{term}') or contains(description,'{term}')"
        );
        let url = build_url(
            &self.base,
            &["search", "project"],
            &[("match", &match_expr)],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_project_search(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS project search response for {term:?}"),
            )
        })
    }

    /// Search projects a person is involved in (`GET
    /// /search/project?match=person/@userid='...'`): every project
    /// whose meta names `userid` in a role. Requires credentials;
    /// an unexpected response is an error, never an empty list. The
    /// user id is validated like a search term — it goes into a
    /// single-quoted XPath string literal, where a quote would break
    /// out of the literal (XPath has no string escape).
    ///
    /// An agent searches by person to find the projects a maintainer
    /// owns or watches, for example when routing a submission to the
    /// right devel project.
    pub fn search_projects_by_person(
        &self,
        userid: &str,
    ) -> Result<Vec<ProjectMatch>, GeectlError> {
        let userid = validate_search_term(userid)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "searching projects by person requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let match_expr = format!("person/@userid='{userid}'");
        let url = build_url(
            &self.base,
            &["search", "project"],
            &[("match", &match_expr)],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_project_search(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS project search response for person {userid:?}"),
            )
        })
    }

    /// Count the projects whose scmsync URL starts with `url_prefix`
    /// (`GET /search/project/id?match=starts-with(scmsync,'...')`).
    /// Requires credentials; a response without a `matches` count is
    /// an error, never zero. The prefix is validated like a search
    /// term — it goes into a single-quoted XPath string literal,
    /// where a quote would break out of the literal (XPath has no
    /// string escape).
    ///
    /// An agent counts scmsync projects to see how much of a source
    /// tree is already git-managed, for example before deciding
    /// whether a bulk migration is worth proposing.
    pub fn count_scmsync_projects(&self, url_prefix: &str) -> Result<u32, GeectlError> {
        let url_prefix = validate_search_term(url_prefix)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "counting scmsync projects requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let match_expr = format!("starts-with(scmsync,'{url_prefix}')");
        let url = build_url(
            &self.base,
            &["search", "project", "id"],
            &[("match", &match_expr)],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_search_matches(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS scmsync search response for {url_prefix:?}"),
            )
        })
    }

    /// The owners of package `package` in project `project` (`GET
    /// /search/owner?project=...&package=...`). Requires credentials;
    /// without them the search fails loudly before any request. An
    /// unexpected response (not a collection) is an error, never an
    /// empty list.
    pub fn owners(&self, project: &str, package: &str) -> Result<Vec<Owner>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "searching owners requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(
            &self.base,
            &["search", "owner"],
            &[("project", project), ("package", package)],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_owners(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS owner search response for package {project}/{package}"),
            )
        })
    }

    /// The owners of binary `binary` (`GET
    /// /search/owner?binary=...`). Requires credentials; an
    /// unexpected response is an error, never an empty list.
    pub fn owners_for_binary(&self, binary: &str) -> Result<Vec<Owner>, GeectlError> {
        if !valid_obs_name(binary) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS binary name: {binary:?}"),
            ));
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "searching owners requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["search", "owner"], &[("binary", binary)]);
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_owners(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS owner search response for binary {binary:?}"),
            )
        })
    }

    /// The maintainers of package `package` in project `project`:
    /// the owners filtered to role `maintainer`, as `(name, role)`
    /// pairs (persons and groups flattened).
    pub fn maintainers(&self, project: &str, package: &str) -> Result<Vec<NamedRole>, GeectlError> {
        let owners = self.owners(project, package)?;
        let mut maintainers = Vec::new();
        for owner in owners {
            maintainers.extend(
                owner
                    .persons
                    .into_iter()
                    .filter(|person| person.role == "maintainer"),
            );
            maintainers.extend(
                owner
                    .groups
                    .into_iter()
                    .filter(|group| group.role == "maintainer"),
            );
        }
        Ok(maintainers)
    }

    /// Read the person `login` (`GET /person/<login>`). Requires
    /// credentials; without them the read fails loudly before any
    /// request. An unknown login is an error, never an empty person.
    pub fn whois(&self, login: &str) -> Result<Person, GeectlError> {
        validate_login(login)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "reading a person requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["person", login], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_person(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS person response for login {login:?}"),
            )
        })
    }

    /// The link target of package `package` in project `project`
    /// (`GET /public/source/<project>/<package>`, reading the
    /// listing's `<linkinfo project package/>`), or None when the
    /// package links nothing. A public read: no credentials. A
    /// missing package is an error, never None — only a listing
    /// without linkinfo reads as "no link".
    pub fn link_target(
        &self,
        project: &str,
        package: &str,
    ) -> Result<Option<(String, String)>, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        Ok(self.public_directory(&[project, package], &[])?.linkinfo)
    }

    /// The per-file diff between two listings, one per package:
    /// files added, modified (contents differ), or deleted, each
    /// with a unified diff. Unchanged files do not appear.
    fn diff_files(
        &self,
        old_project: &str,
        old_package: &str,
        old_files: &[SourceFile],
        new_project: &str,
        new_package: &str,
        new_files: &[SourceFile],
    ) -> Result<Vec<FilePatch>, GeectlError> {
        let mut patches = Vec::new();
        for new in new_files {
            match old_files.iter().find(|old| old.name == new.name) {
                None => {
                    // Added: the whole file is new.
                    let content = self.cat(new_project, new_package, &new.name, None)?;
                    patches.push(FilePatch {
                        filename: new.name.clone(),
                        status: FilePatchStatus::Added,
                        diff: unified_diff("/dev/null", "", &format!("b/{}", new.name), &content),
                    });
                }
                Some(old) if old.md5 != new.md5 => {
                    let old_content = self.cat(old_project, old_package, &new.name, None)?;
                    let new_content = self.cat(new_project, new_package, &new.name, None)?;
                    let diff = unified_diff(
                        &format!("a/{}", new.name),
                        &old_content,
                        &format!("b/{}", new.name),
                        &new_content,
                    );
                    // The md5s differ but the contents may not (a
                    // missing md5 reads as a difference): only a
                    // non-empty diff is a modification.
                    if !diff.is_empty() {
                        patches.push(FilePatch {
                            filename: new.name.clone(),
                            status: FilePatchStatus::Modified,
                            diff,
                        });
                    }
                }
                Some(_) => {}
            }
        }
        for old in old_files {
            if !new_files.iter().any(|new| new.name == old.name) {
                let content = self.cat(old_project, old_package, &old.name, None)?;
                patches.push(FilePatch {
                    filename: old.name.clone(),
                    status: FilePatchStatus::Deleted,
                    diff: unified_diff(&format!("a/{}", old.name), &content, "/dev/null", ""),
                });
            }
        }
        patches.sort_by(|a, b| a.filename.cmp(&b.filename));
        Ok(patches)
    }

    /// The per-file diff between package `old_package` in project
    /// `old_project` and package `new_package` in project
    /// `new_project`: the files added, modified (contents differ),
    /// or deleted, each with a unified diff. Unchanged files do not
    /// appear. The two sides may be differently-named packages; the
    /// same-name case is the per-package half of
    /// [`ObsClient::project_diff`].
    ///
    /// Two public reads (`GET /public/source/<project>/<package>`,
    /// one per side) plus one public file read per added or deleted
    /// file and two per changed file: a public read, no credentials.
    /// A failed read aborts the whole diff with its error, never a
    /// partial list — including a missing package on either side.
    pub fn diff_packages(
        &self,
        old_project: &str,
        old_package: &str,
        new_project: &str,
        new_package: &str,
    ) -> Result<Vec<FilePatch>, GeectlError> {
        for (label, value) in [
            ("old project", old_project),
            ("old package", old_package),
            ("new project", new_project),
            ("new package", new_package),
        ] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        let old_files = self.list_files(old_project, old_package, None)?;
        let new_files = self.list_files(new_project, new_package, None)?;
        self.diff_files(
            old_project,
            old_package,
            &old_files,
            new_project,
            new_package,
            &new_files,
        )
    }

    /// Diff two projects package by package (`old` vs `new`): every
    /// package in either project is classified — identical (same
    /// files, same contents), differs (files added, removed, or
    /// changed), only-in-old, or only-in-new — with a per-file
    /// unified diff for the packages that differ. `include` narrows
    /// the diff to the named packages (empty means all); `exclude`
    /// drops the named packages. With `annotate_requests`, each
    /// differing package is annotated with the ids of the open
    /// submit requests involving it in the `new` project (which
    /// requires credentials, like
    /// [`ObsClient::open_requests_for_package`]).
    ///
    /// A package only in the new project is its own status
    /// (`OnlyInNew`), not folded into identical: the caller decides
    /// what a new package means. The file comparison is by md5, so a
    /// file that only changed metadata does not count as changed.
    ///
    /// Cost: the diff is one sequential fan-out — two package
    /// listings, then per package present in both projects two file
    /// listings plus one file read per added or deleted file and two
    /// per changed file, and with `annotate_requests` one open-request
    /// search per differing package. A large project is that many
    /// sequential round-trips, with no concurrency bound and no cap:
    /// the client is synchronous (`&dyn HttpClient` carries no `Sync`
    /// bound, and the test fake is `RefCell`-based), so the fan-out
    /// cannot be parallelized without a transport refactor. Each
    /// item still fails closed: the first failed read aborts the
    /// whole diff with its error, never a partial list.
    pub fn project_diff(
        &self,
        old: &str,
        new: &str,
        include: &[&str],
        exclude: &[&str],
        annotate_requests: bool,
    ) -> Result<Vec<PackageDiff>, GeectlError> {
        for (label, value) in [("old project", old), ("new project", new)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        for name in include.iter().chain(exclude.iter()) {
            if !valid_obs_name(name) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name: {name:?}"),
                ));
            }
        }
        let old_packages = self.list_packages(old)?;
        let new_packages = self.list_packages(new)?;
        let mut names: Vec<String> = old_packages
            .iter()
            .chain(new_packages.iter())
            .cloned()
            .collect();
        names.sort();
        names.dedup();
        let mut diffs = Vec::new();
        for name in names {
            if !include.is_empty() && !include.contains(&name.as_str()) {
                continue;
            }
            if exclude.contains(&name.as_str()) {
                continue;
            }
            let in_old = old_packages.contains(&name);
            let in_new = new_packages.contains(&name);
            let (status, files) = match (in_old, in_new) {
                (true, false) => (PackageDiffStatus::OnlyInOld, Vec::new()),
                (false, true) => (PackageDiffStatus::OnlyInNew, Vec::new()),
                (true, true) => {
                    let old_files = self.list_files(old, &name, None)?;
                    let new_files = self.list_files(new, &name, None)?;
                    let files = self.diff_files(old, &name, &old_files, new, &name, &new_files)?;
                    let status = if files.is_empty() {
                        PackageDiffStatus::Identical
                    } else {
                        PackageDiffStatus::Differs
                    };
                    (status, files)
                }
                (false, false) => unreachable!("package in neither project"),
            };
            let requests = if annotate_requests && status == PackageDiffStatus::Differs {
                self.open_requests_for_package(new, &name)?
            } else {
                Vec::new()
            };
            diffs.push(PackageDiff {
                name,
                status,
                files,
                requests,
            });
        }
        Ok(diffs)
    }

    /// Build the OBS search URL for open submit requests targeting
    /// `package`. The match expression is XPath over the request XML;
    /// the submit restriction is applied client-side in
    /// `parse_submit_request_ids`, so a request of another type that
    /// targets the package does not block filing.
    /// The package name goes into a single-quoted XPath string
    /// literal, so it must be a valid OBS package name
    /// (`[A-Za-z0-9_.+-:]`); anything else is rejected rather than
    /// quoted, because XPath has no string escape and a quote in the
    /// name would break out of the literal.
    pub fn search_url(&self, package: &str) -> Result<String, GeectlError> {
        if !valid_obs_name(package) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS package name: {package:?}"),
            ));
        }
        // The match narrows to actions targeting the package in the
        // open states. It does NOT filter by action type: the submit
        // filter lives client-side in `parse_submit_request_ids`, on
        // the response the server actually returns. The search
        // engine does whitelist `action/@type`, so the match could
        // filter server-side — that is a follow-up, not this change;
        // the client-side filter stays as defence-in-depth.
        let match_expr = format!(
            "action/target/@package='{package}' and (state/@name='new' or state/@name='review')"
        );
        Ok(build_url(
            &self.base,
            &["search", "request"],
            &[("match", &match_expr)],
        ))
    }

    /// Delete one file from a package on OBS
    /// (`DELETE /source/<project>/<package>/<filename>`). This is an
    /// internal step — detaching a branch whose link target is gone
    /// drops the `_link` — not a public operation, so the caller owns
    /// the dry-run guard. Success is the `<status code="ok"/>` body:
    /// the API discards the backend's revision and reports only
    /// that the delete landed, so no revision is returned.
    pub(crate) fn delete_file(
        &self,
        project: &str,
        package: &str,
        filename: &str,
    ) -> Result<(), GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if !valid_obs_filename(filename) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS file name: {filename:?}"),
            ));
        }
        let url = build_url(
            &self.base,
            &["source", project, package, filename],
            &[("comment", format!("removed {filename}").as_str())],
        );
        let body = self.http.delete(&url, &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS delete response for file {project}/{package}/{filename}"),
            ))
        }
    }

    /// Delete the `_link` of package `package` in project `project`
    /// on OBS (`DELETE /source/<project>/<package>/_link`).
    /// Requires credentials; without them the call fails loudly
    /// before any request. In dry-run the delete is recorded in the
    /// plan and nothing is deleted — the guard runs before the
    /// credentials check, like `create_package`.
    ///
    /// An agent deletes a link to unlink a package from its source:
    /// the package keeps its files and meta, but stops tracking the
    /// source. A link that is not there is refused by the server.
    ///
    /// Success is the `<status code="ok"/>` body; any other body is
    /// a server error, never success.
    pub fn delete_link(
        &self,
        project: &str,
        package: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if !dry_run.allow(&format!("delete link of {project}/{package}")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot delete the link of a package without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        self.delete_file(project, package, "_link")
    }

    /// Delete project `project` on OBS (`DELETE /source/<project>`).
    /// Requires credentials; without them the call fails loudly
    /// before any request. In dry-run the delete is recorded in the
    /// plan and nothing is deleted — the guard runs before the
    /// credentials check, like `create_package`.
    ///
    /// An agent deletes a project to remove a scratch or superseded
    /// project and everything in it. The server deletes a non-empty
    /// project without complaint (it removes the packages after
    /// replying), so the must-be-empty guard is client-side: without
    /// `recursive` the packages are listed first and a project
    /// holding packages is refused. `force` ignores other projects
    /// depending on this one; `comment` is recorded on the delete.
    ///
    /// The list-then-delete is best-effort, not atomic: a package
    /// created between the listing and the delete is deleted with
    /// the project anyway. There is no server-side conditional
    /// delete, so the window cannot be closed from the client; the
    /// guard only catches a project that already holds packages
    /// when it is listed.
    ///
    /// Success is the `<status code="ok"/>` body; any other body is a
    /// server error, never success. Deleting a project that does not
    /// exist reads as `NotFound` from the package-list read (or from
    /// the delete itself, when recursive).
    pub fn delete_project(
        &self,
        project: &str,
        options: &DeleteProjectOptions,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        let mut plan = format!("delete project {project}");
        if options.recursive {
            plan.push_str(" with all its packages");
        }
        if options.force {
            plan.push_str(" (force)");
        }
        // The dry-run guard: in dry-run the delete is recorded in
        // the plan and nothing is deleted — not even the credentials
        // check runs, and the package list is not read.
        if !dry_run.allow(&plan) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot delete a project without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        // The server deletes a non-empty project without complaint,
        // so the must-be-empty guard is client-side: list the
        // packages first and refuse a non-empty project without
        // `recursive`.
        if !options.recursive {
            let url = build_url(&self.base, &["source", project], &[]);
            let directory = self.http.get_with(&url, &self.credentials)?;
            let packages = parse_package_names(&directory)?;
            if !packages.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "project {project} contains packages; it must be empty before deleting it (use recursive to delete it with all its packages)"
                    ),
                ));
            }
        }
        let mut query: Vec<(&str, &str)> = Vec::new();
        if options.force {
            query.push(("force", "1"));
        }
        if let Some(comment) = &options.comment {
            query.push(("comment", comment));
        }
        let url = build_url(&self.base, &["source", project], &query);
        let body = self.http.delete(&url, &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS delete response for project {project}"),
            ))
        }
    }

    /// Restore a deleted package on OBS (`POST
    /// /source/<project>/<package>?cmd=undelete`). Requires
    /// credentials; without them the call fails loudly before any
    /// request. In dry-run the restore is recorded in the plan and
    /// nothing is restored — the guard runs before the credentials
    /// check, like `create_package`.
    ///
    /// An agent restores a package it deleted by mistake (or that a
    /// cleanup deleted too eagerly): the server restores the package
    /// including its sources and meta — binaries are lost and
    /// rebuild. The server records 'package was undeleted' when no
    /// comment is given, so no comment is sent here. Restoring a
    /// package that already exists, or one that was never deleted,
    /// is refused by the server.
    ///
    /// Success is the `<status code="ok"/>` body; any other body is a
    /// server error, never success.
    pub fn undelete_package(
        &self,
        project: &str,
        package: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        // The dry-run guard: in dry-run the restore is recorded in
        // the plan and nothing is restored — not even the credentials
        // check runs.
        if !dry_run.allow(&format!("restore deleted package {project}/{package}")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot restore a deleted package without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(
            &self.base,
            &["source", project, package],
            &[("cmd", "undelete")],
        );
        let body = self.http.post(&url, "", &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS undelete response for package {project}/{package}"),
            ))
        }
    }

    /// Restore a deleted project on OBS (`POST
    /// /source/<project>?cmd=undelete`). Requires credentials; without
    /// them the call fails loudly before any request. In dry-run the
    /// restore is recorded in the plan and nothing is restored — the
    /// guard runs before the credentials check, like `create_package`.
    ///
    /// An agent restores a project it deleted by mistake: the server
    /// restores the project including its packages' sources and meta
    /// — binaries are lost and rebuild. The server records 'project
    /// was undeleted' when no comment is given, so no comment is sent
    /// here. Restoring a project that already exists, or one that was
    /// never deleted, is refused by the server.
    ///
    /// Success is the `<status code="ok"/>` body; any other body is a
    /// server error, never success.
    pub fn undelete_project(&self, project: &str, dry_run: &DryRun) -> Result<(), GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        // The dry-run guard: in dry-run the restore is recorded in
        // the plan and nothing is restored — not even the credentials
        // check runs.
        if !dry_run.allow(&format!("restore deleted project {project}")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot restore a deleted project without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["source", project], &[("cmd", "undelete")]);
        let body = self.http.post(&url, "", &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS undelete response for project {project}"),
            ))
        }
    }

    /// Aggregate a package's binaries into another package on OBS:
    /// PUT an `_aggregate` file naming the source, and when the target
    /// is new, copy the source package's meta to it first (without the
    /// source's scmsync — an aggregate only cares about binaries, and
    /// keeping the scmsync would block creating the `_aggregate`
    /// file). Requires credentials; without them the call fails loudly
    /// before any request. In dry-run the aggregate is recorded in the
    /// plan and nothing is aggregated — the guard runs before the
    /// credentials check, like `create_package`.
    ///
    /// An agent aggregates a package to ship another package's built
    /// binaries under its own name (for example a -devel package
    /// aggregating its base package's binaries): the target builds
    /// nothing itself, it collects the source's binaries at build
    /// time. The target meta is created from the source meta when the
    /// target does not exist yet (or only exists through a project
    /// link); an existing target keeps its own meta. An `_aggregate`
    /// file that already exists is refused, before anything is
    /// written — unlike the command-line client, which writes the
    /// meta first and only then notices.
    ///
    /// The repository mapping is not validated here: the server marks
    /// the aggregate broken at build time when a mapped repository
    /// does not exist. A source and target that are the same package
    /// are refused.
    ///
    /// Success is the revision document on the `_aggregate` PUT; any
    /// other body is a server error, never success.
    pub fn aggregate_package(
        &self,
        source_project: &str,
        source_package: &str,
        target_project: &str,
        target_package: &str,
        options: &AggregateOptions,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        for (label, value) in [
            ("project", source_project),
            ("package", source_package),
            ("project", target_project),
            ("package", target_package),
        ] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        for (source, target) in &options.repository_map {
            for (label, value) in [("repository", source), ("repository", target)] {
                if !valid_obs_name(value) {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!("invalid OBS {label} name: {value:?}"),
                    ));
                }
            }
        }
        if source_project == target_project && source_package == target_package {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "cannot aggregate a package into itself",
            ));
        }
        // The dry-run guard: in dry-run the aggregate is recorded in
        // the plan and nothing is aggregated — not even the
        // credentials check runs.
        if !dry_run.allow(&format!(
            "aggregate package {source_project}/{source_package} into {target_project}/{target_package}{}",
            aggregate_option_suffix(options),
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot aggregate a package without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let source_url = build_url(
            &self.base,
            &["source", source_project, source_package, "_meta"],
            &[],
        );
        let source_meta = self.http.get_with(&source_url, &self.credentials)?;
        let target_url = build_url(
            &self.base,
            &["source", target_project, target_package, "_meta"],
            &[],
        );
        let target_meta = match self.http.get_with(&target_url, &self.credentials) {
            Ok(meta) => Some(meta),
            Err(err) if err.kind() == ErrorKind::NotFound => None,
            Err(err) => return Err(err),
        };
        // The target meta is built from the source meta when the
        // target does not exist yet, or when the target only exists
        // through a project link (its meta names another project):
        // the shadow must not supply the target's meta.
        let (mut doc, put_meta) = match &target_meta {
            Some(meta) => {
                let doc = XmlDoc::parse(meta)?;
                if doc.root_attr("project").as_deref() == Some(target_project) {
                    (doc, false)
                } else {
                    (
                        build_link_meta(&source_meta, target_project, target_package)?,
                        true,
                    )
                }
            }
            None => (
                build_link_meta(&source_meta, target_project, target_package)?,
                true,
            ),
        };
        if options.disable_publish {
            doc.replace_or_insert_element("publish", "<publish><disable/></publish>")?;
        }
        // Refuse an existing `_aggregate` before writing anything:
        // a refusal leaves nothing changed.
        let aggregate_url = build_url(
            &self.base,
            &["source", target_project, target_package, "_aggregate"],
            &[],
        );
        match self.http.get_with(&aggregate_url, &self.credentials) {
            Ok(_) => {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "package {target_project}/{target_package} already has an _aggregate file"
                    ),
                ));
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        let rendered = doc.render();
        if put_meta || target_meta.as_deref() != Some(rendered.as_str()) {
            let body = self.http.put(&target_url, &rendered, &self.credentials)?;
            if !parse_status_ok(&body) {
                return Err(GeectlError::new(
                    ErrorKind::ServerError,
                    format!(
                        "unexpected OBS meta response for package {target_project}/{target_package}"
                    ),
                ));
            }
        }
        let mut aggregate = format!(
            "<aggregatelist><aggregate project=\"{source_project}\"><package>{source_package}</package>"
        );
        if options.no_sources {
            aggregate.push_str("<nosources/>");
        }
        for (source, target) in &options.repository_map {
            aggregate.push_str(&format!(
                "<repository target=\"{target}\" source=\"{source}\"/>"
            ));
        }
        aggregate.push_str("</aggregate></aggregatelist>");
        let body = self
            .http
            .put_file(&aggregate_url, &aggregate, &self.credentials)?;
        if parse_revision(&body).is_none() {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected OBS aggregate response for package {target_project}/{target_package}"
                ),
            ));
        }
        Ok(())
    }

    /// Release a package's sources and binaries on OBS (`POST
    /// /source/<project>/<package>?cmd=release`). Requires
    /// credentials; without them the call fails loudly before any
    /// request. In dry-run the release is recorded in the plan and
    /// nothing is released — the guard runs before the credentials
    /// check, like `create_package`.
    ///
    /// An agent releases a package to transfer its sources and
    /// binaries to a release target without rebuilding them. The
    /// release is accepted either way — the server answers `ok` when
    /// it runs synchronously and `invoked` when it queues the release
    /// as a job — so the caller polls for the released binaries when
    /// it needs them, rather than assuming they landed.
    pub fn release_package(
        &self,
        project: &str,
        package: &str,
        options: &ReleaseOptions,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        validate_release_options(options)?;
        if !dry_run.allow(&format!(
            "release package {project}/{package}{}",
            release_option_suffix(options),
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot release a package without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        self.release(
            &[project, package],
            options,
            &format!("release package {project}/{package}"),
        )
    }

    /// Release a whole project's sources and binaries on OBS (`POST
    /// /source/<project>?cmd=release`). Requires credentials; without
    /// them the call fails loudly before any request. In dry-run the
    /// release is recorded in the plan and nothing is released — the
    /// guard runs before the credentials check, like `create_package`.
    ///
    /// An agent releases a project to transfer all its packages'
    /// sources and binaries to a release target without rebuilding
    /// them. A project release is queued as a job unless `no_delay`
    /// is set, so the server answers `invoked` — the release lands
    /// asynchronously, and the caller polls for the released binaries
    /// when it needs them.
    pub fn release_project(
        &self,
        project: &str,
        options: &ReleaseOptions,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        if !valid_obs_name(project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {project:?}"),
            ));
        }
        validate_release_options(options)?;
        if !dry_run.allow(&format!(
            "release project {project}{}",
            release_option_suffix(options),
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot release a project without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        self.release(&[project], options, &format!("release project {project}"))
    }

    /// The shared release POST for the package and project forms.
    /// `path` is the source path segments, `what` names the release
    /// in errors.
    fn release(
        &self,
        path: &[&str],
        options: &ReleaseOptions,
        what: &str,
    ) -> Result<(), GeectlError> {
        let mut query: Vec<(&str, &str)> = vec![("cmd", "release")];
        if let Some(repository) = &options.repository {
            query.push(("repository", repository));
        }
        if let Some(architecture) = &options.architecture {
            query.push(("arch", architecture));
        }
        if let Some(target_project) = &options.target_project {
            query.push(("target_project", target_project));
        }
        if let Some(target_repository) = &options.target_repository {
            query.push(("target_repository", target_repository));
        }
        if let Some(set_release) = &options.set_release {
            query.push(("setrelease", set_release));
        }
        if options.no_delay {
            query.push(("nodelay", "1"));
        }
        let mut segments = vec!["source"];
        segments.extend_from_slice(path);
        let url = build_url(&self.base, &segments, &query);
        let body = self.http.post(&url, "", &self.credentials)?;
        if parse_status_ok(&body) || parse_status_invoked(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS release response for {what}"),
            ))
        }
    }

    /// Create package `package` in project `project` on OBS (`PUT
    /// /source/<project>/<package>/_meta`) with the given title and
    /// description, optionally wiring it to a git repository through
    /// `<scmsync>`. Requires credentials; without them the call fails
    /// loudly before any request. In dry-run the creation is recorded
    /// in the plan and nothing is created — the guard runs before the
    /// credentials check, like `create_package`.
    ///
    /// An agent creates the package it submits from — the branch
    /// package — and a git-backed package is created the same way, by
    /// naming its repository and branch in `scmsync`: the package
    /// then syncs its sources from git instead of holding uploaded
    /// files. That is the unified operation: one creation, OBS-native
    /// or git-backed, distinguished only by the `scmsync` option.
    ///
    /// Creating in a project whose own meta carries `<scmsync>` is
    /// refused: the project is git-managed, so its packages are
    /// created by the sync, not by hand. An existing package is
    /// refused unless `force` is set, so a re-run never silently
    /// overwrites the first creation. A scmsync without a `#fragment`
    /// is refused: the fragment names the branch to check out.
    /// Success is the `<status code="ok"/>` body: any other body is
    /// a server error, never success.
    pub fn create_package_with_options(
        &self,
        project: &str,
        package: &str,
        title: &str,
        description: &str,
        options: &CreatePackageOptions,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        if let Some(scmsync) = &options.scmsync {
            validate_scmsync(scmsync)?;
        }
        if !dry_run.allow(&format!(
            "create package {project}/{package}{}",
            create_package_option_suffix(options),
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot create a package without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        // A project managed in git creates its own packages through
        // the sync: refuse to create one by hand. The project must
        // exist — a missing project dies here, even with force.
        let project_url = build_url(&self.base, &["source", project, "_meta"], &[]);
        let project_meta = self.http.get_with(&project_url, &self.credentials)?;
        if XmlDoc::parse(&project_meta)?.child("scmsync").is_some() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "cannot create a package in project {project}: the project is managed in git (its meta carries <scmsync>)"
                ),
            ));
        }
        let package_url = build_url(&self.base, &["source", project, package, "_meta"], &[]);
        if !options.force {
            match self.http.get_with(&package_url, &self.credentials) {
                Ok(_) => {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!("package {project}/{package} already exists"),
                    ));
                }
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
        }
        let scmsync = match &options.scmsync {
            Some(scmsync) => format!("<scmsync>{}</scmsync>", xml_escape(scmsync)),
            None => String::new(),
        };
        let meta = format!(
            "<package name=\"{}\" project=\"{}\"><title>{}</title><description>{}</description>{}</package>",
            xml_escape(package),
            xml_escape(project),
            xml_escape(title),
            xml_escape(description),
            scmsync,
        );
        let body = self.http.put(&package_url, &meta, &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS create-package response for package {project}/{package}"),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::Debug;
    use crate::error::ErrorKind;
    use crate::obs::test_support::{basic_credentials, search_without_states, FakeHttp};

    #[test]
    fn devel_project_extracted() {
        // The devel package differs from the queried package: the
        // pair must carry it, not just the project.
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta",
            "<package name=\"foo\" project=\"openSUSE:Factory\"><devel project=\"devel:languages:python\" package=\"python-foo\"/></package>",
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.devel_project("openSUSE:Factory", "foo").unwrap(),
            Some((
                "devel:languages:python".to_string(),
                "python-foo".to_string()
            ))
        );
        // The public read carries no credentials.
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta".to_string(),
                false
            )]
        );
    }

    #[test]
    fn devel_project_missing_is_none() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta",
            "<package name=\"foo\" project=\"openSUSE:Factory\"></package>",
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.devel_project("openSUSE:Factory", "foo").unwrap(),
            None
        );
    }

    #[test]
    fn devel_project_missing_package_is_none() {
        // A 404 means the package is not there, so it has no devel
        // mapping — that is None, not a failed lookup. Any other
        // failure stays an error (see the transport-failure test).
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta",
            ErrorKind::NotFound,
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.devel_project("openSUSE:Factory", "foo").unwrap(),
            None
        );
    }

    #[test]
    fn devel_project_attribute_order_does_not_matter() {
        // The package attribute first: the pair must still be found.
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta",
            "<package name=\"foo\" project=\"openSUSE:Factory\"><devel package=\"foo\" project=\"devel:languages:python\"/></package>",
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.devel_project("openSUSE:Factory", "foo").unwrap(),
            Some(("devel:languages:python".to_string(), "foo".to_string()))
        );
    }

    #[test]
    fn devel_project_rejects_invalid_package_name_without_http() {
        // An invalid name is rejected before any request: no canned
        // response exists, so reaching the HTTP client would error
        // differently (FakeHttp: "no canned response").
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client
            .devel_project("openSUSE:Factory", "foo/bar")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let err = client
            .devel_project("openSUSE:Factory/x", "foo")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn devel_project_malformed_body_is_error_not_none() {
        // A truncated 200 body must not read as "no devel mapping".
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta",
            "<package name=\"foo\" project=\"openSUSE:Factory\"><devel project=\"devel:languages:python\"",
        );
        let client = ObsClient::new(&http);
        let err = client.devel_project("openSUSE:Factory", "foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn devel_project_invalid_project_from_xml_is_error() {
        // The network-sourced project is validated at the source: a
        // name that cannot go into a URL is unexpected server data,
        // not a caller mistake — the caller-supplied package name
        // (see the invalid-package test above) is what InvalidInput
        // is for.
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta",
            "<package name=\"foo\" project=\"openSUSE:Factory\"><devel project=\"devel/foo\" package=\"foo\"/></package>",
        );
        let client = ObsClient::new(&http);
        let err = client.devel_project("openSUSE:Factory", "foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn devel_project_invalid_package_from_xml_is_error() {
        // The package half is validated at the source too.
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta",
            "<package name=\"foo\" project=\"openSUSE:Factory\"><devel project=\"devel:languages:python\" package=\"python/foo\"/></package>",
        );
        let client = ObsClient::new(&http);
        let err = client.devel_project("openSUSE:Factory", "foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn devel_project_without_package_defaults_to_queried_package() {
        // A `<devel>` element without a package attribute means the
        // devel package keeps this package's name: the pair carries
        // the queried package name.
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta",
            "<package name=\"foo\" project=\"openSUSE:Factory\"><devel project=\"devel:languages:python\"/></package>",
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.devel_project("openSUSE:Factory", "foo").unwrap(),
            Some(("devel:languages:python".to_string(), "foo".to_string()))
        );
    }

    #[test]
    fn devel_project_transport_failure_is_error_not_none() {
        // A failed lookup must never read as "no devel mapping": no
        // canned response is a transport error, not None.
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client.devel_project("openSUSE:Factory", "foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NetworkError);
    }

    #[test]
    fn devel_version_reads_the_spec() {
        // The spec is read from the devel package, whose name differs
        // from the queried package here.
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta",
                "<package name=\"foo\" project=\"openSUSE:Factory\"><devel project=\"devel:languages:python\" package=\"python-foo\"/></package>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/devel:languages:python/python-foo/python-foo.spec",
                "Name: python-foo\nVersion: 2.0\nRelease: 1\n",
            );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.devel_version("foo").unwrap(),
            Some("2.0".to_string())
        );
    }

    #[test]
    fn devel_version_none_when_no_devel_project() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta",
            "<package name=\"foo\" project=\"openSUSE:Factory\"></package>",
        );
        let client = ObsClient::new(&http);
        assert_eq!(client.devel_version("foo").unwrap(), None);
    }

    #[test]
    fn devel_version_missing_spec_is_not_found() {
        // The devel package carries no spec: the spec read 404s, and
        // that is an error, not None — None covers no mapping or a
        // spec with no Version only.
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta",
                "<package name=\"foo\" project=\"openSUSE:Factory\"><devel project=\"devel:languages:python\" package=\"python-foo\"/></package>",
            )
            .with_get_error(
                "https://api.opensuse.org/public/source/devel:languages:python/python-foo/python-foo.spec",
                ErrorKind::NotFound,
            );
        let client = ObsClient::new(&http);
        let err = client.devel_version("foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn devel_version_rejects_invalid_project_from_xml() {
        // A project name from network XML is validated too: a value
        // that cannot be a project name is unexpected server data,
        // not a caller mistake, so it errors rather than being
        // interpolated into a URL.
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta",
            "<package name=\"foo\" project=\"openSUSE:Factory\"><devel project=\"weird/project\" package=\"foo\"/></package>",
        );
        let client = ObsClient::new(&http);
        let err = client.devel_version("foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn search_url_rejects_invalid_package_name() {
        // A quote in the name would break out of the XPath string literal.
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.search_url("foo' or '1'='1").is_err());
        assert!(client.search_url("").is_err());
        assert!(client.search_url("foo bar").is_err());
        // Valid OBS names pass.
        assert!(client.search_url("python3-requests").is_ok());
        assert!(client.search_url("foo:bar").is_ok());
    }

    #[test]
    fn parse_spec_version_variants() {
        assert_eq!(
            parse_spec_version("Name: foo\nVersion: 1.2.3\nRelease: 1\n"),
            Some("1.2.3".to_string())
        );
        assert_eq!(parse_spec_version("Name: foo\nRelease: 1\n"), None);
        assert_eq!(
            parse_spec_version("Version: 2.0~rc1\n"),
            Some("2.0~rc1".to_string())
        );
    }

    #[test]
    fn parse_linkinfo_reads_the_attributes() {
        let body = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"_link\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"120\" mtime=\"1700000000\"/><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" baserev=\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\" xsrcmd5=\"ffffffffffffffffffffffffffffffff\" lastworking=\"33333333333333333333333333333333\" error=\"patch bad.patch failed to apply\"/></directory>";
        let info = parse_linkinfo(body).unwrap().unwrap();
        assert_eq!(info.project.as_deref(), Some("test:factory"));
        assert_eq!(info.package.as_deref(), Some("test-pkgA"));
        assert_eq!(
            info.srcmd5.as_deref(),
            Some("dddddddddddddddddddddddddddddddd")
        );
        assert_eq!(
            info.baserev.as_deref(),
            Some("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
        );
        assert_eq!(
            info.xsrcmd5.as_deref(),
            Some("ffffffffffffffffffffffffffffffff")
        );
        assert_eq!(
            info.lastworking.as_deref(),
            Some("33333333333333333333333333333333")
        );
        assert_eq!(
            info.error.as_deref(),
            Some("patch bad.patch failed to apply")
        );
    }

    #[test]
    fn parse_linkinfo_none_without_linkinfo() {
        let body = "<directory name=\"test:factory/test-pkgA\" rev=\"1\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><entry name=\"test-pkgA.spec\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"100\" mtime=\"1700000000\"/></directory>";
        assert_eq!(parse_linkinfo(body).unwrap(), None);
        let body = "<directory name=\"test:factory/test-pkgA\" rev=\"1\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"/>";
        assert_eq!(parse_linkinfo(body).unwrap(), None);
    }

    #[test]
    fn parse_linkinfo_malformed_body_is_error_not_none() {
        assert!(parse_linkinfo("garbage").is_err());
        assert!(parse_linkinfo("<directory name=\"x\">").is_err());
        assert!(parse_linkinfo("<collection/>").is_err());
    }

    #[test]
    fn parse_linkinfo_unescapes_the_error() {
        let body = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><linkinfo project=\"test:factory\" package=\"test-pkgA\" error=\"patch &quot;bad.patch&quot; failed\"/></directory>";
        let info = parse_linkinfo(body).unwrap().unwrap();
        assert_eq!(info.error.as_deref(), Some("patch \"bad.patch\" failed"));
    }

    #[test]
    fn parse_linkinfo_reads_an_error_containing_an_angle_bracket() {
        // A `>` inside the error text is legal XML and must not end
        // the tag: truncating there drops the error, and a broken
        // link would read as unbroken.
        let body = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><linkinfo project=\"test:factory\" package=\"test-pkgA\" error=\"patch failed: a > b\"/></directory>";
        let info = parse_linkinfo(body).unwrap().unwrap();
        assert_eq!(info.error.as_deref(), Some("patch failed: a > b"));
    }

    #[test]
    fn parse_linkinfo_reads_attrs_with_linked_children() {
        let body = "<directory name=\"test:factory/test-pkgA\" rev=\"2\" vrev=\"2\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"><linkinfo project=\"test:factory\" package=\"test-pkgA\" srcmd5=\"dddddddddddddddddddddddddddddddd\" error=\"patch failed\"><linked project=\"test:factory\" package=\"test-pkgB\"/></linkinfo></directory>";
        let info = parse_linkinfo(body).unwrap().unwrap();
        assert_eq!(
            info.srcmd5.as_deref(),
            Some("dddddddddddddddddddddddddddddddd")
        );
        assert_eq!(info.error.as_deref(), Some("patch failed"));
    }

    #[test]
    fn parse_linkinfo_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the <directory>
        // element; the result is the same as without one.
        let plain = "<directory name=\"test:factory/test-pkgA\"/>";
        let prolog = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{plain}");
        assert_eq!(
            parse_linkinfo(&prolog).unwrap(),
            parse_linkinfo(plain).unwrap()
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_linkinfo(&format!("<?xml?>{plain}")).unwrap(),
            parse_linkinfo(plain).unwrap()
        );
        assert_eq!(
            parse_linkinfo(&format!("<?xml version=\"1.0\"?>\n{plain}")).unwrap(),
            parse_linkinfo(plain).unwrap()
        );
        // A prolog that never terminates is malformed, not a missing <directory>.
        assert!(parse_linkinfo("<?xml version=\"1.0\" <directory></directory>").is_err());
        assert!(parse_linkinfo("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(parse_linkinfo("<?xmlfoo version=\"1.0\"?><directory></directory>").is_err());
    }

    #[test]
    fn parse_linkinfo_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_linkinfo("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_linkinfo(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_linkinfo("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn parse_directory_rev_reads_rev_and_vrev() {
        let body = "<directory name=\"srcpkg\" rev=\"42\" srcmd5=\"ffffffffffffffffffffffffffffffff\" vrev=\"1\"><entry md5=\"0d62ceea6020d75154078a20d8c9f9ba\" mtime=\"1282047302\" name=\"foo\" size=\"23\" /></directory>";
        assert_eq!(
            parse_directory_rev(body).unwrap(),
            (
                Some("42".to_string()),
                Some("1".to_string()),
                Some("ffffffffffffffffffffffffffffffff".to_string()),
            )
        );
        let body = "<directory name=\"srcpkg\" srcmd5=\"ffffffffffffffffffffffffffffffff\"/>";
        assert_eq!(
            parse_directory_rev(body).unwrap(),
            (
                None,
                None,
                Some("ffffffffffffffffffffffffffffffff".to_string()),
            )
        );
    }

    #[test]
    fn parse_directory_rev_malformed_body_is_error() {
        assert!(parse_directory_rev("garbage").is_err());
        assert!(parse_directory_rev("<directory name=\"x\">").is_err());
        assert!(parse_directory_rev("<collection/>").is_err());
    }

    #[test]
    fn parse_directory_rev_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the directory
        // element; the result is the same as without one.
        let body = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><directory name=\"srcpkg\" rev=\"42\" srcmd5=\"ffffffffffffffffffffffffffffffff\" vrev=\"1\"><entry md5=\"0d62ceea6020d75154078a20d8c9f9ba\" mtime=\"1282047302\" name=\"foo\" size=\"23\" /></directory>";
        assert_eq!(
            parse_directory_rev(body).unwrap(),
            (
                Some("42".to_string()),
                Some("1".to_string()),
                Some("ffffffffffffffffffffffffffffffff".to_string()),
            )
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_directory_rev(
                "<?xml?><directory name=\"srcpkg\" srcmd5=\"ffffffffffffffffffffffffffffffff\"/>"
            )
            .unwrap(),
            (
                None,
                None,
                Some("ffffffffffffffffffffffffffffffff".to_string()),
            )
        );
        assert_eq!(
            parse_directory_rev("<?xml version=\"1.0\"?>\n<directory name=\"srcpkg\" rev=\"7\"/>")
                .unwrap(),
            (Some("7".to_string()), None, None)
        );
        // A prolog that never terminates is malformed, not a missing directory.
        assert!(parse_directory_rev("<?xml version=\"1.0\" <directory></directory>").is_err());
        assert!(parse_directory_rev("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(parse_directory_rev("<?xmlfoo version=\"1.0\"?><directory></directory>").is_err());
    }

    #[test]
    fn parse_directory_rev_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_directory_rev("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_directory_rev(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_directory_rev("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn parse_package_names_reads_entry_names() {
        let body = "<directory name=\"test:factory\"><entry name=\"pkgA\"/><entry name=\"pkgB\"/><entry name=\"pkgC\"/></directory>";
        assert_eq!(
            parse_package_names(body).unwrap(),
            vec!["pkgA".to_string(), "pkgB".to_string(), "pkgC".to_string()]
        );
        let body = "<directory name=\"test:factory\"/>";
        assert_eq!(parse_package_names(body).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn parse_package_names_malformed_body_is_error() {
        assert!(parse_package_names("garbage").is_err());
        assert!(parse_package_names("<directory name=\"x\">").is_err());
        let body = "<directory name=\"test:factory\"><entry size=\"10\"/></directory>";
        assert!(parse_package_names(body).is_err());
    }

    #[test]
    fn parse_package_names_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the directory
        // element; the result is the same as without one.
        let body = "<?xml version=\"1.0\" encoding=\"UTF-8\"?><directory name=\"test:factory\"><entry name=\"pkgA\"/><entry name=\"pkgB\"/></directory>";
        assert_eq!(
            parse_package_names(body).unwrap(),
            vec!["pkgA".to_string(), "pkgB".to_string()]
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_package_names("<?xml?><directory name=\"test:factory\"/>").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            parse_package_names(
                "<?xml version=\"1.0\"?>\n<directory name=\"test:factory\"><entry name=\"pkgA\"/></directory>"
            )
            .unwrap(),
            vec!["pkgA".to_string()]
        );
        // A prolog that never terminates is malformed, not an empty listing.
        assert!(parse_package_names("<?xml version=\"1.0\" <directory></directory>").is_err());
        assert!(parse_package_names("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(parse_package_names("<?xmlfoo version=\"1.0\"?><directory></directory>").is_err());
    }

    #[test]
    fn parse_package_names_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_package_names("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_package_names(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_package_names("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn delete_project_deletes_an_empty_project() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory",
                "<directory name=\"test:factory\"/>",
            )
            .with_delete(
                "https://api.opensuse.org/source/test:factory",
                "<status code=\"ok\"/>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .delete_project(
                "test:factory",
                &DeleteProjectOptions::default(),
                &DryRun::new(false),
            )
            .is_ok());
        assert_eq!(http.get_calls().len(), 1);
        let deletes = http.delete_calls();
        assert_eq!(deletes.len(), 1);
        assert_eq!(deletes[0].0, "https://api.opensuse.org/source/test:factory");
        assert!(deletes[0].1);
    }

    #[test]
    fn delete_project_refuses_a_non_empty_project_without_recursive() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/test:factory",
            "<directory name=\"test:factory\"><entry name=\"pkgA\"/></directory>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .delete_project(
                "test:factory",
                &DeleteProjectOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        // The refusal happens before the delete: nothing is deleted.
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn delete_project_recursive_deletes_without_the_package_check() {
        let http = FakeHttp::new().with_delete(
            "https://api.opensuse.org/source/test:factory?force=1&comment=cleanup",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = DeleteProjectOptions {
            recursive: true,
            force: true,
            comment: Some("cleanup".to_string()),
        };
        assert!(client
            .delete_project("test:factory", &options, &DryRun::new(false))
            .is_ok());
        // Recursive skips the package list entirely: no GET.
        assert!(http.get_calls().is_empty());
        assert_eq!(http.delete_calls().len(), 1);
    }

    #[test]
    fn delete_project_non_ok_body_is_error_not_success() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory",
                "<directory name=\"test:factory\"/>",
            )
            .with_delete(
                "https://api.opensuse.org/source/test:factory",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .delete_project(
                "test:factory",
                &DeleteProjectOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn delete_project_dry_run_records_plan_and_deletes_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert!(client
            .delete_project("test:factory", &DeleteProjectOptions::default(), &dry_run,)
            .is_ok());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: delete project test:factory".to_string()]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn delete_project_dry_run_names_recursive_and_force() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        let options = DeleteProjectOptions {
            recursive: true,
            force: true,
            comment: None,
        };
        assert!(client
            .delete_project("test:factory", &options, &dry_run)
            .is_ok());
        assert_eq!(
            dry_run.plan(),
            vec![
                "[dry-run] would: delete project test:factory with all its packages (force)"
                    .to_string()
            ]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn delete_project_without_credentials_fails_closed() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/test:factory",
                "<directory name=\"test:factory\"/>",
            )
            .with_delete(
                "https://api.opensuse.org/source/test:factory",
                "<status code=\"ok\"/>",
            );
        let client = ObsClient::new(&http);
        assert!(client
            .delete_project(
                "test:factory",
                &DeleteProjectOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn delete_project_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .delete_project(
                "test:factory/pkg",
                &DeleteProjectOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn undelete_package_posts_the_command_and_succeeds() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=undelete",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .undelete_package("test:factory", "test-pkgA", &DryRun::new(false))
            .is_ok());
        let posts = http.post_calls();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].1, "");
        assert!(posts[0].2);
    }

    #[test]
    fn undelete_package_non_ok_body_is_error_not_success() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=undelete",
            "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .undelete_package("test:factory", "test-pkgA", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn undelete_package_dry_run_records_plan_and_restores_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert!(client
            .undelete_package("test:factory", "test-pkgA", &dry_run)
            .is_ok());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: restore deleted package test:factory/test-pkgA".to_string()]
        );
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn undelete_package_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=undelete",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .undelete_package("test:factory", "test-pkgA", &DryRun::new(false))
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn undelete_package_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .undelete_package("test:factory", "test-pkgA/pkg", &DryRun::new(false))
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn undelete_project_posts_the_command_and_succeeds() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory?cmd=undelete",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .undelete_project("test:factory", &DryRun::new(false))
            .is_ok());
        let posts = http.post_calls();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].1, "");
        assert!(posts[0].2);
    }

    #[test]
    fn undelete_project_non_ok_body_is_error_not_success() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory?cmd=undelete",
            "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .undelete_project("test:factory", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn undelete_project_dry_run_records_plan_and_restores_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert!(client.undelete_project("test:factory", &dry_run).is_ok());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: restore deleted project test:factory".to_string()]
        );
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn undelete_project_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory?cmd=undelete",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .undelete_project("test:factory", &DryRun::new(false))
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn undelete_project_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .undelete_project("test:factory/pkg", &DryRun::new(false))
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn aggregate_package_creates_from_source_meta() {
        let source_meta = "<package name=\"srcpkg\" project=\"srcprj\"><title>Source</title><description>Source package</description><devel project=\"srcdevel\"/><person userid=\"alice\" role=\"maintainer\"/><lock><enable/></lock><scmsync>https://example.com#main</scmsync></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg/_meta",
                source_meta,
            )
            .with_get_error(
                "https://api.opensuse.org/source/dstprj/srcpkg/_meta",
                ErrorKind::NotFound,
            )
            .with_get_error(
                "https://api.opensuse.org/source/dstprj/srcpkg/_aggregate",
                ErrorKind::NotFound,
            )
            .with_put(
                "https://api.opensuse.org/source/dstprj/srcpkg/_meta",
                "<status code=\"ok\"/>",
            )
            .with_put(
                "https://api.opensuse.org/source/dstprj/srcpkg/_aggregate",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .aggregate_package(
                "srcprj",
                "srcpkg",
                "dstprj",
                "srcpkg",
                &AggregateOptions::default(),
                &DryRun::new(false),
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 2);
        // The target meta is built from the source meta: the devel,
        // person, lock, and scmsync entries are stripped, the project
        // and name are set.
        assert_eq!(
            puts[0].1,
            "<package name=\"srcpkg\" project=\"dstprj\"><title>Source</title><description>Source package</description></package>"
        );
        assert_eq!(
            puts[1].1,
            "<aggregatelist><aggregate project=\"srcprj\"><package>srcpkg</package></aggregate></aggregatelist>"
        );
    }

    #[test]
    fn aggregate_package_rename() {
        let source_meta = "<package name=\"srcpkg\" project=\"srcprj\"><title>Source</title><description>Source package</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg/_meta",
                source_meta,
            )
            .with_get_error(
                "https://api.opensuse.org/source/dstprj/dstpkg/_meta",
                ErrorKind::NotFound,
            )
            .with_get_error(
                "https://api.opensuse.org/source/dstprj/dstpkg/_aggregate",
                ErrorKind::NotFound,
            )
            .with_put(
                "https://api.opensuse.org/source/dstprj/dstpkg/_meta",
                "<status code=\"ok\"/>",
            )
            .with_put(
                "https://api.opensuse.org/source/dstprj/dstpkg/_aggregate",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .aggregate_package(
                "srcprj",
                "srcpkg",
                "dstprj",
                "dstpkg",
                &AggregateOptions::default(),
                &DryRun::new(false),
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 2);
        assert_eq!(
            puts[0].1,
            "<package name=\"dstpkg\" project=\"dstprj\"><title>Source</title><description>Source package</description></package>"
        );
        assert_eq!(
            puts[1].1,
            "<aggregatelist><aggregate project=\"srcprj\"><package>srcpkg</package></aggregate></aggregatelist>"
        );
    }

    #[test]
    fn aggregate_package_same_source_and_target_is_refused() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .aggregate_package(
                "srcprj",
                "srcpkg",
                "srcprj",
                "srcpkg",
                &AggregateOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn aggregate_package_refuses_an_existing_aggregate() {
        let source_meta = "<package name=\"srcpkg\" project=\"srcprj\"><title>Source</title><description>Source package</description></package>";
        let target_meta = "<package name=\"dstpkg\" project=\"dstprj\"><title>Target</title><description>Target package</description></package>";
        let aggregate = "<aggregatelist><aggregate project=\"srcprj\"><package>srcpkg</package></aggregate></aggregatelist>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg/_meta",
                source_meta,
            )
            .with_get(
                "https://api.opensuse.org/source/dstprj/dstpkg/_meta",
                target_meta,
            )
            .with_get(
                "https://api.opensuse.org/source/dstprj/dstpkg/_aggregate",
                aggregate,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .aggregate_package(
                "srcprj",
                "srcpkg",
                "dstprj",
                "dstpkg",
                &AggregateOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        // The refusal happens before anything is written.
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn aggregate_package_keeps_an_existing_target_meta() {
        let source_meta = "<package name=\"srcpkg\" project=\"srcprj\"><title>Source</title><description>Source package</description></package>";
        let target_meta = "<package name=\"dstpkg\" project=\"dstprj\"><title>Target</title><description>Target package</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg/_meta",
                source_meta,
            )
            .with_get(
                "https://api.opensuse.org/source/dstprj/dstpkg/_meta",
                target_meta,
            )
            .with_get_error(
                "https://api.opensuse.org/source/dstprj/dstpkg/_aggregate",
                ErrorKind::NotFound,
            )
            .with_put(
                "https://api.opensuse.org/source/dstprj/dstpkg/_aggregate",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .aggregate_package(
                "srcprj",
                "srcpkg",
                "dstprj",
                "dstpkg",
                &AggregateOptions::default(),
                &DryRun::new(false),
            )
            .is_ok());
        // Only the _aggregate is PUT: the existing meta is kept.
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].1,
            "<aggregatelist><aggregate project=\"srcprj\"><package>srcpkg</package></aggregate></aggregatelist>"
        );
    }

    #[test]
    fn aggregate_package_disable_publish_adds_publish_disable() {
        let source_meta = "<package name=\"srcpkg\" project=\"srcprj\"><title>Source</title><description>Source package</description></package>";
        let target_meta = "<package name=\"dstpkg\" project=\"dstprj\"><title>Target</title><description>Target package</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg/_meta",
                source_meta,
            )
            .with_get(
                "https://api.opensuse.org/source/dstprj/dstpkg/_meta",
                target_meta,
            )
            .with_get_error(
                "https://api.opensuse.org/source/dstprj/dstpkg/_aggregate",
                ErrorKind::NotFound,
            )
            .with_put(
                "https://api.opensuse.org/source/dstprj/dstpkg/_meta",
                "<status code=\"ok\"/>",
            )
            .with_put(
                "https://api.opensuse.org/source/dstprj/dstpkg/_aggregate",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = AggregateOptions {
            disable_publish: true,
            ..AggregateOptions::default()
        };
        assert!(client
            .aggregate_package(
                "srcprj",
                "srcpkg",
                "dstprj",
                "dstpkg",
                &options,
                &DryRun::new(false),
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 2);
        assert_eq!(
            puts[0].1,
            "<package name=\"dstpkg\" project=\"dstprj\"><title>Target</title><description>Target package</description><publish><disable/></publish></package>"
        );
    }

    #[test]
    fn aggregate_package_no_sources_and_repository_map_shape_the_document() {
        let source_meta = "<package name=\"srcpkg\" project=\"srcprj\"><title>Source</title><description>Source package</description></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/srcprj/srcpkg/_meta",
                source_meta,
            )
            .with_get_error(
                "https://api.opensuse.org/source/dstprj/srcpkg/_meta",
                ErrorKind::NotFound,
            )
            .with_get_error(
                "https://api.opensuse.org/source/dstprj/srcpkg/_aggregate",
                ErrorKind::NotFound,
            )
            .with_put(
                "https://api.opensuse.org/source/dstprj/srcpkg/_meta",
                "<status code=\"ok\"/>",
            )
            .with_put(
                "https://api.opensuse.org/source/dstprj/srcpkg/_aggregate",
                "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = AggregateOptions {
            repository_map: vec![
                ("standard".to_string(), "distro".to_string()),
                ("images".to_string(), "images".to_string()),
            ],
            no_sources: true,
            ..AggregateOptions::default()
        };
        assert!(client
            .aggregate_package(
                "srcprj",
                "srcpkg",
                "dstprj",
                "srcpkg",
                &options,
                &DryRun::new(false),
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 2);
        assert_eq!(
            puts[1].1,
            "<aggregatelist><aggregate project=\"srcprj\"><package>srcpkg</package><nosources/><repository target=\"distro\" source=\"standard\"/><repository target=\"images\" source=\"images\"/></aggregate></aggregatelist>"
        );
    }

    #[test]
    fn aggregate_package_dry_run_records_plan_and_aggregates_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        let options = AggregateOptions {
            repository_map: vec![("standard".to_string(), "distro".to_string())],
            no_sources: true,
            ..AggregateOptions::default()
        };
        assert!(client
            .aggregate_package("srcprj", "srcpkg", "dstprj", "srcpkg", &options, &dry_run,)
            .is_ok());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: aggregate package srcprj/srcpkg into dstprj/srcpkg (no sources, 1 repository mapping)".to_string()]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn aggregate_package_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/srcprj/srcpkg/_meta",
            "<package name=\"srcpkg\" project=\"srcprj\"><title>Source</title><description>Source package</description></package>",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .aggregate_package(
                "srcprj",
                "srcpkg",
                "dstprj",
                "srcpkg",
                &AggregateOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn aggregate_package_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .aggregate_package(
                "srcprj",
                "srcpkg",
                "dstprj",
                "dst/pkg",
                &AggregateOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        let options = AggregateOptions {
            repository_map: vec![("bad/name".to_string(), "distro".to_string())],
            ..AggregateOptions::default()
        };
        assert!(client
            .aggregate_package(
                "srcprj",
                "srcpkg",
                "dstprj",
                "dstpkg",
                &options,
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn release_package_posts_with_all_options_and_succeeds() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=release&repository=standard&arch=x86_64&target_project=test%3Arelease&target_repository=distro-1-ga&setrelease=1&nodelay=1",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = ReleaseOptions {
            repository: Some("standard".to_string()),
            architecture: Some("x86_64".to_string()),
            target_project: Some("test:release".to_string()),
            target_repository: Some("distro-1-ga".to_string()),
            set_release: Some("1".to_string()),
            no_delay: true,
        };
        assert!(client
            .release_package("test:factory", "test-pkgA", &options, &DryRun::new(false))
            .is_ok());
        let posts = http.post_calls();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].1, "");
        assert!(posts[0].2);
    }

    #[test]
    fn release_package_invoked_is_acceptance() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=release",
            "<status code=\"invoked\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .release_package(
                "test:factory",
                "test-pkgA",
                &ReleaseOptions::default(),
                &DryRun::new(false),
            )
            .is_ok());
    }

    #[test]
    fn release_package_other_body_is_error_not_success() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=release",
            "<revision rev=\"2\" vrev=\"1\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .release_package(
                "test:factory",
                "test-pkgA",
                &ReleaseOptions::default(),
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn release_package_target_project_needs_repository_and_target_repository() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let no_repository = ReleaseOptions {
            target_project: Some("test:release".to_string()),
            target_repository: Some("distro-1-ga".to_string()),
            ..ReleaseOptions::default()
        };
        assert!(client
            .release_package(
                "test:factory",
                "test-pkgA",
                &no_repository,
                &DryRun::new(false),
            )
            .is_err());
        let no_target_repository = ReleaseOptions {
            repository: Some("standard".to_string()),
            target_project: Some("test:release".to_string()),
            ..ReleaseOptions::default()
        };
        assert!(client
            .release_package(
                "test:factory",
                "test-pkgA",
                &no_target_repository,
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn release_package_target_repository_needs_a_target_project() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = ReleaseOptions {
            repository: Some("standard".to_string()),
            target_repository: Some("distro-1-ga".to_string()),
            ..ReleaseOptions::default()
        };
        assert!(client
            .release_package("test:factory", "test-pkgA", &options, &DryRun::new(false))
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn release_package_dry_run_records_plan_and_releases_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        let options = ReleaseOptions {
            repository: Some("standard".to_string()),
            target_project: Some("test:release".to_string()),
            target_repository: Some("distro-1-ga".to_string()),
            no_delay: true,
            ..ReleaseOptions::default()
        };
        assert!(client
            .release_package("test:factory", "test-pkgA", &options, &dry_run)
            .is_ok());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: release package test:factory/test-pkgA (repository standard, to test:release/distro-1-ga, no delay)".to_string()]
        );
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn release_package_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory/test-pkgA?cmd=release",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .release_package(
                "test:factory",
                "test-pkgA",
                &ReleaseOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn release_package_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .release_package(
                "test:factory",
                "test-pkgA/pkg",
                &ReleaseOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn release_project_posts_the_command_and_succeeds() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory?cmd=release&repository=standard&target_project=test%3Arelease&target_repository=distro-1-ga",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = ReleaseOptions {
            repository: Some("standard".to_string()),
            target_project: Some("test:release".to_string()),
            target_repository: Some("distro-1-ga".to_string()),
            ..ReleaseOptions::default()
        };
        assert!(client
            .release_project("test:factory", &options, &DryRun::new(false))
            .is_ok());
        let posts = http.post_calls();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].1, "");
        assert!(posts[0].2);
    }

    #[test]
    fn release_project_invoked_is_acceptance() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory?cmd=release",
            "<status code=\"invoked\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .release_project(
                "test:factory",
                &ReleaseOptions::default(),
                &DryRun::new(false),
            )
            .is_ok());
    }

    #[test]
    fn release_project_dry_run_records_plan_and_releases_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert!(client
            .release_project("test:factory", &ReleaseOptions::default(), &dry_run)
            .is_ok());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: release project test:factory".to_string()]
        );
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn release_project_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/source/test:factory?cmd=release",
            "<status code=\"ok\"/>",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .release_project(
                "test:factory",
                &ReleaseOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn release_project_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .release_project(
                "test:factory/pkg",
                &ReleaseOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn create_package_with_options_creates_with_scmsync() {
        let project_meta = "<project name=\"home:Admin\"><title>Home</title><description>Home project</description></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:Admin/_meta",
                project_meta,
            )
            .with_get_error(
                "https://api.opensuse.org/source/home:Admin/test-GitPkgA/_meta",
                ErrorKind::NotFound,
            )
            .with_put(
                "https://api.opensuse.org/source/home:Admin/test-GitPkgA/_meta",
                "<status code=\"ok\"/>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = CreatePackageOptions {
            scmsync: Some("https://mygitea1.com#main".to_string()),
            ..CreatePackageOptions::default()
        };
        assert!(client
            .create_package_with_options(
                "home:Admin",
                "test-GitPkgA",
                "",
                "",
                &options,
                &DryRun::new(false),
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].1,
            "<package name=\"test-GitPkgA\" project=\"home:Admin\"><title></title><description></description><scmsync>https://mygitea1.com#main</scmsync></package>"
        );
    }

    #[test]
    fn create_package_with_options_creates_without_scmsync() {
        let project_meta = "<project name=\"home:Admin\"><title>Home</title><description>Home project</description></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:Admin/_meta",
                project_meta,
            )
            .with_get_error(
                "https://api.opensuse.org/source/home:Admin/test-pkgEmpty/_meta",
                ErrorKind::NotFound,
            )
            .with_put(
                "https://api.opensuse.org/source/home:Admin/test-pkgEmpty/_meta",
                "<status code=\"ok\"/>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .create_package_with_options(
                "home:Admin",
                "test-pkgEmpty",
                "",
                "",
                &CreatePackageOptions::default(),
                &DryRun::new(false),
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].1,
            "<package name=\"test-pkgEmpty\" project=\"home:Admin\"><title></title><description></description></package>"
        );
    }

    #[test]
    fn create_package_with_options_refuses_an_existing_package() {
        let project_meta = "<project name=\"home:Admin\"><title>Home</title><description>Home project</description></project>";
        let package_meta = "<package name=\"test-GitPkgA\" project=\"home:Admin\"><title></title><description></description><scmsync>https://mygitea1.com#main</scmsync></package>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:Admin/_meta",
                project_meta,
            )
            .with_get(
                "https://api.opensuse.org/source/home:Admin/test-GitPkgA/_meta",
                package_meta,
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = CreatePackageOptions {
            scmsync: Some("https://mygitea2.com#main".to_string()),
            ..CreatePackageOptions::default()
        };
        let err = client
            .create_package_with_options(
                "home:Admin",
                "test-GitPkgA",
                "",
                "",
                &options,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        // The refusal happens before anything is written.
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn create_package_with_options_force_overwrites_an_existing_package() {
        let project_meta = "<project name=\"home:Admin\"><title>Home</title><description>Home project</description></project>";
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/source/home:Admin/_meta",
                project_meta,
            )
            .with_put(
                "https://api.opensuse.org/source/home:Admin/test-GitPkgA/_meta",
                "<status code=\"ok\"/>",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = CreatePackageOptions {
            scmsync: Some("https://mygitea2.com#main".to_string()),
            force: true,
        };
        assert!(client
            .create_package_with_options(
                "home:Admin",
                "test-GitPkgA",
                "",
                "",
                &options,
                &DryRun::new(false),
            )
            .is_ok());
        // Force skips the existence check: the only GET is the
        // project meta.
        assert_eq!(http.get_calls().len(), 1);
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].1,
            "<package name=\"test-GitPkgA\" project=\"home:Admin\"><title></title><description></description><scmsync>https://mygitea2.com#main</scmsync></package>"
        );
    }

    #[test]
    fn create_package_with_options_refuses_a_git_managed_project() {
        let project_meta = "<project name=\"home:Admin\"><title>Home</title><description>Home project</description><scmsync>https://example.com#main</scmsync></project>";
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:Admin/_meta",
            project_meta,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let options = CreatePackageOptions {
            scmsync: Some("https://mygitea1.com#main".to_string()),
            force: true,
        };
        let err = client
            .create_package_with_options(
                "home:Admin",
                "test-GitPkgA",
                "",
                "",
                &options,
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        // The refusal happens before the package is even looked up.
        assert_eq!(http.get_calls().len(), 1);
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn create_package_with_options_missing_project_propagates_not_found() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/source/home:Admin1/_meta",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        for options in [
            CreatePackageOptions::default(),
            CreatePackageOptions {
                force: true,
                ..CreatePackageOptions::default()
            },
        ] {
            let err = client
                .create_package_with_options(
                    "home:Admin1",
                    "test-pkgEmpty",
                    "",
                    "",
                    &options,
                    &DryRun::new(false),
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::NotFound);
        }
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn create_package_with_options_scmsync_without_fragment_is_refused() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        for scmsync in ["https://mygitea1.com", "https://mygitea1.com#"] {
            let options = CreatePackageOptions {
                scmsync: Some(scmsync.to_string()),
                ..CreatePackageOptions::default()
            };
            let err = client
                .create_package_with_options(
                    "home:Admin",
                    "test-GitPkgB",
                    "",
                    "",
                    &options,
                    &DryRun::new(false),
                )
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
        }
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn create_package_with_options_dry_run_records_plan_and_creates_nothing() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        let options = CreatePackageOptions {
            scmsync: Some("https://mygitea1.com#main".to_string()),
            force: true,
        };
        assert!(client
            .create_package_with_options("home:Admin", "test-GitPkgA", "", "", &options, &dry_run,)
            .is_ok());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: create package home:Admin/test-GitPkgA (scmsync https://mygitea1.com#main, force)".to_string()]
        );
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn create_package_with_options_dry_run_defaults_record_plain_plan() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry_run = DryRun::new(true);
        assert!(client
            .create_package_with_options(
                "home:Admin",
                "test-pkgEmpty",
                "",
                "",
                &CreatePackageOptions::default(),
                &dry_run,
            )
            .is_ok());
        assert_eq!(
            dry_run.plan(),
            vec!["[dry-run] would: create package home:Admin/test-pkgEmpty".to_string()]
        );
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn create_package_with_options_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/home:Admin/_meta",
            "<project name=\"home:Admin\"><title>Home</title><description>Home project</description></project>",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .create_package_with_options(
                "home:Admin",
                "test-pkgEmpty",
                "",
                "",
                &CreatePackageOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.get_calls().is_empty());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn create_package_with_options_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .create_package_with_options(
                "home:Admin",
                "",
                "",
                "",
                &CreatePackageOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(client
            .create_package_with_options(
                "home:Admin/pkg",
                "test-pkgEmpty",
                "",
                "",
                &CreatePackageOptions::default(),
                &DryRun::new(false),
            )
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn create_project_puts_meta_and_returns_ok() {
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/_meta",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .create_project(
                "home:alice",
                "Alice's branch",
                "Live test branch",
                &DryRun::new(false)
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].0,
            "https://api.opensuse.org/source/home:alice/_meta"
        );
        assert_eq!(
            puts[0].1,
            "<project name=\"home:alice\"><title>Alice&apos;s branch</title><description>Live test branch</description></project>"
        );
        // The creation PUT carried the credentials.
        assert!(puts[0].2);
    }

    #[test]
    fn create_project_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/_meta",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .create_project(
                "home:alice",
                "Alice's branch",
                "Live test branch",
                &DryRun::new(false)
            )
            .is_err());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn create_project_rejects_invalid_name_without_http() {
        // An invalid name is rejected before any request: no canned
        // response exists, so reaching the HTTP client would error
        // differently (FakeHttp: "no canned response").
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .create_project(
                "home/alice",
                "Alice's branch",
                "Live test branch",
                &DryRun::new(false)
            )
            .is_err());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn create_project_non_ok_body_is_error_not_success() {
        // The server answered 2xx but not ok: a failed creation must
        // never read as success.
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/_meta",
            "<status code=\"not_found\"><summary>no such project</summary></status>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .create_project(
                "home:alice",
                "Alice's branch",
                "Live test branch",
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn create_project_escapes_meta_text() {
        // Title and description go into the `_meta` XML, so markup in
        // them must be escaped, not interpolated raw.
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/_meta",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .create_project(
                "home:alice",
                "A <branch> & friends",
                "x\"y",
                &DryRun::new(false)
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert!(
            puts[0]
                .1
                .contains("<title>A &lt;branch&gt; &amp; friends</title>"),
            "title must be escaped in the _meta XML; got {}",
            puts[0].1
        );
        assert!(
            puts[0].1.contains("<description>x&quot;y</description>"),
            "description must be escaped in the _meta XML; got {}",
            puts[0].1
        );
    }

    #[test]
    fn create_project_dry_run_records_plan_and_creates_nothing() {
        // A dry-run caller invoking create directly must not mutate
        // the server: the creation is recorded in the plan and no PUT
        // runs (no canned response exists, so a PUT would fail). No
        // credentials are needed either: the guard runs before the
        // credentials check, like delete_package — so the client here
        // deliberately has none.
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let dry = DryRun::default();
        assert!(client
            .create_project("home:alice", "Alice's branch", "Live test branch", &dry)
            .is_ok());
        assert!(http.put_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: create project home:alice"]
        );
    }

    #[test]
    fn create_package_puts_meta_and_returns_ok() {
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/foo/_meta",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .create_package(
                "home:alice",
                "foo",
                "Foo",
                "Live test package",
                &DryRun::new(false)
            )
            .is_ok());
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].0,
            "https://api.opensuse.org/source/home:alice/foo/_meta"
        );
        assert_eq!(
            puts[0].1,
            "<package name=\"foo\" project=\"home:alice\"><title>Foo</title><description>Live test package</description></package>"
        );
        assert!(puts[0].2);
    }

    #[test]
    fn create_package_retries_not_found_then_succeeds() {
        // The server's API replies to the project creation before
        // its backend has the project registered, so the immediately
        // following package PUT can 404 on the just-created project
        // (live run 44876734097). The library retries a
        // not-found; the second PUT lands.
        let http = FakeHttp::new()
            .with_put_error_sequence(
                "https://api.opensuse.org/source/home:alice/foo/_meta",
                &[ErrorKind::NotFound],
            )
            .with_put(
                "https://api.opensuse.org/source/home:alice/foo/_meta",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .create_package(
                "home:alice",
                "foo",
                "Foo",
                "Live test package",
                &DryRun::new(false)
            )
            .is_ok());
        assert_eq!(http.put_calls().len(), 2);
    }

    #[test]
    fn create_package_persistent_not_found_surfaces_after_the_budget() {
        // A genuinely missing project still fails loudly once the
        // budget runs out: with a 400 ms budget the helper tries
        // twice (the first failure, one 500 ms sleep, the second
        // failure past the deadline) and returns the not-found
        // instead of hanging. The production budget is 30 s, passed
        // by create_package; the helper takes the budget so this
        // test does not have to wait it out.
        let http = FakeHttp::new().with_put_error_sequence(
            "https://api.opensuse.org/source/home:alice/foo/_meta",
            &[
                ErrorKind::NotFound,
                ErrorKind::NotFound,
                ErrorKind::NotFound,
            ],
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .put_with_registration_retry(
                "https://api.opensuse.org/source/home:alice/foo/_meta",
                "<package name=\"foo\" project=\"home:alice\" />",
                std::time::Duration::from_millis(400),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert_eq!(http.put_calls().len(), 2);
    }

    #[test]
    fn create_package_does_not_retry_other_errors() {
        // Only a not-found retries: a server error fails fast on
        // the first attempt instead of burning the budget.
        let http = FakeHttp::new()
            .with_put_error_sequence(
                "https://api.opensuse.org/source/home:alice/foo/_meta",
                &[ErrorKind::ServerError],
            )
            .with_put(
                "https://api.opensuse.org/source/home:alice/foo/_meta",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .create_package(
                "home:alice",
                "foo",
                "Foo",
                "Live test package",
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert_eq!(http.put_calls().len(), 1);
    }

    #[test]
    fn create_package_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/foo/_meta",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .create_package(
                "home:alice",
                "foo",
                "Foo",
                "Live test package",
                &DryRun::new(false)
            )
            .is_err());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn create_package_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .create_package(
                "home/alice",
                "foo",
                "Foo",
                "Live test package",
                &DryRun::new(false)
            )
            .is_err());
        assert!(client
            .create_package(
                "home:alice",
                "foo/bar",
                "Foo",
                "Live test package",
                &DryRun::new(false)
            )
            .is_err());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn create_package_dry_run_records_plan_and_creates_nothing() {
        // As create_project: the creation is recorded in the plan, no
        // PUT runs, and no credentials are needed.
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let dry = DryRun::default();
        assert!(client
            .create_package("home:alice", "foo", "Foo", "Live test package", &dry)
            .is_ok());
        assert!(http.put_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: create package home:alice/foo"]
        );
    }

    #[test]
    fn upload_file_puts_file_and_returns_revision() {
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/foo/foo.spec",
            "<revision rev=\"1\" vrev=\"1\" srcmd5=\"abc\" version=\"1\" time=\"123\" user=\"Admin\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .upload_file(
                    "home:alice",
                    "foo",
                    "foo.spec",
                    "Name: foo\n",
                    &DryRun::new(false)
                )
                .unwrap(),
            Some(1)
        );
        let puts = http.put_calls();
        assert_eq!(puts.len(), 1);
        assert_eq!(
            puts[0].0,
            "https://api.opensuse.org/source/home:alice/foo/foo.spec"
        );
        assert_eq!(puts[0].1, "Name: foo\n");
        // The upload PUT carried the credentials.
        assert!(puts[0].2);
    }

    #[test]
    fn upload_file_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/foo/foo.spec",
            "<revision rev=\"1\" />",
        );
        let client = ObsClient::new(&http);
        assert!(client
            .upload_file(
                "home:alice",
                "foo",
                "foo.spec",
                "Name: foo\n",
                &DryRun::new(false)
            )
            .is_err());
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn upload_file_rejects_invalid_names_and_filenames_without_http() {
        // An invalid name or filename is rejected before any request:
        // no canned responses exist, so reaching the HTTP client would
        // error differently (FakeHttp: "no canned response").
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let live = DryRun::new(false);
        assert!(client
            .upload_file("home/alice", "foo", "foo.spec", "x", &live)
            .is_err());
        assert!(client
            .upload_file("home:alice", "foo/bar", "foo.spec", "x", &live)
            .is_err());
        // The server's filename rule (^[^./][^/]+$): a slash anywhere,
        // a leading dot, and a one-character name are all invalid.
        for bad in ["foo/bar.spec", ".hidden", "x", "", "_meta", "_attribute"] {
            assert!(
                client
                    .upload_file("home:alice", "foo", bad, "x", &live)
                    .is_err(),
                "filename {bad:?} must be rejected"
            );
        }
        assert!(http.put_calls().is_empty());
    }

    #[test]
    fn upload_file_non_revision_body_is_error_not_success() {
        // The upload response is a revision document, not a status:
        // a `<status code="ok"/>` body (the meta-PUT success shape)
        // must never read as a successful upload.
        let http = FakeHttp::new().with_put(
            "https://api.opensuse.org/source/home:alice/foo/foo.spec",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .upload_file(
                "home:alice",
                "foo",
                "foo.spec",
                "Name: foo\n",
                &DryRun::new(false),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn upload_file_dry_run_records_plan_and_uploads_nothing() {
        // As create_project: the upload is recorded in the plan, no
        // PUT runs, and no credentials are needed.
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let dry = DryRun::default();
        assert_eq!(
            client
                .upload_file("home:alice", "foo", "foo.spec", "Name: foo\n", &dry)
                .unwrap(),
            None
        );
        assert!(http.put_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: upload file home:alice/foo/foo.spec"]
        );
    }

    #[test]
    fn parse_revision_reads_the_rev_attribute() {
        // The backend's revision document, full shape.
        assert_eq!(
            parse_revision("<revision rev=\"3\" vrev=\"3\" srcmd5=\"abc\" version=\"1\" time=\"123\" user=\"Admin\" />"),
            Some(3)
        );
        // `vrev` comes first: the `rev=` inside `vrev=` is not the
        // revision (it is preceded by `v`, not whitespace).
        assert_eq!(parse_revision("<revision vrev=\"3\" rev=\"7\" />"), Some(7));
        // Not a revision document: a status-ok body (the meta-PUT
        // success shape), a revision list, a zero revision, and a
        // `<revisions>` tag all read as none.
        assert_eq!(parse_revision("<status code=\"ok\" />"), None);
        assert_eq!(
            parse_revision("<revisionlist><revision rev=\"1\" /></revisionlist>"),
            None
        );
        assert_eq!(parse_revision("<revision rev=\"0\" />"), None);
        assert_eq!(parse_revision("<revisions rev=\"1\" />"), None);
        assert_eq!(parse_revision(""), None);
    }

    #[test]
    fn delete_package_deletes_when_no_open_requests() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs
            .package_requests_search_url("home:scarabeus_iv", "foo")
            .unwrap();
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_delete(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(obs
            .delete_package("home:scarabeus_iv", "foo", false, &DryRun::new(false))
            .is_ok());
        // The pre-check search ran first, then the DELETE.
        assert_eq!(http.get_calls().len(), 1);
        let deleted = http.delete_calls();
        assert_eq!(deleted.len(), 1);
        assert_eq!(
            deleted[0].0,
            "https://api.opensuse.org/source/home:scarabeus_iv/foo"
        );
        assert!(deleted[0].1);
    }

    #[test]
    fn delete_link_deletes_the_link_file() {
        let http = FakeHttp::new().with_delete(
            "https://api.opensuse.org/source/test:factory/test-pkgA/_link?comment=removed%20_link",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(obs
            .delete_link("test:factory", "test-pkgA", &DryRun::new(false))
            .is_ok());
        let deleted = http.delete_calls();
        assert_eq!(deleted.len(), 1);
        assert_eq!(
            deleted[0].0,
            "https://api.opensuse.org/source/test:factory/test-pkgA/_link?comment=removed%20_link"
        );
        assert!(deleted[0].1);
    }

    #[test]
    fn delete_link_dry_run_records_the_delete_and_sends_nothing() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        assert!(obs.delete_link("test:factory", "test-pkgA", &dry).is_ok());
        assert!(http.delete_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: delete link of test:factory/test-pkgA".to_string()]
        );
    }

    #[test]
    fn delete_link_rejects_invalid_names_and_missing_credentials_without_http() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs
            .delete_link("bad/project", "test-pkgA", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.delete_calls().is_empty());
        use crate::auth::Credentials;
        let obs = ObsClient::with_credentials(&http, Credentials::None).unwrap();
        let err = obs
            .delete_link("test:factory", "test-pkgA", &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn delete_package_refuses_when_open_requests_exist() {
        // A live request involves the package: the delete is refused
        // and no DELETE is made, so the request is not silently
        // revoked/declined by the API.
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs
            .package_requests_search_url("home:scarabeus_iv", "foo")
            .unwrap();
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"1\"><request id=\"1398492\" /></collection>",
            )
            .with_delete(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs
            .delete_package("home:scarabeus_iv", "foo", false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        // The error names the blocking request.
        assert!(err.context.contains("1398492"));
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn delete_package_force_bypasses_precheck_and_forwards_force() {
        // Force skips the search entirely (no canned GET exists, so a
        // search would fail) and forwards force=1 to the API.
        let http = FakeHttp::new().with_delete(
            "https://api.opensuse.org/source/home:scarabeus_iv/foo?force=1",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(obs
            .delete_package("home:scarabeus_iv", "foo", true, &DryRun::new(false))
            .is_ok());
        assert!(http.get_calls().is_empty());
        assert_eq!(http.delete_calls().len(), 1);
        assert_eq!(
            http.delete_calls()[0].0,
            "https://api.opensuse.org/source/home:scarabeus_iv/foo?force=1"
        );
    }

    #[test]
    fn delete_package_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client
            .delete_package("home:scarabeus_iv", "foo", false, &DryRun::new(false))
            .is_err());
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn delete_package_rejects_invalid_names_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .delete_package("home:scarabeus_iv", "foo/bar", false, &DryRun::new(false))
            .is_err());
        assert!(client
            .delete_package("", "foo", false, &DryRun::new(false))
            .is_err());
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
    }

    #[test]
    fn delete_package_non_ok_body_is_error_not_success() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs
            .package_requests_search_url("home:scarabeus_iv", "foo")
            .unwrap();
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_delete(
                "https://api.opensuse.org/source/home:scarabeus_iv/foo",
                "<status code=\"not_found\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs
            .delete_package("home:scarabeus_iv", "foo", false, &DryRun::new(false))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn delete_package_dry_run_records_plan_and_deletes_nothing() {
        // A dry-run caller invoking delete directly must not mutate
        // the server: the delete is recorded in the plan, and neither
        // the pre-check search nor the DELETE runs (no canned
        // responses exist, so either would fail).
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::default();
        assert!(obs
            .delete_package("home:scarabeus_iv", "foo", false, &dry)
            .is_ok());
        assert!(http.get_calls().is_empty());
        assert!(http.delete_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: delete package home:scarabeus_iv/foo"]
        );
    }

    const DIRECTORY_FIXTURE: &str = "<directory name=\"foo\" rev=\"3\" vrev=\"1\" srcmd5=\"0123456789abcdef0123456789abcdef\">\
        <entry name=\"foo.spec\" md5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" size=\"1234\" mtime=\"1700000000\"/>\
        <entry name=\"foo.changes\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\" size=\"567\" mtime=\"1700000001\"/>\
        <entry name=\"_link\"/>\
        <linkinfo project=\"home:Admin\" package=\"foo\"/>\
        <devel project=\"devel:languages:python\" package=\"foo\"/>\
        </directory>";

    #[test]
    fn directory_listing_parses_entries_and_linkinfo() {
        let listing = parse_directory(DIRECTORY_FIXTURE).unwrap();
        assert_eq!(
            listing.entries,
            vec![
                SourceFile {
                    name: "foo.spec".to_string(),
                    size: Some(1234),
                    mtime: Some(1_700_000_000),
                    md5: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
                },
                SourceFile {
                    name: "foo.changes".to_string(),
                    size: Some(567),
                    mtime: Some(1_700_000_001),
                    md5: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
                },
                SourceFile {
                    name: "_link".to_string(),
                    size: None,
                    mtime: None,
                    md5: None,
                },
            ]
        );
        assert_eq!(
            listing.linkinfo,
            Some(("home:Admin".to_string(), "foo".to_string()))
        );
        // The fixture carries a srcmd5 on the opening tag: the
        // listing pins the revision its entries belong to.
        assert_eq!(
            listing.srcmd5.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        // A listing without one reads as None, not an error.
        let listing = parse_directory("<directory name=\"foo\"></directory>").unwrap();
        assert_eq!(listing.srcmd5, None);
        // A self-closing listing carries the srcmd5 too: it is read
        // off the opening tag before the self-closing early return.
        let listing = parse_directory(
            "<directory name=\"foo\" srcmd5=\"0123456789abcdef0123456789abcdef\"/>",
        )
        .unwrap();
        assert_eq!(
            listing.srcmd5.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert!(listing.entries.is_empty());
    }

    #[test]
    fn directory_listing_rejects_malformed_bodies() {
        // Not a directory document at all.
        assert!(parse_directory("").is_err());
        assert!(parse_directory("<collection matches=\"0\"></collection>").is_err());
        assert!(parse_directory("<directorylisting></directorylisting>").is_err());
        // Unterminated, or a missing close tag.
        assert!(parse_directory("<directory name=\"foo\">").is_err());
        assert!(parse_directory("<directory name=\"foo\"><entry name=\"x\"/>").is_err());
        // Trailing garbage after the closing tag.
        assert!(parse_directory("<directory></directory> junk").is_err());
        // An entry without a name, or with a non-numeric size/mtime.
        assert!(parse_directory("<directory><entry md5=\"abc\"/></directory>").is_err());
        assert!(
            parse_directory("<directory><entry name=\"x\" size=\"big\"/></directory>").is_err()
        );
        assert!(
            parse_directory("<directory><entry name=\"x\" mtime=\"soon\"/></directory>").is_err()
        );
        // An unterminated entry tag.
        assert!(parse_directory("<directory><entry name=\"x\"</directory>").is_err());
    }

    #[test]
    fn directory_listing_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the directory
        // element; the result is the same as without one.
        let listing = parse_directory(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><directory name=\"foo\"><entry name=\"foo.spec\"/></directory>",
        )
        .unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].name, "foo.spec");
        // Bare prolog, and whitespace/newline between prolog and element.
        assert!(parse_directory("<?xml?><directory name=\"foo\"/>")
            .unwrap()
            .entries
            .is_empty());
        assert!(
            parse_directory("<?xml version=\"1.0\"?>\n<directory name=\"foo\"/>")
                .unwrap()
                .entries
                .is_empty()
        );
        // A prolog that never terminates is malformed, not an empty listing.
        assert!(parse_directory("<?xml version=\"1.0\" <directory></directory>").is_err());
        assert!(parse_directory("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(parse_directory("<?xmlfoo version=\"1.0\"?><directory></directory>").is_err());
    }

    #[test]
    fn directory_listing_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_directory("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_directory(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_directory("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn directory_listing_self_closing_is_empty() {
        let listing = parse_directory("<directory name=\"foo\"/>").unwrap();
        assert!(listing.entries.is_empty());
        assert_eq!(listing.linkinfo, None);
    }

    #[test]
    fn directory_listing_without_linkinfo_is_none() {
        let listing = parse_directory("<directory><entry name=\"foo.spec\"/></directory>").unwrap();
        assert_eq!(listing.linkinfo, None);
    }

    #[test]
    fn cat_reads_the_file_without_credentials() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo.spec",
            "Name: foo\nVersion: 1.0\n",
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .cat("openSUSE:Factory", "foo", "foo.spec", None)
                .unwrap(),
            "Name: foo\nVersion: 1.0\n"
        );
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo.spec".to_string(),
                false
            )]
        );
    }

    #[test]
    fn cat_at_revision_passes_rev() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo.spec?rev=2",
            "Name: foo\nVersion: 1.0\n",
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .cat(
                    "openSUSE:Factory",
                    "foo",
                    "foo.spec",
                    Some(RevisionSpec::Rev(2))
                )
                .unwrap(),
            "Name: foo\nVersion: 1.0\n"
        );
    }

    #[test]
    fn cat_at_srcmd5_passes_the_hash_as_rev() {
        let hash = "0123456789abcdef0123456789abcdef";
        let http = FakeHttp::new().with_get(
            &format!(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo.spec?rev={hash}"
            ),
            "Name: foo\nVersion: 1.0\n",
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .cat(
                    "openSUSE:Factory",
                    "foo",
                    "foo.spec",
                    Some(RevisionSpec::Md5(hash.to_string()))
                )
                .unwrap(),
            "Name: foo\nVersion: 1.0\n"
        );
        assert_eq!(
            http.get_calls(),
            vec![(
                format!("https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo.spec?rev={hash}"),
                false
            )]
        );
    }

    #[test]
    fn cat_rejects_bad_inputs() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client
            .cat("openSUSE:Factory", "foo/bar", "foo.spec", None)
            .is_err());
        assert!(client
            .cat("openSUSE:Factory", "foo", ".hidden", None)
            .is_err());
        assert!(client.cat("openSUSE:Factory", "foo", "a/b", None).is_err());
        assert!(client
            .cat(
                "openSUSE:Factory",
                "foo",
                "foo.spec",
                Some(RevisionSpec::Rev(0))
            )
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn cat_missing_file_is_an_error() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/nope.spec",
            ErrorKind::NotFound,
        );
        let client = ObsClient::new(&http);
        let err = client
            .cat("openSUSE:Factory", "foo", "nope.spec", None)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    /// A scratch destination for a download test: unique per test
    /// name, removed before use so the exclusive create succeeds.
    fn download_dest(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "geectl-download-source-{name}-{}.bin",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn download_source_to_file_streams_binary_bytes_to_disk() {
        // Bytes `cat` cannot carry (NULs, non-UTF-8) stream to disk
        // through the public source route, with no credentials.
        let bytes: &[u8] = &[0xED, 0xAB, 0x00, 0xEE, 0xDB, 0xFF, 0xFE];
        let http = FakeHttp::new().with_get_to_file(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo-1.0.tar.gz",
            bytes,
        );
        let client = ObsClient::new(&http);
        let path = download_dest("binary");
        let written = client
            .download_source_to_file("openSUSE:Factory", "foo", "foo-1.0.tar.gz", None, &path)
            .unwrap();
        assert_eq!(written, bytes.len() as u64);
        assert_eq!(std::fs::read(&path).unwrap(), bytes.to_vec());
        assert_eq!(
            http.get_to_file_calls(),
            vec![(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo-1.0.tar.gz"
                    .to_string(),
                false
            )]
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn download_source_to_file_streams_a_body_over_the_cap_to_disk() {
        // A body over the transport's 10 MiB cap streams to disk
        // instead of failing: the point of the streamed download.
        let bytes = vec![b'a'; crate::http::MAX_BODY_BYTES + 1];
        let http = FakeHttp::new().with_get_to_file(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo-1.0.tar.gz",
            &bytes,
        );
        let client = ObsClient::new(&http);
        let path = download_dest("over-cap");
        let written = client
            .download_source_to_file("openSUSE:Factory", "foo", "foo-1.0.tar.gz", None, &path)
            .unwrap();
        assert_eq!(written, bytes.len() as u64);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), bytes.len() as u64);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn download_source_to_file_at_revision_passes_rev_number_and_md5() {
        // Both rev forms go into `?rev=` verbatim: a number as
        // itself, an md5 lowercase (an uppercase md5 normalizes).
        let url_rev =
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo-1.0.tar.gz?rev=2";
        let url_md5 = "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo-1.0.tar.gz?rev=aabbccddaabbccddaabbccddaabbccdd";
        let http = FakeHttp::new()
            .with_get_to_file(url_rev, &[1])
            .with_get_to_file(url_md5, &[2]);
        let client = ObsClient::new(&http);
        let path = download_dest("rev-number");
        let written = client
            .download_source_to_file(
                "openSUSE:Factory",
                "foo",
                "foo-1.0.tar.gz",
                Some(&RevisionSpec::Rev(2)),
                &path,
            )
            .unwrap();
        assert_eq!(written, 1);
        let _ = std::fs::remove_file(&path);
        let path = download_dest("rev-md5");
        let md5 = RevisionSpec::parse("AABBCCDDAABBCCDDAABBCCDDAABBCCDD").unwrap();
        let written = client
            .download_source_to_file(
                "openSUSE:Factory",
                "foo",
                "foo-1.0.tar.gz",
                Some(&md5),
                &path,
            )
            .unwrap();
        assert_eq!(written, 1);
        assert_eq!(
            http.get_to_file_calls(),
            vec![(url_rev.to_string(), false), (url_md5.to_string(), false)]
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn download_source_to_file_rejects_bad_inputs() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let path = download_dest("bad-inputs");
        assert!(client
            .download_source_to_file("openSUSE:Factory", "foo/bar", "foo-1.0.tar.gz", None, &path)
            .is_err());
        assert!(client
            .download_source_to_file("openSUSE:Factory", "foo", ".hidden", None, &path)
            .is_err());
        assert!(client
            .download_source_to_file("openSUSE:Factory", "foo", "a/b", None, &path)
            .is_err());
        assert!(client
            .download_source_to_file(
                "openSUSE:Factory",
                "foo",
                "foo-1.0.tar.gz",
                Some(&RevisionSpec::Rev(0)),
                &path
            )
            .is_err());
        assert!(http.get_to_file_calls().is_empty());
    }

    #[test]
    fn download_source_to_file_missing_file_is_an_error() {
        let http = FakeHttp::new().with_get_to_file_error_sequence(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/nope.tar.gz",
            &[ErrorKind::NotFound],
        );
        let client = ObsClient::new(&http);
        let path = download_dest("missing");
        let err = client
            .download_source_to_file("openSUSE:Factory", "foo", "nope.tar.gz", None, &path)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(!path.exists());
    }

    #[test]
    fn download_source_to_file_refuses_a_pre_existing_destination() {
        // The transport creates exclusively: a destination that
        // already exists is an error, never a truncation.
        let http = FakeHttp::new().with_get_to_file(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo-1.0.tar.gz",
            &[1, 2, 3],
        );
        let client = ObsClient::new(&http);
        let path = download_dest("pre-existing");
        std::fs::write(&path, [9, 9]).unwrap();
        let err = client
            .download_source_to_file("openSUSE:Factory", "foo", "foo-1.0.tar.gz", None, &path)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&path).unwrap(), vec![9, 9]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn list_files_reads_the_listing_without_credentials() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            DIRECTORY_FIXTURE,
        );
        let client = ObsClient::new(&http);
        let files = client.list_files("openSUSE:Factory", "foo", None).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].name, "foo.spec");
        assert_eq!(files[2].name, "_link");
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo".to_string(),
                false
            )]
        );
    }

    #[test]
    fn list_files_at_revision_passes_rev() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo?rev=2",
            DIRECTORY_FIXTURE,
        );
        let client = ObsClient::new(&http);
        let files = client
            .list_files("openSUSE:Factory", "foo", Some(&RevisionSpec::Rev(2)))
            .unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].name, "foo.spec");
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo?rev=2".to_string(),
                false
            )]
        );
    }

    #[test]
    fn list_files_at_srcmd5_passes_the_hash_as_rev() {
        let hash = "0123456789abcdef0123456789abcdef";
        let http = FakeHttp::new().with_get(
            &format!("https://api.opensuse.org/public/source/openSUSE:Factory/foo?rev={hash}"),
            DIRECTORY_FIXTURE,
        );
        let client = ObsClient::new(&http);
        let files = client
            .list_files(
                "openSUSE:Factory",
                "foo",
                Some(&RevisionSpec::Md5(hash.to_string())),
            )
            .unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(
            http.get_calls(),
            vec![(
                format!("https://api.opensuse.org/public/source/openSUSE:Factory/foo?rev={hash}"),
                false
            )]
        );
    }

    #[test]
    fn list_files_rejects_rev_zero() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client
            .list_files("openSUSE:Factory", "foo", Some(&RevisionSpec::Rev(0)))
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn list_packages_reads_entry_names() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory",
            "<directory><entry name=\"foo\"/><entry name=\"multibuild-pkg:flavor1\"/><entry name=\"multibuild-pkg:flavor2\"/></directory>",
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.list_packages("openSUSE:Factory").unwrap(),
            vec![
                "foo".to_string(),
                "multibuild-pkg:flavor1".to_string(),
                "multibuild-pkg:flavor2".to_string(),
            ]
        );
    }

    #[test]
    fn list_projects_returns_sorted_names() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source",
            "<directory><entry name=\"home:Bob\"/><entry name=\"openSUSE:Factory\"/><entry name=\"devel:languages:python\"/></directory>",
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.list_projects().unwrap(),
            vec![
                "devel:languages:python".to_string(),
                "home:Bob".to_string(),
                "openSUSE:Factory".to_string(),
            ]
        );
    }

    #[test]
    fn list_projects_uses_credentials() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source",
            "<directory><entry name=\"openSUSE:Factory\"/></directory>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client.list_projects().unwrap(),
            vec!["openSUSE:Factory".to_string()]
        );
        assert_eq!(
            http.get_calls(),
            vec![("https://api.opensuse.org/source".to_string(), true)]
        );
    }

    #[test]
    fn list_projects_malformed_body_is_error_not_empty() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source",
            "<collection></collection>",
        );
        let client = ObsClient::new(&http);
        let err = client.list_projects().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn list_calls_reject_bad_names() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client
            .list_files("openSUSE:Factory", "foo/bar", None)
            .is_err());
        assert!(client.list_packages("openSUSE:Factory/x").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn unified_diff_of_identical_contents_is_empty() {
        assert_eq!(unified_diff("a/x", "one\ntwo\n", "b/x", "one\ntwo\n"), "");
        assert_eq!(unified_diff("a/x", "", "b/x", ""), "");
    }

    #[test]
    fn unified_diff_shows_a_single_change_with_context() {
        let diff = unified_diff(
            "a/x",
            "one\ntwo\nthree\nfour\nfive\n",
            "b/x",
            "one\nTWO\nthree\nfour\nfive\n",
        );
        assert_eq!(
            diff,
            "--- a/x\n+++ b/x\n@@ -1,5 +1,5 @@\n one\n-two\n+TWO\n three\n four\n five\n"
        );
    }

    #[test]
    fn unified_diff_of_a_new_file_is_all_additions() {
        let diff = unified_diff("/dev/null", "", "b/new.txt", "hello\nworld\n");
        assert_eq!(
            diff,
            "--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+hello\n+world\n"
        );
    }

    #[test]
    fn unified_diff_of_a_deleted_file_is_all_deletions() {
        let diff = unified_diff("a/old.txt", "hello\nworld\n", "/dev/null", "");
        assert_eq!(
            diff,
            "--- a/old.txt\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-hello\n-world\n"
        );
    }

    #[test]
    fn unified_diff_splits_distant_changes_into_hunks() {
        // Two changes ten lines apart: two hunks, three context lines
        // each, and no line from the middle in either hunk.
        let old: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        let mut new = old.clone();
        new = new.replacen("line 2\n", "LINE 2\n", 1);
        new = new.replacen("line 18\n", "LINE 18\n", 1);
        let diff = unified_diff("a/x", &old, "b/x", &new);
        assert!(diff.contains("@@ -1,5 +1,5 @@\n"));
        assert!(diff.contains("@@ -15,6 +15,6 @@\n"));
        assert!(!diff.contains("line 10\n"));
        assert_eq!(diff.matches("@@").count(), 4);
    }

    const HISTORY_FIXTURE: &str = "<revisionlist>\
        <revision rev=\"1\" vrev=\"1\">\
        <srcmd5>aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa</srcmd5>\
        <version>1.0</version><time>1700000000</time><user>Admin</user>\
        <comment>Initial import</comment>\
        </revision>\
        <revision rev=\"2\" vrev=\"1\">\
        <srcmd5>bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb</srcmd5>\
        <version>1.1</version><time>1700000001</time><user>Admin</user>\
        <comment>Update to 1.1</comment>\
        </revision>\
        <revision rev=\"3\" vrev=\"1\">\
        <srcmd5>cccccccccccccccccccccccccccccccc</srcmd5>\
        <version>1.2</version><time>1700000002</time><user>Admin</user>\
        <comment>Update to 1.2</comment><requestid>123</requestid>\
        </revision>\
        </revisionlist>";

    #[test]
    fn history_parses_in_server_order() {
        let entries = parse_history(HISTORY_FIXTURE).unwrap();
        assert_eq!(entries.len(), 3);
        // Server order is oldest first: the parser preserves it.
        assert_eq!(entries[0].rev, 1);
        assert_eq!(entries[2].rev, 3);
        assert_eq!(entries[2].vrev, 1);
        assert_eq!(
            entries[2].srcmd5.as_deref(),
            Some("cccccccccccccccccccccccccccccccc")
        );
        assert_eq!(entries[2].version.as_deref(), Some("1.2"));
        assert_eq!(entries[2].time, Some(1_700_000_002));
        assert_eq!(entries[2].user.as_deref(), Some("Admin"));
        assert_eq!(entries[2].comment.as_deref(), Some("Update to 1.2"));
        assert_eq!(entries[2].request_id.as_deref(), Some("123"));
        // The second entry carries no request id.
        assert_eq!(entries[1].rev, 2);
        assert_eq!(entries[1].request_id, None);
    }

    #[test]
    fn history_rejects_malformed_bodies() {
        assert!(parse_history("").is_err());
        assert!(parse_history("<collection matches=\"0\"></collection>").is_err());
        assert!(parse_history("<revisionlists></revisionlists>").is_err());
        assert!(parse_history("<revisionlist>").is_err());
        assert!(
            parse_history("<revisionlist><revision vrev=\"1\"></revision></revisionlist>").is_err()
        );
        assert!(parse_history(
            "<revisionlist><revision rev=\"x\" vrev=\"1\"></revision></revisionlist>"
        )
        .is_err());
        assert!(
            parse_history("<revisionlist><revision rev=\"1\" vrev=\"1\"><time>soon</time></revision></revisionlist>").is_err()
        );
        assert!(
            parse_history("<revisionlist><revision rev=\"1\" vrev=\"1\"></revisionlist>").is_err()
        );
        assert!(parse_history("<revisionlist/>").unwrap().is_empty());
    }

    #[test]
    fn parse_history_accepts_an_xml_prolog() {
        // The server may emit an XML prolog before the <revisionlist>
        // element; the result is the same as without one.
        let plain = "<revisionlist/>";
        let prolog = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>{plain}");
        assert_eq!(
            parse_history(&prolog).unwrap(),
            parse_history(plain).unwrap()
        );
        // Bare prolog, and whitespace/newline between prolog and element.
        assert_eq!(
            parse_history(&format!("<?xml?>{plain}")).unwrap(),
            parse_history(plain).unwrap()
        );
        assert_eq!(
            parse_history(&format!("<?xml version=\"1.0\"?>\n{plain}")).unwrap(),
            parse_history(plain).unwrap()
        );
        // A prolog that never terminates is malformed, not a missing <revisionlist>.
        assert!(parse_history("<?xml version=\"1.0\" <revisionlist></revisionlist>").is_err());
        assert!(parse_history("<?xml version=\"1.0\"").is_err());
        // `<?xmlfoo` is not a prolog at all: the guard rejects it as
        // malformed rather than skipping a prefix that is not one.
        assert!(parse_history("<?xmlfoo version=\"1.0\"?><revisionlist></revisionlist>").is_err());
    }

    #[test]
    fn parse_history_malformed_error_carries_response_preview() {
        // The failure stays fail-closed, but the error names what was
        // received, so the caller can diagnose it.
        let err = parse_history("garbage").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("garbage"), "{}", err.context);
        // A long body is capped: the preview carries the head, not the tail.
        let long = format!("<collection>{}</collection>", "x".repeat(3000));
        let err = parse_history(&long).unwrap_err();
        assert!(err.context.contains("... [truncated,"), "{}", err.context);
        assert!(!err.context.contains(&"x".repeat(3000)), "{}", err.context);
        // An empty body still fails closed, and says so.
        let err = parse_history("").unwrap_err();
        assert!(err.context.contains("<empty response>"), "{}", err.context);
    }

    #[test]
    fn package_log_returns_newest_first() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/openSUSE:Factory/foo/_history",
            HISTORY_FIXTURE,
        );
        let client = ObsClient::new(&http);
        let entries = client.package_log("openSUSE:Factory", "foo", None).unwrap();
        assert_eq!(
            entries.iter().map(|entry| entry.rev).collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
        // No credentials on this client: the read is unauthenticated.
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/source/openSUSE:Factory/foo/_history".to_string(),
                false
            )]
        );
    }

    #[test]
    fn package_log_range_filters_preserving_order() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/openSUSE:Factory/foo/_history",
            HISTORY_FIXTURE,
        );
        let client = ObsClient::new(&http);
        // `--revision=2` returns only r2.
        let entries = client
            .package_log(
                "openSUSE:Factory",
                "foo",
                Some(&RevisionRange::parse("2").unwrap()),
            )
            .unwrap();
        assert_eq!(
            entries.iter().map(|entry| entry.rev).collect::<Vec<_>>(),
            vec![2]
        );
        // `--revision=1:2` returns r2 then r1.
        let entries = client
            .package_log(
                "openSUSE:Factory",
                "foo",
                Some(&RevisionRange::parse("1:2").unwrap()),
            )
            .unwrap();
        assert_eq!(
            entries.iter().map(|entry| entry.rev).collect::<Vec<_>>(),
            vec![2, 1]
        );
        // An md5 range resolves through the history.
        let entries = client
            .package_log(
                "openSUSE:Factory",
                "foo",
                Some(
                    &RevisionRange::parse(
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    )
                    .unwrap(),
                ),
            )
            .unwrap();
        assert_eq!(
            entries.iter().map(|entry| entry.rev).collect::<Vec<_>>(),
            vec![2, 1]
        );
    }

    #[test]
    fn package_log_range_errors_are_fail_closed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/source/openSUSE:Factory/foo/_history",
            HISTORY_FIXTURE,
        );
        let client = ObsClient::new(&http);
        // An unknown md5 is an error, never an empty list.
        assert!(client
            .package_log(
                "openSUSE:Factory",
                "foo",
                Some(&RevisionRange::parse("dddddddddddddddddddddddddddddddd").unwrap()),
            )
            .is_err());
        // A reversed range is an error.
        assert!(client
            .package_log(
                "openSUSE:Factory",
                "foo",
                Some(&RevisionRange::parse("2:1").unwrap()),
            )
            .is_err());
    }

    fn patch_client() -> FakeHttp {
        let rev1 = "<directory><entry name=\"foo.spec\" md5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"/><entry name=\"old.txt\" md5=\"oooooooooooooooooooooooooooooooo\"/></directory>";
        let rev2 = "<directory><entry name=\"foo.spec\" md5=\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"/><entry name=\"new.txt\" md5=\"nnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnn\"/></directory>";
        FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo?rev=1",
                rev1,
            )
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo?rev=2",
                rev2,
            )
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo.spec?rev=1",
                "Name: foo\nVersion: 1.0\n",
            )
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/foo.spec?rev=2",
                "Name: foo\nVersion: 1.1\n",
            )
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/new.txt?rev=2",
                "fresh\n",
            )
            .with_get(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo/old.txt?rev=1",
                "stale\n",
            )
    }

    #[test]
    fn revision_patch_rev1_is_empty() {
        // Revision 1 of an existing package is an empty patch — but
        // the package is read first, so the call still reaches the
        // server.
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo?rev=1",
            "<directory><entry name=\"foo.spec\" md5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"/></directory>",
        );
        let client = ObsClient::new(&http);
        let patch = client.revision_patch("openSUSE:Factory", "foo", 1).unwrap();
        assert_eq!(patch.rev, 1);
        assert!(patch.files.is_empty());
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo?rev=1".to_string(),
                false
            )]
        );
    }

    #[test]
    fn revision_patch_rev1_missing_package_is_error() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo?rev=1",
            ErrorKind::NotFound,
        );
        let client = ObsClient::new(&http);
        let err = client
            .revision_patch("openSUSE:Factory", "foo", 1)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn revision_patch_classifies_added_modified_deleted() {
        let http = patch_client();
        let client = ObsClient::new(&http);
        let patch = client.revision_patch("openSUSE:Factory", "foo", 2).unwrap();
        assert_eq!(patch.rev, 2);
        assert_eq!(
            patch
                .files
                .iter()
                .map(|file| (file.filename.as_str(), file.status))
                .collect::<Vec<_>>(),
            vec![
                ("foo.spec", FilePatchStatus::Modified),
                ("new.txt", FilePatchStatus::Added),
                ("old.txt", FilePatchStatus::Deleted),
            ]
        );
        let modified = &patch.files[0];
        assert!(modified.diff.contains("--- a/foo.spec\n+++ b/foo.spec\n"));
        assert!(modified.diff.contains("-Version: 1.0\n+Version: 1.1\n"));
        // A new file renders with `/dev/null` on the old side.
        assert!(patch.files[1]
            .diff
            .contains("--- /dev/null\n+++ b/new.txt\n"));
        assert!(patch.files[1].diff.contains("+fresh\n"));
        assert!(patch.files[2]
            .diff
            .contains("--- a/old.txt\n+++ /dev/null\n"));
        assert!(patch.files[2].diff.contains("-stale\n"));
    }

    #[test]
    fn revision_patch_rejects_rev_zero() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.revision_patch("openSUSE:Factory", "foo", 0).is_err());
        assert!(http.get_calls().is_empty());
    }

    const PROJECT_META_FIXTURE: &str = "<project name=\"openSUSE:Factory\">\
        <title>Factory</title>\
        <description>The Factory project</description>\
        <url>https://example.com</url>\
        <scmsync>https://src.opensuse.org/pool/factory</scmsync>\
        <devel project=\"openSUSE:Factory:Devel\"/>\
        <person userid=\"Admin\" role=\"maintainer\"/>\
        <group groupid=\"factory-maintainers\" role=\"maintainer\"/>\
        <repository name=\"standard\">\
        <arch>x86_64</arch><arch>aarch64</arch>\
        <path project=\"openSUSE:Factory:ARM\" repository=\"standard\"/>\
        <path project=\"openSUSE:Factory\" repository=\"snapshot\"/>\
        </repository>\
        <repository name=\"images\"><arch>x86_64</arch></repository>\
        </project>";

    const PACKAGE_META_FIXTURE: &str = "<package name=\"foo\" project=\"openSUSE:Factory\">\
        <title>Foo package</title>\
        <description>A test package &amp; more</description>\
        <url>https://example.com/foo</url>\
        <scmsync>https://src.opensuse.org/pool/foo</scmsync>\
        <releasename>foo</releasename>\
        <devel project=\"devel:languages:python\" package=\"foo\"/>\
        <person userid=\"Admin\" role=\"maintainer\"/>\
        <person userid=\"scarabeus_iv\" role=\"bugowner\"/>\
        <group groupid=\"factory-maintainers\" role=\"maintainer\"/>\
        </package>";

    #[test]
    fn project_meta_parses_everything() {
        let meta = parse_project_meta(PROJECT_META_FIXTURE).unwrap();
        assert_eq!(meta.name, "openSUSE:Factory");
        assert_eq!(meta.title.as_deref(), Some("Factory"));
        assert_eq!(meta.description.as_deref(), Some("The Factory project"));
        assert_eq!(meta.url.as_deref(), Some("https://example.com"));
        assert_eq!(
            meta.scmsync.as_deref(),
            Some("https://src.opensuse.org/pool/factory")
        );
        assert_eq!(meta.devel.as_deref(), Some("openSUSE:Factory:Devel"));
        assert_eq!(
            meta.persons,
            vec![NamedRole {
                name: "Admin".to_string(),
                role: "maintainer".to_string(),
            }]
        );
        assert_eq!(
            meta.groups,
            vec![NamedRole {
                name: "factory-maintainers".to_string(),
                role: "maintainer".to_string(),
            }]
        );
        assert_eq!(
            meta.repositories,
            vec![
                Repository {
                    name: "standard".to_string(),
                    arch: vec!["x86_64".to_string(), "aarch64".to_string()],
                    paths: vec![
                        ("openSUSE:Factory:ARM".to_string(), "standard".to_string()),
                        ("openSUSE:Factory".to_string(), "snapshot".to_string()),
                    ],
                },
                Repository {
                    name: "images".to_string(),
                    arch: vec!["x86_64".to_string()],
                    paths: vec![],
                },
            ]
        );
    }

    #[test]
    fn package_meta_parses_everything() {
        let meta = parse_package_meta(PACKAGE_META_FIXTURE).unwrap();
        assert_eq!(meta.name, "foo");
        assert_eq!(meta.project, "openSUSE:Factory");
        assert_eq!(meta.title.as_deref(), Some("Foo package"));
        assert_eq!(meta.description.as_deref(), Some("A test package & more"));
        assert_eq!(meta.url.as_deref(), Some("https://example.com/foo"));
        assert_eq!(
            meta.scmsync.as_deref(),
            Some("https://src.opensuse.org/pool/foo")
        );
        assert_eq!(meta.releasename.as_deref(), Some("foo"));
        assert_eq!(
            meta.devel,
            Some(("devel:languages:python".to_string(), "foo".to_string()))
        );
        assert_eq!(
            meta.persons,
            vec![
                NamedRole {
                    name: "Admin".to_string(),
                    role: "maintainer".to_string(),
                },
                NamedRole {
                    name: "scarabeus_iv".to_string(),
                    role: "bugowner".to_string(),
                },
            ]
        );
        assert_eq!(
            meta.groups,
            vec![NamedRole {
                name: "factory-maintainers".to_string(),
                role: "maintainer".to_string(),
            }]
        );
    }

    #[test]
    fn meta_parses_minimal_documents() {
        let meta = parse_project_meta("<project name=\"home:Admin\"/>").unwrap();
        assert_eq!(meta.name, "home:Admin");
        assert_eq!(meta.title, None);
        assert!(meta.repositories.is_empty());
        let meta = parse_package_meta(
            "<package name=\"foo\" project=\"home:Admin\"><title>Foo</title></package>",
        )
        .unwrap();
        assert_eq!(meta.name, "foo");
        assert_eq!(meta.project, "home:Admin");
        assert_eq!(meta.title.as_deref(), Some("Foo"));
        assert_eq!(meta.devel, None);
    }

    #[test]
    fn meta_rejects_malformed_bodies() {
        assert!(parse_project_meta("").is_err());
        assert!(parse_project_meta("<package name=\"foo\" project=\"p\"/>").is_err());
        assert!(parse_project_meta("<projects name=\"p\"></projects>").is_err());
        assert!(parse_project_meta("<project></project>").is_err());
        assert!(parse_project_meta("<project name=\"p\">").is_err());
        assert!(parse_package_meta("").is_err());
        assert!(parse_package_meta("<project name=\"p\"/>").is_err());
        assert!(parse_package_meta("<package name=\"foo\"></package>").is_err());
        assert!(parse_package_meta("<package name=\"foo\" project=\"p\">").is_err());
    }

    #[test]
    fn project_meta_reads_without_credentials() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/_meta",
            PROJECT_META_FIXTURE,
        );
        let client = ObsClient::new(&http);
        let meta = client.project_meta("openSUSE:Factory").unwrap();
        assert_eq!(meta.name, "openSUSE:Factory");
        assert_eq!(meta.repositories.len(), 2);
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/public/source/openSUSE:Factory/_meta".to_string(),
                false
            )]
        );
    }

    #[test]
    fn package_meta_reads_without_credentials() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo/_meta",
            PACKAGE_META_FIXTURE,
        );
        let client = ObsClient::new(&http);
        let meta = client.package_meta("openSUSE:Factory", "foo").unwrap();
        assert_eq!(meta.name, "foo");
        assert_eq!(meta.project, "openSUSE:Factory");
        assert_eq!(meta.persons.len(), 2);
    }

    #[test]
    fn meta_reads_reject_bad_names() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.project_meta("openSUSE:Factory/x").is_err());
        assert!(client.package_meta("openSUSE:Factory", "foo/bar").is_err());
        assert!(http.get_calls().is_empty());
    }

    const PACKAGE_SEARCH_FIXTURE: &str = "<collection matches=\"2\">\
        <package name=\"foo\" project=\"openSUSE:Factory\">\
        <title>Foo package</title><description>A test package</description>\
        </package>\
        <package name=\"foobar\" project=\"devel:languages:python\"/>\
        </collection>";

    const PROJECT_SEARCH_FIXTURE: &str = "<collection matches=\"1\">\
        <project name=\"devel:languages:python\">\
        <title>Python</title><description>Python packages</description>\
        </project>\
        </collection>";

    #[test]
    fn package_search_parses_matches() {
        let matches = parse_package_search(PACKAGE_SEARCH_FIXTURE).unwrap();
        assert_eq!(
            matches,
            vec![
                PackageMatch {
                    name: "foo".to_string(),
                    project: "openSUSE:Factory".to_string(),
                    title: Some("Foo package".to_string()),
                    description: Some("A test package".to_string()),
                },
                PackageMatch {
                    name: "foobar".to_string(),
                    project: "devel:languages:python".to_string(),
                    title: None,
                    description: None,
                },
            ]
        );
    }

    #[test]
    fn project_search_parses_matches() {
        let matches = parse_project_search(PROJECT_SEARCH_FIXTURE).unwrap();
        assert_eq!(
            matches,
            vec![ProjectMatch {
                name: "devel:languages:python".to_string(),
                title: Some("Python".to_string()),
                description: Some("Python packages".to_string()),
            }]
        );
    }

    #[test]
    fn search_parsers_are_fail_closed() {
        // Not a collection, or no matches count: None.
        assert_eq!(parse_package_search(""), None);
        assert_eq!(parse_package_search("<directory></directory>"), None);
        assert_eq!(
            parse_package_search("<collection><package name=\"x\" project=\"y\"/></collection>"),
            None
        );
        assert_eq!(parse_project_search(""), None);
        // An empty collection (with its count) reads as an empty list.
        assert_eq!(
            parse_package_search("<collection matches=\"0\"></collection>"),
            Some(vec![])
        );
        assert_eq!(
            parse_project_search("<collection matches=\"0\"></collection>"),
            Some(vec![])
        );
        // The count says there are matches but none parsed: None.
        assert_eq!(
            parse_package_search("<collection matches=\"2\"></collection>"),
            None
        );
        assert_eq!(
            parse_project_search("<collection matches=\"1\"></collection>"),
            None
        );
        // An element without its name is skipped, not an error.
        assert_eq!(
            parse_package_search(
                "<collection matches=\"2\"><package project=\"y\"/><package name=\"x\" project=\"y\"/></collection>"
            ),
            Some(vec![PackageMatch {
                name: "x".to_string(),
                project: "y".to_string(),
                title: None,
                description: None,
            }])
        );
    }

    #[test]
    fn search_packages_sends_the_substring_match_authed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/package?match=contains%28%40name%2C%27foo%27%29%20or%20contains%28title%2C%27foo%27%29%20or%20contains%28description%2C%27foo%27%29",
            PACKAGE_SEARCH_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let matches = client.search_packages("foo").unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].name, "foo");
        // The search carries credentials.
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/search/package?match=contains%28%40name%2C%27foo%27%29%20or%20contains%28title%2C%27foo%27%29%20or%20contains%28description%2C%27foo%27%29"
                    .to_string(),
                true
            )]
        );
    }

    #[test]
    fn search_projects_sends_the_substring_match_authed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/project?match=contains%28%40name%2C%27python%27%29%20or%20contains%28title%2C%27python%27%29%20or%20contains%28description%2C%27python%27%29",
            PROJECT_SEARCH_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let matches = client.search_projects("python").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "devel:languages:python");
    }

    #[test]
    fn search_projects_by_person_sends_the_person_match_authed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/project?match=person%2F%40userid%3D%27scarabeus_iv%27",
            PROJECT_SEARCH_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let matches = client.search_projects_by_person("scarabeus_iv").unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "devel:languages:python");
    }

    #[test]
    fn search_projects_by_person_rejects_a_quoted_userid_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.search_projects_by_person("").is_err());
        assert!(client.search_projects_by_person("x' or '1'='1").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn search_projects_by_person_requires_credentials() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.search_projects_by_person("scarabeus_iv").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn count_scmsync_projects_returns_the_matches_count() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/project/id?match=starts-with%28scmsync%2C%27https%3A%2F%2Fgithub.com%2FopenSUSE%2F%27%29",
            "<collection matches=\"3\"/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .count_scmsync_projects("https://github.com/openSUSE/")
                .unwrap(),
            3
        );
    }

    #[test]
    fn count_scmsync_projects_rejects_a_quoted_prefix_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.count_scmsync_projects("").is_err());
        assert!(client.count_scmsync_projects("x' or '1'='1").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn count_scmsync_projects_requires_credentials() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client
            .count_scmsync_projects("https://github.com/openSUSE/")
            .is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn count_scmsync_projects_response_without_a_count_is_an_error() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/project/id?match=starts-with%28scmsync%2C%27https%3A%2F%2Fgithub.com%2FopenSUSE%2F%27%29",
            "<collection/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .count_scmsync_projects("https://github.com/openSUSE/")
            .is_err());
    }

    #[test]
    fn search_requires_credentials() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.search_packages("foo").is_err());
        assert!(client.search_projects("foo").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn search_rejects_bad_terms() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.search_packages("").is_err());
        assert!(client.search_packages("foo' or '1'='1").is_err());
        assert!(client.search_projects("").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn search_unexpected_response_is_an_error() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/package?match=contains%28%40name%2C%27foo%27%29%20or%20contains%28title%2C%27foo%27%29%20or%20contains%28description%2C%27foo%27%29",
            "<collection><package name=\"x\" project=\"y\"/></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.search_packages("foo").is_err());
    }

    const OWNER_SEARCH_FIXTURE: &str = "<collection matches=\"2\">\
        <owner rootproject=\"openSUSE:Factory\" project=\"devel:languages:python\" package=\"foo\">\
        <person name=\"Admin\" role=\"maintainer\"/>\
        <person name=\"scarabeus_iv\" role=\"bugowner\"/>\
        <group name=\"factory-maintainers\" role=\"maintainer\"/>\
        </owner>\
        <owner rootproject=\"openSUSE:Factory\" project=\"openSUSE:Factory\">\
        <person name=\"Admin\" role=\"maintainer\"/>\
        </owner>\
        </collection>";

    const PERSON_FIXTURE: &str = "<person>\
        <login>Admin</login><email>admin@example.com</email>\
        <realname>Admin User</realname><state>confirmed</state>\
        <globalrole>Admin</globalrole>\
        </person>";

    #[test]
    fn owner_search_parses_owners() {
        let owners = parse_owners(OWNER_SEARCH_FIXTURE).unwrap();
        assert_eq!(
            owners,
            vec![
                Owner {
                    root_project: "openSUSE:Factory".to_string(),
                    project: "devel:languages:python".to_string(),
                    package: Some("foo".to_string()),
                    persons: vec![
                        NamedRole {
                            name: "Admin".to_string(),
                            role: "maintainer".to_string(),
                        },
                        NamedRole {
                            name: "scarabeus_iv".to_string(),
                            role: "bugowner".to_string(),
                        },
                    ],
                    groups: vec![NamedRole {
                        name: "factory-maintainers".to_string(),
                        role: "maintainer".to_string(),
                    }],
                },
                Owner {
                    root_project: "openSUSE:Factory".to_string(),
                    project: "openSUSE:Factory".to_string(),
                    package: None,
                    persons: vec![NamedRole {
                        name: "Admin".to_string(),
                        role: "maintainer".to_string(),
                    }],
                    groups: vec![],
                },
            ]
        );
    }

    #[test]
    fn owner_search_parsers_are_fail_closed() {
        assert_eq!(parse_owners(""), None);
        assert_eq!(parse_owners("<directory></directory>"), None);
        // A `matches` count that does not parse is unexpected.
        assert_eq!(
            parse_owners("<collection matches=\"many\"></collection>"),
            None
        );
        // An empty collection reads as an empty list.
        assert_eq!(
            parse_owners("<collection matches=\"0\"></collection>"),
            Some(vec![])
        );
        // An owner without its project is skipped, not an error.
        assert_eq!(
            parse_owners("<collection><owner rootproject=\"p\"/></collection>"),
            Some(vec![])
        );
    }

    #[test]
    fn person_parses_everything() {
        let person = parse_person(PERSON_FIXTURE).unwrap();
        assert_eq!(
            person,
            Person {
                login: "Admin".to_string(),
                email: "admin@example.com".to_string(),
                realname: Some("Admin User".to_string()),
                state: "confirmed".to_string(),
            }
        );
    }

    #[test]
    fn person_without_realname_is_none() {
        let person = parse_person(
            "<person><login>Admin</login><email>a@b.c</email><state>confirmed</state></person>",
        )
        .unwrap();
        assert_eq!(person.realname, None);
    }

    #[test]
    fn person_rejects_malformed_bodies() {
        assert_eq!(parse_person(""), None);
        assert_eq!(parse_person("<collection></collection>"), None);
        assert_eq!(
            parse_person("<person><email>a@b.c</email><state>confirmed</state></person>"),
            None
        );
        assert_eq!(
            parse_person("<person><login>Admin</login><state>confirmed</state></person>"),
            None
        );
        assert_eq!(
            parse_person("<person><login>Admin</login><email>a@b.c</email></person>"),
            None
        );
        assert_eq!(
            parse_person(
                "<person><login>Admin</login><email>a@b.c</email><state>confirmed</state>"
            ),
            None
        );
    }

    #[test]
    fn login_validation_matches_the_server_format() {
        assert!(validate_login("Admin").is_ok());
        assert!(validate_login("ab").is_ok());
        assert!(validate_login("a b").is_ok());
        assert!(validate_login("a$b^c-d.e#f*g+h&i'j\"k").is_ok());
        assert!(validate_login("a").is_err());
        assert!(validate_login("").is_err());
        assert!(validate_login("a/b").is_err());
        assert!(validate_login("a?b").is_err());
        assert!(validate_login("a:b").is_err());
        assert!(validate_login(&"a".repeat(101)).is_err());
        assert!(validate_login(&"a".repeat(100)).is_ok());
    }

    #[test]
    fn owners_searches_by_project_and_package_authed() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/owner?project=openSUSE%3AFactory&package=foo",
            OWNER_SEARCH_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let owners = client.owners("openSUSE:Factory", "foo").unwrap();
        assert_eq!(owners.len(), 2);
        assert_eq!(owners[0].package.as_deref(), Some("foo"));
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/search/owner?project=openSUSE%3AFactory&package=foo"
                    .to_string(),
                true
            )]
        );
    }

    #[test]
    fn owners_for_binary_searches_by_binary() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/owner?binary=python3.11",
            OWNER_SEARCH_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let owners = client.owners_for_binary("python3.11").unwrap();
        assert_eq!(owners.len(), 2);
    }

    #[test]
    fn maintainers_filters_to_the_maintainer_role() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/owner?project=openSUSE%3AFactory&package=foo",
            OWNER_SEARCH_FIXTURE,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client.maintainers("openSUSE:Factory", "foo").unwrap(),
            vec![
                NamedRole {
                    name: "Admin".to_string(),
                    role: "maintainer".to_string(),
                },
                NamedRole {
                    name: "factory-maintainers".to_string(),
                    role: "maintainer".to_string(),
                },
                NamedRole {
                    name: "Admin".to_string(),
                    role: "maintainer".to_string(),
                },
            ]
        );
    }

    #[test]
    fn whois_reads_the_person_authed() {
        let http =
            FakeHttp::new().with_get("https://api.opensuse.org/person/Admin", PERSON_FIXTURE);
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let person = client.whois("Admin").unwrap();
        assert_eq!(person.login, "Admin");
        assert_eq!(person.state, "confirmed");
        assert_eq!(
            http.get_calls(),
            vec![("https://api.opensuse.org/person/Admin".to_string(), true)]
        );
    }

    #[test]
    fn owner_and_whois_reads_require_credentials() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.owners("openSUSE:Factory", "foo").is_err());
        assert!(client.owners_for_binary("python3.11").is_err());
        assert!(client.maintainers("openSUSE:Factory", "foo").is_err());
        assert!(client.whois("Admin").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn owner_and_whois_reads_reject_bad_inputs() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.owners("openSUSE:Factory", "foo/bar").is_err());
        assert!(client.owners_for_binary("python3.11/x").is_err());
        assert!(client.whois("a").is_err());
        assert!(client.whois("a/b").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn link_target_reads_linkinfo_without_credentials() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            DIRECTORY_FIXTURE,
        );
        let client = ObsClient::new(&http);
        assert_eq!(
            client.link_target("openSUSE:Factory", "foo").unwrap(),
            Some(("home:Admin".to_string(), "foo".to_string()))
        );
        assert_eq!(
            http.get_calls(),
            vec![(
                "https://api.opensuse.org/public/source/openSUSE:Factory/foo".to_string(),
                false
            )]
        );
    }

    #[test]
    fn link_target_without_linkinfo_is_none() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            "<directory><entry name=\"foo.spec\"/></directory>",
        );
        let client = ObsClient::new(&http);
        assert_eq!(client.link_target("openSUSE:Factory", "foo").unwrap(), None);
    }

    #[test]
    fn link_target_missing_package_is_an_error() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/public/source/openSUSE:Factory/foo",
            ErrorKind::NotFound,
        );
        let client = ObsClient::new(&http);
        let err = client.link_target("openSUSE:Factory", "foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn link_target_rejects_bad_names() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.link_target("openSUSE:Factory", "foo/bar").is_err());
        assert!(http.get_calls().is_empty());
    }

    /// The request-search URL `open_requests_for_package` builds,
    /// for the FakeHttp fixture keys. The URL builder itself is
    /// tested separately; here it only supplies the fixture key.
    fn requests_url(project: &str, package: &str) -> String {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        client
            .package_requests_search_url(project, package)
            .unwrap()
    }

    const OLD_PACKAGES_FIXTURE: &str = "<directory>\
        <entry name=\"common-one\"/><entry name=\"common-two\"/>\
        <entry name=\"common-three\"/><entry name=\"old-only\"/>\
        </directory>";

    const NEW_PACKAGES_FIXTURE: &str = "<directory>\
        <entry name=\"common-one\"/><entry name=\"common-two\"/>\
        <entry name=\"common-three\"/><entry name=\"new-only\"/>\
        </directory>";

    /// A FakeHttp with the two project listings and the file
    /// listings for the shared packages: `common-one` and
    /// `common-three` identical on both sides, `common-two` with a
    /// changed `foo.spec`.
    fn diff_http() -> FakeHttp {
        FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/old",
                OLD_PACKAGES_FIXTURE,
            )
            .with_get(
                "https://api.opensuse.org/public/source/new",
                NEW_PACKAGES_FIXTURE,
            )
            .with_get(
                "https://api.opensuse.org/public/source/old/common-one",
                "<directory><entry name=\"a.spec\" md5=\"111\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/new/common-one",
                "<directory><entry name=\"a.spec\" md5=\"111\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/old/common-two",
                "<directory><entry name=\"foo.spec\" md5=\"aaa\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/new/common-two",
                "<directory><entry name=\"foo.spec\" md5=\"bbb\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/old/common-three",
                "<directory><entry name=\"c.spec\" md5=\"222\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/new/common-three",
                "<directory><entry name=\"c.spec\" md5=\"222\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/old/common-two/foo.spec",
                "Version: 1.0\n",
            )
            .with_get(
                "https://api.opensuse.org/public/source/new/common-two/foo.spec",
                "Version: 1.1\n",
            )
    }

    #[test]
    fn project_diff_classifies_packages() {
        let http = diff_http();
        let client = ObsClient::new(&http);
        let diffs = client.project_diff("old", "new", &[], &[], false).unwrap();
        assert_eq!(
            diffs
                .iter()
                .map(|d| (d.name.as_str(), d.status))
                .collect::<Vec<_>>(),
            vec![
                ("common-one", PackageDiffStatus::Identical),
                ("common-three", PackageDiffStatus::Identical),
                ("common-two", PackageDiffStatus::Differs),
                ("new-only", PackageDiffStatus::OnlyInNew),
                ("old-only", PackageDiffStatus::OnlyInOld),
            ]
        );
        assert_eq!(diffs[0].files, Vec::<FilePatch>::new());
        assert_eq!(diffs[0].requests, Vec::<String>::new());
        let changed = &diffs[2];
        assert_eq!(
            changed.files,
            vec![FilePatch {
                filename: "foo.spec".to_string(),
                status: FilePatchStatus::Modified,
                diff: "--- a/foo.spec\n+++ b/foo.spec\n@@ -1 +1 @@\n-Version: 1.0\n+Version: 1.1\n"
                    .to_string(),
            }]
        );
    }

    #[test]
    fn project_diff_include_and_exclude_narrow_the_diff() {
        let http = diff_http();
        let client = ObsClient::new(&http);
        let diffs = client
            .project_diff("old", "new", &["common-two"], &[], false)
            .unwrap();
        assert_eq!(
            diffs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            vec!["common-two"]
        );
        let http = diff_http();
        let client = ObsClient::new(&http);
        let diffs = client
            .project_diff("old", "new", &[], &["common-two"], false)
            .unwrap();
        assert_eq!(
            diffs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            vec!["common-one", "common-three", "new-only", "old-only"]
        );
        let http = diff_http();
        let client = ObsClient::new(&http);
        let diffs = client
            .project_diff("old", "new", &["common-one"], &["common-one"], false)
            .unwrap();
        assert_eq!(diffs, Vec::<PackageDiff>::new());
    }

    #[test]
    fn project_diff_annotations_carry_open_request_ids() {
        let http = diff_http().with_get(
            &requests_url("new", "common-two"),
            "<collection matches=\"1\"><request id=\"42\"/></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let diffs = client.project_diff("old", "new", &[], &[], true).unwrap();
        let changed = diffs
            .iter()
            .find(|d| d.name == "common-two")
            .expect("common-two differs");
        assert_eq!(changed.requests, vec!["42".to_string()]);
        let identical = diffs
            .iter()
            .find(|d| d.name == "common-one")
            .expect("common-one identical");
        assert_eq!(identical.requests, Vec::<String>::new());
    }

    #[test]
    fn project_diff_annotation_requires_credentials() {
        let http = diff_http();
        let client = ObsClient::new(&http);
        let err = client
            .project_diff("old", "new", &[], &[], true)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
    }

    #[test]
    fn project_diff_rejects_bad_names_without_requesting() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client
            .project_diff("old", "new", &["bad/name"], &[], false)
            .is_err());
        assert!(client
            .project_diff("old", "new", &[], &["bad/name"], false)
            .is_err());
        assert!(client.project_diff("o/l", "new", &[], &[], false).is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn diff_packages_diffs_differently_named_packages() {
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/old/old-pkg",
                "<directory><entry name=\"foo.spec\" md5=\"aaa\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/new/new-pkg",
                "<directory><entry name=\"foo.spec\" md5=\"bbb\"/><entry name=\"extra.txt\" md5=\"eee\"/></directory>",
            )
            .with_get(
                "https://api.opensuse.org/public/source/old/old-pkg/foo.spec",
                "Version: 1.0\n",
            )
            .with_get(
                "https://api.opensuse.org/public/source/new/new-pkg/foo.spec",
                "Version: 1.1\n",
            )
            .with_get(
                "https://api.opensuse.org/public/source/new/new-pkg/extra.txt",
                "extra\n",
            );
        let client = ObsClient::new(&http);
        let patches = client
            .diff_packages("old", "old-pkg", "new", "new-pkg")
            .unwrap();
        assert_eq!(
            patches
                .iter()
                .map(|file| (file.filename.as_str(), file.status))
                .collect::<Vec<_>>(),
            vec![
                ("extra.txt", FilePatchStatus::Added),
                ("foo.spec", FilePatchStatus::Modified),
            ]
        );
        assert!(patches[1].diff.contains("-Version: 1.0\n+Version: 1.1\n"));
    }

    #[test]
    fn diff_packages_missing_package_is_error_not_partial() {
        // The old side reads; the new side's listing 404s, so the
        // diff is an error, never a partial list of the old side's
        // files.
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/public/source/old/old-pkg",
                "<directory><entry name=\"foo.spec\" md5=\"aaa\"/></directory>",
            )
            .with_get_error(
                "https://api.opensuse.org/public/source/new/new-pkg",
                ErrorKind::NotFound,
            );
        let client = ObsClient::new(&http);
        let err = client
            .diff_packages("old", "old-pkg", "new", "new-pkg")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn diff_packages_rejects_bad_names_without_requesting() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert!(client.diff_packages("o/l", "a", "new", "b").is_err());
        assert!(client.diff_packages("old", "a/b", "new", "b").is_err());
        assert!(client.diff_packages("old", "a", "n/w", "b").is_err());
        assert!(client.diff_packages("old", "a", "new", "b/c").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn search_projects_match_source_or_target() {
        let search = search_without_states().projects(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "source[@project='foo' or @project='bar'] or target[@project='foo' or @project='bar']"
        );
    }

    #[test]
    fn search_packages_match_source_or_target() {
        let search = search_without_states().packages(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "source[@package='foo' or @package='bar'] or target[@package='foo' or @package='bar']"
        );
    }

    #[test]
    fn debug_create_package_probe_reports_each_retry() {
        // One 404 on the just-created project, then the PUT lands:
        // the probe line names the retry against the 30 s budget.
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new()
            .with_put_error_sequence(
                "https://api.opensuse.org/source/home:alice/foo/_meta",
                &[ErrorKind::NotFound],
            )
            .with_put(
                "https://api.opensuse.org/source/home:alice/foo/_meta",
                "<status code=\"ok\" />",
            );
        let client = ObsClient::with_credentials(&http, basic_credentials())
            .unwrap()
            .with_debug(debug);
        assert!(client
            .create_package(
                "home:alice",
                "foo",
                "Foo",
                "Live test package",
                &DryRun::new(false)
            )
            .is_ok());
        assert_eq!(http.put_calls().len(), 2);
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            ["retry: create_package registration probe 1/30s: still not found".to_string()]
        );
    }
}
