//! Seeding the schedule by hand (FTC_BETA_PLAN.md, F3).
//!
//! `GET /lead-scout/schedule?event=` shows the form; `POST
//! /api/schedule/import?event=` imports. Reading the CSV and every rule about
//! it live in `tt_core::schedule`. This is the glue, and it stores what it
//! read the way a sync does:
//!
//! - the event, its teams, and the roster links through the same `Repo`
//!   calls as FIRST's sync (`tt_upstream::sync::sync_events`);
//! - the matches as a TBA `/event/{key}/matches` body appended to the
//!   upstream log and applied by `tt_upstream::project`, as a pushed bundle
//!   is. So the change log, the live stream, and snapshots carry them as
//!   they would synced ones;
//! - then `SqliteRepo::tidy_schedule`, so a re-import replaces the schedule
//!   without deleting an observation.

use chrono::Utc;
use serde_json::json;
use tracing::{info, warn};
use tt_core::records::Event;
use tt_core::schedule::{self, Schedule};
use tt_core::upstream::team_key;
use tt_core::user::User;
use tt_repo::{NewUpstream, Repo};
use tt_templates::{Nav, ScheduleImportPage};
use tt_upstream::project;

use crate::events::EventContext;
use crate::startup::AppState;

const NOT_SAVED: &str = "The server's storage did not answer. Try again.";

/// What the form posts.
#[derive(Debug, Clone, Default)]
pub struct Posted {
    pub code: String,
    pub name: String,
    pub timezone: String,
    pub date: String,
    pub schedule: String,
    pub roster: String,
}

impl Posted {
    pub fn from_pairs(pairs: &[(String, String)]) -> Self {
        let field = |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        Self {
            code: field("code"),
            name: field("name"),
            timezone: field("timezone"),
            date: field("date"),
            schedule: field("schedule"),
            roster: field("roster"),
        }
    }
}

/// What an import did, for the page.
#[derive(Debug, Clone)]
pub struct Imported {
    pub event_key: String,
    pub report: Vec<String>,
    /// `"Q4 blue1: 10002"`.
    pub surrogates: Vec<String>,
}

/// The page. `posted` is a refused import's form, shown again as typed.
pub async fn page(
    state: &AppState,
    nav: Nav,
    context: &EventContext,
    posted: Option<Posted>,
    errors: Vec<String>,
    imported: Option<Imported>,
) -> ScheduleImportPage {
    let event = context.selected.as_ref();
    let event_query = nav.event.query();
    let mut page = ScheduleImportPage {
        title: "Match schedule".into(),
        nav,
        refused: posted.is_some(),
        errors,
        report: Vec::new(),
        surrogates: Vec::new(),
        header: schedule::header(),
        code: String::new(),
        name: String::new(),
        timezone: String::new(),
        date: String::new(),
        schedule: String::new(),
        roster: String::new(),
        event_name: String::new(),
        stored: None,
        current: String::new(),
        post_href: format!("/api/schedule/import{event_query}"),
        back_href: format!("/lead-scout{event_query}"),
        assignments_href: format!("/lead-scout/assignments{event_query}"),
    };
    if let Some(imported) = imported {
        page.report = imported.report;
        page.surrogates = imported.surrogates;
    }
    if let Some(unknown) = &context.unknown {
        page.errors
            .push(format!("There is no event “{unknown}” on this server yet."));
    }

    if let Some(event) = event {
        page.event_name = event.name.clone();
        page.code = event
            .event_code
            .clone()
            .unwrap_or_else(|| event.key.clone())
            .to_ascii_uppercase();
        page.name = event.name.clone();
        page.timezone = event.timezone.clone().unwrap_or_default();
        page.date = event
            .start_date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        page.stored = crate::events::stored(&*state.repo, context).await;
        match state.repo.event_matches(&event.key).await {
            Ok(matches) if !matches.is_empty() => {
                let rows: Vec<_> = matches
                    .iter()
                    .filter(|m| m.comp_level == tt_core::matches::CompLevel::Qualification)
                    .map(|m| (m.match_number, m.red, m.blue))
                    .collect();
                page.current = schedule::format(&rows);
            }
            Ok(_) => {}
            Err(e) => warn!("matches for {}: {e}", event.key),
        }
    }
    if page.date.is_empty() {
        let zone = page.timezone.parse().ok();
        page.date = tt_core::timezone::local_date(zone, Utc::now())
            .format("%Y-%m-%d")
            .to_string();
    }
    if let Some(posted) = posted {
        page.code = posted.code;
        page.name = posted.name;
        page.timezone = posted.timezone;
        page.date = posted.date;
        page.schedule = posted.schedule;
        page.roster = posted.roster;
    }
    page
}

/// An import: what it did, or every reason it did nothing.
pub async fn import(
    state: &AppState,
    user: &User,
    posted: &Posted,
) -> Result<Imported, Vec<String>> {
    let now = Utc::now();
    let key = tt_core::upstream::normalize_event_key(&posted.code, state.season.season);
    let existing = match &key {
        Some(key) => state.repo.event(key).await.map_err(|e| {
            warn!("loading event {key}: {e}");
            vec![NOT_SAVED.to_string()]
        })?,
        None => None,
    };

    let event = schedule::event_from_form(
        &posted.code,
        &posted.name,
        &posted.timezone,
        &posted.date,
        state.season.season,
        existing.as_ref(),
        now,
    );
    let parsed = schedule::parse(&posted.schedule, &posted.roster);
    let (event, parsed) = match (event, parsed) {
        (Ok(event), Ok(parsed)) => (event, parsed),
        (event, parsed) => {
            let mut errors = event.err().unwrap_or_default();
            errors.extend(parsed.err().unwrap_or_default());
            return Err(errors);
        }
    };

    let stored = store(state, &event, &parsed).await.map_err(|e| {
        warn!("importing the schedule for {}: {e}", event.key);
        vec![NOT_SAVED.to_string()]
    })?;
    info!(
        user = %user.email,
        event = %event.key,
        matches = parsed.matches.len(),
        teams = parsed.roster.len(),
        "schedule imported by hand"
    );
    Ok(Imported {
        event_key: event.key.clone(),
        report: stored,
        surrogates: parsed.surrogates.iter().map(|s| s.describe()).collect(),
    })
}

/// The TBA `/event/{key}/matches` body a sync would have fetched.
fn matches_body(event_key: &str, schedule: &Schedule) -> String {
    let alliance = |slots: &tt_core::Alliance| {
        json!({
            "score": -1,
            "team_keys": slots.iter().flatten().map(|t| team_key(*t)).collect::<Vec<_>>(),
        })
    };
    let matches: Vec<_> = schedule
        .matches
        .iter()
        .map(|m| {
            json!({
                "key": format!("{event_key}_qm{}", m.number),
                "comp_level": "qm",
                "set_number": 1,
                "match_number": m.number,
                "alliances": { "red": alliance(&m.red), "blue": alliance(&m.blue) },
            })
        })
        .collect();
    serde_json::Value::Array(matches).to_string()
}

/// Store a parsed schedule, and say what that did.
async fn store(state: &AppState, event: &Event, parsed: &Schedule) -> tt_repo::Result<Vec<String>> {
    let now = Utc::now();
    let repo = &*state.repo;
    repo.upsert_event(event, now).await?;

    for team in &parsed.roster {
        // A name typed in wins; otherwise a team already known keeps its own.
        let known = repo.team(team.number).await?;
        if team.name.is_some() || known.is_none() {
            let mut stored = known.unwrap_or(tt_core::records::Team {
                number: team.number,
                name: String::new(),
                nickname: None,
                school: None,
                city: None,
                state: None,
                country: None,
                rookie_year: None,
                website: None,
            });
            stored.name = team
                .name
                .clone()
                .unwrap_or_else(|| format!("Team {}", team.number));
            repo.upsert_team(&stored, now).await?;
        }
        repo.link_event_team(&event.key, team.number, now).await?;
    }

    let path = format!("/event/{}/matches", event.key);
    repo.append_upstream(&NewUpstream {
        api: "tba".into(),
        path: path.clone(),
        etag: None,
        body: matches_body(&event.key, parsed),
        fetched_at: now,
        via: "import".into(),
    })
    .await?;
    let projected = project::project(repo, &[("tba".into(), path)]).await;

    let keys: Vec<String> = parsed
        .matches
        .iter()
        .map(|m| format!("{}_qm{}", event.key, m.number))
        .collect();
    let roster: Vec<i32> = parsed.roster.iter().map(|t| t.number).collect();
    let tidied = state.repo.tidy_schedule(&event.key, &keys, &roster).await?;

    let plural = |n: u64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut report = vec![format!(
        "Imported {} and {} for {}.",
        plural(projected.report.matches as u64, "match", "matches"),
        plural(parsed.roster.len() as u64, "team", "teams"),
        event.name
    )];
    if parsed.roster_from_schedule {
        report.push("No roster was given, so the roster is the schedule's teams.".into());
    }
    if tidied.removed_matches > 0 {
        report.push(format!(
            "Removed {} no longer on the schedule.",
            plural(tidied.removed_matches, "match", "matches")
        ));
    }
    for (key, observed) in &tidied.kept_matches {
        let label = match repo.match_by_key(key).await? {
            Some(m) => m.label(),
            None => key.clone(),
        };
        report.push(format!(
            "Kept {label}: it is not on the new schedule, but it has {}.",
            plural(*observed as u64, "observation", "observations")
        ));
    }
    if tidied.dropped_assignments > 0 {
        report.push(format!(
            "Dropped {} to robots no longer in that match.",
            plural(tidied.dropped_assignments, "assignment", "assignments")
        ));
    }
    if tidied.unlinked_teams > 0 {
        report.push(format!(
            "Took {} off the event.",
            plural(tidied.unlinked_teams, "team", "teams")
        ));
    }
    report.extend(projected.report.problems);
    Ok(report)
}
