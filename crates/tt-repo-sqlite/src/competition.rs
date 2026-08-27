//! Events, teams, matches, and statistics against SQLite.
//!
//! Every write here is a real `ON CONFLICT` upsert. That is possible because the
//! schema declares the unique constraints the retired PostgreSQL schema was
//! missing, which is what forced it into a select-then-insert-or-update dance
//! that was also a race (REBUILD_SPEC.md 12.11).
//!
//! A note on the SQL strings: no `--` comments inside them. Rust's backslash
//! line continuation removes the newline, so a SQL line comment silently eats
//! the rest of the statement while remaining valid SQL.

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::Row;
use tt_core::matches::CompLevel;
use tt_core::records::{Event, MatchRecord, Team, TeamEventStats};
use tt_repo::Result;

use crate::SqliteRepo;
use crate::users::{from_sql, query_err, to_sql};

fn date_to_sql(date: Option<NaiveDate>) -> Option<String> {
    date.map(|d| d.format("%Y-%m-%d").to_string())
}

fn date_from_sql(raw: Option<String>) -> Option<NaiveDate> {
    raw.as_deref()
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
}

fn event_from_row(row: &sqlx::sqlite::SqliteRow) -> Event {
    Event {
        key: row.get("tba_key"),
        name: row.get("name"),
        location: row.get("location"),
        timezone: row.get("timezone"),
        start_date: date_from_sql(row.get("start_date")),
        end_date: date_from_sql(row.get("end_date")),
        event_code: row.get("event_code"),
        event_type: row.get("event_type"),
        district_key: row.get("district_key"),
        week: row.get("week"),
    }
}

fn team_from_row(row: &sqlx::sqlite::SqliteRow) -> Team {
    Team {
        number: row.get("team_number"),
        name: row.get("name"),
        nickname: row.get("nickname"),
        school: row.get("school"),
        city: row.get("city"),
        state: row.get("state"),
        country: row.get("country"),
        rookie_year: row.get("rookie_year"),
        website: row.get("website"),
    }
}

fn match_from_row(row: &sqlx::sqlite::SqliteRow) -> MatchRecord {
    let ts = |column: &str| -> Option<DateTime<Utc>> {
        row.get::<Option<String>, _>(column)
            .as_deref()
            .and_then(from_sql)
    };
    MatchRecord {
        key: row.get("tba_key"),
        event_key: row.get("event_key"),
        // A row whose comp_level is unreadable is a qualification match; the
        // alternative is dropping a real match out of the schedule.
        comp_level: CompLevel::parse(&row.get::<String, _>("comp_level"))
            .unwrap_or(CompLevel::Qualification),
        set_number: row.get("set_number"),
        match_number: row.get("match_number"),
        red: [row.get("red1"), row.get("red2"), row.get("red3")],
        blue: [row.get("blue1"), row.get("blue2"), row.get("blue3")],
        red_score: row.get("red_score"),
        blue_score: row.get("blue_score"),
        winner: row.get("winner"),
        played: row.get::<i64, _>("played") != 0,
        scheduled_at: ts("scheduled_at"),
        actual_at: ts("actual_at"),
    }
}

fn stats_from_row(row: &sqlx::sqlite::SqliteRow) -> TeamEventStats {
    TeamEventStats {
        team_number: row.get("team_number"),
        event_key: row.get("event_key"),
        opr: row.get("opr"),
        dpr: row.get("dpr"),
        ccwm: row.get("ccwm"),
        auto_opr: row.get("auto_opr"),
        teleop_opr: row.get("teleop_opr"),
        endgame_opr: row.get("endgame_opr"),
        rank: row.get("rank"),
        matches_played: row.get("matches_played"),
        qual_average: row.get("qual_average"),
        avg_match_points: row.get("avg_match_points"),
        wins: row.get("wins"),
        losses: row.get("losses"),
        ties: row.get("ties"),
        dq_count: row.get("dq_count"),
        qual_points: row.get("qual_points"),
        elim_points: row.get("elim_points"),
        award_points: row.get("award_points"),
        alliance_points: row.get("alliance_points"),
        total_points: row.get("total_points"),
        synced_at: row
            .get::<Option<String>, _>("synced_at")
            .as_deref()
            .and_then(from_sql),
    }
}

impl SqliteRepo {
    // ── Events ──────────────────────────────────────────────────────────────

    pub(crate) async fn upsert_event_impl(&self, event: &Event, now: DateTime<Utc>) -> Result<()> {
        let ts = to_sql(now);
        sqlx::query(
            "INSERT INTO events (tba_key, name, location, timezone, start_date, end_date, \
                                 event_code, event_type, district_key, week, created_at, updated_at) \
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
        )
        .bind(&event.key)
        .bind(&event.name)
        .bind(&event.location)
        .bind(&event.timezone)
        .bind(date_to_sql(event.start_date))
        .bind(date_to_sql(event.end_date))
        .bind(&event.event_code)
        .bind(&event.event_type)
        .bind(&event.district_key)
        .bind(event.week)
        .bind(&ts)
        .bind(&ts)
        .execute(&self.pool)
        .await
        .map_err(|e| query_err("upserting event", e))?;
        Ok(())
    }

    pub(crate) async fn event_impl(&self, key: &str) -> Result<Option<Event>> {
        let row = sqlx::query(
            "SELECT tba_key, name, location, timezone, start_date, end_date, event_code, \
                    event_type, district_key, week \
             FROM events WHERE tba_key = ?",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| query_err("loading event", e))?;
        Ok(row.as_ref().map(event_from_row))
    }

    pub(crate) async fn list_events_impl(&self) -> Result<Vec<Event>> {
        let rows = sqlx::query(
            "SELECT tba_key, name, location, timezone, start_date, end_date, event_code, \
                    event_type, district_key, week \
             FROM events ORDER BY start_date IS NULL, start_date, name",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| query_err("listing events", e))?;
        Ok(rows.iter().map(event_from_row).collect())
    }

    pub(crate) async fn events_for_team_impl(&self, team_number: i32) -> Result<Vec<Event>> {
        let rows = sqlx::query(
            "SELECT e.tba_key, e.name, e.location, e.timezone, e.start_date, e.end_date, \
                    e.event_code, e.event_type, e.district_key, e.week \
             FROM events e \
             JOIN event_teams et ON et.event_key = e.tba_key \
             WHERE et.team_number = ? \
             ORDER BY e.start_date IS NULL, e.start_date, e.name",
        )
        .bind(team_number)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| query_err("listing team events", e))?;
        Ok(rows.iter().map(event_from_row).collect())
    }

    pub(crate) async fn active_events_impl(
        &self,
        date: NaiveDate,
        lookahead_days: i64,
    ) -> Result<Vec<Event>> {
        let today = date.format("%Y-%m-%d").to_string();
        let horizon = (date + chrono::TimeDelta::days(lookahead_days))
            .format("%Y-%m-%d")
            .to_string();

        // ISO-8601 dates compare correctly as strings, which is the whole reason
        // they are stored that way.
        let rows = sqlx::query(
            "SELECT tba_key, name, location, timezone, start_date, end_date, event_code, \
                    event_type, district_key, week \
             FROM events \
             WHERE (start_date <= ? AND end_date >= ?) \
                OR (start_date > ? AND start_date <= ?) \
             ORDER BY start_date",
        )
        .bind(&today)
        .bind(&today)
        .bind(&today)
        .bind(&horizon)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| query_err("listing active events", e))?;
        Ok(rows.iter().map(event_from_row).collect())
    }

    // ── Teams ───────────────────────────────────────────────────────────────

    pub(crate) async fn upsert_team_impl(&self, team: &Team, now: DateTime<Utc>) -> Result<()> {
        let ts = to_sql(now);
        sqlx::query(
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
        )
        .bind(team.number)
        .bind(&team.name)
        .bind(&team.nickname)
        .bind(&team.school)
        .bind(&team.city)
        .bind(&team.state)
        .bind(&team.country)
        .bind(team.rookie_year)
        .bind(&team.website)
        .bind(&ts)
        .bind(&ts)
        .execute(&self.pool)
        .await
        .map_err(|e| query_err("upserting team", e))?;
        Ok(())
    }

    pub(crate) async fn team_impl(&self, number: i32) -> Result<Option<Team>> {
        let row = sqlx::query(
            "SELECT team_number, name, nickname, school, city, state, country, rookie_year, website \
             FROM teams WHERE team_number = ?",
        )
        .bind(number)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| query_err("loading team", e))?;
        Ok(row.as_ref().map(team_from_row))
    }

    pub(crate) async fn event_teams_impl(&self, event_key: &str) -> Result<Vec<Team>> {
        let rows = sqlx::query(
            "SELECT t.team_number, t.name, t.nickname, t.school, t.city, t.state, t.country, \
                    t.rookie_year, t.website \
             FROM teams t \
             JOIN event_teams et ON et.team_number = t.team_number \
             WHERE et.event_key = ? \
             ORDER BY t.team_number",
        )
        .bind(event_key)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| query_err("listing event teams", e))?;
        Ok(rows.iter().map(team_from_row).collect())
    }

    pub(crate) async fn link_event_team_impl(
        &self,
        event_key: &str,
        team_number: i32,
        now: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO event_teams (event_key, team_number, created_at) VALUES (?, ?, ?) \
             ON CONFLICT (event_key, team_number) DO NOTHING",
        )
        .bind(event_key)
        .bind(team_number)
        .bind(to_sql(now))
        .execute(&self.pool)
        .await
        .map_err(|e| query_err("linking event team", e))?;
        Ok(())
    }

    // ── Matches ─────────────────────────────────────────────────────────────

    pub(crate) async fn upsert_match_impl(
        &self,
        record: &MatchRecord,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let ts = to_sql(now);
        sqlx::query(
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
        )
        .bind(&record.key)
        .bind(&record.event_key)
        .bind(record.comp_level.as_str())
        .bind(record.set_number)
        .bind(record.match_number)
        .bind(record.red[0])
        .bind(record.red[1])
        .bind(record.red[2])
        .bind(record.blue[0])
        .bind(record.blue[1])
        .bind(record.blue[2])
        .bind(record.red_score)
        .bind(record.blue_score)
        .bind(&record.winner)
        .bind(record.played as i64)
        .bind(record.scheduled_at.map(to_sql))
        .bind(record.actual_at.map(to_sql))
        .bind(&ts)
        .bind(&ts)
        .execute(&self.pool)
        .await
        .map_err(|e| query_err("upserting match", e))?;
        Ok(())
    }

    pub(crate) async fn event_matches_impl(&self, event_key: &str) -> Result<Vec<MatchRecord>> {
        let rows = sqlx::query(
            "SELECT tba_key, event_key, comp_level, set_number, match_number, \
                    red1, red2, red3, blue1, blue2, blue3, red_score, blue_score, winner, played, \
                    scheduled_at, actual_at \
             FROM matches WHERE event_key = ? \
             ORDER BY CASE comp_level WHEN 'qm' THEN 0 WHEN 'sf' THEN 1 ELSE 2 END, \
                      set_number, match_number",
        )
        .bind(event_key)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| query_err("listing matches", e))?;
        Ok(rows.iter().map(match_from_row).collect())
    }

    pub(crate) async fn team_matches_impl(
        &self,
        event_key: &str,
        team_number: i32,
    ) -> Result<Vec<MatchRecord>> {
        let rows = sqlx::query(
            "SELECT tba_key, event_key, comp_level, set_number, match_number, \
                    red1, red2, red3, blue1, blue2, blue3, red_score, blue_score, winner, played, \
                    scheduled_at, actual_at \
             FROM matches \
             WHERE event_key = ? \
               AND ? IN (red1, red2, red3, blue1, blue2, blue3) \
             ORDER BY CASE comp_level WHEN 'qm' THEN 0 WHEN 'sf' THEN 1 ELSE 2 END, \
                      set_number, match_number",
        )
        .bind(event_key)
        .bind(team_number)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| query_err("listing team matches", e))?;
        Ok(rows.iter().map(match_from_row).collect())
    }

    // ── Statistics ──────────────────────────────────────────────────────────

    pub(crate) async fn upsert_team_stats_impl(
        &self,
        stats: &TeamEventStats,
        now: DateTime<Utc>,
    ) -> Result<()> {
        sqlx::query(
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
                qual_average = excluded.qual_average, avg_match_points = excluded.avg_match_points, \
                wins = excluded.wins, losses = excluded.losses, ties = excluded.ties, \
                dq_count = excluded.dq_count, qual_points = excluded.qual_points, \
                elim_points = excluded.elim_points, award_points = excluded.award_points, \
                alliance_points = excluded.alliance_points, total_points = excluded.total_points, \
                synced_at = excluded.synced_at",
        )
        .bind(stats.team_number)
        .bind(&stats.event_key)
        .bind(stats.opr)
        .bind(stats.dpr)
        .bind(stats.ccwm)
        .bind(stats.auto_opr)
        .bind(stats.teleop_opr)
        .bind(stats.endgame_opr)
        .bind(stats.rank)
        .bind(stats.matches_played)
        .bind(stats.qual_average)
        .bind(stats.avg_match_points)
        .bind(stats.wins)
        .bind(stats.losses)
        .bind(stats.ties)
        .bind(stats.dq_count)
        .bind(stats.qual_points)
        .bind(stats.elim_points)
        .bind(stats.award_points)
        .bind(stats.alliance_points)
        .bind(stats.total_points)
        .bind(to_sql(stats.synced_at.unwrap_or(now)))
        .execute(&self.pool)
        .await
        .map_err(|e| query_err("upserting team stats", e))?;
        Ok(())
    }

    pub(crate) async fn team_stats_impl(
        &self,
        event_key: &str,
        team_number: i32,
    ) -> Result<Option<TeamEventStats>> {
        let row = sqlx::query(
            "SELECT team_number, event_key, opr, dpr, ccwm, auto_opr, teleop_opr, endgame_opr, \
                    rank, matches_played, qual_average, avg_match_points, wins, losses, ties, \
                    dq_count, qual_points, elim_points, award_points, alliance_points, \
                    total_points, synced_at \
             FROM team_event_stats WHERE event_key = ? AND team_number = ?",
        )
        .bind(event_key)
        .bind(team_number)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| query_err("loading team stats", e))?;
        Ok(row.as_ref().map(stats_from_row))
    }

    pub(crate) async fn event_stats_impl(&self, event_key: &str) -> Result<Vec<TeamEventStats>> {
        let rows = sqlx::query(
            "SELECT team_number, event_key, opr, dpr, ccwm, auto_opr, teleop_opr, endgame_opr, \
                    rank, matches_played, qual_average, avg_match_points, wins, losses, ties, \
                    dq_count, qual_points, elim_points, award_points, alliance_points, \
                    total_points, synced_at \
             FROM team_event_stats WHERE event_key = ? \
             ORDER BY rank IS NULL, rank, team_number",
        )
        .bind(event_key)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| query_err("listing event stats", e))?;
        Ok(rows.iter().map(stats_from_row).collect())
    }
}
