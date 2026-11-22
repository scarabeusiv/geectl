use super::build_url;
use super::elements;
use super::meta_envelope;
use super::tag_attr;
use super::valid_obs_name;
use super::ObsClient;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use crate::xml::skip_xml_prolog;
use crate::xml::xml_unescape;

/// A staged request in a staging project: the request id, its action
/// type, creator, state, and first target package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedRequest {
    /// The request number.
    pub id: u64,
    /// The action type (`submit`, `delete`, ...).
    pub request_type: String,
    /// The login that created the request.
    pub creator: String,
    /// The request state (`new`, `review`, ...).
    pub state: String,
    /// The first target package, when the request names one.
    pub package: Option<String>,
    /// The request this one supersedes, when it supersedes one.
    pub superseded_by: Option<u64>,
}

/// A staging project in a staging workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagingProject {
    /// The staging project name (`openSUSE:Factory:Staging:A`).
    pub name: String,
    /// The overall state (`acceptable`, `testing`, ...), when the
    /// read asked for status.
    pub state: Option<String>,
    /// Staged requests, when the read asked for requests; None
    /// when it did not ask, so an absent list never reads as an
    /// empty one.
    pub staged_requests: Option<Vec<StagedRequest>>,
    /// Untracked requests, when the read asked for requests; None
    /// when it did not ask.
    pub untracked_requests: Option<Vec<StagedRequest>>,
    /// Obsolete staged requests, when the read asked for requests;
    /// None when it did not ask.
    pub obsolete_requests: Option<Vec<StagedRequest>>,
}

/// Options for a staging read: which optional sections the server
/// sends back. Both default on: an agent watching its submission
/// wants the staged requests and the overall state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StagingReadOptions {
    /// Include the staged, untracked, and obsolete request lists.
    pub requests: bool,
    /// Include the overall state of each staging project.
    pub status: bool,
}

impl Default for StagingReadOptions {
    fn default() -> Self {
        StagingReadOptions {
            requests: true,
            status: true,
        }
    }
}

/// Parse a staging-projects document: either a `<staging_projects>`
/// collection (the list read) or a single `<staging_project>` (the
/// one-project read). None when the body is neither: an unexpected
/// response is an error, never an empty list. An empty collection
/// reads as an empty list.
pub(crate) fn parse_staging_projects(body: &str) -> Option<Vec<StagingProject>> {
    let body = skip_xml_prolog(body).ok()?;
    let body = body.trim();
    if body.starts_with("<staging_projects") {
        let mut out = Vec::new();
        for (tag, inner) in elements(body, "staging_project").ok()? {
            out.push(staging_project_from(&tag, &inner)?);
        }
        Some(out)
    } else if body.starts_with("<staging_project") {
        let (tag, inner) = meta_envelope(body, "staging_project").ok()?;
        Some(vec![staging_project_from(&tag, inner)?])
    } else {
        None
    }
}

/// One staging project from its opening tag and inner XML. None when
/// the project carries no name: a staging project always has one, so
/// its absence means the server changed shape underneath us.
fn staging_project_from(open_tag: &str, inner: &str) -> Option<StagingProject> {
    let name = xml_unescape(&tag_attr(open_tag, "name")?);
    let state = tag_attr(open_tag, "state").map(|s| xml_unescape(&s));
    Some(StagingProject {
        name,
        state,
        staged_requests: parse_request_list(inner, "staged_requests").ok()?,
        untracked_requests: parse_request_list(inner, "untracked_requests").ok()?,
        obsolete_requests: parse_request_list(inner, "obsolete_requests").ok()?,
    })
}

/// The requests in one list block (`<staged_requests>`,
/// `<untracked_requests>`, `<obsolete_requests>`). None when the
/// block is absent: the server sends it only when the read asked for
/// requests, so an absent list never reads as an empty one. A request
/// element missing its id, type, creator, or state is an error, not
/// a skip: the server changed shape underneath us. A request element
/// carrying a present-but-unparsable id or `superseded_by` is an
/// error too.
fn parse_request_list(inner: &str, list: &str) -> Result<Option<Vec<StagedRequest>>, GeectlError> {
    let mut out = Vec::new();
    let mut present = false;
    for (_, block_inner) in elements(inner, list)? {
        present = true;
        for (element, _) in elements(&block_inner, "request")? {
            out.push(parse_staged_request(&element)?);
        }
    }
    Ok(if present { Some(out) } else { None })
}

fn parse_staged_request(element: &str) -> Result<StagedRequest, GeectlError> {
    let missing = |attr: &str| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected OBS staging response: request is missing {attr}"),
        )
    };
    let Some(id_raw) = tag_attr(element, "id") else {
        return Err(missing("id"));
    };
    let id = id_raw.parse::<u64>().map_err(|_| {
        GeectlError::new(
            ErrorKind::ServerError,
            format!("unexpected OBS staging response: request id {id_raw:?} is not a number"),
        )
    })?;
    let Some(request_type) = tag_attr(element, "type") else {
        return Err(missing("type"));
    };
    let Some(creator) = tag_attr(element, "creator") else {
        return Err(missing("creator"));
    };
    let Some(state) = tag_attr(element, "state") else {
        return Err(missing("state"));
    };
    let package = tag_attr(element, "package").map(|p| xml_unescape(&p));
    let superseded_by = match tag_attr(element, "superseded_by") {
        Some(raw) if !raw.is_empty() => Some(raw.parse::<u64>().map_err(|_| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS staging response: superseded_by {raw:?} is not a number"),
            )
        })?),
        _ => None,
    };
    Ok(StagedRequest {
        id,
        request_type: xml_unescape(&request_type),
        creator: xml_unescape(&creator),
        state: xml_unescape(&state),
        package,
        superseded_by,
    })
}

impl<'a> ObsClient<'a> {
    /// List the staging projects of staging workflow
    /// `workflow_project` (`GET
    /// /staging/<workflow_project>/staging_projects`): one entry per
    /// staging project, with its staged requests and overall state
    /// when the options ask for them. Requires credentials; without
    /// them the call fails loudly before any request. A workflow
    /// project with no staging workflow is a NotFound error, never an
    /// empty list. An unexpected response is an error, never an empty
    /// list.
    ///
    /// An agent lists staging projects to see where its submission
    /// sits in the Factory pipeline: which staging holds it, and
    /// whether that staging is acceptable.
    pub fn list_staging_projects(
        &self,
        workflow_project: &str,
        options: StagingReadOptions,
    ) -> Result<Vec<StagingProject>, GeectlError> {
        if !valid_obs_name(workflow_project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {workflow_project:?}"),
            ));
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "listing staging projects requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(
            &self.base,
            &["staging", workflow_project, "staging_projects"],
            &staging_query(options),
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_staging_projects(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS staging response for {workflow_project}"),
            )
        })
    }

    /// Read one staging project (`GET
    /// /staging/<workflow_project>/staging_projects/<name>`). Same
    /// options and failure terms as [`ObsClient::list_staging_projects`].
    pub fn staging_project(
        &self,
        workflow_project: &str,
        name: &str,
        options: StagingReadOptions,
    ) -> Result<StagingProject, GeectlError> {
        if !valid_obs_name(workflow_project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {workflow_project:?}"),
            ));
        }
        if !valid_obs_name(name) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name: {name:?}"),
            ));
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "reading a staging project requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(
            &self.base,
            &["staging", workflow_project, "staging_projects", name],
            &staging_query(options),
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        let mut projects = parse_staging_projects(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS staging response for {workflow_project}/{name}"),
            )
        })?;
        if projects.len() != 1 {
            return Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS staging response for {workflow_project}/{name}: not one staging project"),
            ));
        }
        Ok(projects.remove(0))
    }
}

/// The query pairs for a staging read: `requests=1` and `status=1`
/// only for the sections the options ask for.
fn staging_query(options: StagingReadOptions) -> Vec<(&'static str, &'static str)> {
    let mut query = Vec::new();
    if options.requests {
        query.push(("requests", "1"));
    }
    if options.status {
        query.push(("status", "1"));
    }
    query
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use crate::obs::test_support::{basic_credentials, FakeHttp};

    const LIST_BODY: &str = "<staging_projects><staging_project name=\"openSUSE:Factory:Staging:A\" state=\"acceptable\"><staged_requests count=\"1\"><request id=\"1398977\" type=\"submit\" creator=\"scarabeus_iv\" state=\"new\" package=\"patterns-network\"/></staged_requests><untracked_requests count=\"0\"/><obsolete_requests count=\"0\"/></staging_project><staging_project name=\"openSUSE:Factory:Staging:B\" state=\"testing\"><staged_requests count=\"1\"><request id=\"1398978\" type=\"submit\" creator=\"scarabeus_iv\" state=\"review\" package=\"patterns-cinnamon\" superseded_by=\"1398000\"/></staged_requests><untracked_requests count=\"1\"><request id=\"1399000\" type=\"submit\" creator=\"alice\" state=\"new\" package=\"shadow\"/></untracked_requests><obsolete_requests count=\"0\"/></staging_project></staging_projects>";

    #[test]
    fn parse_staging_projects_reads_names_states_and_requests() {
        let projects = parse_staging_projects(LIST_BODY).unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].name, "openSUSE:Factory:Staging:A");
        assert_eq!(projects[0].state.as_deref(), Some("acceptable"));
        let staged = projects[0].staged_requests.as_ref().unwrap();
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].id, 1398977);
        assert_eq!(staged[0].request_type, "submit");
        assert_eq!(staged[0].creator, "scarabeus_iv");
        assert_eq!(staged[0].state, "new");
        assert_eq!(staged[0].package.as_deref(), Some("patterns-network"));
        assert_eq!(staged[0].superseded_by, None);
        assert!(projects[0].untracked_requests.as_ref().unwrap().is_empty());
        assert!(projects[0].obsolete_requests.as_ref().unwrap().is_empty());
        assert_eq!(projects[1].name, "openSUSE:Factory:Staging:B");
        assert_eq!(projects[1].state.as_deref(), Some("testing"));
        let staged = projects[1].staged_requests.as_ref().unwrap();
        assert_eq!(staged[0].superseded_by, Some(1398000));
        let untracked = projects[1].untracked_requests.as_ref().unwrap();
        assert_eq!(untracked.len(), 1);
        assert_eq!(untracked[0].id, 1399000);
    }

    #[test]
    fn parse_staging_projects_empty_collection_is_an_empty_list() {
        assert_eq!(
            parse_staging_projects("<staging_projects/>").unwrap(),
            Vec::new()
        );
        assert_eq!(
            parse_staging_projects("<staging_projects></staging_projects>").unwrap(),
            Vec::new()
        );
    }

    #[test]
    fn parse_staging_projects_single_document_reads_one_project() {
        let body = "<staging_project name=\"openSUSE:Factory:Staging:A\" state=\"acceptable\"><staged_requests count=\"0\"/><untracked_requests count=\"0\"/><obsolete_requests count=\"0\"/></staging_project>";
        let projects = parse_staging_projects(body).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "openSUSE:Factory:Staging:A");
        assert_eq!(projects[0].state.as_deref(), Some("acceptable"));
        assert!(projects[0].staged_requests.as_ref().unwrap().is_empty());
    }

    #[test]
    fn parse_staging_projects_lists_absent_when_not_asked_are_none() {
        let body = "<staging_projects><staging_project name=\"openSUSE:Factory:Staging:A\" state=\"acceptable\"/></staging_projects>";
        let projects = parse_staging_projects(body).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].staged_requests, None);
        assert_eq!(projects[0].untracked_requests, None);
        assert_eq!(projects[0].obsolete_requests, None);
    }

    #[test]
    fn parse_staging_projects_malformed_request_numbers_fail_the_parse() {
        let bad_id = "<staging_projects><staging_project name=\"openSUSE:Factory:Staging:A\"><staged_requests count=\"1\"><request id=\"abc\" type=\"submit\" creator=\"alice\" state=\"new\"/></staged_requests><untracked_requests count=\"0\"/><obsolete_requests count=\"0\"/></staging_project></staging_projects>";
        assert!(parse_staging_projects(bad_id).is_none());
        let bad_superseded = "<staging_projects><staging_project name=\"openSUSE:Factory:Staging:A\"><staged_requests count=\"1\"><request id=\"7\" type=\"submit\" creator=\"alice\" state=\"new\" superseded_by=\"abc\"/></staged_requests><untracked_requests count=\"0\"/><obsolete_requests count=\"0\"/></staging_project></staging_projects>";
        assert!(parse_staging_projects(bad_superseded).is_none());
    }

    #[test]
    fn parse_staging_projects_rejects_a_foreign_document() {
        assert!(parse_staging_projects("<collection matches=\"0\"/>").is_none());
        assert!(parse_staging_projects("").is_none());
        assert!(
            parse_staging_projects("<staging_projects><staging_project/></staging_projects>")
                .is_none()
        );
    }

    #[test]
    fn parse_staging_projects_request_missing_required_attributes_fails_the_parse() {
        for missing in ["id", "type", "creator", "state"] {
            let request = match missing {
                "id" => "<request type=\"submit\" creator=\"alice\" state=\"new\"/>",
                "type" => "<request id=\"7\" creator=\"alice\" state=\"new\"/>",
                "creator" => "<request id=\"7\" type=\"submit\" state=\"new\"/>",
                "state" => "<request id=\"7\" type=\"submit\" creator=\"alice\"/>",
                _ => unreachable!("unknown attribute {missing}"),
            };
            let body = format!("<staging_projects><staging_project name=\"openSUSE:Factory:Staging:A\"><staged_requests count=\"1\">{request}</staged_requests><untracked_requests count=\"0\"/><obsolete_requests count=\"0\"/></staging_project></staging_projects>");
            assert!(
                parse_staging_projects(&body).is_none(),
                "request missing {missing} must fail the parse"
            );
        }
    }

    #[test]
    fn parse_staged_request_missing_required_attributes_names_the_attribute() {
        for missing in ["id", "type", "creator", "state"] {
            let element = match missing {
                "id" => "<request type=\"submit\" creator=\"alice\" state=\"new\"/>",
                "type" => "<request id=\"7\" creator=\"alice\" state=\"new\"/>",
                "creator" => "<request id=\"7\" type=\"submit\" state=\"new\"/>",
                "state" => "<request id=\"7\" type=\"submit\" creator=\"alice\"/>",
                _ => unreachable!("unknown attribute {missing}"),
            };
            let err = parse_staged_request(element).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::ServerError);
            assert!(
                err.to_string()
                    .contains(&format!("request is missing {missing}")),
                "{err}"
            );
        }
    }

    #[test]
    fn list_staging_projects_builds_the_url_with_both_sections() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/staging/openSUSE:Factory/staging_projects?requests=1&status=1",
            LIST_BODY,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let projects = client
            .list_staging_projects("openSUSE:Factory", StagingReadOptions::default())
            .unwrap();
        assert_eq!(projects.len(), 2);
        assert_eq!(http.get_calls().len(), 1);
        assert!(http.get_calls()[0].1);
    }

    #[test]
    fn list_staging_projects_without_sections_sends_no_query() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/staging/openSUSE:Factory/staging_projects",
            "<staging_projects/>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let projects = client
            .list_staging_projects(
                "openSUSE:Factory",
                StagingReadOptions {
                    requests: false,
                    status: false,
                },
            )
            .unwrap();
        assert!(projects.is_empty());
    }

    #[test]
    fn staging_project_builds_the_member_url() {
        let body = "<staging_project name=\"openSUSE:Factory:Staging:A\" state=\"acceptable\"><staged_requests count=\"0\"/><untracked_requests count=\"0\"/><obsolete_requests count=\"0\"/></staging_project>";
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/staging/openSUSE:Factory/staging_projects/openSUSE:Factory:Staging:A?requests=1&status=1",
            body,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let project = client
            .staging_project(
                "openSUSE:Factory",
                "openSUSE:Factory:Staging:A",
                StagingReadOptions::default(),
            )
            .unwrap();
        assert_eq!(project.name, "openSUSE:Factory:Staging:A");
    }

    #[test]
    fn list_staging_projects_missing_workflow_is_not_found_not_empty() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/staging/openSUSE:Factory/staging_projects?requests=1&status=1",
            ErrorKind::NotFound,
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .list_staging_projects("openSUSE:Factory", StagingReadOptions::default())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn list_staging_projects_rejects_invalid_names_and_missing_credentials_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .list_staging_projects("bad/project", StagingReadOptions::default())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
        use crate::auth::Credentials;
        let client = ObsClient::with_credentials(&http, Credentials::None).unwrap();
        let err = client
            .list_staging_projects("openSUSE:Factory", StagingReadOptions::default())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }
}
