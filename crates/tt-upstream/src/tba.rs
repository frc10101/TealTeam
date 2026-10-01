//! The Blue Alliance API client (I2).
//!
//! Rankings, OPRs, component OPRs, and match results. TBA allows direct browser
//! requests, which is why the retired plan's "relay server" was unnecessary:
//! any client with signal can fetch this itself and hand the Pi a bundle (S4).
//!
//! Requests are conditional (I9). TBA tags its responses with an `ETag`; the
//! client keeps the last tagged body per path and sends the tag back as
//! `If-None-Match`, and an unchanged resource comes back as an empty `304`.
//! An untagged response is used and simply not kept.
//! During quals the loop asks for the same four resources per event every two
//! minutes and most of them have not moved, so on a phone tether this is most
//! of the data the Pi would otherwise spend.
//!
//! The same requests work from a browser (S4). TBA's CORS preflight allows
//! `If-None-Match` and exposes `ETag`, and a request carrying its own
//! `If-None-Match` skips the browser's HTTP cache, so the 304 reaches this code
//! rather than being answered from that cache. A browser's cache here lives
//! only as long as its page or worker, so [`TbaClient::remember`] seeds it from
//! the client's own upstream log.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use reqwest::StatusCode;
use reqwest::header::{ETAG, HeaderValue, IF_NONE_MATCH};
use serde::de::DeserializeOwned;
use tracing::{debug, warn};

use crate::journal::{Fetched, Recorder};
use crate::{
    MAX_ATTEMPTS, REQUEST_TIMEOUT, Result, Uplink, UpstreamError, backoff, is_retryable, probe,
    truncate,
};
use chrono::Utc;
use tt_core::upstream::{ComponentOprs, Match, Oprs, Rankings, TbaEvent};

const API: &str = "tba";
pub const DEFAULT_BASE_URL: &str = "https://www.thebluealliance.com/api/v3";

/// How many paths' responses are kept for revalidation. Four resources per
/// event, so this covers sixteen events -- far more than are ever live at once.
const CACHE_PATHS: usize = 64;

#[derive(Clone)]
pub struct TbaClient {
    http: reqwest::Client,
    base_url: String,
    auth_key: String,
    uplink: Uplink,
    /// Shared between clones, like the uplink.
    cache: Arc<Mutex<Cache>>,
    /// Where new responses go for the upstream log (S1).
    recorder: Option<Recorder>,
}

/// The last good response per path, for `If-None-Match` (I9).
///
/// In memory only. After a restart the first pass fetches everything in full,
/// once, unless someone [remembered](TbaClient::remember) what was fetched.
#[derive(Default)]
struct Cache {
    entries: HashMap<String, Cached>,
    /// Bumped on every touch; the lowest `used` is the least recently used.
    clock: u64,
}

struct Cached {
    etag: HeaderValue,
    body: Arc<str>,
    used: u64,
}

impl Cache {
    fn get(&mut self, path: &str) -> Option<(HeaderValue, Arc<str>)> {
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(path).map(|c| {
            c.used = clock;
            (c.etag.clone(), c.body.clone())
        })
    }

    fn put(&mut self, path: &str, etag: HeaderValue, body: Arc<str>) {
        if self.entries.len() >= CACHE_PATHS && !self.entries.contains_key(path) {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, c)| c.used)
                .map(|(p, _)| p.clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
        self.clock += 1;
        let used = self.clock;
        self.entries
            .insert(path.to_string(), Cached { etag, body, used });
    }
}

impl TbaClient {
    pub fn new(auth_key: impl Into<String>, uplink: Uplink) -> Result<Self> {
        let auth_key = auth_key.into();
        if auth_key.trim().is_empty() {
            return Err(UpstreamError::NotConfigured("The Blue Alliance"));
        }
        Ok(Self {
            http: reqwest::Client::builder().build().map_err(|source| {
                UpstreamError::Transport {
                    api: API,
                    path: "<client>".into(),
                    source,
                }
            })?,
            base_url: DEFAULT_BASE_URL.to_string(),
            auth_key,
            uplink,
            cache: Arc::default(),
            recorder: None,
        })
    }

    /// Send every response with new content to `recorder` (S1).
    pub fn with_recorder(mut self, recorder: Recorder) -> Self {
        self.recorder = Some(recorder);
        self
    }

    /// Point at a different host. For tests against a local stub.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// The key this client sends. The Pi hands it to a lead scout's device,
    /// which fetches with its own signal (S7).
    pub fn auth_key(&self) -> &str {
        &self.auth_key
    }

    /// Where requests go: TBA, or a test's stub.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Read `TBA_AUTH_KEY`. Absent means the whole TBA sync is disabled, which
    /// is a supported configuration rather than an error.
    ///
    /// `TBA_BASE_URL` points it somewhere else, such as a browser test's stub
    /// (S7). Unset or blank is TBA.
    pub fn from_env(uplink: Uplink) -> Option<Self> {
        let key = std::env::var("TBA_AUTH_KEY").ok()?;
        let client = Self::new(key.trim(), uplink).ok()?;
        match std::env::var("TBA_BASE_URL") {
            Ok(base) if !base.trim().is_empty() => {
                Some(client.with_base_url(base.trim().trim_end_matches('/')))
            }
            _ => Some(client),
        }
    }

    /// Hold `body` as what `path` last returned under `etag`, so the next
    /// request for it is conditional (S4).
    ///
    /// For a client starting cold: it seeds the cache from the newest row per
    /// path in its upstream log, and a phone that fetched the schedule
    /// yesterday spends a 304 on it today. `path` is the log's, e.g.
    /// `/event/2026mabil/matches`. A tag that is not a valid header is ignored.
    pub fn remember(&self, path: &str, etag: &str, body: impl Into<Arc<str>>) {
        let Ok(etag) = HeaderValue::from_str(etag) else {
            return;
        };
        if let Ok(mut cache) = self.cache.lock() {
            cache.put(path, etag, body.into());
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        // Skip the internet probe for a LAN stub; it is reachable exactly when
        // the internet is not.
        if !probe::is_local(&self.base_url) && !probe::probe(&self.uplink).await {
            return Err(UpstreamError::Offline);
        }

        let url = format!("{}{path}", self.base_url);
        // Read once, before any retry: a 304 vouches for the tag that was sent,
        // so the body it refers to is this one, whatever happens meanwhile.
        let cached = self.cache.lock().ok().and_then(|mut c| c.get(path));
        let mut last: Option<UpstreamError> = None;

        for attempt in 0..MAX_ATTEMPTS {
            let mut request = self
                .http
                .get(&url)
                // Per request, not on the client: a browser's client has no
                // timeout of its own.
                .timeout(REQUEST_TIMEOUT)
                .header("X-TBA-Auth-Key", &self.auth_key)
                .header("Accept", "application/json");
            if let Some((etag, _)) = &cached {
                request = request.header(IF_NONE_MATCH, etag.clone());
            }

            match request.send().await {
                Ok(response) => {
                    // Unchanged: the body we already hold is current. A 304 we
                    // did not ask for falls through to the error below.
                    if response.status() == StatusCode::NOT_MODIFIED
                        && let Some((_, body)) = &cached
                    {
                        debug!("tba {path} not modified");
                        return self.parse(path, body);
                    }

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
                            crate::sleep(backoff(attempt)).await;
                            last = Some(error);
                            continue;
                        }
                        return Err(error);
                    }

                    let etag = response.headers().get(ETAG).cloned();
                    let body =
                        response
                            .text()
                            .await
                            .map_err(|source| UpstreamError::Transport {
                                api: API,
                                path: path.to_string(),
                                source,
                            })?;

                    let value = self.parse(path, &body)?;
                    let body: Arc<str> = body.into();
                    if let Some(recorder) = &self.recorder {
                        recorder.record(Fetched {
                            api: API,
                            path: path.to_string(),
                            etag: etag
                                .as_ref()
                                .and_then(|e| e.to_str().ok())
                                .map(str::to_string),
                            body: body.clone(),
                            fetched_at: Utc::now(),
                        });
                    }
                    // Only a body that parsed is worth revalidating against.
                    if let Some(etag) = etag
                        && let Ok(mut cache) = self.cache.lock()
                    {
                        cache.put(path, etag, body);
                    }
                    return Ok(value);
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
                        crate::sleep(backoff(attempt)).await;
                        last = Some(error);
                        continue;
                    }
                    return Err(error);
                }
            }
        }

        Err(last.unwrap_or(UpstreamError::Offline))
    }

    /// Read a body, and tell the uplink how it went.
    fn parse<T: DeserializeOwned>(&self, path: &str, body: &str) -> Result<T> {
        match serde_json::from_str(body) {
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
        }
    }

    // Each of these reads a `null` body as empty: TBA answers `null`, not
    // `{}`, for an event it has nothing for yet.

    pub async fn oprs(&self, event_key: &str) -> Result<Oprs> {
        self.get_or_empty(&format!("/event/{event_key}/oprs")).await
    }

    pub async fn component_oprs(&self, event_key: &str) -> Result<ComponentOprs> {
        self.get_or_empty(&format!("/event/{event_key}/coprs"))
            .await
    }

    pub async fn rankings(&self, event_key: &str) -> Result<Rankings> {
        self.get_or_empty(&format!("/event/{event_key}/rankings"))
            .await
    }

    pub async fn matches(&self, event_key: &str) -> Result<Vec<Match>> {
        self.get_or_empty(&format!("/event/{event_key}/matches"))
            .await
    }

    /// Every event TBA has for `year`: one request, to learn its keys for
    /// FIRST's codes (I15).
    pub async fn events(&self, year: i32) -> Result<Vec<TbaEvent>> {
        self.get_or_empty(&format!("/events/{year}")).await
    }

    async fn get_or_empty<T: DeserializeOwned + Default>(&self, path: &str) -> Result<T> {
        Ok(self.get::<Option<T>>(path).await?.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(n: usize) -> HeaderValue {
        HeaderValue::from_str(&format!("\"v{n}\"")).unwrap()
    }

    #[test]
    fn a_full_cache_forgets_the_path_least_recently_used() {
        let mut cache = Cache::default();
        for n in 0..CACHE_PATHS {
            cache.put(&format!("/p{n}"), tag(n), "[]".into());
        }
        // Touch the oldest, so the second-oldest is now the one to go.
        assert!(cache.get("/p0").is_some());

        cache.put("/new", tag(99), "[]".into());
        assert_eq!(cache.entries.len(), CACHE_PATHS);
        assert!(cache.get("/p0").is_some());
        assert!(cache.get("/p1").is_none());
        assert!(cache.get("/new").is_some());
    }

    #[test]
    fn replacing_a_path_does_not_evict_anything() {
        let mut cache = Cache::default();
        for n in 0..CACHE_PATHS {
            cache.put(&format!("/p{n}"), tag(n), "[]".into());
        }
        cache.put("/p5", tag(500), "[1]".into());
        assert_eq!(cache.entries.len(), CACHE_PATHS);
        let (etag, body) = cache.get("/p5").unwrap();
        assert_eq!((etag, &*body), (tag(500), "[1]"));
    }
}
