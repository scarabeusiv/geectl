use super::action_tag_type;
use super::changelog_names_cve;
use super::request_block_matches_action;
use super::valid_attribute_name;
use super::valid_obs_name;
use super::validate_request_id;
use super::ActionShape;
use super::ActionTarget;
use super::CveId;
use super::DryRun;
use super::ObsClient;
use super::FACTORY_PROJECT;
use crate::debug::Debug;
use crate::error::ErrorKind;
use crate::error::GeectlError;
use crate::xml::xml_escape;

/// What OBS does with the source branch when a submit request is
/// accepted. The values are exactly the server's
/// `BsRequestAction::VALID_SOURCEUPDATE_OPTIONS`; anything else is
/// refused by the server at creation, so this enum is the only way to
/// name one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceUpdate {
    /// Delete the source package on accept (and the source project
    /// with it, when it was its last package and it is not a
    /// top-level `home:<user>` project). This is the default: the
    /// branch dies when its request lands, like a GitHub head branch
    /// on merge, so accepted branches never pile up as leftovers.
    #[default]
    Cleanup,
    /// Keep the source package, updated to the accepted state. This
    /// is what [`SrPayload::without_cleanup`] selects, and what
    /// `osc sr --no-cleanup` sends.
    Update,
    /// Leave the source package untouched on accept.
    NoUpdate,
}

impl SourceUpdate {
    /// The server's spelling of this value, as it appears in
    /// `<sourceupdate>`.
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceUpdate::Cleanup => "cleanup",
            SourceUpdate::Update => "update",
            SourceUpdate::NoUpdate => "noupdate",
        }
    }
}

/// The validated submit-request payload: where the change comes from,
/// where it goes, what it says, and the one `.changes` entry it carries.
///
/// The `.changes` entry is not part of the OBS request XML (it lives
/// in the package's `.changes` file, in the branch); it is carried
/// here so the filing step can log the complete submission and the
/// caller can validate the changelog rules before anything is filed.
///
/// The description follows the guide's SR message format
/// (`references/request-text.md` "SR and PR messages"): one headline
/// of at most 160 characters of prose (`Update <pkg> to <ver>: <what
/// changes>`), then `- ` lines for staging, needs, and rebuilds. The
/// library validates the description is non-empty; the headline
/// shape is the caller's responsibility.
///
/// Branch cleanup is the default: the payload carries
/// `opt_sourceupdate="cleanup"`, so OBS deletes the source branch on
/// accept. A caller that needs the branch to survive — a forward from
/// a devel project, where the source is a package that must keep
/// existing — opts out with [`SrPayload::without_cleanup`].
///
/// The fields are private: a payload can only be built through
/// [`SrPayload::new`], so a struct literal cannot bypass the
/// validation below. Read them through the accessors; [`SrFiling`]
/// re-validates the semantics on the filing path as well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SrPayload {
    /// The project the change comes from (the branch, e.g.
    /// `home:scarabeus_iv`).
    source_project: String,
    /// The package in the source project.
    source_package: String,
    /// The project the SR targets (the devel project). Never
    /// `openSUSE:Factory` — a submit request targets the devel
    /// project; Factory comes via a forward.
    target_project: String,
    /// The package in the target project.
    target_package: String,
    /// The SR description text (the human-readable summary).
    description: String,
    /// The one `.changes` entry for this submission.
    changes_entry: String,
    /// The CVEs this submission claims to fix. The `.changes` entry
    /// must name each of them (see [`changelog_names_cve`]).
    cves: Vec<CveId>,
    /// What OBS does with the source branch when this request is
    /// accepted. The default is [`SourceUpdate::Cleanup`]: the branch
    /// dies on accept, so accepted branches never pile up as
    /// leftovers. See [`SrPayload::without_cleanup`] for the opt-out.
    source_update: SourceUpdate,
}

impl SrPayload {
    /// Build and validate a payload. Every name must be a valid OBS
    /// project/package name, the target must not be
    /// `openSUSE:Factory`, the description and `.changes` entry must
    /// be non-empty, and the `.changes` entry must name every CVE in
    /// `cves`. The payload carries `opt_sourceupdate="cleanup"` by
    /// default, so OBS deletes the source branch on accept; use
    /// [`SrPayload::without_cleanup`] when the branch must survive.
    /// Returns Err instead of building a payload that could not be
    /// filed.
    pub fn new(
        source_project: &str,
        source_package: &str,
        target_project: &str,
        target_package: &str,
        description: &str,
        changes_entry: &str,
        cves: &[CveId],
    ) -> Result<Self, GeectlError> {
        let payload = SrPayload {
            source_project: source_project.to_string(),
            source_package: source_package.to_string(),
            target_project: target_project.to_string(),
            target_package: target_package.to_string(),
            description: description.to_string(),
            changes_entry: changes_entry.to_string(),
            cves: cves.to_vec(),
            source_update: SourceUpdate::default(),
        };
        payload.validate_semantics()?;
        // The payload must render to well-formed XML, or it is not a
        // payload at all — check now, not at filing time.
        payload.validate_xml()?;
        Ok(payload)
    }

    /// The semantic checks [`SrPayload::new`] runs: every name must
    /// be a valid OBS project/package name, the target must not be
    /// `openSUSE:Factory`, the description and `.changes` entry must
    /// be non-empty, and the `.changes` entry must name every CVE in
    /// `cves`. [`SrFiling::file`] runs the same checks again on the
    /// filing path, so a payload that reaches filing is validated
    /// there too, not only at construction.
    pub fn validate_semantics(&self) -> Result<(), GeectlError> {
        for (label, value) in [
            ("source project", self.source_project.as_str()),
            ("target project", self.target_project.as_str()),
        ] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS project name for {label}: {value:?}"),
                ));
            }
        }
        for (label, value) in [
            ("source package", self.source_package.as_str()),
            ("target package", self.target_package.as_str()),
        ] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name for {label}: {value:?}"),
                ));
            }
        }
        // A submit request targets the devel project, never Factory
        // directly: the Factory step is a forward after the devel
        // request lands.
        if self.target_project == FACTORY_PROJECT {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "submit requests target the devel project, never openSUSE:Factory directly",
            ));
        }
        if self.description.trim().is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "SR description must not be empty",
            ));
        }
        if self.changes_entry.trim().is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "SR .changes entry must not be empty",
            ));
        }
        for cve in &self.cves {
            if !changelog_names_cve(&self.changes_entry, cve) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "the .changes entry does not name {cve}, which this submission claims to fix"
                    ),
                ));
            }
        }
        Ok(())
    }

    /// The project the change comes from (the branch).
    pub fn source_project(&self) -> &str {
        &self.source_project
    }

    /// The package in the source project.
    pub fn source_package(&self) -> &str {
        &self.source_package
    }

    /// The project the SR targets (the devel project).
    pub fn target_project(&self) -> &str {
        &self.target_project
    }

    /// The package in the target project.
    pub fn target_package(&self) -> &str {
        &self.target_package
    }

    /// The SR description text (the human-readable summary).
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The one `.changes` entry for this submission.
    pub fn changes_entry(&self) -> &str {
        &self.changes_entry
    }

    /// The CVEs this submission claims to fix.
    pub fn cves(&self) -> &[CveId] {
        &self.cves
    }

    /// What OBS does with the source branch when this request is
    /// accepted.
    pub fn source_update(&self) -> SourceUpdate {
        self.source_update
    }

    /// Opt out of the default branch cleanup: the source branch is
    /// kept (updated) instead of deleted on accept. Use this only
    /// when the branch must survive the request — the case the
    /// packaging guide names is a forward from a devel project (a
    /// Factory forward), where the source is a package that must keep
    /// existing. From a `home:` subproject the default stays:
    /// pr-guard refuses the SR without cleanup, and without it
    /// accepted branches pile up as leftovers. This selects the
    /// `update` value (what `osc sr --no-cleanup` sends), not
    /// `noupdate`: the source is still updated to the accepted state,
    /// it is just not deleted.
    pub fn without_cleanup(mut self) -> Self {
        self.source_update = SourceUpdate::Update;
        self
    }

    /// Render the OBS request XML for this payload. All text is
    /// XML-escaped, so a description containing `&` or `<` cannot break
    /// out of its element. The action carries
    /// `<options><sourceupdate>…</sourceupdate></options>` with the
    /// payload's source-update value (`cleanup` by default).
    ///
    /// Crate-internal: the rendering is used by [`SrFiling`] (the
    /// dry-run note) and by the unit tests. Consumers never see XML;
    /// they file the typed payload through [`SrFiling::file`].
    pub(crate) fn to_xml(&self) -> String {
        format!(
            "<request>\n  <action type=\"submit\">\n    <source project=\"{}\" package=\"{}\" />\n    <target project=\"{}\" package=\"{}\" />\n    <options><sourceupdate>{}</sourceupdate></options>\n  </action>\n  <description>{}</description>\n</request>",
            xml_escape(&self.source_project),
            xml_escape(&self.source_package),
            xml_escape(&self.target_project),
            xml_escape(&self.target_package),
            self.source_update.as_str(),
            xml_escape(&self.description),
        )
    }

    /// Verify the rendered XML is well-formed: tags are balanced and
    /// properly nested, there is exactly one root element (`request`),
    /// and the required elements are present in the expected nesting
    /// (`request > action[type=submit] > source, target, options >
    /// sourceupdate` and `request > description`). The
    /// `<sourceupdate>` value is also checked against the values the
    /// server accepts. This is a structural check on the generated
    /// XML, not a re-derivation of the payload.
    pub fn validate_xml(&self) -> Result<(), GeectlError> {
        validate_sr_xml(&self.to_xml())
    }
}

/// Verify rendered SR XML: well-formed, with the required
/// structure, a `<sourceupdate>` value the server accepts, and an
/// `<action>` carrying `type="submit"`. The action-type check exists
/// so a future edit to `to_xml` that changes the action type fails
/// here, at payload build time, instead of at the API.
fn validate_sr_xml(xml: &str) -> Result<(), GeectlError> {
    check_well_formed(xml, SUBMIT_REQUIRED_PATHS)?;
    // The rendered XML must carry exactly one <sourceupdate> with
    // a value the server accepts. The value comes from the
    // `SourceUpdate` enum, so it is always one of the three —
    // this check exists so a future edit to `to_xml` that drops
    // or mangles the attribute fails here, at payload build time,
    // instead of at the API.
    match sourceupdate_value(xml) {
        Some(value) if ["update", "noupdate", "cleanup"].contains(&value) => {}
        Some(value) => {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("malformed SR XML: unknown <sourceupdate> value {value:?}"),
            ));
        }
        // A missing <sourceupdate> is refused by check_well_formed
        // above, so None here means the element is duplicated.
        None => {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "malformed SR XML: duplicated <sourceupdate> in <action>",
            ));
        }
    }
    if request_block_matches_action(xml, "submit", ActionShape::Any) != Some(true) {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed SR XML: <action> is not type=\"submit\"",
        ));
    }
    Ok(())
}

/// The `<sourceupdate>` value the rendered XML carries, if exactly one
/// `<sourceupdate>…</sourceupdate>` element is present. Any other
/// shape (none, or more than one) reads as absent, so the caller
/// refuses it rather than guessing which one applies.
fn sourceupdate_value(xml: &str) -> Option<&str> {
    let open = "<sourceupdate>";
    let close = "</sourceupdate>";
    let start = xml.find(open)? + open.len();
    let rest = &xml[start..];
    let end = rest.find(close)?;
    let value = &rest[..end];
    // Exactly one element: a second opening or closing tag anywhere
    // means the document carries two values, not one.
    if xml.matches(open).count() != 1 || xml.matches(close).count() != 1 {
        return None;
    }
    Some(value)
}

/// The validated maintenance-incident payload: which packages from
/// which project go into which maintenance project, and what the
/// incident says.
///
/// One action per source package: a single request carries the whole
/// incident, so the packages land together. The target is the
/// maintenance project itself (`openSUSE:Maintenance`, or the
/// incident project for a follow-up); the target package is never
/// specified — the server forbids it (`IllegalRequest`); the incident
/// package is named at accept time, when the server merges the source
/// into the incident project. The optional release project is the maintained product
/// the update releases into; the server resolves it to its update
/// instance and requires `maintenance_release` kind.
///
/// No source-update by default: unlike a submit request, whose
/// branch is a throwaway by default, the incident source is the
/// maintenance working area, which the maintainer may reuse for the
/// next incident. Opt in with [`MaintenanceIncidentPayload::with_cleanup`]
/// when the branch must die on accept.
///
/// A project-level incident (no packages at all) is legal on the
/// server but out of scope here: the skill files incidents per
/// package, and the dedup check keys on the source package. The
/// fields are private: a payload can only be built through
/// [`MaintenanceIncidentPayload::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceIncidentPayload {
    /// The project the update comes from (the maintenance branch).
    source_project: String,
    /// The packages in the source project, one action each.
    source_packages: Vec<String>,
    /// The maintenance project the incident targets.
    target_project: String,
    /// The maintained product the update releases into, if any.
    target_releaseproject: Option<String>,
    /// What OBS does with the source branch when the incident is
    /// accepted. None renders no `<options>` at all.
    source_update: Option<SourceUpdate>,
    /// The incident description text.
    description: String,
}

impl MaintenanceIncidentPayload {
    /// Build and validate a payload. Every name must be a valid OBS
    /// project/package name, at least one source package is required,
    /// with no duplicates, and the description must be non-empty.
    /// Returns Err instead of
    /// building a payload that could not be filed.
    pub fn new(
        source_project: &str,
        source_packages: &[&str],
        target_project: &str,
        description: &str,
    ) -> Result<Self, GeectlError> {
        let payload = MaintenanceIncidentPayload {
            source_project: source_project.to_string(),
            source_packages: source_packages.iter().map(|p| p.to_string()).collect(),
            target_project: target_project.to_string(),
            target_releaseproject: None,
            source_update: None,
            description: description.to_string(),
        };
        payload.validate_semantics()?;
        payload.validate_xml()?;
        Ok(payload)
    }

    /// Set the release project the update releases into. The name
    /// must be a valid OBS project name; the server resolves it to
    /// its update instance at filing time, requires
    /// `maintenance_release` kind, and stores the resolved name. The
    /// dedup keys on the release project as filed, so a release
    /// project that resolves elsewhere may slip past a later dedup;
    /// a `maintenance_release` project resolves to itself, so the
    /// normal case is unaffected.
    pub fn with_release_project(mut self, release_project: &str) -> Result<Self, GeectlError> {
        if !valid_obs_name(release_project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name for release project: {release_project:?}"),
            ));
        }
        self.target_releaseproject = Some(release_project.to_string());
        self.validate_xml()?;
        Ok(self)
    }

    /// Opt in to branch cleanup: the source branch is deleted when
    /// the incident is accepted. Use this only when the branch is a
    /// throwaway; by default the branch survives the incident.
    pub fn with_cleanup(mut self) -> Result<Self, GeectlError> {
        self.source_update = Some(SourceUpdate::Cleanup);
        self.validate_xml()?;
        Ok(self)
    }

    /// The project the update comes from.
    pub fn source_project(&self) -> &str {
        &self.source_project
    }

    /// The packages in the source project, one action each.
    pub fn source_packages(&self) -> &[String] {
        &self.source_packages
    }

    /// The maintenance project the incident targets.
    pub fn target_project(&self) -> &str {
        &self.target_project
    }

    /// The maintained product the update releases into, if any.
    pub fn target_releaseproject(&self) -> Option<&str> {
        self.target_releaseproject.as_deref()
    }

    /// What OBS does with the source branch on accept, if any.
    pub fn source_update(&self) -> Option<SourceUpdate> {
        self.source_update
    }

    /// The incident description text.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The semantic checks [`MaintenanceIncidentPayload::new`] runs:
    /// every name must be a valid OBS project/package name, at least
    /// one source package is required, with no duplicates, and the
    /// description must be non-empty.
    /// [`SrFiling::file_maintenance_incident`] runs the
    /// same checks again on the filing path.
    pub fn validate_semantics(&self) -> Result<(), GeectlError> {
        if !valid_obs_name(&self.source_project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "invalid OBS project name for source project: {:?}",
                    self.source_project
                ),
            ));
        }
        if self.source_packages.is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "a maintenance incident needs at least one source package",
            ));
        }
        // One action per source package: a duplicate would render two
        // identical actions, which the server refuses
        // (`ConflictingActions` on the incident `uniq_key`).
        let mut seen: Vec<&String> = Vec::new();
        for package in &self.source_packages {
            if !valid_obs_name(package) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name for source package: {package:?}"),
                ));
            }
            if seen.contains(&package) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("a maintenance incident lists source package {package:?} twice"),
                ));
            }
            seen.push(package);
        }
        if !valid_obs_name(&self.target_project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "invalid OBS project name for target project: {:?}",
                    self.target_project
                ),
            ));
        }
        if let Some(release_project) = &self.target_releaseproject {
            if !valid_obs_name(release_project) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS project name for release project: {release_project:?}"),
                ));
            }
        }
        if self.description.trim().is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "maintenance incident description must not be empty",
            ));
        }
        Ok(())
    }

    /// Render the OBS request XML for this payload: one
    /// `maintenance_incident` action per source package, each with
    /// its source and the shared target (plus the release project,
    /// when set), and an `<options>` block only when cleanup is
    /// opted in. All text is XML-escaped.
    pub fn to_xml(&self) -> String {
        let mut actions = String::new();
        for package in &self.source_packages {
            actions.push_str("  <action type=\"maintenance_incident\">\n");
            actions.push_str(&format!(
                "    <source project=\"{}\" package=\"{}\" />\n",
                xml_escape(&self.source_project),
                xml_escape(package),
            ));
            actions.push_str(&format!(
                "    <target project=\"{}\"{} />\n",
                xml_escape(&self.target_project),
                match &self.target_releaseproject {
                    Some(release_project) =>
                        format!(" releaseproject=\"{}\"", xml_escape(release_project)),
                    None => String::new(),
                },
            ));
            if let Some(source_update) = self.source_update {
                actions.push_str(&format!(
                    "    <options><sourceupdate>{}</sourceupdate></options>\n",
                    source_update.as_str(),
                ));
            }
            actions.push_str("  </action>\n");
        }
        format!(
            "<request>\n{actions}  <description>{}</description>\n</request>",
            xml_escape(&self.description),
        )
    }

    /// Verify the rendered XML is well-formed and carries exactly
    /// one `maintenance_incident` action per source package, each
    /// with its source and target, plus the description — and a
    /// `<sourceupdate>` in every action exactly when cleanup is
    /// opted in, with a value the server accepts.
    pub fn validate_xml(&self) -> Result<(), GeectlError> {
        let xml = self.to_xml();
        check_well_formed(
            &xml,
            &[
                "request/action",
                "request/action/source",
                "request/action/target",
                "request/description",
            ],
        )?;
        let types = action_types(&xml)?;
        if types.len() != self.source_packages.len()
            || types.iter().any(|t| t != "maintenance_incident")
        {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "malformed maintenance incident XML: expected {} maintenance_incident action(s), found {} action(s) of type(s) {types:?}",
                    self.source_packages.len(),
                    types.len(),
                ),
            ));
        }
        let expected = if self.source_update.is_some() {
            self.source_packages.len()
        } else {
            0
        };
        if xml.matches("<sourceupdate>cleanup</sourceupdate>").count() != expected
            || xml.matches("<sourceupdate>").count() != expected
        {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "malformed maintenance incident XML: <sourceupdate> does not match the opted-in cleanup",
            ));
        }
        Ok(())
    }
}

/// The validated change-devel payload: which package's devel
/// project changes, and to what.
///
/// The source is the NEW devel (project + package); the target is
/// the package whose devel definition changes. On accept the server
/// points the target package's devel at the source. No options are
/// ever sent: the server forbids `sourceupdate`, `source_rev`,
/// `target_releaseproject`, and `target_repository` on this action
/// type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeDevelPayload {
    /// The project holding the new devel package.
    source_project: String,
    /// The new devel package.
    source_package: String,
    /// The project holding the package whose devel changes.
    target_project: String,
    /// The package whose devel changes.
    target_package: String,
    /// The request description text.
    description: String,
}

impl ChangeDevelPayload {
    /// Build and validate a payload. Every name must be a valid OBS
    /// project/package name and the description must be non-empty.
    pub fn new(
        source_project: &str,
        source_package: &str,
        target_project: &str,
        target_package: &str,
        description: &str,
    ) -> Result<Self, GeectlError> {
        let payload = ChangeDevelPayload {
            source_project: source_project.to_string(),
            source_package: source_package.to_string(),
            target_project: target_project.to_string(),
            target_package: target_package.to_string(),
            description: description.to_string(),
        };
        payload.validate_semantics()?;
        payload.validate_xml()?;
        Ok(payload)
    }

    /// The project holding the new devel package.
    pub fn source_project(&self) -> &str {
        &self.source_project
    }

    /// The new devel package.
    pub fn source_package(&self) -> &str {
        &self.source_package
    }

    /// The project holding the package whose devel changes.
    pub fn target_project(&self) -> &str {
        &self.target_project
    }

    /// The package whose devel changes.
    pub fn target_package(&self) -> &str {
        &self.target_package
    }

    /// The request description text.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The semantic checks [`ChangeDevelPayload::new`] runs: every
    /// name must be a valid OBS project/package name and the
    /// description must be non-empty.
    pub fn validate_semantics(&self) -> Result<(), GeectlError> {
        for (label, value) in [
            ("source project", self.source_project.as_str()),
            ("target project", self.target_project.as_str()),
        ] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS project name for {label}: {value:?}"),
                ));
            }
        }
        for (label, value) in [
            ("source package", self.source_package.as_str()),
            ("target package", self.target_package.as_str()),
        ] {
            if !valid_obs_name(value) {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid OBS package name for {label}: {value:?}"),
                ));
            }
        }
        if self.description.trim().is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "change-devel description must not be empty",
            ));
        }
        Ok(())
    }

    /// Render the OBS request XML for this payload: one
    /// `change_devel` action with its source and target. All text
    /// is XML-escaped. No `<options>`: the server forbids them here.
    pub fn to_xml(&self) -> String {
        format!(
            "<request>\n  <action type=\"change_devel\">\n    <source project=\"{}\" package=\"{}\" />\n    <target project=\"{}\" package=\"{}\" />\n  </action>\n  <description>{}</description>\n</request>",
            xml_escape(&self.source_project),
            xml_escape(&self.source_package),
            xml_escape(&self.target_project),
            xml_escape(&self.target_package),
            xml_escape(&self.description),
        )
    }

    /// Verify the rendered XML is well-formed and carries exactly
    /// one `change_devel` action with its source and target, plus
    /// the description — and no `<options>` block.
    pub fn validate_xml(&self) -> Result<(), GeectlError> {
        let xml = self.to_xml();
        check_well_formed(
            &xml,
            &[
                "request/action",
                "request/action/source",
                "request/action/target",
                "request/description",
            ],
        )?;
        if action_types(&xml)? != ["change_devel"] {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "malformed change-devel XML: expected exactly one change_devel action",
            ));
        }
        if xml.contains("<options>") {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "malformed change-devel XML: <options> is forbidden on this action type",
            ));
        }
        Ok(())
    }
}

/// What a delete request targets: one package, one repository, or
/// the whole project. The server forbids a target package and a
/// target repository together, so the shape is an enum, not two
/// optional fields — the forbidden combination is unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteTarget {
    /// Delete one package from the target project.
    Package(String),
    /// Delete one repository from the target project.
    Repository(String),
    /// Delete the whole target project.
    WholeProject,
}

/// One maintained instance of a package: a project carrying the
/// maintenance attribute, with the package it would branch from and
/// where the branch would land. From the dry-run branch report
/// (`POST /source?cmd=branch&dryrun=1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintainedInstance {
    /// The project the package is branched from.
    pub source_project: String,
    /// The package that would be branched.
    pub source_package: String,
    /// The devel project the branch copies newer sources from, when
    /// the package has a devel package.
    pub devel_project: Option<String>,
    /// The devel package the branch copies newer sources from.
    pub devel_package: Option<String>,
    /// The project the branch would land in.
    pub target_project: String,
    /// The package the branch would land as — the report's name with
    /// `:` → `_`, matching the live branch.
    pub target_package: String,
}

/// Options for branching a maintained package (`POST
/// /source?cmd=branch`). The attribute marks which projects maintain
/// the package; the update project attribute points each of them at
/// its update project; the target project overrides the
/// server-derived branch project.
#[derive(Debug, Clone)]
pub struct MaintenanceBranchOptions {
    pub(crate) attribute: String,
    pub(crate) update_project_attribute: String,
    pub(crate) target_project: Option<String>,
}

impl MaintenanceBranchOptions {
    /// Defaults: attribute `OBS:Maintained`, update project
    /// attribute `OBS:UpdateProject`, no target project override
    /// (the server derives
    /// `home:<user>:branches:OBS_Maintained:<package>` — the attribute
    /// with `:` → `_`).
    pub fn new() -> Self {
        MaintenanceBranchOptions {
            attribute: "OBS:Maintained".to_string(),
            update_project_attribute: "OBS:UpdateProject".to_string(),
            target_project: None,
        }
    }

    /// The attribute that marks a project as maintaining the package
    /// (`NAMESPACE:NAME`, e.g. `OBS:Maintained`).
    pub fn with_attribute(mut self, attribute: &str) -> Result<Self, GeectlError> {
        if !valid_attribute_name(attribute) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS attribute name: {attribute:?} (expected NAMESPACE:NAME)"),
            ));
        }
        self.attribute = attribute.to_string();
        Ok(self)
    }

    /// The attribute that points a project at its update project
    /// (`NAMESPACE:NAME`, e.g. `OBS:UpdateProject`). The server
    /// refuses a value that is not in that shape.
    pub fn with_update_project_attribute(mut self, attribute: &str) -> Result<Self, GeectlError> {
        if !valid_attribute_name(attribute) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS attribute name: {attribute:?} (expected NAMESPACE:NAME)"),
            ));
        }
        self.update_project_attribute = attribute.to_string();
        Ok(self)
    }

    /// Branch into this project instead of the server-derived one.
    pub fn with_target_project(mut self, target_project: &str) -> Result<Self, GeectlError> {
        if !valid_obs_name(target_project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("invalid OBS project name for target project: {target_project:?}"),
            ));
        }
        self.target_project = Some(target_project.to_string());
        Ok(self)
    }
}

impl Default for MaintenanceBranchOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Where a maintenance branch landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceBranchResult {
    /// The project the branch landed in.
    pub target_project: String,
    /// The package the branch landed as. None when several packages
    /// were branched: the server then reports only the project.
    pub target_package: Option<String>,
    /// The project the package was branched from (single-package
    /// branches only).
    pub source_project: Option<String>,
    /// The package that was branched (single-package branches only).
    pub source_package: Option<String>,
}

/// The validated delete-request payload: what gets deleted, and
/// what the request says.
///
/// The target project is always required; the target (package,
/// repository, or whole project) comes from [`DeleteTarget`]. On
/// accept the server deletes exactly that — a package, a repository,
/// or the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletePayload {
    /// The project the deletion targets.
    target_project: String,
    /// What inside it gets deleted.
    target: DeleteTarget,
    /// The request description text.
    description: String,
    /// The auto-accept time, when set: the server accepts the
    /// request on its own once this UTC time passes (`osc
    /// deletereq --accept-in-hours`). `None` files a plain
    /// request that waits for a human.
    accept_at: Option<String>,
}

impl DeletePayload {
    /// Build and validate a payload. The target project and (for a
    /// package or repository target) the target name must be valid
    /// OBS names, and the description must be non-empty.
    pub fn new(
        target_project: &str,
        target: DeleteTarget,
        description: &str,
    ) -> Result<Self, GeectlError> {
        let payload = DeletePayload {
            target_project: target_project.to_string(),
            target,
            description: description.to_string(),
            accept_at: None,
        };
        payload.validate_semantics()?;
        payload.validate_xml()?;
        Ok(payload)
    }

    /// The project the deletion targets.
    pub fn target_project(&self) -> &str {
        &self.target_project
    }

    /// What inside it gets deleted.
    pub fn target(&self) -> &DeleteTarget {
        &self.target
    }

    /// The request description text.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The auto-accept time, when one is set.
    pub fn accept_at(&self) -> Option<&str> {
        self.accept_at.as_deref()
    }

    /// Set the auto-accept time to `hours` from now (UTC), as `osc
    /// deletereq --accept-in-hours` does: the server accepts the
    /// request on its own once that time passes. The timestamp is
    /// computed now, in ISO-8601 UTC with a `+00:00` offset, and
    /// rendered after `<description>`, the order osc emits. Hours
    /// that overflow the clock or would land past year 9999 are
    /// refused — a four-digit year is the most the format carries.
    /// Zero hours is allowed, matching osc: the CLI passes
    /// `--accept-in-hours 0` through as a timer for now (the value
    /// arrives as the string `"0"`, which is truthy in Python), so
    /// the method and the CLI agree on 0.
    pub fn with_accept_at_in_hours(mut self, hours: u64) -> Result<Self, GeectlError> {
        self.accept_at = Some(accept_at_in_hours(hours)?);
        self.validate_xml()?;
        Ok(self)
    }

    /// The semantic checks [`DeletePayload::new`] runs: the target
    /// project and the target name (when there is one) must be valid
    /// OBS names, and the description must be non-empty.
    pub fn validate_semantics(&self) -> Result<(), GeectlError> {
        if !valid_obs_name(&self.target_project) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "invalid OBS project name for target project: {:?}",
                    self.target_project
                ),
            ));
        }
        match &self.target {
            DeleteTarget::Package(package) => {
                if !valid_obs_name(package) {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!("invalid OBS package name for target package: {package:?}"),
                    ));
                }
            }
            DeleteTarget::Repository(repository) => {
                if !valid_obs_name(repository) {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "invalid OBS repository name for target repository: {repository:?}"
                        ),
                    ));
                }
            }
            DeleteTarget::WholeProject => {}
        }
        if self.description.trim().is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "delete description must not be empty",
            ));
        }
        Ok(())
    }

    /// Render the OBS request XML for this payload: one `delete`
    /// action with its target (package, repository, or neither for
    /// a whole-project delete), then the description, then the
    /// auto-accept time when one is set. All text is XML-escaped.
    pub fn to_xml(&self) -> String {
        let target_attrs = match &self.target {
            DeleteTarget::Package(package) => {
                format!(" package=\"{}\"", xml_escape(package))
            }
            DeleteTarget::Repository(repository) => {
                format!(" repository=\"{}\"", xml_escape(repository))
            }
            DeleteTarget::WholeProject => String::new(),
        };
        let accept_at = match &self.accept_at {
            Some(accept_at) => format!("\n  <accept_at>{}</accept_at>", xml_escape(accept_at)),
            None => String::new(),
        };
        format!(
            "<request>\n  <action type=\"delete\">\n    <target project=\"{}\"{} />\n  </action>\n  <description>{}</description>{}\n</request>",
            xml_escape(&self.target_project),
            target_attrs,
            xml_escape(&self.description),
            accept_at,
        )
    }

    /// Verify the rendered XML is well-formed and carries exactly
    /// one `delete` action with its target, plus the description —
    /// and an `<accept_at>` exactly when an auto-accept time is set.
    pub fn validate_xml(&self) -> Result<(), GeectlError> {
        let xml = self.to_xml();
        let mut required = vec![
            "request/action",
            "request/action/target",
            "request/description",
        ];
        if self.accept_at.is_some() {
            required.push("request/accept_at");
        }
        check_well_formed(&xml, &required)?;
        if action_types(&xml)? != ["delete"] {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "malformed delete XML: expected exactly one delete action",
            ));
        }
        let expected = usize::from(self.accept_at.is_some());
        if xml.matches("<accept_at>").count() != expected
            || xml.matches("</accept_at>").count() != expected
        {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "malformed delete XML: <accept_at> does not match the opted-in auto-accept time",
            ));
        }
        Ok(())
    }
}

/// The UTC timestamp `hours` from now, in whole-second ISO-8601
/// with a `+00:00` offset. osc emits the same instant
/// typically with microseconds (`isoformat()` omits the fraction
/// when it lands on a whole second); geectl emits whole seconds.
/// The server stores its
/// own rendering of the value (no offset) on the request. The clock
/// read fails
/// only when the system clock predates the epoch; an hour count
/// that overflows the clock or lands past year 9999 is refused.
fn accept_at_in_hours(hours: u64) -> Result<String, GeectlError> {
    let too_many = || {
        GeectlError::new(
            ErrorKind::InvalidInput,
            format!("auto-accept time overflows the clock: {hours} hours is too many"),
        )
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| {
            GeectlError::new(
                ErrorKind::InternalError,
                "system clock predates the Unix epoch; cannot compute an auto-accept time",
            )
        })?;
    let delta = hours.checked_mul(3600).ok_or_else(too_many)?;
    let secs = now.as_secs().checked_add(delta).ok_or_else(too_many)?;
    let (year, _, _) = civil_from_days((secs / 86_400) as i64);
    if year > 9999 {
        return Err(too_many());
    }
    Ok(iso8601_utc_from_unix(secs))
}

/// Format Unix `secs` as ISO-8601 UTC
/// (`2026-11-22T12:34:56+00:00`). The civil conversion is Howard
/// Hinnant's days-from-civil algorithm, so leap years (including
/// the 400-year rule) fall out of the arithmetic instead of a
/// lookup table.
fn iso8601_utc_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}+00:00",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// The (year, month, day) of `days` counted from the Unix epoch.
/// Hinnant's civil-from-days: the era split keeps every division
/// exact, so 2000-02-29 exists and 2100-02-29 does not.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { y + 1 } else { y }, month, day)
}

/// The `type` attributes of the `<action>` elements in `xml`, in
/// order. Err when an action tag is unterminated or carries no type
/// — a generated document is never that shape, so it is malformed,
/// not skippable. Used by the maintenance payloads' `validate_xml`
/// to prove the rendered actions are the ones the payload claims.
fn action_types(xml: &str) -> Result<Vec<String>, GeectlError> {
    let malformed = || {
        GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed request XML: action element without a type",
        )
    };
    let mut types = Vec::new();
    let mut rest = xml;
    while let Some(pos) = rest.find("<action") {
        let after_tag = &rest[pos + "<action".len()..];
        let Some(first) = after_tag.chars().next() else {
            break;
        };
        if !(first.is_whitespace() || first == '/' || first == '>') {
            rest = after_tag;
            continue;
        }
        let Some(end) = after_tag.find('>') else {
            return Err(malformed());
        };
        let Some(action_type) = action_tag_type(&after_tag[..end]) else {
            return Err(malformed());
        };
        types.push(action_type);
        rest = &after_tag[end..];
    }
    Ok(types)
}

/// Check that `xml` is well-formed: every tag is balanced and properly
/// nested, there is exactly one root element, and the element paths in
/// `required` are all present. Returns the first problem found. The
/// required paths are the caller's contract — a submit payload needs
/// its action/source/target/options/sourceupdate/description, a
/// maintenance incident its own shape — so the balance check is
/// shared and the structure check is per payload.
fn check_well_formed(xml: &str, required: &[&str]) -> Result<(), GeectlError> {
    let mut stack: Vec<String> = Vec::new();
    // The element path of each element, for the required-structure check.
    let mut paths: Vec<String> = Vec::new();
    let bytes = xml.as_bytes();
    let mut i = 0;
    let mut root_seen = false;
    while i < bytes.len() {
        let Some(open) = xml[i..].find('<') else {
            break;
        };
        // `start` is the position of this tag; `i` (the scan position)
        // only advances at the end of the iteration, after the tag.
        let start = i + open;
        let Some(close_rel) = xml[start..].find('>') else {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "malformed XML: unterminated tag",
            ));
        };
        let close = start + close_rel;
        let raw = &xml[start + 1..close];
        if raw.starts_with('?') || raw.starts_with('!') {
            // Declaration / comment / doctype: not generated here, but
            // tolerated so a strict checker does not reject a harmless
            // prefix.
            i = close + 1;
            continue;
        }
        if let Some(name) = raw.strip_prefix('/') {
            // Close tag: must match the innermost open tag.
            let name = name.trim();
            match stack.pop() {
                Some(open_name) if open_name == name => {}
                Some(open_name) => {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!(
                            "malformed XML: closing </{name}> does not match open <{open_name}>"
                        ),
                    ));
                }
                None => {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        format!("malformed XML: closing </{name}> with nothing open"),
                    ));
                }
            }
        } else if let Some(without_slash) = raw.strip_suffix('/') {
            // Self-closing tag: no stack change. It is a leaf; record
            // its path for the structure check.
            let name = without_slash.split_whitespace().next().unwrap_or("");
            if name.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "malformed XML: empty tag",
                ));
            }
            let mut path = stack.join("/");
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(name);
            paths.push(path);
        } else {
            // Open tag: push it. The tag name is the first word.
            let name = raw.split_whitespace().next().unwrap_or("");
            if name.is_empty() {
                return Err(GeectlError::new(
                    ErrorKind::InvalidInput,
                    "malformed XML: empty tag",
                ));
            }
            if stack.is_empty() {
                if root_seen {
                    return Err(GeectlError::new(
                        ErrorKind::InvalidInput,
                        "malformed XML: more than one root element",
                    ));
                }
                root_seen = true;
            }
            stack.push(name.to_string());
            let path = stack.join("/");
            paths.push(path);
        }
        i = close + 1;
    }
    if !stack.is_empty() {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("malformed XML: unclosed tag(s): {}", stack.join(", ")),
        ));
    }
    if !root_seen {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed XML: no root element",
        ));
    }
    // Required structure: exactly one root `request`, plus the
    // element paths the caller requires.
    let has = |path: &str| paths.iter().any(|p| p == path);
    if !has("request") {
        return Err(GeectlError::new(
            ErrorKind::InvalidInput,
            "malformed request XML: root element is not <request>",
        ));
    }
    for path in required {
        if !has(path) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("malformed request XML: missing <{path}>"),
            ));
        }
    }
    Ok(())
}

/// The element paths a submit-request payload must carry, for
/// [`check_well_formed`].
const SUBMIT_REQUIRED_PATHS: &[&str] = &[
    "request/action",
    "request/action/source",
    "request/action/target",
    "request/action/options",
    "request/action/options/sourceupdate",
    "request/description",
];

/// Files a submit request through the OBS API, and refiles one over
/// an existing request.
///
/// The only filing path. In dry-run, [`SrFiling::file`] logs exactly
/// what would be filed and verifies the payload, and files nothing. In
/// live mode it POSTs the payload XML, then supersedes every open
/// submit request for the package from the authenticated user with
/// the new request id. [`SrFiling::refile`] extends that path: it
/// files the new request and supersedes the named old one with it,
/// plus every other open submit request for the package — the new
/// request is filed first, and only then are the old ones superseded
/// pointing at it, in that order.
pub struct SrFiling<'a> {
    obs: &'a ObsClient<'a>,
    dry_run: &'a DryRun,
    debug: Debug,
}

/// The result of filing a submit request: the new request id, and
/// the open submit requests it superseded — every open submit
/// request for the package from the authenticated user, in the order
/// they were superseded. Empty when nothing was open.
#[derive(Debug, PartialEq, Eq)]
pub struct FiledRequest {
    /// The new submit request id.
    pub id: String,
    /// The ids superseded with the new request, in supersede order.
    pub superseded: Vec<String>,
}

impl<'a> SrFiling<'a> {
    pub fn new(obs: &'a ObsClient<'a>, dry_run: &'a DryRun) -> Self {
        SrFiling {
            obs,
            dry_run,
            debug: Debug::disabled(),
        }
    }

    /// Report the channel's filing decision lines to `debug`
    /// instead of dropping them. The sink is off by default; the
    /// caller that owns the debug channel passes its sink here.
    pub fn with_debug(mut self, debug: Debug) -> Self {
        self.debug = debug;
        self
    }

    /// File `payload`. Returns the filed request in live mode, or
    /// None in dry-run (nothing was filed). The payload is validated
    /// before either path is taken. In live mode the filing
    /// supersedes every open submit request for the package from the
    /// authenticated user: the new request is filed first, and only
    /// then is each open one superseded pointing at it, so
    /// `superseded_by` always names a request that exists. A second
    /// filing at the same head files and supersedes again — the
    /// supersede is unconditional. The payload carries
    /// `opt_sourceupdate="cleanup"` by default, so OBS deletes the
    /// source branch when the request is accepted; see
    /// [`SrPayload::without_cleanup`] for the opt-out.
    pub fn file(&self, payload: &SrPayload) -> Result<Option<FiledRequest>, GeectlError> {
        let supersede_note = format!(
            "would: supersede all open submit requests targeting {} with the new request",
            payload.target_package
        );
        self.file_with_supersede(payload, &[], &[supersede_note])
    }

    /// File `payload` as a new submit request and supersede `old_id`
    /// with it, plus every other open submit request for the package
    /// from the authenticated user: the full refile flow. Returns the
    /// filed request in live mode, or None in dry-run (nothing was
    /// filed or superseded).
    ///
    /// Branch deletion of the OLD branch is NOT part of this method:
    /// the old branch stays until the caller deletes it explicitly
    /// with [`ObsClient::delete_package`], once the new request is
    /// resolved (or the old branch is otherwise safe to drop). A
    /// superseded request never reaches accept, so the server-side
    /// cleanup never runs for the old branch — only the NEW branch
    /// dies on accept, by the payload's default cleanup. The full
    /// sequence is: file -> supersede -> delete old branch.
    pub fn refile(
        &self,
        payload: &SrPayload,
        old_id: &str,
    ) -> Result<Option<FiledRequest>, GeectlError> {
        // Validate the old id before filing anything: a malformed id
        // must not leave a filed-but-unlinked new request behind.
        let old = validate_request_id(old_id)?;
        let supersede_notes = vec![
            format!("would: supersede submit request {old} with the new request"),
            format!(
                "would: supersede all other open submit requests targeting {} with the new request",
                payload.target_package
            ),
        ];
        self.file_with_supersede(payload, &[old_id.to_string()], &supersede_notes)
    }

    /// File `payload` and supersede every open submit request for
    /// the package from the authenticated user, plus `extra` (the
    /// named request a refile supersedes, which may be declined and
    /// so absent from the open list), the named request first.
    /// Returns the filed request in live mode, or None in dry-run.
    /// In dry-run nothing is read or filed: the plan records the
    /// supersede as `dry_run_supersede` lines against the package,
    /// not as ids. In live mode the order is the guarantee: the new
    /// request is filed FIRST, and only then is each open one
    /// superseded pointing at it, so `superseded_by` always names a
    /// request that exists. (The API does not verify the replacement
    /// exists — it only checks the number is positive — so filing
    /// first is what makes the link real. osc files first too, in
    /// `submitreq --supersede`.)
    ///
    /// If superseding fails after the new request is filed, the error
    /// names the new id: the requests not yet superseded are still
    /// open and the new one unlinked, and the caller finishes the
    /// supersede manually. That is a deliberate divergence from osc,
    /// which supersedes with `can_fail=True` and only warns — a
    /// half-done refile passing silently is a failed refile that
    /// reads as a success.
    fn file_with_supersede(
        &self,
        payload: &SrPayload,
        extra: &[String],
        dry_run_supersede: &[String],
    ) -> Result<Option<FiledRequest>, GeectlError> {
        // The payload is validated on the filing path, not only at
        // construction: the semantic checks run here as well as the
        // XML check, so a payload that reaches filing is a valid
        // payload by both measures.
        payload.validate_semantics()?;
        payload.validate_xml()?;
        if !self.dry_run.allow(&format!(
            "file submit request for {} to {}",
            payload.target_package, payload.target_project
        )) {
            // Dry-run: log exactly what would be filed, in full, so
            // the filing path is exercised end to end without filing.
            // (The `allow` call above already recorded the would-do
            // line in the plan.) The detail lines go through `note`,
            // not `eprintln`, so the plan carries them too.
            self.log_would_be_submit_request(payload);
            for note in dry_run_supersede {
                self.dry_run.note(note);
            }
            return Ok(None);
        }
        let open = self
            .obs
            .open_own_target_request_ids(&payload.target_package)?;
        let mut supersede: Vec<String> = extra.to_vec();
        for id in open {
            if !supersede.contains(&id) {
                supersede.push(id);
            }
        }
        if supersede.is_empty() {
            self.debug.emit(&format!(
                "decision: no open submit request targets {}; filing",
                payload.target_package
            ));
        } else {
            self.debug.emit(&format!(
                "decision: filing for {}; superseding open submit request(s) {} with the new request",
                payload.target_package,
                supersede.join(", ")
            ));
        }
        let id = self.obs.create_request(payload)?;
        let superseded = self.supersede_all(&supersede, &id)?;
        Ok(Some(FiledRequest { id, superseded }))
    }

    /// Log the would-be submit request in full, for the dry-run
    /// plan: source, target, description, `.changes` entry, and the
    /// payload XML. The `allow` call already recorded the would-do
    /// line; these are the detail lines.
    fn log_would_be_submit_request(&self, payload: &SrPayload) {
        self.dry_run
            .note("--- would-be submit request (dry-run) ---");
        self.dry_run.note(&format!(
            "source: {}/{}",
            payload.source_project, payload.source_package
        ));
        self.dry_run.note(&format!(
            "target: {}/{}",
            payload.target_project, payload.target_package
        ));
        self.dry_run
            .note(&format!("description: {}", payload.description));
        self.dry_run
            .note(&format!("changes entry: {}", payload.changes_entry));
        self.dry_run.note(&payload.to_xml());
        self.dry_run.note("--- end of would-be submit request ---");
    }

    /// Supersede each id in `ids` with `new_id`, in order, returning
    /// the superseded ids. On the first failure the error names the
    /// new id, the id that failed, and every id still open (the
    /// failed one plus the ones not yet attempted), so the caller
    /// finishes the supersede manually.
    fn supersede_all(&self, ids: &[String], new_id: &str) -> Result<Vec<String>, GeectlError> {
        let mut done = Vec::new();
        for (index, old_id) in ids.iter().enumerate() {
            match self.obs.supersede_request(old_id, new_id) {
                Ok(()) => done.push(old_id.clone()),
                Err(err) => {
                    let remaining: Vec<&str> = ids[index..].iter().map(String::as_str).collect();
                    return Err(GeectlError::new(
                        err.kind(),
                        format!(
                            "filed submit request {new_id} but could not supersede {old_id}: {}; submit requests {} are still open — supersede them manually with the new id",
                            err.context,
                            remaining.join(", ")
                        ),
                    ));
                }
            }
        }
        Ok(done)
    }

    /// File `payload` as a maintenance incident request. Returns the
    /// new OBS request id in live mode, or None in dry-run (nothing
    /// was filed). The payload is validated before either path is
    /// taken. In live mode an open maintenance incident request
    /// already targeting any of the payload's sources is refused
    /// (dedup, one check per source package); a filing without a
    /// release project is refused by any open incident for the same
    /// source package and target, whatever release project it
    /// resolved to — that over-blocks the narrow cases where the
    /// link-derived release project differs. The filing pins
    /// `source_rev` (`addrevision=1`), so the incident freezes the
    /// exact source it was filed against. The payload carries no
    /// sourceupdate by default; see
    /// [`MaintenanceIncidentPayload::with_cleanup`] for the opt-in
    /// branch cleanup on accept.
    pub fn file_maintenance_incident(
        &self,
        payload: &MaintenanceIncidentPayload,
    ) -> Result<Option<String>, GeectlError> {
        self.file_maintenance_incident_with_except(payload, None)
    }

    fn file_maintenance_incident_with_except(
        &self,
        payload: &MaintenanceIncidentPayload,
        except: Option<u64>,
    ) -> Result<Option<String>, GeectlError> {
        payload.validate_semantics()?;
        payload.validate_xml()?;
        let sources = payload
            .source_packages
            .iter()
            .map(|package| format!("{}/{}", payload.source_project, package))
            .collect::<Vec<String>>()
            .join(", ");
        if !self.dry_run.allow(&format!(
            "file maintenance incident request for {sources} to {}",
            payload.target_project
        )) {
            self.dry_run
                .note("--- would-be maintenance incident request (dry-run) ---");
            for package in &payload.source_packages {
                self.dry_run
                    .note(&format!("source: {}/{}", payload.source_project, package));
            }
            self.dry_run
                .note(&format!("target: {}", payload.target_project));
            self.dry_run
                .note(&format!("description: {}", payload.description));
            self.dry_run.note(&payload.to_xml());
            self.dry_run
                .note("--- end of would-be maintenance incident request ---");
            return Ok(None);
        }
        // Live: the dedup check is part of filing, one check per
        // source package — an incident carries one action per source,
        // and an open incident for any of them blocks the filing.
        // The match mirrors the server's incident identity
        // (`uniq_key`: target, source package, release project) — the
        // source project is not part of it, so an equivalent incident
        // from another branch blocks too, and the server would not
        // catch it (no cross-request duplicate check).
        for package in &payload.source_packages {
            self.check_no_open_action_request(
                &ActionTarget {
                    action_type: "maintenance_incident",
                    source_project: None,
                    source_package: Some(package),
                    target_project: &payload.target_project,
                    target_package: None,
                    target_repository: None,
                    shape: ActionShape::ReleaseProject(payload.target_releaseproject.as_deref()),
                },
                &format!("{} for {}", payload.target_project, package),
                except,
            )?;
        }
        let xml = payload.to_xml();
        let id = self.obs.create_request_with_add_revision(&xml)?;
        Ok(Some(id))
    }

    /// File `payload` as a new maintenance incident request and
    /// supersede `old_id` with it: the full refile flow, same order
    /// guarantee as [`SrFiling::refile`] — file first, supersede
    /// second, so `superseded_by` always names a request that
    /// exists. Returns the new request id in live mode, or None in
    /// dry-run (nothing was filed or superseded).
    pub fn refile_maintenance_incident(
        &self,
        payload: &MaintenanceIncidentPayload,
        old_id: &str,
    ) -> Result<Option<String>, GeectlError> {
        self.refile_with(
            |except| self.file_maintenance_incident_with_except(payload, except),
            old_id,
            "maintenance incident",
        )
    }

    /// File `payload` as a change devel request: move the devel
    /// project of a package. Returns the new OBS request id in live
    /// mode, or None in dry-run (nothing was filed). The payload is
    /// validated before either path is taken. In live mode an open
    /// change devel request already targeting the same target
    /// is refused (dedup); the dedup keys on the target alone, and
    /// the source is not part of the identity. The server sets the
    /// devel project on accept; there is no branch cleanup to opt into.
    pub fn file_change_devel(
        &self,
        payload: &ChangeDevelPayload,
    ) -> Result<Option<String>, GeectlError> {
        payload.validate_semantics()?;
        payload.validate_xml()?;
        if !self.dry_run.allow(&format!(
            "file change devel request for {}/{} to {}/{}",
            payload.source_project,
            payload.source_package,
            payload.target_project,
            payload.target_package
        )) {
            self.dry_run
                .note("--- would-be change devel request (dry-run) ---");
            self.dry_run.note(&format!(
                "source: {}/{}",
                payload.source_project, payload.source_package
            ));
            self.dry_run.note(&format!(
                "target: {}/{}",
                payload.target_project, payload.target_package
            ));
            self.dry_run
                .note(&format!("description: {}", payload.description));
            self.dry_run.note(&payload.to_xml());
            self.dry_run
                .note("--- end of would-be change devel request ---");
            return Ok(None);
        }
        // The match mirrors the server's change-devel identity
        // (`uniq_key`: target project and package only) — the source
        // is not part of it, so a request naming another new devel
        // for the same target blocks too, and the server would not
        // catch it (no cross-request duplicate check).
        self.check_no_open_action_request(
            &ActionTarget {
                action_type: "change_devel",
                source_project: None,
                source_package: None,
                target_project: &payload.target_project,
                target_package: Some(&payload.target_package),
                target_repository: None,
                shape: ActionShape::Any,
            },
            &format!("{}/{}", payload.target_project, payload.target_package),
            None,
        )?;
        let xml = payload.to_xml();
        let id = self
            .obs
            .create_request_with_query(&xml, &[("cmd", "create")], "submit")?;
        Ok(Some(id))
    }

    /// File `payload` as a delete request: delete a package, a
    /// repository, or a whole project. Returns the new OBS request id
    /// in live mode, or None in dry-run (nothing was filed). The
    /// payload is validated before either path is taken. In live mode
    /// an open delete request already targeting the same project (and
    /// package or repository, when the payload names one) is refused
    /// (dedup). The server deletes the target on accept.
    pub fn file_delete_request(
        &self,
        payload: &DeletePayload,
    ) -> Result<Option<String>, GeectlError> {
        payload.validate_semantics()?;
        payload.validate_xml()?;
        let target = match &payload.target {
            DeleteTarget::Package(package) => {
                format!("{}/{}", payload.target_project, package)
            }
            DeleteTarget::Repository(repository) => {
                format!("{} (repository {repository})", payload.target_project)
            }
            DeleteTarget::WholeProject => payload.target_project.clone(),
        };
        if !self
            .dry_run
            .allow(&format!("file delete request for {target}"))
        {
            self.dry_run
                .note("--- would-be delete request (dry-run) ---");
            self.dry_run.note(&format!("target: {target}"));
            self.dry_run
                .note(&format!("description: {}", payload.description));
            self.dry_run.note(&payload.to_xml());
            self.dry_run.note("--- end of would-be delete request ---");
            return Ok(None);
        }
        let (target_package, target_repository) = match &payload.target {
            DeleteTarget::Package(package) => (Some(package.as_str()), None),
            DeleteTarget::Repository(repository) => (None, Some(repository.as_str())),
            DeleteTarget::WholeProject => (None, None),
        };
        self.check_no_open_action_request(
            &ActionTarget {
                action_type: "delete",
                source_project: None,
                source_package: None,
                target_project: &payload.target_project,
                target_package,
                target_repository,
                shape: ActionShape::DeleteTarget {
                    package: target_package,
                    repository: target_repository,
                },
            },
            &target,
            None,
        )?;
        let xml = payload.to_xml();
        let id = self
            .obs
            .create_request_with_query(&xml, &[("cmd", "create")], "submit")?;
        Ok(Some(id))
    }

    /// Refuse when an open request already matches `target` exactly.
    /// The search filters on the action type server-side; the parser
    /// re-checks the type and narrows by shape (delete target shape,
    /// incident release project) where the search cannot. `except`,
    /// when set, is the request the refile flow is about to
    /// supersede: it is open by definition and does not block.
    /// `what` names the target in the refusal, e.g.
    /// `home:alice:branches:foo/foo`. A declined request is closed
    /// and does not block — only `new`/`review` requests are in the
    /// search.
    fn check_no_open_action_request(
        &self,
        target: &ActionTarget<'_>,
        what: &str,
        except: Option<u64>,
    ) -> Result<(), GeectlError> {
        let open = self.obs.open_action_request_ids(target)?;
        let blocking: Vec<&String> = open
            .iter()
            .filter(|id| match except {
                None => true,
                // An id that does not parse cannot be the exempted
                // one: it blocks, rather than being skipped on a
                // failed comparison.
                Some(n) => id.parse::<u64>() != Ok(n),
            })
            .collect();
        if !blocking.is_empty() {
            let blocking = blocking
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<&str>>()
                .join(", ");
            self.debug.emit(&format!(
                "decision: open {} request(s) {blocking} already target {what}; refusing to file",
                target.action_type
            ));
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!(
                    "an open {} request already targets {what} ({blocking}); not filing a duplicate",
                    target.action_type
                ),
            ));
        }
        self.debug.emit(&format!(
            "decision: no open {} request targets {what}; filing",
            target.action_type
        ));
        Ok(())
    }

    /// The shared refile flow: file first, then supersede the old
    /// request pointing at the new one, so `superseded_by` always
    /// names a request that exists. `file` is the filing step (with
    /// the old request exempted from dedup); `kind` names the
    /// request type in messages ("maintenance incident").
    /// If superseding fails after the new request is filed, the error
    /// names the new id: the old request is still open and the new
    /// one unlinked, and the caller finishes the supersede manually.
    /// A half-done refile passing silently is a failed refile that
    /// reads as a success.
    fn refile_with<F>(
        &self,
        file: F,
        old_id: &str,
        kind: &str,
    ) -> Result<Option<String>, GeectlError>
    where
        F: FnOnce(Option<u64>) -> Result<Option<String>, GeectlError>,
    {
        let old = validate_request_id(old_id)?;
        let new_id = file(Some(old))?;
        let Some(new_id) = new_id else {
            if !self.dry_run.enabled() {
                return Err(GeectlError::new(
                    ErrorKind::InternalError,
                    "filing in live mode returned no request id",
                ));
            }
            self.dry_run.note(&format!(
                "would: supersede {kind} request {old} with the new request"
            ));
            return Ok(None);
        };
        match self.obs.supersede_request(old_id, &new_id) {
            Ok(()) => Ok(Some(new_id)),
            Err(err) => Err(GeectlError::new(
                err.kind(),
                format!(
                    "filed {kind} request {new_id} but could not supersede {old}: {}; the old request is still open — supersede it manually with the new id",
                    err.context
                ),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::Debug;
    use crate::error::ErrorKind;
    use crate::obs::build_url;
    use crate::obs::test_support::{basic_credentials, payload, FakeHttp};

    fn payload_with_cve() -> SrPayload {
        SrPayload::new(
            "home:scarabeus_iv",
            "foo",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0: fix CVE-2026-12345",
            "-------------------------------------------------------------------\nFri Nov 20 07:00:00 UTC 2026 - Tomáš Chvátal <tomas.chvatal@gmail.com>\n\n- CVE-2026-12345: fix the overflow (bsc#1234567)\n",
            &[CveId::parse("CVE-2026-12345").unwrap()],
        )
        .unwrap()
    }

    #[test]
    fn validate_xml_rejects_a_non_submit_action() {
        // A future `to_xml` edit that changes the action type must
        // fail at payload build time, not at the API.
        let xml = payload().to_xml();
        assert!(validate_sr_xml(&xml).is_ok());
        let delete = xml.replace("<action type=\"submit\">", "<action type=\"delete\">");
        let err = validate_sr_xml(&delete).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("type=\"submit\""), "{}", err.context);
        // Either quote style counts: a single-quoted submit passes.
        let single = xml.replace("<action type=\"submit\">", "<action type='submit'>");
        assert!(validate_sr_xml(&single).is_ok());
    }

    #[test]
    fn source_update_values_are_the_server_values() {
        assert_eq!(SourceUpdate::default(), SourceUpdate::Cleanup);
        assert_eq!(SourceUpdate::Cleanup.as_str(), "cleanup");
        assert_eq!(SourceUpdate::Update.as_str(), "update");
        assert_eq!(SourceUpdate::NoUpdate.as_str(), "noupdate");
    }

    #[test]
    fn new_payload_defaults_to_cleanup() {
        assert_eq!(payload().source_update, SourceUpdate::Cleanup);
        assert!(payload()
            .to_xml()
            .contains("<options><sourceupdate>cleanup</sourceupdate></options>"));
        assert!(payload().validate_xml().is_ok());
    }

    #[test]
    fn without_cleanup_keeps_the_branch() {
        let xml = payload().without_cleanup().to_xml();
        assert!(xml.contains("<options><sourceupdate>update</sourceupdate></options>"));
        assert!(!xml.contains("cleanup"));
        assert!(payload().without_cleanup().validate_xml().is_ok());
    }

    #[test]
    fn payload_fields_are_read_through_accessors() {
        let payload = payload();
        assert_eq!(payload.source_project(), "home:scarabeus_iv");
        assert_eq!(payload.source_package(), "foo");
        assert_eq!(payload.target_project(), "devel:languages:python");
        assert_eq!(payload.target_package(), "foo");
        assert_eq!(payload.description(), "Update foo to 2.0");
        assert!(payload.changes_entry().contains("Update to version 2.0."));
        assert!(payload.cves().is_empty());
        assert_eq!(payload.source_update(), SourceUpdate::Cleanup);
    }

    #[test]
    fn validate_semantics_accepts_a_valid_payload() {
        assert!(payload().validate_semantics().is_ok());
        assert!(payload_with_cve().validate_semantics().is_ok());
    }

    #[test]
    fn noupdate_renders_and_validates() {
        let mut p = payload();
        p.source_update = SourceUpdate::NoUpdate;
        assert!(p
            .to_xml()
            .contains("<options><sourceupdate>noupdate</sourceupdate></options>"));
        assert!(p.validate_xml().is_ok());
    }

    #[test]
    fn sourceupdate_value_extracts_and_rejects() {
        assert_eq!(sourceupdate_value(&payload().to_xml()), Some("cleanup"));
        assert_eq!(sourceupdate_value("<request></request>"), None);
        // Two values is not a value.
        assert_eq!(
            sourceupdate_value(
                "<options><sourceupdate>cleanup</sourceupdate><sourceupdate>update</sourceupdate></options>"
            ),
            None
        );
        // An unknown value extracts, so the caller can refuse it.
        assert_eq!(
            sourceupdate_value("<options><sourceupdate>wipe</sourceupdate></options>"),
            Some("wipe")
        );
    }

    #[test]
    fn new_rejects_invalid_names_and_empty_text() {
        assert!(SrPayload::new(
            "home:scarabeus_iv",
            "foo/bar",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0",
            "- Update to version 2.0.\n",
            &[],
        )
        .is_err());
        assert!(SrPayload::new(
            "home:scarabeus_iv",
            "foo",
            "devel:languages:python",
            "foo",
            "   ",
            "- Update to version 2.0.\n",
            &[],
        )
        .is_err());
        assert!(SrPayload::new(
            "home:scarabeus_iv",
            "foo",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0",
            "  ",
            &[],
        )
        .is_err());
    }

    #[test]
    fn new_rejects_factory_as_target() {
        // A submit request targets the devel project, never Factory
        // directly.
        assert!(SrPayload::new(
            "home:scarabeus_iv",
            "foo",
            "openSUSE:Factory",
            "foo",
            "Update foo to 2.0",
            "- Update to version 2.0.\n",
            &[],
        )
        .is_err());
    }

    #[test]
    fn new_rejects_unicode_names() {
        // OBS names are ASCII-only; a unicode package name is rejected
        // at payload build time, not at the API.
        assert!(SrPayload::new(
            "home:scarabeus_iv",
            "föö",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0",
            "- Update to version 2.0.\n",
            &[],
        )
        .is_err());
    }

    #[test]
    fn check_well_formed_accepts_generated_xml() {
        assert!(check_well_formed(&payload().to_xml(), SUBMIT_REQUIRED_PATHS).is_ok());
    }

    #[test]
    fn check_well_formed_rejects_mismatched_tags() {
        assert!(check_well_formed("<request><action></request>", SUBMIT_REQUIRED_PATHS).is_err());
        assert!(check_well_formed("<request><action>", SUBMIT_REQUIRED_PATHS).is_err());
        assert!(check_well_formed(
            "<request></request><request></request>",
            SUBMIT_REQUIRED_PATHS
        )
        .is_err());
        assert!(check_well_formed("", SUBMIT_REQUIRED_PATHS).is_err());
    }

    #[test]
    fn check_well_formed_rejects_missing_elements() {
        // Well-formed XML but not an SR: no action/source/target.
        assert!(check_well_formed(
            "<request><description>x</description></request>",
            SUBMIT_REQUIRED_PATHS
        )
        .is_err());
        assert!(check_well_formed(
            "<request><action><source project=\"a\" package=\"b\" /></action></request>",
            SUBMIT_REQUIRED_PATHS
        )
        .is_err());
        // Well-formed SR XML, but without the options block: the
        // cleanup attribute is part of the payload now.
        assert!(check_well_formed(
            "<request><action><source project=\"a\" package=\"b\" /><target project=\"c\" package=\"b\" /></action><description>x</description></request>",
            SUBMIT_REQUIRED_PATHS
        )
        .is_err());
    }

    fn incident_payload() -> MaintenanceIncidentPayload {
        MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &["foo"],
            "openSUSE:Maintenance",
            "Maintenance incident for foo",
        )
        .expect("test incident payload")
    }

    #[test]
    fn maintenance_incident_to_xml_renders_one_action_per_package() {
        assert_eq!(
            incident_payload().to_xml(),
            "<request>\n  <action type=\"maintenance_incident\">\n    <source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" />\n    <target project=\"openSUSE:Maintenance\" />\n  </action>\n  <description>Maintenance incident for foo</description>\n</request>",
        );
    }

    #[test]
    fn maintenance_incident_to_xml_with_release_project_and_cleanup() {
        let payload = MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &["foo", "bar"],
            "openSUSE:Maintenance",
            "Maintenance incident for foo and bar",
        )
        .expect("test incident payload")
        .with_release_project("openSUSE:Leap:15.6:Update")
        .expect("release project")
        .with_cleanup()
        .expect("cleanup");
        assert_eq!(
            payload.to_xml(),
            "<request>\n  <action type=\"maintenance_incident\">\n    <source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" />\n    <target project=\"openSUSE:Maintenance\" releaseproject=\"openSUSE:Leap:15.6:Update\" />\n    <options><sourceupdate>cleanup</sourceupdate></options>\n  </action>\n  <action type=\"maintenance_incident\">\n    <source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"bar\" />\n    <target project=\"openSUSE:Maintenance\" releaseproject=\"openSUSE:Leap:15.6:Update\" />\n    <options><sourceupdate>cleanup</sourceupdate></options>\n  </action>\n  <description>Maintenance incident for foo and bar</description>\n</request>",
        );
        assert!(payload.validate_xml().is_ok());
    }

    #[test]
    fn maintenance_incident_validation_refuses_bad_input() {
        // No source packages: a project-level incident is out of scope.
        assert!(MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &[],
            "openSUSE:Maintenance",
            "Maintenance incident",
        )
        .is_err());
        // Bad names anywhere.
        assert!(MaintenanceIncidentPayload::new(
            "home:alice/bad",
            &["foo"],
            "openSUSE:Maintenance",
            "Maintenance incident",
        )
        .is_err());
        assert!(MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &["foo/bar"],
            "openSUSE:Maintenance",
            "Maintenance incident",
        )
        .is_err());
        assert!(MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &["foo"],
            "openSUSE:Maintenance?",
            "Maintenance incident",
        )
        .is_err());
        // Empty description.
        assert!(MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &["foo"],
            "openSUSE:Maintenance",
            "   ",
        )
        .is_err());
        // Bad release project.
        assert!(incident_payload()
            .with_release_project("openSUSE:Leap/15.6")
            .is_err());
    }

    #[test]
    fn maintenance_incident_validation_refuses_duplicate_source_packages() {
        // Two identical actions would render, which the server refuses
        // (`ConflictingActions` on the incident `uniq_key`).
        assert!(MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &["foo", "foo"],
            "openSUSE:Maintenance",
            "Maintenance incident",
        )
        .is_err());
    }

    #[test]
    fn maintenance_incident_xml_escapes_description() {
        let payload = MaintenanceIncidentPayload::new(
            "home:alice:branches:OBS_Maintained:foo",
            &["foo"],
            "openSUSE:Maintenance",
            "Fix foo & bar <baz>",
        )
        .expect("test incident payload");
        assert!(payload.to_xml().contains("Fix foo &amp; bar &lt;baz&gt;"));
        assert!(payload.validate_xml().is_ok());
    }

    #[test]
    fn change_devel_to_xml_and_validation() {
        let payload = ChangeDevelPayload::new(
            "devel:languages:python",
            "python-foo",
            "openSUSE:Factory",
            "python-foo",
            "Change the devel project for python-foo",
        )
        .expect("test change-devel payload");
        assert_eq!(
            payload.to_xml(),
            "<request>\n  <action type=\"change_devel\">\n    <source project=\"devel:languages:python\" package=\"python-foo\" />\n    <target project=\"openSUSE:Factory\" package=\"python-foo\" />\n  </action>\n  <description>Change the devel project for python-foo</description>\n</request>",
        );
        assert!(payload.validate_xml().is_ok());
        assert!(ChangeDevelPayload::new(
            "devel:languages:python",
            "python-foo",
            "openSUSE:Factory",
            "python-foo?",
            "Change the devel project",
        )
        .is_err());
        assert!(ChangeDevelPayload::new(
            "devel:languages:python",
            "python-foo",
            "openSUSE:Factory",
            "python-foo",
            "",
        )
        .is_err());
    }

    #[test]
    fn delete_to_xml_and_validation() {
        let package = DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Package("foo".to_string()),
            "Drop foo",
        )
        .expect("test delete payload");
        assert_eq!(
            package.to_xml(),
            "<request>\n  <action type=\"delete\">\n    <target project=\"openSUSE:Factory\" package=\"foo\" />\n  </action>\n  <description>Drop foo</description>\n</request>",
        );
        assert!(package.validate_xml().is_ok());
        let repository = DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Repository("standard".to_string()),
            "Drop the standard repository",
        )
        .expect("test delete payload");
        assert!(repository
            .to_xml()
            .contains("<target project=\"openSUSE:Factory\" repository=\"standard\" />"));
        assert!(repository.validate_xml().is_ok());
        let project = DeletePayload::new(
            "home:alice:branches:foo",
            DeleteTarget::WholeProject,
            "Drop the branch project",
        )
        .expect("test delete payload");
        assert!(project
            .to_xml()
            .contains("<target project=\"home:alice:branches:foo\" />"));
        assert!(project.validate_xml().is_ok());
        assert!(DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Package("foo/bar".to_string()),
            "Drop foo",
        )
        .is_err());
        assert!(DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Package("foo".to_string()),
            "   ",
        )
        .is_err());
    }

    #[test]
    fn delete_accept_at_renders_after_description() {
        let payload = DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Package("foo".to_string()),
            "Drop foo",
        )
        .expect("test delete payload")
        .with_accept_at_in_hours(48)
        .expect("48 hours from now is representable");
        let xml = payload.to_xml();
        assert!(
            xml.contains("</description>\n  <accept_at>"),
            "accept_at must follow the description, osc order: {xml}"
        );
        assert!(
            xml.ends_with("</accept_at>\n</request>"),
            "accept_at must close the document: {xml}"
        );
        assert!(
            xml.ends_with("+00:00</accept_at>\n</request>"),
            "accept_at must carry a UTC offset: {xml}"
        );
        assert!(payload.validate_xml().is_ok());
        assert!(payload.accept_at().is_some());
        // Without the timer the rendering is unchanged.
        let plain = DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Package("foo".to_string()),
            "Drop foo",
        )
        .expect("test delete payload");
        assert!(!plain.to_xml().contains("accept_at"));
        assert!(plain.accept_at().is_none());
        assert!(plain.validate_xml().is_ok());
    }

    #[test]
    fn iso8601_utc_conversion_matches_known_instants() {
        // Epoch, a second before Y2K, the 400-year leap day, a
        // 4-year leap day, a plain instant, the century non-leap
        // year (2100 has no Feb 29, so Mar 1 follows Feb 28), and
        // the 32-bit rollover.
        assert_eq!(iso8601_utc_from_unix(0), "1970-01-01T00:00:00+00:00");
        assert_eq!(
            iso8601_utc_from_unix(946_684_799),
            "1999-12-31T23:59:59+00:00"
        );
        assert_eq!(
            iso8601_utc_from_unix(951_782_400),
            "2000-02-29T00:00:00+00:00"
        );
        assert_eq!(
            iso8601_utc_from_unix(1_709_164_800),
            "2024-02-29T00:00:00+00:00"
        );
        assert_eq!(
            iso8601_utc_from_unix(1_795_350_896),
            "2026-11-22T12:34:56+00:00"
        );
        assert_eq!(
            iso8601_utc_from_unix(4_107_542_400),
            "2100-03-01T00:00:00+00:00"
        );
        assert_eq!(
            iso8601_utc_from_unix(2_147_483_647),
            "2038-01-19T03:14:07+00:00"
        );
    }

    #[test]
    fn accept_at_in_hours_rejects_overflow() {
        let err = DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Package("foo".to_string()),
            "Drop foo",
        )
        .expect("test delete payload")
        .with_accept_at_in_hours(u64::MAX)
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("too many"), "{}", err.context);
    }

    #[test]
    fn accept_at_in_hours_year_bound() {
        // ~6,900 years out is still representable (year ~8870);
        // ~11,400 years out lands past year 9999, the most the
        // four-digit format carries.
        let payload = DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Package("foo".to_string()),
            "Drop foo",
        )
        .expect("test delete payload");
        assert!(payload.clone().with_accept_at_in_hours(60_000_000).is_ok());
        let err = payload.with_accept_at_in_hours(100_000_000).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("too many"), "{}", err.context);
    }

    #[test]
    fn payload_cve_rule_requires_the_entry_to_name_each_cve() {
        // The entry names the CVE: accepted.
        assert!(payload_with_cve().to_xml().contains("CVE-2026-12345"));
        // The entry does not name the CVE: refused.
        let cve = CveId::parse("CVE-2026-12345").unwrap();
        assert!(SrPayload::new(
            "home:scarabeus_iv",
            "foo",
            "devel:languages:python",
            "foo",
            "Update foo to 2.0: fix CVE-2026-12345",
            "- Update to version 2.0.\n",
            &[cve],
        )
        .is_err());
    }

    #[test]
    fn file_dry_run_returns_none_and_makes_no_requests() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::default();
        let filing = SrFiling::new(&obs, &dry);
        let payload = payload();
        assert_eq!(filing.file(&payload).unwrap(), None);
        // Nothing was read or filed: no GET (open-request search) and no POST.
        assert!(http.get_calls().is_empty());
        assert!(http.post_calls().is_empty());
        // The would-do line plus the full would-be request, recorded
        // in the plan (not just printed).
        assert_eq!(
            dry.plan(),
            vec![
                "[dry-run] would: file submit request for foo to devel:languages:python"
                    .to_string(),
                "[dry-run] --- would-be submit request (dry-run) ---".to_string(),
                "[dry-run] source: home:scarabeus_iv/foo".to_string(),
                "[dry-run] target: devel:languages:python/foo".to_string(),
                "[dry-run] description: Update foo to 2.0".to_string(),
                format!("[dry-run] changes entry: {}", payload.changes_entry()),
                format!("[dry-run] {}", payload.to_xml()),
                "[dry-run] --- end of would-be submit request ---".to_string(),
                "[dry-run] would: supersede all open submit requests targeting foo with the new request"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn file_live_returns_id_when_no_open_sr() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing.file(&payload()).unwrap(),
            Some(FiledRequest {
                id: "1400001".to_string(),
                superseded: vec![],
            })
        );
        // The open-request search ran first, then the filing POST.
        assert_eq!(http.get_calls().len(), 1);
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn file_live_supersedes_open_own_sr() {
        // Supersede-all: an open SR from the same user already
        // targets the package, so filing files the new request and
        // supersedes the open one with it — no refusal, no POST
        // skipped.
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"1\"><request id=\"1398492\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
            )
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            )
            .with_post(
                "https://api.opensuse.org/request/1398492?cmd=changestate&newstate=superseded&superseded_by=1400001",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing.file(&payload()).unwrap(),
            Some(FiledRequest {
                id: "1400001".to_string(),
                superseded: vec!["1398492".to_string()],
            })
        );
        // The open-request search ran first, then the filing POST,
        // then the supersede POST — file before supersede, so
        // superseded_by always names a request that exists.
        assert_eq!(http.get_calls().len(), 1);
        let posted = http.post_calls();
        assert_eq!(posted.len(), 2);
        assert_eq!(posted[0].0, "https://api.opensuse.org/request?cmd=create");
        assert_eq!(
            posted[1].0,
            "https://api.opensuse.org/request/1398492?cmd=changestate&newstate=superseded&superseded_by=1400001"
        );
        assert_eq!(posted[1].1, "superseded by 1400001");
    }

    #[test]
    fn file_live_skips_other_users_open_sr() {
        // The supersede set is the filer's own requests only: an
        // open SR from another user targeting the package stays
        // open, and no supersede POST is made for it.
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"1\"><request id=\"1398492\" creator=\"someone_else\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
            )
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing.file(&payload()).unwrap(),
            Some(FiledRequest {
                id: "1400001".to_string(),
                superseded: vec![],
            })
        );
        // Only the filing POST: nothing was superseded.
        assert_eq!(http.post_calls().len(), 1);
        assert_eq!(
            http.post_calls()[0].0,
            "https://api.opensuse.org/request?cmd=create"
        );
    }

    #[test]
    fn file_live_supersede_failure_names_new_id_and_remaining() {
        // Two open own SRs; the first supersede succeeds and the
        // second fails: the error names the new id, the failed id,
        // and the ids still open.
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"2\"><request id=\"1398492\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request><request id=\"1398493\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
            )
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            )
            .with_post(
                "https://api.opensuse.org/request/1398492?cmd=changestate&newstate=superseded&superseded_by=1400001",
                "<status code=\"ok\" />",
            )
            .with_post(
                "https://api.opensuse.org/request/1398493?cmd=changestate&newstate=superseded&superseded_by=1400001",
                "<status code=\"not_found\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing.file(&payload()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("1400001"), "error names the new id");
        assert!(err.context.contains("1398493"), "error names the failed id");
        assert!(
            !err.context.contains("1398492"),
            "already-superseded id is not still open"
        );
        // All three POSTs were attempted: the filing plus both
        // supersedes, in order.
        assert_eq!(http.post_calls().len(), 3);
    }

    #[test]
    fn file_live_transport_failure_is_error() {
        // Live mode, credentials present, the open-request search is
        // empty, but the POST itself fails (no canned response =
        // transport error): filing returns Err, it does not silently
        // report a filed id.
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs.search_url("foo").unwrap();
        let http = FakeHttp::new().with_get(&search_url, "<collection matches=\"0\"></collection>");
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert!(filing.file(&payload()).is_err());
    }

    fn refile_http() -> FakeHttp {
        // The canned responses for a live refile: the open-request
        // search is empty, filing creates 1400001, superseding
        // 1397357 with it succeeds.
        let probe_http = FakeHttp::new();
        let obs_probe = ObsClient::with_credentials(&probe_http, basic_credentials()).unwrap();
        let search_url = obs_probe.search_url("foo").unwrap();
        FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            )
            .with_post(
                "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1400001",
                "<status code=\"ok\" />",
            )
    }

    #[test]
    fn refile_live_files_then_supersedes_in_order() {
        let http = refile_http();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing.refile(&payload(), "1397357").unwrap(),
            Some(FiledRequest {
                id: "1400001".to_string(),
                superseded: vec!["1397357".to_string()],
            })
        );
        // Open-request search first, then the filing POST, then the
        // supersede POST — file before supersede, so superseded_by
        // always names a request that exists.
        assert_eq!(http.get_calls().len(), 1);
        let posted = http.post_calls();
        assert_eq!(posted.len(), 2);
        assert_eq!(posted[0].0, "https://api.opensuse.org/request?cmd=create");
        assert_eq!(
            posted[1].0,
            "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1400001"
        );
        assert_eq!(posted[1].1, "superseded by 1400001");
    }

    #[test]
    fn refile_live_files_when_old_is_the_only_open_sr() {
        // The old request is open for the same package, so it is in
        // the open list and the refile supersedes it along with any
        // others — a refile over an open request files first, then
        // supersedes.
        let probe_http = FakeHttp::new();
        let obs_probe = ObsClient::with_credentials(&probe_http, basic_credentials()).unwrap();
        let search_url = obs_probe.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"1\"><request id=\"1397357\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
            )
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            )
            .with_post(
                "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1400001",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing.refile(&payload(), "1397357").unwrap(),
            Some(FiledRequest {
                id: "1400001".to_string(),
                superseded: vec!["1397357".to_string()],
            })
        );
        assert_eq!(http.post_calls().len(), 2);
    }

    #[test]
    fn refile_live_supersedes_all_open_srs() {
        // Supersede-all: a second open request for the package no
        // longer blocks the filing — the refile files the new
        // request and supersedes the named old one plus the other
        // open one, the named one first.
        let probe_http = FakeHttp::new();
        let obs_probe = ObsClient::with_credentials(&probe_http, basic_credentials()).unwrap();
        let search_url = obs_probe.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"2\"><request id=\"1397357\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request><request id=\"1398492\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
            )
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            )
            .with_post(
                "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1400001",
                "<status code=\"ok\" />",
            )
            .with_post(
                "https://api.opensuse.org/request/1398492?cmd=changestate&newstate=superseded&superseded_by=1400001",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing.refile(&payload(), "1397357").unwrap(),
            Some(FiledRequest {
                id: "1400001".to_string(),
                superseded: vec!["1397357".to_string(), "1398492".to_string()],
            })
        );
        // Filing POST first, then both supersedes in order — file
        // before supersede, so superseded_by always names a request
        // that exists.
        let posted = http.post_calls();
        assert_eq!(posted.len(), 3);
        assert_eq!(posted[0].0, "https://api.opensuse.org/request?cmd=create");
        assert_eq!(
            posted[1].0,
            "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1400001"
        );
        assert_eq!(
            posted[2].0,
            "https://api.opensuse.org/request/1398492?cmd=changestate&newstate=superseded&superseded_by=1400001"
        );
    }

    #[test]
    fn refile_dry_run_returns_none_and_records_both_steps() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::default();
        let filing = SrFiling::new(&obs, &dry);
        let payload = payload();
        assert_eq!(filing.refile(&payload, "1397357").unwrap(), None);
        assert!(http.get_calls().is_empty());
        assert!(http.post_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec![
                "[dry-run] would: file submit request for foo to devel:languages:python"
                    .to_string(),
                "[dry-run] --- would-be submit request (dry-run) ---".to_string(),
                "[dry-run] source: home:scarabeus_iv/foo".to_string(),
                "[dry-run] target: devel:languages:python/foo".to_string(),
                "[dry-run] description: Update foo to 2.0".to_string(),
                format!("[dry-run] changes entry: {}", payload.changes_entry()),
                format!("[dry-run] {}", payload.to_xml()),
                "[dry-run] --- end of would-be submit request ---".to_string(),
                "[dry-run] would: supersede submit request 1397357 with the new request"
                    .to_string(),
                "[dry-run] would: supersede all other open submit requests targeting foo with the new request"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn refile_supersede_failure_names_the_new_id() {
        // Filing succeeds but the supersede does not: the error must
        // carry the new id so the caller can finish manually — the
        // old request is still open and the new one unlinked.
        let probe_http = FakeHttp::new();
        let obs_probe = ObsClient::with_credentials(&probe_http, basic_credentials()).unwrap();
        let search_url = obs_probe.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            )
            .with_post(
                "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1400001",
                "<status code=\"not_found\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing.refile(&payload(), "1397357").unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ServerError);
        assert!(err.context.contains("1400001"), "error names the new id");
        assert!(err.context.contains("1397357"), "error names the old id");
    }

    #[test]
    fn refile_rejects_invalid_old_id_without_http() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert!(filing.refile(&payload(), "not-an-id").is_err());
        assert!(http.get_calls().is_empty());
        assert!(http.post_calls().is_empty());
    }

    /// The search URL for an action-type dedup check, built from the
    /// same match expression the client builds: state plus the
    /// action's source/target parts, in production order.
    fn action_match_url(action_type: &str, parts: &[&str]) -> String {
        let match_expr = format!(
            "(state/@name='new' or state/@name='review') and action/@type='{action_type}' and action[{}]",
            parts.join(" and ")
        );
        build_url(
            "https://api.opensuse.org",
            &["search", "request"],
            &[("match", &match_expr)],
        )
    }

    fn change_devel_payload() -> ChangeDevelPayload {
        ChangeDevelPayload::new(
            "devel:languages:python",
            "foo",
            "openSUSE:Factory",
            "foo",
            "Move foo to Factory",
        )
        .expect("test change devel payload")
    }

    fn delete_package_payload() -> DeletePayload {
        DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Package("foo".to_string()),
            "Drop foo",
        )
        .expect("test delete payload")
    }

    fn delete_repository_payload() -> DeletePayload {
        DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::Repository("standard".to_string()),
            "Drop the standard repository",
        )
        .expect("test delete payload")
    }

    fn delete_project_payload() -> DeletePayload {
        DeletePayload::new(
            "openSUSE:Factory",
            DeleteTarget::WholeProject,
            "Drop Factory",
        )
        .expect("test delete payload")
    }

    fn incident_payload_with_release_project() -> MaintenanceIncidentPayload {
        incident_payload()
            .with_release_project("openSUSE:Leap:15.6:Update")
            .expect("test incident release project")
    }

    #[test]
    fn file_maintenance_incident_dry_run_returns_none_and_makes_no_requests() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::default();
        let filing = SrFiling::new(&obs, &dry);
        let payload = incident_payload();
        assert_eq!(filing.file_maintenance_incident(&payload).unwrap(), None);
        assert!(http.get_calls().is_empty());
        assert!(http.post_calls().is_empty());
        assert_eq!(
            dry.plan(),
            vec![
                "[dry-run] would: file maintenance incident request for home:alice:branches:OBS_Maintained:foo/foo to openSUSE:Maintenance"
                    .to_string(),
                "[dry-run] --- would-be maintenance incident request (dry-run) ---".to_string(),
                "[dry-run] source: home:alice:branches:OBS_Maintained:foo/foo".to_string(),
                "[dry-run] target: openSUSE:Maintenance".to_string(),
                "[dry-run] description: Maintenance incident for foo".to_string(),
                format!("[dry-run] {}", payload.to_xml()),
                "[dry-run] --- end of would-be maintenance incident request ---".to_string(),
            ]
        );
    }

    #[test]
    fn file_maintenance_incident_live_files_with_add_revision() {
        let search_url = action_match_url(
            "maintenance_incident",
            &[
                "source/@package='foo'",
                "target/@project='openSUSE:Maintenance'",
            ],
        );
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create&addrevision=1",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let payload = incident_payload();
        assert_eq!(
            filing.file_maintenance_incident(&payload).unwrap(),
            Some("1400001".to_string())
        );
        // The dedup search ran first, then the filing POST — with
        // addrevision=1, which pins source_rev to the filed source.
        assert_eq!(http.get_calls().len(), 1);
        let posted = http.post_calls();
        assert_eq!(posted.len(), 1);
        assert_eq!(
            posted[0].0,
            "https://api.opensuse.org/request?cmd=create&addrevision=1"
        );
        assert_eq!(posted[0].1, payload.to_xml());
    }

    #[test]
    fn file_maintenance_incident_live_refuses_when_open_incident_exists() {
        let search_url = action_match_url(
            "maintenance_incident",
            &[
                "source/@package='foo'",
                "target/@project='openSUSE:Maintenance'",
            ],
        );
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399001\" creator=\"alice\"><action type=\"maintenance_incident\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing
            .file_maintenance_incident(&incident_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn file_maintenance_incident_live_refuses_when_open_incident_from_other_branch_exists() {
        // The server's incident identity has no source project, so an
        // equivalent incident filed from another branch blocks too.
        let search_url = action_match_url(
            "maintenance_incident",
            &[
                "source/@package='foo'",
                "target/@project='openSUSE:Maintenance'",
            ],
        );
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399001\" creator=\"bob\"><action type=\"maintenance_incident\"><source project=\"home:bob:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing
            .file_maintenance_incident(&incident_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn file_maintenance_incident_live_files_when_open_incident_has_other_release_project() {
        // The open incident releases elsewhere: the server's incident
        // identity includes the release project, so this is a
        // different incident, not a duplicate.
        let search_url = action_match_url(
            "maintenance_incident",
            &[
                "source/@package='foo'",
                "target/@project='openSUSE:Maintenance'",
            ],
        );
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"1\"><request id=\"1399001\" creator=\"alice\"><action type=\"maintenance_incident\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"new\" /></request></collection>",
            )
            .with_post(
                "https://api.opensuse.org/request?cmd=create&addrevision=1",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing
                .file_maintenance_incident(&incident_payload_with_release_project())
                .unwrap(),
            Some("1400001".to_string())
        );
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn refile_maintenance_incident_live_files_then_supersedes_in_order() {
        let search_url = action_match_url(
            "maintenance_incident",
            &[
                "source/@package='foo'",
                "target/@project='openSUSE:Maintenance'",
            ],
        );
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"1\"><request id=\"1397357\" creator=\"alice\"><action type=\"maintenance_incident\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"new\" /></request></collection>",
            )
            .with_post(
                "https://api.opensuse.org/request?cmd=create&addrevision=1",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            )
            .with_post(
                "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1400001",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing
                .refile_maintenance_incident(&incident_payload(), "1397357")
                .unwrap(),
            Some("1400001".to_string())
        );
        // The old incident is open for the same source package,
        // but the refile exempts it: dedup first, then the filing
        // POST, then the supersede POST.
        assert_eq!(http.get_calls().len(), 1);
        let posted = http.post_calls();
        assert_eq!(posted.len(), 2);
        assert_eq!(
            posted[0].0,
            "https://api.opensuse.org/request?cmd=create&addrevision=1"
        );
        assert_eq!(
            posted[1].0,
            "https://api.opensuse.org/request/1397357?cmd=changestate&newstate=superseded&superseded_by=1400001"
        );
    }

    #[test]
    fn refile_maintenance_incident_dry_run_returns_none_and_records_both_steps() {
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::default();
        let filing = SrFiling::new(&obs, &dry);
        let payload = incident_payload();
        assert_eq!(
            filing
                .refile_maintenance_incident(&payload, "1397357")
                .unwrap(),
            None
        );
        assert!(http.get_calls().is_empty());
        assert!(http.post_calls().is_empty());
        assert!(dry.plan().iter().any(|line| line
            == "[dry-run] would: file maintenance incident request for home:alice:branches:OBS_Maintained:foo/foo to openSUSE:Maintenance"));
        assert!(dry.plan().iter().any(|line| line
            == "[dry-run] would: supersede maintenance incident request 1397357 with the new request"));
    }

    #[test]
    fn file_change_devel_live_returns_id() {
        let search_url = action_match_url(
            "change_devel",
            &[
                "target/@project='openSUSE:Factory'",
                "target/@package='foo'",
            ],
        );
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing.file_change_devel(&change_devel_payload()).unwrap(),
            Some("1400001".to_string())
        );
        assert_eq!(http.get_calls().len(), 1);
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn file_change_devel_live_refuses_when_open_request_exists() {
        let search_url = action_match_url(
            "change_devel",
            &[
                "target/@project='openSUSE:Factory'",
                "target/@package='foo'",
            ],
        );
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399002\" creator=\"alice\"><action type=\"change_devel\"><source project=\"devel:languages:python\" package=\"foo\" /><target project=\"openSUSE:Factory\" package=\"foo\" /></action><state name=\"review\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing
            .file_change_devel(&change_devel_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn file_change_devel_live_refuses_when_open_request_names_other_source() {
        // The server's change-devel identity is the target alone, so
        // an open request naming another new devel blocks too.
        let search_url = action_match_url(
            "change_devel",
            &[
                "target/@project='openSUSE:Factory'",
                "target/@package='foo'",
            ],
        );
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399002\" creator=\"alice\"><action type=\"change_devel\"><source project=\"devel:languages:go\" package=\"foo\" /><target project=\"openSUSE:Factory\" package=\"foo\" /></action><state name=\"review\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing
            .file_change_devel(&change_devel_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn file_delete_request_live_returns_id_for_package() {
        let search_url = action_match_url("delete", &["target/@project='openSUSE:Factory'"]);
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing
                .file_delete_request(&delete_package_payload())
                .unwrap(),
            Some("1400001".to_string())
        );
        assert_eq!(http.get_calls().len(), 1);
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn file_delete_request_live_refuses_when_open_delete_exists() {
        let search_url = action_match_url("delete", &["target/@project='openSUSE:Factory'"]);
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399003\" creator=\"alice\"><action type=\"delete\"><target project=\"openSUSE:Factory\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing
            .file_delete_request(&delete_package_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn file_delete_request_live_files_repository_target() {
        // A repository delete has no package in the target: the
        // dedup match carries only the target project, and the parser
        // narrows by shape instead.
        let search_url = action_match_url("delete", &["target/@project='openSUSE:Factory'"]);
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing
                .file_delete_request(&delete_repository_payload())
                .unwrap(),
            Some("1400001".to_string())
        );
        assert_eq!(http.get_calls().len(), 1);
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn file_delete_request_live_refuses_when_open_whole_project_delete_exists() {
        // An open whole-project delete covers every package in the
        // project: filing a package delete under it is refused.
        let search_url = action_match_url("delete", &["target/@project='openSUSE:Factory'"]);
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399004\" creator=\"alice\"><action type=\"delete\"><target project=\"openSUSE:Factory\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing
            .file_delete_request(&delete_package_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.context.contains("1399004"));
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn file_delete_request_live_files_whole_project_when_open_package_delete_exists() {
        // The reverse is not a conflict: an open package delete does
        // not block deleting the whole project.
        let search_url = action_match_url("delete", &["target/@project='openSUSE:Factory'"]);
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"1\"><request id=\"1399004\" creator=\"alice\"><action type=\"delete\"><target project=\"openSUSE:Factory\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
            )
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        assert_eq!(
            filing
                .file_delete_request(&delete_project_payload())
                .unwrap(),
            Some("1400001".to_string())
        );
        assert_eq!(http.post_calls().len(), 1);
    }

    #[test]
    fn file_delete_request_live_refuses_whole_project_when_open_repository_delete_exists() {
        // A repository delete and a whole-project delete share the
        // server's delete identity, so the open repository delete
        // blocks deleting the whole project.
        let search_url = action_match_url("delete", &["target/@project='openSUSE:Factory'"]);
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399004\" creator=\"alice\"><action type=\"delete\"><target project=\"openSUSE:Factory\" repository=\"standard\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry);
        let err = filing
            .file_delete_request(&delete_project_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.post_calls().is_empty());
    }

    #[test]
    fn maintenance_branch_options_reject_invalid_values() {
        assert!(MaintenanceBranchOptions::new()
            .with_attribute("no-colon")
            .is_err());
        assert!(MaintenanceBranchOptions::new()
            .with_attribute("OBS:")
            .is_err());
        assert!(MaintenanceBranchOptions::new()
            .with_attribute(":Maintained")
            .is_err());
        assert!(MaintenanceBranchOptions::new()
            .with_update_project_attribute("a:b:c")
            .is_err());
        assert!(MaintenanceBranchOptions::new()
            .with_target_project("foo/bar")
            .is_err());
        // The defaults are the server defaults.
        let options = MaintenanceBranchOptions::new();
        assert_eq!(options.attribute, "OBS:Maintained");
        assert_eq!(options.update_project_attribute, "OBS:UpdateProject");
        assert_eq!(options.target_project, None);
    }

    #[test]
    fn debug_file_reports_the_filing_decision() {
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry).with_debug(debug);
        assert_eq!(
            filing.file(&payload()).unwrap(),
            Some(FiledRequest {
                id: "1400001".to_string(),
                superseded: vec![],
            })
        );
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            ["decision: no open submit request targets foo; filing".to_string()]
        );
    }

    #[test]
    fn debug_file_reports_the_supersede_decision() {
        let (debug, lines) = Debug::captured(true, false);
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(
                &search_url,
                "<collection matches=\"1\"><request id=\"1398492\" creator=\"scarabeus_iv\"><action type=\"submit\"><source project=\"home:scarabeus_iv\" package=\"foo\" /><target project=\"devel:languages:python\" package=\"foo\" /></action><state name=\"new\" /></request></collection>",
            )
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            )
            .with_post(
                "https://api.opensuse.org/request/1398492?cmd=changestate&newstate=superseded&superseded_by=1400001",
                "<status code=\"ok\" />",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry).with_debug(debug);
        assert_eq!(
            filing.file(&payload()).unwrap(),
            Some(FiledRequest {
                id: "1400001".to_string(),
                superseded: vec!["1398492".to_string()],
            })
        );
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            [
                "decision: filing for foo; superseding open submit request(s) 1398492 with the new request"
                    .to_string()
            ]
        );
    }

    #[test]
    fn debug_file_reports_nothing_when_debug_is_off() {
        let (debug, lines) = Debug::captured(false, false);
        let http = FakeHttp::new();
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let search_url = obs.search_url("foo").unwrap();
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create",
                "<request id=\"1400001\" creator=\"scarabeus_iv\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry).with_debug(debug);
        assert_eq!(
            filing.file(&payload()).unwrap(),
            Some(FiledRequest {
                id: "1400001".to_string(),
                superseded: vec![],
            })
        );
        assert!(lines.lock().unwrap().is_empty());
    }

    #[test]
    fn debug_maintenance_incident_reports_the_dedup_decision() {
        let (debug, lines) = Debug::captured(true, false);
        let search_url = action_match_url(
            "maintenance_incident",
            &[
                "source/@package='foo'",
                "target/@project='openSUSE:Maintenance'",
            ],
        );
        let http = FakeHttp::new()
            .with_get(&search_url, "<collection matches=\"0\"></collection>")
            .with_post(
                "https://api.opensuse.org/request?cmd=create&addrevision=1",
                "<request id=\"1400001\" creator=\"alice\"></request>",
            );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry).with_debug(debug);
        let payload = incident_payload();
        assert_eq!(
            filing.file_maintenance_incident(&payload).unwrap(),
            Some("1400001".to_string())
        );
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            ["decision: no open maintenance_incident request targets openSUSE:Maintenance for foo; filing"
                .to_string()]
        );
    }

    #[test]
    fn debug_maintenance_incident_reports_the_dedup_refusal() {
        let (debug, lines) = Debug::captured(true, false);
        let search_url = action_match_url(
            "maintenance_incident",
            &[
                "source/@package='foo'",
                "target/@project='openSUSE:Maintenance'",
            ],
        );
        let http = FakeHttp::new().with_get(
            &search_url,
            "<collection matches=\"1\"><request id=\"1399001\" creator=\"alice\"><action type=\"maintenance_incident\"><source project=\"home:alice:branches:OBS_Maintained:foo\" package=\"foo\" /><target project=\"openSUSE:Maintenance\" /></action><state name=\"new\" /></request></collection>",
        );
        let obs = ObsClient::with_credentials(&http, basic_credentials()).unwrap();
        let dry = DryRun::new(false);
        let filing = SrFiling::new(&obs, &dry).with_debug(debug);
        let err = filing
            .file_maintenance_incident(&incident_payload())
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        let got: Vec<String> = lines.lock().unwrap().clone();
        assert_eq!(
            got,
            ["decision: open maintenance_incident request(s) 1399001 already target openSUSE:Maintenance for foo; refusing to file"
                .to_string()]
        );
    }
}
