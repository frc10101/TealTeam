//! Landing upstream data in the database (I4-I7).
//!
//! # Partial success is the normal case
//!
//! At an event, a sync runs over a tethered phone that comes and goes. Every
//! function here therefore reports what it managed rather than failing whole:
//! one event's teams failing to fetch must not abandon the other eleven, and a
//! missing component-OPR endpoint must not discard the rankings that came with
//! it. [`SyncReport`] carries the counts and the failures together.
//!
//! The one thing that does abort is a total absence of connectivity, because
//! then nothing after it will work either.

use chrono::{NaiveDate, Utc};
use tracing::{info, warn};
use tt_core::records::{Event, MatchRecord, Team, TeamEventStats};
use tt_core::upstream::{self, Phase};
use tt_repo::Repo;

use crate::first::{EventFilters, FirstClient};
use crate::tba::TbaClient;
use crate::{Result, Uplink, UpstreamError};

/// What a sync managed to do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub events: usize,
    pub teams: usize,
    pub event_teams: usize,
    pub matches: usize,
    pub stats: usize,
    /// Things that went wrong but did not stop the run, in words an operator can
    /// act on.
    pub problems: Vec<String>,
}

impl SyncReport {
    pub fn merge(&mut self, other: SyncReport) {
        self.events += other.events;
        self.teams += other.teams;
        self.event_teams += other.event_teams;
        self.matches += other.matches;
        self.stats += other.stats;
        self.problems.extend(other.problems);
    }

    fn problem(&mut self, message: impl Into<String>) {
        let message = message.into();
        warn!("{message}");
        self.problems.push(message);
    }

    /// Whether anything at all landed. A run with no changes and no problems is
    /// a healthy no-op; a run with no changes and problems is a failure.
    pub fn is_empty(&self) -> bool {
        self.events == 0
            && self.teams == 0
            && self.event_teams == 0
            && self.matches == 0
            && self.stats == 0
    }

    pub fn summary(&self) -> String {
        format!(
            "{} event(s), {} team(s), {} roster link(s), {} match(es), {} stat row(s), {} problem(s)",
            self.events,
            self.teams,
            self.event_teams,
            self.matches,
            self.stats,
            self.problems.len()
        )
    }
}

// ── FIRST: events, teams, rosters (I4) ──────────────────────────────────────

/// Pull the event list and each event's roster into the database.
pub async fn sync_events<R: Repo + Sync>(
    repo: &R,
    client: &FirstClient,
    filters: &EventFilters,
) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    let now = Utc::now();

    let events = client.events(filters).await?;
    info!("FIRST returned {} event(s)", events.len());

    for raw in &events {
        let Some(key) = raw.tba_key() else {
            report.problem(format!("event {:?} has no usable code or date", raw.name));
            continue;
        };

        let event = Event {
            key: key.clone(),
            name: raw.name.clone(),
            location: Some(raw.location()).filter(|l| !l.is_empty()),
            timezone: raw.timezone.clone(),
            start_date: upstream::parse_date(&raw.date_start).and_then(to_naive),
            end_date: upstream::parse_date(&raw.date_end).and_then(to_naive),
            event_code: Some(raw.code.trim().to_lowercase()),
            event_type: raw.event_type.clone(),
            district_key: raw.district_code.clone(),
            week: raw.week_number,
        };

        if let Err(e) = repo.upsert_event(&event, now).await {
            report.problem(format!("storing event {key}: {e}"));
            continue;
        }
        report.events += 1;

        match client.event_teams(&raw.code).await {
            Ok(teams) => {
                let roster = store_roster(repo, &key, &teams).await;
                report.merge(roster);
            }
            Err(e) if e.is_offline() => return Err(e),
            Err(e) => report.problem(format!("fetching teams for {key}: {e}")),
        }
    }

    Ok(report)
}

async fn store_roster<R: Repo + Sync>(
    repo: &R,
    event_key: &str,
    teams: &[upstream::FirstTeam],
) -> SyncReport {
    let mut report = SyncReport::default();
    let now = Utc::now();

    for raw in teams {
        let team = Team {
            number: raw.team_number,
            name: raw.display_name(),
            nickname: raw.name_short.clone(),
            school: raw.school_name.clone(),
            city: raw.city.clone(),
            state: raw.state_prov.clone(),
            country: raw.country.clone(),
            rookie_year: raw.rookie_year,
            website: raw.website.clone(),
        };

        if let Err(e) = repo.upsert_team(&team, now).await {
            report.problem(format!("storing team {}: {e}", raw.team_number));
            continue;
        }
        report.teams += 1;

        match repo.link_event_team(event_key, raw.team_number, now).await {
            Ok(()) => report.event_teams += 1,
            Err(e) => report.problem(format!(
                "linking team {} to {event_key}: {e}",
                raw.team_number
            )),
        }
    }

    report
}

fn to_naive(parts: (i32, u32, u32)) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(parts.0, parts.1, parts.2)
}

// ── TBA: matches and statistics (I5, I6) ────────────────────────────────────

/// Pull an event's match schedule and results.
pub async fn sync_matches<R: Repo + Sync>(
    repo: &R,
    client: &TbaClient,
    event_key: &str,
) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    let now = Utc::now();

    let matches = client.matches(event_key).await?;

    for raw in &matches {
        let Some(comp_level) = raw.comp_level() else {
            report.problem(format!(
                "match {} has an unrecognised comp_level {:?}",
                raw.key, raw.comp_level
            ));
            continue;
        };

        let record = MatchRecord {
            key: raw.key.clone(),
            event_key: event_key.to_string(),
            comp_level,
            set_number: raw.set_number,
            match_number: raw.match_number,
            red: raw.red_teams(),
            blue: raw.blue_teams(),
            red_score: raw.red_score(),
            blue_score: raw.blue_score(),
            winner: raw.winner().map(|w| w.as_str().to_string()),
            played: raw.played(),
            scheduled_at: raw.scheduled_unix().and_then(from_unix),
            actual_at: raw.actual_unix().and_then(from_unix),
        };

        match repo.upsert_match(&record, now).await {
            Ok(()) => report.matches += 1,
            Err(e) => report.problem(format!("storing match {}: {e}", raw.key)),
        }
    }

    info!("synced {} match(es) for {event_key}", report.matches);
    Ok(report)
}

fn from_unix(seconds: i64) -> Option<chrono::DateTime<Utc>> {
    chrono::DateTime::from_timestamp(seconds, 0)
}

/// Pull rankings and OPRs for every team on an event's roster.
pub async fn sync_stats<R: Repo + Sync>(
    repo: &R,
    client: &TbaClient,
    event_key: &str,
) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    let now = Utc::now();

    let oprs = client.oprs(event_key).await?;
    let rankings = client.rankings(event_key).await?;

    // Component OPRs are a bonus, not a requirement: TBA publishes them late and
    // not for every event. Losing them must not discard the rankings.
    let components = match client.component_oprs(event_key).await {
        Ok(c) => Some(c),
        Err(e) if e.is_offline() => return Err(e),
        Err(e) => {
            report.problem(format!("component OPRs unavailable for {event_key}: {e}"));
            None
        }
    };

    let roster = repo
        .event_teams(event_key)
        .await
        .map_err(|e| UpstreamError::Status {
            api: "repo",
            path: event_key.to_string(),
            status: 0,
            body: e.to_string(),
        })?;

    for team in &roster {
        let key = upstream::team_key(team.number);
        let ranking = rankings.iter().find(|r| r.team_key == key);

        let (auto, teleop, endgame) = match &components {
            Some(c) => (
                c.phase_opr(&key, Phase::Auto),
                c.phase_opr(&key, Phase::Teleop),
                c.phase_opr(&key, Phase::Endgame),
            ),
            None => (None, None, None),
        };

        let stats = TeamEventStats {
            team_number: team.number,
            event_key: event_key.to_string(),
            opr: oprs.oprs.get(&key).copied(),
            dpr: oprs.dprs.get(&key).copied(),
            ccwm: oprs.ccwms.get(&key).copied(),
            auto_opr: auto,
            teleop_opr: teleop,
            endgame_opr: endgame,
            rank: ranking.map(|r| r.rank),
            matches_played: ranking.map(|r| r.matches_played),
            qual_average: ranking.and_then(|r| r.effective_qual_average()),
            avg_match_points: ranking.and_then(|r| r.effective_avg_match_points()),
            wins: ranking.map(|r| r.record.wins),
            losses: ranking.map(|r| r.record.losses),
            ties: ranking.map(|r| r.record.ties),
            dq_count: ranking.map(|r| r.dq),
            qual_points: ranking
                .and_then(|r| r.effective_qual_points())
                .map(clamp_i32),
            elim_points: ranking.and_then(|r| r.elim_points).map(clamp_i32),
            award_points: ranking.and_then(|r| r.award_points).map(clamp_i32),
            alliance_points: ranking.and_then(|r| r.alliance_points).map(clamp_i32),
            total_points: ranking
                .and_then(|r| r.effective_total_points())
                .map(clamp_i32),
            synced_at: Some(now),
        };

        match repo.upsert_team_stats(&stats, now).await {
            Ok(()) => report.stats += 1,
            Err(e) => report.problem(format!("storing stats for team {}: {e}", team.number)),
        }
    }

    info!("synced stats for {} team(s) at {event_key}", report.stats);
    Ok(report)
}

/// TBA's point fields are i64; the schema stores i32. Saturate rather than wrap,
/// so an absurd upstream value becomes an absurd stored value instead of a
/// negative one.
fn clamp_i32(value: i64) -> i32 {
    value.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

// ── Orchestration (I7, I8) ──────────────────────────────────────────────────

/// Everything, for one event.
pub async fn sync_event<R: Repo + Sync>(repo: &R, tba: &TbaClient, event_key: &str) -> SyncReport {
    let mut report = SyncReport::default();

    match sync_matches(repo, tba, event_key).await {
        Ok(r) => report.merge(r),
        Err(e) if e.is_offline() => {
            report.problem(format!("no connection while syncing {event_key}"));
            return report;
        }
        Err(e) => report.problem(format!("matches for {event_key}: {e}")),
    }

    match sync_stats(repo, tba, event_key).await {
        Ok(r) => report.merge(r),
        Err(e) => report.problem(format!("stats for {event_key}: {e}")),
    }

    report
}

/// The pre-event bulk load (I8).
///
/// One command, run at the shop before anyone leaves. It covers the majority of
/// the tedious upstream data, which is why the refurbish plan puts it ahead of
/// every cleverer sync mechanism.
pub async fn bulk_load<R: Repo + Sync>(
    repo: &R,
    first: &FirstClient,
    tba: Option<&TbaClient>,
    filters: &EventFilters,
    uplink: &Uplink,
) -> Result<SyncReport> {
    let mut report = sync_events(repo, first, filters).await?;

    if let Some(tba) = tba {
        let events = repo.list_events().await.unwrap_or_default();
        for event in &events {
            report.merge(sync_event(repo, tba, &event.key).await);
        }
    } else {
        report.problem("TBA key not configured; skipped matches and statistics");
    }

    uplink.record_sync();
    info!("bulk load complete: {}", report.summary());
    Ok(report)
}

/// How often to sync while an event is running.
pub const INTERVAL_DURING_EVENT: std::time::Duration = std::time::Duration::from_secs(2 * 60);
/// How often to sync otherwise.
pub const INTERVAL_BETWEEN_EVENTS: std::time::Duration =
    std::time::Duration::from_secs(3 * 60 * 60);
/// How far ahead an upcoming event counts as imminent.
pub const LOOKAHEAD_DAYS: i64 = 1;

/// Choose a sync cadence from what is on the calendar.
pub fn interval_for(active_event_count: usize) -> std::time::Duration {
    if active_event_count > 0 {
        INTERVAL_DURING_EVENT
    } else {
        INTERVAL_BETWEEN_EVENTS
    }
}

/// Sync whatever is currently relevant: events running today, plus anything
/// starting within a day.
pub async fn sync_active<R: Repo + Sync>(repo: &R, tba: &TbaClient, uplink: &Uplink) -> SyncReport {
    let today = Utc::now().date_naive();
    let mut report = SyncReport::default();

    let events = match repo.active_events(today, LOOKAHEAD_DAYS).await {
        Ok(events) => events,
        Err(e) => {
            report.problem(format!("listing active events: {e}"));
            return report;
        }
    };

    if events.is_empty() {
        return report;
    }

    for event in &events {
        report.merge(sync_event(repo, tba, &event.key).await);
    }

    if !report.is_empty() {
        uplink.record_sync();
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_merge_their_counts_and_problems() {
        let mut a = SyncReport {
            events: 1,
            teams: 2,
            ..Default::default()
        };
        a.merge(SyncReport {
            teams: 3,
            matches: 4,
            problems: vec!["bad".into()],
            ..Default::default()
        });

        assert_eq!(a.events, 1);
        assert_eq!(a.teams, 5);
        assert_eq!(a.matches, 4);
        assert_eq!(a.problems, vec!["bad".to_string()]);
    }

    #[test]
    fn an_untouched_report_is_empty_even_with_problems() {
        let mut report = SyncReport::default();
        assert!(report.is_empty());
        report.problem("everything failed");
        assert!(report.is_empty(), "problems are not progress");
    }

    #[test]
    fn the_cadence_speeds_up_during_an_event() {
        assert_eq!(interval_for(0), INTERVAL_BETWEEN_EVENTS);
        assert_eq!(interval_for(1), INTERVAL_DURING_EVENT);
        assert_eq!(interval_for(5), INTERVAL_DURING_EVENT);
    }

    #[test]
    fn oversized_upstream_points_saturate_rather_than_wrapping_negative() {
        assert_eq!(clamp_i32(42), 42);
        assert_eq!(clamp_i32(i64::MAX), i32::MAX);
        assert_eq!(clamp_i32(i64::MIN), i32::MIN);
    }

    #[test]
    fn unix_timestamps_convert_and_reject_nonsense() {
        assert!(from_unix(1_773_500_400).is_some());
        assert!(from_unix(0).is_some());
        assert!(from_unix(i64::MAX).is_none());
    }

    #[test]
    fn a_summary_names_every_count() {
        let report = SyncReport {
            events: 1,
            teams: 2,
            event_teams: 3,
            matches: 4,
            stats: 5,
            problems: vec!["x".into()],
        };
        let text = report.summary();
        for fragment in [
            "1 event",
            "2 team",
            "3 roster",
            "4 match",
            "5 stat",
            "1 problem",
        ] {
            assert!(
                text.contains(fragment),
                "{text:?} should mention {fragment:?}"
            );
        }
    }
}
