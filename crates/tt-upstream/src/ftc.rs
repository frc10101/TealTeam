//! The optional FTC Events API pull (FTC_BETA_PLAN.md, F7).
//!
//! One event, `USALATQ` in season 2026, over HTTP basic auth to
//! `https://ftc-api.firstinspires.org/v2.0`. `docs/ftc/FTC_BETA_API.md` has the
//! mapping. A pull fetches the event, its teams, the qualification hybrid
//! schedule, the rankings, and the qualification score sheets, and stores
//! them the way the CSV import (F3) does:
//!
//! - the event and roster through the same `Repo` calls as F3 and FIRST's
//!   sync;
//! - the schedule as the TBA `/event/{key}/matches` body F3 writes, applied
//!   by [`project`](crate::project), so the two paths store the same rows;
//! - the rankings into `team_event_stats`, with the OPRs worked out from
//!   the played matches and score sheets (F10, `tt_core::ftc::ratings`);
//! - the score sheets in the log, read only for the phase OPRs.
//!
//! Then it appends each new body to the upstream log as `api = 'ftc'`.
//!
//! The caller then tidies the schedule ([`Pulled::tidy`]), as F3 does, so a
//! re-pull keeps every match that has an observation.
//!
//! Each request sends `If-Modified-Since` with the `Last-Modified` of the
//! newest logged body for it, which the log keeps in its `etag` column. It
//! never also sends `FMS-OnlyModifiedSince`: that returns only the changed
//! records, and a body here must be the whole current state. A 503 (or 429)
//! is waited out when its `Retry-After` is short ([`MAX_RETRY_WAIT`]);
//! otherwise that request fails with [`UpstreamError::Busy`], and the pull
//! stores nothing (unless only the score sheets were busy).

use std::collections::BTreeSet;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use tracing::{debug, info, warn};
use tt_core::ftc::{self, Events, HybridSchedule, Rankings, ScoreSheets, Teams, Worked};
use tt_core::records::{Event, TeamEventStats};
use tt_core::schedule::Surrogate;
use tt_core::{timezone, upstream};
use tt_repo::{NewUpstream, Repo};

use crate::{
    MAX_ATTEMPTS, REQUEST_TIMEOUT, Result, Uplink, UpstreamError, backoff, is_retryable, probe,
    truncate,
};

pub const API: &str = "ftc";
pub const DEFAULT_BASE_URL: &str = "https://ftc-api.firstinspires.org/v2.0";
pub const DEFAULT_EVENT_CODE: &str = "USALATQ";
/// This build's one season.
pub const SEASON: i32 = 2026;
/// Where the event is, when FTC does not say: Athens, Alabama.
pub const DEFAULT_ZONE: chrono_tz::Tz = chrono_tz::America::Chicago;
/// The most pages of the roster to follow. USALATQ has one.
const MAX_PAGES: u32 = 5;
/// The longest a `Retry-After` is waited out inside a pull. Longer, and the
/// pull stops and says when to try again.
pub const MAX_RETRY_WAIT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct FtcClient {
    http: reqwest::Client,
    base_url: String,
    username: String,
    key: String,
    event_code: String,
    uplink: Uplink,
}

/// One answer: a body, or "nothing new since the time you gave".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    Body {
        body: String,
        last_modified: Option<String>,
    },
    NotModified,
}

impl FtcClient {
    pub fn new(
        username: impl Into<String>,
        key: impl Into<String>,
        uplink: Uplink,
    ) -> Result<Self> {
        let (username, key) = (username.into(), key.into());
        if username.trim().is_empty() || key.trim().is_empty() {
            return Err(UpstreamError::NotConfigured("FTC Events API"));
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
            username: username.trim().to_string(),
            key: key.trim().to_string(),
            event_code: DEFAULT_EVENT_CODE.to_string(),
            uplink,
        })
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    pub fn with_event_code(mut self, code: impl Into<String>) -> Self {
        self.event_code = code.into().trim().to_ascii_uppercase();
        self
    }

    /// `FTC_API_USERNAME` and `FTC_API_KEY`, both required; `FTC_EVENT_CODE`
    /// (default `USALATQ`) and `FTC_API_BASE_URL` (for a stub) optional.
    pub fn from_env(uplink: Uplink) -> Option<Self> {
        let env = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let mut client = Self::new(env("FTC_API_USERNAME")?, env("FTC_API_KEY")?, uplink).ok()?;
        if let Some(code) = env("FTC_EVENT_CODE") {
            client = client.with_event_code(code);
        }
        if let Some(base) = env("FTC_API_BASE_URL") {
            client = client.with_base_url(base);
        }
        Some(client)
    }

    pub fn event_code(&self) -> &str {
        &self.event_code
    }

    /// The event's key here, the one the CSV import makes from the same code:
    /// `2026usalatq`.
    pub fn event_key(&self) -> String {
        upstream::normalize_event_key(&self.event_code, SEASON).unwrap_or_default()
    }

    /// `GET {base}{path}`, with `If-Modified-Since` when given.
    pub async fn get(&self, path: &str, if_modified_since: Option<&str>) -> Result<Fetched> {
        if !probe::is_local(&self.base_url) && !probe::probe(&self.uplink).await {
            return Err(UpstreamError::Offline);
        }
        let url = format!("{}{path}", self.base_url);
        let mut last = None;
        for attempt in 0..MAX_ATTEMPTS {
            let mut request = self
                .http
                .get(&url)
                .timeout(REQUEST_TIMEOUT)
                .basic_auth(&self.username, Some(&self.key))
                .header("Accept", "application/json");
            if let Some(since) = if_modified_since {
                request = request.header("If-Modified-Since", since);
            }
            let error =
                match request.send().await {
                    Ok(response) => {
                        let status = response.status().as_u16();
                        if status == 304 {
                            self.uplink.record_success();
                            return Ok(Fetched::NotModified);
                        }
                        if response.status().is_success() {
                            let last_modified = response
                                .headers()
                                .get("Last-Modified")
                                .and_then(|v| v.to_str().ok())
                                .map(str::to_string);
                            let body = response.text().await.map_err(|source| {
                                UpstreamError::Transport {
                                    api: API,
                                    path: path.to_string(),
                                    source,
                                }
                            })?;
                            self.uplink.record_success();
                            return Ok(Fetched::Body {
                                body,
                                last_modified,
                            });
                        }
                        if status == 503 || status == 429 {
                            // Busy: wait as long as it asks, if that is short.
                            let asked = retry_after(response.headers(), Utc::now());
                            let error = UpstreamError::Busy {
                                api: API,
                                path: path.to_string(),
                                status,
                                retry_after: asked.map(|d| d.as_secs()),
                            };
                            self.uplink.record_error(&error.to_string());
                            let wait = asked.unwrap_or_else(|| backoff(attempt));
                            if attempt + 1 >= MAX_ATTEMPTS || wait > MAX_RETRY_WAIT {
                                return Err(error);
                            }
                            debug!("{error}; retrying in {wait:?}");
                            crate::sleep(wait).await;
                            last = Some(error);
                            continue;
                        }
                        let body = truncate(&response.text().await.unwrap_or_default());
                        let error = UpstreamError::Status {
                            api: API,
                            path: path.to_string(),
                            status,
                            body,
                        };
                        self.uplink.record_error(&error.to_string());
                        if !is_retryable(status) {
                            return Err(error);
                        }
                        error
                    }
                    Err(source) => {
                        let error = UpstreamError::Transport {
                            api: API,
                            path: path.to_string(),
                            source,
                        };
                        self.uplink.record_error(&error.to_string());
                        error
                    }
                };
            if attempt + 1 < MAX_ATTEMPTS {
                debug!("{error}; retrying");
                crate::sleep(backoff(attempt)).await;
            }
            last = Some(error);
        }
        Err(last.unwrap_or(UpstreamError::Offline))
    }
}

/// `Retry-After` as a wait: seconds, or an HTTP date.
fn retry_after(headers: &reqwest::header::HeaderMap, now: DateTime<Utc>) -> Option<Duration> {
    let raw = headers.get("Retry-After")?.to_str().ok()?.trim();
    if let Ok(seconds) = raw.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let at = DateTime::parse_from_rfc2822(raw).ok()?.with_timezone(&Utc);
    Some((at - now).to_std().unwrap_or(Duration::ZERO))
}

/// What a pull did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pulled {
    pub event_key: String,
    pub event_name: String,
    /// Some response had new content. False: every one was a 304 or the same
    /// body as before, and nothing was stored.
    pub changed: bool,
    pub teams: usize,
    pub matches: usize,
    pub played: usize,
    pub surrogates: Vec<Surrogate>,
    pub rankings: usize,
    pub score_sheets: usize,
    /// The OPRs, when this pull worked them out again (F10).
    pub rated: Option<Rated>,
    /// The schedule's match keys and the event's roster, when the schedule or
    /// the roster changed: for `SqliteRepo::tidy_schedule`, as after an
    /// import.
    pub tidy: Option<(Vec<String>, Vec<i32>)>,
    pub problems: Vec<String>,
}

/// What working out the OPRs came to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rated {
    /// Teams given an OPR: none while too few matches have been played.
    pub teams: usize,
    /// Played matches they were worked out from.
    pub played: usize,
}

/// One request: the newest body for it, and whether this pull brought a new
/// one. A new body is logged only once the pull has stored what it says
/// ([`log`]), so a pull cut short leaves no `Last-Modified` behind that would
/// turn the next pull's request into a 304 for data never stored.
struct Logged {
    path: String,
    body: Option<String>,
    last_modified: Option<String>,
    changed: bool,
}

async fn fetch<R: Repo + Sync>(repo: &R, client: &FtcClient, path: &str) -> Result<Logged> {
    let stored = repo
        .latest_upstream(API, path)
        .await
        .map_err(|e| repo_error(path, e))?;
    let since = stored.as_ref().and_then(|s| s.entry.etag.clone());
    let stored = stored.map(|s| s.entry.body);
    match client.get(path, since.as_deref()).await? {
        Fetched::NotModified => Ok(Logged {
            path: path.to_string(),
            body: stored,
            last_modified: None,
            changed: false,
        }),
        Fetched::Body {
            body,
            last_modified,
        } => {
            if let Err(source) = serde_json::from_str::<serde_json::Value>(&body) {
                return Err(UpstreamError::Payload {
                    api: API,
                    path: path.to_string(),
                    source,
                });
            }
            Ok(Logged {
                path: path.to_string(),
                changed: stored.as_deref() != Some(body.as_str()),
                body: Some(body),
                last_modified,
            })
        }
    }
}

/// Append each new body to the upstream log as `ftc`, with its
/// `Last-Modified` for the next pull's `If-Modified-Since`.
async fn log<R: Repo + Sync>(repo: &R, fetched: &[&Logged]) -> Result<()> {
    for logged in fetched.iter().filter(|l| l.changed) {
        let Some(body) = &logged.body else { continue };
        repo.append_upstream(&NewUpstream {
            api: API.into(),
            path: logged.path.clone(),
            etag: logged.last_modified.clone(),
            body: body.clone(),
            fetched_at: Utc::now(),
            via: "pi".into(),
        })
        .await
        .map_err(|e| repo_error(&logged.path, e))?;
    }
    Ok(())
}

fn repo_error(path: &str, e: tt_repo::RepoError) -> UpstreamError {
    UpstreamError::Status {
        api: "repo",
        path: path.to_string(),
        status: 0,
        body: e.to_string(),
    }
}

fn parse<T: serde::de::DeserializeOwned + Default>(logged: &Logged, path: &str) -> Result<T> {
    match &logged.body {
        None => Ok(T::default()),
        Some(body) => serde_json::from_str(body).map_err(|source| UpstreamError::Payload {
            api: API,
            path: path.to_string(),
            source,
        }),
    }
}

/// Pull the event. An `Err` is a request that failed whole (no key, a 401,
/// no internet); what was stored before it stays stored.
pub async fn pull<R: Repo + Sync>(repo: &R, client: &FtcClient) -> Result<Pulled> {
    let code = client.event_code().to_string();
    let key = client.event_key();
    let season = SEASON;
    let mut pulled = Pulled {
        event_key: key.clone(),
        ..Pulled::default()
    };

    let events_path = format!("/{season}/events?eventCode={code}");
    let events = fetch(repo, client, &events_path).await?;
    let teams_path = |page: u32| match page {
        1 => format!("/{season}/teams?eventCode={code}"),
        n => format!("/{season}/teams?eventCode={code}&page={n}"),
    };
    let mut team_pages = vec![fetch(repo, client, &teams_path(1)).await?];
    let total = parse::<Teams>(&team_pages[0], &teams_path(1))?
        .page_total
        .unwrap_or(1)
        .min(MAX_PAGES);
    for page in 2..=total {
        team_pages.push(fetch(repo, client, &teams_path(page)).await?);
    }
    let schedule_path = format!("/{season}/schedule/{code}/qual/hybrid");
    let schedule = fetch(repo, client, &schedule_path).await?;
    let rankings_path = format!("/{season}/rankings/{code}");
    let rankings = fetch(repo, client, &rankings_path).await?;
    // Raw only, for after the event: a failure here costs nothing else.
    let scores_path = format!("/{season}/scores/{code}/qual");
    let scores = match fetch(repo, client, &scores_path).await {
        Ok(scores) => {
            pulled.score_sheets = scores
                .body
                .as_deref()
                .and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok())
                .and_then(|v| {
                    v.get("matchScores")
                        .and_then(|m| m.as_array())
                        .map(Vec::len)
                })
                .unwrap_or(0);
            Some(scores)
        }
        Err(e) if e.is_offline() => return Err(e),
        Err(e) => {
            pulled
                .problems
                .push(format!("The score sheets did not come: {e}"));
            None
        }
    };

    let teams_changed = team_pages.iter().any(|p| p.changed);
    pulled.changed = events.changed || teams_changed || schedule.changed || rankings.changed;

    // The event.
    let now = Utc::now();
    let existing = repo.event(&key).await.map_err(|e| repo_error(&key, e))?;
    let listed = parse::<Events>(&events, &events_path)?
        .events
        .into_iter()
        .find(|e| e.code.trim().eq_ignore_ascii_case(&code));
    let event = match (listed, existing) {
        (Some(raw), existing) if events.changed || existing.is_none() => {
            let event = to_event(&key, &raw, existing.as_ref());
            repo.upsert_event(&event, now)
                .await
                .map_err(|e| repo_error(&key, e))?;
            pulled.changed = true;
            event
        }
        (_, Some(existing)) => existing,
        _ => {
            pulled.problems.push(format!(
                "FTC Events lists no event {code} for {season} yet, so nothing was stored."
            ));
            return Ok(pulled);
        }
    };
    pulled.event_name = event.name.clone();
    if !pulled.changed {
        return Ok(pulled);
    }
    let zone = event
        .timezone
        .as_deref()
        .and_then(|z| z.parse().ok())
        .unwrap_or(DEFAULT_ZONE);

    // The roster.
    let mut listed_teams = Vec::new();
    for (i, page) in team_pages.iter().enumerate() {
        listed_teams.extend(parse::<Teams>(page, &teams_path(i as u32 + 1))?.teams);
    }
    pulled.teams = listed_teams.len();
    if teams_changed {
        let stored = crate::sync::store_roster(repo, &key, &listed_teams).await;
        pulled.problems.extend(stored.problems);
    }

    // The schedule, as the CSV import stores it.
    let parsed = parse::<HybridSchedule>(&schedule, &schedule_path)?;
    let mapped = ftc::map_schedule(&key, &parsed, zone);
    pulled.matches = mapped.keys.len();
    pulled.played = mapped.played;
    pulled.problems.extend(mapped.problems.iter().cloned());
    if mapped.keys.is_empty() {
        pulled
            .problems
            .push("The qualification schedule is not posted yet.".into());
    } else {
        if schedule.changed {
            let path = format!("/event/{key}/matches");
            repo.append_upstream(&NewUpstream {
                api: "tba".into(),
                path: path.clone(),
                etag: None,
                body: mapped.body.clone(),
                fetched_at: now,
                via: "ftc".into(),
            })
            .await
            .map_err(|e| repo_error(&path, e))?;
            let projected = crate::project::project(repo, &[("tba".into(), path)]).await;
            pulled.problems.extend(projected.report.problems);
        }
        if schedule.changed || teams_changed {
            let mut roster: BTreeSet<i32> = listed_teams.iter().map(|t| t.team_number).collect();
            for team in &mapped.teams {
                if !roster.contains(team) {
                    // A scheduled team the roster missed: link it, as F3 does.
                    ensure_team(repo, &key, *team).await?;
                    roster.insert(*team);
                }
            }
            pulled.tidy = Some((mapped.keys.clone(), roster.into_iter().collect()));
        }
    }
    pulled.surrogates = mapped.surrogates;

    // The rankings, and the OPRs worked out from the played matches (F10).
    let ranked = parse::<Rankings>(&rankings, &rankings_path)?.rankings;
    let scores_changed = scores.as_ref().is_some_and(|s| s.changed);
    pulled.rankings = ranked.len();
    if rankings.changed || schedule.changed || scores_changed || teams_changed {
        let sheets = match &scores {
            Some(logged) => parse::<ScoreSheets>(logged, &scores_path).unwrap_or_default(),
            None => ScoreSheets::default(),
        };
        let worked = ftc::ratings(&parsed, &sheets);
        let fresh = (rankings.changed && !ranked.is_empty()).then_some(ranked.as_slice());
        let stored = store_stats(repo, &key, fresh, &worked, &mut pulled.problems).await?;
        if fresh.is_some() {
            pulled.rankings = stored.ranked;
        }
        pulled.rated = Some(Rated {
            teams: stored.rated,
            played: worked.played,
        });
    }

    // Stored: now the bodies may go in the log.
    let mut fetched = vec![&events, &schedule, &rankings];
    fetched.extend(&team_pages);
    fetched.extend(&scores);
    log(repo, &fetched).await?;

    info!(
        event = %key,
        teams = pulled.teams,
        matches = pulled.matches,
        rankings = pulled.rankings,
        "pulled from FTC Events"
    );
    Ok(pulled)
}

fn to_event(key: &str, raw: &upstream::FirstEvent, existing: Option<&Event>) -> Event {
    let given = raw.timezone.as_deref().unwrap_or_default();
    let zone = timezone::event_zone(given, &raw.country, &raw.stateprov)
        .map(|tz| tz.name().to_string())
        .or_else(|| existing.and_then(|e| e.timezone.clone()))
        .or_else(|| Some(DEFAULT_ZONE.name().to_string()));
    let date = |raw: &str| {
        upstream::parse_date(raw).and_then(|(y, m, d)| NaiveDate::from_ymd_opt(y, m, d))
    };
    Event {
        key: key.to_string(),
        name: Some(raw.name.trim().to_string())
            .filter(|n| !n.is_empty())
            .or_else(|| existing.map(|e| e.name.clone()))
            .unwrap_or_else(|| raw.code.clone()),
        location: Some(raw.location()).filter(|l| !l.is_empty()),
        timezone: zone,
        start_date: date(&raw.date_start).or_else(|| existing.and_then(|e| e.start_date)),
        end_date: date(&raw.date_end).or_else(|| existing.and_then(|e| e.end_date)),
        event_code: Some(raw.code.trim().to_ascii_lowercase()),
        event_type: raw.event_type.clone(),
        district_key: raw.district_code.clone(),
        week: None,
    }
}

async fn ensure_team<R: Repo + Sync>(repo: &R, event_key: &str, number: i32) -> Result<()> {
    let now = Utc::now();
    let known = repo
        .team(number)
        .await
        .map_err(|e| repo_error(event_key, e))?;
    if known.is_none() {
        let team = tt_core::records::Team {
            number,
            name: format!("Team {number}"),
            nickname: None,
            school: None,
            city: None,
            state: None,
            country: None,
            rookie_year: None,
            website: None,
        };
        repo.upsert_team(&team, now)
            .await
            .map_err(|e| repo_error(event_key, e))?;
    }
    repo.link_event_team(event_key, number, now)
        .await
        .map_err(|e| repo_error(event_key, e))
}

/// What [`store_stats`] stored.
struct Stored {
    ranked: usize,
    rated: usize,
}

/// Every team at the event: its rank, record, and average when `ranked` is
/// a fresh ranking, kept as they were otherwise (typed in, or the last
/// pull's), and the OPRs `worked` out, empty where they could not be (F10).
/// As FRC's `sync::store_stats` stores TBA's.
async fn store_stats<R: Repo + Sync>(
    repo: &R,
    event_key: &str,
    ranked: Option<&[ftc::Ranking]>,
    worked: &Worked,
    problems: &mut Vec<String>,
) -> Result<Stored> {
    let now = Utc::now();
    let roster = repo
        .event_teams(event_key)
        .await
        .map_err(|e| repo_error(event_key, e))?;
    let before = repo
        .event_stats(event_key)
        .await
        .map_err(|e| repo_error(event_key, e))?;
    let mut stored = Stored {
        ranked: 0,
        rated: 0,
    };
    for team in &roster {
        let key = upstream::team_key(team.number);
        let mut stats = before
            .iter()
            .find(|s| s.team_number == team.number)
            .cloned()
            .unwrap_or_else(|| TeamEventStats {
                team_number: team.number,
                event_key: event_key.to_string(),
                ..TeamEventStats::default()
            });
        if let Some(ranked) = ranked {
            let ranking = ranked.iter().find(|r| r.team_number == team.number);
            stats.rank = ranking.map(|r| r.rank).filter(|r| *r > 0);
            stats.matches_played = ranking.and_then(|r| r.matches_played);
            stats.qual_average = ranking.and_then(|r| r.qual_average);
            stats.wins = ranking.and_then(|r| r.wins);
            stats.losses = ranking.and_then(|r| r.losses);
            stats.ties = ranking.and_then(|r| r.ties);
            stats.dq_count = ranking.and_then(|r| r.dq);
            stored.ranked += usize::from(ranking.is_some());
        }
        stats.opr = worked.oprs.oprs.get(&key).copied();
        stats.dpr = worked.oprs.dprs.get(&key).copied();
        stats.ccwm = worked.oprs.ccwms.get(&key).copied();
        (stats.auto_opr, stats.teleop_opr, stats.endgame_opr) = worked.components.phases(&key);
        stats.synced_at = Some(now);
        match repo.upsert_team_stats(&stats, now).await {
            Ok(()) => stored.rated += usize::from(stats.opr.is_some()),
            Err(e) => {
                warn!("storing FTC stats for {}: {e}", team.number);
                problems.push(format!("storing the stats for team {}: {e}", team.number));
            }
        }
    }
    let unknown = ranked
        .unwrap_or_default()
        .iter()
        .filter(|r| !roster.iter().any(|t| t.number == r.team_number))
        .count();
    if unknown > 0 {
        problems.push(format!(
            "{unknown} ranked team(s) are not on the event's roster and were left out."
        ));
    }
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_and_a_username_are_both_needed() {
        assert!(FtcClient::new("", "key", Uplink::new()).is_err());
        assert!(FtcClient::new("user", " ", Uplink::new()).is_err());
    }

    #[test]
    fn retry_after_is_seconds_or_a_date() {
        use reqwest::header::{HeaderMap, HeaderValue};
        let now = DateTime::parse_from_rfc3339("2026-10-24T14:10:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let with = |v: &'static str| {
            let mut h = HeaderMap::new();
            h.insert("Retry-After", HeaderValue::from_static(v));
            retry_after(&h, now)
        };
        assert_eq!(with("3"), Some(Duration::from_secs(3)));
        assert_eq!(
            with("Sat, 24 Oct 2026 14:12:00 GMT"),
            Some(Duration::from_secs(120))
        );
        assert_eq!(with("Sat, 24 Oct 2026 14:00:00 GMT"), Some(Duration::ZERO));
        assert_eq!(with("soon"), None);
        assert_eq!(retry_after(&HeaderMap::new(), now), None);
    }

    #[test]
    fn the_event_key_is_the_one_the_csv_import_makes() {
        let client = FtcClient::new("user", "key", Uplink::new()).unwrap();
        assert_eq!(client.event_key(), "2026usalatq");
        assert_eq!(
            client.event_key(),
            upstream::normalize_event_key("USALATQ", 2026).unwrap()
        );
    }
}
