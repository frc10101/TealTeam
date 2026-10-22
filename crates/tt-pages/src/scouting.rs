//! The scouting page and the reading of its save (U4), on both sides (OF1).
//!
//! A scout picks a match, then one of its robots, then fills in a form drawn
//! from the season schema. Every step is in the URL -- `?event=&match=&team=` --
//! so a reload, a bookmark, or a second tab lands on the same step.
//!
//! The match and robot are always re-checked against the schedule on save. The
//! alliance is read off the match rather than asked, which removes one more
//! thing a scout could get wrong.
//!
//! `tt-web` serves this over the Pi's database. With no server, the service
//! worker makes it over the device's copy (C5), and a save goes into the
//! device's outbox (C7) instead of the Pi's tables: [`read_post`] is the
//! same check on both sides, and each side records what it returns its own
//! way. The device says so: [`Saved::Held`] words the confirmation for a
//! save that has not reached the server yet.

use tracing::warn;
use tt_core::assignments::{self, AssigneeKey};
use tt_core::form::{FormErrors, RawAnswers, read_answers};
use tt_core::record_id;
use tt_core::records::MatchRecord;
use tt_core::season::SeasonSchema;
use tt_core::user::User;
use tt_repo::{LocalRepo, NewObservation};
use tt_templates::{
    AssignedCard, DeclinedNotice, Draft, DutyRow, Keypad, MatchLink, MatchPicker, Nav, RosterEntry,
    ScoutForm, SubmissionPage, choose_href, draft_key, scout_href,
};

use crate::events::EventContext;

const STORAGE_DOWN: &str =
    "The server's storage is unavailable, so nothing can be recorded right now.";

/// `?match=`, `&team=`, `&choose=`, and `&saved=`: which step of the page to
/// show.
///
/// Never rejects, like [`crate::events::requested`]: a mangled link should
/// land on a working page that says what was wrong.
#[derive(Debug, Default, Clone)]
pub struct ScoutParams {
    pub match_key: Option<String>,
    pub team: Option<i32>,
    /// Show the robot picker even where the scout has an assignment: the
    /// deliberate override (L5).
    pub choose: bool,
    /// The robot the scout just saved, for the confirmation.
    pub saved: Option<i32>,
}

impl ScoutParams {
    /// From a query, `lookup` giving each name's value.
    pub fn from_lookup<'a>(lookup: impl Fn(&str) -> Option<&'a str>) -> Self {
        Self {
            match_key: match_key(lookup("match")),
            team: team_number(lookup("team")),
            choose: lookup("choose").is_some_and(|v| !v.trim().is_empty()),
            saved: team_number(lookup("saved")),
        }
    }
}

fn match_key(raw: Option<&str>) -> Option<String> {
    raw.map(|k| k.trim().to_ascii_lowercase())
        .filter(|k| !k.is_empty())
}

fn team_number(raw: Option<&str>) -> Option<i32> {
    raw.and_then(|t| t.trim().parse().ok()).filter(|t| *t > 0)
}

/// How a save the URL names is confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saved {
    /// In the server's tables, waiting for review.
    OnServer,
    /// In this device's outbox (C7), sent when the server answers (OF1).
    Held,
}

/// What only the server knows, for [`page`]. The device has none of it.
#[derive(Default)]
pub struct ServerOnly {
    /// The scout's declined records (L10), from [`declined_notices`].
    pub declined: Vec<DeclinedNotice>,
    /// S9's live stream, to say when the lead moves this scout's robot.
    pub stream: Option<String>,
}

/// Assemble the scouting page for `context`'s event.
///
/// With no match or robot named, a scout with an assignment (L4) lands on it:
/// "You are scouting 1678", form open, no picker (L5). An assignment is
/// theirs if it names their account or the tablet they are on (`device_id`).
///
/// `draft` is a form coming back after a failed save; `errors` are messages
/// from that save. Storage failures degrade to a page that says so; a failed
/// read of assignments only costs the shortcut. `fresh_id` makes the record
/// id a new form is shown with (D7).
#[allow(clippy::too_many_arguments)]
pub async fn page<R: LocalRepo>(
    repo: &R,
    season: &SeasonSchema,
    user: &User,
    device_id: Option<i64>,
    nav: Nav,
    context: &EventContext,
    params: &ScoutParams,
    draft: Option<Draft>,
    mut errors: Vec<String>,
    saved_as: Saved,
    server: ServerOnly,
    fresh_id: impl FnOnce() -> String,
) -> SubmissionPage {
    let storage_ready = nav.storage_ready;
    let mut page = SubmissionPage {
        title: "Scout".into(),
        nav,
        unavailable: String::new(),
        picker: None,
        saved: String::new(),
        saved_draft: String::new(),
        watch: String::new(),
        errors: Vec::new(),
        notice: String::new(),
        form: None,
        assigned: None,
        off_assignment: String::new(),
        next_duty: None,
        missed: Vec::new(),
        upcoming: Vec::new(),
        agenda: String::new(),
        keypad: None,
        declined: Vec::new(),
    };

    let Some(event) = &context.selected else {
        page.unavailable = if storage_ready {
            "No events have been loaded yet, so there is nothing to scout.".into()
        } else {
            STORAGE_DOWN.into()
        };
        page.errors = errors;
        return page;
    };
    if let Some(unknown) = &context.unknown {
        errors.push(format!(
            "There is no event “{unknown}” on this server. Showing {} instead.",
            event.name
        ));
    }

    let matches = repo.event_matches(&event.key).await.unwrap_or_else(|e| {
        warn!("matches for {}: {e}", event.key);
        Vec::new()
    });
    if matches.is_empty() {
        page.unavailable = format!(
            "{} has no match schedule yet. Scouting opens once it is published.",
            event.name
        );
        page.errors = errors;
        return page;
    }

    page.declined = server.declined;

    let recorded_here = repo
        .recorded_by(&event.key, user.id)
        .await
        .unwrap_or_else(|e| {
            warn!("robots recorded by {} at {}: {e}", user.id, event.key);
            Vec::new()
        });
    let event_assignments = repo
        .event_assignments(&event.key)
        .await
        .unwrap_or_else(|e| {
            warn!("assignments for {}: {e}", event.key);
            Vec::new()
        });
    let mut me = vec![AssigneeKey::Scout(user.id)];
    me.extend(device_id.map(AssigneeKey::Device));
    let agenda = assignments::agenda(&matches, &event_assignments, &me, &recorded_here);
    // Storage only, even when empty: the retired page asked FIRST here
    // (REBUILD_SPEC.md 12.7). The robots come from the match; the roster only
    // adds names and the keypad's type-ahead.
    let roster = repo.event_teams(&event.key).await.unwrap_or_else(|e| {
        warn!("roster for {}: {e}", event.key);
        Vec::new()
    });
    let index_of = |key: &str| matches.iter().position(|m| m.key == key);

    // Which match, which robot, and whether the robot came from an assignment.
    let mut following = false;
    let (index, team) = match (&params.match_key, params.team) {
        (Some(key), team) => {
            let index = index_of(key).or_else(|| {
                errors.push(format!("There is no match “{key}” at {}.", event.name));
                None
            });
            match index {
                Some(i) => {
                    let assigned = (!params.choose && team.is_none())
                        .then(|| agenda.open_in(key))
                        .flatten();
                    following = assigned.is_some();
                    (i, team.or(assigned))
                }
                None => (next_to_scout(&matches), None),
            }
        }
        // The keypad (L3): a team, no match. Its next unplayed match, or its
        // last one once they are all played.
        (None, Some(team)) => {
            let theirs = |m: &&MatchRecord| m.alliance_of(team).is_some();
            let found = matches
                .iter()
                .filter(theirs)
                .find(|m| !m.played)
                .or_else(|| matches.iter().rfind(|m| m.alliance_of(team).is_some()));
            match found {
                Some(m) => (index_of(&m.key).expect("from this list"), Some(team)),
                None => {
                    errors.push(format!(
                        "Team {team} is not on the schedule at {}.",
                        event.name
                    ));
                    (next_to_scout(&matches), None)
                }
            }
        }
        (None, None) => match (&agenda.next, params.choose) {
            (Some((key, team)), false) => {
                following = true;
                (index_of(key).expect("from this schedule"), Some(*team))
            }
            _ => (next_to_scout(&matches), None),
        },
    };
    let record = &matches[index];
    let label = record.label();

    let recorded: Vec<i32> = recorded_here
        .iter()
        .filter(|(key, _)| *key == record.key)
        .map(|(_, team)| *team)
        .collect();

    let team = team.filter(|&team| {
        let in_match = record.alliance_of(team).is_some();
        if !in_match {
            errors.push(format!("Team {team} is not in {label}."));
        }
        in_match
    });

    // Only confirm a save that is really there, not whatever the URL claims.
    if let Some(saved) = params.saved.filter(|t| recorded.contains(t)) {
        page.saved = match saved_as {
            Saved::OnServer => {
                format!("Saved team {saved} in {label}. It is waiting for the lead scout's review.")
            }
            Saved::Held => format!(
                "Saved team {saved} in {label} on this tablet. The server can't be reached, \
                 so it is held here and sent as soon as the server answers."
            ),
        };
        page.saved_draft = draft_key(user.id, &event.key, &record.key, saved, season.version);
    }

    let duty_link = |(key, team): &(String, i32)| {
        let label = matches
            .iter()
            .find(|m| m.key == *key)
            .map(MatchRecord::label)
            .unwrap_or_default();
        MatchLink {
            href: scout_href(&event.key, key, Some(*team)),
            label: format!("team {team} in {label}"),
        }
    };
    page.missed = agenda.missed.iter().map(duty_link).collect();
    // Pointing at the next assignment is noise while the page shows it.
    page.next_duty = agenda
        .next
        .as_ref()
        .filter(|(key, t)| !(*key == record.key && team == Some(*t)))
        .map(duty_link);

    let assigned_here = agenda.open_in(&record.key);
    if let (Some(team), Some(mine)) = (team, assigned_here)
        && team != mine
    {
        page.off_assignment = format!(
            "You are assigned team {mine} in {label}. Make sure {team} is the robot you are watching."
        );
    }

    let team_name = |number: i32| {
        roster
            .iter()
            .find(|t| t.number == number)
            .map(|t| t.name.clone())
    };
    // C8: the whole list, on the page and kept on the device, so a scout
    // whose network drops still knows which robot is next.
    let zone = event.zone();
    let duty_row = |(key, duty_team): &(String, i32)| {
        let m = matches.iter().find(|m| m.key == *key)?;
        Some(DutyRow {
            label: m.label(),
            station: station(m, *duty_team),
            team: *duty_team,
            team_name: team_name(*duty_team).unwrap_or_default(),
            time: m
                .scheduled_at
                .map(|at| tt_core::timezone::clock_time(zone, at))
                .unwrap_or_default(),
            href: scout_href(&event.key, key, Some(*duty_team)),
            current: *key == record.key && team == Some(*duty_team),
        })
    };
    page.upcoming = agenda.upcoming.iter().filter_map(duty_row).collect();
    let kept = |rows: &[DutyRow]| -> Vec<serde_json::Value> {
        rows.iter()
            .map(|d| {
                serde_json::json!({
                    "label": d.label,
                    "station": d.station,
                    "team": d.team,
                    "name": d.team_name,
                    "time": d.time,
                })
            })
            .collect()
    };
    let missed_rows: Vec<DutyRow> = agenda.missed.iter().filter_map(duty_row).collect();
    page.agenda = serde_json::json!({
        "user": user.id,
        "scout": user.name,
        "device": device_id,
        "event": event.key,
        "event_name": event.name,
        "pks": agenda
            .upcoming
            .iter()
            .chain(&agenda.missed)
            .map(|(key, team)| format!("{key}:{team}"))
            .collect::<Vec<_>>(),
        "upcoming": kept(&page.upcoming),
        "missed": kept(&missed_rows),
    })
    .to_string();

    if following && let Some(team) = team {
        page.assigned = Some(AssignedCard {
            team,
            team_name: team_name(team).unwrap_or_default(),
            where_: format!("{label} · {}", station(record, team)),
            choose_href: choose_href(&event.key, &record.key),
        });
    } else {
        page.picker = Some(MatchPicker::new(
            &event.key,
            &matches,
            index,
            team,
            &recorded,
            assigned_here,
        ));
        page.keypad = Some(Keypad {
            event_key: event.key.clone(),
            roster: roster
                .iter()
                .map(|t| RosterEntry {
                    number: t.number,
                    name: t.name.clone(),
                    is_viewer: false,
                })
                .collect(),
        });
    }

    if let Some(team) = team {
        if recorded.contains(&team) {
            page.notice = format!("You have already recorded team {team} in {label}.");
        } else {
            let name = match team_name(team) {
                Some(name) => Some(name),
                None => match repo.team(team).await {
                    Ok(found) => found.map(|t| t.name),
                    Err(e) => {
                        warn!("loading team {team}: {e}");
                        None
                    }
                },
            };
            let mut form = ScoutForm::new(
                season,
                record,
                team,
                name.as_deref(),
                draft.unwrap_or_else(|| Draft::fresh(fresh_id())),
                user.team_number.is_none(),
            );
            form.draft_key = draft_key(user.id, &event.key, &record.key, team, season.version);
            page.form = Some(form);
        }
    }

    // S9: so the page can say when the lead moves one of this scout's robots.
    if let Some(stream) = server.stream {
        let labels: std::collections::BTreeMap<&str, String> = matches
            .iter()
            .map(|m| (m.key.as_str(), m.label()))
            .collect();
        page.watch = serde_json::json!({
            "stream": stream,
            "user": user.id,
            "device": device_id,
            "current": team.filter(|_| following).map(|t| format!("{}:{t}", record.key)),
            "labels": labels,
            "href": format!("/submission?event={}", event.key),
        })
        .to_string();
    }

    page.errors = errors;
    page
}

/// `"Red 2"`: where `team` stands in `record`, or empty when it is not in it.
fn station(record: &MatchRecord, team: i32) -> String {
    [("Red", &record.red), ("Blue", &record.blue)]
        .into_iter()
        .find_map(|(side, slots)| {
            let i = slots.iter().position(|s| *s == Some(team))?;
            Some(format!("{side} {}", i + 1))
        })
        .unwrap_or_default()
}

/// The match a scout most likely wants: the first not yet played, or the last
/// one once they all have been.
fn next_to_scout(matches: &[MatchRecord]) -> usize {
    matches
        .iter()
        .position(|m| !m.played)
        .unwrap_or(matches.len().saturating_sub(1))
}

/// What the scouting page tells a scout about their declined records (L10).
pub async fn declined_notices<R: LocalRepo>(
    repo: &R,
    user: &User,
    context: &EventContext,
    matches: &[MatchRecord],
) -> Vec<DeclinedNotice> {
    let Some(event) = &context.selected else {
        return Vec::new();
    };
    let declined = repo
        .declined_for(&event.key, user.id)
        .await
        .unwrap_or_else(|e| {
            warn!("declined observations for {}: {e}", user.id);
            Vec::new()
        });
    declined
        .iter()
        .map(|o| {
            let label = matches
                .iter()
                .find(|m| m.key == o.match_key)
                .map(MatchRecord::label)
                .unwrap_or_else(|| o.match_key.clone());
            DeclinedNotice {
                what: format!("team {} in {label}", o.team_number),
                reason: o.review_note.clone().unwrap_or_default(),
                reviewer: o
                    .reviewer_name
                    .clone()
                    .unwrap_or_else(|| "The lead scout".into()),
                href: scout_href(&o.event_key, &o.match_key, Some(o.team_number)),
            }
        })
        .collect()
}

/// A save that did not happen, and what to show instead.
pub struct Rejected {
    /// The event to show: the match's, once the match is known.
    pub event_key: Option<String>,
    pub params: ScoutParams,
    /// The form to show again, answers intact. `None` when there is no form to
    /// go back to.
    pub draft: Option<Draft>,
    pub errors: Vec<String>,
}

/// What a save that broke the one-per-robot rule shows, on either side.
pub const ONE_PER_ROBOT: &str = "Not saved: each scout keeps one observation per robot per match.";

/// A posted form, checked against the schedule and the season, as the
/// observation to record: the server records it in its tables, the device
/// queues it (OF1). The rejection is boxed only because it is large; it is
/// the rarer path.
///
/// `fresh_id` stands in for a form that somehow lost its id: it still saves,
/// and the per-scout coverage index stops a second copy, so a scout's work is
/// never refused over it.
#[allow(clippy::too_many_arguments)]
pub async fn read_post<R: LocalRepo>(
    repo: &R,
    season: &SeasonSchema,
    user: &User,
    device_id: Option<i64>,
    pairs: &[(String, String)],
    now: chrono::DateTime<chrono::Utc>,
    fresh_id: impl FnOnce() -> String,
) -> Result<NewObservation, Box<Rejected>> {
    let posted = |name: &str| {
        pairs
            .iter()
            .rev()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    let params = ScoutParams {
        match_key: match_key(posted("match")),
        team: team_number(posted("team")),
        choose: false,
        saved: None,
    };
    let record_id = posted("record_id")
        .and_then(record_id::normalize)
        .unwrap_or_else(fresh_id);
    let answers = RawAnswers::from_pairs(pairs);
    let draft = |errors: FormErrors| Draft {
        record_id: record_id.clone(),
        answers: answers.clone(),
        errors,
    };

    // The schedule decides which match, robot, and alliance this is -- never
    // the post alone.
    let found = match &params.match_key {
        Some(key) => repo.match_by_key(key).await,
        None => Ok(None),
    };
    let record = match found {
        Ok(Some(record)) => record,
        Ok(None) => {
            return Err(Box::new(Rejected {
                event_key: None,
                params,
                draft: None,
                errors: vec!["Not saved: that match is not on the schedule.".into()],
            }));
        }
        Err(e) => {
            warn!("loading match for a submission: {e}");
            return Err(Box::new(Rejected {
                event_key: None,
                params,
                draft: None,
                errors: vec![format!("Not saved. {STORAGE_DOWN}")],
            }));
        }
    };
    let event_key = Some(record.event_key.clone());

    let Some((team, alliance)) = params
        .team
        .and_then(|team| Some((team, record.alliance_of(team)?)))
    else {
        return Err(Box::new(Rejected {
            event_key,
            params,
            draft: None,
            errors: vec!["Not saved: pick the robot you watched from this match.".into()],
        }));
    };

    let payload = match read_answers(season, &answers) {
        Ok(payload) => payload,
        Err(errors) => {
            return Err(Box::new(Rejected {
                event_key,
                params,
                draft: Some(draft(errors)),
                errors: Vec::new(),
            }));
        }
    };

    Ok(NewObservation {
        client_record_id: record_id,
        match_key: record.key.clone(),
        event_key: record.event_key.clone(),
        team_number: team,
        alliance,
        payload,
        schema_version: season.version,
        scouter_id: Some(user.id),
        device_id,
        submitting_team: user.team_number,
        observed_at: now,
    })
}

/// Where a saved observation sends the scout: the same match, confirmed.
pub fn saved_href(observation: &NewObservation) -> String {
    format!(
        "/submission?event={}&match={}&saved={}",
        observation.event_key, observation.match_key, observation.team_number
    )
}

/// What a save that broke the one-per-robot rule shows: the match again,
/// with nothing to go back to, since that robot is already recorded.
pub fn one_per_robot(observation: &NewObservation) -> Box<Rejected> {
    Box::new(Rejected {
        event_key: Some(observation.event_key.clone()),
        params: ScoutParams {
            match_key: Some(observation.match_key.clone()),
            team: Some(observation.team_number),
            choose: false,
            saved: None,
        },
        draft: None,
        errors: vec![ONE_PER_ROBOT.into()],
    })
}
