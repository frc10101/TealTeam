//! The lead scout's assignment grid (L1), the changes made from it (L2), and
//! the coverage it shows (L6).
//!
//! Every match at the selected event, its six robots, and who is watching each
//! one. A lead scout assigns a match at a time, hands the open robots out
//! automatically, clears a match or the event, and names tablets.
//!
//! Each change is a form post answered with a 303 back to the grid, carrying a
//! `done=` note for the confirmation. A refused change re-renders the page with
//! the reason, and for a match edit, with what was picked.

use std::collections::HashMap;
use std::convert::Infallible;

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use tracing::{info, warn};
use tt_core::assignments::{self, AssigneeKey};
use tt_core::connectivity::{describe_age, describe_offset};
use tt_core::records::MatchRecord;
use tt_core::user::User;
use tt_repo::{DEVICE_ONLINE_WINDOW, Device, NewAssignment, Repo, Scout};
use tt_templates::{
    AssigneeChoice, AssignmentGrid, AssignmentsPage, DeviceRow, MatchEditor, Nav, PoolEntry,
    TallyRow, assignments_href,
};

use crate::events::EventContext;
use crate::startup::AppState;

/// Longest tablet name kept. Enough for "Stands Left, second row".
const DEVICE_NAME_MAX: usize = 60;

/// `?edit=`, and the `done=` note a change leaves behind.
///
/// Never rejects: a mangled link lands on the grid.
#[derive(Debug, Default)]
pub struct GridParams {
    /// The match whose editor is open.
    pub edit: Option<String>,
    pub done: Option<Done>,
}

/// What the change that led here did.
///
/// Read from the URL, so it is only a message: a crafted link can make the
/// page say "Cleared 40 assignments" to the person who crafted it, and change
/// nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Done {
    Saved { match_key: String },
    ClearedMatch { match_key: String },
    Distributed { count: u64 },
    Cleared { count: u64 },
    Renamed,
}

impl Done {
    fn query(&self) -> String {
        match self {
            Self::Saved { match_key } => format!("done=saved&match={match_key}"),
            Self::ClearedMatch { match_key } => format!("done=cleared-match&match={match_key}"),
            Self::Distributed { count } => format!("done=auto&n={count}"),
            Self::Cleared { count } => format!("done=cleared&n={count}"),
            Self::Renamed => "done=renamed".into(),
        }
    }

    fn message(&self, matches: &[MatchRecord]) -> String {
        let label = |key: &str| {
            matches
                .iter()
                .find(|m| m.key == key)
                .map(MatchRecord::label)
                .unwrap_or_else(|| "the match".into())
        };
        let robots = |n: u64| if n == 1 { "robot" } else { "robots" };
        match self {
            Self::Saved { match_key } => format!("Saved {}.", label(match_key)),
            Self::ClearedMatch { match_key } => format!("Cleared {}.", label(match_key)),
            Self::Distributed { count: 0 } => {
                "Nothing to hand out: every robot in those matches already has somebody, \
                 or there were more robots than people for each match."
                    .into()
            }
            Self::Distributed { count } => format!("Handed out {count} {}.", robots(*count)),
            Self::Cleared { count } => format!(
                "Cleared {count} {}.",
                if *count == 1 {
                    "assignment"
                } else {
                    "assignments"
                }
            ),
            Self::Renamed => "Tablet renamed.".into(),
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for GridParams {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Infallible> {
        let query = Query::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .map(|Query(query)| query)
            .unwrap_or_default();
        let get = |name: &str| {
            query
                .get(name)
                .map(|v| v.trim().to_ascii_lowercase())
                .filter(|v| !v.is_empty())
        };
        let count = || get("n").and_then(|n| n.parse().ok()).unwrap_or(0);
        let done = match get("done").as_deref() {
            Some("saved") => get("match").map(|match_key| Done::Saved { match_key }),
            Some("cleared-match") => get("match").map(|match_key| Done::ClearedMatch { match_key }),
            Some("auto") => Some(Done::Distributed { count: count() }),
            Some("cleared") => Some(Done::Cleared { count: count() }),
            Some("renamed") => Some(Done::Renamed),
            _ => None,
        };
        Ok(Self {
            edit: get("edit"),
            done,
        })
    }
}

/// Where a change sends the lead scout: the grid, with its note, scrolled to
/// what changed.
fn back_to_grid(event_key: &str, edit: Option<&str>, done: &Done, anchor: &str) -> String {
    let edit = edit.map(|key| format!("&edit={key}")).unwrap_or_default();
    format!(
        "/lead-scout/assignments?event={event_key}{edit}&{}#{anchor}",
        done.query()
    )
}

/// A match edit that was refused, with what was picked so it can be fixed.
pub struct Draft {
    pub match_key: String,
    pub chosen: Vec<(i32, Option<AssigneeKey>)>,
}

/// Everyone and everything that can be handed a robot.
struct People {
    scouts: Vec<Scout>,
    devices: Vec<Device>,
}

impl People {
    async fn load(state: &AppState) -> tt_repo::Result<Self> {
        Ok(Self {
            scouts: state.repo.list_scouts().await?,
            devices: state.repo.list_devices().await?,
        })
    }

    fn knows(&self, key: AssigneeKey) -> bool {
        match key {
            AssigneeKey::Scout(id) => self.scouts.iter().any(|s| s.id == id),
            AssigneeKey::Device(id) => self.devices.iter().any(|d| d.id == id),
        }
    }

    fn is_online(&self, key: AssigneeKey, now: DateTime<Utc>) -> bool {
        match key {
            AssigneeKey::Scout(id) => self
                .scouts
                .iter()
                .any(|s| s.id == id && s.is_online(now, DEVICE_ONLINE_WINDOW)),
            AssigneeKey::Device(id) => self
                .devices
                .iter()
                .any(|d| d.id == id && d.is_online(now, DEVICE_ONLINE_WINDOW)),
        }
    }

    fn name_of(&self, key: AssigneeKey) -> String {
        match key {
            AssigneeKey::Scout(id) => self
                .scouts
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.name.clone()),
            AssigneeKey::Device(id) => self
                .devices
                .iter()
                .find(|d| d.id == id)
                .map(Device::display_name),
        }
        .unwrap_or_else(|| "Someone".into())
    }

    /// For the editor's selects: scouts, then tablets, online ones marked.
    fn choices(&self, now: DateTime<Utc>) -> Vec<AssigneeChoice> {
        let online = |yes: bool| if yes { " · online" } else { "" };
        self.scouts
            .iter()
            .map(|s| AssigneeChoice {
                key: AssigneeKey::Scout(s.id),
                label: format!(
                    "{}{}",
                    s.name,
                    online(s.is_online(now, DEVICE_ONLINE_WINDOW))
                ),
            })
            .chain(self.devices.iter().map(|d| AssigneeChoice {
                key: AssigneeKey::Device(d.id),
                label: format!(
                    "{} (tablet){}",
                    d.display_name(),
                    online(d.is_online(now, DEVICE_ONLINE_WINDOW))
                ),
            }))
            .collect()
    }

    /// For auto-distribute. Scouts online now start ticked; tablets never do,
    /// because a scout and the tablet in their hands are one person, and
    /// ticking both would hand that person twice the robots.
    fn pool(&self, now: DateTime<Utc>) -> Vec<PoolEntry> {
        self.scouts
            .iter()
            .map(|s| {
                let online = s.is_online(now, DEVICE_ONLINE_WINDOW);
                PoolEntry {
                    value: AssigneeKey::Scout(s.id).to_string(),
                    label: s.name.clone(),
                    by_device: false,
                    online,
                    checked: online,
                }
            })
            .chain(self.devices.iter().map(|d| PoolEntry {
                value: AssigneeKey::Device(d.id).to_string(),
                label: d.display_name(),
                by_device: true,
                online: d.is_online(now, DEVICE_ONLINE_WINDOW),
                checked: false,
            }))
            .collect()
    }

    fn device_rows(&self, now: DateTime<Utc>) -> Vec<DeviceRow> {
        self.devices
            .iter()
            .map(|d| DeviceRow {
                id: d.id,
                name: d.name.clone().unwrap_or_default(),
                display: d.display_name(),
                online: d.is_online(now, DEVICE_ONLINE_WINDOW),
                last_seen: d
                    .last_seen_at
                    .map(|seen| describe_age(now - seen))
                    .unwrap_or_else(|| "never".into()),
                last_user: d
                    .last_user_id
                    .and_then(|id| self.scouts.iter().find(|s| s.id == id))
                    .map(|s| s.name.clone())
                    .unwrap_or_default(),
                clock: d
                    .clock_offset_ms
                    .map(|ms| describe_offset(ms).0)
                    .unwrap_or_default(),
                clock_off: d.clock_offset_ms.is_some_and(|ms| describe_offset(ms).1),
            })
            .collect()
    }
}

/// Assemble the grid for `context`'s event.
///
/// A failed read of the schedule or the assignments replaces the grid with a
/// message: a grid reading "Unassigned" everywhere because a query failed would
/// send a lead scout off to redo work that is already done. A failed roster
/// read only costs the team names; a failed read of scouts and tablets, the
/// tools that need them.
pub async fn page(
    state: &AppState,
    nav: Nav,
    context: &EventContext,
    params: &GridParams,
    draft: Option<Draft>,
    errors: Vec<String>,
) -> AssignmentsPage {
    let storage_ready = nav.storage_ready;
    let mut page = AssignmentsPage {
        title: "Assignments".into(),
        nav,
        event_name: String::new(),
        unavailable: String::new(),
        errors,
        notice: String::new(),
        grid: None,
        editor: None,
        pool: Vec::new(),
        devices: Vec::new(),
        coverage: Vec::new(),
        live_href: String::new(),
        stream_href: String::new(),
    };

    let Some(event) = &context.selected else {
        page.unavailable = if storage_ready {
            "No events have been loaded yet, so there is nothing to assign.".into()
        } else {
            "The server's storage is unavailable, so assignments cannot be shown.".into()
        };
        return page;
    };
    page.event_name = event.name.clone();
    if let Some(unknown) = &context.unknown {
        page.errors.push(format!(
            "There is no event “{unknown}” on this server. Showing {} instead.",
            event.name
        ));
    }

    let matches = match state.repo.event_matches(&event.key).await {
        Ok(matches) => matches,
        Err(e) => {
            warn!("matches for {}: {e}", event.key);
            page.unavailable = "Could not read the match schedule. Reload to try again.".into();
            return page;
        }
    };
    if matches.is_empty() {
        page.unavailable = format!(
            "{} has no match schedule yet. Assignments open once it is published.",
            event.name
        );
        return page;
    }

    // Sightings too: without them every played robot would read "Missed".
    let loaded = async {
        let assignments = state.repo.event_assignments(&event.key).await?;
        let sightings = state.repo.event_sightings(&event.key).await?;
        tt_repo::Result::Ok((assignments, sightings))
    };
    let (assignments, sightings) = match loaded.await {
        Ok(loaded) => loaded,
        Err(e) => {
            warn!("assignments for {}: {e}", event.key);
            page.unavailable = "Could not read the assignments. Reload to try again.".into();
            return page;
        }
    };
    let roster = state
        .repo
        .event_teams(&event.key)
        .await
        .unwrap_or_else(|e| {
            warn!("roster for {}: {e}", event.key);
            Vec::new()
        });
    let now = Utc::now();
    let people = People::load(state).await.unwrap_or_else(|e| {
        warn!("listing scouts and tablets: {e}");
        People {
            scouts: Vec::new(),
            devices: Vec::new(),
        }
    });

    if let Some(done) = &params.done {
        page.notice = done.message(&matches);
    }

    // A refused edit reopens the match it was for, whatever the URL says.
    let editing = draft
        .as_ref()
        .map(|d| d.match_key.as_str())
        .or(params.edit.as_deref());
    if let Some(key) = editing {
        match matches.iter().position(|m| m.key == key) {
            Some(i) => {
                page.editor = Some(MatchEditor::new(
                    &event.key,
                    &matches[i],
                    matches.get(i + 1),
                    &roster,
                    &assignments,
                    &people.choices(now),
                    draft.as_ref().map(|d| d.chosen.as_slice()),
                ));
            }
            None => page
                .errors
                .push(format!("There is no match “{key}” at {}.", event.name)),
        }
    }

    page.coverage = assignments::tallies(&matches, &assignments, &sightings)
        .into_iter()
        .map(|t| TallyRow {
            by_device: matches!(t.assignee, AssigneeKey::Device(_)),
            online: people.is_online(t.assignee, now),
            name: t.name,
            recorded: t.recorded,
            missed: t.missed,
            to_come: t.to_come,
        })
        .collect();
    page.pool = people.pool(now);
    page.devices = people.device_rows(now);
    page.live_href = assignments_href(&event.key, None);
    page.stream_href = crate::sync::stream_href(state).await.unwrap_or_default();
    page.grid = Some(AssignmentGrid::new(
        &event.key,
        &matches,
        &roster,
        &assignments,
        &sightings,
    ));
    page
}

// ── Changes (L2) ────────────────────────────────────────────────────────────

/// A change that did not happen, and what to show instead.
pub struct Refused {
    /// The event to show, once known.
    pub event_key: Option<String>,
    pub draft: Option<Draft>,
    pub errors: Vec<String>,
}

impl Refused {
    fn because(event_key: Option<&str>, message: impl Into<String>) -> Box<Self> {
        Box::new(Self {
            event_key: event_key.map(str::to_string),
            draft: None,
            errors: vec![message.into()],
        })
    }
}

const NOT_SAVED: &str = "Not saved: the server's storage did not answer. Try again.";

type Outcome = Result<String, Box<Refused>>;

fn posted<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .rev()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
}

async fn load_match(state: &AppState, key: Option<&str>) -> Result<MatchRecord, Box<Refused>> {
    let key = key.map(str::to_ascii_lowercase);
    match key {
        Some(key) => match state.repo.match_by_key(&key).await {
            Ok(Some(record)) => Ok(record),
            Ok(None) => Err(Refused::because(None, "That match is not on the schedule.")),
            Err(e) => {
                warn!("loading match {key}: {e}");
                Err(Refused::because(None, NOT_SAVED))
            }
        },
        None => Err(Refused::because(None, "No match was named.")),
    }
}

/// `POST /api/assignments/match`: set all six robots of one match.
///
/// Fields are `a.<team number>` = `u:<id>`, `d:<id>`, or blank for nobody.
/// A robot the post does not mention keeps its assignment; one not in the
/// match is ignored, so a form older than a schedule change cannot assign a
/// robot that has left -- and any assignment to a robot that has left is
/// removed.
pub async fn save_match(state: &AppState, user: &User, pairs: &[(String, String)]) -> Outcome {
    let record = load_match(state, posted(pairs, "match")).await?;
    let event = record.event_key.as_str();
    let people = People::load(state).await.map_err(|e| {
        warn!("listing scouts and tablets: {e}");
        Refused::because(Some(event), NOT_SAVED)
    })?;

    let mut chosen = Vec::new();
    let mut errors = Vec::new();
    for team in record.teams() {
        let Some((_, raw)) = pairs.iter().rev().find(|(k, _)| *k == format!("a.{team}")) else {
            continue;
        };
        let key = AssigneeKey::parse(raw);
        match key {
            Some(key) if !people.knows(key) => {
                errors.push(format!(
                    "Team {team}: that scout or tablet no longer exists."
                ));
                chosen.push((team, None));
            }
            None if !raw.trim().is_empty() => {
                errors.push(format!("Team {team}: that is not a scout or a tablet."));
                chosen.push((team, None));
            }
            _ => chosen.push((team, key)),
        }
    }
    let picked: Vec<AssigneeKey> = chosen.iter().filter_map(|c| c.1).collect();
    for key in assignments::doubled(&picked) {
        errors.push(format!(
            "{} is down for two robots in {}. Nobody can watch two at once.",
            people.name_of(key),
            record.label()
        ));
    }
    if !errors.is_empty() {
        return Err(Box::new(Refused {
            event_key: Some(event.to_string()),
            draft: Some(Draft {
                match_key: record.key.clone(),
                chosen,
            }),
            errors,
        }));
    }

    let now = Utc::now();
    let set: Vec<NewAssignment> = chosen
        .iter()
        .filter_map(|&(team, key)| {
            Some(NewAssignment {
                match_key: record.key.clone(),
                event_key: event.to_string(),
                team_number: team,
                assignee: key?,
            })
        })
        .collect();
    let stored = async {
        state.repo.set_assignments(&set, user.id, now).await?;
        for &(team, _) in chosen.iter().filter(|c| c.1.is_none()) {
            state.repo.unassign(&record.key, team).await?;
        }
        // Saving a match leaves it holding what the form showed, so an
        // assignment to a robot the schedule has since moved out goes too.
        for stale in state
            .repo
            .event_assignments(event)
            .await?
            .iter()
            .filter(|a| a.match_key == record.key && record.alliance_of(a.team_number).is_none())
        {
            state.repo.unassign(&record.key, stale.team_number).await?;
        }
        tt_repo::Result::Ok(())
    };
    if let Err(e) = stored.await {
        warn!("saving assignments for {}: {e}", record.key);
        return Err(Box::new(Refused {
            event_key: Some(event.to_string()),
            draft: Some(Draft {
                match_key: record.key.clone(),
                chosen,
            }),
            errors: vec![NOT_SAVED.into()],
        }));
    }
    info!(user = %user.email, r#match = %record.key, "assignments saved");

    let done = Done::Saved {
        match_key: record.key.clone(),
    };
    let next = match posted(pairs, "then") {
        Some("next") => next_match(state, &record).await,
        _ => None,
    };
    Ok(match next {
        Some(next) => back_to_grid(event, Some(&next), &done, "edit"),
        None => back_to_grid(event, None, &done, &record.key),
    })
}

async fn next_match(state: &AppState, record: &MatchRecord) -> Option<String> {
    let matches = state.repo.event_matches(&record.event_key).await.ok()?;
    let i = matches.iter().position(|m| m.key == record.key)?;
    matches.get(i + 1).map(|m| m.key.clone())
}

/// `POST /api/assignments/clear-match`.
pub async fn clear_match(state: &AppState, user: &User, pairs: &[(String, String)]) -> Outcome {
    let record = load_match(state, posted(pairs, "match")).await?;
    let event = record.event_key.as_str();
    match state.repo.clear_assignments(event, Some(&record.key)).await {
        Ok(count) => {
            info!(user = %user.email, r#match = %record.key, count, "match assignments cleared");
            let done = Done::ClearedMatch {
                match_key: record.key.clone(),
            };
            Ok(back_to_grid(event, None, &done, &record.key))
        }
        Err(e) => {
            warn!("clearing {}: {e}", record.key);
            Err(Refused::because(Some(event), NOT_SAVED))
        }
    }
}

async fn known_event(state: &AppState, key: Option<&str>) -> Result<String, Box<Refused>> {
    let Some(key) = key else {
        return Err(Refused::because(None, "No event was named."));
    };
    match state.repo.event(key).await {
        Ok(Some(event)) => Ok(event.key),
        Ok(None) => Err(Refused::because(
            None,
            format!("There is no event “{key}”."),
        )),
        Err(e) => {
            warn!("loading event {key}: {e}");
            Err(Refused::because(None, NOT_SAVED))
        }
    }
}

/// `POST /api/assignments/clear?event=`: every assignment at the event.
///
/// Needs `confirm=yes`, a checkbox, so the button alone cannot do it -- a
/// confirmation that works without JavaScript.
pub async fn clear_all(
    state: &AppState,
    user: &User,
    event_key: Option<&str>,
    pairs: &[(String, String)],
) -> Outcome {
    let event = known_event(state, event_key).await?;
    if posted(pairs, "confirm") != Some("yes") {
        return Err(Refused::because(
            Some(&event),
            "Nothing was cleared. Tick the box to confirm first.",
        ));
    }
    match state.repo.clear_assignments(&event, None).await {
        Ok(count) => {
            info!(user = %user.email, %event, count, "all assignments cleared");
            Ok(back_to_grid(&event, None, &Done::Cleared { count }, "main"))
        }
        Err(e) => {
            warn!("clearing assignments at {event}: {e}");
            Err(Refused::because(Some(&event), NOT_SAVED))
        }
    }
}

/// `POST /api/assignments/auto?event=`: hand every open robot in the next
/// `matches` unplayed matches (all of them when blank) to the ticked
/// `assignee`s, per [`assignments::distribute`].
pub async fn distribute(
    state: &AppState,
    user: &User,
    event_key: Option<&str>,
    pairs: &[(String, String)],
) -> Outcome {
    let event = known_event(state, event_key).await?;
    let refuse = |message: &str| Refused::because(Some(&event), message);

    let limit = match posted(pairs, "matches") {
        None => None,
        Some(raw) => match raw.parse::<usize>() {
            Ok(n) if n > 0 => Some(n),
            _ => {
                return Err(refuse(
                    "Nothing was handed out: the number of matches must be a whole number above zero.",
                ));
            }
        },
    };

    let people = People::load(state).await.map_err(|e| {
        warn!("listing scouts and tablets: {e}");
        refuse(NOT_SAVED)
    })?;
    let mut pool: Vec<AssigneeKey> = Vec::new();
    for key in pairs
        .iter()
        .filter(|(k, _)| k == "assignee")
        .filter_map(|(_, v)| AssigneeKey::parse(v))
        .filter(|key| people.knows(*key))
    {
        if !pool.contains(&key) {
            pool.push(key);
        }
    }
    if pool.is_empty() {
        return Err(refuse(
            "Nothing was handed out: tick at least one scout or tablet.",
        ));
    }

    let loaded = async {
        let matches = state.repo.event_matches(&event).await?;
        let existing = state.repo.event_assignments(&event).await?;
        tt_repo::Result::Ok((matches, existing))
    };
    let (matches, existing) = loaded.await.map_err(|e| {
        warn!("loading {event} for auto-distribute: {e}");
        refuse(NOT_SAVED)
    })?;
    let upcoming: Vec<MatchRecord> = matches
        .into_iter()
        .filter(|m| !m.played)
        .take(limit.unwrap_or(usize::MAX))
        .collect();

    let picks = assignments::distribute(&upcoming, &existing, &pool);
    let set: Vec<NewAssignment> = picks
        .into_iter()
        .map(|p| NewAssignment {
            match_key: p.match_key,
            event_key: event.clone(),
            team_number: p.team_number,
            assignee: p.assignee,
        })
        .collect();
    if let Err(e) = state.repo.set_assignments(&set, user.id, Utc::now()).await {
        warn!("auto-distributing at {event}: {e}");
        return Err(refuse(NOT_SAVED));
    }
    info!(user = %user.email, %event, count = set.len(), people = pool.len(), "auto-distributed");

    let count = set.len() as u64;
    Ok(back_to_grid(
        &event,
        None,
        &Done::Distributed { count },
        "upcoming",
    ))
}

/// `POST /api/devices/{id}/rename?event=`. A blank name clears it, and the
/// tablet goes back to reading `Device 0191f7ac`.
pub async fn rename_device(
    state: &AppState,
    user: &User,
    event_key: Option<&str>,
    id: i64,
    pairs: &[(String, String)],
) -> Outcome {
    let event = known_event(state, event_key).await?;
    let name: String = posted(pairs, "name")
        .unwrap_or_default()
        .chars()
        .take(DEVICE_NAME_MAX)
        .collect();
    match state.repo.rename_device(id, &name, Utc::now()).await {
        Ok(()) => {
            info!(user = %user.email, device = id, %name, "tablet renamed");
            Ok(back_to_grid(&event, None, &Done::Renamed, "tablets"))
        }
        Err(e) => {
            warn!("renaming device {id}: {e}");
            Err(Refused::because(Some(&event), NOT_SAVED))
        }
    }
}
