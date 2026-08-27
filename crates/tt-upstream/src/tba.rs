//! The Blue Alliance API client (I2).
//!
//! Rankings, OPRs, component OPRs, and match results. TBA allows direct browser
//! requests, which is why the retired plan's "relay server" was unnecessary:
//! any client with signal can fetch this itself and hand the Pi a bundle (S4).

use serde::de::DeserializeOwned;
use tracing::{debug, warn};

use crate::{
    MAX_ATTEMPTS, REQUEST_TIMEOUT, Result, Uplink, UpstreamError, backoff, is_retryable, probe,
    truncate,
};
use tt_core::upstream::{ComponentOprs, Match, Oprs, Ranking};

const API: &str = "tba";
pub const DEFAULT_BASE_URL: &str = "https://www.thebluealliance.com/api/v3";

#[derive(Clone)]
pub struct TbaClient {
    http: reqwest::Client,
    base_url: String,
    auth_key: String,
    uplink: Uplink,
}

impl TbaClient {
    pub fn new(auth_key: impl Into<String>, uplink: Uplink) -> Result<Self> {
        let auth_key = auth_key.into();
        if auth_key.trim().is_empty() {
            return Err(UpstreamError::NotConfigured("The Blue Alliance"));
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
            auth_key,
            uplink,
        })
    }

    /// Point at a different host. For tests against a local stub.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Read `TBA_AUTH_KEY`. Absent means the whole TBA sync is disabled, which
    /// is a supported configuration rather than an error.
    pub fn from_env(uplink: Uplink) -> Option<Self> {
        let key = std::env::var("TBA_AUTH_KEY").ok()?;
        Self::new(key.trim(), uplink).ok()
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        // Skip the internet probe for a LAN stub; it is reachable exactly when
        // the internet is not.
        if !probe::is_local(&self.base_url) && !probe::probe(&self.uplink).await {
            return Err(UpstreamError::Offline);
        }

        let url = format!("{}{path}", self.base_url);
        let mut last: Option<UpstreamError> = None;

        for attempt in 0..MAX_ATTEMPTS {
            let response = self
                .http
                .get(&url)
                .header("X-TBA-Auth-Key", &self.auth_key)
                .header("Accept", "application/json")
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

    pub async fn oprs(&self, event_key: &str) -> Result<Oprs> {
        self.get(&format!("/event/{event_key}/oprs")).await
    }

    pub async fn component_oprs(&self, event_key: &str) -> Result<ComponentOprs> {
        self.get(&format!("/event/{event_key}/coprs")).await
    }

    pub async fn rankings(&self, event_key: &str) -> Result<Vec<Ranking>> {
        #[derive(serde::Deserialize)]
        struct Envelope {
            #[serde(default)]
            rankings: Vec<Ranking>,
        }
        let envelope: Envelope = self.get(&format!("/event/{event_key}/rankings")).await?;
        Ok(envelope.rankings)
    }

    pub async fn matches(&self, event_key: &str) -> Result<Vec<Match>> {
        self.get(&format!("/event/{event_key}/matches")).await
    }
}
