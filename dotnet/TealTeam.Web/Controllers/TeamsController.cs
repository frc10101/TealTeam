using Dapper;
using Microsoft.AspNetCore.Mvc;
using TealTeam.Web.Data;
using TealTeam.Web.Models;
using TealTeam.Web.Services;

namespace TealTeam.Web.Controllers;

/// <summary>Team lookup page and HTMX fragments. Port of internal/handlers/team.go.</summary>
public class TeamsController(Db db, SessionService sessions, FirstSyncService firstSync, ILogger<TeamsController> logger)
    : AppController(db, sessions)
{
    [HttpGet("/teams")]
    public async Task<IActionResult> TeamPage([FromQuery] string? team)
    {
        var user = await GetSessionUserAsync();

        ViewData["Title"] = "Teams";
        ViewData["User"] = user;

        if (!string.IsNullOrEmpty(team))
        {
            ViewData["TeamSearchValue"] = team;
            var (_, errMsg) = await HydrateTeamLookupDataAsync(user, team);
            if (errMsg != "")
            {
                ViewData["TeamError"] = errMsg;
            }
        }

        await HydrateEventSelectionDataAsync(user, HttpContext.RequestAborted);
        return Page("Team");
    }

    [HttpGet("/hx/teams/search")]
    public async Task<IActionResult> TeamSearch([FromQuery] string? team)
    {
        var user = await GetSessionUserAsync();
        ViewData["User"] = user;

        var (statusCode, errMsg) = await HydrateTeamLookupDataAsync(user, team ?? "");
        if (errMsg != "")
        {
            return StatusCode(statusCode,
                $"""<div id="team-info-container"><div class="card"><div class="card-body text-center text-red-400 py-8">{System.Net.WebUtility.HtmlEncode(errMsg)}</div></div></div>""");
        }

        return Fragment("_TeamInfo");
    }

    private async Task<(int StatusCode, string Error)> HydrateTeamLookupDataAsync(User? user, string teamNumberRaw)
    {
        if (teamNumberRaw == "")
        {
            return (400, "Team number is required");
        }

        if (!int.TryParse(teamNumberRaw, out var teamNumber))
        {
            return (400, "Invalid team number");
        }

        Team? team;
        await using (var conn = await Db.OpenAsync(HttpContext.RequestAborted))
        {
            team = await conn.QuerySingleOrDefaultAsync<Team>(
                "SELECT * FROM teams WHERE team_number = @teamNumber LIMIT 1", new { teamNumber });
        }

        if (team == null)
        {
            return (404, $"Team {teamNumber} not found");
        }

        ViewData["User"] = user;
        ViewData["Team"] = team;
        ViewData["TeamNumber"] = team.TeamNumber;
        ViewData["TeamName"] = team.Name;

        await HydrateTeamEventsDataAsync(teamNumber);
        return (200, "");
    }

    private async Task HydrateTeamEventsDataAsync(int teamNumber)
    {
        var ct = HttpContext.RequestAborted;
        var eventIds = await GetEventsForTeamAsync(teamNumber, ct);
        if (eventIds.Count == 0)
        {
            // No events locally; try syncing from the FIRST API.
            try
            {
                await firstSync.SyncTeamForUserAsync(teamNumber, ct);
                eventIds = await GetEventsForTeamAsync(teamNumber, ct);
            }
            catch (Exception ex)
            {
                logger.LogWarning(ex, "failed to sync events for team {Team}", teamNumber);
                ViewData["Events"] = new List<EventOption>();
                return;
            }
        }

        if (eventIds.Count > 0)
        {
            await using var conn = await Db.OpenAsync(ct);
            ViewData["Events"] = (await conn.QueryAsync<EventOption>(
                "SELECT id, name FROM events WHERE id = ANY(@eventIds) ORDER BY start_date",
                new { eventIds })).ToList();
        }
    }

    /// <summary>Syncs a team's past events from the FIRST API, then re-renders the team info fragment.</summary>
    [HttpPost("/hx/teams/fetch-past-events")]
    public async Task<IActionResult> FetchPastEvents()
    {
        var user = await GetSessionUserAsync();
        var teamNumberRaw = ((string?)Request.Form["team"] ?? "").Trim();
        if (teamNumberRaw == "")
        {
            return BadRequest("Team number is required");
        }
        if (!int.TryParse(teamNumberRaw, out var teamNumber))
        {
            return BadRequest("Invalid team number");
        }

        try
        {
            await firstSync.SyncTeamForUserAsync(teamNumber, HttpContext.RequestAborted);
        }
        catch (Exception ex)
        {
            logger.LogWarning(ex, "failed to fetch past events for team {Team}", teamNumber);
        }

        ViewData["User"] = user;
        var (_, errMsg) = await HydrateTeamLookupDataAsync(user, teamNumberRaw);
        if (errMsg != "")
        {
            return StatusCode(500,
                $"""<div id="team-info-container"><div class="card"><div class="card-body text-center text-red-400 py-8">{System.Net.WebUtility.HtmlEncode(errMsg)}</div></div></div>""");
        }

        return Fragment("_TeamInfo");
    }

    [HttpGet("/hx/teams/data")]
    public async Task<IActionResult> TeamEventData([FromQuery] string? team, [FromQuery(Name = "event_id")] string? eventIdRaw)
    {
        if (string.IsNullOrEmpty(team))
        {
            return BadRequest(new { error = "Team number is required" });
        }
        if (!int.TryParse(team, out var teamNumber))
        {
            return BadRequest(new { error = "Invalid team number" });
        }
        if (string.IsNullOrEmpty(eventIdRaw))
        {
            return BadRequest(new { error = "Event ID is required" });
        }
        if (!int.TryParse(eventIdRaw, out var eventId))
        {
            return BadRequest(new { error = "Invalid event ID" });
        }

        var ct = HttpContext.RequestAborted;
        Team? teamRecord;
        var viewerTeamId = 0;

        await using (var conn = await Db.OpenAsync(ct))
        {
            teamRecord = await conn.QuerySingleOrDefaultAsync<Team>(
                "SELECT * FROM teams WHERE team_number = @teamNumber LIMIT 1", new { teamNumber });
            if (teamRecord == null)
            {
                return NotFound(new { error = "Team not found" });
            }

            // Resolve the viewer's team ID for notes filtering.
            var user = await GetSessionUserAsync();
            if (user?.TeamNumber != null)
            {
                viewerTeamId = await conn.ExecuteScalarAsync<int?>(
                    "SELECT id FROM teams WHERE team_number = @teamNumber LIMIT 1",
                    new { teamNumber = user.TeamNumber.Value }) ?? 0;
            }
        }

        ViewData["TeamNumber"] = teamRecord.TeamNumber;
        ViewData["TeamName"] = teamRecord.Name;

        await HydrateTeamEventDataAsync(teamRecord.Id, eventId, viewerTeamId, ct);
        return Fragment("_TeamData");
    }

    /// <summary>
    /// Team performance data for an event. Notes are filtered to the viewer's
    /// own team (submitting_team_id); pass viewerTeamId 0 to hide all notes.
    /// </summary>
    private async Task HydrateTeamEventDataAsync(int teamId, int eventId, int viewerTeamId, CancellationToken ct)
    {
        await using var conn = await Db.OpenAsync(ct);

        var eventName = await conn.ExecuteScalarAsync<string?>(
            "SELECT name FROM events WHERE id = @eventId", new { eventId });
        if (eventName != null)
        {
            ViewData["EventName"] = eventName;
        }

        var stats = await conn.QuerySingleOrDefaultAsync<TeamEventStats>(
            "SELECT * FROM team_event_stats WHERE team_id = @teamId AND event_id = @eventId LIMIT 1",
            new { teamId, eventId });
        if (stats != null)
        {
            ViewData["TeamStats"] = stats;
        }

        var scoutingData = (await conn.QueryAsync<ScoutingData>(
            "SELECT * FROM scouting_data WHERE team_id = @teamId AND event_id = @eventId ORDER BY scouted_at DESC",
            new { teamId, eventId })).ToList();

        ViewData["ScoutingData"] = scoutingData;
        ViewData["ScoutingCount"] = scoutingData.Count;

        if (scoutingData.Count == 0)
        {
            return;
        }

        var allianceColors = new Dictionary<string, int>();
        var autoHangs = new Dictionary<string, int>();

        ViewData["MostCommonStartPos"] = MostCommon(scoutingData, sd => sd.StartingPosition);
        ViewData["MostCommonDefense"] = MostCommon(scoutingData, sd => sd.DefenseRating);
        ViewData["MostCommonTraversal"] = MostCommon(scoutingData, sd => sd.Traversal);
        ViewData["MostCommonScoringStrategy"] = MostCommon(scoutingData, sd => sd.ScoringStrategy);
        ViewData["MostCommonHangLevel"] = MostCommon(scoutingData, sd => sd.HangLevel);
        ViewData["MostCommonHangPosition"] = MostCommon(scoutingData, sd => sd.HangPosition);
        ViewData["MostCommonAccuracy"] = MostCommon(scoutingData, sd => sd.AccuracyRating);

        foreach (var sd in scoutingData)
        {
            allianceColors[sd.AllianceColor] = allianceColors.GetValueOrDefault(sd.AllianceColor) + 1;
            if (!string.IsNullOrEmpty(sd.AutoHang))
            {
                autoHangs[sd.AutoHang!] = autoHangs.GetValueOrDefault(sd.AutoHang!) + 1;
            }
        }

        ViewData["AllianceColorStats"] = allianceColors;
        ViewData["AutoHangStats"] = autoHangs;

        // Latest qualitative data.
        var latest = scoutingData[0];
        ViewData["ShootingSpeed"] = latest.ShootingSpeed ?? "";
        ViewData["Capacity"] = latest.Capacity ?? "";
        ViewData["Defendability"] = latest.Defendability ?? "";
        ViewData["ScoringStrategy"] = latest.ScoringStrategy ?? "";

        // Notes only from the viewer's own team.
        var notes = new List<NoteEntry>();
        for (var i = 0; i < scoutingData.Count; i++)
        {
            var sd = scoutingData[i];
            if (!string.IsNullOrEmpty(sd.Notes) && viewerTeamId > 0 && sd.SubmittingTeamId == viewerTeamId)
            {
                notes.Add(new NoteEntry { Note = sd.Notes!, ScoutedAt = sd.ScoutedAt, MatchIndex = i + 1 });
            }
        }
        ViewData["AllNotes"] = notes;
        if (notes.Count > 0)
        {
            ViewData["LatestNotes"] = notes[0].Note;
        }

        static string MostCommon(List<ScoutingData> data, Func<ScoutingData, string?> selector)
        {
            var counts = new Dictionary<string, int>();
            foreach (var sd in data)
            {
                var value = selector(sd);
                if (!string.IsNullOrEmpty(value))
                {
                    counts[value!] = counts.GetValueOrDefault(value!) + 1;
                }
            }

            var best = "";
            var maxCount = 0;
            foreach (var (value, count) in counts)
            {
                if (count > maxCount)
                {
                    maxCount = count;
                    best = value;
                }
            }

            return best;
        }
    }
}
