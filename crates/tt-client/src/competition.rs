//! Events, teams, matches, and statistics: `tt_repo_sqlite::competition`, on
//! the device. On a phone these rows come from upstream responses, through
//! the same upserts the Pi's sync uses (S4).
//!
//! No `--` comments inside the SQL strings, for the server's reason: the
//! backslash continuation removes the newline, and a line comment would eat
//! the rest of the statement while staying valid SQL.

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{OptionalExtension, Row, params};
use tt_core::matches::CompLevel;
use tt_core::records::{Event, MatchRecord, Team, TeamEventStats};
use tt_repo::Result;

use crate::ClientRepo;
use crate::sql::{Context, to_sql, ts_column};

fn date_to_sql(date: Option<NaiveDate>) -> Option<String> {
    date.map(|d| d.format("%Y-%m-%d").to_string())
}

fn date_from_sql(raw: Option<String>) -> Option<NaiveDate> {
    raw.as_deref()
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
}

fn event_from_row(row: &Row) -> rusqlite::Result<Event> {
    Ok(Event {
        key: row.get("tba_key")?,
        name: row.get("name")?,
        location: row.get("location")?,
        timezone: row.get("timezone")?,
        start_date: date_from_sql(row.get("start_date")?),
        end_date: date_from_sql(row.get("end_date")?),
        event_code: row.get("event_code")?,
        event_type: row.get("event_type")?,
        district_key: row.get("district_key")?,
        week: row.get("week")?,
    })
}

fn team_from_row(row: &Row) -> rusqlite::Result<Team> {
    Ok(Team {
        number: row.get("team_number")?,
        name: row.get("name")?,
        nickname: row.get("nickname")?,
        school: row.get("school")?,
        city: row.get("city")?,
        state: row.get("state")?,
        country: row.get("country")?,
        rookie_year: row.get("rookie_year")?,
        website: row.get("website")?,
    })
}

fn match_from_row(row: &Row) -> rusqlite::Result<MatchRecord> {
    Ok(MatchRecord {
        key: row.get("tba_key")?,
        event_key: row.get("event_key")?,
        // As on the server: unreadable is a qualification match, rather than
        // a real match dropped from the schedule.
        comp_level: CompLevel::parse(&row.get::<_, String>("comp_level")?)
            .unwrap_or(CompLevel::Qualification),
        set_number: row.get("set_number")?,
        match_number: row.get("match_number")?,
        red: [row.get("red1")?, row.get("red2")?, row.get("red3")?],
        blue: [row.get("blue1")?, row.get("blue2")?, row.get("blue3")?],
        red_score: row.get("red_score")?,
        blue_score: row.get("blue_score")?,
        winner: row.get("winner")?,
        played: row.get::<_, i64>("played")? != 0,
        scheduled_at: ts_column(row.get("scheduled_at")?),
        actual_at: ts_column(row.get("actual_at")?),
    })
}

fn stats_from_row(row: &Row) -> rusqlite::Result<TeamEventStats> {
    Ok(TeamEventStats {
        team_number: row.get("team_number")?,
        event_key: row.get("event_key")?,
        opr: row.get("opr")?,
        dpr: row.get("dpr")?,
        ccwm: row.get("ccwm")?,
        auto_opr: row.get("auto_opr")?,
        teleop_opr: row.get("teleop_opr")?,
        endgame_opr: row.get("endgame_opr")?,
        rank: row.get("rank")?,
        matches_played: row.get("matches_played")?,
        qual_average: row.get("qual_average")?,
        avg_match_points: row.get("avg_match_points")?,
        wins: row.get("wins")?,
        losses: row.get("losses")?,
        ties: row.get("ties")?,
        dq_count: row.get("dq_count")?,
        qual_points: row.get("qual_points")?,
        elim_points: row.get("elim_points")?,
        award_points: row.get("award_points")?,
        alliance_points: row.get("alliance_points")?,
        total_points: row.get("total_points")?,
        synced_at: ts_column(row.get("synced_at")?),
    })
}

const EVENT_COLUMNS: &str = "tba_key, name, location, timezone, start_date, end_date, \
                             event_code, event_type, district_key, week";

const MATCH_COLUMNS: &str = "tba_key, event_key, comp_level, set_number, match_number, \
                             red1, red2, red3, blue1, blue2, blue3, red_score, blue_score, \
                             winner, played, scheduled_at, actual_at";

/// Playing order, as `event_matches` and `team_matches` sort on the server.
const MATCH_ORDER: &str = "ORDER BY CASE comp_level WHEN 'qm' THEN 0 WHEN 'ef' THEN 1 \
                           WHEN 'qf' THEN 2 WHEN 'sf' THEN 3 ELSE 4 END, \
                           set_number, match_number";

const STATS_COLUMNS: &str = "team_number, event_key, opr, dpr, ccwm, auto_opr, teleop_opr, \
                             endgame_opr, rank, matches_played, qual_average, avg_match_points, \
                             wins, losses, ties, dq_count, qual_points, elim_points, \
                             award_points, alliance_points, total_points, synced_at";

impl ClientRepo {
    // ── Events ──────────────────────────────────────────────────────────────

    pub(crate) fn upsert_event_impl(&self, event: &Event, now: DateTime<Utc>) -> Result<()> {
        let ts = to_sql(now);
        self.conn
            .execute(
                "INSERT INTO events (tba_key, name, location, timezone, start_date, end_date, \
                                     event_code, event_type, district_key, week, created_at, \
                                     updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (tba_key) DO UPDATE SET \
                    name = excluded.name, \
                    location = excluded.location, \
                    timezone = COALESCE(excluded.timezone, events.timezone), \
                    start_date = excluded.start_date, \
                    end_date = excluded.end_date, \
                    event_code = excluded.event_code, \
                    event_type = excluded.event_type, \
                    district_key = excluded.district_key, \
                    week = excluded.week, \
                    updated_at = excluded.updated_at",
                params![
                    event.key,
                    event.name,
                    event.location,
                    event.timezone,
                    date_to_sql(event.start_date),
                    date_to_sql(event.end_date),
                    event.event_code,
                    event.event_type,
                    event.district_key,
                    event.week,
                    ts,
                    ts,
                ],
            )
            .ctx("upserting event")?;
        Ok(())
    }

    pub(crate) fn event_impl(&self, key: &str) -> Result<Option<Event>> {
        self.conn
            .query_row(
                &format!("SELECT {EVENT_COLUMNS} FROM events WHERE tba_key = ?"),
                [key],
                event_from_row,
            )
            .optional()
            .ctx("loading event")
    }

    pub(crate) fn list_events_impl(&self) -> Result<Vec<Event>> {
        self.all(
            &format!(
                "SELECT {EVENT_COLUMNS} FROM events \
                 ORDER BY start_date IS NULL, start_date, name"
            ),
            [],
            event_from_row,
            "listing events",
        )
    }

    pub(crate) fn events_for_team_impl(&self, team_number: i32) -> Result<Vec<Event>> {
        self.all(
            "SELECT e.tba_key, e.name, e.location, e.timezone, e.start_date, e.end_date, \
                    e.event_code, e.event_type, e.district_key, e.week \
             FROM events e \
             JOIN event_teams et ON et.event_key = e.tba_key \
             WHERE et.team_number = ? \
             ORDER BY e.start_date IS NULL, e.start_date, e.name",
            [team_number],
            event_from_row,
            "listing team events",
        )
    }

    pub(crate) fn active_events_impl(
        &self,
        date: NaiveDate,
        lookahead_days: i64,
    ) -> Result<Vec<Event>> {
        let today = date.format("%Y-%m-%d").to_string();
        let horizon = (date + chrono::TimeDelta::days(lookahead_days))
            .format("%Y-%m-%d")
            .to_string();
        self.all(
            &format!(
                "SELECT {EVENT_COLUMNS} FROM events \
                 WHERE (start_date <= ?1 AND end_date >= ?1) \
                    OR (start_date > ?1 AND start_date <= ?2) \
                 ORDER BY start_date"
            ),
            params![today, horizon],
            event_from_row,
            "listing active events",
        )
    }

    // ── Teams ───────────────────────────────────────────────────────────────

    pub(crate) fn upsert_team_impl(&self, team: &Team, now: DateTime<Utc>) -> Result<()> {
        let ts = to_sql(now);
        self.conn
            .execute(
                "INSERT INTO teams (team_number, name, nickname, school, city, state, country, \
                                    rookie_year, website, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (team_number) DO UPDATE SET \
                    name = excluded.name, \
                    nickname = COALESCE(excluded.nickname, teams.nickname), \
                    school = COALESCE(excluded.school, teams.school), \
                    city = COALESCE(excluded.city, teams.city), \
                    state = COALESCE(excluded.state, teams.state), \
                    country = COALESCE(excluded.country, teams.country), \
                    rookie_year = COALESCE(excluded.rookie_year, teams.rookie_year), \
                    website = COALESCE(excluded.website, teams.website), \
                    updated_at = excluded.updated_at",
                params![
                    team.number,
                    team.name,
                    team.nickname,
                    team.school,
                    team.city,
                    team.state,
                    team.country,
                    team.rookie_year,
                    team.website,
                    ts,
                    ts,
                ],
            )
            .ctx("upserting team")?;
        Ok(())
    }

    pub(crate) fn team_impl(&self, number: i32) -> Result<Option<Team>> {
        self.conn
            .query_row(
                "SELECT team_number, name, nickname, school, city, state, country, \
                        rookie_year, website \
                 FROM teams WHERE team_number = ?",
                [number],
                team_from_row,
            )
            .optional()
            .ctx("loading team")
    }

    pub(crate) fn event_teams_impl(&self, event_key: &str) -> Result<Vec<Team>> {
        self.all(
            "SELECT t.team_number, t.name, t.nickname, t.school, t.city, t.state, t.country, \
                    t.rookie_year, t.website \
             FROM teams t \
             JOIN event_teams et ON et.team_number = t.team_number \
             WHERE et.event_key = ? \
             ORDER BY t.team_number",
            [event_key],
            team_from_row,
            "listing event teams",
        )
    }

    pub(crate) fn link_event_team_impl(
        &self,
        event_key: &str,
        team_number: i32,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO event_teams (event_key, team_number, created_at) VALUES (?, ?, ?) \
                 ON CONFLICT (event_key, team_number) DO NOTHING",
                params![event_key, team_number, to_sql(now)],
            )
            .ctx("linking event team")?;
        Ok(())
    }

    // ── Matches ─────────────────────────────────────────────────────────────

    pub(crate) fn upsert_match_impl(&self, record: &MatchRecord, now: DateTime<Utc>) -> Result<()> {
        let ts = to_sql(now);
        self.conn
            .execute(
                "INSERT INTO matches (tba_key, event_key, comp_level, set_number, match_number, \
                                      red1, red2, red3, blue1, blue2, blue3, \
                                      red_score, blue_score, winner, played, \
                                      scheduled_at, actual_at, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (tba_key) DO UPDATE SET \
                    red1 = excluded.red1, red2 = excluded.red2, red3 = excluded.red3, \
                    blue1 = excluded.blue1, blue2 = excluded.blue2, blue3 = excluded.blue3, \
                    red_score = excluded.red_score, blue_score = excluded.blue_score, \
                    winner = excluded.winner, played = excluded.played, \
                    scheduled_at = excluded.scheduled_at, actual_at = excluded.actual_at, \
                    updated_at = excluded.updated_at",
                params![
                    record.key,
                    record.event_key,
                    record.comp_level.as_str(),
                    record.set_number,
                    record.match_number,
                    record.red[0],
                    record.red[1],
                    record.red[2],
                    record.blue[0],
                    record.blue[1],
                    record.blue[2],
                    record.red_score,
                    record.blue_score,
                    record.winner,
                    record.played as i64,
                    record.scheduled_at.map(to_sql),
                    record.actual_at.map(to_sql),
                    ts,
                    ts,
                ],
            )
            .ctx("upserting match")?;
        Ok(())
    }

    pub(crate) fn match_by_key_impl(&self, key: &str) -> Result<Option<MatchRecord>> {
        self.conn
            .query_row(
                &format!("SELECT {MATCH_COLUMNS} FROM matches WHERE tba_key = ?"),
                [key],
                match_from_row,
            )
            .optional()
            .ctx("loading match")
    }

    pub(crate) fn event_matches_impl(&self, event_key: &str) -> Result<Vec<MatchRecord>> {
        self.all(
            &format!("SELECT {MATCH_COLUMNS} FROM matches WHERE event_key = ? {MATCH_ORDER}"),
            [event_key],
            match_from_row,
            "listing matches",
        )
    }

    pub(crate) fn team_matches_impl(
        &self,
        event_key: &str,
        team_number: i32,
    ) -> Result<Vec<MatchRecord>> {
        self.all(
            &format!(
                "SELECT {MATCH_COLUMNS} FROM matches \
                 WHERE event_key = ? AND ? IN (red1, red2, red3, blue1, blue2, blue3) \
                 {MATCH_ORDER}"
            ),
            params![event_key, team_number],
            match_from_row,
            "listing team matches",
        )
    }

    // ── Statistics ──────────────────────────────────────────────────────────

    pub(crate) fn upsert_team_stats_impl(
        &self,
        stats: &TeamEventStats,
        now: DateTime<Utc>,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO team_event_stats (team_number, event_key, opr, dpr, ccwm, \
                     auto_opr, teleop_opr, endgame_opr, rank, matches_played, qual_average, \
                     avg_match_points, wins, losses, ties, dq_count, qual_points, elim_points, \
                     award_points, alliance_points, total_points, synced_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (team_number, event_key) DO UPDATE SET \
                    opr = excluded.opr, dpr = excluded.dpr, ccwm = excluded.ccwm, \
                    auto_opr = COALESCE(excluded.auto_opr, team_event_stats.auto_opr), \
                    teleop_opr = COALESCE(excluded.teleop_opr, team_event_stats.teleop_opr), \
                    endgame_opr = COALESCE(excluded.endgame_opr, team_event_stats.endgame_opr), \
                    rank = excluded.rank, matches_played = excluded.matches_played, \
                    qual_average = excluded.qual_average, \
                    avg_match_points = excluded.avg_match_points, \
                    wins = excluded.wins, losses = excluded.losses, ties = excluded.ties, \
                    dq_count = excluded.dq_count, qual_points = excluded.qual_points, \
                    elim_points = excluded.elim_points, award_points = excluded.award_points, \
                    alliance_points = excluded.alliance_points, \
                    total_points = excluded.total_points, \
                    synced_at = excluded.synced_at",
                params![
                    stats.team_number,
                    stats.event_key,
                    stats.opr,
                    stats.dpr,
                    stats.ccwm,
                    stats.auto_opr,
                    stats.teleop_opr,
                    stats.endgame_opr,
                    stats.rank,
                    stats.matches_played,
                    stats.qual_average,
                    stats.avg_match_points,
                    stats.wins,
                    stats.losses,
                    stats.ties,
                    stats.dq_count,
                    stats.qual_points,
                    stats.elim_points,
                    stats.award_points,
                    stats.alliance_points,
                    stats.total_points,
                    to_sql(stats.synced_at.unwrap_or(now)),
                ],
            )
            .ctx("upserting team stats")?;
        Ok(())
    }

    pub(crate) fn team_stats_impl(
        &self,
        event_key: &str,
        team_number: i32,
    ) -> Result<Option<TeamEventStats>> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {STATS_COLUMNS} FROM team_event_stats \
                     WHERE event_key = ? AND team_number = ?"
                ),
                params![event_key, team_number],
                stats_from_row,
            )
            .optional()
            .ctx("loading team stats")
    }

    pub(crate) fn event_stats_impl(&self, event_key: &str) -> Result<Vec<TeamEventStats>> {
        self.all(
            &format!(
                "SELECT {STATS_COLUMNS} FROM team_event_stats WHERE event_key = ? \
                 ORDER BY rank IS NULL, rank, team_number"
            ),
            [event_key],
            stats_from_row,
            "listing event stats",
        )
    }
}
