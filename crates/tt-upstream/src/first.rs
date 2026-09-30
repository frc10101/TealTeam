//! FIRST Events API client (I1).
//!
//! The authoritative source for which events exist and who is attending them.
//! HTTP basic auth with a username and token from
//! <https://frc-events.firstinspires.org/services/API>.

use serde::de::DeserializeOwned;
use std::collections::HashMap;
use tracing::{debug, warn};

use crate::journal::{Fetched, Recorder};
use crate::{
    MAX_ATTEMPTS, REQUEST_TIMEOUT, Result, Uplink, UpstreamError, backoff, is_retryable, probe,
    truncate,
};
use chrono::Utc;
use tt_core::upstream::{FirstEvent, FirstTeam};

const API: &str = "first";
pub const DEFAULT_BASE_URL: &str = "https://frc-api.firstinspires.org/v3.0";

/// The most pages of one listing to follow. The largest roster, a
/// Championship division, is two.
const MAX_PAGES: u32 = 10;

/// Default country filter, applied only when no event or team filter is set.
pub const DEFAULT_COUNTRY: &str = "USA";

#[derive(Clone)]
pub struct FirstClient {
    http: reqwest::Client,
    base_url: String,
    username: String,
    token: String,
    season: i32,
    uplink: Uplink,
    /// Where new responses go for the upstream log (S1).
    recorder: Option<Recorder>,
}

impl FirstClient {
    pub fn new(
        username: impl Into<String>,
        token: impl Into<String>,
        season: i32,
        uplink: Uplink,
    ) -> Result<Self> {
        let (username, token) = (username.into(), token.into());
        if username.trim().is_empty() || token.trim().is_empty() {
            return Err(UpstreamError::NotConfigured("FIRST Events API"));
        }
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .map_err(|source| UpstreamError::Transport {
                    api: API,
                    path: "<client>".into(),
                    source,
                })?,
            base_url: DEFAULT_BASE_URL.to_string(),
            username,
            token,
            season,
            uplink,
            recorder: None,
        })
    }

    /// Send every response to `recorder` (S1). FIRST sends no ETags, so the
    /// log itself drops a body that has not changed.
    pub fn with_recorder(mut self, recorder: Recorder) -> Self {
        self.recorder = Some(recorder);
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn season(&self) -> i32 {
        self.season
    }

    /// Build from `FIRST_API_USERNAME`, `FIRST_API_KEY`, and `FIRST_SEASON`.
    pub fn from_env(uplink: Uplink) -> Option<Self> {
        let username = std::env::var("FIRST_API_USERNAME").ok()?;
        let token = std::env::var("FIRST_API_KEY").ok()?;
        let season = std::env::var("FIRST_SEASON")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(2026);
        Self::new(username.trim(), token.trim(), season, uplink).ok()
    }

    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &HashMap<String, String>,
    ) -> Result<T> {
        if !probe::is_local(&self.base_url) && !probe::probe(&self.uplink).await {
            return Err(UpstreamError::Offline);
        }

        let url = format!("{}{path}", self.base_url);
        let mut last: Option<UpstreamError> = None;

        for attempt in 0..MAX_ATTEMPTS {
            let response = self
                .http
                .get(&url)
                .basic_auth(&self.username, Some(&self.token))
                .header("Accept", "application/json")
                .query(query)
                .send()
                .await;

            match response {
                Ok(response) => {
                    let status = response.status().as_u16();
                    if !response.status().is_success() {
                        let body = truncate(&response.text().await.unwrap_or_default());
                        let error = UpstreamError::Status {
                            api: API,
                            path: path.to_string(),
                            status,
                            body,
                        };
                        self.uplink.record_error(&error.to_string());

                        if attempt + 1 < MAX_ATTEMPTS && is_retryable(status) {
                            warn!("{error}; retrying");
                            tokio::time::sleep(backoff(attempt)).await;
                            last = Some(error);
                            continue;
                        }
                        return Err(error);
                    }

                    let body =
                        response
                            .text()
                            .await
                            .map_err(|source| UpstreamError::Transport {
                                api: API,
                                path: path.to_string(),
                                source,
                            })?;

                    return match serde_json::from_str(&body) {
                        Ok(value) => {
                            self.uplink.record_success();
                            if let Some(recorder) = &self.recorder {
                                recorder.record(Fetched {
                                    api: API,
                                    path: logged_path(path, query),
                                    etag: None,
                                    body: body.into(),
                                    fetched_at: Utc::now(),
                                });
                            }
                            Ok(value)
                        }
                        Err(source) => {
                            let error = UpstreamError::Payload {
                                api: API,
                                path: path.to_string(),
                                source,
                            };
                            self.uplink.record_error(&error.to_string());
                            Err(error)
                        }
                    };
                }
                Err(source) => {
                    let error = UpstreamError::Transport {
                        api: API,
                        path: path.to_string(),
                        source,
                    };
                    self.uplink.record_error(&error.to_string());
                    if attempt + 1 < MAX_ATTEMPTS {
                        debug!("{error}; retrying");
                        tokio::time::sleep(backoff(attempt)).await;
                        last = Some(error);
                        continue;
                    }
                    return Err(error);
                }
            }
        }

        Err(last.unwrap_or(UpstreamError::Offline))
    }

    /// Season events, optionally filtered.
    ///
    /// Note the response envelope is PascalCase `Events` here but lowercase
    /// `teams` on the teams endpoint -- FIRST is not consistent about it.
    pub async fn events(&self, filters: &EventFilters) -> Result<Vec<FirstEvent>> {
        #[derive(serde::Deserialize)]
        struct Envelope {
            #[serde(rename = "Events", default)]
            events: Vec<FirstEvent>,
        }
        let envelope: Envelope = self
            .get(&format!("/{}/events", self.season), &filters.to_query())
            .await?;

        let mut events = envelope.events;
        // The country filter is applied here rather than upstream because the
        // API has no parameter for it, and pulling every event on earth to throw
        // most away is exactly what a tethered phone cannot afford.
        if filters.is_unfiltered()
            && let Some(country) = filters.country.as_deref().filter(|c| !c.is_empty())
        {
            events.retain(|e| e.country.eq_ignore_ascii_case(country));
        }
        Ok(events)
    }

    /// An event's roster, every page of it. FIRST sends 65 teams a page, and
    /// a Championship division has more: reading only the first left nine of
    /// Milstein's 74 off in 2026.
    pub async fn event_teams(&self, event_code: &str) -> Result<Vec<FirstTeam>> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Envelope {
            #[serde(default)]
            teams: Vec<FirstTeam>,
            #[serde(default)]
            page_total: Option<u32>,
        }
        let mut teams = Vec::new();
        let mut page = 1;
        loop {
            let mut query = HashMap::new();
            query.insert("eventCode".to_string(), event_code.to_string());
            if page > 1 {
                query.insert("page".to_string(), page.to_string());
            }
            let envelope: Envelope = self.get(&format!("/{}/teams", self.season), &query).await?;
            teams.extend(envelope.teams);
            // A bound, in case a page count ever came back nonsense.
            if page >= envelope.page_total.unwrap_or(1).min(MAX_PAGES) {
                return Ok(teams);
            }
            page += 1;
        }
    }
}

/// `path?a=1&b=2`, parameters sorted, so one request always logs as one path.
fn logged_path(path: &str, query: &HashMap<String, String>) -> String {
    let mut pairs: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
    if pairs.is_empty() {
        return path.to_string();
    }
    pairs.sort();
    format!("{path}?{}", pairs.join("&"))
}

/// Which slice of the season to pull.
#[derive(Debug, Clone, Default)]
pub struct EventFilters {
    pub event_code: Option<String>,
    pub team_number: Option<i32>,
    /// Applied client-side, and only when neither other filter is set.
    pub country: Option<String>,
}

impl EventFilters {
    /// Every event in the default country.
    pub fn all() -> Self {
        Self {
            country: Some(DEFAULT_COUNTRY.to_string()),
            ..Self::default()
        }
    }

    pub fn for_team(team_number: i32) -> Self {
        Self {
            team_number: Some(team_number),
            ..Self::default()
        }
    }

    pub fn for_event(event_code: impl Into<String>) -> Self {
        Self {
            event_code: Some(event_code.into()),
            ..Self::default()
        }
    }

    /// True when neither a specific event nor a specific team was requested, so
    /// the country filter is worth applying.
    pub fn is_unfiltered(&self) -> bool {
        self.event_code.as_deref().unwrap_or("").trim().is_empty() && self.team_number.is_none()
    }

    pub fn to_query(&self) -> HashMap<String, String> {
        let mut query = HashMap::new();
        if let Some(code) = self.event_code.as_deref().filter(|c| !c.trim().is_empty()) {
            query.insert("eventCode".into(), code.trim().to_string());
        }
        if let Some(team) = self.team_number {
            query.insert("teamNumber".into(), team.to_string());
        }
        query
    }

    /// Build from `FIRST_EVENT_CODE`, `FIRST_TEAM_NUMBER`, `FIRST_COUNTRY`.
    pub fn from_env() -> Self {
        let env = |key: &str| {
            std::env::var(key)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        Self {
            event_code: env("FIRST_EVENT_CODE"),
            team_number: env("FIRST_TEAM_NUMBER").and_then(|v| v.parse().ok()),
            country: env("FIRST_COUNTRY").or_else(|| Some(DEFAULT_COUNTRY.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_required() {
        let uplink = Uplink::new();
        assert!(FirstClient::new("", "token", 2026, uplink.clone()).is_err());
        assert!(FirstClient::new("user", "  ", 2026, uplink).is_err());
    }

    #[test]
    fn filters_map_onto_query_parameters() {
        assert_eq!(
            EventFilters::for_event("MABIL").to_query()["eventCode"],
            "MABIL"
        );
        assert_eq!(
            EventFilters::for_team(10101).to_query()["teamNumber"],
            "10101"
        );
        assert!(EventFilters::all().to_query().is_empty());
    }

    #[test]
    fn the_country_filter_only_applies_when_nothing_else_is_set() {
        assert!(EventFilters::all().is_unfiltered());
        assert!(!EventFilters::for_team(10101).is_unfiltered());
        assert!(!EventFilters::for_event("MABIL").is_unfiltered());
        // A blank event code is not a filter.
        assert!(
            EventFilters {
                event_code: Some("  ".into()),
                ..EventFilters::default()
            }
            .is_unfiltered()
        );
    }

    #[test]
    fn blank_filters_are_omitted_rather_than_sent_empty() {
        let filters = EventFilters {
            event_code: Some("   ".into()),
            ..EventFilters::default()
        };
        assert!(filters.to_query().is_empty());
    }
}
