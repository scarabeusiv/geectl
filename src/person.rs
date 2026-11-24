//! A person search across both backends: the OBS person-match
//! (`search_projects_by_person`) and the Gitea user search
//! (`GiteaClient::user_search`) under one operation. Unified for
//! the end user, separate backend logic underneath: the query is
//! one string, and each arm answers it its own way — the OBS arm
//! matches the login exactly (every project whose meta names it
//! in a role), the Gitea arm matches the keyword against login,
//! full name, and email. With no `--backend` both arms run, OBS
//! first then Gitea; a backend scopes the search to that arm.
//!
//! An arm is `None` when its credentials are not configured: a
//! search on that arm fails closed naming the missing
//! credentials, and a search on the other arm never needs them —
//! the same arm rule as the unified pull-request client.

use crate::error::{ErrorKind, GeectlError};
use crate::gitea::{GiteaClient, GiteaUser};
use crate::obs::{ObsClient, ProjectMatch};
use std::str::FromStr;

/// Which backend a person search runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonBackend {
    /// The OBS person-match: projects naming the login.
    Obs,
    /// The Gitea user search: users matching the keyword.
    Gitea,
}

impl PersonBackend {
    /// The lowercase spelling of this backend, as it appears in
    /// `--backend` filters.
    pub fn as_str(&self) -> &'static str {
        match self {
            PersonBackend::Obs => "obs",
            PersonBackend::Gitea => "gitea",
        }
    }
}

impl FromStr for PersonBackend {
    type Err = GeectlError;

    /// Parse a `--backend` value; anything else is an invalid
    /// input naming the backend.
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input {
            "obs" => Ok(PersonBackend::Obs),
            "gitea" => Ok(PersonBackend::Gitea),
            _ => Err(GeectlError::new(
                ErrorKind::InvalidInput,
                format!("unknown person-search backend: {input:?}"),
            )),
        }
    }
}

/// A person search: the query both arms answer, and the backend
/// scoping it to one arm. With no backend both arms run.
#[derive(Debug, Clone, Default)]
pub struct PersonSearch {
    /// The query: an OBS login, a Gitea keyword.
    pub query: String,
    /// Only this backend, or both when unset.
    pub backend: Option<PersonBackend>,
}

impl PersonSearch {
    /// Check the query before any arm runs: an empty query fails
    /// as invalid input whether or not credentials are
    /// configured. A quote in the query is not refused here — the
    /// OBS arm refuses it (its XPath literal cannot hold one)
    /// while the Gitea arm answers it — so a Gitea-scoped search
    /// for a quoted keyword still runs.
    pub fn validate(&self) -> Result<(), GeectlError> {
        if self.query.is_empty() {
            return Err(GeectlError::new(
                ErrorKind::InvalidInput,
                "person search query must not be empty",
            ));
        }
        Ok(())
    }
}

/// The result of a person search: the OBS arm's projects and the
/// Gitea arm's users. An arm that did not run leaves its list
/// empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersonSearchResult {
    /// The projects naming the login, from the OBS arm.
    pub projects: Vec<ProjectMatch>,
    /// The users matching the keyword, from the Gitea arm.
    pub users: Vec<GiteaUser>,
}

/// The uniform person-search client: one interface over the OBS
/// and Gitea clients. An arm is `None` when its credentials are
/// not configured: a search on that arm fails closed naming the
/// missing credentials, and a search on the other arm never needs
/// them.
pub struct PersonClient<'a> {
    obs: Option<&'a ObsClient<'a>>,
    gitea: Option<&'a GiteaClient<'a>>,
}

impl<'a> PersonClient<'a> {
    /// Build the uniform client from the two backend clients.
    /// Either arm may be `None` when its credentials are not
    /// configured; a search on the missing arm fails closed, a
    /// search on the other arm never needs it.
    pub fn new(obs: Option<&'a ObsClient<'a>>, gitea: Option<&'a GiteaClient<'a>>) -> Self {
        PersonClient { obs, gitea }
    }

    /// Search for a person as the two lists. With no backend
    /// both arms run, OBS first then Gitea; a backend scopes the
    /// search to that arm, and the other arm's list stays empty.
    /// The OBS arm matches the login exactly; the Gitea arm
    /// matches the keyword.
    pub fn search(&self, query: &PersonSearch) -> Result<PersonSearchResult, GeectlError> {
        query.validate()?;
        let mut result = PersonSearchResult::default();
        if query.backend.is_none() || query.backend == Some(PersonBackend::Obs) {
            let obs = self.obs.ok_or_else(missing_obs_arm)?;
            result.projects = obs.search_projects_by_person(&query.query)?;
        }
        if query.backend.is_none() || query.backend == Some(PersonBackend::Gitea) {
            let gitea = self.gitea.ok_or_else(missing_gitea_arm)?;
            result.users = gitea.user_search(&query.query)?;
        }
        Ok(result)
    }
}

/// The missing-OBS-arm error: the search needs OBS credentials.
fn missing_obs_arm() -> GeectlError {
    GeectlError::new(
        ErrorKind::ConfigError,
        "cannot search for a person on OBS without OBS credentials (GEECTL_OBS_APIKEY[_FILE] or GEECTL_OBS_USERNAME + GEECTL_OBS_PASSWORD[_FILE])",
    )
}

/// The missing-Gitea-arm error: the search needs a Gitea token.
fn missing_gitea_arm() -> GeectlError {
    GeectlError::new(
        ErrorKind::ConfigError,
        "cannot search for a person on Gitea without a Gitea token (GEECTL_GITEA_TOKEN or GEECTL_GITEA_TOKEN_FILE)",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Credentials;
    use crate::http::HttpClient;
    use crate::obs::test_support::{basic_credentials, FakeHttp};

    const PROJECT_SEARCH_FIXTURE: &str = "<collection matches=\"1\">\
        <project name=\"devel:languages:python\">\
        <title>Python</title><description>Python packages</description>\
        </project>\
        </collection>";

    const USER_SEARCH_FIXTURE: &str = r#"{"ok": true, "data": [{"login": "alice", "full_name": "Alice", "email": "alice@example.com"}]}"#;

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

    fn both_arms_http() -> FakeHttp {
        FakeHttp::new()
            .with_get(
                "https://api.opensuse.org/search/project?match=person%2F%40userid%3D%27alice%27",
                PROJECT_SEARCH_FIXTURE,
            )
            .with_get(
                "https://src.opensuse.org/api/v1/users/search?q=alice&page=1&limit=50",
                USER_SEARCH_FIXTURE,
            )
    }

    fn search(query: &str, backend: Option<PersonBackend>) -> PersonSearch {
        PersonSearch {
            query: query.to_string(),
            backend,
        }
    }

    #[test]
    fn backend_spellings_round_trip() {
        assert_eq!("obs".parse::<PersonBackend>().unwrap(), PersonBackend::Obs);
        assert_eq!(
            "gitea".parse::<PersonBackend>().unwrap(),
            PersonBackend::Gitea
        );
        assert_eq!(PersonBackend::Obs.as_str(), "obs");
        assert_eq!(PersonBackend::Gitea.as_str(), "gitea");
    }

    #[test]
    fn backend_rejects_unknown() {
        let err = "github"
            .parse::<PersonBackend>()
            .expect_err("an unknown backend must not parse");
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn validate_rejects_an_empty_query() {
        let err = search("", None)
            .validate()
            .expect_err("an empty query must not validate");
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn search_runs_both_arms_obs_first() {
        let http = both_arms_http();
        let obs = obs_client(&http);
        let gitea = gitea_client(&http);
        let client = PersonClient::new(Some(&obs), Some(&gitea));
        let result = client.search(&search("alice", None)).unwrap();
        assert_eq!(result.projects.len(), 1);
        assert_eq!(result.projects[0].name, "devel:languages:python");
        assert_eq!(result.users.len(), 1);
        assert_eq!(result.users[0].login, "alice");
        assert_eq!(result.users[0].full_name, "Alice");
        assert_eq!(result.users[0].email, "alice@example.com");
    }

    #[test]
    fn search_scoped_to_obs_leaves_users_empty() {
        let http = FakeHttp::new().with_get(
            "https://api.opensuse.org/search/project?match=person%2F%40userid%3D%27alice%27",
            PROJECT_SEARCH_FIXTURE,
        );
        let obs = obs_client(&http);
        let gitea = gitea_client(&http);
        let client = PersonClient::new(Some(&obs), Some(&gitea));
        let result = client
            .search(&search("alice", Some(PersonBackend::Obs)))
            .unwrap();
        assert_eq!(result.projects.len(), 1);
        assert!(result.users.is_empty());
        assert!(http
            .get_calls()
            .iter()
            .all(|call| call.0.contains("api.opensuse.org")));
    }

    #[test]
    fn search_scoped_to_gitea_leaves_projects_empty() {
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/users/search?q=alice&page=1&limit=50",
            USER_SEARCH_FIXTURE,
        );
        let obs = obs_client(&http);
        let gitea = gitea_client(&http);
        let client = PersonClient::new(Some(&obs), Some(&gitea));
        let result = client
            .search(&search("alice", Some(PersonBackend::Gitea)))
            .unwrap();
        assert!(result.projects.is_empty());
        assert_eq!(result.users.len(), 1);
        assert!(http
            .get_calls()
            .iter()
            .all(|call| call.0.contains("src.opensuse.org")));
    }

    #[test]
    fn search_missing_arm_fails_closed() {
        let http = both_arms_http();
        let obs = obs_client(&http);
        let gitea = gitea_client(&http);
        // No arms at all: the OBS arm runs first and fails.
        let err = PersonClient::new(None, None)
            .search(&search("alice", None))
            .expect_err("a search without arms must fail");
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.to_string().contains("OBS credentials"),
            "the missing-arm error must name the credentials; got {err}"
        );
        // OBS arm present, Gitea missing: the both-arms search
        // fails on the Gitea arm, the OBS-scoped one runs.
        let err = PersonClient::new(Some(&obs), None)
            .search(&search("alice", None))
            .expect_err("a both-arms search without a Gitea arm must fail");
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        assert!(
            err.to_string().contains("Gitea token"),
            "the missing-arm error must name the token; got {err}"
        );
        let result = PersonClient::new(Some(&obs), None)
            .search(&search("alice", Some(PersonBackend::Obs)))
            .unwrap();
        assert_eq!(result.projects.len(), 1);
        // Gitea arm present, OBS missing: the mirror case.
        let err = PersonClient::new(None, Some(&gitea))
            .search(&search("alice", None))
            .expect_err("a both-arms search without an OBS arm must fail");
        assert_eq!(err.kind(), ErrorKind::ConfigError);
        let result = PersonClient::new(None, Some(&gitea))
            .search(&search("alice", Some(PersonBackend::Gitea)))
            .unwrap();
        assert_eq!(result.users.len(), 1);
    }

    #[test]
    fn search_gitea_scoped_answers_a_quoted_query() {
        // The quote is refused only by the OBS arm (its XPath
        // literal cannot hold one); the Gitea arm answers it —
        // the query is URL-encoded, not refused.
        let http = FakeHttp::new().with_get(
            "https://src.opensuse.org/api/v1/users/search?q=o%27brien&page=1&limit=50",
            USER_SEARCH_FIXTURE,
        );
        let obs = obs_client(&http);
        let gitea = gitea_client(&http);
        let client = PersonClient::new(Some(&obs), Some(&gitea));
        let result = client
            .search(&search("o'brien", Some(PersonBackend::Gitea)))
            .unwrap();
        assert_eq!(result.users.len(), 1);
    }

    #[test]
    fn search_obs_arm_refuses_a_quoted_query_without_http() {
        let http = both_arms_http();
        let obs = obs_client(&http);
        let gitea = gitea_client(&http);
        let client = PersonClient::new(Some(&obs), Some(&gitea));
        let err = client
            .search(&search("o'brien", None))
            .expect_err("the OBS arm must refuse a quoted query");
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(http.get_calls().is_empty());
    }

    #[test]
    fn search_validates_before_any_arm_runs() {
        let http = FakeHttp::new();
        let obs = obs_client(&http);
        let gitea = gitea_client(&http);
        let client = PersonClient::new(Some(&obs), Some(&gitea));
        assert!(client.search(&search("", None)).is_err());
        assert!(http.get_calls().is_empty());
    }
}
