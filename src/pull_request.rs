//! Unified pull requests: one interface for an OBS submit request and a
//! Gitea pull request.
//!
//! A pull request is named by a backend-neutral handle
//! ([`PullRequestRef`]) and read as one shape ([`PullRequest`]) on both
//! backends, with a unified status ([`PullRequestStatus`]). This module
//! carries the handle, the shapes, and the status mappings.

use crate::error::{ErrorKind, GeectlError};
use crate::gitea::{
    validate_gitea_name, GiteaClient, GiteaIssue, GiteaPullRequest, GiteaPullRequestRef,
    GiteaPullRequestState, GiteaReview,
};
use crate::obs::request::{
    validate_request_id, Request, RequestAction, RequestSearch, RequestState, ReviewState,
    ReviewTarget,
};
use crate::obs::ObsClient;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A pull request, named by a backend-neutral handle. The backend kind is
/// the variant; the id is the payload. An OBS request id is globally
/// unique; a Gitea PR number is unique per repo, so the Gitea variant
/// carries the repo with it. The handle carries no host: the configured
/// instance supplies it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum PullRequestRef {
    /// An OBS submit request, by its number.
    Obs(u64),
    /// A Gitea pull request, by repo and number on the configured instance.
    Gitea {
        /// The repo owner.
        owner: String,
        /// The repo name.
        repo: String,
        /// The pull request number.
        number: u64,
    },
}

impl fmt::Display for PullRequestRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PullRequestRef::Obs(id) => write!(f, "obs:{id}"),
            PullRequestRef::Gitea {
                owner,
                repo,
                number,
            } => write!(f, "gitea:{owner}/{repo}#{number}"),
        }
    }
}

impl From<PullRequestRef> for String {
    fn from(value: PullRequestRef) -> Self {
        value.to_string()
    }
}

impl FromStr for PullRequestRef {
    type Err = GeectlError;

    /// Parse the one text form: `obs:<id>` or
    /// `gitea:<owner>/<repo>#<number>`. Anything else is an invalid
    /// input naming the handle.
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if let Some(id) = input.strip_prefix("obs:") {
            return Ok(PullRequestRef::Obs(validate_request_id(id)?));
        }
        if let Some(rest) = input.strip_prefix("gitea:") {
            let (repo_part, number) = rest.split_once('#').ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid pull-request handle: {input:?}"),
                )
            })?;
            let (owner, repo) = repo_part.split_once('/').ok_or_else(|| {
                GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid pull-request handle: {input:?}"),
                )
            })?;
            validate_gitea_name(owner, "owner")?;
            validate_gitea_name(repo, "repo")?;
            let number = validate_request_id(number).map_err(|_| {
                GeectlError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid Gitea pull-request number: {number:?}"),
                )
            })?;
            return Ok(PullRequestRef::Gitea {
                owner: owner.to_string(),
                repo: repo.to_string(),
                number,
            });
        }
        Err(GeectlError::new(
            ErrorKind::InvalidInput,
            format!("invalid pull-request handle: {input:?}"),
        ))
    }
}

impl TryFrom<String> for PullRequestRef {
    type Error = GeectlError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

/// The unified state of a pull request: the union of both backends'
/// states. A Gitea close carries no recorded outcome, so it reads as
/// `Closed`, never as `Declined` or `Withdrawn`: the decline and the
/// withdrawal are different operations with different OBS mappings, and
/// the Gitea side records neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestStatus {
    /// Open for review: OBS `new`/`review`, Gitea `open`.
    Open,
    /// Landed: OBS `accepted`, Gitea closed and merged.
    Merged,
    /// Refused: OBS `declined`. Gitea has no PR state for this.
    Declined,
    /// Withdrawn by its creator: OBS `revoked`. Gitea has no PR state
    /// for this.
    Withdrawn,
    /// Closed without merging: Gitea closed, not merged. OBS has
    /// no closed state: its declined requests read as `declined`,
    /// so OBS never returns `closed`.
    Closed,
    /// Replaced by a newer request: OBS `superseded`. Gitea has no
    /// PR state for this.
    Superseded,
    /// Deleted: OBS `deleted`. Gitea has no PR state for this.
    Deleted,
}

impl PullRequestStatus {
    /// The lowercase spelling of this state, as it appears in JSON and
    /// in `--state` filters.
    pub fn as_str(&self) -> &'static str {
        match self {
            PullRequestStatus::Open => "open",
            PullRequestStatus::Merged => "merged",
            PullRequestStatus::Declined => "declined",
            PullRequestStatus::Withdrawn => "withdrawn",
            PullRequestStatus::Closed => "closed",
            PullRequestStatus::Superseded => "superseded",
            PullRequestStatus::Deleted => "deleted",
        }
    }

    /// The unified state of a Gitea pull request: `state` is the wire
    /// `state` (`open`/`closed`), `merged` the wire `merged` flag. An
    /// unknown wire state is a server error naming the state.
    pub fn from_gitea(state: &str, merged: bool) -> Result<Self, GeectlError> {
        match (state, merged) {
            ("open", _) => Ok(PullRequestStatus::Open),
            ("closed", true) => Ok(PullRequestStatus::Merged),
            ("closed", false) => Ok(PullRequestStatus::Closed),
            _ => Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unknown Gitea pull-request state: {state:?}"),
            )),
        }
    }
}

impl FromStr for PullRequestStatus {
    type Err = GeectlError;

    /// Parse the lowercase spelling (`open`, `merged`, …). Anything
    /// else is an invalid input naming the state.
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input {
            "open" => Ok(PullRequestStatus::Open),
            "merged" => Ok(PullRequestStatus::Merged),
            "declined" => Ok(PullRequestStatus::Declined),
            "withdrawn" => Ok(PullRequestStatus::Withdrawn),
            "closed" => Ok(PullRequestStatus::Closed),
            "superseded" => Ok(PullRequestStatus::Superseded),
            "deleted" => Ok(PullRequestStatus::Deleted),
            _ => Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("unknown pull-request state: {input:?}"),
            )),
        }
    }
}

/// Which backend a pull-request operation runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PullRequestBackend {
    /// OBS submit requests.
    Obs,
    /// Gitea pull requests.
    Gitea,
}

impl PullRequestBackend {
    /// The lowercase spelling of this backend, as it appears in
    /// `--backend` filters.
    pub fn as_str(&self) -> &'static str {
        match self {
            PullRequestBackend::Obs => "obs",
            PullRequestBackend::Gitea => "gitea",
        }
    }
}

impl FromStr for PullRequestBackend {
    type Err = GeectlError;

    /// Parse the lowercase spelling (`obs`, `gitea`). Anything else
    /// is an invalid input naming the backend.
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input {
            "obs" => Ok(PullRequestBackend::Obs),
            "gitea" => Ok(PullRequestBackend::Gitea),
            _ => Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("unknown pull-request backend: {input:?}"),
            )),
        }
    }
}

impl From<RequestState> for PullRequestStatus {
    fn from(state: RequestState) -> Self {
        match state {
            RequestState::New | RequestState::Review => PullRequestStatus::Open,
            RequestState::Accepted => PullRequestStatus::Merged,
            RequestState::Declined => PullRequestStatus::Declined,
            RequestState::Revoked => PullRequestStatus::Withdrawn,
            RequestState::Superseded => PullRequestStatus::Superseded,
            RequestState::Deleted => PullRequestStatus::Deleted,
        }
    }
}

/// One review on a pull request: the union of the per-review states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpinionState {
    /// Asked for; nobody has acted on it yet.
    Asked,
    /// Approved by the reviewer.
    Approved,
    /// Changes requested by the reviewer.
    ChangesRequested,
    /// A comment review, with no approval verdict.
    Commented,
    /// Dismissed by a maintainer.
    Dismissed,
    /// Superseded by a newer review of the same target.
    Superseded,
    /// The reviewer was removed, so the review no longer applies.
    Obsolete,
}

impl OpinionState {
    /// The lowercase spelling of this state, as it appears in JSON.
    pub fn as_str(&self) -> &'static str {
        match self {
            OpinionState::Asked => "asked",
            OpinionState::Approved => "approved",
            OpinionState::ChangesRequested => "changes_requested",
            OpinionState::Commented => "commented",
            OpinionState::Dismissed => "dismissed",
            OpinionState::Superseded => "superseded",
            OpinionState::Obsolete => "obsolete",
        }
    }

    /// The unified state of a Gitea review: `state` is the wire `state`,
    /// `dismissed` the wire `dismissed` flag. A dismissed review reads
    /// as `Dismissed` whatever its state was. A requested review
    /// (`REQUEST_REVIEW`) is an outstanding ask, so it reads as `Asked`.
    /// An unknown wire state is a server error naming the state.
    pub fn from_gitea(state: &str, dismissed: bool) -> Result<Self, GeectlError> {
        if dismissed {
            return Ok(OpinionState::Dismissed);
        }
        match state {
            "PENDING" | "REQUEST_REVIEW" => Ok(OpinionState::Asked),
            "APPROVED" => Ok(OpinionState::Approved),
            "REQUEST_CHANGES" => Ok(OpinionState::ChangesRequested),
            "COMMENT" => Ok(OpinionState::Commented),
            _ => Err(GeectlError::new(
                ErrorKind::ServerError,
                format!("unknown Gitea review state: {state:?}"),
            )),
        }
    }
}

impl From<ReviewState> for OpinionState {
    fn from(state: ReviewState) -> Self {
        match state {
            ReviewState::New => OpinionState::Asked,
            ReviewState::Accepted => OpinionState::Approved,
            ReviewState::Declined => OpinionState::ChangesRequested,
            ReviewState::Superseded => OpinionState::Superseded,
            ReviewState::Obsoleted => OpinionState::Obsolete,
        }
    }
}

/// One endpoint of a pull request: the repository it comes from or goes
/// to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestEndpoint {
    /// `project/package` on OBS, `owner/repo` on Gitea.
    #[serde(default)]
    pub repository: String,
    /// The branch, on Gitea; `None` on OBS.
    #[serde(default)]
    pub branch: Option<String>,
    /// The head/base SHA on Gitea; the OBS source revision when the
    /// action carries one.
    #[serde(default)]
    pub commit: Option<String>,
}

/// One review on a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewOpinion {
    /// The asked-of target on OBS (user, group, project, or package);
    /// the reviewer login on Gitea.
    #[serde(default)]
    pub reviewer: String,
    /// The review's state.
    pub state: OpinionState,
    /// The review's comment, when it carries one.
    #[serde(default)]
    pub comment: String,
    /// When the review was given, when the backend records it.
    #[serde(default)]
    pub when: Option<String>,
}

/// One comment on a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewComment {
    /// The comment id.
    pub id: u64,
    /// The comment author.
    #[serde(default)]
    pub author: String,
    /// When the comment was created.
    #[serde(default)]
    pub created: String,
    /// The comment body.
    #[serde(default)]
    pub body: String,
    /// The parent comment, for OBS threading; `None` on Gitea — issue
    /// comments do not thread.
    #[serde(default)]
    pub parent_id: Option<u64>,
}

/// One pull request, read the same way on both backends. Fields a
/// backend does not have are defaulted (empty/false/`None`), so the
/// JSON keys are the same on every read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    /// The handle.
    pub reference: PullRequestRef,
    /// The title.
    #[serde(default)]
    pub title: String,
    /// The description (OBS) / body (Gitea); empty when none.
    #[serde(default)]
    pub description: String,
    /// The creator (OBS) / author login (Gitea).
    #[serde(default)]
    pub author: String,
    /// The unified state.
    pub status: PullRequestStatus,
    /// The Gitea draft flag; false on OBS.
    #[serde(default)]
    pub draft: bool,
    /// The Gitea merged flag; OBS accepted reads as merged.
    #[serde(default)]
    pub merged: bool,
    /// The Gitea mergeable flag; `None` on OBS.
    #[serde(default)]
    pub mergeable: Option<bool>,
    /// Where the change comes from.
    #[serde(default)]
    pub source: PullRequestEndpoint,
    /// Where the change goes to.
    #[serde(default)]
    pub target: PullRequestEndpoint,
    /// One entry per review asked or given, in order.
    #[serde(default)]
    pub reviews: Vec<ReviewOpinion>,
    /// The reviews-fetch error on the Gitea arm, when the PR read
    /// but its reviews did not; `None` otherwise, and always
    /// `None` on OBS, whose reviews arrive with the request.
    #[serde(default)]
    pub reviews_error: Option<String>,
    /// Gitea label names; empty on OBS.
    #[serde(default)]
    pub labels: Vec<String>,
}

impl PullRequest {
    /// Read an OBS submit request as the unified shape. The title
    /// is empty when the request carries none; the endpoints are
    /// the first action's — a request with several actions reads
    /// its first. A review reads its asked-of target as the
    /// reviewer.
    pub fn from_obs(request: &Request) -> Result<Self, GeectlError> {
        let (source, target) = request
            .actions
            .first()
            .map(endpoints_for_action)
            .unwrap_or_default();
        let mut reviews = Vec::new();
        for review in &request.reviews {
            let target = ReviewTarget::new(
                review.by_user.as_deref(),
                review.by_group.as_deref(),
                review.by_project.as_deref(),
                review.by_package.as_deref(),
            )?;
            reviews.push(ReviewOpinion {
                reviewer: target.describe(),
                state: OpinionState::from(review.state),
                comment: review.comment.clone(),
                when: review.when.clone(),
            });
        }
        Ok(PullRequest {
            reference: PullRequestRef::Obs(request.id),
            title: request.title.clone().unwrap_or_default(),
            description: request.description.clone(),
            author: request.creator.clone(),
            status: PullRequestStatus::from(request.state.name),
            draft: false,
            merged: request.state.name == RequestState::Accepted,
            mergeable: None,
            source,
            target,
            reviews,
            reviews_error: None,
            labels: Vec::new(),
        })
    }

    /// Read a Gitea pull request as the unified shape. The repo
    /// the PR was read from is the caller's `owner`/`repo`: the
    /// head repo is the fork, the base repo the target, and neither
    /// names the repo the PR belongs to.
    pub fn from_gitea(
        owner: &str,
        repo: &str,
        pr: &GiteaPullRequest,
        reviews: &[GiteaReview],
    ) -> Result<Self, GeectlError> {
        let mut opinions = Vec::new();
        for review in reviews {
            opinions.push(ReviewOpinion {
                reviewer: review.user.login.clone(),
                state: OpinionState::from_gitea(&review.state, review.dismissed)?,
                comment: review.body.clone().unwrap_or_default(),
                when: review.submitted_at.clone(),
            });
        }
        Ok(PullRequest {
            reference: PullRequestRef::Gitea {
                owner: owner.to_string(),
                repo: repo.to_string(),
                number: pr.number,
            },
            title: pr.title.clone(),
            description: pr.body.clone().unwrap_or_default(),
            author: pr.user.login.clone(),
            status: PullRequestStatus::from_gitea(&pr.state, pr.merged)?,
            draft: pr.draft,
            merged: pr.merged,
            mergeable: pr.mergeable,
            source: gitea_endpoint(&pr.head),
            target: gitea_endpoint(&pr.base),
            reviews: opinions,
            reviews_error: None,
            labels: pr.labels.iter().map(|label| label.name.clone()).collect(),
        })
    }
}

/// One endpoint of an OBS request, from a project and an optional
/// package: `project` alone when no package is named.
fn obs_endpoint(project: &str, package: Option<&str>) -> PullRequestEndpoint {
    PullRequestEndpoint {
        repository: match package {
            Some(package) => format!("{project}/{package}"),
            None => project.to_string(),
        },
        branch: None,
        commit: None,
    }
}

/// The source endpoint of an OBS submit-style action: the project
/// and package it comes from, with the source revision when the
/// action pins one.
fn obs_source_endpoint(project: &str, package: &str, rev: Option<&str>) -> PullRequestEndpoint {
    PullRequestEndpoint {
        repository: format!("{project}/{package}"),
        branch: None,
        commit: rev.map(str::to_string),
    }
}

/// The source and target endpoints of an OBS request action: the
/// submit-style actions read source and target, the target-only
/// actions (delete, role, bugowner) read an empty source and the
/// named target.
fn endpoints_for_action(action: &RequestAction) -> (PullRequestEndpoint, PullRequestEndpoint) {
    match action {
        RequestAction::Submit {
            source_project,
            source_package,
            source_rev,
            target_project,
            target_package,
            ..
        } => (
            obs_source_endpoint(source_project, source_package, source_rev.as_deref()),
            target_project
                .as_deref()
                .map_or_else(PullRequestEndpoint::default, |project| {
                    obs_endpoint(project, target_package.as_deref())
                }),
        ),
        RequestAction::Delete {
            target_project,
            target_package,
            ..
        }
        | RequestAction::AddRole {
            target_project,
            target_package,
            ..
        }
        | RequestAction::SetBugowner {
            target_project,
            target_package,
            ..
        } => (
            PullRequestEndpoint::default(),
            obs_endpoint(target_project, target_package.as_deref()),
        ),
        RequestAction::ChangeDevel {
            source_project,
            source_package,
            target_project,
            target_package,
        } => (
            obs_source_endpoint(source_project, source_package, None),
            obs_endpoint(target_project, Some(target_package)),
        ),
        RequestAction::MaintenanceIncident {
            source_project,
            source_package,
            target_project,
            target_package,
            ..
        }
        | RequestAction::MaintenanceRelease {
            source_project,
            source_package,
            target_project,
            target_package,
            ..
        }
        | RequestAction::Release {
            source_project,
            source_package,
            target_project,
            target_package,
            ..
        } => (
            obs_source_endpoint(source_project, source_package, None),
            obs_endpoint(target_project, target_package.as_deref()),
        ),
    }
}

/// One endpoint of a Gitea pull request: the repo the branch lives
/// in (empty when the fork is deleted), the branch, and the head
/// commit SHA.
fn gitea_endpoint(endpoint: &GiteaPullRequestRef) -> PullRequestEndpoint {
    PullRequestEndpoint {
        repository: endpoint
            .repo
            .as_ref()
            .map(|repo| repo.full_name.clone())
            .unwrap_or_default(),
        branch: Some(endpoint.branch.clone()),
        commit: Some(endpoint.sha.clone()),
    }
}

/// The list shape for pull-request triage: detail is a `show` call
/// away.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestSummary {
    /// The handle.
    pub reference: PullRequestRef,
    /// The title.
    #[serde(default)]
    pub title: String,
    /// The creator (OBS) / author login (Gitea).
    #[serde(default)]
    pub author: String,
    /// The unified state.
    pub status: PullRequestStatus,
    /// Where the change comes from.
    #[serde(default)]
    pub source: PullRequestEndpoint,
    /// Where the change goes to.
    #[serde(default)]
    pub target: PullRequestEndpoint,
}

impl PullRequestSummary {
    /// Read an OBS submit request as the list shape.
    pub fn from_obs(request: &Request) -> Result<Self, GeectlError> {
        let pull_request = PullRequest::from_obs(request)?;
        Ok(PullRequestSummary {
            reference: pull_request.reference,
            title: pull_request.title,
            author: pull_request.author,
            status: pull_request.status,
            source: pull_request.source,
            target: pull_request.target,
        })
    }

    /// Read a Gitea pull request as the list shape.
    pub fn from_gitea(owner: &str, repo: &str, pr: &GiteaPullRequest) -> Result<Self, GeectlError> {
        let pull_request = PullRequest::from_gitea(owner, repo, pr, &[])?;
        Ok(PullRequestSummary {
            reference: pull_request.reference,
            title: pull_request.title,
            author: pull_request.author,
            status: pull_request.status,
            source: pull_request.source,
            target: pull_request.target,
        })
    }

    /// Read a global-search item as the list shape. The search
    /// answer carries no head/base endpoints, so both read empty;
    /// the repo comes from the item's `repository` block.
    pub fn from_gitea_issue(issue: &GiteaIssue) -> Result<Self, GeectlError> {
        let (owner, repo) = issue.repository.full_name.split_once('/').ok_or_else(|| {
            GeectlError::new(
                ErrorKind::ServerError,
                format!(
                    "unexpected Gitea issue-search response: repository {:?} is not owner/repo",
                    issue.repository.full_name
                ),
            )
        })?;
        let merged = issue.pull_request.as_ref().is_some_and(|pr| pr.merged);
        Ok(PullRequestSummary {
            reference: PullRequestRef::Gitea {
                owner: owner.to_string(),
                repo: repo.to_string(),
                number: issue.number,
            },
            title: issue.title.clone(),
            author: issue.user.login.clone(),
            status: PullRequestStatus::from_gitea(&issue.state, merged)?,
            source: PullRequestEndpoint::default(),
            target: PullRequestEndpoint::default(),
        })
    }
}

/// A pull-request search: the filters `pull-request search`
/// carries. The backend scopes the search to one arm; with no
/// backend both arms run, OBS first then Gitea. The states are
/// the unified states; with none given each arm applies its own
/// default (new/review/declined on OBS, open on Gitea). The
/// creator filters by author. The repo scopes the Gitea arm to
/// one repo (`owner`/`repo`); a repo-scoped search needs the
/// Gitea backend — the OBS arm has no repo to scope to.
#[derive(Debug, Clone, Default)]
pub struct PullRequestSearch {
    /// Only this backend, or both when unset.
    pub backend: Option<PullRequestBackend>,
    /// Only these unified states, or each arm's default when empty.
    pub states: Vec<PullRequestStatus>,
    /// Only pull requests by this author, when set.
    pub creator: Option<String>,
    /// Only this Gitea repo, when set; needs the Gitea backend.
    pub repo: Option<(String, String)>,
}

impl PullRequestSearch {
    /// Check the filters before any arm runs: a repo-scoped search
    /// needs the Gitea backend — the OBS arm has no repo to scope
    /// to, so without it the repo filter would be silently ignored.
    pub fn validate(&self) -> Result<(), GeectlError> {
        if self.repo.is_some() && self.backend != Some(PullRequestBackend::Gitea) {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "a repo-scoped search needs backend gitea: the OBS arm has no repo to scope to",
            ));
        }
        Ok(())
    }
}

/// The uniform pull-request client: one interface over the OBS and
/// Gitea clients. An arm is `None` when its credentials are not
/// configured: a read on that arm fails closed naming the missing
/// credentials, and a read on the other arm never needs them.
pub struct PullRequestClient<'a> {
    obs: Option<&'a ObsClient<'a>>,
    gitea: Option<&'a GiteaClient<'a>>,
}

impl<'a> PullRequestClient<'a> {
    /// Build the uniform client from the two backend clients.
    /// Either arm may be `None` when its credentials are not
    /// configured; a read on the missing arm fails closed, a read
    /// on the other arm never needs it.
    pub fn new(obs: Option<&'a ObsClient<'a>>, gitea: Option<&'a GiteaClient<'a>>) -> Self {
        PullRequestClient { obs, gitea }
    }

    /// The OBS backend client, when configured.
    pub fn obs(&self) -> Option<&ObsClient<'a>> {
        self.obs
    }

    /// The Gitea backend client, when configured.
    pub fn gitea(&self) -> Option<&GiteaClient<'a>> {
        self.gitea
    }

    /// Read one pull request as the unified shape: the OBS arm
    /// reads the request with its reviews, the Gitea arm reads the
    /// PR and its reviews. A Gitea reviews fetch that fails leaves
    /// the PR with no reviews and the error recorded, instead of
    /// failing the read.
    pub fn show(&self, reference: &PullRequestRef) -> Result<PullRequest, GeectlError> {
        match reference {
            PullRequestRef::Obs(id) => {
                let obs = self.obs.ok_or_else(missing_obs_arm)?;
                let request = obs.show_request(&id.to_string())?;
                PullRequest::from_obs(&request)
            }
            PullRequestRef::Gitea {
                owner,
                repo,
                number,
            } => {
                let gitea = self.gitea.ok_or_else(missing_gitea_arm)?;
                let pr = gitea.pr_get(owner, repo, *number)?;
                let (reviews, reviews_error) = match gitea.pr_reviews(owner, repo, *number) {
                    Ok(reviews) => (reviews, None),
                    Err(err) => (Vec::new(), Some(err.to_string())),
                };
                let mut pull_request = PullRequest::from_gitea(owner, repo, &pr, &reviews)?;
                pull_request.reviews_error = reviews_error;
                Ok(pull_request)
            }
        }
    }

    /// Search pull requests as the list shape. With no backend
    /// both arms run, OBS first then Gitea; a backend scopes the
    /// search to that arm. The OBS arm filters server-side (the
    /// state mapping and the creator); the Gitea arm queries the
    /// wire state the requested states need and filters
    /// client-side to the exact states and creator, since the
    /// server has no unified-state or author filter for the list
    /// and search endpoints. A repo-scoped search needs the Gitea
    /// backend; without it the search fails as invalid input
    /// instead of returning unscoped OBS hits.
    pub fn search(
        &self,
        query: &PullRequestSearch,
    ) -> Result<Vec<PullRequestSummary>, GeectlError> {
        query.validate()?;
        let mut summaries = Vec::new();
        if query.backend.is_none() || query.backend == Some(PullRequestBackend::Obs) {
            let obs = self.obs.ok_or_else(missing_obs_arm)?;
            let mut search = RequestSearch::new();
            if let Some(states) = obs_states_for(&query.states) {
                search = search.states(states);
            }
            if let Some(creator) = &query.creator {
                search = search.creators(vec![creator.clone()]);
            }
            for request in &obs.search_requests(&search)? {
                summaries.push(PullRequestSummary::from_obs(request)?);
            }
        }
        if query.backend.is_none() || query.backend == Some(PullRequestBackend::Gitea) {
            let gitea = self.gitea.ok_or_else(missing_gitea_arm)?;
            let wire = gitea_state_for(&query.states);
            let mut items = match &query.repo {
                Some((owner, repo)) => gitea
                    .pr_list(owner, repo, wire)?
                    .iter()
                    .map(|pr| PullRequestSummary::from_gitea(owner, repo, pr))
                    .collect::<Result<Vec<_>, _>>()?,
                None => gitea
                    .pr_search(wire)?
                    .iter()
                    .map(PullRequestSummary::from_gitea_issue)
                    .collect::<Result<Vec<_>, _>>()?,
            };
            items.retain(|summary| matches_query(summary, query));
            summaries.extend(items);
        }
        Ok(summaries)
    }
}

/// The OBS states a unified-state filter reads as: `Open` reads
/// `new`/`review`, `Merged` reads `accepted`, `Closed` reads
/// `declined` (a closed PR reads as declined on OBS, which has no
/// `closed` state of its own), the rest read as themselves.
/// `None` is the search default — new/review/declined — so an
/// empty filter leaves the search unset.
fn obs_states_for(states: &[PullRequestStatus]) -> Option<Vec<RequestState>> {
    if states.is_empty() {
        return None;
    }
    let mut mapped: Vec<RequestState> = Vec::new();
    for state in states {
        let obs: &[RequestState] = match state {
            PullRequestStatus::Open => &[RequestState::New, RequestState::Review],
            PullRequestStatus::Merged => &[RequestState::Accepted],
            PullRequestStatus::Declined => &[RequestState::Declined],
            PullRequestStatus::Withdrawn => &[RequestState::Revoked],
            PullRequestStatus::Closed => &[RequestState::Declined],
            PullRequestStatus::Superseded => &[RequestState::Superseded],
            PullRequestStatus::Deleted => &[RequestState::Deleted],
        };
        for state in obs {
            if !mapped.contains(state) {
                mapped.push(*state);
            }
        }
    }
    Some(mapped)
}

/// The wire state a unified-state filter queries the Gitea arm
/// with: `Open` alone queries `open`, non-open states alone
/// query `closed`, a mix queries `all` and filters client-side.
/// An empty filter queries `open`, the Gitea arm's default.
fn gitea_state_for(states: &[PullRequestStatus]) -> GiteaPullRequestState {
    if states.is_empty() {
        return GiteaPullRequestState::Open;
    }
    let want_open = states.contains(&PullRequestStatus::Open);
    let want_closed = states.iter().any(|state| *state != PullRequestStatus::Open);
    match (want_open, want_closed) {
        (true, false) => GiteaPullRequestState::Open,
        (false, true) => GiteaPullRequestState::Closed,
        _ => GiteaPullRequestState::All,
    }
}

/// Whether a Gitea summary matches the search's exact states and
/// creator. The OBS arm needs no such filter: it filters
/// server-side.
fn matches_query(summary: &PullRequestSummary, query: &PullRequestSearch) -> bool {
    if !query.states.is_empty() && !query.states.contains(&summary.status) {
        return false;
    }
    if let Some(creator) = &query.creator {
        if summary.author != *creator {
            return false;
        }
    }
    true
}

/// The missing-OBS-arm error: the read needs OBS credentials.
fn missing_obs_arm() -> GeectlError {
    GeectlError::new(
        ErrorKind::ConfigError,
        "cannot read an OBS pull request without OBS credentials (GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD or GEECTL_OBS_PASSWORD_FILE)",
    )
}

/// The missing-Gitea-arm error: the read needs a Gitea token.
fn missing_gitea_arm() -> GeectlError {
    GeectlError::new(
        ErrorKind::ConfigError,
        "cannot read a Gitea pull request without a Gitea token (GEECTL_GITEA_TOKEN or GEECTL_GITEA_TOKEN_FILE)",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Credentials;
    use crate::http::{HttpClient, Timeout, UreqClient};
    use crate::obs::test_support::{basic_credentials, FakeHttp};

    fn endpoint(repository: &str) -> PullRequestEndpoint {
        PullRequestEndpoint {
            repository: repository.to_string(),
            branch: None,
            commit: None,
        }
    }

    #[test]
    fn handle_obs_round_trips() {
        let parsed: PullRequestRef = "obs:1398977".parse().unwrap();
        assert_eq!(parsed, PullRequestRef::Obs(1398977));
        assert_eq!(parsed.to_string(), "obs:1398977");
    }

    #[test]
    fn handle_gitea_round_trips() {
        let parsed: PullRequestRef = "gitea:openSUSE/geectl#122".parse().unwrap();
        assert_eq!(
            parsed,
            PullRequestRef::Gitea {
                owner: "openSUSE".to_string(),
                repo: "geectl".to_string(),
                number: 122,
            }
        );
        assert_eq!(parsed.to_string(), "gitea:openSUSE/geectl#122");
    }

    #[test]
    fn handle_obs_id_is_canonical() {
        let parsed: PullRequestRef = "obs:007".parse().unwrap();
        assert_eq!(parsed, PullRequestRef::Obs(7));
        assert_eq!(parsed.to_string(), "obs:7");
    }

    #[test]
    fn handle_rejects_malformed() {
        for input in [
            "",
            "123",
            "obs:",
            "obs:0",
            "obs:-1",
            "obs:12x",
            "obs: 1",
            "OBS:1",
            "gitea:owner/repo",
            "gitea:owner/repo#",
            "gitea:owner/repo#0",
            "gitea:owner/repo#x",
            "gitea:owner#1",
            "gitea:/repo#1",
            "gitea:owner/#1",
            "gitea:a/b/c#1",
            "gitea:owner/repo#1#2",
            "gitea:own er/repo#1",
            "gitea:owner/repo #1",
            "github:owner/repo#1",
        ] {
            assert!(
                input.parse::<PullRequestRef>().is_err(),
                "{input:?} must not parse"
            );
        }
    }

    #[test]
    fn status_maps_every_obs_state() {
        assert_eq!(
            PullRequestStatus::from(RequestState::New),
            PullRequestStatus::Open
        );
        assert_eq!(
            PullRequestStatus::from(RequestState::Review),
            PullRequestStatus::Open
        );
        assert_eq!(
            PullRequestStatus::from(RequestState::Accepted),
            PullRequestStatus::Merged
        );
        assert_eq!(
            PullRequestStatus::from(RequestState::Declined),
            PullRequestStatus::Declined
        );
        assert_eq!(
            PullRequestStatus::from(RequestState::Revoked),
            PullRequestStatus::Withdrawn
        );
        assert_eq!(
            PullRequestStatus::from(RequestState::Superseded),
            PullRequestStatus::Superseded
        );
        assert_eq!(
            PullRequestStatus::from(RequestState::Deleted),
            PullRequestStatus::Deleted
        );
    }

    #[test]
    fn status_maps_gitea_states() {
        assert_eq!(
            PullRequestStatus::from_gitea("open", false).unwrap(),
            PullRequestStatus::Open
        );
        assert_eq!(
            PullRequestStatus::from_gitea("open", true).unwrap(),
            PullRequestStatus::Open
        );
        assert_eq!(
            PullRequestStatus::from_gitea("closed", true).unwrap(),
            PullRequestStatus::Merged
        );
        assert_eq!(
            PullRequestStatus::from_gitea("closed", false).unwrap(),
            PullRequestStatus::Closed
        );
        let err =
            PullRequestStatus::from_gitea("draft", false).expect_err("an unknown wire state fails");
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn status_spellings_match_json() {
        for status in [
            PullRequestStatus::Open,
            PullRequestStatus::Merged,
            PullRequestStatus::Declined,
            PullRequestStatus::Withdrawn,
            PullRequestStatus::Closed,
            PullRequestStatus::Superseded,
            PullRequestStatus::Deleted,
        ] {
            assert_eq!(
                serde_json::to_string(&status).unwrap(),
                format!("\"{}\"", status.as_str()),
                "{status:?} serde spelling must match as_str",
            );
        }
        for state in [
            OpinionState::Asked,
            OpinionState::Approved,
            OpinionState::ChangesRequested,
            OpinionState::Commented,
            OpinionState::Dismissed,
            OpinionState::Superseded,
            OpinionState::Obsolete,
        ] {
            assert_eq!(
                serde_json::to_string(&state).unwrap(),
                format!("\"{}\"", state.as_str()),
                "{state:?} serde spelling must match as_str",
            );
        }
    }

    #[test]
    fn opinion_maps_every_obs_review_state() {
        assert_eq!(OpinionState::from(ReviewState::New), OpinionState::Asked);
        assert_eq!(
            OpinionState::from(ReviewState::Accepted),
            OpinionState::Approved
        );
        assert_eq!(
            OpinionState::from(ReviewState::Declined),
            OpinionState::ChangesRequested
        );
        assert_eq!(
            OpinionState::from(ReviewState::Superseded),
            OpinionState::Superseded
        );
        assert_eq!(
            OpinionState::from(ReviewState::Obsoleted),
            OpinionState::Obsolete
        );
    }

    #[test]
    fn opinion_maps_gitea_states() {
        assert_eq!(
            OpinionState::from_gitea("PENDING", false).unwrap(),
            OpinionState::Asked
        );
        assert_eq!(
            OpinionState::from_gitea("REQUEST_REVIEW", false).unwrap(),
            OpinionState::Asked
        );
        assert_eq!(
            OpinionState::from_gitea("APPROVED", false).unwrap(),
            OpinionState::Approved
        );
        assert_eq!(
            OpinionState::from_gitea("REQUEST_CHANGES", false).unwrap(),
            OpinionState::ChangesRequested
        );
        assert_eq!(
            OpinionState::from_gitea("COMMENT", false).unwrap(),
            OpinionState::Commented
        );
        assert_eq!(
            OpinionState::from_gitea("APPROVED", true).unwrap(),
            OpinionState::Dismissed
        );
        let err =
            OpinionState::from_gitea("UNKNOWN", false).expect_err("an unknown wire state fails");
        assert_eq!(err.kind(), ErrorKind::ServerError);
    }

    #[test]
    fn pull_request_json_carries_the_text_handle_and_all_keys() {
        let pull_request = PullRequest {
            reference: PullRequestRef::Obs(1398977),
            title: "Patterns".to_string(),
            description: String::new(),
            author: "scarabeusiv".to_string(),
            status: PullRequestStatus::Open,
            draft: false,
            merged: false,
            mergeable: None,
            source: endpoint("home:scarabeus_iv:patterns/patterns-base"),
            target: endpoint("openSUSE:Factory/patterns-base"),
            reviews: vec![ReviewOpinion {
                reviewer: "dimstar".to_string(),
                state: OpinionState::Approved,
                comment: String::new(),
                when: None,
            }],
            reviews_error: None,
            labels: vec![],
        };
        let json = serde_json::to_value(&pull_request).unwrap();
        assert_eq!(json["reference"], "obs:1398977");
        assert_eq!(json["status"], "open");
        assert_eq!(json["title"], "Patterns");
        assert_eq!(json["author"], "scarabeusiv");
        assert_eq!(json["draft"], false);
        assert_eq!(json["merged"], false);
        assert_eq!(json["mergeable"], serde_json::Value::Null);
        assert_eq!(json["reviews"][0]["state"], "approved");
        assert!(
            json.as_object().unwrap().contains_key("reviews_error"),
            "reviews_error must be emitted on every read"
        );
        assert_eq!(json["reviews_error"], serde_json::Value::Null);
        assert_eq!(json["labels"], serde_json::json!([]));
        let round_tripped: PullRequest = serde_json::from_value(json).unwrap();
        assert_eq!(round_tripped, pull_request);
    }

    #[test]
    fn pull_request_summary_json_round_trips() {
        let summary = PullRequestSummary {
            reference: PullRequestRef::Gitea {
                owner: "openSUSE".to_string(),
                repo: "geectl".to_string(),
                number: 122,
            },
            title: "change_review_state".to_string(),
            author: "scarabeusiv".to_string(),
            status: PullRequestStatus::Open,
            source: endpoint("scarabeusiv/geectl"),
            target: endpoint("openSUSE/geectl"),
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("\"gitea:openSUSE/geectl#122\""));
        let round_tripped: PullRequestSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, summary);
    }

    #[test]
    fn client_builds_from_both_backend_clients() {
        let http = UreqClient::new(Timeout::default()).unwrap();
        let obs = ObsClient::new(&http);
        let gitea = GiteaClient::new(
            &http,
            Credentials::Bearer {
                token: "test-token".to_string(),
            },
        )
        .unwrap();
        let client = PullRequestClient::new(Some(&obs), Some(&gitea));
        assert!(client.obs().is_some());
        assert!(client.gitea().is_some());
        let _neither = PullRequestClient::new(None, None);
    }
    const REQUEST_XML: &str = r#"<request creator="creator" id="123">
  <action type="submit">
    <source package="abc" project="xyz" />
    <options>
      <sourceupdate>cleanup</sourceupdate>
      <updatelink>1</updatelink>
    </options>
    <acceptinfo rev="5" srcmd5="deadbeef" />
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

    const DELETE_REQUEST_XML: &str = r#"<request creator="creator" id="124">
  <action type="delete">
    <target project="home:foo" package="bar" />
  </action>
  <state name="declined" when="2010-12-27T01:36:29" who="abc" />
</request>"#;

    /// A pull-request JSON body, shaped like Gitea 1.24.7 answers.
    const PR_JSON: &str = r#"{"number": 122, "user": {"login": "scarabeusiv"}, "title": "change_review_state", "body": "A test PR body.", "labels": [{"name": "bug"}], "state": "open", "draft": false, "merged": false, "mergeable": true, "head": {"label": "scarabeusiv:change-review-state", "ref": "change-review-state", "sha": "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa", "repo": {"full_name": "scarabeusiv/geectl"}}, "base": {"label": "main", "ref": "main", "sha": "e490820bfabf010f5f267335067a52757aca72af2f62b02e38dad0d43a50f7aa", "repo": {"full_name": "openSUSE/geectl"}}}"#;

    /// Two reviews, shaped like Gitea 1.24.7 answers.
    const REVIEWS_JSON: &str = r#"[{"user": {"login": "plusky"}, "state": "APPROVED", "body": "Looks good.", "dismissed": false, "submitted_at": "2026-11-22T19:52:58Z"}, {"user": {"login": "mimi1vx"}, "state": "COMMENT", "body": "One nit.", "dismissed": false, "submitted_at": "2026-11-22T19:53:10Z"}]"#;

    /// An issue-search item, shaped like Gitea 1.24.7 answers.
    fn issue_json(number: u64, state: &str, merged: bool, author: &str) -> String {
        format!(
            r#"{{"number": {number}, "user": {{"login": "{author}"}}, "title": "change_review_state", "body": "A test PR body.", "labels": [], "state": "{state}", "pull_request": {{"merged": {merged}, "draft": false}}, "repository": {{"full_name": "openSUSE/geectl"}}}}"#
        )
    }

    fn obs_client<'a>(http: &'a dyn HttpClient) -> ObsClient<'a> {
        ObsClient::with_credentials(http, basic_credentials()).unwrap()
    }

    fn gitea_client<'a>(http: &'a dyn HttpClient) -> GiteaClient<'a> {
        GiteaClient::new(
            http,
            Credentials::Bearer {
                token: "test-token".to_string(),
            },
        )
        .unwrap()
    }

    #[test]
    fn show_obs_reads_the_request() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/request/123?withhistory=1",
            REQUEST_XML,
        );
        let obs = obs_client(&http);
        let client = PullRequestClient::new(Some(&obs), None);
        let pr = client.show(&PullRequestRef::Obs(123)).unwrap();
        assert_eq!(pr.reference, PullRequestRef::Obs(123));
        assert_eq!(pr.title, "");
        assert_eq!(pr.author, "creator");
        assert_eq!(pr.status, PullRequestStatus::Open);
        assert!(!pr.draft);
        assert!(!pr.merged);
        assert_eq!(pr.mergeable, None);
        // The first action is the submit: source xyz/abc, no
        // revision pinned, no target named.
        assert_eq!(pr.source.repository, "xyz/abc");
        assert_eq!(pr.source.commit, None);
        assert_eq!(pr.target.repository, "");
        assert_eq!(pr.reviews.len(), 1);
        assert_eq!(pr.reviews[0].reviewer, "group group1");
        assert_eq!(pr.reviews[0].state, OpinionState::Asked);
        assert_eq!(pr.reviews[0].comment, "review start");
        assert_eq!(pr.reviews[0].when.as_deref(), Some("2010-12-28T00:11:22"));
        assert!(pr.labels.is_empty());
    }

    #[test]
    fn show_obs_delete_reads_the_target() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/request/124?withhistory=1",
            DELETE_REQUEST_XML,
        );
        let obs = obs_client(&http);
        let client = PullRequestClient::new(Some(&obs), None);
        let pr = client.show(&PullRequestRef::Obs(124)).unwrap();
        assert_eq!(pr.status, PullRequestStatus::Declined);
        assert_eq!(pr.source.repository, "");
        assert_eq!(pr.target.repository, "home:foo/bar");
    }

    #[test]
    fn show_gitea_reads_the_pr_and_its_reviews() {
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/repos/openSUSE/geectl/pulls/122",
                PR_JSON,
            )
            .with_get(
                "https://src.opensuse.org/api/v1/repos/openSUSE/geectl/pulls/122/reviews?page=1&limit=50",
                REVIEWS_JSON,
            );
        let gitea = gitea_client(&http);
        let client = PullRequestClient::new(None, Some(&gitea));
        let reference = PullRequestRef::Gitea {
            owner: "openSUSE".to_string(),
            repo: "geectl".to_string(),
            number: 122,
        };
        let pr = client.show(&reference).unwrap();
        assert_eq!(pr.reference, reference);
        assert_eq!(pr.title, "change_review_state");
        assert_eq!(pr.description, "A test PR body.");
        assert_eq!(pr.author, "scarabeusiv");
        assert_eq!(pr.status, PullRequestStatus::Open);
        assert_eq!(pr.mergeable, Some(true));
        assert_eq!(pr.source.repository, "scarabeusiv/geectl");
        assert_eq!(pr.source.branch.as_deref(), Some("change-review-state"));
        assert_eq!(pr.target.repository, "openSUSE/geectl");
        assert_eq!(pr.target.branch.as_deref(), Some("main"));
        assert_eq!(pr.labels, vec!["bug"]);
        assert_eq!(pr.reviews.len(), 2);
        assert_eq!(pr.reviews[0].reviewer, "plusky");
        assert_eq!(pr.reviews[0].state, OpinionState::Approved);
        assert_eq!(pr.reviews[0].comment, "Looks good.");
        assert_eq!(pr.reviews[1].state, OpinionState::Commented);
    }

    #[test]
    fn show_gitea_reads_the_pr_when_reviews_fail() {
        // The reviews are enrichment: when their fetch fails the
        // read still returns the PR, with no reviews and the error
        // recorded, instead of failing the whole read.
        let http = FakeHttp::new()
            .with_get(
                "https://src.opensuse.org/api/v1/repos/openSUSE/geectl/pulls/122",
                PR_JSON,
            )
            .with_get_error(
                "https://src.opensuse.org/api/v1/repos/openSUSE/geectl/pulls/122/reviews?page=1&limit=50",
                ErrorKind::NetworkError,
            );
        let gitea = gitea_client(&http);
        let client = PullRequestClient::new(None, Some(&gitea));
        let reference = PullRequestRef::Gitea {
            owner: "openSUSE".to_string(),
            repo: "geectl".to_string(),
            number: 122,
        };
        let pr = client.show(&reference).unwrap();
        assert_eq!(pr.reference, reference);
        assert_eq!(pr.status, PullRequestStatus::Open);
        assert!(pr.reviews.is_empty());
        assert_eq!(
            pr.reviews_error.as_deref(),
            Some("network_error: a network error occurred (canned GET error for https://src.opensuse.org/api/v1/repos/openSUSE/geectl/pulls/122/reviews?page=1&limit=50)")
        );
    }

    #[test]
    fn show_missing_arm_fails_closed() {
        let http = FakeHttp::new();
        let obs = obs_client(&http);
        let gitea = gitea_client(&http);
        let err = PullRequestClient::new(None, None)
            .show(&PullRequestRef::Obs(1))
            .expect_err("an OBS read without an OBS arm must fail");
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        let reference = PullRequestRef::Gitea {
            owner: "openSUSE".to_string(),
            repo: "geectl".to_string(),
            number: 122,
        };
        let err = PullRequestClient::new(Some(&obs), None)
            .show(&reference)
            .expect_err("a Gitea read without a Gitea arm must fail");
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        let err = PullRequestClient::new(None, Some(&gitea))
            .show(&PullRequestRef::Obs(1))
            .expect_err("an OBS read without an OBS arm must fail");
        assert_eq!(err.kind(), ErrorKind::ConfigError);
    }

    #[test]
    fn search_obs_defaults_to_the_open_states() {
        let collection = format!("<collection matches=\"1\">\n{REQUEST_XML}\n</collection>");
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/request?match=state%5B%40name%3D%27new%27%20or%20%40name%3D%27review%27%20or%20%40name%3D%27declined%27%5D",
            &collection,
        );
        let obs = obs_client(&http);
        let client = PullRequestClient::new(Some(&obs), None);
        let query = PullRequestSearch {
            backend: Some(PullRequestBackend::Obs),
            ..Default::default()
        };
        let summaries = client.search(&query).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].reference, PullRequestRef::Obs(123));
        assert_eq!(summaries[0].status, PullRequestStatus::Open);
        assert_eq!(summaries[0].source.repository, "xyz/abc");
    }

    #[test]
    fn search_obs_maps_open_to_new_and_review() {
        let collection = format!("<collection matches=\"1\">\n{REQUEST_XML}\n</collection>");
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/request?match=state%5B%40name%3D%27new%27%20or%20%40name%3D%27review%27%5D",
            &collection,
        );
        let obs = obs_client(&http);
        let client = PullRequestClient::new(Some(&obs), None);
        let query = PullRequestSearch {
            backend: Some(PullRequestBackend::Obs),
            states: vec![PullRequestStatus::Open],
            ..Default::default()
        };
        let summaries = client.search(&query).unwrap();
        assert_eq!(summaries.len(), 1);
    }

    #[test]
    fn search_obs_maps_merged_to_accepted_and_filters_creator() {
        let collection = "<collection matches=\"0\"></collection>".to_string();
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/request?match=%40creator%3D%27scarabeusiv%27%20and%20state%5B%40name%3D%27accepted%27%5D",
            &collection,
        );
        let obs = obs_client(&http);
        let client = PullRequestClient::new(Some(&obs), None);
        let query = PullRequestSearch {
            backend: Some(PullRequestBackend::Obs),
            states: vec![PullRequestStatus::Merged],
            creator: Some("scarabeusiv".to_string()),
            ..Default::default()
        };
        let summaries = client.search(&query).unwrap();
        assert!(summaries.is_empty());
    }

    #[test]
    fn search_closed_reads_obs_declined() {
        // OBS has no closed state: `--state closed` reads OBS
        // declined requests, which report as `declined` — an OBS
        // search never returns a summary with status `closed`.
        let collection = format!("<collection matches=\"1\">\n{DELETE_REQUEST_XML}\n</collection>");
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/request?match=state%5B%40name%3D%27declined%27%5D",
            &collection,
        );
        let obs = obs_client(&http);
        let client = PullRequestClient::new(Some(&obs), None);
        let query = PullRequestSearch {
            backend: Some(PullRequestBackend::Obs),
            states: vec![PullRequestStatus::Closed],
            ..Default::default()
        };
        let summaries = client.search(&query).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].status, PullRequestStatus::Declined);
    }

    #[test]
    fn search_gitea_lists_one_repo_when_scoped() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/openSUSE/geectl/pulls?state=open&page=1&limit=50",
            &format!("[{PR_JSON}]"),
        );
        let gitea = gitea_client(&http);
        let client = PullRequestClient::new(None, Some(&gitea));
        let query = PullRequestSearch {
            backend: Some(PullRequestBackend::Gitea),
            repo: Some(("openSUSE".to_string(), "geectl".to_string())),
            ..Default::default()
        };
        let summaries = client.search(&query).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(
            summaries[0].reference,
            PullRequestRef::Gitea {
                owner: "openSUSE".to_string(),
                repo: "geectl".to_string(),
                number: 122,
            }
        );
        assert_eq!(summaries[0].status, PullRequestStatus::Open);
        assert_eq!(summaries[0].source.repository, "scarabeusiv/geectl");
    }

    #[test]
    fn search_gitea_searches_globally_and_filters_states() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/issues/search?type=pulls&state=all&page=1&limit=50",
            &format!(
                "[{}, {}, {}]",
                issue_json(122, "open", false, "scarabeusiv"),
                issue_json(123, "closed", true, "scarabeusiv"),
                issue_json(124, "closed", false, "scarabeusiv"),
            ),
        );
        let gitea = gitea_client(&http);
        let client = PullRequestClient::new(None, Some(&gitea));
        let query = PullRequestSearch {
            backend: Some(PullRequestBackend::Gitea),
            states: vec![PullRequestStatus::Open, PullRequestStatus::Merged],
            ..Default::default()
        };
        let summaries = client.search(&query).unwrap();
        // Open and merged stay; closed-unmerged is filtered out.
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].status, PullRequestStatus::Open);
        assert_eq!(summaries[1].status, PullRequestStatus::Merged);
        assert_eq!(
            summaries[1].reference,
            PullRequestRef::Gitea {
                owner: "openSUSE".to_string(),
                repo: "geectl".to_string(),
                number: 123,
            }
        );
        // The search answer carries no endpoints.
        assert_eq!(summaries[0].source.repository, "");
    }

    #[test]
    fn search_gitea_closed_reads_closed() {
        // The Gitea side of the closed asymmetry: `--state closed`
        // queries the closed wire state and the summaries report
        // as `closed`, unlike the OBS arm, which reports declined.
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/issues/search?type=pulls&state=closed&page=1&limit=50",
            &format!("[{}]", issue_json(124, "closed", false, "scarabeusiv")),
        );
        let gitea = gitea_client(&http);
        let client = PullRequestClient::new(None, Some(&gitea));
        let query = PullRequestSearch {
            backend: Some(PullRequestBackend::Gitea),
            states: vec![PullRequestStatus::Closed],
            ..Default::default()
        };
        let summaries = client.search(&query).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].status, PullRequestStatus::Closed);
    }

    #[test]
    fn search_gitea_filters_creator_client_side() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/issues/search?type=pulls&state=open&page=1&limit=50",
            &format!(
                "[{}, {}]",
                issue_json(122, "open", false, "scarabeusiv"),
                issue_json(123, "open", false, "plusky"),
            ),
        );
        let gitea = gitea_client(&http);
        let client = PullRequestClient::new(None, Some(&gitea));
        let query = PullRequestSearch {
            backend: Some(PullRequestBackend::Gitea),
            creator: Some("plusky".to_string()),
            ..Default::default()
        };
        let summaries = client.search(&query).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].author, "plusky");
    }

    #[test]
    fn search_runs_obs_first_then_gitea() {
        let collection = format!("<collection matches=\"1\">\n{REQUEST_XML}\n</collection>");
        let http = FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/search/request?match=state%5B%40name%3D%27new%27%20or%20%40name%3D%27review%27%20or%20%40name%3D%27declined%27%5D",
                &collection,
            )
            .with_get(
                "https://src.opensuse.org/api/v1/repos/issues/search?type=pulls&state=open&page=1&limit=50",
                &format!("[{}]", issue_json(122, "open", false, "scarabeusiv")),
            );
        let obs = obs_client(&http);
        let gitea = gitea_client(&http);
        let client = PullRequestClient::new(Some(&obs), Some(&gitea));
        let summaries = client.search(&PullRequestSearch::default()).unwrap();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].reference, PullRequestRef::Obs(123));
        assert_eq!(
            summaries[1].reference,
            PullRequestRef::Gitea {
                owner: "openSUSE".to_string(),
                repo: "geectl".to_string(),
                number: 122,
            }
        );
    }

    #[test]
    fn search_backend_scopes_to_one_arm() {
        // OBS only, no Gitea arm at all: the search never touches
        // the missing arm.
        let collection = format!("<collection matches=\"1\">\n{REQUEST_XML}\n</collection>");
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/request?match=state%5B%40name%3D%27new%27%20or%20%40name%3D%27review%27%20or%20%40name%3D%27declined%27%5D",
            &collection,
        );
        let obs = obs_client(&http);
        let client = PullRequestClient::new(Some(&obs), None);
        let query = PullRequestSearch {
            backend: Some(PullRequestBackend::Obs),
            ..Default::default()
        };
        assert_eq!(client.search(&query).unwrap().len(), 1);

        // Gitea only, no OBS arm at all.
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/repos/issues/search?type=pulls&state=open&page=1&limit=50",
            &format!("[{}]", issue_json(122, "open", false, "scarabeusiv")),
        );
        let gitea = gitea_client(&http);
        let client = PullRequestClient::new(None, Some(&gitea));
        let query = PullRequestSearch {
            backend: Some(PullRequestBackend::Gitea),
            ..Default::default()
        };
        assert_eq!(client.search(&query).unwrap().len(), 1);
    }

    #[test]
    fn search_without_backend_fails_when_an_arm_is_missing() {
        // No backend means both arms run, so a missing arm fails
        // the search closed instead of silently returning one
        // backend's results: the caller scopes with --backend.
        let collection = format!("<collection matches=\"1\">\n{REQUEST_XML}\n</collection>");
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/request?match=state%5B%40name%3D%27new%27%20or%20%40name%3D%27review%27%20or%20%40name%3D%27declined%27%5D",
            &collection,
        );
        let obs = obs_client(&http);
        let err = PullRequestClient::new(Some(&obs), None)
            .search(&PullRequestSearch::default())
            .expect_err("a both-arms search without a Gitea arm must fail");
        assert_eq!(err.kind(), ErrorKind::ConfigError);
    }

    #[test]
    fn search_repo_scoped_search_needs_the_gitea_backend() {
        // A repo scopes the Gitea arm only: with no backend, or
        // with the OBS backend, the repo filter would be silently
        // ignored on the OBS arm, so the search fails as invalid
        // input instead. No HTTP call runs: the fake has no canned
        // responses, so any arm running would fail with a network
        // error, not the invalid-input error asserted here.
        let http = FakeHttp::new();
        let obs = obs_client(&http);
        let gitea = gitea_client(&http);
        let client = PullRequestClient::new(Some(&obs), Some(&gitea));
        for backend in [None, Some(PullRequestBackend::Obs)] {
            let query = PullRequestSearch {
                backend,
                repo: Some(("openSUSE".to_string(), "geectl".to_string())),
                ..Default::default()
            };
            let err = client
                .search(&query)
                .expect_err("a repo-scoped search without the Gitea backend must fail");
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn status_parses_its_spellings() {
        for (spelling, status) in [
            ("open", PullRequestStatus::Open),
            ("merged", PullRequestStatus::Merged),
            ("declined", PullRequestStatus::Declined),
            ("withdrawn", PullRequestStatus::Withdrawn),
            ("closed", PullRequestStatus::Closed),
            ("superseded", PullRequestStatus::Superseded),
            ("deleted", PullRequestStatus::Deleted),
        ] {
            assert_eq!(spelling.parse::<PullRequestStatus>().unwrap(), status);
            assert_eq!(status.as_str(), spelling);
        }
        for invalid in ["", "new", "review", "OPEN", "accepted", "revoked"] {
            assert!(
                invalid.parse::<PullRequestStatus>().is_err(),
                "{invalid:?} must not parse as a pull-request state"
            );
        }
    }

    #[test]
    fn backend_parses_its_spellings() {
        assert_eq!(
            "obs".parse::<PullRequestBackend>().unwrap(),
            PullRequestBackend::Obs
        );
        assert_eq!(
            "gitea".parse::<PullRequestBackend>().unwrap(),
            PullRequestBackend::Gitea
        );
        assert_eq!(PullRequestBackend::Obs.as_str(), "obs");
        assert_eq!(PullRequestBackend::Gitea.as_str(), "gitea");
        for invalid in ["", "github", "OBS", "gitea "] {
            assert!(
                invalid.parse::<PullRequestBackend>().is_err(),
                "{invalid:?} must not parse as a pull-request backend"
            );
        }
    }

    #[test]
    fn state_mapping_covers_every_unified_state() {
        // OBS arm: Open reads new/review, Closed reads declined
        // like Declined, the rest read as themselves; an empty
        // filter leaves the search at its default.
        assert_eq!(obs_states_for(&[]), None);
        assert_eq!(
            obs_states_for(&[PullRequestStatus::Open]),
            Some(vec![RequestState::New, RequestState::Review])
        );
        assert_eq!(
            obs_states_for(&[PullRequestStatus::Declined, PullRequestStatus::Closed]),
            Some(vec![RequestState::Declined])
        );
        assert_eq!(
            obs_states_for(&[PullRequestStatus::Withdrawn]),
            Some(vec![RequestState::Revoked])
        );
        assert_eq!(
            obs_states_for(&[PullRequestStatus::Superseded, PullRequestStatus::Deleted]),
            Some(vec![RequestState::Superseded, RequestState::Deleted])
        );
        // Gitea arm: open alone, closed alone, mixed, default.
        assert_eq!(gitea_state_for(&[]), GiteaPullRequestState::Open);
        assert_eq!(
            gitea_state_for(&[PullRequestStatus::Open]),
            GiteaPullRequestState::Open
        );
        assert_eq!(
            gitea_state_for(&[PullRequestStatus::Merged]),
            GiteaPullRequestState::Closed
        );
        assert_eq!(
            gitea_state_for(&[PullRequestStatus::Open, PullRequestStatus::Closed]),
            GiteaPullRequestState::All
        );
    }
}
