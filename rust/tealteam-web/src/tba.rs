// Client + DTOs for The Blue Alliance API v3 (port of internal/frc/tba_client.go).

use std::collections::HashMap;
use std::time::Duration;

use once_cell::sync::Lazy;
use serde::Deserialize;
use serde_json::Value;

use crate::connectivity;

const BASE_URL: &str = "https://www.thebluealliance.com/api/v3";

static HTTP: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("http client")
});

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct OprData {
    pub oprs: HashMap<String, f64>,
    pub dprs: HashMap<String, f64>,
    pub ccwms: HashMap<String, f64>,
}

/// Dynamic component OPR breakdown: component-name -> teamKey -> value.
/// Includes fallback matching for season schema variance.
#[derive(Debug, Clone, Default)]
pub struct ComponentOprData {
    pub components: HashMap<String, HashMap<String, f64>>,
}

impl ComponentOprData {
    fn component_map(
        &self,
        preferred_names: &[&str],
        contains_all: &[&str],
    ) -> Option<&HashMap<String, f64>> {
        if self.components.is_empty() {
            return None;
        }

        let lookup: HashMap<String, &HashMap<String, f64>> = self
            .components
            .iter()
            .map(|(k, v)| (k.trim().to_lowercase(), v))
            .collect();

        for name in preferred_names {
            if let Some(values) = lookup.get(&name.trim().to_lowercase()) {
                return Some(values);
            }
        }

        // Fallback for new seasons: largest component map matching all tokens.
        let mut best: Option<&HashMap<String, f64>> = None;
        let mut best_size = 0usize;
        for (name, values) in &lookup {
            if contains_all.iter().all(|t| name.contains(t)) && values.len() > best_size {
                best = Some(values);
                best_size = values.len();
            }
        }
        best
    }

    pub fn team_phase_oprs(&self, team_key: &str) -> (Option<f64>, Option<f64>, Option<f64>) {
        let auto = self
            .component_map(
                &["totalAutoPoints", "autoPoints", "Hub Auto Points"],
                &["auto", "points"],
            )
            .and_then(|m| m.get(team_key).copied());
        let teleop = self
            .component_map(
                &["totalTeleopPoints", "teleopPoints", "Hub Teleop Points"],
                &["teleop", "points"],
            )
            .and_then(|m| m.get(team_key).copied());
        let endgame = self
            .component_map(
                &["endGameTowerPoints", "endgamePoints", "Hub Endgame Points"],
                &["endgame", "points"],
            )
            .and_then(|m| m.get(team_key).copied());
        (auto, teleop, endgame)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RankingRecord {
    pub wins: i32,
    pub losses: i32,
    pub ties: i32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RankingInfo {
    pub team_key: String,
    pub rank: i32,
    pub matches_played: i32,
    pub qual_average: Option<f64>,
    pub extra_stats: Vec<f64>,
    pub sort_orders: Vec<f64>,
    pub record: RankingRecord,
    pub dq: i32,
    pub qual_points: Option<i32>,
    pub elim_points: Option<i32>,
    pub award_points: Option<i32>,
    pub alliance_points: Option<i32>,
    pub tie_points: Option<i32>,
    pub total_points: Option<i32>,
}

impl RankingInfo {
    // Fallback extraction for year-specific ranking schema variance.
    pub fn effective_qual_average(&self) -> Option<f64> {
        self.qual_average.or_else(|| self.sort_orders.first().copied())
    }

    pub fn effective_avg_match_points(&self) -> Option<f64> {
        self.sort_orders.get(1).copied()
    }

    pub fn effective_total_points(&self) -> Option<i64> {
        if let Some(p) = self.total_points {
            return Some(p as i64);
        }
        self.extra_stats.first().map(|v| v.round() as i64)
    }

    pub fn effective_qual_points(&self) -> Option<i64> {
        match self.qual_points {
            Some(p) => Some(p as i64),
            None => self.effective_total_points(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TbaAlliance {
    #[serde(rename = "team_keys")]
    pub teams: Vec<String>,
    pub score: i32,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TbaAlliances {
    pub red: TbaAlliance,
    pub blue: TbaAlliance,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MatchInfo {
    pub key: String,
    pub event_key: String,
    pub comp_level: String,
    pub set_number: i32,
    pub match_number: i32,
    pub alliances: TbaAlliances,
    pub actual_time: i64,
    pub predicted_time: i64,
    // TBA v3 sends the scheduled time as "time" (the Go/.NET ports read
    // "scheduled_time", which TBA never sends — kept as an alias anyway).
    #[serde(rename = "time", alias = "scheduled_time")]
    pub scheduled_time: i64,
    pub score_breakdown: Option<Value>,
}

pub struct TbaClient {
    auth_key: String,
}

impl TbaClient {
    pub fn new(auth_key: &str) -> Self {
        Self {
            auth_key: auth_key.to_string(),
        }
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        if !connectivity::should_skip_connectivity_check(BASE_URL) {
            if let Err(e) = connectivity::ensure_internet_for_base_url(BASE_URL).await {
                connectivity::record_api_error(&e.to_string());
                return Err(anyhow::Error::new(e));
            }
        }

        let endpoint = format!("{BASE_URL}{path}");
        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 0..connectivity::API_RETRY_MAX_ATTEMPTS {
            let request = HTTP
                .get(&endpoint)
                .header("X-TBA-Auth-Key", &self.auth_key)
                .header("Accept", "application/json");

            match request.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        let body = resp.text().await.unwrap_or_default();
                        let truncated = &body[..body.len().min(4096)];
                        let msg =
                            format!("tba api {path} returned {}: {truncated}", status.as_u16());
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
                        let msg = format!("tba api {path} returned invalid payload: {e}");
                        connectivity::record_api_error(&msg);
                        anyhow::anyhow!(msg)
                    })?;
                    connectivity::record_api_success();
                    return Ok(result);
                }
                Err(e) => {
                    let msg = format!("tba api {path} request failed: {e}");
                    connectivity::record_api_error(&msg);
                    last_error = Some(anyhow::anyhow!(msg));
                    if attempt < connectivity::API_RETRY_MAX_ATTEMPTS - 1 {
                        tokio::time::sleep(connectivity::backoff_delay(attempt)).await;
                        continue;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("tba api {path} retries exhausted")))
    }

    pub async fn get_event_oprs(&self, event_key: &str) -> anyhow::Result<OprData> {
        self.get_json(&format!("/event/{event_key}/oprs")).await
    }

    pub async fn get_event_component_oprs(
        &self,
        event_key: &str,
    ) -> anyhow::Result<ComponentOprData> {
        let raw: HashMap<String, HashMap<String, f64>> =
            self.get_json(&format!("/event/{event_key}/coprs")).await?;
        Ok(ComponentOprData { components: raw })
    }

    pub async fn get_event_rankings(&self, event_key: &str) -> anyhow::Result<Vec<RankingInfo>> {
        #[derive(Deserialize)]
        struct RankingsResponse {
            #[serde(default)]
            rankings: Vec<RankingInfo>,
        }
        let resp: RankingsResponse = self.get_json(&format!("/event/{event_key}/rankings")).await?;
        Ok(resp.rankings)
    }

    pub async fn get_event_matches(&self, event_key: &str) -> anyhow::Result<Vec<MatchInfo>> {
        self.get_json(&format!("/event/{event_key}/matches")).await
    }
}
