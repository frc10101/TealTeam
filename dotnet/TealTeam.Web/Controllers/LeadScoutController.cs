using Dapper;
using Microsoft.AspNetCore.Mvc;
using TealTeam.Web.Data;
using TealTeam.Web.Models;
using TealTeam.Web.Services;

namespace TealTeam.Web.Controllers;

/// <summary>
/// Lead scout panel: pending submission review, team rankings, pick list page,
/// and point weight settings. Port of internal/handlers/lead_scout.go and
/// lead_scout_weights.go.
/// </summary>
public class LeadScoutController(
    Db db,
    SessionService sessions,
    FirstSyncService firstSync,
    ScoutingPointsService scoutingPoints,
    ILogger<LeadScoutController> logger)
    : AppController(db, sessions)
{
    [HttpGet("/lead-scout")]
    public async Task<IActionResult> AdminViewer([FromQuery(Name = "team_sort")] string? teamSort)
    {
        var user = await GetSessionUserAsync();
        if (user == null)
        {
            return Redirect("/sign-in");
        }
        if (!user.IsAdmin && !user.IsLeadScout)
        {
            return Redirect("/");
        }

        ViewData["Title"] = "Lead Scout Panel";
        ViewData["Description"] = "Approve submissions, review rankings, and coordinate match strategy.";
        ViewData["User"] = user;

        var ct = HttpContext.RequestAborted;
        try
        {
            var session = await GetSessionAsync();
            if (session?.SelectedEventId != null)
            {
                var eventId = session.SelectedEventId.Value;
                ViewData["SelectedEventID"] = eventId;
                ViewData["TeamRankingSort"] = teamSort ?? "";
                ViewData["TeamRankings"] = await LoadTeamPointRankingsAsync(eventId, teamSort ?? "", ct);
                ViewData["PickListTeams"] = await LoadPickListTeamsAsync(eventId, ct);
            }

            ViewData["PendingSubmissions"] = await LoadPendingSubmissionsAsync(ct);
        }
        catch (Exception ex)
        {
            logger.LogWarning(ex, "failed to load lead scout panel data");
        }

        return Page("AdminViewer");
    }

    private async Task<List<PendingSubmissionRow>> LoadPendingSubmissionsAsync(CancellationToken ct)
    {
        await using var conn = await Db.OpenAsync(ct);
        var rows = await conn.QueryAsync<(int Id, string EventName, int TeamNumber, string TeamName, string? ScoutName, string? Notes)>("""
            SELECT scouting_submissions.id, events.name AS event_name, teams.team_number,
                   teams.name AS team_name, users.name AS scout_name, scouting_submissions.notes
            FROM scouting_submissions
            JOIN events ON events.id = scouting_submissions.event_id
            JOIN teams ON teams.id = scouting_submissions.team_id
            LEFT JOIN users ON users.id = scouting_submissions.scouter_id
            ORDER BY scouting_submissions.created_at
            """);

        return rows.Select(row => new PendingSubmissionRow
        {
            Id = row.Id,
            EventName = row.EventName,
            TeamNumber = row.TeamNumber,
            TeamName = row.TeamName,
            ScoutName = string.IsNullOrWhiteSpace(row.ScoutName) ? "Unknown" : row.ScoutName,
            FlagLabel = string.IsNullOrWhiteSpace(row.Notes) ? "Missing notes" : "Clean",
            FlagClass = string.IsNullOrWhiteSpace(row.Notes) ? "text-yellow-300" : "text-teal-300",
        }).ToList();
    }

    private async Task<List<PickListTeam>> LoadPickListTeamsAsync(int eventId, CancellationToken ct)
    {
        await using var conn = await Db.OpenAsync(ct);
        return (await conn.QueryAsync<PickListTeam>("""
            SELECT teams.team_number, teams.name AS team_name, team_event_stats.rank
            FROM teams
            JOIN event_teams ON event_teams.team_id = teams.id
            LEFT JOIN team_event_stats ON team_event_stats.team_id = teams.id
                AND team_event_stats.event_id = event_teams.event_id
            WHERE event_teams.event_id = @eventId
            ORDER BY team_number
            """, new { eventId })).ToList();
    }

    private async Task<List<TeamPointSummary>> LoadTeamPointRankingsAsync(int eventId, string sortKey, CancellationToken ct)
    {
        await using var conn = await Db.OpenAsync(ct);
        var teams = (await conn.QueryAsync<(int TeamId, int TeamNumber, string TeamName, int? Rank)>("""
            SELECT teams.id AS team_id, teams.team_number, teams.name AS team_name, team_event_stats.rank
            FROM teams
            JOIN event_teams ON event_teams.team_id = teams.id
            LEFT JOIN team_event_stats ON team_event_stats.team_id = teams.id
                AND team_event_stats.event_id = event_teams.event_id
            WHERE event_teams.event_id = @eventId
            ORDER BY teams.team_number
            """, new { eventId })).ToList();

        var metrics = (await conn.QueryAsync<ScoutingMetricRow>("""
            SELECT scouting_data.team_id,
                   COALESCE(scouting_data.defense_rating, '') AS defense_rating,
                   COALESCE(scouting_data.traversal, '') AS traversal,
                   COALESCE(scouting_data.shooting_speed, '') AS shooting_speed,
                   COALESCE(scouting_data.capacity, '') AS capacity,
                   COALESCE(scouting_data.scoring_strategy, '') AS scoring_strategy,
                   COALESCE(scouting_data.hang_level, '') AS hang_level,
                   COALESCE(scouting_data.auto_hang, '') AS auto_hang,
                   COALESCE(scouting_data.hang_position, '') AS hang_position,
                   COALESCE(scouting_data.starting_position, '') AS starting_position
            FROM scouting_data
            WHERE scouting_data.event_id = @eventId
            """, new { eventId })).ToList();

        var cfg = await scoutingPoints.LoadEffectiveConfigAsync(ct);
        var pointsByTeam = new Dictionary<int, int>();
        var matchesByTeam = new Dictionary<int, int>();
        foreach (var row in metrics)
        {
            pointsByTeam[row.TeamId] = pointsByTeam.GetValueOrDefault(row.TeamId)
                + ScoutingPointsService.CalculateScoutingPoints(row, cfg);
            matchesByTeam[row.TeamId] = matchesByTeam.GetValueOrDefault(row.TeamId) + 1;
        }

        var summaries = teams.Select(t => new TeamPointSummary
        {
            TeamId = t.TeamId,
            TeamNumber = t.TeamNumber,
            TeamName = t.TeamName,
            Rank = t.Rank,
            Points = pointsByTeam.GetValueOrDefault(t.TeamId),
            Matches = matchesByTeam.GetValueOrDefault(t.TeamId),
        }).ToList();

        sortKey = sortKey.Trim().ToLowerInvariant();
        if (sortKey == "") sortKey = "rank";

        summaries.Sort((left, right) =>
        {
            switch (sortKey)
            {
                case "points":
                    if (left.Points != right.Points) return right.Points.CompareTo(left.Points);
                    break;
                case "name":
                    var cmp = string.Compare(left.TeamName.Trim(), right.TeamName.Trim(),
                        StringComparison.OrdinalIgnoreCase);
                    if (cmp != 0) return cmp;
                    break;
                case "number":
                    if (left.TeamNumber != right.TeamNumber) return left.TeamNumber.CompareTo(right.TeamNumber);
                    break;
                default:
                    if (left.Rank != null || right.Rank != null)
                    {
                        if (left.Rank == null) return 1;
                        if (right.Rank == null) return -1;
                        if (left.Rank.Value != right.Rank.Value) return left.Rank.Value.CompareTo(right.Rank.Value);
                    }
                    break;
            }

            if (left.TeamNumber != right.TeamNumber) return left.TeamNumber.CompareTo(right.TeamNumber);
            return string.Compare(left.TeamName, right.TeamName, StringComparison.Ordinal);
        });

        return summaries;
    }

    [HttpGet("/lead-scout/submissions/{id:int}")]
    public async Task<IActionResult> ViewSubmission(int id)
    {
        var user = await GetSessionUserAsync();
        if (user == null || (!user.IsAdmin && !user.IsLeadScout))
        {
            return Redirect("/");
        }

        ViewData["Title"] = "Submission Details";
        ViewData["User"] = user;

        SubmissionDetailRow? detail;
        try
        {
            await using var conn = await Db.OpenAsync(HttpContext.RequestAborted);
            var row = await conn.QuerySingleOrDefaultAsync("""
                SELECT scouting_submissions.id,
                    events.name AS event_name,
                    teams.team_number,
                    teams.name AS team_name,
                    users.name AS scout_name,
                    COALESCE(scouting_submissions.alliance_color, '') AS alliance_color,
                    COALESCE(scouting_submissions.notes, '') AS notes,
                    COALESCE(scouting_submissions.starting_position, '') AS starting_position,
                    COALESCE(scouting_submissions.defense_rating, '') AS defense_rating,
                    COALESCE(scouting_submissions.traversal, '') AS traversal,
                    COALESCE(scouting_submissions.scoring_strategy, '') AS scoring_strategy,
                    COALESCE(scouting_submissions.shooting_speed, '') AS shooting_speed,
                    COALESCE(scouting_submissions.capacity, '') AS capacity,
                    COALESCE(scouting_submissions.defendability, '') AS defendability,
                    COALESCE(scouting_submissions.hang_level, '') AS hang_level,
                    COALESCE(scouting_submissions.auto_hang, '') AS auto_hang,
                    COALESCE(scouting_submissions.hang_position, '') AS hang_position,
                    TO_CHAR(scouting_submissions.created_at, 'YYYY-MM-DD HH24:MI:SS') AS created_at
                FROM scouting_submissions
                JOIN events ON events.id = scouting_submissions.event_id
                JOIN teams ON teams.id = scouting_submissions.team_id
                LEFT JOIN users ON users.id = scouting_submissions.scouter_id
                WHERE scouting_submissions.id = @id
                """, new { id });

            if (row == null)
            {
                return Page("SubmissionDetail");
            }

            string scoutName = string.IsNullOrWhiteSpace((string?)row.scout_name) ? "Unknown" : (string)row.scout_name;
            string notes = (string)row.notes;
            detail = new SubmissionDetailRow
            {
                Id = (int)row.id,
                EventName = (string)row.event_name,
                TeamNumber = (int)row.team_number,
                TeamName = (string)row.team_name,
                ScoutName = scoutName,
                AllianceColor = (string)row.alliance_color,
                Notes = notes,
                StartingPosition = (string)row.starting_position,
                DefenseRating = (string)row.defense_rating,
                Traversal = (string)row.traversal,
                ScoringStrategy = (string)row.scoring_strategy,
                ShootingSpeed = (string)row.shooting_speed,
                Capacity = (string)row.capacity,
                Defendability = (string)row.defendability,
                HangLevel = (string)row.hang_level,
                AutoHang = (string)row.auto_hang,
                HangPosition = (string)row.hang_position,
                FlagLabel = string.IsNullOrWhiteSpace(notes) ? "Missing notes" : "Clean",
                FlagClass = string.IsNullOrWhiteSpace(notes) ? "text-yellow-300" : "text-teal-300",
                CreatedAt = (string)row.created_at,
            };
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to load submission {Id}", id);
            return StatusCode(500, "Failed to load submission");
        }

        ViewData["Submission"] = detail;
        return Page("SubmissionDetail");
    }

    [HttpPost("/hx/lead-scout/submissions/{id:int}/approve")]
    public async Task<IActionResult> ApproveSubmission(int id)
    {
        var user = await GetSessionUserAsync();
        if (user == null || (!user.IsAdmin && !user.IsLeadScout))
        {
            return Redirect("/");
        }

        var ct = HttpContext.RequestAborted;
        int teamNumber;
        try
        {
            await using var conn = await Db.OpenAsync(ct);
            var submission = await conn.QuerySingleOrDefaultAsync<ScoutingSubmission>(
                "SELECT * FROM scouting_submissions WHERE id = @id", new { id });
            if (submission == null)
            {
                return NotFound("Submission not found");
            }

            var teamNumberNullable = await conn.ExecuteScalarAsync<int?>(
                "SELECT team_number FROM teams WHERE id = @teamId", new { teamId = submission.TeamId });
            if (teamNumberNullable == null)
            {
                return StatusCode(500, "Team not found");
            }
            teamNumber = teamNumberNullable.Value;

            // Carry team attribution into scouting_data (drives the notes
            // privacy filter on the team page). Resolve from the scouter for
            // legacy submissions that predate attribution.
            if (submission.SubmittingTeamId == null && submission.ScouterId != null)
            {
                submission.SubmittingTeamId = await conn.ExecuteScalarAsync<int?>("""
                    SELECT t.id FROM users u
                    JOIN teams t ON t.team_number = u.team_number
                    WHERE u.id = @scouterId
                    """, new { scouterId = submission.ScouterId.Value });
            }

            await using var tx = await conn.BeginTransactionAsync(ct);
            await conn.ExecuteAsync("""
                INSERT INTO scouting_data (event_id, team_id, alliance_color, notes, starting_position,
                    defense_rating, traversal, scoring_strategy, shooting_speed, capacity, defendability,
                    hang_level, auto_hang, hang_position, scouted_at, scouter_id, submitting_team_id)
                VALUES (@EventId, @TeamId, @AllianceColor, @Notes, @StartingPosition,
                    @DefenseRating, @Traversal, @ScoringStrategy, @ShootingSpeed, @Capacity, @Defendability,
                    @HangLevel, @AutoHang, @HangPosition, @ScoutedAt, @ScouterId, @SubmittingTeamId)
                """, submission, tx);
            await conn.ExecuteAsync("DELETE FROM scouting_submissions WHERE id = @id", new { id }, tx);
            await tx.CommitAsync(ct);
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to approve submission {Id}", id);
            return StatusCode(500, "Failed to approve submission");
        }

        // Refresh the team's info from FIRST in the background after approval.
        _ = Task.Run(async () =>
        {
            try
            {
                await firstSync.SyncTeamForUserAsync(teamNumber, CancellationToken.None);
            }
            catch (Exception ex)
            {
                logger.LogWarning(ex, "failed to sync team after approval (team {Team})", teamNumber);
            }
        });

        return UpNavigate("/lead-scout");
    }

    [HttpPost("/hx/lead-scout/submissions/{id:int}/decline")]
    public async Task<IActionResult> DeclineSubmission(int id)
    {
        var user = await GetSessionUserAsync();
        if (user == null || (!user.IsAdmin && !user.IsLeadScout))
        {
            return Redirect("/");
        }

        try
        {
            await using var conn = await Db.OpenAsync(HttpContext.RequestAborted);
            await conn.ExecuteAsync("DELETE FROM scouting_submissions WHERE id = @id", new { id });
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to decline submission {Id}", id);
            return StatusCode(500, "Failed to decline submission");
        }

        return UpNavigate("/lead-scout");
    }

    [HttpGet("/lead-scout/weights")]
    public async Task<IActionResult> WeightsPage([FromQuery] string? updated, [FromQuery] string? error)
    {
        var user = await GetSessionUserAsync();
        if (user == null)
        {
            return Redirect("/sign-in");
        }
        if (!user.IsAdmin && !user.IsLeadScout)
        {
            return Redirect("/");
        }

        var sections = await scoutingPoints.LoadSectionsAsync(HttpContext.RequestAborted);
        sections.Sort((a, b) => string.Compare(a.Label, b.Label, StringComparison.Ordinal));

        ViewData["Title"] = "Point Weight Settings";
        ViewData["Description"] = "Adjust category point values used for team ranking scores.";
        ViewData["User"] = user;
        ViewData["Sections"] = sections;
        ViewData["Updated"] = updated == "1";
        ViewData["Error"] = error;
        return Page("LeadScoutWeights");
    }

    [HttpPost("/lead-scout/weights")]
    public async Task<IActionResult> WeightsUpdate()
    {
        var user = await GetSessionUserAsync();
        if (user == null)
        {
            return Redirect("/sign-in");
        }
        if (!user.IsAdmin && !user.IsLeadScout)
        {
            return Redirect("/");
        }

        var baseSections = await scoutingPoints.LoadSectionsAsync(HttpContext.RequestAborted);
        var parsedSections = ScoutingPointsService.ParseSectionsFromForm(Request.Form, baseSections);
        if (parsedSections == null)
        {
            return Redirect("/lead-scout/weights?error=Invalid+weight+value");
        }

        try
        {
            await scoutingPoints.PersistSectionsAsync(parsedSections, HttpContext.RequestAborted);
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to persist scouting point weights");
            return Redirect("/lead-scout/weights?error=Failed+to+save+weights");
        }

        return Redirect("/lead-scout?team_sort=points");
    }
}
