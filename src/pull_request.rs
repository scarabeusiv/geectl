//! Unified pull requests: one interface for an OBS submit request and a
//! Gitea pull request.
//!
//! A pull request is named by a backend-neutral handle
//! ([`PullRequestRef`]) and read as one shape ([`PullRequest`]) on both
//! backends, with a unified status ([`PullRequestStatus`]). This module
//! carries the handle, the shapes, and the status mappings.

use crate::error::{ErrorKind, GeectlError};
use crate::gitea::{validate_gitea_name, GiteaClient};
use crate::obs::request::{validate_request_id, RequestState, ReviewState};
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
    /// Closed without merging: Gitea closed, not merged.
    Closed,
    /// Replaced by a newer request: OBS `superseded`.
    Superseded,
    /// Deleted: OBS `deleted`.
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
    /// unknown wire state is an invalid input naming the state.
    pub fn from_gitea(state: &str, merged: bool) -> Result<Self, GeectlError> {
        match (state, merged) {
            ("open", _) => Ok(PullRequestStatus::Open),
            ("closed", true) => Ok(PullRequestStatus::Merged),
            ("closed", false) => Ok(PullRequestStatus::Closed),
            _ => Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("unknown Gitea pull-request state: {state:?}"),
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
    /// An unknown wire state is an invalid input naming the state.
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
                ErrorKind::InvalidInput,
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
    /// Gitea label names; empty on OBS.
    #[serde(default)]
    pub labels: Vec<String>,
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

/// The uniform pull-request client: one interface over the OBS and
/// Gitea clients.
pub struct PullRequestClient<'a> {
    obs: &'a ObsClient<'a>,
    gitea: &'a GiteaClient<'a>,
}

impl<'a> PullRequestClient<'a> {
    /// Build the uniform client from the two backend clients.
    pub fn new(obs: &'a ObsClient<'a>, gitea: &'a GiteaClient<'a>) -> Self {
        PullRequestClient { obs, gitea }
    }

    /// The OBS backend client.
    pub fn obs(&self) -> &ObsClient<'a> {
        self.obs
    }

    /// The Gitea backend client.
    pub fn gitea(&self) -> &GiteaClient<'a> {
        self.gitea
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Credentials;
    use crate::http::{Timeout, UreqClient};

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
        assert!(PullRequestStatus::from_gitea("draft", false).is_err());
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
        assert!(OpinionState::from_gitea("UNKNOWN", false).is_err());
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
        let _client = PullRequestClient::new(&obs, &gitea);
    }
}
