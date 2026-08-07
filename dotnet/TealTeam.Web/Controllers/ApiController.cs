using Dapper;
using Microsoft.AspNetCore.Mvc;
using TealTeam.Web.Data;
using TealTeam.Web.Models;
using TealTeam.Web.Services;

namespace TealTeam.Web.Controllers;

/// <summary>
/// JSON APIs: manual FIRST sync, pick list persistence, and network status.
/// Ports of internal/handlers/frc_sync.go, the pick list handlers in
/// lead_scout.go, and network_status.go.
/// </summary>
public class ApiController(Db db, SessionService sessions, FirstSyncService firstSync, ILogger<ApiController> logger)
    : AppController(db, sessions)
{
    [HttpPost("/api/frc/sync")]
    public async Task<IActionResult> FrcSync()
    {
        var user = await GetSessionUserAsync();
        if (user == null || (!user.IsAdmin && !user.IsLeadScout))
        {
            return Unauthorized("Unauthorized");
        }

        try
        {
            using var cts = CancellationTokenSource.CreateLinkedTokenSource(HttpContext.RequestAborted);
            cts.CancelAfter(TimeSpan.FromSeconds(90));
            var result = await firstSync.SyncNowAsync(cts.Token);
            logger.LogInformation("FRC sync completed: events={Events} teams={Teams} event_teams={EventTeams}",
                result.Events, result.Teams, result.EventTeams);
            return Json(new
            {
                season = result.Season,
                events = result.Events,
                teams = result.Teams,
                eventTeams = result.EventTeams,
            });
        }
        catch (SyncSkippedException)
        {
            return BadRequest("FIRST API credentials missing");
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "FRC sync failed");
            return StatusCode(500, ex.Message);
        }
    }

    public class PickListEntryRequest
    {
        [System.Text.Json.Serialization.JsonPropertyName("picked_team_number")]
        public int PickedTeamNumber { get; set; }

        [System.Text.Json.Serialization.JsonPropertyName("color")]
        public string? Color { get; set; }

        [System.Text.Json.Serialization.JsonPropertyName("crossed")]
        public bool Crossed { get; set; }

        [System.Text.Json.Serialization.JsonPropertyName("position")]
        public int Position { get; set; }
    }

    private async Task<(User User, Session Session)?> RequirePickListContextAsync()
    {
        var user = await GetSessionUserAsync();
        if (user == null)
        {
            return null;
        }

        var session = await GetSessionAsync();
        if (session == null)
        {
            return null;
        }

        return (user, session);
    }

    [HttpGet("/api/pick-list")]
    public async Task<IActionResult> GetPickList()
    {
        var context = await RequirePickListContextAsync();
        if (context == null)
        {
            return Unauthorized(new { error = "not authenticated" });
        }

        var (user, session) = context.Value;
        if (user.TeamNumber is null or 0)
        {
            return BadRequest(new { error = "user has no team assigned" });
        }
        if (session.SelectedEventId == null)
        {
            return BadRequest(new { error = "no event selected" });
        }

        try
        {
            await using var conn = await Db.OpenAsync(HttpContext.RequestAborted);
            var entries = (await conn.QueryAsync<PickListEntry>("""
                SELECT * FROM pick_list_entries
                WHERE team_number = @teamNumber AND event_id = @eventId
                ORDER BY position
                """, new { teamNumber = user.TeamNumber.Value, eventId = session.SelectedEventId.Value })).ToList();

            return Json(new
            {
                entries = entries.Select(e => new
                {
                    picked_team_number = e.PickedTeamNumber,
                    color = e.Color,
                    crossed = e.Crossed,
                    position = e.Position,
                }),
            });
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to load pick list");
            return StatusCode(500, new { error = "failed to load pick list" });
        }
    }

    [HttpPost("/api/pick-list/entry")]
    public async Task<IActionResult> SavePickListEntry([FromBody] PickListEntryRequest? request)
    {
        var context = await RequirePickListContextAsync();
        if (context == null)
        {
            return Unauthorized(new { error = "not authenticated" });
        }

        var (user, session) = context.Value;
        if (user.TeamNumber is null or 0)
        {
            return BadRequest(new { error = "user has no team assigned" });
        }
        if (session.SelectedEventId == null)
        {
            return BadRequest(new { error = "no event selected" });
        }
        if (request == null || request.PickedTeamNumber == 0)
        {
            return BadRequest(new { error = "invalid request" });
        }

        try
        {
            await using var conn = await Db.OpenAsync(HttpContext.RequestAborted);
            await conn.ExecuteAsync("""
                INSERT INTO pick_list_entries (team_number, event_id, picked_team_number, color, crossed, position)
                VALUES (@teamNumber, @eventId, @pickedTeamNumber, @color, @crossed, @position)
                ON CONFLICT (team_number, event_id, picked_team_number) DO UPDATE SET
                    color = EXCLUDED.color, crossed = EXCLUDED.crossed, position = EXCLUDED.position,
                    updated_at = CURRENT_TIMESTAMP
                """, new
            {
                teamNumber = user.TeamNumber.Value,
                eventId = session.SelectedEventId.Value,
                pickedTeamNumber = request.PickedTeamNumber,
                color = request.Color,
                crossed = request.Crossed,
                position = request.Position,
            });
            return Json(new { status = "saved" });
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to save pick list entry");
            return StatusCode(500, new { error = "failed to save entry" });
        }
    }

    [HttpDelete("/api/pick-list/entry")]
    public async Task<IActionResult> DeletePickListEntry([FromQuery] string? team)
    {
        var context = await RequirePickListContextAsync();
        if (context == null)
        {
            return Unauthorized(new { error = "not authenticated" });
        }

        var (user, session) = context.Value;
        if (user.TeamNumber is null or 0)
        {
            return BadRequest(new { error = "user has no team assigned" });
        }
        if (session.SelectedEventId == null)
        {
            return BadRequest(new { error = "no event selected" });
        }
        if (string.IsNullOrEmpty(team))
        {
            return BadRequest(new { error = "team number required" });
        }
        if (!int.TryParse(team, out var pickedTeamNumber))
        {
            return BadRequest(new { error = "invalid team number" });
        }

        try
        {
            await using var conn = await Db.OpenAsync(HttpContext.RequestAborted);
            await conn.ExecuteAsync("""
                DELETE FROM pick_list_entries
                WHERE team_number = @teamNumber AND event_id = @eventId AND picked_team_number = @pickedTeamNumber
                """, new
            {
                teamNumber = user.TeamNumber.Value,
                eventId = session.SelectedEventId.Value,
                pickedTeamNumber,
            });
            return Json(new { status = "deleted" });
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to delete pick list entry");
            return StatusCode(500, new { error = "failed to delete entry" });
        }
    }

    /// <summary>HTMX badge polled by the submission panel's network status container.</summary>
    [HttpGet("/hx/network/status")]
    public async Task<IActionResult> NetworkStatusBadge()
    {
        using var cts = CancellationTokenSource.CreateLinkedTokenSource(HttpContext.RequestAborted);
        cts.CancelAfter(TimeSpan.FromSeconds(2));
        await Connectivity.RefreshAsync(cts.Token);

        var snapshot = Connectivity.GetSnapshot();
        var status = ClassifyNetworkState(snapshot);
        var (label, cssClass) = status switch
        {
            "internet-ok" => ("Internet OK", "bg-teal-900/40 text-teal-200 border-teal-600"),
            "api-error" => ("API Error", "bg-amber-900/40 text-amber-200 border-amber-600"),
            _ => ("Offline", "bg-red-900/40 text-red-200 border-red-600"),
        };

        ViewData["NetworkStatus"] = status;
        ViewData["NetworkStatusLabel"] = label;
        ViewData["NetworkStatusClass"] = cssClass;
        ViewData["LastSyncText"] = FormatStatusTime(snapshot.LastSuccessfulSync);
        ViewData["LastAPISuccessText"] = FormatStatusTime(snapshot.LastApiSuccessAt);
        ViewData["LastAPIErrorText"] = FormatStatusTime(snapshot.LastApiErrorAt);
        ViewData["LastAPIError"] = snapshot.LastApiError;
        ViewData["InternetError"] = snapshot.InternetError;
        return Fragment("_NetworkStatusBadge");
    }

    private static string FormatStatusTime(DateTime t)
        => t == default
            ? "Never"
            : t.ToLocalTime().ToString("MMM d, h:mm:ss tt", System.Globalization.CultureInfo.InvariantCulture);

    [HttpGet("/api/network/status")]
    public async Task<IActionResult> NetworkStatus()
    {
        using var cts = CancellationTokenSource.CreateLinkedTokenSource(HttpContext.RequestAborted);
        cts.CancelAfter(TimeSpan.FromSeconds(2));
        await Connectivity.RefreshAsync(cts.Token);

        var snapshot = Connectivity.GetSnapshot();
        return Json(new { status = ClassifyNetworkState(snapshot), data = snapshot });
    }

    private static readonly TimeSpan RecentApiSuccessWindow = TimeSpan.FromMinutes(10);

    private static string ClassifyNetworkState(NetworkStatusSnapshot s)
    {
        if (s.LastApiSuccessAt != default && DateTime.UtcNow - s.LastApiSuccessAt <= RecentApiSuccessWindow)
        {
            if (s.LastApiErrorAt == default || s.LastApiSuccessAt > s.LastApiErrorAt)
            {
                return "internet-ok";
            }
        }

        if (!s.InternetReachable)
        {
            return "offline";
        }
        if (s.LastApiErrorAt != default && (s.LastApiSuccessAt == default || s.LastApiErrorAt > s.LastApiSuccessAt))
        {
            return "api-error";
        }
        return "internet-ok";
    }
}
