use super::build_url;
use super::parse_request_id;
use super::parse_request_ids;
use super::parse_request_ids_for_action;
use super::parse_search_matches;
use super::parse_status_ok;
use super::parse_submit_request_ids;
use super::valid_obs_name;
use super::ActionShape;
use super::ActionTarget;
use super::DryRun;
use super::ObsClient;
use super::SourceUpdate;
use super::SrPayload;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use crate::xml::xml_escape;
use crate::xml::xml_unescape;
use std::fmt;

/// Validate an OBS request id: a non-empty run of ASCII digits that
/// reads as a positive number. Returns the canonical number, so
/// `old != new` is a numeric comparison (a leading-zero spelling of
/// the same id still counts as the same request). The API only checks
/// the id is a positive number (`check_supersede_state`); the shape
/// check here keeps a malformed id from ever reaching a URL.
pub(crate) fn validate_request_id(id: &str) -> Result<u64, GeectlError> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid OBS request id: {id:?}"),
        ));
    }
    let number = id.parse::<u64>().map_err(|_| {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid OBS request id: {id:?}"),
        )
    })?;
    if number == 0 {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid OBS request id: {id:?}"),
        ));
    }
    Ok(number)
}

/// A single XML element found by scanning: its tag name, the raw
/// attribute text between the tag name and the closing `>` (without a
/// trailing `/` for a self-closing tag), and its inner XML (empty for
/// a self-closing tag). Borrowed from the document being parsed.
struct Element<'a> {
    pub(crate) name: &'a str,
    attrs: &'a str,
    inner: &'a str,
}

/// Split `xml` — the inner XML of one parent element — into its
/// top-level child elements, in document order. Text between elements
/// must be whitespace only: a request document never carries bare text
/// under a parent, so anything else is a shape change and the split
/// fails. A child is found by its opening tag and its matching close
/// tag (plain search, the same discipline as
/// [`parse_submit_request_ids`]): these documents never nest an
/// element inside another of the same name, so no depth counting is
/// needed, and a missing close tag fails the whole document rather
/// than yielding a partial element list.
fn child_elements(xml: &str) -> Option<Vec<Element<'_>>> {
    let mut elements = Vec::new();
    let mut rest = xml;
    loop {
        let text_end = rest.find('<').unwrap_or(rest.len());
        if !rest[..text_end].trim().is_empty() {
            return None;
        }
        rest = &rest[text_end..];
        if rest.is_empty() {
            return Some(elements);
        }
        let after_lt = &rest[1..];
        if after_lt.starts_with('/') || after_lt.starts_with('!') || after_lt.starts_with('?') {
            return None;
        }
        let name_end = after_lt
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .unwrap_or(after_lt.len());
        let name = &after_lt[..name_end];
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
        {
            return None;
        }
        let after_name = &after_lt[name_end..];
        let tag_end = after_name.find('>')?;
        let tag = &after_name[..tag_end];
        if let Some(attrs) = tag.trim_end().strip_suffix('/') {
            elements.push(Element {
                name,
                attrs: attrs.trim_end(),
                inner: "",
            });
            rest = &after_name[tag_end + 1..];
            continue;
        }
        let inner_start = &after_name[tag_end + 1..];
        let close_tag = format!("</{name}>");
        let close = inner_start.find(&close_tag)?;
        elements.push(Element {
            name,
            attrs: tag,
            inner: &inner_start[..close],
        });
        rest = &inner_start[close + close_tag.len()..];
    }
}

/// The value of the `name` attribute in an opening tag's attribute
/// text, in either quote style, order-tolerant. The attribute name
/// must stand on its own — `subtype="..."` must not match a lookup
/// for `type` — so the character before `name=` has to be whitespace
/// or the start of the text. The value is returned raw: the attribute
/// values this parser reads (ids, logins, project and package names,
/// timestamps, state and role names) cannot carry XML entities, so
/// there is nothing to unescape in them. None when the attribute is
/// absent or unterminated.
fn attr_value<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=");
    let mut rest = attrs;
    while let Some(pos) = rest.find(&needle) {
        if pos > 0 && !rest.as_bytes()[pos - 1].is_ascii_whitespace() {
            rest = &rest[pos + needle.len()..];
            continue;
        }
        let after_eq = &rest[pos + needle.len()..];
        let quote = after_eq.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let close = after_eq[1..].find(quote)?;
        return Some(&after_eq[1..1 + close]);
    }
    None
}

/// The text of a leaf element's inner XML, unescaped. The inner XML
/// must be pure text: any `<` in it means the element carries markup
/// the model does not expect, and the leaf fails. An empty element
/// reads as the empty string.
fn leaf_text(inner: &str) -> Option<String> {
    if inner.contains('<') {
        return None;
    }
    Some(xml_unescape(inner))
}

/// The state a submit request is in. The values are exactly the
/// server's `VALID_REQUEST_STATES` (`BsRequest`): a request the
/// library reads can be in any of them, including `deleted`, which
/// only an admin can produce — the library never sends it (see
/// `ObsClient::change_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestState {
    /// Freshly filed; nobody has acted on it yet.
    New,
    /// Under review; reviewers have been asked.
    Review,
    /// Accepted; the change landed.
    Accepted,
    /// Declined; the change was refused.
    Declined,
    /// Revoked by its creator; withdrawn.
    Revoked,
    /// Superseded by a newer request.
    Superseded,
    /// Deleted by an admin.
    Deleted,
}

impl RequestState {
    /// The server's spelling of this state, as it appears in
    /// `<state name="…">` and in `newstate=` query values.
    pub fn as_str(&self) -> &'static str {
        match self {
            RequestState::New => "new",
            RequestState::Review => "review",
            RequestState::Accepted => "accepted",
            RequestState::Declined => "declined",
            RequestState::Revoked => "revoked",
            RequestState::Superseded => "superseded",
            RequestState::Deleted => "deleted",
        }
    }

    /// Parse a state name the server sent. None for anything else:
    /// an unknown state is a server shape change, not a state the
    /// library can act on.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "new" => Some(RequestState::New),
            "review" => Some(RequestState::Review),
            "accepted" => Some(RequestState::Accepted),
            "declined" => Some(RequestState::Declined),
            "revoked" => Some(RequestState::Revoked),
            "superseded" => Some(RequestState::Superseded),
            "deleted" => Some(RequestState::Deleted),
            _ => None,
        }
    }
}

/// The state of one review on a request. The values are exactly the
/// server's `VALID_REVIEW_STATES` (`Review`) — a review's own states,
/// not the request's: `obsoleted` is set when the reviewer is removed
/// and never appears on a request, and a request's `review` and
/// `revoked` never appear on a review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    /// Asked for; nobody has acted on it yet.
    New,
    /// Accepted by the reviewer.
    Accepted,
    /// Declined by the reviewer.
    Declined,
    /// Superseded by a newer review of the same target.
    Superseded,
    /// The reviewer was removed, so the review no longer applies.
    Obsoleted,
}

impl ReviewState {
    /// The server's spelling of this state, as it appears in
    /// `<review state="…">`.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReviewState::New => "new",
            ReviewState::Accepted => "accepted",
            ReviewState::Declined => "declined",
            ReviewState::Superseded => "superseded",
            ReviewState::Obsoleted => "obsoleted",
        }
    }

    /// Parse a review state name the server sent. None for anything
    /// else: an unknown state is a server shape change, not a state
    /// the library can act on.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "new" => Some(ReviewState::New),
            "accepted" => Some(ReviewState::Accepted),
            "declined" => Some(ReviewState::Declined),
            "superseded" => Some(ReviewState::Superseded),
            "obsoleted" => Some(ReviewState::Obsoleted),
            _ => None,
        }
    }
}

/// A request's priority. The values are exactly the server's
/// `VALID_REQUEST_PRIORITIES` (`BsRequest`): anything else is refused
/// by the server at creation, so a priority the parser does not
/// recognise is a shape change, not a priority. The server omits the
/// `<priority>` element for the default `moderate` on its own output;
/// the serializer renders the model as given, so a `Moderate` it was
/// told about is written out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestPriority {
    /// Below the default; handled when there is time.
    Low,
    /// The default; the server omits this value from its output.
    Moderate,
    /// Above the default; handled soon.
    Important,
    /// The highest; handled first.
    Critical,
}

impl RequestPriority {
    /// The server's spelling of this priority.
    pub fn as_str(&self) -> &'static str {
        match self {
            RequestPriority::Low => "low",
            RequestPriority::Moderate => "moderate",
            RequestPriority::Important => "important",
            RequestPriority::Critical => "critical",
        }
    }

    /// Parse a priority the server sent. None for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(RequestPriority::Low),
            "moderate" => Some(RequestPriority::Moderate),
            "important" => Some(RequestPriority::Important),
            "critical" => Some(RequestPriority::Critical),
            _ => None,
        }
    }
}

/// The `<acceptinfo>` of a submit action: the revision the request
/// was accepted at, when it was accepted. Only present on a request
/// that has been accepted; a request that is still open carries none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptInfo {
    /// The accepted source revision number.
    pub rev: String,
    /// The accepted source md5.
    pub srcmd5: String,
    /// The origin project, when the accepted revision came from
    /// elsewhere (a forwarded request).
    pub oproject: Option<String>,
    /// The origin package, when the accepted revision came from
    /// elsewhere.
    pub opackage: Option<String>,
    /// The origin source md5, when the accepted revision came from
    /// elsewhere.
    pub osrcmd5: Option<String>,
    /// The expanded source md5 at accept time.
    pub xsrcmd5: Option<String>,
    /// The origin expanded source md5, when the accepted revision
    /// came from elsewhere.
    pub oxsrcmd5: Option<String>,
}

/// One action of a submit request: what the request does. The
/// variants are the action types the server renders
/// (`BsRequestAction`): a document carrying an action type the
/// library does not know is refused at parse time, so a new server
/// action type surfaces as an error instead of silently dropping the
/// action that carries the change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestAction {
    /// Submit a package's changes from a source to a target. The
    /// target is optional: a submit action the server returns can
    /// carry no `<target>` element at all.
    Submit {
        /// The project the change comes from.
        source_project: String,
        /// The package in the source project.
        source_package: String,
        /// The source revision, when the request pins one.
        source_rev: Option<String>,
        /// The project the change goes to, when the action names one.
        target_project: Option<String>,
        /// The package in the target project, when the action names
        /// one.
        target_package: Option<String>,
        /// What happens to the source branch on accept, when the
        /// request sets it.
        sourceupdate: Option<SourceUpdate>,
        /// Whether the request updates the devel-project link on
        /// accept.
        updatelink: bool,
        /// Whether the request makes the origin older on accept.
        makeoriginolder: bool,
        /// The accept revision, when the request was accepted.
        acceptinfo: Option<AcceptInfo>,
    },
    /// Delete a project or a package.
    Delete {
        /// The project to delete.
        target_project: String,
        /// The package to delete; absent when the whole project goes.
        target_package: Option<String>,
        /// The repository to delete, when the request names one.
        target_repository: Option<String>,
    },
    /// Grant a person or a group a role on a project or a package.
    AddRole {
        /// The project the role is granted on.
        target_project: String,
        /// The package the role is granted on; absent for a
        /// project-level grant.
        target_package: Option<String>,
        /// The person and the role granted, when the action names one.
        person: Option<(String, String)>,
        /// The group and the role granted, when the action names one.
        group: Option<(String, String)>,
    },
    /// Set the bugowner of a project or a package.
    SetBugowner {
        /// The project the bugowner is set on.
        target_project: String,
        /// The package the bugowner is set on; absent for a
        /// project-level setting.
        target_package: Option<String>,
        /// The person set as bugowner, when the action names one.
        person_name: Option<String>,
        /// The group set as bugowner, when the action names one.
        group_name: Option<String>,
    },
    /// Change the devel project of a package.
    ChangeDevel {
        /// The project the package lives in.
        source_project: String,
        /// The package whose devel project changes.
        source_package: String,
        /// The project that becomes the devel project.
        target_project: String,
        /// The package in the devel project.
        target_package: String,
    },
    /// Create a maintenance incident from a source package.
    MaintenanceIncident {
        /// The project the incident comes from.
        source_project: String,
        /// The package the incident is built from.
        source_package: String,
        /// The maintenance project the incident goes to.
        target_project: String,
        /// The package in the target project, when the action names
        /// one.
        target_package: Option<String>,
    },
    /// Release a maintenance incident to the maintenance project.
    MaintenanceRelease {
        /// The project the release comes from.
        source_project: String,
        /// The package the release is built from.
        source_package: String,
        /// The project the release goes to.
        target_project: String,
        /// The package in the target project, when the action names
        /// one.
        target_package: Option<String>,
    },
    /// Release a package to another project.
    Release {
        /// The project the release comes from.
        source_project: String,
        /// The package being released.
        source_package: String,
        /// The project the release goes to.
        target_project: String,
        /// The package in the target project, when the action names
        /// one.
        target_package: Option<String>,
    },
}

impl RequestAction {
    /// The server's spelling of this action's type, as it appears in
    /// `<action type="…">`.
    pub fn type_name(&self) -> &'static str {
        match self {
            RequestAction::Submit { .. } => "submit",
            RequestAction::Delete { .. } => "delete",
            RequestAction::AddRole { .. } => "add_role",
            RequestAction::SetBugowner { .. } => "set_bugowner",
            RequestAction::ChangeDevel { .. } => "change_devel",
            RequestAction::MaintenanceIncident { .. } => "maintenance_incident",
            RequestAction::MaintenanceRelease { .. } => "maintenance_release",
            RequestAction::Release { .. } => "release",
        }
    }
}

/// The `<state>` element of a request: the state it is in now, who
/// put it there, and when. The `comment` is the message left with the
/// last state change; it is empty when the change carried none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestStateInfo {
    /// The state the request is in.
    pub name: RequestState,
    /// Who made the last state change.
    pub who: String,
    /// When the last state change happened.
    pub when: String,
    /// When the request was created. The server always sends this;
    /// older documents may not carry it.
    pub created: Option<String>,
    /// The request this one was superseded by, when it was superseded.
    pub superseded_by: Option<u64>,
    /// Who approved the request, when it carries an approval.
    pub approver: Option<String>,
    /// The message left with the last state change; empty when none.
    pub comment: String,
}

/// One `<review>` on a request: a review asked of a user, a group, a
/// project, or a package. The `comment` is the reason the review was
/// asked for, or the message left with it. The `history` is what
/// happened to the review since it was asked — accepted, declined,
/// assigned, reopened, obsoleted — one entry per change, in order;
/// a review nobody has acted on yet has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestReview {
    /// The review's state.
    pub state: ReviewState,
    /// When the review was asked for or last changed.
    pub when: Option<String>,
    /// Who asked for or last changed the review.
    pub who: Option<String>,
    /// The user the review is asked of, when it names one.
    pub by_user: Option<String>,
    /// The group the review is asked of, when it names one.
    pub by_group: Option<String>,
    /// The project the review is asked of, when it names one.
    pub by_project: Option<String>,
    /// The package the review is asked of, when it names one.
    pub by_package: Option<String>,
    /// The reason for the review; empty when none was given.
    pub comment: String,
    /// What happened to the review since it was asked, in order.
    pub history: Vec<RequestHistory>,
}

/// One `<history>` entry of a request: a state change, who made it
/// and when, and what it was. The first entry is always the creation
/// (`description` "Request created", `comment` the request's
/// description); the rest are the state changes in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHistory {
    /// Who made the change.
    pub who: String,
    /// When the change happened.
    pub when: String,
    /// What the change was; empty when the entry carries none.
    pub description: String,
    /// The message left with the change; empty when none.
    pub comment: String,
}

/// A submit request as the server renders it: its id and creator,
/// what it does, the state it is in, its reviews and history, and its
/// message. Read with `ObsClient::show_request`; a document the
/// parser does not recognise in full is refused rather than read
/// partially, so a caller never acts on a half-parsed request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The request number.
    pub id: u64,
    /// Who filed the request.
    pub creator: String,
    /// The request's title, when the document carries one. The server
    /// does not render a title on its own output; older documents may
    /// carry one.
    pub title: Option<String>,
    /// The request's description (its message); empty when none.
    pub description: String,
    /// What the request does, in order.
    pub actions: Vec<RequestAction>,
    /// The request's priority, when the document carries one. The
    /// server omits the default `moderate` from its output.
    pub priority: Option<RequestPriority>,
    /// The state the request is in now.
    pub state: RequestStateInfo,
    /// The reviews asked on the request, in order.
    pub reviews: Vec<RequestReview>,
    /// The request's history, in order, starting with its creation.
    /// Only present when the document was fetched with history (see
    /// `ObsClient::show_request`); empty otherwise.
    pub history: Vec<RequestHistory>,
    /// When the request is scheduled to be accepted, when one is set.
    pub accept_at: Option<String>,
}

impl Request {
    /// The actions of the given types, in order. An empty type list
    /// returns every action. A type name that matches no action
    /// simply returns nothing for it — filtering is the caller's
    /// choice, not a parse error.
    pub fn actions_of(&self, types: &[&str]) -> Vec<&RequestAction> {
        if types.is_empty() {
            return self.actions.iter().collect();
        }
        self.actions
            .iter()
            .filter(|action| types.contains(&action.type_name()))
            .collect()
    }

    /// Render this request as the XML the server renders: the same
    /// elements, in the server's order, with the same attribute
    /// order. Empty comments and descriptions are dropped (an empty
    /// `<comment></comment>` reads back as the empty string, so
    /// writing it out would add nothing). The round-trip is
    /// structural — `parse_request(&request.to_xml())` reads back the
    /// same request — not byte-identical to a server document, which
    /// may spell attributes in another order.
    pub fn to_xml(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "<request creator=\"{}\" id=\"{}\">\n",
            xml_escape(&self.creator),
            self.id
        ));
        for action in &self.actions {
            out.push_str(&action_to_xml(action));
        }
        if let Some(priority) = self.priority {
            out.push_str(&format!("  <priority>{}</priority>\n", priority.as_str()));
        }
        out.push_str(&state_to_xml(&self.state));
        for review in &self.reviews {
            out.push_str(&review_to_xml(review));
        }
        for entry in &self.history {
            out.push_str(&history_to_xml(entry, 2));
        }
        if let Some(accept_at) = &self.accept_at {
            out.push_str(&format!(
                "  <accept_at>{}</accept_at>\n",
                xml_escape(accept_at)
            ));
        }
        if let Some(title) = &self.title {
            out.push_str(&format!("  <title>{}</title>\n", xml_escape(title)));
        }
        if !self.description.is_empty() {
            out.push_str(&format!(
                "  <description>{}</description>\n",
                xml_escape(&self.description)
            ));
        }
        out.push_str("</request>");
        out
    }
}

/// Parse a `<request>` document into a [`Request`]. The document
/// must be a whole request: an `id` and a `creator` on the root, at
/// least one action, exactly one `<state>`. Anything the parser does
/// not recognise — an unknown action type, an unknown child element,
/// a missing required attribute, a duplicated singleton — fails the
/// whole document (None), never a partial request. An old-style
/// `<request type="submit"><submit>…` document parses to the modern
/// action form, and re-serialises as `<action>`; the `type`
/// attribute on the root is dropped on the way out.
pub fn parse_request(xml: &str) -> Option<Request> {
    let xml = xml.trim();
    let root = parse_root(xml, "request")?;
    let id = attr_value(root.attrs, "id")?;
    let id = id.parse::<u64>().ok().filter(|id| *id > 0)?;
    let creator = attr_value(root.attrs, "creator")?;
    if creator.is_empty() {
        return None;
    }
    let mut actions = Vec::new();
    let mut has_action_element = false;
    let mut has_submit_element = false;
    let mut state: Option<RequestStateInfo> = None;
    let mut reviews = Vec::new();
    let mut history = Vec::new();
    let mut title: Option<String> = None;
    let mut description = String::new();
    let mut seen_description = false;
    let mut priority: Option<RequestPriority> = None;
    let mut accept_at: Option<String> = None;
    for child in child_elements(root.inner)? {
        match child.name {
            "action" => {
                has_action_element = true;
                actions.push(parse_action(&child)?);
            }
            "submit" => {
                has_submit_element = true;
                actions.push(parse_legacy_submit(&child)?);
            }
            "state" => {
                if state.is_some() {
                    return None;
                }
                state = Some(parse_state(&child)?);
            }
            "review" => reviews.push(parse_review(&child)?),
            "history" => history.push(parse_request_history(&child)?),
            "title" => {
                if title.is_some() {
                    return None;
                }
                title = Some(leaf_text(child.inner)?);
            }
            "description" => {
                if seen_description {
                    return None;
                }
                seen_description = true;
                description = leaf_text(child.inner)?;
            }
            "priority" => {
                if priority.is_some() {
                    return None;
                }
                priority = Some(RequestPriority::parse(&leaf_text(child.inner)?)?);
            }
            "accept_at" => {
                if accept_at.is_some() {
                    return None;
                }
                accept_at = Some(leaf_text(child.inner)?);
            }
            _ => return None,
        }
    }
    // The legacy form carries its submit action as `<submit>` and no
    // `<action>` at all; a document mixing the two shapes is neither,
    // so it fails rather than guessing which half to keep.
    if has_action_element && has_submit_element {
        return None;
    }
    if actions.is_empty() {
        return None;
    }
    Some(Request {
        id,
        creator: creator.to_string(),
        title,
        description,
        actions,
        priority,
        state: state?,
        reviews,
        history,
        accept_at,
    })
}

/// The root element of a document: its attribute text and inner XML.
/// The document must start with `<name` and end with `</name>` (after
/// trimming). A self-closing root reads as an empty document of this
/// shape. A leading XML declaration (`<?xml …?>`) is skipped: the
/// server does not send one on these responses, but a document that
/// carries one is the same document.
fn parse_root<'a>(xml: &'a str, name: &'a str) -> Option<Element<'a>> {
    let xml = xml.trim();
    let xml = match xml.strip_prefix("<?xml") {
        Some(rest) => {
            // The prolog must start here: `<?xmlfoo` is not a
            // prolog, so it is not stripped — the `<` handling below
            // rejects `<?...` as not an element.
            let first = rest.chars().next();
            if !matches!(first, Some(c) if c.is_whitespace() || c == '?') {
                xml
            } else {
                let end = rest.find("?>")?;
                rest[end + 2..].trim_start()
            }
        }
        None => xml,
    };
    let after_lt = xml.strip_prefix('<')?;
    if after_lt.starts_with('/') || after_lt.starts_with('!') || after_lt.starts_with('?') {
        return None;
    }
    if !after_lt.starts_with(name) {
        return None;
    }
    let after_name = &after_lt[name.len()..];
    match after_name.chars().next() {
        Some(c) if c.is_whitespace() || c == '>' => {}
        _ => return None,
    }
    let tag_end = after_name.find('>')?;
    let tag = &after_name[..tag_end];
    if let Some(attrs) = tag.trim_end().strip_suffix('/') {
        // A self-closing root has no children: it is an empty
        // document of this shape (an empty `<comments />` reads as an
        // empty list; an empty `<request />` fails later, on its
        // missing actions and state).
        return Some(Element {
            name,
            attrs: attrs.trim_end(),
            inner: "",
        });
    }
    let inner_start = &after_name[tag_end + 1..];
    let close_tag = format!("</{name}>");
    let close = inner_start.rfind(&close_tag)?;
    let inner = &inner_start[..close];
    let after_close = &inner_start[close + close_tag.len()..];
    if !after_close.trim().is_empty() {
        return None;
    }
    Some(Element {
        name,
        attrs: tag,
        inner,
    })
}

/// Parse one `<action>` element. The `type` attribute names the
/// variant; anything else fails. Each variant takes exactly the
/// children the server renders for it — a missing required child, a
/// duplicated singleton, or an unknown child fails the action.
fn parse_action(element: &Element<'_>) -> Option<RequestAction> {
    let action_type = attr_value(element.attrs, "type")?;
    match action_type {
        "submit" => parse_submit_action(element),
        "delete" => parse_delete_action(element),
        "add_role" => parse_add_role_action(element),
        "set_bugowner" => parse_set_bugowner_action(element),
        "change_devel" => parse_change_devel_action(element),
        "maintenance_incident" | "maintenance_release" | "release" => {
            parse_source_target_action(element, action_type)
        }
        _ => None,
    }
}

/// Parse the `<source>` child of an action element: the project and
/// package are required, the revision optional. Exactly one source;
/// a missing or duplicated one fails.
fn action_source(element: &Element<'_>) -> Option<(String, String, Option<String>)> {
    let mut source: Option<(String, String, Option<String>)> = None;
    for child in child_elements(element.inner)? {
        if child.name != "source" {
            continue;
        }
        if source.is_some() {
            return None;
        }
        let project = attr_value(child.attrs, "project")?;
        let package = attr_value(child.attrs, "package")?;
        if project.is_empty() || package.is_empty() {
            return None;
        }
        source = Some((
            project.to_string(),
            package.to_string(),
            attr_value(child.attrs, "rev").map(str::to_string),
        ));
    }
    source
}

/// A target reference: the project, the package (when the action
/// names one), and the repository (when it pins one).
type TargetRef = (String, Option<String>, Option<String>);

/// Parse the `<target>` child of an action element: the project is
/// required, the package and repository optional. At most one target;
/// a duplicated one fails. None when the action carries no target.
fn action_target(element: &Element<'_>) -> Option<Option<TargetRef>> {
    let mut target: Option<TargetRef> = None;
    for child in child_elements(element.inner)? {
        if child.name != "target" {
            continue;
        }
        if target.is_some() {
            return None;
        }
        let project = attr_value(child.attrs, "project")?;
        if project.is_empty() {
            return None;
        }
        target = Some((
            project.to_string(),
            attr_value(child.attrs, "package").map(str::to_string),
            attr_value(child.attrs, "repository").map(str::to_string),
        ));
    }
    Some(target)
}

/// Parse a `<submit>` action: a source, an optional target, optional
/// options, and an optional acceptinfo. Any other child fails.
fn parse_submit_action(element: &Element<'_>) -> Option<RequestAction> {
    let mut sourceupdate: Option<SourceUpdate> = None;
    let mut updatelink = false;
    let mut seen_updatelink = false;
    let mut makeoriginolder = false;
    let mut seen_makeoriginolder = false;
    let mut acceptinfo: Option<AcceptInfo> = None;
    let mut seen_options = false;
    let mut seen_acceptinfo = false;
    for child in child_elements(element.inner)? {
        match child.name {
            "source" | "target" => {}
            "options" => {
                if seen_options {
                    return None;
                }
                seen_options = true;
                for option in child_elements(child.inner)? {
                    match option.name {
                        "sourceupdate" => {
                            if sourceupdate.is_some() {
                                return None;
                            }
                            sourceupdate = Some(parse_sourceupdate(&leaf_text(option.inner)?)?);
                        }
                        "updatelink" => {
                            if seen_updatelink {
                                return None;
                            }
                            seen_updatelink = true;
                            updatelink = parse_option_bool(&leaf_text(option.inner)?)?;
                        }
                        "makeoriginolder" => {
                            if seen_makeoriginolder {
                                return None;
                            }
                            seen_makeoriginolder = true;
                            makeoriginolder = parse_option_bool(&leaf_text(option.inner)?)?;
                        }
                        _ => return None,
                    }
                }
            }
            "acceptinfo" => {
                if seen_acceptinfo {
                    return None;
                }
                seen_acceptinfo = true;
                if !child.inner.trim().is_empty() {
                    return None;
                }
                let rev = attr_value(child.attrs, "rev")?;
                let srcmd5 = attr_value(child.attrs, "srcmd5")?;
                if rev.is_empty() || srcmd5.is_empty() {
                    return None;
                }
                acceptinfo = Some(AcceptInfo {
                    rev: rev.to_string(),
                    srcmd5: srcmd5.to_string(),
                    oproject: attr_value(child.attrs, "oproject").map(str::to_string),
                    opackage: attr_value(child.attrs, "opackage").map(str::to_string),
                    osrcmd5: attr_value(child.attrs, "osrcmd5").map(str::to_string),
                    xsrcmd5: attr_value(child.attrs, "xsrcmd5").map(str::to_string),
                    oxsrcmd5: attr_value(child.attrs, "oxsrcmd5").map(str::to_string),
                });
            }
            _ => return None,
        }
    }
    let (source_project, source_package, source_rev) = action_source(element)?;
    let (target_project, target_package) = match action_target(element)? {
        Some((project, package, _)) => (Some(project), package),
        None => (None, None),
    };
    Some(RequestAction::Submit {
        source_project,
        source_package,
        source_rev,
        target_project,
        target_package,
        sourceupdate,
        updatelink,
        makeoriginolder,
        acceptinfo,
    })
}

/// Parse a `<sourceupdate>` value: exactly the three values the
/// server accepts, as [`SourceUpdate`]. Anything else fails.
fn parse_sourceupdate(value: &str) -> Option<SourceUpdate> {
    match value {
        "cleanup" => Some(SourceUpdate::Cleanup),
        "update" => Some(SourceUpdate::Update),
        "noupdate" => Some(SourceUpdate::NoUpdate),
        _ => None,
    }
}

/// Parse an `<updatelink>` / `<makeoriginolder>` value. The server
/// writes `true`; documents produced elsewhere may write `1`. Both
/// spellings read as true, `false` and `0` as false; anything else
/// fails.
fn parse_option_bool(value: &str) -> Option<bool> {
    match value {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

/// Parse a `<delete>` action: a target naming the project, and the
/// package when a package (rather than the whole project) goes.
fn parse_delete_action(element: &Element<'_>) -> Option<RequestAction> {
    for child in child_elements(element.inner)? {
        if child.name != "target" {
            return None;
        }
    }
    let (target_project, target_package, target_repository) = action_target(element)??;
    Some(RequestAction::Delete {
        target_project,
        target_package,
        target_repository,
    })
}

/// Parse an `<add_role>` action: a target, and a person and/or a
/// group with the role each is granted. At least one of person and
/// group is required.
fn parse_add_role_action(element: &Element<'_>) -> Option<RequestAction> {
    let mut person: Option<(String, String)> = None;
    let mut group: Option<(String, String)> = None;
    for child in child_elements(element.inner)? {
        match child.name {
            "target" => {}
            "person" => {
                if person.is_some() {
                    return None;
                }
                let name = attr_value(child.attrs, "name")?;
                let role = attr_value(child.attrs, "role")?;
                if name.is_empty() || role.is_empty() {
                    return None;
                }
                person = Some((name.to_string(), role.to_string()));
            }
            "group" => {
                if group.is_some() {
                    return None;
                }
                let name = attr_value(child.attrs, "name")?;
                let role = attr_value(child.attrs, "role")?;
                if name.is_empty() || role.is_empty() {
                    return None;
                }
                group = Some((name.to_string(), role.to_string()));
            }
            _ => return None,
        }
    }
    if person.is_none() && group.is_none() {
        return None;
    }
    let (target_project, target_package, _) = action_target(element)??;
    Some(RequestAction::AddRole {
        target_project,
        target_package,
        person,
        group,
    })
}

/// Parse a `<set_bugowner>` action: a target, and a person and/or a
/// group set as bugowner. The person and group carry a name only, no
/// role. At least one of person and group is required.
fn parse_set_bugowner_action(element: &Element<'_>) -> Option<RequestAction> {
    let mut person_name: Option<String> = None;
    let mut group_name: Option<String> = None;
    for child in child_elements(element.inner)? {
        match child.name {
            "target" => {}
            "person" => {
                if person_name.is_some() {
                    return None;
                }
                let name = attr_value(child.attrs, "name")?;
                if name.is_empty() {
                    return None;
                }
                person_name = Some(name.to_string());
            }
            "group" => {
                if group_name.is_some() {
                    return None;
                }
                let name = attr_value(child.attrs, "name")?;
                if name.is_empty() {
                    return None;
                }
                group_name = Some(name.to_string());
            }
            _ => return None,
        }
    }
    if person_name.is_none() && group_name.is_none() {
        return None;
    }
    let (target_project, target_package, _) = action_target(element)??;
    Some(RequestAction::SetBugowner {
        target_project,
        target_package,
        person_name,
        group_name,
    })
}

/// Parse a `<change_devel>` action: a source and a target, both
/// naming a project and a package.
fn parse_change_devel_action(element: &Element<'_>) -> Option<RequestAction> {
    for child in child_elements(element.inner)? {
        if child.name != "source" && child.name != "target" {
            return None;
        }
    }
    let (source_project, source_package, _) = action_source(element)?;
    let (target_project, target_package, _) = action_target(element)??;
    let target_package = target_package?;
    Some(RequestAction::ChangeDevel {
        source_project,
        source_package,
        target_project,
        target_package,
    })
}

/// Parse a maintenance or release action: a source and a target,
/// both naming a project; the target package is optional.
fn parse_source_target_action(element: &Element<'_>, action_type: &str) -> Option<RequestAction> {
    for child in child_elements(element.inner)? {
        if child.name != "source" && child.name != "target" {
            return None;
        }
    }
    let (source_project, source_package, _) = action_source(element)?;
    let (target_project, target_package, _) = action_target(element)??;
    match action_type {
        "maintenance_incident" => Some(RequestAction::MaintenanceIncident {
            source_project,
            source_package,
            target_project,
            target_package,
        }),
        "maintenance_release" => Some(RequestAction::MaintenanceRelease {
            source_project,
            source_package,
            target_project,
            target_package,
        }),
        "release" => Some(RequestAction::Release {
            source_project,
            source_package,
            target_project,
            target_package,
        }),
        _ => None,
    }
}

/// Parse an old-style `<submit>` element (the legacy request form):
/// a source and a target, no options. The caller lifts it into a
/// modern submit action.
fn parse_legacy_submit(element: &Element<'_>) -> Option<RequestAction> {
    for child in child_elements(element.inner)? {
        if child.name != "source" && child.name != "target" {
            return None;
        }
    }
    let (source_project, source_package, source_rev) = action_source(element)?;
    let (target_project, target_package) = match action_target(element)? {
        Some((project, package, _)) => (Some(project), package),
        None => (None, None),
    };
    Some(RequestAction::Submit {
        source_project,
        source_package,
        source_rev,
        target_project,
        target_package,
        sourceupdate: None,
        updatelink: false,
        makeoriginolder: false,
        acceptinfo: None,
    })
}

/// Parse a `<state>` element: the name, who, and when are required;
/// `created`, `superseded_by`, and `approver` are optional. The
/// comment child is optional and reads as empty when absent.
fn parse_state(element: &Element<'_>) -> Option<RequestStateInfo> {
    let name = RequestState::parse(attr_value(element.attrs, "name")?)?;
    let who = attr_value(element.attrs, "who")?;
    let when = attr_value(element.attrs, "when")?;
    if who.is_empty() || when.is_empty() {
        return None;
    }
    let created = attr_value(element.attrs, "created").map(str::to_string);
    let superseded_by = match attr_value(element.attrs, "superseded_by") {
        Some(value) => Some(value.parse::<u64>().ok().filter(|id| *id > 0)?),
        None => None,
    };
    let approver = attr_value(element.attrs, "approver").map(str::to_string);
    let mut comment = String::new();
    let mut seen_comment = false;
    for child in child_elements(element.inner)? {
        if child.name != "comment" {
            return None;
        }
        if seen_comment {
            return None;
        }
        seen_comment = true;
        comment = leaf_text(child.inner)?;
    }
    Some(RequestStateInfo {
        name,
        who: who.to_string(),
        when: when.to_string(),
        created,
        superseded_by,
        approver,
        comment,
    })
}

/// Parse a `<review>` element: the state is required; when, who, and
/// the `by_*` fields are optional. The comment child is optional and
/// reads as empty when absent. A review that was acted on carries
/// one `<history>` child per change, each with a description of what
/// happened; they read like request history entries.
fn parse_review(element: &Element<'_>) -> Option<RequestReview> {
    let state = ReviewState::parse(attr_value(element.attrs, "state")?)?;
    let when = attr_value(element.attrs, "when").map(str::to_string);
    let who = attr_value(element.attrs, "who").map(str::to_string);
    let by_user = attr_value(element.attrs, "by_user").map(str::to_string);
    let by_group = attr_value(element.attrs, "by_group").map(str::to_string);
    let by_project = attr_value(element.attrs, "by_project").map(str::to_string);
    let by_package = attr_value(element.attrs, "by_package").map(str::to_string);
    let mut comment = String::new();
    let mut seen_comment = false;
    let mut history = Vec::new();
    for child in child_elements(element.inner)? {
        match child.name {
            "comment" => {
                if seen_comment {
                    return None;
                }
                seen_comment = true;
                comment = leaf_text(child.inner)?;
            }
            // A review that was acted on carries one `<history>`
            // child per change (accepted, declined, assigned,
            // reopened, obsoleted), each with a description of what
            // happened — never empty, and never a placeholder. The
            // entries read like request history entries.
            "history" => history.push(parse_request_history(&child)?),
            _ => return None,
        }
    }
    Some(RequestReview {
        state,
        when,
        who,
        by_user,
        by_group,
        by_project,
        by_package,
        comment,
        history,
    })
}

/// Parse a `<history>` element: who and when are required; the
/// description and comment children are optional and read as empty
/// when absent.
fn parse_request_history(element: &Element<'_>) -> Option<RequestHistory> {
    let who = attr_value(element.attrs, "who")?;
    let when = attr_value(element.attrs, "when")?;
    if who.is_empty() || when.is_empty() {
        return None;
    }
    let mut description = String::new();
    let mut comment = String::new();
    let mut seen_description = false;
    let mut seen_comment = false;
    for child in child_elements(element.inner)? {
        match child.name {
            "description" => {
                if seen_description {
                    return None;
                }
                seen_description = true;
                description = leaf_text(child.inner)?;
            }
            "comment" => {
                if seen_comment {
                    return None;
                }
                seen_comment = true;
                comment = leaf_text(child.inner)?;
            }
            _ => return None,
        }
    }
    Some(RequestHistory {
        who: who.to_string(),
        when: when.to_string(),
        description,
        comment,
    })
}

/// Render one action as XML, in the server's shape: the same
/// elements, in the server's order, with the same attribute order
/// (`project` before `package`, `rev` last on a source).
fn action_to_xml(action: &RequestAction) -> String {
    let mut out = String::new();
    match action {
        RequestAction::Submit {
            source_project,
            source_package,
            source_rev,
            target_project,
            target_package,
            sourceupdate,
            updatelink,
            makeoriginolder,
            acceptinfo,
        } => {
            out.push_str("  <action type=\"submit\">\n");
            out.push_str(&source_to_xml(
                source_project,
                source_package,
                source_rev.as_deref(),
            ));
            if let Some(target_project) = target_project {
                out.push_str(&target_to_xml(
                    target_project,
                    target_package.as_deref(),
                    None,
                ));
            }
            if sourceupdate.is_some() || *updatelink || *makeoriginolder {
                out.push_str("    <options>\n");
                if let Some(update) = sourceupdate {
                    out.push_str(&format!(
                        "      <sourceupdate>{}</sourceupdate>\n",
                        update.as_str()
                    ));
                }
                if *updatelink {
                    out.push_str("      <updatelink>true</updatelink>\n");
                }
                if *makeoriginolder {
                    out.push_str("      <makeoriginolder>true</makeoriginolder>\n");
                }
                out.push_str("    </options>\n");
            }
            if let Some(info) = acceptinfo {
                out.push_str(&format!(
                    "    <acceptinfo rev=\"{}\" srcmd5=\"{}\"",
                    xml_escape(&info.rev),
                    xml_escape(&info.srcmd5)
                ));
                for (name, value) in [
                    ("oproject", &info.oproject),
                    ("opackage", &info.opackage),
                    ("osrcmd5", &info.osrcmd5),
                    ("xsrcmd5", &info.xsrcmd5),
                    ("oxsrcmd5", &info.oxsrcmd5),
                ] {
                    if let Some(value) = value {
                        out.push_str(&format!(" {name}=\"{}\"", xml_escape(value)));
                    }
                }
                out.push_str(" />\n");
            }
            out.push_str("  </action>\n");
        }
        RequestAction::Delete {
            target_project,
            target_package,
            target_repository,
        } => {
            out.push_str("  <action type=\"delete\">\n");
            out.push_str(&target_to_xml(
                target_project,
                target_package.as_deref(),
                target_repository.as_deref(),
            ));
            out.push_str("  </action>\n");
        }
        RequestAction::AddRole {
            target_project,
            target_package,
            person,
            group,
        } => {
            out.push_str("  <action type=\"add_role\">\n");
            out.push_str(&target_to_xml(
                target_project,
                target_package.as_deref(),
                None,
            ));
            if let Some((name, role)) = person {
                out.push_str(&format!(
                    "    <person name=\"{}\" role=\"{}\" />\n",
                    xml_escape(name),
                    xml_escape(role)
                ));
            }
            if let Some((name, role)) = group {
                out.push_str(&format!(
                    "    <group name=\"{}\" role=\"{}\" />\n",
                    xml_escape(name),
                    xml_escape(role)
                ));
            }
            out.push_str("  </action>\n");
        }
        RequestAction::SetBugowner {
            target_project,
            target_package,
            person_name,
            group_name,
        } => {
            out.push_str("  <action type=\"set_bugowner\">\n");
            out.push_str(&target_to_xml(
                target_project,
                target_package.as_deref(),
                None,
            ));
            if let Some(name) = person_name {
                out.push_str(&format!("    <person name=\"{}\" />\n", xml_escape(name)));
            }
            if let Some(name) = group_name {
                out.push_str(&format!("    <group name=\"{}\" />\n", xml_escape(name)));
            }
            out.push_str("  </action>\n");
        }
        RequestAction::ChangeDevel {
            source_project,
            source_package,
            target_project,
            target_package,
        } => {
            out.push_str("  <action type=\"change_devel\">\n");
            out.push_str(&source_to_xml(source_project, source_package, None));
            out.push_str(&target_to_xml(target_project, Some(target_package), None));
            out.push_str("  </action>\n");
        }
        RequestAction::MaintenanceIncident {
            source_project,
            source_package,
            target_project,
            target_package,
        }
        | RequestAction::MaintenanceRelease {
            source_project,
            source_package,
            target_project,
            target_package,
        }
        | RequestAction::Release {
            source_project,
            source_package,
            target_project,
            target_package,
        } => {
            out.push_str(&format!("  <action type=\"{}\">\n", action.type_name()));
            out.push_str(&source_to_xml(source_project, source_package, None));
            out.push_str(&target_to_xml(
                target_project,
                target_package.as_deref(),
                None,
            ));
            out.push_str("  </action>\n");
        }
    }
    out
}

/// Render a `<source>` element: `project`, then `package`, then
/// `rev` when one is pinned.
fn source_to_xml(project: &str, package: &str, rev: Option<&str>) -> String {
    let mut out = format!(
        "    <source project=\"{}\" package=\"{}\"",
        xml_escape(project),
        xml_escape(package)
    );
    if let Some(rev) = rev {
        out.push_str(&format!(" rev=\"{}\"", xml_escape(rev)));
    }
    out.push_str(" />\n");
    out
}

/// Render a `<target>` element: `project`, then `package`, then
/// `repository`, each only when present.
fn target_to_xml(project: &str, package: Option<&str>, repository: Option<&str>) -> String {
    let mut out = format!("    <target project=\"{}\"", xml_escape(project));
    if let Some(package) = package {
        out.push_str(&format!(" package=\"{}\"", xml_escape(package)));
    }
    if let Some(repository) = repository {
        out.push_str(&format!(" repository=\"{}\"", xml_escape(repository)));
    }
    out.push_str(" />\n");
    out
}

/// Render a `<state>` element: `name`, `who`, `when`, then `created`,
/// `superseded_by`, and `approver` when present. The comment child is
/// written only when non-empty; an empty comment reads back as the
/// empty string, so writing it out would add nothing.
fn state_to_xml(state: &RequestStateInfo) -> String {
    let mut out = format!(
        "  <state name=\"{}\" who=\"{}\" when=\"{}\"",
        state.name.as_str(),
        xml_escape(&state.who),
        xml_escape(&state.when)
    );
    if let Some(created) = &state.created {
        out.push_str(&format!(" created=\"{}\"", xml_escape(created)));
    }
    if let Some(superseded_by) = state.superseded_by {
        out.push_str(&format!(" superseded_by=\"{superseded_by}\""));
    }
    if let Some(approver) = &state.approver {
        out.push_str(&format!(" approver=\"{}\"", xml_escape(approver)));
    }
    if state.comment.is_empty() {
        out.push_str(" />\n");
    } else {
        out.push_str(&format!(
            ">\n    <comment>{}</comment>\n  </state>\n",
            xml_escape(&state.comment)
        ));
    }
    out
}

/// Render a `<review>` element: `state`, `when`, `who`, then the
/// `by_*` fields when present. The comment child is written only when
/// non-empty; the review's history entries follow it, in order.
fn review_to_xml(review: &RequestReview) -> String {
    let mut out = format!("  <review state=\"{}\"", review.state.as_str());
    if let Some(when) = &review.when {
        out.push_str(&format!(" when=\"{}\"", xml_escape(when)));
    }
    if let Some(who) = &review.who {
        out.push_str(&format!(" who=\"{}\"", xml_escape(who)));
    }
    if let Some(by_group) = &review.by_group {
        out.push_str(&format!(" by_group=\"{}\"", xml_escape(by_group)));
    }
    if let Some(by_user) = &review.by_user {
        out.push_str(&format!(" by_user=\"{}\"", xml_escape(by_user)));
    }
    if let Some(by_package) = &review.by_package {
        out.push_str(&format!(" by_package=\"{}\"", xml_escape(by_package)));
    }
    if let Some(by_project) = &review.by_project {
        out.push_str(&format!(" by_project=\"{}\"", xml_escape(by_project)));
    }
    out.push_str(">\n");
    if !review.comment.is_empty() {
        out.push_str(&format!(
            "    <comment>{}</comment>\n",
            xml_escape(&review.comment)
        ));
    }
    for entry in &review.history {
        out.push_str(&history_to_xml(entry, 4));
    }
    out.push_str("  </review>\n");
    out
}

/// Render a `<history>` element at `indent` spaces: `who`, then
/// `when`. The description and comment children are written only
/// when non-empty, indented two further spaces.
fn history_to_xml(entry: &RequestHistory, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let child_pad = " ".repeat(indent + 2);
    if entry.description.is_empty() && entry.comment.is_empty() {
        return format!(
            "{pad}<history who=\"{}\" when=\"{}\" />\n",
            xml_escape(&entry.who),
            xml_escape(&entry.when)
        );
    }
    let mut out = format!(
        "{pad}<history who=\"{}\" when=\"{}\">\n",
        xml_escape(&entry.who),
        xml_escape(&entry.when)
    );
    if !entry.description.is_empty() {
        out.push_str(&format!(
            "{child_pad}<description>{}</description>\n",
            xml_escape(&entry.description)
        ));
    }
    if !entry.comment.is_empty() {
        out.push_str(&format!(
            "{child_pad}<comment>{}</comment>\n",
            xml_escape(&entry.comment)
        ));
    }
    out.push_str(&format!("{pad}</history>\n"));
    out
}

/// Parse a `<collection>` of whole `<request>` elements — the shape
/// the request search returns — into the requests it carries. Each
/// element is parsed with [`parse_request`]; any element that fails
/// fails the whole collection, since a search result the library
/// cannot read in full is not a result it can act on. The number of
/// elements parsed must equal the `matches` count: the server returns
/// every match in one response (over the limit it refuses with 403
/// rather than truncating), so a short collection is a server shape
/// change, not a partial result.
pub fn parse_request_collection(body: &str) -> Option<Vec<Request>> {
    if !body.contains("<collection") {
        return None;
    }
    let matches = parse_search_matches(body)?;
    let mut requests = Vec::new();
    let mut parsed = 0usize;
    let mut rest = body;
    while let Some(pos) = rest.find("<request") {
        let after_tag = &rest[pos + "<request".len()..];
        let first = after_tag.chars().next()?;
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let end = after_tag.find('>')?;
        let tag = &after_tag[..end];
        parsed += 1;
        if tag.trim_end().ends_with('/') {
            // Self-closing: no children, so not a whole request.
            return None;
        }
        let inner = &after_tag[end + 1..];
        let close = inner.find("</request>")?;
        let block = &inner[..close];
        let element = format!("<request{tag}>{block}</request>");
        requests.push(parse_request(&element)?);
        rest = &inner[close + "</request>".len()..];
    }
    if parsed != matches as usize {
        return None;
    }
    Some(requests)
}

/// One comment on a request: who left it, when, what it says, and
/// which comment it replies to, when it is a reply. Comments on a
/// request and comments on its actions are listed together, in id
/// order — the server sorts them, and a threaded reply carries its
/// parent's id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestComment {
    /// The comment's id, for threading replies under it.
    pub id: u64,
    /// Who left the comment.
    pub who: String,
    /// When the comment was left.
    pub when: String,
    /// The comment this one replies to, when it is a reply.
    pub parent_id: Option<u64>,
    /// The comment's text.
    pub body: String,
}

/// Parse a `<comments>` document — the shape the comment list returns
/// — into the comments it carries, in document order. An empty
/// document reads as an empty list: a request with no comments is a
/// normal request. Anything the parser does not recognise — an
/// unknown child element, a missing required attribute — fails the
/// whole document.
pub fn parse_comments(body: &str) -> Option<Vec<RequestComment>> {
    if !body.contains("<comments") {
        return None;
    }
    let root = parse_root(body.trim(), "comments")?;
    let mut comments = Vec::new();
    for child in child_elements(root.inner)? {
        if child.name != "comment" {
            return None;
        }
        let id = attr_value(child.attrs, "id")?;
        let id = id.parse::<u64>().ok().filter(|id| *id > 0)?;
        let who = attr_value(child.attrs, "who")?;
        let when = attr_value(child.attrs, "when")?;
        if who.is_empty() || when.is_empty() {
            return None;
        }
        let parent_id = match attr_value(child.attrs, "parent") {
            Some(value) => Some(value.parse::<u64>().ok().filter(|id| *id > 0)?),
            None => None,
        };
        let body = leaf_text(child.inner)?;
        comments.push(RequestComment {
            id,
            who: who.to_string(),
            when: when.to_string(),
            parent_id,
            body,
        });
    }
    Some(comments)
}

/// Who a review is asked of: a user, a group, a project, or a
/// project/package pair. At least one of user, group, and project is
/// required — the server refuses a review asked of nobody — and a
/// package review needs its project, since a package name alone
/// names nothing. Built through [`ReviewTarget::new`], so an invalid
/// target fails before any request is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTarget {
    /// The user the review is asked of, when it names one.
    pub by_user: Option<String>,
    /// The group the review is asked of, when it names one.
    pub by_group: Option<String>,
    /// The project the review is asked of, when it names one.
    pub by_project: Option<String>,
    /// The package the review is asked of, when it names one.
    pub by_package: Option<String>,
}

impl ReviewTarget {
    /// Build a review target from its parts. Each part is optional;
    /// an empty string for a part is refused rather than silently
    /// dropped, and a project or package name must be a valid OBS
    /// name.
    pub fn new(
        by_user: Option<&str>,
        by_group: Option<&str>,
        by_project: Option<&str>,
        by_package: Option<&str>,
    ) -> Result<Self, GeectlError> {
        let by_user = validate_review_party("user", by_user)?;
        let by_group = validate_review_party("group", by_group)?;
        let by_project = match by_project {
            Some("") => {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "a review project name must not be empty",
                ));
            }
            Some(value) if !valid_obs_name(value) => {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS project name: {value:?}"),
                ));
            }
            Some(value) => Some(value.to_string()),
            None => None,
        };
        let by_package = match by_package {
            Some("") => {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "a review package name must not be empty",
                ));
            }
            Some(value) if !valid_obs_name(value) => {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name: {value:?}"),
                ));
            }
            Some(value) => Some(value.to_string()),
            None => None,
        };
        if by_user.is_none() && by_group.is_none() && by_project.is_none() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "a review must be asked of a user, a group, or a project",
            ));
        }
        if by_package.is_some() && by_project.is_none() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "a review asked of a package needs its project",
            ));
        }
        Ok(ReviewTarget {
            by_user,
            by_group,
            by_project,
            by_package,
        })
    }
}

/// Validate one review party (a user or a group login): absent is
/// fine, present must be non-empty.
fn validate_review_party(label: &str, value: Option<&str>) -> Result<Option<String>, GeectlError> {
    match value {
        Some("") => Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("a review {label} name must not be empty"),
        )),
        Some(value) => Ok(Some(value.to_string())),
        None => Ok(None),
    }
}

impl ReviewTarget {
    /// The target as a short phrase for logs and dry-run plans, e.g.
    /// `user Admin`, `group reviewers`, `project openSUSE:Factory`,
    /// `project openSUSE:Factory/package foo`. The parts present are
    /// joined with `, `.
    pub fn describe(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(by_user) = &self.by_user {
            parts.push(format!("user {by_user}"));
        }
        if let Some(by_group) = &self.by_group {
            parts.push(format!("group {by_group}"));
        }
        if let Some(by_project) = &self.by_project {
            match &self.by_package {
                Some(by_package) => {
                    parts.push(format!("project {by_project}/package {by_package}"));
                }
                None => parts.push(format!("project {by_project}")),
            }
        }
        parts.join(", ")
    }
}

/// One XPath predicate over the request XML, for the `match=`
/// parameter of the request search. The tree carries the precedence:
/// `and` binds tighter than `or`, and rendering parenthesises a child
/// whose operator differs from its parent's, so the string reads back
/// with the same grouping the tree has. A leaf is a single comparison
/// (`@name='foo'`, `contains(@name, 'foo')`); `Pred` is an element
/// predicate (`state[@name='new']`) — the brackets scope the inner
/// query, so it never needs parens of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XpathQuery {
    /// No predicate; renders as the empty string.
    Empty,
    /// `path='value'`.
    Eq(String, String),
    /// `not(inner)`.
    Not(Box<XpathQuery>),
    /// `contains(path, 'value')`.
    Contains(String, String),
    /// `left and right`.
    And(Box<XpathQuery>, Box<XpathQuery>),
    /// `left or right`.
    Or(Box<XpathQuery>, Box<XpathQuery>),
    /// `element[inner]`.
    Pred(String, Box<XpathQuery>),
}

impl XpathQuery {
    /// The `and` of several queries, left-associated, skipping empty
    /// ones. No queries reads as the empty query.
    pub fn and_all(queries: Vec<XpathQuery>) -> XpathQuery {
        queries
            .into_iter()
            .filter(|query| *query != XpathQuery::Empty)
            .reduce(|left, right| XpathQuery::And(Box::new(left), Box::new(right)))
            .unwrap_or(XpathQuery::Empty)
    }

    /// The `or` of several queries, left-associated, skipping empty
    /// ones. No queries reads as the empty query.
    pub fn or_all(queries: Vec<XpathQuery>) -> XpathQuery {
        queries
            .into_iter()
            .filter(|query| *query != XpathQuery::Empty)
            .reduce(|left, right| XpathQuery::Or(Box::new(left), Box::new(right)))
            .unwrap_or(XpathQuery::Empty)
    }

    /// `path='value1' or path='value2' …` for a list of values. An
    /// empty list reads as the empty query.
    pub fn eq_list(path: &str, values: &[String]) -> XpathQuery {
        XpathQuery::or_all(
            values
                .iter()
                .map(|value| XpathQuery::Eq(path.to_string(), value.clone()))
                .collect(),
        )
    }
}

impl fmt::Display for XpathQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XpathQuery::Empty => Ok(()),
            XpathQuery::Eq(path, value) => write!(f, "{path}='{value}'"),
            XpathQuery::Not(inner) => write!(f, "not({inner})"),
            XpathQuery::Contains(path, value) => write!(f, "contains({path}, '{value}')"),
            XpathQuery::And(left, right) => {
                let rendered_left = render_xpath_child(left, true);
                let rendered_right = render_xpath_child(right, true);
                write!(f, "{rendered_left} and {rendered_right}")
            }
            XpathQuery::Or(left, right) => {
                let rendered_left = render_xpath_child(left, false);
                let rendered_right = render_xpath_child(right, false);
                write!(f, "{rendered_left} or {rendered_right}")
            }
            XpathQuery::Pred(element, inner) => write!(f, "{element}[{inner}]"),
        }
    }
}

/// Render a child of an `and` (`is_and`) or `or` node: a child whose
/// operator differs from its parent's is parenthesised, so the
/// precedence the tree carries survives the flat string; a child with
/// the same operator, or a leaf, renders bare. An `and` of two `and`s
/// reads flat (`a and b and c`); an `or` under an `and` reads
/// parenthesised (`(a or b) and c`).
fn render_xpath_child(child: &XpathQuery, is_and: bool) -> String {
    let differs = match child {
        XpathQuery::And(..) => !is_and,
        XpathQuery::Or(..) => is_and,
        _ => false,
    };
    if differs {
        format!("({child})")
    } else {
        format!("{child}")
    }
}

/// The action types the request search can filter by: exactly the
/// types [`RequestAction`] parses. An unknown type in a search would
/// match nothing on the server, so the builder refuses it instead.
const KNOWN_ACTION_TYPES: &[&str] = &[
    "submit",
    "delete",
    "add_role",
    "set_bugowner",
    "change_devel",
    "maintenance_incident",
    "maintenance_release",
    "release",
];

/// A typed request search: the filters for `GET /search/request`,
/// built into the `match=` expression the server evaluates. Every
/// filter is a list, and a list matches when any of its values
/// matches; the filters themselves are `and`-joined. The clause order
/// is fixed (ids, projects, packages, source, target, creators, who,
/// states, types, exclusions), so the same search always builds the
/// same expression.
///
/// The states default to the open states — `new`, `review`,
/// `declined` — the set a tool triaging open work wants. Pass an
/// explicit (possibly empty) state list to override: an empty list
/// means no state filter at all.
///
/// The `source_packages` / `target_packages` filters use the
/// singular attribute (`source[@package='…']`): the search engine's
/// whitelist has `source/@package` and `target/@package` and no
/// plural form, so the plural is refused by the server with 400.
///
/// History: `with_history` adds the creation entry and the request
/// history to each result; `with_full_history` adds all history
/// elements. When both are set the server prefers the full history.
#[derive(Debug, Clone, Default)]
pub struct RequestSearch {
    /// Match requests with one of these numbers.
    request_ids: Vec<u64>,
    /// Match requests whose source or target project is one of these.
    projects: Vec<String>,
    /// Match requests whose source or target package is one of these.
    packages: Vec<String>,
    /// Match requests whose source project is one of these.
    source_projects: Vec<String>,
    /// Match requests whose source package is one of these.
    source_packages: Vec<String>,
    /// Match requests whose target project is one of these.
    target_projects: Vec<String>,
    /// Match requests whose target package is one of these.
    target_packages: Vec<String>,
    /// Match requests created by one of these users.
    creators: Vec<String>,
    /// Match requests one of these users interacted with (as the last
    /// actor on the state, or in the history).
    who: Vec<String>,
    /// Match requests in one of these states. `None` is the default
    /// open states; `Some(vec![])` is no state filter.
    states: Option<Vec<RequestState>>,
    /// Match requests with an action of one of these types.
    types: Vec<String>,
    /// Exclude requests targeting one of these projects.
    exclude_target_projects: Vec<String>,
    /// Include the creation entry and the request history in each
    /// result.
    pub(crate) with_history: bool,
    /// Include all history elements in each result.
    with_full_history: bool,
}

impl RequestSearch {
    /// An empty search: the default open states, no other filter, no
    /// history.
    pub fn new() -> Self {
        RequestSearch::default()
    }

    /// Match requests with one of these numbers.
    pub fn request_ids(mut self, request_ids: Vec<u64>) -> Self {
        self.request_ids = request_ids;
        self
    }

    /// Match requests whose source or target project is one of these.
    pub fn projects(mut self, projects: Vec<String>) -> Self {
        self.projects = projects;
        self
    }

    /// Match requests whose source or target package is one of these.
    pub fn packages(mut self, packages: Vec<String>) -> Self {
        self.packages = packages;
        self
    }

    /// Match requests whose source project is one of these.
    pub fn source_projects(mut self, source_projects: Vec<String>) -> Self {
        self.source_projects = source_projects;
        self
    }

    /// Match requests whose source package is one of these.
    pub fn source_packages(mut self, source_packages: Vec<String>) -> Self {
        self.source_packages = source_packages;
        self
    }

    /// Match requests whose target project is one of these.
    pub fn target_projects(mut self, target_projects: Vec<String>) -> Self {
        self.target_projects = target_projects;
        self
    }

    /// Match requests whose target package is one of these.
    pub fn target_packages(mut self, target_packages: Vec<String>) -> Self {
        self.target_packages = target_packages;
        self
    }

    /// Match requests created by one of these users.
    pub fn creators(mut self, creators: Vec<String>) -> Self {
        self.creators = creators;
        self
    }

    /// Match requests one of these users interacted with.
    pub fn who(mut self, who: Vec<String>) -> Self {
        self.who = who;
        self
    }

    /// Match requests in one of these states. An empty list means no
    /// state filter at all.
    pub fn states(mut self, states: Vec<RequestState>) -> Self {
        self.states = Some(states);
        self
    }

    /// Match requests with an action of one of these types.
    pub fn types(mut self, types: Vec<String>) -> Self {
        self.types = types;
        self
    }

    /// Exclude requests targeting one of these projects.
    pub fn exclude_target_projects(mut self, exclude_target_projects: Vec<String>) -> Self {
        self.exclude_target_projects = exclude_target_projects;
        self
    }

    /// Include the creation entry and the request history in each
    /// result.
    pub fn with_history(mut self, with_history: bool) -> Self {
        self.with_history = with_history;
        self
    }

    /// Include all history elements in each result.
    pub fn with_full_history(mut self, with_full_history: bool) -> Self {
        self.with_full_history = with_full_history;
        self
    }

    /// Whether the search asks for the request history.
    pub fn wants_history(&self) -> bool {
        self.with_history
    }

    /// Whether the search asks for the full history.
    pub fn wants_full_history(&self) -> bool {
        self.with_full_history
    }

    /// Build the `match=` expression for this search. Every value is
    /// validated first: project and package names must be valid OBS
    /// names, user names must be non-empty and quote-free, and action
    /// types must be ones the server renders. A value that would
    /// break out of its XPath string literal is refused, because
    /// XPath has no string escape and a quote in a value would
    /// rewrite the query. A search with no filter at all — no
    /// clause, not even a state — is refused too: the server answers
    /// an empty `match=` with 400 (`empty_match`), so there is no
    /// unfiltered search.
    pub fn match_expr(&self) -> Result<String, GeectlError> {
        for (label, values) in [
            ("project", &self.projects),
            ("package", &self.packages),
            ("source project", &self.source_projects),
            ("source package", &self.source_packages),
            ("target project", &self.target_projects),
            ("target package", &self.target_packages),
            ("excluded target project", &self.exclude_target_projects),
        ] {
            for value in values {
                if !valid_obs_name(value) {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!("invalid OBS {label} name: {value:?}"),
                    ));
                }
            }
        }
        for (label, values) in [("creator", &self.creators), ("user", &self.who)] {
            for value in values {
                validate_xpath_value(label, value)?;
            }
        }
        for action_type in &self.types {
            if !KNOWN_ACTION_TYPES.contains(&action_type.as_str()) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "unknown request action type: {action_type:?} (known: {})",
                        KNOWN_ACTION_TYPES.join(", ")
                    ),
                ));
            }
        }
        let mut clauses: Vec<XpathQuery> = Vec::new();
        if !self.request_ids.is_empty() {
            let ids: Vec<String> = self.request_ids.iter().map(|id| id.to_string()).collect();
            clauses.push(XpathQuery::eq_list("@id", &ids));
        }
        if !self.projects.is_empty() {
            clauses.push(XpathQuery::Or(
                Box::new(XpathQuery::Pred(
                    "source".to_string(),
                    Box::new(XpathQuery::eq_list("@project", &self.projects)),
                )),
                Box::new(XpathQuery::Pred(
                    "target".to_string(),
                    Box::new(XpathQuery::eq_list("@project", &self.projects)),
                )),
            ));
        }
        if !self.packages.is_empty() {
            clauses.push(XpathQuery::Or(
                Box::new(XpathQuery::Pred(
                    "source".to_string(),
                    Box::new(XpathQuery::eq_list("@package", &self.packages)),
                )),
                Box::new(XpathQuery::Pred(
                    "target".to_string(),
                    Box::new(XpathQuery::eq_list("@package", &self.packages)),
                )),
            ));
        }
        if !self.source_projects.is_empty() {
            clauses.push(XpathQuery::Pred(
                "source".to_string(),
                Box::new(XpathQuery::eq_list("@project", &self.source_projects)),
            ));
        }
        if !self.source_packages.is_empty() {
            clauses.push(XpathQuery::Pred(
                "source".to_string(),
                Box::new(XpathQuery::eq_list("@package", &self.source_packages)),
            ));
        }
        if !self.target_projects.is_empty() {
            clauses.push(XpathQuery::Pred(
                "target".to_string(),
                Box::new(XpathQuery::eq_list("@project", &self.target_projects)),
            ));
        }
        if !self.target_packages.is_empty() {
            clauses.push(XpathQuery::Pred(
                "target".to_string(),
                Box::new(XpathQuery::eq_list("@package", &self.target_packages)),
            ));
        }
        if !self.creators.is_empty() {
            clauses.push(XpathQuery::eq_list("@creator", &self.creators));
        }
        if !self.who.is_empty() {
            clauses.push(XpathQuery::Or(
                Box::new(XpathQuery::Pred(
                    "state".to_string(),
                    Box::new(XpathQuery::eq_list("@who", &self.who)),
                )),
                Box::new(XpathQuery::Pred(
                    "history".to_string(),
                    Box::new(XpathQuery::eq_list("@who", &self.who)),
                )),
            ));
        }
        let states = match &self.states {
            Some(states) => states.clone(),
            None => vec![
                RequestState::New,
                RequestState::Review,
                RequestState::Declined,
            ],
        };
        if !states.is_empty() {
            let values: Vec<String> = states
                .iter()
                .map(|state| state.as_str().to_string())
                .collect();
            clauses.push(XpathQuery::Pred(
                "state".to_string(),
                Box::new(XpathQuery::eq_list("@name", &values)),
            ));
        }
        if !self.types.is_empty() {
            clauses.push(XpathQuery::Pred(
                "action".to_string(),
                Box::new(XpathQuery::eq_list("@type", &self.types)),
            ));
        }
        for project in &self.exclude_target_projects {
            // `target[not(@project='X')]`: no target of the request
            // is project X. The outer form `not(target[@project='X'])`
            // is refused by the server with 400 — verified live
            // against the production API — so this predicate form is
            // the only one the dialect accepts. One caveat: the
            // server evaluates the match per joined action row, so a
            // multi-action request targeting both X and another
            // project still matches via the other row, and a request
            // with no target at all is not excluded. The form is
            // exact for the overwhelmingly common single-action case.
            clauses.push(XpathQuery::Pred(
                "target".to_string(),
                Box::new(XpathQuery::Not(Box::new(XpathQuery::Eq(
                    "@project".to_string(),
                    project.clone(),
                )))),
            ));
        }
        if clauses.is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "a request search needs at least one filter; the server refuses an empty match",
            ));
        }
        Ok(XpathQuery::and_all(clauses).to_string())
    }
}

/// Validate a value that goes into a single-quoted XPath string
/// literal: it must be non-empty, and it must not contain a single
/// quote, which would break out of the literal (XPath has no string
/// escape).
fn validate_xpath_value(label: &str, value: &str) -> Result<(), GeectlError> {
    if value.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("an XPath {label} value must not be empty"),
        ));
    }
    if value.contains('\'') {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("an XPath {label} value must not contain a single quote: {value:?}"),
        ));
    }
    Ok(())
}

impl<'a> ObsClient<'a> {
    /// True when an open submit request targets `package`.
    ///
    /// Uses the OBS search API, matching on actions targeting the
    /// package in the `new`/`review` states, then keeping only the
    /// requests that carry a submit action: the type filter lives
    /// client-side in `parse_submit_request_ids` as defence-in-depth
    /// — the engine does whitelist `action/@type`, so filtering
    /// server-side is a follow-up, not this change. The
    /// search has no public equivalent: without credentials it is
    /// refused here, before any request, instead of made and 401'd —
    /// a failed check is fail-closed (an error), never "no open SR".
    /// An unexpected response (no `matches` count) is an error, not
    /// false.
    pub fn has_open_sr(&self, package: &str) -> Result<bool, GeectlError> {
        Ok(!self.open_target_request_ids(package)?.is_empty())
    }

    /// The ids of the open submit requests targeting `package` (state
    /// `new` or `review`): the id-returning form of the
    /// [`ObsClient::has_open_sr`] search. A caller that needs to
    /// exempt one request — [`SrFiling::refile`] exempting the request
    /// it supersedes — uses this instead of the boolean, so the
    /// exemption is decided on the ids the server actually returned.
    /// Requires credentials; an unexpected response is an error,
    /// never an empty list.
    pub(crate) fn open_target_request_ids(
        &self,
        package: &str,
    ) -> Result<Vec<String>, GeectlError> {
        let url = self.search_url(package)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "checking for open submit requests requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_submit_request_ids(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS search response for package {package:?}"),
            )
        })
    }

    /// The ids of the open requests (`new` or `review` state) whose
    /// single action matches `target` exactly. The search filters on
    /// source/target, state, and action type (`action/@type`); the
    /// parser re-checks the type and narrows by shape where the
    /// search cannot — a delete search cannot match "no target
    /// package", and the incident identity includes the release
    /// project. The match mirrors the server's action identity —
    /// delete shapes are compared by subsumption, not by key (a
    /// whole-project target subsumes package/repository targets):
    /// the incident keys on target and source package — and on the
    /// release project when one is given, while a filing without one
    /// matches any stored release project — and change-devel keys on
    /// the target alone. A declined request is closed
    /// and does not block a re-file, so it is not in the state set —
    /// only genuinely open requests block a filing. Requires
    /// credentials; an unexpected response is an error, never an
    /// empty list.
    pub(crate) fn open_action_request_ids(
        &self,
        target: &ActionTarget<'_>,
    ) -> Result<Vec<String>, GeectlError> {
        let states = "(state/@name='new' or state/@name='review')";
        let mut action_parts: Vec<String> = Vec::new();
        if let Some(source_project) = target.source_project {
            if !valid_obs_name(source_project) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS project name for source project: {source_project:?}"),
                ));
            }
            action_parts.push(format!("source/@project='{source_project}'"));
        }
        if let Some(source_package) = target.source_package {
            if !valid_obs_name(source_package) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name for source package: {source_package:?}"),
                ));
            }
            action_parts.push(format!("source/@package='{source_package}'"));
        }
        if !valid_obs_name(target.target_project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "invalid OBS project name for target project: {:?}",
                    target.target_project
                ),
            ));
        }
        action_parts.push(format!("target/@project='{}'", target.target_project));
        if let Some(target_package) = target.target_package {
            if !valid_obs_name(target_package) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name for target package: {target_package:?}"),
                ));
            }
            // A delete search leaves the package out: the whole-project
            // form has no target package, and a match on one would
            // never see an open whole-project delete. The parser
            // narrows by shape instead.
            if !matches!(target.shape, ActionShape::DeleteTarget { .. }) {
                action_parts.push(format!("target/@package='{target_package}'"));
            }
        }
        if let Some(target_repository) = target.target_repository {
            if !valid_obs_name(target_repository) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "invalid OBS repository name for target repository: {target_repository:?}"
                    ),
                ));
            }
            // Validated but never added to the match: the search
            // whitelist has no `action/target/@repository`, and a
            // delete dedup matches the project — the parser narrows
            // by shape instead.
        }
        let match_expr = format!(
            "{states} and action/@type='{}' and action[{}]",
            target.action_type,
            action_parts.join(" and ")
        );
        let url = build_url(
            &self.base,
            &["search", "request"],
            &[("match", &match_expr)],
        );
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "checking for open requests requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_request_ids_for_action(&body, target.action_type, target.shape).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected OBS search response for {} requests",
                    target.action_type
                ),
            )
        })
    }

    /// File `payload` as a submit request: render it and POST it to
    /// the OBS API (`POST /request?cmd=create`) and return the new
    /// request id. Requires credentials; without them the filing
    /// fails loudly before any request.
    ///
    /// Private: [`SrFiling`] is the only caller. Filing goes through
    /// the guarded workflow there (payload validation, dry-run, and
    /// the dedup check) — a direct call would bypass all three, so
    /// there is no direct call to bypass them with. The request XML
    /// is rendered here from the typed payload; no caller passes XML.
    pub(crate) fn create_request(&self, payload: &SrPayload) -> Result<String, GeectlError> {
        self.create_request_with_query(&payload.to_xml(), &[("cmd", "create")], "submit")
    }

    /// File a maintenance incident request: the same filing POST as
    /// [`ObsClient::create_request`], with `addrevision=1`, which pins
    /// `source_rev` to the current expanded srcmd5 at filing time and
    /// refuses to file when the source is broken. The server applies
    /// it to submit, release, maintenance_incident and
    /// maintenance_release actions (`RequestController#create` sets it,
    /// `BsRequest#set_add_revision`, enforced in
    /// `BsRequestAction#check_for_expand_errors!`); this library sends
    /// it only for maintenance incidents, where the pin is the point —
    /// an incident must freeze the exact source it was filed against.
    /// Same convention as `create_request`: [`SrFiling`] is the single
    /// permitted caller in this crate.
    pub(crate) fn create_request_with_add_revision(
        &self,
        xml: &str,
    ) -> Result<String, GeectlError> {
        self.create_request_with_query(
            xml,
            &[("cmd", "create"), ("addrevision", "1")],
            "maintenance incident",
        )
    }

    /// The shared filing POST: `POST /request` with the given query
    /// parameters and the request XML as the body, returning the new
    /// request id. Requires credentials; without them the filing
    /// fails loudly before any request. `kind` names the request
    /// type in the credentials error only.
    pub(crate) fn create_request_with_query(
        &self,
        xml: &str,
        query: &[(&str, &str)],
        kind: &str,
    ) -> Result<String, GeectlError> {
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                format!(
                    "cannot file a {kind} request without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)"
                ),
            ));
        }
        let url = build_url(&self.base, &["request"], query);
        let body = self.http.post(&url, xml, &self.credentials)?;
        parse_request_id(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS create-request response: no request id",
            )
        })
    }

    /// Supersede submit request `old_id` with `new_id`: the old
    /// request moves to state `superseded`, with `superseded_by` set
    /// to the new id. Requires credentials; without them the call
    /// fails loudly before any request.
    ///
    /// Semantics, verified against the OBS API server
    /// (`BsRequest#change_state` and
    /// `BsRequestPermissionCheck#cmd_changestate_permissions`) and
    /// the live API — not taken from osc:
    /// - `superseded_by` is REQUIRED: the server refuses a supersede
    ///   without it (`PostRequestMissingParameter`), and the model
    ///   requires it to be a positive number. The library always
    ///   sends it.
    /// - Allowed from `new`, `review`, and `declined`. Refused from a
    ///   final state: `accepted` cannot change state at all, and a
    ///   final state cannot move to another final state, so
    ///   `accepted`, `superseded`, or `revoked` -> `superseded` is
    ///   refused. The library does not pre-check the old request's
    ///   state, so the server stays the authority on the transition
    ///   and its 403 is surfaced as-is.
    /// - The server does NOT verify that `superseded_by` points at a
    ///   real request — it only checks the number is positive. The
    ///   ordering guarantee (the new request exists first) belongs to
    ///   the caller; [`SrFiling::refile`] provides it.
    /// - Who may supersede: an admin, the request creator, or anyone
    ///   with write permission in the target or the source project.
    ///   Anyone else gets the server's 403.
    /// - The comment sent with the supersede is `superseded by
    ///   <new>`, the form osc sends and the form the live API shows
    ///   on superseded requests.
    /// - Success is the `<status code="ok"/>` body: any other body
    ///   is a server error, never success.
    pub fn supersede_request(&self, old_id: &str, new_id: &str) -> Result<(), GeectlError> {
        let old = validate_request_id(old_id)?;
        let new = validate_request_id(new_id)?;
        if old == new {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("cannot supersede submit request {old_id} with itself"),
            ));
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot supersede a submit request without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(
            &self.base,
            &["request", &old.to_string()],
            &[
                ("cmd", "changestate"),
                ("newstate", "superseded"),
                ("superseded_by", &new.to_string()),
            ],
        );
        // The comment travels as the request body, as plain text:
        // the server's `RequestController#request_command` takes
        // `request.raw_post` as the comment
        // (openSUSE/open-build-service,
        // `src/api/app/controllers/request_controller.rb`), so the body
        // is the comment verbatim and its content type is its actual
        // type (`text/plain`), not the `application/xml` the filing
        // POST sends. The server reads the raw body regardless of
        // content type, so either would reach it — but the header
        // must say what the body is.
        let body = self
            .http
            .post_text(&url, &format!("superseded by {new}"), &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS supersede response for submit request {old}"),
            ))
        }
    }

    /// The request ids in `new`, `review`, or `declined` state that
    /// involve `project`/`package` as a submit source or target.
    /// Used by [`ObsClient::delete_package`]: deleting a package
    /// revokes/declines exactly these requests, so they block the
    /// delete until they are resolved or the caller forces it.
    /// Requires credentials; an unexpected response is an error,
    /// never an empty list.
    pub fn open_requests_for_package(
        &self,
        project: &str,
        package: &str,
    ) -> Result<Vec<String>, GeectlError> {
        let url = self.package_requests_search_url(project, package)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "searching for requests involving a package requires OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_request_ids(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS search response for package {project}/{package}"),
            )
        })
    }

    /// Build the OBS search URL for requests involving a package:
    /// source or target is this project/package, in `new`, `review`,
    /// or `declined` state. The state set is exactly the set the
    /// API's package-delete callbacks touch
    /// (`Package#open_requests_with_package_as_source` /
    /// `#open_requests_with_package_as_target`): a delete revokes
    /// those as source and declines those as target. `superseded`,
    /// `revoked`, and `accepted` requests are already closed and are
    /// not touched by a delete.
    pub fn package_requests_search_url(
        &self,
        project: &str,
        package: &str,
    ) -> Result<String, GeectlError> {
        for (label, value) in [("project", project), ("package", package)] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS {label} name: {value:?}"),
                ));
            }
        }
        let match_expr = format!(
            "(action/source/@project='{project}' and action/source/@package='{package}' or action/target/@project='{project}' and action/target/@package='{package}') and (state/@name='new' or state/@name='review' or state/@name='declined')"
        );
        Ok(build_url(
            &self.base,
            &["search", "request"],
            &[("match", &match_expr)],
        ))
    }

    /// Read submit request `id` (`GET /request/:id`): what it does,
    /// the state it is in, its reviews, and its history. An agent
    /// reads a request before acting on it — to see what it changes,
    /// who filed it, what state it is in, and what has already been
    /// said about it — and after acting on it, to confirm the state
    /// it landed in. Requires credentials; without them the call
    /// fails loudly before any request. A request that does not exist
    /// reads as `NotFound`, and a document the parser does not
    /// recognise in full is a server error, never a partial request.
    ///
    /// The read asks for the history with the request
    /// (`withhistory=1`): the creation entry and the state changes
    /// are part of reading a request, not a separate operation — the
    /// separate operation ([`ObsClient::request_log`]) is for when
    /// the history alone is what the caller wants.
    ///
    /// The operation is the same whether the request's packages are
    /// OBS-native or git-backed: a request is a server object, and
    /// its actions name projects and packages without saying how
    /// their sources are stored, so nothing here branches on the
    /// backend.
    pub fn show_request(&self, id: &str) -> Result<Request, GeectlError> {
        self.fetch_request(id)
    }

    /// Read one request's history (`GET /request/:id` with
    /// `withhistory=1`, returning only the history entries): the
    /// creation entry first, then each state change in order, with
    /// who made it, when, and what it was. An agent reads the log to
    /// audit what happened to a request — who declined it and why,
    /// when it was superseded — without pulling the whole request.
    /// Requires credentials; without them the call fails loudly
    /// before any request. The history the server sends with
    /// `withhistory` is the request history (creation plus state
    /// changes); review history is not included, by the server's own
    /// choice.
    pub fn request_log(&self, id: &str) -> Result<Vec<RequestHistory>, GeectlError> {
        Ok(self.fetch_request(id)?.history)
    }

    /// The shared read behind [`ObsClient::show_request`] and
    /// [`ObsClient::request_log`]: fetch the request document with
    /// its history and parse it in full.
    fn fetch_request(&self, id: &str) -> Result<Request, GeectlError> {
        let id = validate_request_id(id)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot read a submit request without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(
            &self.base,
            &["request", &id.to_string()],
            &[("withhistory", "1")],
        );
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_request(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS request response for submit request {id}"),
            )
        })
    }

    /// Format a retry budget for a probe line: whole seconds as
    /// `30s`, anything else in milliseconds (`1500ms`).
    /// `Duration::as_secs` truncates, so a 1500 ms budget would
    /// print as `1s`.
    pub(crate) fn format_budget(budget: std::time::Duration) -> String {
        let millis = budget.as_millis();
        if millis.is_multiple_of(1000) {
            format!("{}s", millis / 1000)
        } else {
            format!("{millis}ms")
        }
    }

    /// POST the changestate command at `url` with `comment` as the
    /// body, retrying transient failures while `budget` lasts and
    /// checking the request's state before any re-issue: the
    /// server's API replies before its backend has registered a
    /// just-filed request, so the changestate POST that follows a
    /// filing can fail while the request is about to exist (the
    /// accept POST can return 500 for a request the backend has
    /// not registered yet). Only transient
    /// failures retry — a 5xx, a timeout, a network error, or a
    /// not-found (the just-filed request is not visible yet while
    /// the backend registers it, the same race as the package-create
    /// retry). A deterministic failure fails fast instead of
    /// burning the budget — bad credentials, or a malformed id
    /// refused client-side. A well-formed id that does not exist is
    /// different: it 404s, which is in the retry set, so it takes
    /// up to 60 s to fail, the same tradeoff as `create_package`'s
    /// genuinely-missing project. A failed state read is just
    /// "state unknown" —
    /// the same backend that 500s the POST can 500 the follow-up
    /// GET — and the retry keeps going until the deadline, which
    /// still fails loudly. The retry sleeps 500 ms between
    /// attempts, so a request that never changes state still fails
    /// loudly once the budget runs out instead of hanging.
    ///
    /// The state check is what makes this retry safe where the
    /// HTTP layer's POST policy is not: a POST is never retried
    /// there, because a retried mutation can duplicate — but here
    /// the read-back proves whether the change already landed, so
    /// a mutation that went through is never re-issued. The check is
    /// end-state-based, not attribution-based: it proves the request
    /// is in the target state, not that this call's POST put it
    /// there — if another actor moved it first, this call's comment
    /// is never posted, and the request is already where the caller
    /// wanted it. There is a narrow window between the state read
    /// and the re-POST: if another actor moves the request in
    /// between, the re-POST can fail fast (a 403 on an
    /// already-accepted request, for example) even though the request
    /// is in the target state; the window is one sleep wide and the
    /// caller's flow is sequential, so the retry documents it rather
    /// than closing it. A 200 with a non-ok body keeps the
    /// pre-existing fail-fast: the retry covers transport-level
    /// transient failures, and a non-ok body is read as the server
    /// refusing the change. Callers pass the budget;
    /// [`ObsClient::change_state`] passes 60 s.
    fn post_changestate_with_retry(
        &self,
        url: &str,
        comment: &str,
        id: u64,
        state: RequestState,
        budget: std::time::Duration,
    ) -> Result<(), GeectlError> {
        let deadline = std::time::Instant::now() + budget;
        let mut probe = 0;
        loop {
            match self.http.post_text(url, comment, &self.credentials) {
                Ok(body) => {
                    if parse_status_ok(&body) {
                        return Ok(());
                    }
                    return Err(GeectlError::new(
                        ErrorKind::ServerError,
                        format!("unexpected OBS changestate response for submit request {id}"),
                    ));
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        ErrorKind::ServerError
                            | ErrorKind::Timeout
                            | ErrorKind::NetworkError
                            | ErrorKind::NotFound
                    ) =>
                {
                    // The 500 can come after the state change
                    // landed, so check before re-issuing: a request
                    // already in the target state means the change
                    // went through, and the retry returns success
                    // without posting again. The check is a decision
                    // point on the debug channel, and the probe that
                    // carries it reports its number against the
                    // retry budget.
                    match self.show_request(&id.to_string()) {
                        Ok(request) if request.state.name == state => {
                            self.debug.emit(&format!(
                                "decision: request {id} already {}; change landed, not re-issuing",
                                state.as_str()
                            ));
                            return Ok(());
                        }
                        Ok(request) => {
                            probe += 1;
                            self.debug.emit(&format!(
                                "retry: change_state request {id} probe {probe}/{}: still {}",
                                Self::format_budget(budget),
                                request.state.name.as_str()
                            ));
                            self.debug.emit(&format!(
                                "decision: request {id} still {}, re-issuing {}",
                                request.state.name.as_str(),
                                state.as_str()
                            ));
                        }
                        Err(_) => {
                            probe += 1;
                            self.debug.emit(&format!(
                                "retry: change_state request {id} probe {probe}/{}: state unknown",
                                Self::format_budget(budget)
                            ));
                            self.debug.emit(&format!(
                                "decision: request {id} state unknown, re-issuing {}",
                                state.as_str()
                            ));
                        }
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err(err);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Err(err) => return Err(err),
            }
        }
    }

    /// Change submit request `id` to `state` (`POST
    /// /request/:id?cmd=changestate&newstate=<state>`), with
    /// `comment` as the message left with the change. Requires
    /// credentials; without them the call fails loudly before any
    /// request. Takes the caller's [`DryRun`]: in dry-run the change
    /// is recorded in the plan and nothing changes, so a dry-run
    /// caller cannot mutate the server by invoking this directly.
    ///
    /// Semantics, verified against the OBS API server
    /// (`BsRequest#change_state` and
    /// `BsRequestPermissionCheck#cmd_changestate_permissions`) and
    /// the live API — not taken from anywhere else:
    /// - The server does not pre-check here, and neither does the
    ///   library: the server stays the authority on the transition,
    ///   and its 403 is surfaced as-is. A request in `accepted`
    ///   cannot change state at all, and a final state cannot move to
    ///   another final state.
    /// - Accepting requires write permission on the target, and the
    ///   server can forbid self-accept outright (the
    ///   `OBS:CreatorCannotAcceptOwnRequests` attribute, set on the
    ///   target package or project). An agent accepts its own
    ///   request only when the target allows it; the server's 403
    ///   says so when it does not.
    /// - Accepting a request in `review` state requires `force`
    ///   (`force=1`): without it the server refuses. `force` is only
    ///   honoured for accept; it is ignored for the other
    ///   transitions, so the wrappers for decline and revoke do not
    ///   take one.
    /// - `superseded` is refused here: superseding needs a
    ///   replacement id, and [`ObsClient::supersede_request`] is the
    ///   path that carries it. `deleted` is refused here as well:
    ///   deleting a request is an admin-only server operation, and
    ///   revoking is the agent's way to withdraw one.
    /// - Re-opening a request to `new` or `review` resets its
    ///   non-accepted reviews to `new` and forces the state to
    ///   `review` when reviews are still open; that is the server's
    ///   behaviour, not the library's.
    /// - Success is the `<status code="ok"/>` body: any other body is
    ///   a server error, never success.
    /// - The POST retries transient failures for a bounded 60 s:
    ///   the server's API replies before its backend has
    ///   registered a just-filed request, so the changestate POST
    ///   that follows a filing can fail while the request is about
    ///   to exist (the accept POST can return 500 for a request
    ///   the backend has not registered yet). Before any re-issue
    ///   the retry reads the
    ///   request back, so a change that already landed is never
    ///   posted again; a request that never changes state still
    ///   fails loudly once the budget runs out. Only transient
    ///   failures retry — anything else fails fast, except a
    ///   well-formed id that does not exist, which 404s and retries
    ///   for up to 60 s (the same tradeoff as a genuinely-missing
    ///   project for `create_package`).
    pub fn change_state(
        &self,
        id: &str,
        state: RequestState,
        comment: &str,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        let id = validate_request_id(id)?;
        let state_name = match state {
            RequestState::New
            | RequestState::Review
            | RequestState::Accepted
            | RequestState::Declined
            | RequestState::Revoked => state.as_str(),
            RequestState::Superseded => {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "superseding a submit request needs a replacement id; use supersede_request",
                ));
            }
            RequestState::Deleted => {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "deleting a submit request is an admin-only server operation; revoke it instead",
                ));
            }
        };
        if !dry_run.allow(&format!(
            "change state of submit request {id} to {state_name}"
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot change a submit request's state without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let mut query: Vec<(&str, &str)> = vec![("cmd", "changestate"), ("newstate", state_name)];
        if force {
            query.push(("force", "1"));
        }
        let url = build_url(&self.base, &["request", &id.to_string()], &query);
        // The comment travels as the request body, as plain text:
        // the server's `RequestController#request_command` takes
        // `request.raw_post` as the comment (see
        // `supersede_request`), so the body is the comment verbatim
        // and its content type is its actual type (`text/plain`).
        self.post_changestate_with_retry(
            &url,
            comment,
            id,
            state,
            std::time::Duration::from_secs(60),
        )
    }

    /// Accept submit request `id` (`POST
    /// /request/:id?cmd=changestate&newstate=accepted`), with
    /// `comment` as the message left with the accept. An agent
    /// accepts a request it reviewed and agrees with — the change
    /// lands, and the source branch dies on accept when the request
    /// was filed with cleanup (the default). `force` is for
    /// accepting a request that is still in `review` state: the
    /// server refuses that without `force=1`, so passing it is a
    /// deliberate override of the open reviews, not a default. See
    /// [`ObsClient::change_state`] for the semantics.
    pub fn accept_request(
        &self,
        id: &str,
        comment: &str,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        self.change_state(id, RequestState::Accepted, comment, force, dry_run)
    }

    /// Decline submit request `id` (`POST
    /// /request/:id?cmd=changestate&newstate=declined`), with
    /// `comment` as the reason. An agent declines a request it
    /// reviewed and disagrees with; the reason is the review, so a
    /// decline without one is possible but the caller should say
    /// why. See [`ObsClient::change_state`] for the semantics.
    pub fn decline_request(
        &self,
        id: &str,
        comment: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        self.change_state(id, RequestState::Declined, comment, false, dry_run)
    }

    /// Revoke submit request `id` (`POST
    /// /request/:id?cmd=changestate&newstate=revoked`), with
    /// `comment` as the reason. Revoking is how a request is
    /// withdrawn — by its creator, or by anyone the server allows to
    /// withdraw it. See [`ObsClient::change_state`] for the semantics.
    pub fn revoke_request(
        &self,
        id: &str,
        comment: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        self.change_state(id, RequestState::Revoked, comment, false, dry_run)
    }

    /// Best-effort [`ObsClient::change_state`]: try the state change,
    /// and return whether it happened. A request-side or transport
    /// failure — the request is already closed or does not exist, the
    /// server is unreachable or errored — reports the failure on the
    /// debug channel (gated on `--debug`) and propagates the error,
    /// so the decline reason reaches the payload instead of hiding
    /// behind a bare `false`. An authentication failure does not get
    /// swallowed: it almost always means
    /// misconfiguration (wrong or expired credentials), not a missing
    /// request, so it propagates — a batch with bad credentials fails
    /// loudly instead of reporting all-false. The one exception is a
    /// request already in the target state: the server answers a
    /// changestate on such a request with the same 403 as a genuine
    /// auth failure, so the status alone cannot tell them apart and
    /// this reads the request back — already in the target state
    /// reports `false` (the change is a no-op), while a request in
    /// another state, or a read that fails itself (bad credentials
    /// fail the read too), propagates the auth failure. A caller bug
    /// does not get swallowed either: a malformed id or state, and
    /// missing credentials ([`ErrorKind::ConfigError`]), still return
    /// an error. In dry-run the change is recorded in the plan (as
    /// [`ObsClient::change_state`] does) and this returns `true`.
    pub fn try_change_state(
        &self,
        id: &str,
        state: RequestState,
        comment: &str,
        force: bool,
        dry_run: &DryRun,
    ) -> Result<bool, GeectlError> {
        match self.change_state(id, state, comment, force, dry_run) {
            Ok(()) => Ok(true),
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::NotFound
                        | ErrorKind::ServerError
                        | ErrorKind::Timeout
                        | ErrorKind::NetworkError
                        | ErrorKind::TlsError
                ) =>
            {
                self.debug.emit(&format!(
                    "decision: could not change state of submit request {id}: {err}"
                ));
                Err(err)
            }
            Err(err) if err.kind() == ErrorKind::AuthFailed => {
                // The server answers a changestate on a request
                // already in the target state with the same 403 as a
                // genuine auth failure, so the status alone cannot
                // tell them apart: read the request back. Already in
                // the target state means the change is a no-op and
                // this reports false; a request in another state, or
                // a read that fails itself (bad credentials fail the
                // read too), means the 403 was real and the auth
                // failure propagates.
                match self.show_request(id) {
                    Ok(request) if request.state.name == state => {
                        self.debug.emit(&format!(
                            "decision: submit request {id} already {}; not re-issuing",
                            state.as_str()
                        ));
                        Ok(false)
                    }
                    _ => Err(err),
                }
            }
            Err(err) => Err(err),
        }
    }

    /// Ask `target` to review submit request `id` (`POST
    /// /request/:id?cmd=addreview`), with `comment` as the reason the
    /// review is asked for. The review moves the request to `review`
    /// state. An agent asks for a review when a request needs eyes it
    /// does not have — a maintainer of the target, a group that owns
    /// the area — and the reason is the ask, so an empty comment is
    /// refused: a review that says nothing about what it wants is not
    /// a review. Requires credentials; without them the call fails
    /// loudly before any request. Takes the caller's [`DryRun`]: in
    /// dry-run the review is recorded in the plan and nothing is
    /// asked.
    ///
    /// The target is validated up front ([`ReviewTarget::new`]): at
    /// least one of a user, a group, and a project must be named, and
    /// a package review needs its project. Success is the `<status
    /// code="ok"/>` body: any other body is a server error, never
    /// success.
    pub fn add_review(
        &self,
        id: &str,
        target: &ReviewTarget,
        comment: &str,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        let id = validate_request_id(id)?;
        if comment.is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "a review needs a reason; an empty comment asks for a review that says nothing",
            ));
        }
        if !dry_run.allow(&format!(
            "add a review by {} to submit request {id}",
            target.describe()
        )) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot add a review to a submit request without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let mut query: Vec<(&str, &str)> = vec![("cmd", "addreview")];
        if let Some(by_user) = &target.by_user {
            query.push(("by_user", by_user));
        }
        if let Some(by_group) = &target.by_group {
            query.push(("by_group", by_group));
        }
        if let Some(by_project) = &target.by_project {
            query.push(("by_project", by_project));
        }
        if let Some(by_package) = &target.by_package {
            query.push(("by_package", by_package));
        }
        let url = build_url(&self.base, &["request", &id.to_string()], &query);
        // The reason travels as the request body, as plain text:
        // the server's `RequestController#request_command` takes
        // `request.raw_post` as the comment (see
        // `supersede_request`), so the body is the reason verbatim
        // and its content type is its actual type (`text/plain`).
        let body = self.http.post_text(&url, comment, &self.credentials)?;
        if parse_status_ok(&body) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS addreview response for submit request {id}"),
            ))
        }
    }

    /// List the comments on submit request `id` (`GET
    /// /comments/request/:id`): who said what, when, and which
    /// comment each reply answers. The list covers the comments on
    /// the request and the comments on its actions, in id order.
    /// Requires credentials; without them the call fails loudly
    /// before any request. A request with no comments reads as an
    /// empty list, not an error; a document the parser does not
    /// recognise in full is a server error.
    pub fn list_comments(&self, id: &str) -> Result<Vec<RequestComment>, GeectlError> {
        let id = validate_request_id(id)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot list a submit request's comments without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let url = build_url(&self.base, &["comments", "request", &id.to_string()], &[]);
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_comments(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS comments response for submit request {id}"),
            )
        })
    }

    /// Comment on submit request `id` (`POST
    /// /comments/request/:id`), with `parent_id` naming the comment
    /// being replied to, when this is a reply. An agent comments to
    /// discuss a request — asking a question, leaving a note for the
    /// reviewer — without changing its state. The comment text
    /// travels as the request body, as plain text; an empty comment
    /// is refused, since a comment that says nothing is not a
    /// comment. Requires credentials; without them the call fails
    /// loudly before any request. Takes the caller's [`DryRun`]: in
    /// dry-run the comment is recorded in the plan and nothing is
    /// posted. Success is the `<status code="ok"/>` body: any other
    /// body is a server error, never success.
    pub fn add_comment(
        &self,
        id: &str,
        body: &str,
        parent_id: Option<&str>,
        dry_run: &DryRun,
    ) -> Result<(), GeectlError> {
        let id = validate_request_id(id)?;
        // The parent is a comment id: the same positive-number shape
        // as a request id, so the same check applies.
        let parent = match parent_id {
            Some(parent) => Some(validate_request_id(parent)?),
            None => None,
        };
        if body.is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "a comment with no text says nothing; refusing an empty comment",
            ));
        }
        if !dry_run.allow(&format!("comment on submit request {id}")) {
            return Ok(());
        }
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot comment on a submit request without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let parent_string;
        let mut query: Vec<(&str, &str)> = Vec::new();
        if let Some(parent) = parent {
            parent_string = parent.to_string();
            query.push(("parent_id", &parent_string));
        }
        let url = build_url(
            &self.base,
            &["comments", "request", &id.to_string()],
            &query,
        );
        let response = self.http.post_text(&url, body, &self.credentials)?;
        if parse_status_ok(&response) {
            Ok(())
        } else {
            Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unexpected OBS comment response for submit request {id}"),
            ))
        }
    }

    /// Read the diff of submit request `id` (`POST
    /// /request/:id?cmd=diff`): what the request changes, as a
    /// unified diff — or, with `view_xml`, as the request XML with
    /// the diff injected into it. An agent reads the diff to review
    /// a request before acting on it: accepting or declining without
    /// seeing the change is not a review. Requires credentials;
    /// without them the call fails loudly before any request. The
    /// diff is returned as the raw response body, unparsed: it is
    /// caller-consumed text, not a structure the library acts on.
    ///
    /// The diff is fetched with a POST because that is the only way
    /// the API offers it — but it changes nothing on the server, so
    /// it takes no [`DryRun`]: a dry-run caller reading a diff is
    /// still only reading.
    pub fn request_diff(&self, id: &str, view_xml: bool) -> Result<String, GeectlError> {
        let id = validate_request_id(id)?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot read a submit request's diff without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let mut query: Vec<(&str, &str)> = vec![("cmd", "diff")];
        if view_xml {
            query.push(("view", "xml"));
        }
        let url = build_url(&self.base, &["request", &id.to_string()], &query);
        self.http.post_text(&url, "", &self.credentials)
    }

    /// List the submit requests matching `search` (`GET
    /// /search/request`): the typed list-my-requests operation. An
    /// agent lists requests to triage — its own open requests, the
    /// open requests targeting a package, the requests waiting on a
    /// reviewer — and each result is a whole [`Request`], parsed in
    /// full, so the caller can filter further without another round
    /// trip. Requires credentials; without them the call fails loudly
    /// before any request. An unexpected response is an error, never
    /// an empty list.
    pub fn search_requests(&self, search: &RequestSearch) -> Result<Vec<Request>, GeectlError> {
        let match_expr = search.match_expr()?;
        if !self.has_basic() {
            return Err(GeectlError::new(
                ErrorKind::ConfigError,
                "cannot search submit requests without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
            ));
        }
        let mut query: Vec<(&str, &str)> = vec![("match", &match_expr)];
        if search.wants_history() {
            query.push(("withhistory", "1"));
        }
        if search.wants_full_history() {
            query.push(("withfullhistory", "1"));
        }
        let url = build_url(&self.base, &["search", "request"], &query);
        let body = self.http.get_with(&url, &self.credentials)?;
        parse_request_collection(&body).ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                "unexpected OBS request search response",
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::Debug;
    use crate::error::ErrorKind;
    use crate::obs::test_support::{basic_credentials, payload, search_without_states, FakeHttp};

    #[test]
    fn has_open_sr_detects_request() {
        // The search URL is built by the client; can it with the
        // expected response.
        let http_for_url = FakeHttp::new();
        let client = ObsClient::with_credentials(&http_for_url, basic_credentials()).unwrap();
        let url = client.search_url("foo").unwrap();
        let http = FakeHttp::new().with_get(
            &url,
            "<collection matches=\"1\"><request id=\"1398492\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.has_open_sr("foo").unwrap());
        // The search carried the credentials.
        assert_eq!(http.get_calls().len(), 1);
        assert!(http.get_calls()[0].1);
    }

    #[test]
    fn has_open_sr_ignores_non_submit_requests() {
        // The search match does not filter by action type (the
        // filter lives client-side), so a delete request targeting
        // the package must not read as an open submit request.
        let http_for_url = FakeHttp::new();
        let client = ObsClient::with_credentials(&http_for_url, basic_credentials()).unwrap();
        let url = client.search_url("foo").unwrap();
        let http = FakeHttp::new().with_get(
            &url,
            "<collection matches=\"1\"><request id=\"1398492\" creator=\"scarabeus_iv\"><action type=\"delete\"><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(!client.has_open_sr("foo").unwrap());
    }

    #[test]
    fn has_open_sr_empty_is_false() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let url = client.search_url("foo").unwrap();
        let http = FakeHttp::new().with_get(&url, "<collection matches=\"0\"></collection>");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(!client.has_open_sr("foo").unwrap());
    }

    #[test]
    fn has_open_sr_unexpected_response_is_error_not_false() {
        // A response with no matches count is a failed lookup: it must
        // never read as "no open SR".
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let url = client.search_url("foo").unwrap();
        let http = FakeHttp::new().with_get(&url, "<collection></collection>");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client.has_open_sr("foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn has_open_sr_without_credentials_fails_closed() {
        // No credentials: refused before any request, not made and 401'd.
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client.has_open_sr("foo").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn create_request_posts_payload_and_returns_id() {
        let payload = payload();
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request?cmd=create",
            "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let id = client.create_request(&payload).unwrap();
        assert_eq!(id, "1400001");
        let posted = http.post_calls();
        assert_eq!(posted.len(), 1);
        assert_eq!(posted[0].0, "https://api.opensuse.org/request?cmd=create");
        // The posted body is the payload's own rendering: the typed
        // path is the only path, and no XML is ever handed in.
        assert_eq!(posted[0].1, payload.to_xml());
        // The filing POST carried the credentials.
        assert!(posted[0].2);
    }

    #[test]
    fn create_request_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request?cmd=create",
            "<request id=\"1400001\"></request>",
        );
        let client = ObsClient::new(&http);
        assert!(client.create_request(&payload()).is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn create_request_transport_error_fails() {
        // No canned POST response: the transport fails, so filing fails
        // instead of silently reporting success.
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.create_request(&payload()).is_err());
    }

    #[test]
    fn to_xml_renders_submit_action() {
        let xml = payload().to_xml();
        assert!(xml.contains("<request>"));
        assert!(xml.contains("<action type=\"submit\">"));
        assert!(xml.contains("<source project=\"home:scarabeus_iv\" package=\"foo\" />"));
        assert!(xml.contains("<target project=\"devel:languages:python\" package=\"foo\" />"));
        assert!(xml.contains("<options><sourceupdate>cleanup</sourceupdate></options>"));
        assert!(xml.contains("<description>Update foo to 2.0</description>"));
        assert!(xml.contains("</request>"));
    }

    #[test]
    fn to_xml_escapes_description() {
        let p = SrPayload::new(
            "home:scarabeus_iv",
            "foo",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0 & fix <bugs>",
            "- Update to version 2.0.\n",
            &[],
        )
        .unwrap();
        let xml = p.to_xml();
        assert!(xml.contains("Update foo to 2.0 &amp; fix &lt;bugs&gt;"));
        assert!(!xml.contains("fix <bugs>"));
        assert!(p.validate_xml().is_ok());
    }

    #[test]
    fn validate_request_id_accepts_and_rejects() {
        assert_eq!(validate_request_id("1398492").unwrap(), 1398492);
        // A leading-zero spelling is the same request, canonicalized.
        assert_eq!(validate_request_id("01398492").unwrap(), 1398492);
        assert!(validate_request_id("").is_err());
        assert!(validate_request_id("abc").is_err());
        assert!(validate_request_id("13984a2").is_err());
        assert!(validate_request_id("0").is_err());
        // Overflow: more digits than a u64 holds.
        assert!(validate_request_id("99999999999999999999999").is_err());
    }

    #[test]
    fn supersede_request_posts_changestate_with_superseded_by() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1397897",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.supersede_request("1397357", "1397897").is_ok());
        let posted = http.post_calls();
        assert_eq!(posted.len(), 1);
        assert_eq!(
            posted[0].0,
            "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1397897"
        );
        // The comment is the form osc sends and the live API shows.
        assert_eq!(posted[0].1, "superseded by 1397897");
        assert!(posted[0].2);
    }

    #[test]
    fn supersede_request_rejects_invalid_ids_without_http() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        for (old, new) in [
            ("", "1397897"),
            ("1397357", ""),
            ("abc", "1397897"),
            ("1397357", "def"),
            ("0", "1397897"),
            ("1397357", "1397357"),
        ] {
            assert!(client.supersede_request(old, new).is_err());
        }
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn supersede_request_without_credentials_fails_closed() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1397897",
            "<status code=\"ok\" />",
        );
        let client = ObsClient::new(&http);
        assert!(client.supersede_request("1397357", "1397897").is_err());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn supersede_request_non_ok_body_is_error_not_success() {
        // The server answered 2xx but not ok: a failed supersede must
        // never read as success.
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1397897",
            "<status code=\"not_found\"><summary>no such request</summary></status>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client.supersede_request("1397357", "1397897").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn supersede_request_transport_failure_is_error() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client.supersede_request("1397357", "1397897").is_err());
    }

    #[test]
    fn package_requests_search_url_matches_source_or_target() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        assert_eq!(
            client
                .package_requests_search_url("home:scarabeus_iv", "foo")
                .unwrap(),
            "https://api.opensuse.org/search/request?match=%28action%2Fsource%2F%40project%3D%27home%3Ascarabeus_iv%27%20and%20action%2Fsource%2F%40package%3D%27foo%27%20or%20action%2Ftarget%2F%40project%3D%27home%3Ascarabeus_iv%27%20and%20action%2Ftarget%2F%40package%3D%27foo%27%29%20and%20%28state%2F%40name%3D%27new%27%20or%20state%2F%40name%3D%27review%27%20or%20state%2F%40name%3D%27declined%27%29"
        );
        // Invalid names are rejected before any request.
        assert!(client
            .package_requests_search_url("home:foo", "foo/bar")
            .is_err());
        assert!(client.package_requests_search_url("", "foo").is_err());
    }

    #[test]
    fn open_requests_for_package_returns_ids() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let url = client
            .package_requests_search_url("home:scarabeus_iv", "foo")
            .unwrap();
        let http = FakeHttp::new().with_get(
            &url,
            "<collection matches=\"1\"><request id=\"1398492\" /></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .open_requests_for_package("home:scarabeus_iv", "foo")
                .unwrap(),
            vec!["1398492".to_string()]
        );
        assert_eq!(http.get_calls().len(), 1);
        assert!(http.get_calls()[0].1);
    }

    #[test]
    fn open_requests_for_package_empty_is_empty() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let url = client
            .package_requests_search_url("home:scarabeus_iv", "foo")
            .unwrap();
        let http = FakeHttp::new().with_get(&url, "<collection matches=\"0\"></collection>");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(client
            .open_requests_for_package("home:scarabeus_iv", "foo")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn open_requests_for_package_unexpected_response_is_error() {
        let http = FakeHttp::new();
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let url = client
            .package_requests_search_url("home:scarabeus_iv", "foo")
            .unwrap();
        let http = FakeHttp::new().with_get(&url, "<request id=\"1398492\" />");
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = client
            .open_requests_for_package("home:scarabeus_iv", "foo")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn open_requests_for_package_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client
            .open_requests_for_package("home:scarabeus_iv", "foo")
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    const REQUEST1_XML: &str = r#"<request creator="creator" id="42">
  <action type="submit">
    <source package="bar" project="foo" rev="1" />
    <target package="bar" project="foobar" />
  </action>
  <action type="delete">
    <target project="deleteme" />
  </action>
  <state name="accepted" when="2010-12-27T01:36:29" who="user1" />
  <history when="2010-12-13T13:02:03" who="creator">
    <description>Create Request</description>
    <comment>foobar</comment>
  </history>
  <title>title of the request</title>
  <description>this is a
very long
description</description>
</request>"#;

    const REQUEST2_XML: &str = r#"<request creator="creator" id="123">
  <action type="submit">
    <source package="abc" project="xyz" />
    <options>
      <sourceupdate>cleanup</sourceupdate>
      <updatelink>1</updatelink>
    </options>
  </action>
  <action type="add_role">
    <target project="home:foo" />
    <person name="bar" role="maintainer" />
    <group name="groupxyz" role="reader" />
  </action>
  <state approver="someone" name="review" when="2010-12-27T01:36:29" who="abc" />
  <review by_group="group1" state="new" when="2010-12-28T00:11:22" who="abc">
    <comment>review start</comment>
  </review>
  <history when="2010-12-11T00:00:00" who="creator">
    <description>Created request</description>
  </history>
</request>"#;

    const LIST_VIEW1_XML: &str = r#"<request creator="Admin" id="62">
  <action type="set_bugowner">
    <target project="foo" />
    <person name="buguser" />
  </action>
  <action type="add_role">
    <target project="foobar" />
    <person name="xyz" role="maintainer" />
    <group name="group1" role="reader" />
  </action>
  <action type="add_role">
    <target project="foo" package="bar" />
    <person name="abc" role="reviewer" />
  </action>
  <action type="change_devel">
    <source project="devprj" package="devpkg" />
    <target project="foo" package="bar" />
  </action>
  <action type="submit">
    <source project="srcprj" package="srcpackage" />
    <target project="tgtprj" package="tgtpackage" />
  </action>
  <action type="submit">
    <source project="foo" package="bar" />
    <target project="baz" />
  </action>
  <action type="delete">
    <target project="deleteme" />
  </action>
  <action type="delete">
    <target project="foo" package="bar" />
  </action>
  <state name="new" when="2010-12-29T14:57:25" who="Admin">
    <comment></comment>
  </state>
</request>"#;

    /// A minimal request carrying one action, for the construction
    /// tests: id 42, creator `creator`, state `new`.
    fn request_with_action(action: RequestAction) -> Request {
        Request {
            id: 42,
            creator: "creator".to_string(),
            title: None,
            description: String::new(),
            actions: vec![action],
            priority: None,
            state: RequestStateInfo {
                name: RequestState::New,
                who: "creator".to_string(),
                when: "2010-12-13T13:02:03".to_string(),
                created: None,
                superseded_by: None,
                approver: None,
                comment: String::new(),
            },
            reviews: Vec::new(),
            history: Vec::new(),
            accept_at: None,
        }
    }

    /// A request document carrying one action fragment, for the
    /// action-parsing tests.
    fn request_document_with_action(action_xml: &str) -> String {
        format!(
            "<request creator=\"creator\" id=\"42\">\n  {action_xml}\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n</request>"
        )
    }

    #[test]
    fn request_state_names_round_trip() {
        for (name, state) in [
            ("new", RequestState::New),
            ("review", RequestState::Review),
            ("accepted", RequestState::Accepted),
            ("declined", RequestState::Declined),
            ("revoked", RequestState::Revoked),
            ("superseded", RequestState::Superseded),
            ("deleted", RequestState::Deleted),
        ] {
            assert_eq!(state.as_str(), name);
            assert_eq!(RequestState::parse(name), Some(state));
        }
        assert_eq!(RequestState::parse("exploded"), None);
    }

    #[test]
    fn request_priority_names_round_trip() {
        for (name, priority) in [
            ("low", RequestPriority::Low),
            ("moderate", RequestPriority::Moderate),
            ("important", RequestPriority::Important),
            ("critical", RequestPriority::Critical),
        ] {
            assert_eq!(priority.as_str(), name);
            assert_eq!(RequestPriority::parse(name), Some(priority));
        }
        assert_eq!(RequestPriority::parse("urgent"), None);
    }

    #[test]
    fn request_createsr_serialises_a_submit_action() {
        let action = RequestAction::Submit {
            source_project: "foo".to_string(),
            source_package: "bar".to_string(),
            source_rev: Some("42".to_string()),
            target_project: Some("foobar".to_string()),
            target_package: Some("bar".to_string()),
            sourceupdate: None,
            updatelink: false,
            makeoriginolder: false,
            acceptinfo: None,
        };
        assert_eq!(action.type_name(), "submit");
        let request = request_with_action(action);
        assert_eq!(
            request.to_xml(),
            "<request creator=\"creator\" id=\"42\">\n  <action type=\"submit\">\n    <source project=\"foo\" package=\"bar\" rev=\"42\" />\n    <target project=\"foobar\" package=\"bar\" />\n  </action>\n  <state name=\"new\" who=\"creator\" when=\"2010-12-13T13:02:03\" />\n</request>"
        );
        // The serialised form parses back to the same request.
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_createsr_with_option_serialises_options() {
        let action = RequestAction::Submit {
            source_project: "foo".to_string(),
            source_package: "bar".to_string(),
            source_rev: None,
            target_project: Some("foobar".to_string()),
            target_package: Some("bar".to_string()),
            sourceupdate: Some(SourceUpdate::Cleanup),
            updatelink: true,
            makeoriginolder: false,
            acceptinfo: None,
        };
        let request = request_with_action(action);
        assert_eq!(
            request.to_xml(),
            "<request creator=\"creator\" id=\"42\">\n  <action type=\"submit\">\n    <source project=\"foo\" package=\"bar\" />\n    <target project=\"foobar\" package=\"bar\" />\n    <options>\n      <sourceupdate>cleanup</sourceupdate>\n      <updatelink>true</updatelink>\n    </options>\n  </action>\n  <state name=\"new\" who=\"creator\" when=\"2010-12-13T13:02:03\" />\n</request>"
        );
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_createsr_missing_tgt_package_serialises_project_only() {
        let action = RequestAction::Submit {
            source_project: "foo".to_string(),
            source_package: "bar".to_string(),
            source_rev: None,
            target_project: Some("foobar".to_string()),
            target_package: None,
            sourceupdate: None,
            updatelink: false,
            makeoriginolder: false,
            acceptinfo: None,
        };
        let request = request_with_action(action);
        assert!(request.to_xml().contains("<target project=\"foobar\" />"));
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_create_add_role_person_serialises() {
        let action = RequestAction::AddRole {
            target_project: "foo".to_string(),
            target_package: Some("bar".to_string()),
            person: Some(("user".to_string(), "reader".to_string())),
            group: None,
        };
        let request = request_with_action(action);
        assert_eq!(
            request.to_xml(),
            "<request creator=\"creator\" id=\"42\">\n  <action type=\"add_role\">\n    <target project=\"foo\" package=\"bar\" />\n    <person name=\"user\" role=\"reader\" />\n  </action>\n  <state name=\"new\" who=\"creator\" when=\"2010-12-13T13:02:03\" />\n</request>"
        );
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_create_add_role_group_serialises() {
        let action = RequestAction::AddRole {
            target_project: "foo".to_string(),
            target_package: Some("bar".to_string()),
            person: None,
            group: Some(("group".to_string(), "reviewer".to_string())),
        };
        let request = request_with_action(action);
        assert!(request
            .to_xml()
            .contains("<group name=\"group\" role=\"reviewer\" />"));
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_create_add_role_person_group_serialises_both() {
        let action = RequestAction::AddRole {
            target_project: "foo".to_string(),
            target_package: Some("bar".to_string()),
            person: Some(("user".to_string(), "reader".to_string())),
            group: Some(("group".to_string(), "reviewer".to_string())),
        };
        let request = request_with_action(action);
        let xml = request.to_xml();
        assert!(xml.contains("<person name=\"user\" role=\"reader\" />"));
        assert!(xml.contains("<group name=\"group\" role=\"reviewer\" />"));
        assert_eq!(parse_request(&xml), Some(request));
    }

    #[test]
    fn request_create_set_bugowner_project_serialises() {
        let action = RequestAction::SetBugowner {
            target_project: "foobar".to_string(),
            target_package: None,
            person_name: Some("buguser".to_string()),
            group_name: None,
        };
        let request = request_with_action(action);
        assert_eq!(
            request.to_xml(),
            "<request creator=\"creator\" id=\"42\">\n  <action type=\"set_bugowner\">\n    <target project=\"foobar\" />\n    <person name=\"buguser\" />\n  </action>\n  <state name=\"new\" who=\"creator\" when=\"2010-12-13T13:02:03\" />\n</request>"
        );
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_create_set_bugowner_package_serialises() {
        let action = RequestAction::SetBugowner {
            target_project: "foobar".to_string(),
            target_package: Some("baz".to_string()),
            person_name: Some("buguser".to_string()),
            group_name: None,
        };
        let request = request_with_action(action);
        assert!(request
            .to_xml()
            .contains("<target project=\"foobar\" package=\"baz\" />"));
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_create_delete_project_serialises() {
        let action = RequestAction::Delete {
            target_project: "foo".to_string(),
            target_package: None,
            target_repository: None,
        };
        let request = request_with_action(action);
        assert!(request.to_xml().contains("<target project=\"foo\" />"));
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_create_delete_package_serialises() {
        let action = RequestAction::Delete {
            target_project: "foo".to_string(),
            target_package: Some("deleteme".to_string()),
            target_repository: None,
        };
        let request = request_with_action(action);
        assert!(request
            .to_xml()
            .contains("<target project=\"foo\" package=\"deleteme\" />"));
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_create_change_devel_serialises() {
        let action = RequestAction::ChangeDevel {
            source_project: "foo".to_string(),
            source_package: "bar".to_string(),
            target_project: "devprj".to_string(),
            target_package: "devpkg".to_string(),
        };
        let request = request_with_action(action);
        assert_eq!(
            request.to_xml(),
            "<request creator=\"creator\" id=\"42\">\n  <action type=\"change_devel\">\n    <source project=\"foo\" package=\"bar\" />\n    <target project=\"devprj\" package=\"devpkg\" />\n  </action>\n  <state name=\"new\" who=\"creator\" when=\"2010-12-13T13:02:03\" />\n</request>"
        );
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_action_from_xml_add_role_parses_and_round_trips() {
        let xml = request_document_with_action(
            "<action type=\"add_role\">\n  <target package=\"bar\" project=\"foo\" />\n  <person name=\"user\" role=\"reader\" />\n  <group name=\"group\" role=\"reviewer\" />\n</action>",
        );
        let request = parse_request(&xml).expect("add_role parses");
        assert_eq!(
            request.actions[0],
            RequestAction::AddRole {
                target_project: "foo".to_string(),
                target_package: Some("bar".to_string()),
                person: Some(("user".to_string(), "reader".to_string())),
                group: Some(("group".to_string(), "reviewer".to_string())),
            }
        );
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_action_from_xml_submit_with_options_parses_and_round_trips() {
        let xml = request_document_with_action(
            "<action type=\"submit\">\n  <source package=\"bar\" project=\"foo\" />\n  <target package=\"bar\" project=\"foobar\" />\n  <options>\n    <sourceupdate>cleanup</sourceupdate>\n    <updatelink>1</updatelink>\n  </options>\n</action>",
        );
        let request = parse_request(&xml).expect("submit with options parses");
        assert_eq!(
            request.actions[0],
            RequestAction::Submit {
                source_project: "foo".to_string(),
                source_package: "bar".to_string(),
                source_rev: None,
                target_project: Some("foobar".to_string()),
                target_package: Some("bar".to_string()),
                sourceupdate: Some(SourceUpdate::Cleanup),
                updatelink: true,
                makeoriginolder: false,
                acceptinfo: None,
            }
        );
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_action_from_xml_submit_with_acceptinfo_parses_and_round_trips() {
        let xml = request_document_with_action(
            "<action type=\"submit\">\n  <source package=\"bar\" project=\"testprj\" />\n  <target package=\"baz\" project=\"foobar\" />\n  <acceptinfo rev=\"5\" srcmd5=\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\" xsrcmd5=\"ffffffffffffffffffffffffffffffff\" />\n</action>",
        );
        let request = parse_request(&xml).expect("submit with acceptinfo parses");
        assert_eq!(
            request.actions[0],
            RequestAction::Submit {
                source_project: "testprj".to_string(),
                source_package: "bar".to_string(),
                source_rev: None,
                target_project: Some("foobar".to_string()),
                target_package: Some("baz".to_string()),
                sourceupdate: None,
                updatelink: false,
                makeoriginolder: false,
                acceptinfo: Some(AcceptInfo {
                    rev: "5".to_string(),
                    srcmd5: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                    oproject: None,
                    opackage: None,
                    osrcmd5: None,
                    xsrcmd5: Some("ffffffffffffffffffffffffffffffff".to_string()),
                    oxsrcmd5: None,
                }),
            }
        );
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_parse_rejects_unknown_action_type() {
        // An unknown action type at construction is unrepresentable in
        // the typed model, and an unknown action kwarg cannot be
        // passed to a typed constructor at all — so the parse refusal
        // is the adoption of both invalid-argument cases: a document
        // carrying an action the library does not know fails whole.
        let xml = request_document_with_action(
            "<action type=\"foo\"><source package=\"bar\" project=\"foo\" /></action>",
        );
        assert_eq!(parse_request(&xml), None);
    }

    #[test]
    fn request_read_request1_parses_and_round_trips() {
        let request = parse_request(REQUEST1_XML).expect("request 1 parses");
        assert_eq!(request.id, 42);
        assert_eq!(request.creator, "creator");
        assert_eq!(
            request.actions[0],
            RequestAction::Submit {
                source_project: "foo".to_string(),
                source_package: "bar".to_string(),
                source_rev: Some("1".to_string()),
                target_project: Some("foobar".to_string()),
                target_package: Some("bar".to_string()),
                sourceupdate: None,
                updatelink: false,
                makeoriginolder: false,
                acceptinfo: None,
            }
        );
        assert_eq!(
            request.actions[1],
            RequestAction::Delete {
                target_project: "deleteme".to_string(),
                target_package: None,
                target_repository: None,
            }
        );
        assert_eq!(request.state.name, RequestState::Accepted);
        assert_eq!(request.state.when, "2010-12-27T01:36:29");
        assert_eq!(request.state.who, "user1");
        assert_eq!(request.state.approver, None);
        assert_eq!(request.state.comment, "");
        assert_eq!(request.history.len(), 1);
        assert_eq!(request.history[0].when, "2010-12-13T13:02:03");
        assert_eq!(request.history[0].who, "creator");
        assert_eq!(request.history[0].description, "Create Request");
        assert_eq!(request.history[0].comment, "foobar");
        assert_eq!(request.title.as_deref(), Some("title of the request"));
        assert_eq!(request.description, "this is a\nvery long\ndescription");
        assert!(request.reviews.is_empty());
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_read_request2_parses_and_round_trips() {
        let request = parse_request(REQUEST2_XML).expect("request 2 parses");
        assert_eq!(request.id, 123);
        assert_eq!(request.creator, "creator");
        // The submit action carries no target element at all.
        assert_eq!(
            request.actions[0],
            RequestAction::Submit {
                source_project: "xyz".to_string(),
                source_package: "abc".to_string(),
                source_rev: None,
                target_project: None,
                target_package: None,
                sourceupdate: Some(SourceUpdate::Cleanup),
                updatelink: true,
                makeoriginolder: false,
                acceptinfo: None,
            }
        );
        assert_eq!(
            request.actions[1],
            RequestAction::AddRole {
                target_project: "home:foo".to_string(),
                target_package: None,
                person: Some(("bar".to_string(), "maintainer".to_string())),
                group: Some(("groupxyz".to_string(), "reader".to_string())),
            }
        );
        assert_eq!(request.state.name, RequestState::Review);
        assert_eq!(request.state.when, "2010-12-27T01:36:29");
        assert_eq!(request.state.approver.as_deref(), Some("someone"));
        assert_eq!(request.state.who, "abc");
        assert_eq!(request.state.comment, "");
        assert_eq!(request.reviews.len(), 1);
        assert_eq!(request.reviews[0].state, ReviewState::New);
        assert_eq!(request.reviews[0].by_group.as_deref(), Some("group1"));
        assert_eq!(
            request.reviews[0].when.as_deref(),
            Some("2010-12-28T00:11:22")
        );
        assert_eq!(request.reviews[0].who.as_deref(), Some("abc"));
        assert_eq!(request.reviews[0].comment, "review start");
        assert_eq!(request.reviews[0].by_user, None);
        assert_eq!(request.history.len(), 1);
        assert_eq!(request.history[0].when, "2010-12-11T00:00:00");
        assert_eq!(request.history[0].who, "creator");
        assert_eq!(request.history[0].comment, "");
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_review_with_history_parses_and_round_trips() {
        // A review that was acted on carries one `<history>` child
        // per change, each with a description of what happened —
        // never empty, never a placeholder. The parser must read it,
        // not refuse the whole request.
        let xml = "<request creator=\"creator\" id=\"7\">\n  <action type=\"submit\">\n    <source package=\"bar\" project=\"foo\" />\n    <target package=\"bar\" project=\"foobar\" />\n  </action>\n  <state name=\"review\" when=\"2010-12-27T01:36:29\" who=\"abc\" />\n  <review by_user=\"Admin\" state=\"accepted\" when=\"2010-12-28T00:11:22\" who=\"abc\">\n    <comment>looks good</comment>\n    <history when=\"2010-12-28T00:12:00\" who=\"Admin\">\n      <description>Review got accepted</description>\n    </history>\n  </review>\n</request>";
        let request = parse_request(xml).expect("review with history parses");
        assert_eq!(request.reviews.len(), 1);
        assert_eq!(request.reviews[0].state, ReviewState::Accepted);
        assert_eq!(request.reviews[0].history.len(), 1);
        assert_eq!(request.reviews[0].history[0].who, "Admin");
        assert_eq!(
            request.reviews[0].history[0].description,
            "Review got accepted"
        );
        assert_eq!(request.reviews[0].history[0].comment, "");
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_review_obsoleted_parses() {
        // `obsoleted` is a review-only state, set when the reviewer
        // is removed; it never appears on a request itself.
        let xml = "<request creator=\"creator\" id=\"8\">\n  <action type=\"submit\">\n    <source package=\"bar\" project=\"foo\" />\n    <target package=\"bar\" project=\"foobar\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"abc\" />\n  <review by_user=\"gone\" state=\"obsoleted\" when=\"2010-12-28T00:11:22\" who=\"abc\">\n    <history when=\"2010-12-28T00:12:00\" who=\"abc\">\n      <description>Review was updated</description>\n    </history>\n  </review>\n</request>";
        let request = parse_request(xml).expect("obsoleted review parses");
        assert_eq!(request.reviews.len(), 1);
        assert_eq!(request.reviews[0].state, ReviewState::Obsoleted);
        assert_eq!(request.reviews[0].history.len(), 1);
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_read_request3_empty_comment_and_description_drop_on_serialise() {
        let xml = "<request creator=\"xyz\" id=\"2\">\n  <action type=\"set_bugowner\">\n    <target project=\"foo\" />\n    <person name=\"buguser\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-28T12:36:29\" who=\"xyz\">\n    <comment></comment>\n  </state>\n  <description></description>\n</request>";
        let request = parse_request(xml).expect("request 3 parses");
        assert_eq!(request.id, 2);
        assert_eq!(request.creator, "xyz");
        assert_eq!(request.state.name, RequestState::New);
        assert_eq!(request.state.comment, "");
        assert_eq!(request.description, "");
        assert!(request.history.is_empty());
        assert!(request.reviews.is_empty());
        let serialised = request.to_xml();
        assert!(
            !serialised.contains("<comment>"),
            "empty comment drops: {serialised}"
        );
        assert!(
            !serialised.contains("<description>"),
            "empty description drops: {serialised}"
        );
        assert_eq!(parse_request(&serialised), Some(request));
    }

    #[test]
    fn request_legacy_form_parses_and_reserialises_modern() {
        let xml = "<request creator=\"olduser\" id=\"1234\" type=\"submit\">\n  <submit>\n    <source package=\"baz\" project=\"foobar\" />\n    <target package=\"baz\" project=\"foo\" />\n  </submit>\n  <state name=\"new\" when=\"2010-12-30T02:11:22\" who=\"olduser\" />\n</request>";
        let request = parse_request(xml).expect("legacy request parses");
        assert_eq!(request.id, 1234);
        assert_eq!(request.creator, "olduser");
        assert_eq!(
            request.actions[0],
            RequestAction::Submit {
                source_project: "foobar".to_string(),
                source_package: "baz".to_string(),
                source_rev: None,
                target_project: Some("foo".to_string()),
                target_package: Some("baz".to_string()),
                sourceupdate: None,
                updatelink: false,
                makeoriginolder: false,
                acceptinfo: None,
            }
        );
        assert_eq!(request.state.name, RequestState::New);
        assert_eq!(request.state.when, "2010-12-30T02:11:22");
        assert_eq!(request.state.who, "olduser");
        // The modern form: an `<action>` element, and no `type` on
        // the root.
        let serialised = request.to_xml();
        assert!(serialised.contains("<action type=\"submit\">"));
        assert!(serialised.contains("<request creator=\"olduser\" id=\"1234\">"));
        assert!(!serialised.contains("<submit>"));
        assert_eq!(parse_request(&serialised), Some(request));
    }

    #[test]
    fn request_get_actions_filters_by_type() {
        let request = parse_request(LIST_VIEW1_XML).expect("list view parses");
        assert_eq!(request.actions.len(), 8);
        assert_eq!(request.actions_of(&["submit"]).len(), 2);
        for action in request.actions_of(&["submit"]) {
            assert_eq!(action.type_name(), "submit");
        }
        assert_eq!(
            request
                .actions_of(&["submit", "delete", "change_devel"])
                .len(),
            5
        );
        assert_eq!(request.actions_of(&[]).len(), 8);
        assert_eq!(request.actions_of(&["delete"]).len(), 2);
    }

    #[test]
    fn request_parse_rejects_unknown_child_element() {
        let xml = "<request creator=\"creator\" id=\"42\">\n  <action type=\"delete\">\n    <target project=\"foo\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n  <surprise>hello</surprise>\n</request>";
        assert_eq!(parse_request(xml), None);
    }

    #[test]
    fn request_parse_requires_a_state() {
        let xml = "<request creator=\"creator\" id=\"42\">\n  <action type=\"delete\">\n    <target project=\"foo\" />\n  </action>\n</request>";
        assert_eq!(parse_request(xml), None);
    }

    #[test]
    fn request_parse_rejects_missing_or_zero_id() {
        let no_id = "<request creator=\"creator\">\n  <action type=\"delete\">\n    <target project=\"foo\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n</request>";
        assert_eq!(parse_request(no_id), None);
        let zero_id = no_id.replace("creator=\"creator\"", "creator=\"creator\" id=\"0\"");
        assert_eq!(parse_request(&zero_id), None);
    }

    #[test]
    fn open_action_request_ids_searches_by_source_target_and_type() {
        let match_expr = "(state/@name='new' or state/@name='review') and action/@type='maintenance_incident' and action[source/@project='home:alice:branches:OBS_Maintained:foo' and source/@package='foo' and target/@project='openSUSE:Maintenance']";
        let url = build_url(
            "https://api.opensuse.org",
            &["search", "request"],
            &[("match", match_expr)],
        );
        let http = FakeHttp::new().with_get(
            &url,
            "<collection matches=\"2\"><request id=\"2\" creator=\"alice\"><action type=\"maintenance_incident\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"review\" /></request><request id=\"9\" creator=\"alice\"><action type=\"submit\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"new\" /></request></collection>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        // Only the maintenance_incident request blocks, not the
        // submit one that happens to match the same source/target.
        // The parser re-checks the type even though the search
        // already filtered on it.
        assert_eq!(
            client
                .open_action_request_ids(&ActionTarget {
                    action_type: "maintenance_incident",
                    source_project: Some("home:alice:branches:OBS_Maintained:foo"),
                    source_package: Some("foo"),
                    target_project: "openSUSE:Maintenance",
                    target_package: None,
                    target_repository: None,
                    shape: ActionShape::ReleaseProject(None),
                })
                .unwrap(),
            vec!["2".to_string()]
        );
        assert_eq!(http.get_calls().len(), 1);
        assert!(http.get_calls()[0].1);
    }

    #[test]
    fn open_action_request_ids_without_credentials_fails_loud() {
        let http = FakeHttp::new();
        let client = ObsClient::new(&http);
        let err = client
            .open_action_request_ids(&ActionTarget {
                action_type: "maintenance_incident",
                source_project: Some("home:alice:branches:OBS_Maintained:foo"),
                source_package: Some("foo"),
                target_project: "openSUSE:Maintenance",
                target_package: None,
                target_repository: None,
                shape: ActionShape::Any,
            })
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
    }

    #[test]
    fn request_parse_rejects_duplicate_state() {
        let xml = "<request creator=\"creator\" id=\"42\">\n  <action type=\"delete\">\n    <target project=\"foo\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n  <state name=\"review\" when=\"2010-12-28T01:36:29\" who=\"creator\" />\n</request>";
        assert_eq!(parse_request(xml), None);
    }

    #[test]
    fn request_parse_accepts_priority_and_rejects_an_unknown_one() {
        let xml = "<request creator=\"creator\" id=\"42\">\n  <action type=\"delete\">\n    <target project=\"foo\" />\n  </action>\n  <priority>important</priority>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n</request>";
        let request = parse_request(xml).expect("priority parses");
        assert_eq!(request.priority, Some(RequestPriority::Important));
        assert_eq!(parse_request(&request.to_xml()), Some(request));
        let bad = xml.replace("important", "urgent");
        assert_eq!(parse_request(&bad), None);
    }

    #[test]
    fn request_parse_accepts_accept_at() {
        let xml = "<request creator=\"creator\" id=\"42\">\n  <action type=\"delete\">\n    <target project=\"foo\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n  <accept_at>2010-12-30T00:00:00</accept_at>\n</request>";
        let request = parse_request(xml).expect("accept_at parses");
        assert_eq!(request.accept_at.as_deref(), Some("2010-12-30T00:00:00"));
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn request_parse_unescapes_entities_in_text() {
        let xml = "<request creator=\"creator\" id=\"42\">\n  <action type=\"delete\">\n    <target project=\"foo\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n  <title>fish &amp; chips</title>\n  <description>5 &lt; 6 &amp;&amp; 7 &gt; 6</description>\n</request>";
        let request = parse_request(xml).expect("entities parse");
        assert_eq!(request.title.as_deref(), Some("fish & chips"));
        assert_eq!(request.description, "5 < 6 && 7 > 6");
        // Re-serialising escapes them again, and re-parsing reads
        // the same request back.
        assert_eq!(parse_request(&request.to_xml()), Some(request));
    }

    #[test]
    fn review_target_describe_names_each_part() {
        let target = ReviewTarget::new(Some("Admin"), None, None, None).unwrap();
        assert_eq!(target.describe(), "user Admin");
        let target = ReviewTarget::new(None, Some("reviewers"), None, None).unwrap();
        assert_eq!(target.describe(), "group reviewers");
        let target = ReviewTarget::new(None, None, Some("openSUSE:Factory"), None).unwrap();
        assert_eq!(target.describe(), "project openSUSE:Factory");
        let target = ReviewTarget::new(None, None, Some("openSUSE:Factory"), Some("foo")).unwrap();
        assert_eq!(target.describe(), "project openSUSE:Factory/package foo");
        let target = ReviewTarget::new(
            Some("Admin"),
            Some("reviewers"),
            Some("openSUSE:Factory"),
            None,
        )
        .unwrap();
        assert_eq!(
            target.describe(),
            "user Admin, group reviewers, project openSUSE:Factory"
        );
    }

    #[test]
    fn xpath_noop_renders_a_bare_comparison() {
        let query = XpathQuery::Eq("@name".to_string(), "foo".to_string());
        assert_eq!(query.to_string(), "@name='foo'");
    }

    #[test]
    fn xpath_not_wraps_a_comparison() {
        let query = XpathQuery::Not(Box::new(XpathQuery::Eq(
            "@name".to_string(),
            "foo".to_string(),
        )));
        assert_eq!(query.to_string(), "not(@name='foo')");
    }

    #[test]
    fn xpath_contains_renders_a_contains_call() {
        let query = XpathQuery::Contains("@name".to_string(), "foo".to_string());
        assert_eq!(query.to_string(), "contains(@name, 'foo')");
    }

    #[test]
    fn xpath_and_chains_flat() {
        let query = XpathQuery::and_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
        ]);
        assert_eq!(query.to_string(), "@name='foo' and @name='bar'");
        let query = XpathQuery::and_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
            XpathQuery::Eq("@name".to_string(), "baz".to_string()),
        ]);
        assert_eq!(
            query.to_string(),
            "@name='foo' and @name='bar' and @name='baz'"
        );
    }

    #[test]
    fn xpath_or_chains_flat() {
        let query = XpathQuery::or_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
        ]);
        assert_eq!(query.to_string(), "@name='foo' or @name='bar'");
        let query = XpathQuery::or_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
            XpathQuery::Eq("@name".to_string(), "baz".to_string()),
        ]);
        assert_eq!(
            query.to_string(),
            "@name='foo' or @name='bar' or @name='baz'"
        );
    }

    #[test]
    fn xpath_and_or_parenthesises_the_and_side() {
        let and = XpathQuery::and_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
        ]);
        let query = XpathQuery::or_all(vec![
            and,
            XpathQuery::Eq("@name".to_string(), "baz".to_string()),
        ]);
        assert_eq!(
            query.to_string(),
            "(@name='foo' and @name='bar') or @name='baz'"
        );
        let query = XpathQuery::or_all(vec![
            query,
            XpathQuery::Eq("@name".to_string(), "xyz".to_string()),
        ]);
        assert_eq!(
            query.to_string(),
            "(@name='foo' and @name='bar') or @name='baz' or @name='xyz'"
        );
    }

    #[test]
    fn xpath_or_and_parenthesises_the_or_side() {
        let or = XpathQuery::or_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
        ]);
        let query = XpathQuery::and_all(vec![
            or,
            XpathQuery::Eq("@name".to_string(), "baz".to_string()),
        ]);
        assert_eq!(
            query.to_string(),
            "(@name='foo' or @name='bar') and @name='baz'"
        );
        let query = XpathQuery::and_all(vec![
            query,
            XpathQuery::Eq("@name".to_string(), "xyz".to_string()),
        ]);
        assert_eq!(
            query.to_string(),
            "(@name='foo' or @name='bar') and @name='baz' and @name='xyz'"
        );
    }

    #[test]
    fn xpath_and_or_and_parenthesises_both_and_sides() {
        let left = XpathQuery::and_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
        ]);
        let right = XpathQuery::and_all(vec![
            XpathQuery::Eq("@name".to_string(), "baz".to_string()),
            XpathQuery::Eq("@name".to_string(), "xyz".to_string()),
        ]);
        let query = XpathQuery::Or(Box::new(left), Box::new(right));
        assert_eq!(
            query.to_string(),
            "(@name='foo' and @name='bar') or (@name='baz' and @name='xyz')"
        );
    }

    #[test]
    fn xpath_or_and_or_parenthesises_both_or_sides() {
        let left = XpathQuery::or_all(vec![
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name".to_string(), "bar".to_string()),
        ]);
        let right = XpathQuery::or_all(vec![
            XpathQuery::Eq("@name".to_string(), "baz".to_string()),
            XpathQuery::Eq("@name".to_string(), "xyz".to_string()),
        ]);
        let query = XpathQuery::And(Box::new(left), Box::new(right));
        assert_eq!(
            query.to_string(),
            "(@name='foo' or @name='bar') and (@name='baz' or @name='xyz')"
        );
    }

    #[test]
    fn xpath_multiple_terms_and_join() {
        let query = XpathQuery::and_all(vec![
            XpathQuery::Eq("@name1".to_string(), "foo".to_string()),
            XpathQuery::Eq("@name2".to_string(), "bar".to_string()),
        ]);
        assert_eq!(query.to_string(), "@name1='foo' and @name2='bar'");
    }

    #[test]
    fn xpath_eq_list_or_joins_values() {
        let query = XpathQuery::eq_list(
            "@name",
            &["foo".to_string(), "bar".to_string(), "baz".to_string()],
        );
        assert_eq!(
            query.to_string(),
            "@name='foo' or @name='bar' or @name='baz'"
        );
    }

    #[test]
    fn xpath_not_eq_list_and_joins_nots() {
        let query = XpathQuery::and_all(
            ["foo", "bar", "baz"]
                .iter()
                .map(|value| {
                    XpathQuery::Not(Box::new(XpathQuery::Eq(
                        "@name".to_string(),
                        (*value).to_string(),
                    )))
                })
                .collect(),
        );
        assert_eq!(
            query.to_string(),
            "not(@name='foo') and not(@name='bar') and not(@name='baz')"
        );
    }

    #[test]
    fn xpath_element_predicate_scopes_its_inner_query() {
        let query = XpathQuery::Pred(
            "state".to_string(),
            Box::new(XpathQuery::Eq("@name".to_string(), "new".to_string())),
        );
        assert_eq!(query.to_string(), "state[@name='new']");
    }

    #[test]
    fn xpath_empty_renders_empty_and_is_skipped_by_and_all() {
        assert_eq!(XpathQuery::Empty.to_string(), "");
        let query = XpathQuery::and_all(vec![
            XpathQuery::Empty,
            XpathQuery::Eq("@name".to_string(), "foo".to_string()),
        ]);
        assert_eq!(query.to_string(), "@name='foo'");
        assert_eq!(XpathQuery::and_all(vec![]), XpathQuery::Empty);
    }

    #[test]
    fn search_default_matches_the_open_states() {
        assert_eq!(
            RequestSearch::new().match_expr().unwrap(),
            "state[@name='new' or @name='review' or @name='declined']"
        );
    }

    #[test]
    fn search_request_ids_or_join() {
        let search = search_without_states().request_ids(vec![1, 2]);
        assert_eq!(search.match_expr().unwrap(), "@id='1' or @id='2'");
    }

    #[test]
    fn search_source_projects_match_source_only() {
        let search =
            search_without_states().source_projects(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "source[@project='foo' or @project='bar']"
        );
    }

    #[test]
    fn search_source_packages_use_the_singular_attribute() {
        // The search engine's whitelist has `source/@package` and
        // no plural form; the plural is refused by the server.
        let search =
            search_without_states().source_packages(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "source[@package='foo' or @package='bar']"
        );
    }

    #[test]
    fn search_target_projects_match_target_only() {
        let search =
            search_without_states().target_projects(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "target[@project='foo' or @project='bar']"
        );
    }

    #[test]
    fn search_target_packages_use_the_singular_attribute() {
        // As for source packages: the whitelist has
        // `target/@package`, singular; the plural is refused.
        let search =
            search_without_states().target_packages(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "target[@package='foo' or @package='bar']"
        );
    }

    #[test]
    fn search_creators_or_join() {
        let search = search_without_states().creators(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "@creator='foo' or @creator='bar'"
        );
    }

    #[test]
    fn search_who_matches_state_or_history() {
        let search = search_without_states().who(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "state[@who='foo' or @who='bar'] or history[@who='foo' or @who='bar']"
        );
    }

    #[test]
    fn search_states_match_the_given_states() {
        let search = RequestSearch::new().states(vec![RequestState::New, RequestState::Review]);
        assert_eq!(
            search.match_expr().unwrap(),
            "state[@name='new' or @name='review']"
        );
    }

    #[test]
    fn search_types_match_action_types() {
        let search =
            search_without_states().types(vec!["submit".to_string(), "delete".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "action[@type='submit' or @type='delete']"
        );
    }

    #[test]
    fn search_exclude_target_projects_and_joins_nots() {
        // `target[not(@project='X')]`: no target of the request is
        // project X. The outer form `not(target[@project='X'])` is
        // refused by the server with 400 (verified live against the
        // production API), so this predicate form is the one emitted.
        let search = search_without_states()
            .exclude_target_projects(vec!["foo".to_string(), "bar".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "target[not(@project='foo')] and target[not(@project='bar')]"
        );
    }

    #[test]
    fn search_with_no_filter_is_refused() {
        // No clause at all — not even a state — is refused: the
        // server answers an empty `match=` with 400 (`empty_match`).
        let err = search_without_states().match_expr().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn search_combined_filters_parenthesise_the_or_clause() {
        // An or-clause and-joined onto the states needs parens, or
        // the `and` would bind tighter than the `or` and change the
        // query's meaning.
        let search = RequestSearch::new().projects(vec!["foo".to_string()]);
        assert_eq!(
            search.match_expr().unwrap(),
            "(source[@project='foo'] or target[@project='foo']) and state[@name='new' or @name='review' or @name='declined']"
        );
    }

    #[test]
    fn search_rejects_an_invalid_project_name() {
        let search = search_without_states().projects(vec!["foo' or '1'='1".to_string()]);
        let err = search.match_expr().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn search_rejects_a_quoted_creator() {
        let search = search_without_states().creators(vec!["o'brien".to_string()]);
        let err = search.match_expr().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn search_rejects_an_unknown_action_type() {
        let search = search_without_states().types(vec!["explode".to_string()]);
        let err = search.match_expr().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn show_request_reads_a_request_with_its_history() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/request/42?withhistory=1",
            REQUEST1_XML,
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let request = obs.show_request("42").expect("show reads the request");
        assert_eq!(request.id, 42);
        assert_eq!(request.state.name, RequestState::Accepted);
        assert_eq!(request.actions.len(), 2);
        assert_eq!(request.history.len(), 1);
        assert_eq!(request.history[0].comment, "foobar");
        let calls = http.get_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].0,
            "https://api.opensuse.org/request/42?withhistory=1"
        );
        assert!(calls[0].1, "the read is credentialed");
    }

    #[test]
    fn show_request_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let err = obs.show_request("42").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn show_request_invalid_id_without_http() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert!(obs.show_request("0").is_err());
        assert!(obs.show_request("not-an-id").is_err());
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn show_request_malformed_response_is_error() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/request/42?withhistory=1",
            "<request this is not xml",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs.show_request("42").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn show_request_not_found_maps_to_not_found() {
        let http = FakeHttp::new().with_get_error(
            "https://api.opensuse.org/request/42?withhistory=1",
            ErrorKind::NotFound,
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs.show_request("42").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn request_log_returns_only_the_history() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/request/42?withhistory=1",
            REQUEST1_XML,
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let log = obs.request_log("42").expect("log reads the history");
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].description, "Create Request");
        assert_eq!(log[0].comment, "foobar");
        assert_eq!(log[0].who, "creator");
    }

    #[test]
    fn change_state_accept_posts_changestate_with_the_comment() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        obs.change_state("42", RequestState::Accepted, "looks good", false, &dry)
            .expect("accept posts");
        let calls = http.post_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].0,
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted"
        );
        assert_eq!(calls[0].1, "looks good");
        assert!(calls[0].2, "the change is credentialed");
    }

    #[test]
    fn change_state_force_adds_the_force_param() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted&force=1",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        obs.change_state(
            "42",
            RequestState::Accepted,
            "overriding review",
            true,
            &dry,
        )
        .expect("forced accept posts");
        assert_eq!(
            http.post_calls()[0].0,
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted&force=1"
        );
    }

    #[test]
    fn change_state_refuses_superseded_and_deleted() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs
            .change_state("42", RequestState::Superseded, "x", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let err = obs
            .change_state("42", RequestState::Deleted, "x", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn change_state_dry_run_records_the_plan_and_posts_nothing() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        obs.change_state("42", RequestState::Accepted, "looks good", false, &dry)
            .expect("dry-run accept records");
        assert!(http.post_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: change state of submit request 42 to accepted".to_string()]
        );
    }

    #[test]
    fn change_state_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let dry = DryRun::new(false);
        let err = obs
            .change_state("42", RequestState::Accepted, "ok", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn change_state_unexpected_response_is_error() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=declined",
            "<status code=\"not_found\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs
            .change_state("42", RequestState::Declined, "no", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn change_state_retries_server_error_then_succeeds() {
        // The server's API replies before its backend has
        // registered a just-filed request, so the changestate POST
        // that follows a filing can 500 while the request is about
        // to exist. The library retries
        // a transient failure; the second POST lands. The state
        // check between attempts reads the request back as still
        // new, so the retry goes ahead.
        let http = FakeHttp::new()
            .with_post_error_sequence(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                &[ErrorKind::ServerError],
            )
            .with_post(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "<status code=\"ok\" />",
            )
            .with_get(
                "https://api.opensuse.org/request/42?withhistory=1",
                &REQUEST1_XML.replace("name=\"accepted\"", "name=\"new\""),
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        obs.change_state("42", RequestState::Accepted, "looks good", false, &dry)
            .expect("accept retries and lands");
        assert_eq!(http.post_calls().len(), 2);
        assert_eq!(http.get_calls().len(), 1);
    }

    #[test]
    fn change_state_does_not_reissue_when_already_accepted() {
        // The 500 can come after the state change landed, so the
        // retry reads the request back first: a request already in
        // the target state means the change went through, and the
        // POST is never issued again.
        let http = FakeHttp::new()
            .with_post_error_sequence(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                &[ErrorKind::ServerError],
            )
            .with_post(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "<status code=\"ok\" />",
            )
            .with_get(
                "https://api.opensuse.org/request/42?withhistory=1",
                REQUEST1_XML,
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        obs.change_state("42", RequestState::Accepted, "looks good", false, &dry)
            .expect("an already-accepted request reads as success");
        assert_eq!(http.post_calls().len(), 1);
        assert_eq!(http.get_calls().len(), 1);
    }

    #[test]
    fn change_state_persistent_server_error_surfaces_after_the_budget() {
        // A request that never changes state still fails loudly
        // once the budget runs out: with a 400 ms budget the helper
        // tries twice (the first failure, one 500 ms sleep, the
        // second failure past the deadline) and returns the server
        // error instead of hanging. The production budget is 60 s,
        // passed by change_state; the helper takes the budget so
        // this test does not have to wait it out. The state check
        // between attempts has no canned read, so it treats the
        // state as unknown and keeps retrying.
        let http = FakeHttp::new().with_post_error_sequence(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
            &[
                ErrorKind::ServerError,
                ErrorKind::ServerError,
                ErrorKind::ServerError,
            ],
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs
            .post_changestate_with_retry(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "looks good",
                42,
                RequestState::Accepted,
                std::time::Duration::from_millis(400),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert_eq!(http.post_calls().len(), 2);
    }

    #[test]
    fn change_state_does_not_retry_deterministic_errors() {
        // Only transient failures retry: bad credentials fail fast
        // on the first attempt instead of burning the budget, and
        // no state read runs.
        let http = FakeHttp::new()
            .with_post_error_sequence(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                &[ErrorKind::AuthFailed],
            )
            .with_post(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs
            .change_state("42", RequestState::Accepted, "looks good", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AuthFailed);
        assert_eq!(http.post_calls().len(), 1);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn accept_decline_revoke_post_the_right_newstate() {
        let http = FakeHttp::new()
            .with_post(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "<status code=\"ok\" />",
            )
            .with_post(
                "https://api.opensuse.org/request/43?cmd=changestate&newstate=declined",
                "<status code=\"ok\" />",
            )
            .with_post(
                "https://api.opensuse.org/request/44?cmd=changestate&newstate=revoked",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        obs.accept_request("42", "landing it", false, &dry)
            .expect("accept posts");
        obs.decline_request("43", "not this way", &dry)
            .expect("decline posts");
        obs.revoke_request("44", "withdrawing", &dry)
            .expect("revoke posts");
        let calls = http.post_calls();
        assert_eq!(calls.len(), 3);
        assert_eq!(
            calls[0].0,
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted"
        );
        assert_eq!(
            calls[1].0,
            "https://api.opensuse.org/request/43?cmd=changestate&newstate=declined"
        );
        assert_eq!(
            calls[2].0,
            "https://api.opensuse.org/request/44?cmd=changestate&newstate=revoked"
        );
    }

    #[test]
    fn try_change_state_true_on_success() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        assert!(obs
            .try_change_state("42", RequestState::Accepted, "ok", false, &dry)
            .unwrap());
    }

    #[test]
    fn try_change_state_propagates_a_refused_change() {
        // A transport failure (TLS verification here — a kind the
        // changestate retry does not retry, so the test stays fast)
        // propagates with the error, so the payload distinguishes a
        // refused change from a request already in the target state
        // (which still reports false).
        let http = FakeHttp::new().with_post_error(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
            ErrorKind::TlsError,
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs
            .try_change_state("42", RequestState::Accepted, "ok", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::TlsError);
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn try_change_state_propagates_an_auth_failure() {
        // An auth failure is misconfiguration, not a refused change:
        // it propagates so a batch with wrong or expired credentials
        // fails loudly instead of reporting all-false.
        let http = FakeHttp::new().with_post_error(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
            ErrorKind::AuthFailed,
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs
            .try_change_state("42", RequestState::Accepted, "ok", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AuthFailed);
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn try_change_state_false_when_already_in_the_target_state() {
        // The server answers a changestate on a request already in
        // the target state with the same 403 as a genuine auth
        // failure, so the status alone cannot tell them apart: the
        // read-back finds request 42 already accepted and the
        // lenient call reports false instead of failing.
        let http = FakeHttp::new()
            .with_post_error(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                ErrorKind::AuthFailed,
            )
            .with_get(
                "https://api.opensuse.org/request/42?withhistory=1",
                REQUEST1_XML,
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        assert!(!obs
            .try_change_state("42", RequestState::Accepted, "ok", false, &dry)
            .unwrap());
        assert_eq!(http.post_calls().len(), 1);
        assert_eq!(http.get_calls().len(), 1);
    }

    #[test]
    fn try_change_state_propagates_auth_failure_when_state_differs() {
        // The read-back finds request 123 in review, not accepted,
        // so the 403 was a real refusal and the auth failure
        // propagates.
        let http = FakeHttp::new()
            .with_post_error(
                "https://api.opensuse.org/request/123?cmd=changestate&newstate=accepted",
                ErrorKind::AuthFailed,
            )
            .with_get(
                "https://api.opensuse.org/request/123?withhistory=1",
                REQUEST2_XML,
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs
            .try_change_state("123", RequestState::Accepted, "ok", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AuthFailed);
        assert_eq!(http.post_calls().len(), 1);
        assert_eq!(http.get_calls().len(), 1);
    }

    #[test]
    fn try_change_state_propagates_auth_failure_when_the_read_fails() {
        // Bad credentials fail the read-back too, so a failed read
        // cannot clear the 403: the auth failure propagates.
        let http = FakeHttp::new()
            .with_post_error(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                ErrorKind::AuthFailed,
            )
            .with_get_error(
                "https://api.opensuse.org/request/42?withhistory=1",
                ErrorKind::AuthFailed,
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs
            .try_change_state("42", RequestState::Accepted, "ok", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::AuthFailed);
        assert_eq!(http.post_calls().len(), 1);
        assert_eq!(http.get_calls().len(), 1);
    }

    #[test]
    fn try_change_state_propagates_missing_credentials() {
        // Missing credentials are a configuration failure, not a
        // refused change: they propagate before any request.
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let dry = DryRun::new(false);
        let err = obs
            .try_change_state("42", RequestState::Accepted, "ok", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn try_change_state_propagates_a_caller_bug() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs
            .try_change_state("not-an-id", RequestState::Accepted, "ok", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn try_change_state_true_in_dry_run() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        assert!(obs
            .try_change_state("42", RequestState::Accepted, "ok", false, &dry)
            .unwrap());
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn add_review_posts_addreview_with_the_reason() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=addreview&by_user=Admin",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let target = ReviewTarget::new(Some("Admin"), None, None, None).unwrap();
        obs.add_review("42", &target, "please review", &dry)
            .expect("review posts");
        let calls = http.post_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].0,
            "https://api.opensuse.org/request/42?cmd=addreview&by_user=Admin"
        );
        assert_eq!(calls[0].1, "please review");
        assert!(calls[0].2, "the review is credentialed");
    }

    #[test]
    fn add_review_by_project_and_package_names_both() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=addreview&by_project=openSUSE%3AFactory&by_package=foo",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let target = ReviewTarget::new(None, None, Some("openSUSE:Factory"), Some("foo")).unwrap();
        obs.add_review("42", &target, "package review", &dry)
            .expect("project/package review posts");
        assert_eq!(
            http.post_calls()[0].0,
            "https://api.opensuse.org/request/42?cmd=addreview&by_project=openSUSE%3AFactory&by_package=foo"
        );
    }

    #[test]
    fn add_review_refuses_an_empty_reason() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let target = ReviewTarget::new(Some("Admin"), None, None, None).unwrap();
        let err = obs.add_review("42", &target, "", &dry).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn review_target_requires_someone_and_a_project_for_a_package() {
        assert!(ReviewTarget::new(None, None, None, None).is_err());
        assert!(ReviewTarget::new(None, None, None, Some("foo")).is_err());
        assert!(ReviewTarget::new(Some(""), None, None, None).is_err());
        assert!(ReviewTarget::new(None, None, Some("not a project!"), None).is_err());
        assert!(ReviewTarget::new(None, None, Some("openSUSE:Factory"), Some("foo")).is_ok());
    }

    #[test]
    fn add_review_dry_run_records_the_plan_and_posts_nothing() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        let target = ReviewTarget::new(Some("Admin"), None, None, None).unwrap();
        obs.add_review("42", &target, "please review", &dry)
            .expect("dry-run review records");
        assert!(http.post_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: add a review by user Admin to submit request 42".to_string()]
        );
    }

    #[test]
    fn add_review_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let dry = DryRun::new(false);
        let target = ReviewTarget::new(Some("Admin"), None, None, None).unwrap();
        let err = obs
            .add_review("42", &target, "please review", &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn add_review_unexpected_response_is_error() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=addreview&by_user=Admin",
            "<status code=\"not_found\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let target = ReviewTarget::new(Some("Admin"), None, None, None).unwrap();
        let err = obs
            .add_review("42", &target, "please review", &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    const COMMENTS_XML: &str = r#"<comments request="42">
  <comment who="user1" when="2010-12-29T10:00:00" id="1">first comment</comment>
  <comment who="user2" when="2010-12-29T11:00:00" id="2" parent="1">a reply</comment>
</comments>"#;

    #[test]
    fn parse_comments_reads_comments_and_replies_in_order() {
        let comments = parse_comments(COMMENTS_XML).expect("comments parse");
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].id, 1);
        assert_eq!(comments[0].who, "user1");
        assert_eq!(comments[0].when, "2010-12-29T10:00:00");
        assert_eq!(comments[0].parent_id, None);
        assert_eq!(comments[0].body, "first comment");
        assert_eq!(comments[1].id, 2);
        assert_eq!(comments[1].who, "user2");
        assert_eq!(comments[1].parent_id, Some(1));
        assert_eq!(comments[1].body, "a reply");
    }

    #[test]
    fn parse_comments_empty_is_empty() {
        assert_eq!(
            parse_comments("<comments request=\"42\"></comments>"),
            Some(Vec::new())
        );
        assert_eq!(
            parse_comments("<comments request=\"42\" />"),
            Some(Vec::new())
        );
    }

    #[test]
    fn parse_comments_rejects_a_malformed_document() {
        assert_eq!(parse_comments("not xml at all"), None);
        assert_eq!(
            parse_comments("<comments request=\"42\"><note>hi</note></comments>"),
            None
        );
        assert_eq!(
            parse_comments(
                "<comments request=\"42\"><comment who=\"user1\">no id</comment></comments>"
            ),
            None
        );
    }

    #[test]
    fn parse_root_strips_only_a_well_formed_xml_prolog() {
        // A well-formed prolog is stripped.
        assert_eq!(
            parse_comments("<?xml version=\"1.0\"?><comments request=\"42\"></comments>"),
            Some(Vec::new())
        );
        // `<?xmlfoo` is not a prolog: the document does not parse.
        assert_eq!(
            parse_comments("<?xmlfoo version=\"1.0\"?><comments request=\"42\"></comments>"),
            None
        );
        // An unterminated prolog is not stripped either.
        assert_eq!(
            parse_comments("<?xml version=\"1.0\"<comments request=\"42\"></comments>"),
            None
        );
    }

    #[test]
    fn list_comments_returns_the_parsed_comments() {
        let http =
            FakeHttp::new().with_get("https://api.opensuse.org/comments/request/42", COMMENTS_XML);
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let comments = obs.list_comments("42").expect("comments list");
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[1].parent_id, Some(1));
        let calls = http.get_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "https://api.opensuse.org/comments/request/42");
        assert!(calls[0].1, "the list is credentialed");
    }

    #[test]
    fn list_comments_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let err = obs.list_comments("42").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn list_comments_unexpected_response_is_error() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/comments/request/42",
            "not comments at all",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs.list_comments("42").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn add_comment_posts_the_comment_as_plain_text() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/comments/request/42",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        obs.add_comment("42", "looks good to me", None, &dry)
            .expect("comment posts");
        let calls = http.post_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "https://api.opensuse.org/comments/request/42");
        assert_eq!(calls[0].1, "looks good to me");
        assert!(calls[0].2, "the comment is credentialed");
    }

    #[test]
    fn add_comment_reply_names_its_parent() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/comments/request/42?parent_id=1",
            "<status code=\"ok\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        obs.add_comment("42", "agreed", Some("1"), &dry)
            .expect("reply posts");
        assert_eq!(
            http.post_calls()[0].0,
            "https://api.opensuse.org/comments/request/42?parent_id=1"
        );
    }

    #[test]
    fn add_comment_refuses_an_empty_body() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs.add_comment("42", "", None, &dry).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn add_comment_dry_run_records_the_plan_and_posts_nothing() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(true);
        obs.add_comment("42", "looks good to me", None, &dry)
            .expect("dry-run comment records");
        assert!(http.post_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec!["[dry-run] would: comment on submit request 42".to_string()]
        );
    }

    #[test]
    fn add_comment_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let dry = DryRun::new(false);
        let err = obs
            .add_comment("42", "looks good to me", None, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn add_comment_unexpected_response_is_error() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/comments/request/42",
            "<status code=\"not_found\" />",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let err = obs
            .add_comment("42", "looks good to me", None, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn request_diff_plain_posts_diff_and_returns_the_body() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=diff",
            "diff --git a/foo b/foo\n",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let diff = obs.request_diff("42", false).expect("diff reads");
        assert_eq!(diff, "diff --git a/foo b/foo\n");
        let calls = http.post_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "https://api.opensuse.org/request/42?cmd=diff");
        assert_eq!(calls[0].1, "");
        assert!(calls[0].2, "the diff read is credentialed");
    }

    #[test]
    fn request_diff_xml_adds_the_view_param() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request/42?cmd=diff&view=xml",
            "<request id=\"42\"></request>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let diff = obs.request_diff("42", true).expect("xml diff reads");
        assert_eq!(diff, "<request id=\"42\"></request>");
        assert_eq!(
            http.post_calls()[0].0,
            "https://api.opensuse.org/request/42?cmd=diff&view=xml"
        );
    }

    #[test]
    fn request_diff_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let err = obs.request_diff("42", false).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn search_requests_returns_parsed_requests() {
        let collection =
            format!("<collection matches=\"2\">\n{REQUEST1_XML}\n{REQUEST2_XML}\n</collection>");
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/request?match=%28%40id%3D%2742%27%20or%20%40id%3D%27123%27%29%20and%20state%5B%40name%3D%27new%27%20or%20%40name%3D%27review%27%20or%20%40name%3D%27declined%27%5D",
            &collection,
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search = RequestSearch::new().request_ids(vec![42, 123]);
        let requests = obs.search_requests(&search).expect("search parses");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].id, 42);
        assert_eq!(requests[1].id, 123);
        let calls = http.get_calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].1, "the search is credentialed");
    }

    #[test]
    fn search_requests_empty_collection_is_empty() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/request?match=state%5B%40name%3D%27new%27%20or%20%40name%3D%27review%27%20or%20%40name%3D%27declined%27%5D",
            "<collection matches=\"0\"></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let requests = obs
            .search_requests(&RequestSearch::new())
            .expect("empty search parses");
        assert!(requests.is_empty());
    }

    #[test]
    fn search_requests_unexpected_response_is_error() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/request?match=state%5B%40name%3D%27new%27%20or%20%40name%3D%27review%27%20or%20%40name%3D%27declined%27%5D",
            "not a collection at all",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let err = obs.search_requests(&RequestSearch::new()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn search_requests_without_credentials_fails_closed() {
        let http = FakeHttp::new();
        let obs = ObsClient::new(&http);
        let err = obs.search_requests(&RequestSearch::new()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn search_requests_with_history_adds_the_history_param() {
        // The search is the default open-states search with the
        // history flag: an all-empty search is refused (the server
        // answers an empty match with 400), so the flag rides on a
        // real filter.
        let search = RequestSearch::new().with_history(true);
        let url = build_url(
            "https://api.opensuse.org",
            &["search", "request"],
            &[
                ("match", &search.match_expr().unwrap()),
                ("withhistory", "1"),
            ],
        );
        let http = FakeHttp::new().with_get(&url, "<collection matches=\"0\"></collection>");
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let requests = obs.search_requests(&search).expect("history search parses");
        assert!(requests.is_empty());
        assert_eq!(http.get_calls()[0].0, url);
    }

    #[test]
    fn search_requests_with_full_history_adds_the_full_history_param() {
        // As above, with the full-history flag instead.
        let search = RequestSearch::new().with_full_history(true);
        let url = build_url(
            "https://api.opensuse.org",
            &["search", "request"],
            &[
                ("match", &search.match_expr().unwrap()),
                ("withfullhistory", "1"),
            ],
        );
        let http = FakeHttp::new().with_get(&url, "<collection matches=\"0\"></collection>");
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let requests = obs.search_requests(&search).expect("full history parses");
        assert!(requests.is_empty());
        assert_eq!(http.get_calls()[0].0, url);
    }

    #[test]
    fn parse_request_collection_rejects_a_malformed_request() {
        let collection = "<collection matches=\"1\">\n<request creator=\"creator\" id=\"42\">\n  <action type=\"explode\">\n    <target project=\"foo\" />\n  </action>\n  <state name=\"new\" when=\"2010-12-27T01:36:29\" who=\"creator\" />\n</request>\n</collection>";
        assert_eq!(parse_request_collection(collection), None);
    }

    #[test]
    fn parse_request_collection_rejects_a_count_with_no_requests() {
        assert_eq!(
            parse_request_collection("<collection matches=\"2\"></collection>"),
            None
        );
    }

    #[test]
    fn parse_request_collection_rejects_a_short_collection() {
        // The server returns every match in one response (over the
        // limit it refuses with 403 rather than truncating), so a
        // collection carrying fewer requests than its `matches`
        // count is a shape change, not a partial result.
        let collection = format!("<collection matches=\"2\">\n{REQUEST1_XML}\n</collection>");
        assert_eq!(parse_request_collection(&collection), None);
    }

    #[test]
    fn create_request_with_add_revision_posts_add_revision() {
        let http = FakeHttp::new().with_post(
            "https://api.opensuse.org/request?cmd=create&addrevision=1",
            "<request id=\"1400001\" creator=\"alice\"></request>",
        );
        let client = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        assert_eq!(
            client
                .create_request_with_add_revision("<request></request>")
                .unwrap(),
            "1400001".to_string()
        );
    }

    #[test]
    fn debug_change_state_retry_reports_probe_and_decision() {
        // The changestate POST 500s, the state check reads the
        // request back as still new, and the retry goes ahead: one
        // probe line and one accept-state decision line, in time
        // order.
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new()
            .with_post_error_sequence(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                &[ErrorKind::ServerError],
            )
            .with_post(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "<status code=\"ok\" />",
            )
            .with_get(
                "https://api.opensuse.org/request/42?withhistory=1",
                &REQUEST1_XML.replace("name=\"accepted\"", "name=\"new\""),
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials())
            .unwrap()
            .with_debug(debug);
        let dry = DryRun::new(false);
        obs.change_state("42", RequestState::Accepted, "looks good", false, &dry)
            .expect("accept retries and lands");
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            [
                "retry: change_state request 42 probe 1/60s: still new".to_string(),
                "decision: request 42 still new, re-issuing accepted".to_string(),
            ]
        );
    }

    #[test]
    fn debug_change_state_already_accepted_reports_the_decision() {
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new()
            .with_post_error_sequence(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                &[ErrorKind::ServerError],
            )
            .with_post(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "<status code=\"ok\" />",
            )
            .with_get(
                "https://api.opensuse.org/request/42?withhistory=1",
                REQUEST1_XML,
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials())
            .unwrap()
            .with_debug(debug);
        let dry = DryRun::new(false);
        obs.change_state("42", RequestState::Accepted, "looks good", false, &dry)
            .expect("an already-accepted request reads as success");
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            ["decision: request 42 already accepted; change landed, not re-issuing".to_string()]
        );
    }

    #[test]
    fn debug_change_state_unknown_state_reports_probe_and_decision() {
        // No canned read for the state check, so every probe reads
        // the state as unknown and keeps retrying: with a 1500 ms
        // budget the helper tries four times (three 500 ms sleeps).
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new().with_post_error_sequence(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
            &[
                ErrorKind::ServerError,
                ErrorKind::ServerError,
                ErrorKind::ServerError,
                ErrorKind::ServerError,
            ],
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials())
            .unwrap()
            .with_debug(debug);
        let err = obs
            .post_changestate_with_retry(
                "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
                "looks good",
                42,
                RequestState::Accepted,
                std::time::Duration::from_millis(1500),
            )
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert_eq!(http.post_calls().len(), 4);
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            [
                "retry: change_state request 42 probe 1/1500ms: state unknown".to_string(),
                "decision: request 42 state unknown, re-issuing accepted".to_string(),
                "retry: change_state request 42 probe 2/1500ms: state unknown".to_string(),
                "decision: request 42 state unknown, re-issuing accepted".to_string(),
                "retry: change_state request 42 probe 3/1500ms: state unknown".to_string(),
                "decision: request 42 state unknown, re-issuing accepted".to_string(),
                "retry: change_state request 42 probe 4/1500ms: state unknown".to_string(),
                "decision: request 42 state unknown, re-issuing accepted".to_string(),
            ]
        );
    }

    #[test]
    fn format_budget_prints_whole_seconds_and_millis() {
        assert_eq!(
            ObsClient::format_budget(std::time::Duration::from_secs(30)),
            "30s"
        );
        assert_eq!(
            ObsClient::format_budget(std::time::Duration::from_secs(60)),
            "60s"
        );
        assert_eq!(
            ObsClient::format_budget(std::time::Duration::from_millis(1500)),
            "1500ms"
        );
        assert_eq!(
            ObsClient::format_budget(std::time::Duration::from_millis(500)),
            "500ms"
        );
        assert_eq!(
            ObsClient::format_budget(std::time::Duration::from_millis(0)),
            "0s"
        );
    }

    #[test]
    fn debug_try_change_state_reports_a_refused_change() {
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new().with_post_error(
            "https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted",
            ErrorKind::TlsError,
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials())
            .unwrap()
            .with_debug(debug);
        let dry = DryRun::new(false);
        let err = obs
            .try_change_state("42", RequestState::Accepted, "ok", false, &dry)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::TlsError);
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            ["decision: could not change state of submit request 42: tls_error: TLS verification failed (canned POST error for https://api.opensuse.org/request/42?cmd=changestate&newstate=accepted)"
                .to_string()]
        );
    }
}
