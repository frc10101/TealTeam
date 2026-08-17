//! Client for the FIRST Events API v3, a port of `internal/frc/first_api.go`.
//!
//! Basic auth from `FIRST_API_USERNAME` / `FIRST_API_KEY`, a connectivity
//! preflight before each call, and bounded retries with backoff on transient
//! failures. Every outcome is recorded in [`super::connectivity`] so the UI
//! can show why data is stale.
//!
//! The API is the source of events, team rosters and match schedules; it is
//! not consulted for results, which come from The Blue Alliance
//! ([`super::tba`]).

use std::collections::HashMap;
use std::time::Duration;

use base64::Engine;
use once_cell::sync::Lazy;
use serde::Deserialize;

use super::connectivity;

const DEFAULT_BASE_URL: &str = "https://frc-api.firstinspires.org/v3.0";

static HTTP: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("http client")
});

/// An event as returned by `/{season}/events`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FirstEvent {
    #[serde(rename = "eventCode")]
    pub event_code: String,
    pub code: String,
    pub name: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(rename = "districtCode")]
    pub district_code: String,
    pub venue: String,
    pub city: String,
    pub stateprov: String,
    pub country: String,
    #[serde(rename = "dateStart")]
    pub date_start: String,
    #[serde(rename = "dateEnd")]
    pub date_end: String,
    #[serde(rename = "weekNumber")]
    pub week_number: i32,
}

impl FirstEvent {
    /// The event code, from whichever field this endpoint populated —
    /// FIRST returns it as `eventCode` in some responses and `code` in others.
    pub fn effective_code(&self) -> String {
        let code = self.event_code.trim();
        if !code.is_empty() {
            code.to_string()
        } else {
            self.code.trim().to_string()
        }
    }
}

/// A team as returned by `/{season}/teams`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FirstTeam {
    #[serde(rename = "teamNumber")]
    pub team_number: i32,
    #[serde(rename = "nameFull")]
    pub name_full: String,
    #[serde(rename = "nameShort")]
    pub name_short: String,
    #[serde(rename = "schoolName")]
    pub school_name: String,
    pub city: String,
    #[serde(rename = "stateProv")]
    pub state_prov: String,
    pub country: String,
    #[serde(rename = "rookieYear")]
    pub rookie_year: i32,
    pub website: String,
}

impl FirstTeam {
    /// Short name if there is one, otherwise the full name — the short name
    /// is the nickname people actually use.
    pub fn display_name(&self) -> String {
        let name = self.name_short.trim();
        if !name.is_empty() {
            name.to_string()
        } else {
            self.name_full.trim().to_string()
        }
    }
}

/// One robot in a scheduled match. `station` is `Red1`..`Blue3`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ScheduleMatchTeam {
    #[serde(rename = "teamNumber")]
    pub team_number: i32,
    pub station: String,
    pub surrogate: bool,
    pub dq: bool,
}

/// One scheduled match. `start_time` is usually RFC3339 but occasionally
/// arrives without an offset — see
/// [`crate::views::coach`] for how that is interpreted.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ScheduleMatch {
    pub description: String,
    #[serde(rename = "tournamentLevel")]
    pub tournament_level: String,
    #[serde(rename = "matchNumber")]
    pub match_number: i32,
    #[serde(rename = "startTime")]
    pub start_time: String,
    pub field: String,
    pub teams: Vec<ScheduleMatchTeam>,
}

/// An authenticated FIRST Events API client.
pub struct FirstApiClient {
    base_url: String,
    auth_header: String,
}

impl FirstApiClient {
    /// Client for explicit credentials.
    pub fn new(username: &str, key: &str) -> Self {
        let credentials =
            base64::engine::general_purpose::STANDARD.encode(format!("{username}:{key}"));
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            auth_header: format!("Basic {credentials}"),
        }
    }

    /// Client from `FIRST_API_USERNAME` / `FIRST_API_KEY`.
    ///
    /// `None` when either is missing, which callers treat as "this deployment
    /// has no FIRST access" and degrade to local data rather than erroring.
    pub fn from_environment() -> Option<Self> {
        let username = std::env::var("FIRST_API_USERNAME").unwrap_or_default();
        let key = std::env::var("FIRST_API_KEY").unwrap_or_default();
        let (username, key) = (username.trim(), key.trim());
        if username.is_empty() || key.is_empty() {
            return None;
        }
        Some(Self::new(username, key))
    }

    /// Season year from `FIRST_SEASON`, defaulting to the current game year.
    pub fn season_from_environment() -> i32 {
        std::env::var("FIRST_SEASON")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(2026)
    }

    /// Issues a GET and decodes the response.
    ///
    /// Preflights connectivity (skipped for LAN/loopback base URLs), retries
    /// throttling and 5xx responses with backoff, and records success or
    /// failure in [`super::connectivity`] either way.
    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &HashMap<String, String>,
    ) -> anyhow::Result<T> {
        if !connectivity::should_skip_connectivity_check(&self.base_url) {
            if let Err(e) = connectivity::ensure_internet_for_base_url(&self.base_url).await {
                connectivity::record_api_error(&e.to_string());
                return Err(anyhow::Error::new(e));
            }
        }

        let endpoint = format!("{}{path}", self.base_url);
        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 0..connectivity::API_RETRY_MAX_ATTEMPTS {
            let request = HTTP
                .get(&endpoint)
                .header("Authorization", &self.auth_header)
                .header("Accept", "application/json")
                .query(&query);

            match request.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        let body = resp.text().await.unwrap_or_default();
                        let truncated = &body[..body.len().min(4096)];
                        let msg =
                            format!("first api {path} returned {}: {truncated}", status.as_u16());
                        connectivity::record_api_error(&msg);
                        if attempt < connectivity::API_RETRY_MAX_ATTEMPTS - 1
                            && connectivity::should_retry_status_code(status.as_u16())
                        {
                            last_error = Some(anyhow::anyhow!(msg));
                            tokio::time::sleep(connectivity::backoff_delay(attempt)).await;
                            continue;
                        }
                        return Err(anyhow::anyhow!(msg));
                    }

                    let result: T = resp.json().await.map_err(|e| {
                        let msg = format!("first api {path} returned invalid payload: {e}");
                        connectivity::record_api_error(&msg);
                        anyhow::anyhow!(msg)
                    })?;
                    connectivity::record_api_success();
                    return Ok(result);
                }
                Err(e) => {
                    let msg = format!("first api {path} request failed: {e}");
                    connectivity::record_api_error(&msg);
                    last_error = Some(anyhow::anyhow!(msg));
                    if attempt < connectivity::API_RETRY_MAX_ATTEMPTS - 1 {
                        tokio::time::sleep(connectivity::backoff_delay(attempt)).await;
                        continue;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("first api {path} retries exhausted")))
    }

    /// Events for a season, narrowed by the given filters (team number,
    /// event code, country).
    pub async fn get_season_events(
        &self,
        season: i32,
        filters: &HashMap<String, String>,
    ) -> anyhow::Result<Vec<FirstEvent>> {
        #[derive(Deserialize)]
        struct EventsResponse {
            #[serde(rename = "Events", default)]
            events: Vec<FirstEvent>,
        }
        let resp: EventsResponse = self.get_json(&format!("/{season}/events"), filters).await?;
        Ok(resp.events)
    }

    /// The team roster for one event.
    pub async fn get_event_teams(
        &self,
        season: i32,
        event_code: &str,
    ) -> anyhow::Result<Vec<FirstTeam>> {
        #[derive(Deserialize)]
        struct TeamsResponse {
            #[serde(default)]
            teams: Vec<FirstTeam>,
        }
        let mut query = HashMap::new();
        query.insert("eventCode".to_string(), event_code.to_string());
        let resp: TeamsResponse = self.get_json(&format!("/{season}/teams"), &query).await?;
        Ok(resp.teams)
    }

    /// The match schedule for one event. FIRST requires at least one filter,
    /// so callers pass a team number or a tournament level.
    pub async fn get_match_schedule(
        &self,
        season: i32,
        event_code: &str,
        filters: &HashMap<String, String>,
    ) -> anyhow::Result<Vec<ScheduleMatch>> {
        #[derive(Deserialize)]
        struct ScheduleResponse {
            #[serde(rename = "Schedule", default)]
            schedule: Vec<ScheduleMatch>,
        }
        let resp: ScheduleResponse = self
            .get_json(&format!("/{season}/schedule/{event_code}"), filters)
            .await?;
        Ok(resp.schedule)
    }
}
