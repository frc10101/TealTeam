// Shared TBA statistics/matches sync used by both the background syncer and
// the per-team login sync (port of Services/TbaStatsSync.cs).

use chrono::{DateTime, TimeZone, Utc};
use sqlx::PgPool;
use tracing::{info, warn};

use crate::tba::TbaClient;

#[derive(sqlx::FromRow)]
struct EventTeamKeyRow {
    team_id: i32,
    tba_key: Option<String>,
    team_number: i32,
}

pub async fn sync_team_stats_for_event(
    pool: &PgPool,
    tba: &TbaClient,
    event_id: i32,
    event_tba_key: &str,
) -> anyhow::Result<()> {
    let opr_data = tba.get_event_oprs(event_tba_key).await.map_err(|e| {
        warn!("failed to fetch OPR data for event {event_tba_key}: {e}");
        e
    })?;

    // Component OPRs are not critical; continue without them.
    let component_data = match tba.get_event_component_oprs(event_tba_key).await {
        Ok(data) => Some(data),
        Err(e) => {
            warn!("failed to fetch component OPR data for event {event_tba_key}: {e}");
            None
        }
    };

    let rankings = tba.get_event_rankings(event_tba_key).await.map_err(|e| {
        warn!("failed to fetch rankings for event {event_tba_key}: {e}");
        e
    })?;

    let rankings_by_team: std::collections::HashMap<&str, &crate::tba::RankingInfo> =
        rankings.iter().map(|r| (r.team_key.as_str(), r)).collect();

    let event_teams: Vec<EventTeamKeyRow> = sqlx::query_as(
        "SELECT teams.id AS team_id, teams.tba_key, teams.team_number
         FROM event_teams
         JOIN teams ON teams.id = event_teams.team_id
         WHERE event_teams.event_id = $1",
    )
    .bind(event_id)
    .fetch_all(pool)
    .await?;

    let mut stats_updated = 0;
    for et in &event_teams {
        let tba_key = match &et.tba_key {
            Some(k) if !k.is_empty() => k.clone(),
            _ => format!("frc{}", et.team_number),
        };

        let opr = opr_data.oprs.get(&tba_key).copied();
        let dpr = opr_data.dprs.get(&tba_key).copied();
        let ccwm = opr_data.ccwms.get(&tba_key).copied();

        let (auto_opr, teleop_opr, endgame_opr) = component_data
            .as_ref()
            .map(|c| c.team_phase_oprs(&tba_key))
            .unwrap_or((None, None, None));

        let mut rank = None;
        let mut matches_played = None;
        let mut qual_average = None;
        let mut avg_match_points = None;
        let mut wins = None;
        let mut losses = None;
        let mut ties = None;
        let mut dq_count = None;
        let mut qual_points = None;
        let mut elim_points = None;
        let mut award_points = None;
        let mut alliance_points = None;
        let mut total_points = None;

        if let Some(ranking) = rankings_by_team.get(tba_key.as_str()) {
            rank = Some(ranking.rank);
            matches_played = Some(ranking.matches_played);
            qual_average = ranking.effective_qual_average();
            avg_match_points = ranking.effective_avg_match_points();
            wins = Some(ranking.record.wins);
            losses = Some(ranking.record.losses);
            ties = Some(ranking.record.ties);
            dq_count = Some(ranking.dq);
            qual_points = ranking.effective_qual_points();
            elim_points = ranking.elim_points.map(|v| v as i64);
            award_points = ranking.award_points.map(|v| v as i64);
            alliance_points = ranking.alliance_points.map(|v| v as i64);
            total_points = ranking.effective_total_points();
        }

        let result = sqlx::query(
            "INSERT INTO team_event_stats (team_id, event_id, opr, dpr, ccwm,
                auto_opr, teleop_opr, endgame_opr, rank, matches_played,
                qual_average, avg_match_points, wins, losses, ties, dq_count,
                qual_points, elim_points, award_points, alliance_points, total_points)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                $15, $16, $17, $18, $19, $20, $21)
             ON CONFLICT (team_id, event_id) DO UPDATE SET
                opr = EXCLUDED.opr, dpr = EXCLUDED.dpr, ccwm = EXCLUDED.ccwm,
                auto_opr = EXCLUDED.auto_opr, teleop_opr = EXCLUDED.teleop_opr,
                endgame_opr = EXCLUDED.endgame_opr, rank = EXCLUDED.rank,
                matches_played = EXCLUDED.matches_played, qual_average = EXCLUDED.qual_average,
                avg_match_points = EXCLUDED.avg_match_points, wins = EXCLUDED.wins,
                losses = EXCLUDED.losses, ties = EXCLUDED.ties, dq_count = EXCLUDED.dq_count,
                qual_points = EXCLUDED.qual_points, elim_points = EXCLUDED.elim_points,
                award_points = EXCLUDED.award_points, alliance_points = EXCLUDED.alliance_points,
                total_points = EXCLUDED.total_points, updated_at = CURRENT_TIMESTAMP",
        )
        .bind(et.team_id)
        .bind(event_id)
        .bind(opr)
        .bind(dpr)
        .bind(ccwm)
        .bind(auto_opr)
        .bind(teleop_opr)
        .bind(endgame_opr)
        .bind(rank)
        .bind(matches_played)
        .bind(qual_average)
        .bind(avg_match_points)
        .bind(wins)
        .bind(losses)
        .bind(ties)
        .bind(dq_count)
        .bind(qual_points.map(|v| v as i32))
        .bind(elim_points.map(|v| v as i32))
        .bind(award_points.map(|v| v as i32))
        .bind(alliance_points.map(|v| v as i32))
        .bind(total_points.map(|v| v as i32))
        .execute(pool)
        .await;

        match result {
            Ok(_) => stats_updated += 1,
            Err(e) => warn!("failed to upsert stats for team {} at event {event_id}: {e}", et.team_id),
        }
    }

    info!("synced TBA stats for {stats_updated} teams at event {event_id}");
    Ok(())
}

pub async fn sync_event_matches(
    pool: &PgPool,
    tba: &TbaClient,
    event_id: i32,
    event_tba_key: &str,
) -> anyhow::Result<()> {
    let matches = tba.get_event_matches(event_tba_key).await.map_err(|e| {
        warn!("failed to fetch matches for event {event_tba_key}: {e}");
        e
    })?;

    for m in &matches {
        let mut comp_level = m.comp_level.trim().to_lowercase();
        if comp_level.is_empty() {
            comp_level = "qm".to_string();
        }

        let match_number = normalize_match_number(&comp_level, m.set_number, m.match_number);
        let mut winning_alliance = "";
        if m.alliances.red.score >= 0 && m.alliances.blue.score >= 0 {
            if m.alliances.red.score > m.alliances.blue.score {
                winning_alliance = "red";
            } else if m.alliances.blue.score > m.alliances.red.score {
                winning_alliance = "blue";
            }
        }

        let played = m.actual_time > 0
            || (m.score_breakdown.as_ref().map(|v| !v.is_null()).unwrap_or(false)
                && m.alliances.red.score >= 0
                && m.alliances.blue.score >= 0);

        // Alliance team numbers from TBA keys ("frc1234" -> 1234)
        let red1 = parse_tba_team_number(&m.alliances.red.teams, 0);
        let red2 = parse_tba_team_number(&m.alliances.red.teams, 1);
        let red3 = parse_tba_team_number(&m.alliances.red.teams, 2);
        let blue1 = parse_tba_team_number(&m.alliances.blue.teams, 0);
        let blue2 = parse_tba_team_number(&m.alliances.blue.teams, 1);
        let blue3 = parse_tba_team_number(&m.alliances.blue.teams, 2);

        let result = sqlx::query(
            "INSERT INTO matches (event_id, match_number, match_type, red_score, blue_score, played,
                tba_key, comp_level, set_number, scheduled_time, actual_time, winning_alliance,
                red1, red2, red3, blue1, blue2, blue3)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
             ON CONFLICT (event_id, match_number, match_type) DO UPDATE SET
                red_score = EXCLUDED.red_score, blue_score = EXCLUDED.blue_score, played = EXCLUDED.played,
                tba_key = EXCLUDED.tba_key, comp_level = EXCLUDED.comp_level, set_number = EXCLUDED.set_number,
                scheduled_time = EXCLUDED.scheduled_time, actual_time = EXCLUDED.actual_time,
                winning_alliance = EXCLUDED.winning_alliance,
                red1 = EXCLUDED.red1, red2 = EXCLUDED.red2, red3 = EXCLUDED.red3,
                blue1 = EXCLUDED.blue1, blue2 = EXCLUDED.blue2, blue3 = EXCLUDED.blue3,
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(event_id)
        .bind(match_number)
        .bind(&comp_level)
        .bind(m.alliances.red.score)
        .bind(m.alliances.blue.score)
        .bind(played)
        .bind(&m.key)
        .bind(&comp_level)
        .bind(m.set_number)
        .bind(unix_to_utc(m.scheduled_time))
        .bind(unix_to_utc(m.actual_time))
        .bind(winning_alliance)
        .bind(red1)
        .bind(red2)
        .bind(red3)
        .bind(blue1)
        .bind(blue2)
        .bind(blue3)
        .execute(pool)
        .await;

        if let Err(e) = result {
            warn!("failed to upsert match {} for event {event_id}: {e}", m.key);
        }
    }

    info!("synced {} matches for event {event_id}", matches.len());
    Ok(())
}

pub fn normalize_tba_event_key(raw: &str, season: i32) -> String {
    let key = raw.trim().to_lowercase();
    if key.is_empty() {
        return String::new();
    }
    let season_prefix = season.to_string();
    if key.starts_with(&season_prefix) {
        key
    } else {
        format!("{season_prefix}{key}")
    }
}

fn normalize_match_number(comp_level: &str, set_number: i32, match_number: i32) -> i32 {
    if comp_level == "qm" || set_number <= 0 {
        match_number
    } else {
        set_number * 100 + match_number
    }
}

fn unix_to_utc(ts: i64) -> Option<DateTime<Utc>> {
    if ts <= 0 {
        None
    } else {
        Utc.timestamp_opt(ts, 0).single()
    }
}

fn parse_tba_team_number(teams: &[String], index: usize) -> Option<i32> {
    let key = teams.get(index)?.trim();
    key.strip_prefix("frc")
        .or_else(|| key.strip_prefix("FRC"))
        .and_then(|n| n.parse().ok())
}
