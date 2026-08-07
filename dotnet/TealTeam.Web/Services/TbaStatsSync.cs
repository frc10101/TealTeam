using Dapper;
using TealTeam.Web.Data;

namespace TealTeam.Web.Services;

/// <summary>
/// Shared TBA statistics/matches sync used by both the background syncer and
/// the per-team login sync. Port of the stats upsert logic from
/// internal/frc/team_stats_sync.go and sync.go.
/// </summary>
public class TbaStatsSync(Db db, ILogger<TbaStatsSync> logger)
{
    private class EventTeamKeyRow
    {
        public int TeamId { get; set; }
        public string? TbaKey { get; set; }
        public int TeamNumber { get; set; }
    }

    public async Task SyncTeamStatsForEventAsync(TbaClient tbaClient, int eventId, string eventTbaKey, CancellationToken ct)
    {
        OprData oprData;
        try
        {
            oprData = await tbaClient.GetEventOprsAsync(eventTbaKey, ct);
        }
        catch (Exception ex)
        {
            logger.LogWarning(ex, "failed to fetch OPR data for event {Event}", eventTbaKey);
            throw;
        }

        ComponentOprData? componentData = null;
        try
        {
            componentData = await tbaClient.GetEventComponentOprsAsync(eventTbaKey, ct);
        }
        catch (Exception ex)
        {
            // Not critical, continue without component data.
            logger.LogWarning(ex, "failed to fetch component OPR data for event {Event}", eventTbaKey);
        }

        List<RankingInfo> rankings;
        try
        {
            rankings = await tbaClient.GetEventRankingsAsync(eventTbaKey, ct);
        }
        catch (Exception ex)
        {
            logger.LogWarning(ex, "failed to fetch rankings for event {Event}", eventTbaKey);
            throw;
        }

        var rankingsByTeamKey = new Dictionary<string, RankingInfo>();
        foreach (var ranking in rankings)
        {
            rankingsByTeamKey[ranking.TeamKey] = ranking;
        }

        await using var conn = await db.OpenAsync(ct);
        var eventTeams = (await conn.QueryAsync<EventTeamKeyRow>("""
            SELECT teams.id AS team_id, teams.tba_key, teams.team_number
            FROM event_teams
            JOIN teams ON teams.id = event_teams.team_id
            WHERE event_teams.event_id = @eventId
            """, new { eventId })).ToList();

        var statsUpdated = 0;
        foreach (var et in eventTeams)
        {
            var tbaKey = string.IsNullOrEmpty(et.TbaKey) ? $"frc{et.TeamNumber}" : et.TbaKey;

            double? opr = oprData.Oprs.TryGetValue(tbaKey, out var o) ? o : null;
            double? dpr = oprData.Dprs.TryGetValue(tbaKey, out var d) ? d : null;
            double? ccwm = oprData.Ccwms.TryGetValue(tbaKey, out var c) ? c : null;

            double? autoOpr = null, teleopOpr = null, endgameOpr = null;
            if (componentData != null)
            {
                (autoOpr, teleopOpr, endgameOpr) = componentData.TeamPhaseOprs(tbaKey);
            }

            int? rank = null;
            var matchesPlayed = 0;
            double? qualAverage = null, avgMatchPoints = null;
            int wins = 0, losses = 0, ties = 0, dqCount = 0;
            long? qualPoints = null, elimPoints = null, awardPoints = null, alliancePoints = null, totalPoints = null;

            if (rankingsByTeamKey.TryGetValue(tbaKey, out var ranking))
            {
                rank = ranking.Rank;
                matchesPlayed = ranking.MatchesPlayed;
                qualAverage = ranking.EffectiveQualAverage();
                avgMatchPoints = ranking.EffectiveAvgMatchPoints();
                wins = ranking.Record.Wins;
                losses = ranking.Record.Losses;
                ties = ranking.Record.Ties;
                dqCount = ranking.Dq;
                qualPoints = ranking.EffectiveQualPoints();
                elimPoints = ranking.EffectiveElimPoints();
                awardPoints = ranking.EffectiveAwardPoints();
                alliancePoints = ranking.EffectiveAlliancePoints();
                totalPoints = ranking.EffectiveTotalPoints();
            }

            try
            {
                await conn.ExecuteAsync("""
                    INSERT INTO team_event_stats (team_id, event_id, opr, dpr, ccwm, auto_opr, teleop_opr, endgame_opr,
                        rank, matches_played, qual_average, avg_match_points, wins, losses, ties, dq_count,
                        qual_points, elim_points, award_points, alliance_points, total_points)
                    VALUES (@teamId, @eventId, @opr, @dpr, @ccwm, @autoOpr, @teleopOpr, @endgameOpr,
                        @rank, @matchesPlayed, @qualAverage, @avgMatchPoints, @wins, @losses, @ties, @dqCount,
                        @qualPoints, @elimPoints, @awardPoints, @alliancePoints, @totalPoints)
                    ON CONFLICT (team_id, event_id) DO UPDATE SET
                        opr = EXCLUDED.opr, dpr = EXCLUDED.dpr, ccwm = EXCLUDED.ccwm,
                        auto_opr = EXCLUDED.auto_opr, teleop_opr = EXCLUDED.teleop_opr, endgame_opr = EXCLUDED.endgame_opr,
                        rank = EXCLUDED.rank, matches_played = EXCLUDED.matches_played,
                        qual_average = EXCLUDED.qual_average, avg_match_points = EXCLUDED.avg_match_points,
                        wins = EXCLUDED.wins, losses = EXCLUDED.losses, ties = EXCLUDED.ties, dq_count = EXCLUDED.dq_count,
                        qual_points = EXCLUDED.qual_points, elim_points = EXCLUDED.elim_points,
                        award_points = EXCLUDED.award_points, alliance_points = EXCLUDED.alliance_points,
                        total_points = EXCLUDED.total_points, updated_at = CURRENT_TIMESTAMP
                    """, new
                {
                    teamId = et.TeamId,
                    eventId,
                    opr,
                    dpr,
                    ccwm,
                    autoOpr,
                    teleopOpr,
                    endgameOpr,
                    rank,
                    matchesPlayed,
                    qualAverage,
                    avgMatchPoints,
                    wins,
                    losses,
                    ties,
                    dqCount,
                    qualPoints,
                    elimPoints,
                    awardPoints,
                    alliancePoints,
                    totalPoints,
                });
                statsUpdated++;
            }
            catch (Exception ex)
            {
                logger.LogWarning(ex, "failed to upsert team stats (team {Team}, event {Event})", et.TeamId, eventId);
            }
        }

        logger.LogInformation("synced TBA stats for {Count} teams at event {Event}", statsUpdated, eventId);
    }

    public async Task SyncEventMatchesAsync(TbaClient tbaClient, int eventId, string eventTbaKey, CancellationToken ct)
    {
        List<MatchInfo> matches;
        try
        {
            matches = await tbaClient.GetEventMatchesAsync(eventTbaKey, ct);
        }
        catch (Exception ex)
        {
            logger.LogWarning(ex, "failed to fetch matches for event {Event}", eventTbaKey);
            throw;
        }

        await using var conn = await db.OpenAsync(ct);
        foreach (var match in matches)
        {
            var compLevel = match.CompLevel.Trim().ToLowerInvariant();
            if (compLevel == "") compLevel = "qm";

            var matchNumber = NormalizeMatchNumber(compLevel, match.SetNumber, match.MatchNumber);
            var winningAlliance = "";
            if (match.Alliances.Red.Score >= 0 && match.Alliances.Blue.Score >= 0)
            {
                if (match.Alliances.Red.Score > match.Alliances.Blue.Score) winningAlliance = "red";
                else if (match.Alliances.Blue.Score > match.Alliances.Red.Score) winningAlliance = "blue";
            }

            var played = match.ActualTime > 0 ||
                (match.ScoreBreakdown is { ValueKind: not System.Text.Json.JsonValueKind.Null } &&
                 match.Alliances.Red.Score >= 0 && match.Alliances.Blue.Score >= 0);

            // Parse alliance team numbers from TBA keys ("frc1234" → 1234)
            var red1 = ParseTbaTeamNumber(match.Alliances.Red.Teams, 0);
            var red2 = ParseTbaTeamNumber(match.Alliances.Red.Teams, 1);
            var red3 = ParseTbaTeamNumber(match.Alliances.Red.Teams, 2);
            var blue1 = ParseTbaTeamNumber(match.Alliances.Blue.Teams, 0);
            var blue2 = ParseTbaTeamNumber(match.Alliances.Blue.Teams, 1);
            var blue3 = ParseTbaTeamNumber(match.Alliances.Blue.Teams, 2);

            try
            {
                await conn.ExecuteAsync("""
                    INSERT INTO matches (event_id, match_number, match_type, red_score, blue_score, played,
                        tba_key, comp_level, set_number, scheduled_time, actual_time, winning_alliance,
                        red1, red2, red3, blue1, blue2, blue3)
                    VALUES (@eventId, @matchNumber, @matchType, @redScore, @blueScore, @played,
                        @tbaKey, @compLevel, @setNumber, @scheduledTime, @actualTime, @winningAlliance,
                        @red1, @red2, @red3, @blue1, @blue2, @blue3)
                    ON CONFLICT (event_id, match_number, match_type) DO UPDATE SET
                        red_score = EXCLUDED.red_score, blue_score = EXCLUDED.blue_score, played = EXCLUDED.played,
                        tba_key = EXCLUDED.tba_key, comp_level = EXCLUDED.comp_level, set_number = EXCLUDED.set_number,
                        scheduled_time = EXCLUDED.scheduled_time, actual_time = EXCLUDED.actual_time,
                        winning_alliance = EXCLUDED.winning_alliance,
                        red1 = EXCLUDED.red1, red2 = EXCLUDED.red2, red3 = EXCLUDED.red3,
                        blue1 = EXCLUDED.blue1, blue2 = EXCLUDED.blue2, blue3 = EXCLUDED.blue3,
                        updated_at = CURRENT_TIMESTAMP
                    """, new
                {
                    eventId,
                    matchNumber,
                    matchType = compLevel,
                    redScore = match.Alliances.Red.Score,
                    blueScore = match.Alliances.Blue.Score,
                    played,
                    tbaKey = match.Key,
                    compLevel,
                    setNumber = match.SetNumber,
                    scheduledTime = UnixToUtc(match.ScheduledTime),
                    actualTime = UnixToUtc(match.ActualTime),
                    winningAlliance,
                    red1, red2, red3, blue1, blue2, blue3,
                });
            }
            catch (Exception ex)
            {
                logger.LogWarning(ex, "failed to upsert match {Key} for event {Event}", match.Key, eventId);
            }
        }

        logger.LogInformation("synced {Count} matches for event {Event}", matches.Count, eventId);
    }

    public static string NormalizeTbaEventKey(string raw, int season)
    {
        var key = raw.Trim().ToLowerInvariant();
        if (key == "") return "";
        var seasonPrefix = season.ToString();
        return key.StartsWith(seasonPrefix) ? key : seasonPrefix + key;
    }

    private static int NormalizeMatchNumber(string compLevel, int setNumber, int matchNumber)
        => compLevel == "qm" || setNumber <= 0 ? matchNumber : setNumber * 100 + matchNumber;

    private static DateTime? UnixToUtc(long ts)
        => ts <= 0 ? null : DateTimeOffset.FromUnixTimeSeconds(ts).UtcDateTime;

    private static int? ParseTbaTeamNumber(List<string> teams, int index)
    {
        if (index >= teams.Count) return null;
        var key = teams[index].Trim();
        if (key.StartsWith("frc", StringComparison.OrdinalIgnoreCase) &&
            int.TryParse(key[3..], out var n)) return n;
        return null;
    }
}
