using Dapper;
using Microsoft.AspNetCore.Mvc;
using TealTeam.Web.Data;
using TealTeam.Web.Models;
using TealTeam.Web.Services;

namespace TealTeam.Web.Controllers;

/// <summary>
/// Shared controller plumbing: session helpers, page/partial rendering, and
/// the event-selection/summary hydration used across pages
/// (port of internal/handlers/pages.go and match_utils.go helpers).
/// </summary>
public abstract class AppController(Db db, SessionService sessions) : Controller
{
    protected readonly Db Db = db;
    protected readonly SessionService Sessions = sessions;

    protected bool IsUnpoly => Request.Headers.ContainsKey("X-Up-Version");

    protected ViewResult Page(string name) => View($"~/Views/Pages/{name}.cshtml");

    protected PartialViewResult Fragment(string name) => PartialView($"~/Views/Partials/{name}.cshtml");

    protected Task<Session?> GetSessionAsync() => Sessions.GetSessionAsync(HttpContext);

    protected Task<User?> GetSessionUserAsync() => Sessions.GetSessionUserAsync(HttpContext);

    /// <summary>Events available to a user: their team's events, or all events for team-less users.</summary>
    protected async Task<List<int>> GetAvailableEventsForUserAsync(User user, CancellationToken ct)
    {
        if (user.TeamNumber is > 0)
        {
            return await GetEventsForTeamAsync(user.TeamNumber.Value, ct);
        }

        await using var conn = await Db.OpenAsync(ct);
        return (await conn.QueryAsync<int>("SELECT id FROM events ORDER BY start_date")).ToList();
    }

    protected async Task<List<int>> GetEventsForTeamAsync(int teamNumber, CancellationToken ct)
    {
        await using var conn = await Db.OpenAsync(ct);
        return (await conn.QueryAsync<int>("""
            SELECT DISTINCT event_teams.event_id
            FROM event_teams
            JOIN teams ON teams.id = event_teams.team_id
            WHERE teams.team_number = @teamNumber
            """, new { teamNumber })).ToList();
    }

    protected async Task HydrateEventSelectionDataAsync(User? user, CancellationToken ct)
    {
        if (user == null)
        {
            return;
        }

        var eventIds = await GetAvailableEventsForUserAsync(user, ct);
        if (eventIds.Count == 0)
        {
            return;
        }

        await using var conn = await Db.OpenAsync(ct);
        var events = (await conn.QueryAsync<EventOption>(
            "SELECT id, name FROM events WHERE id = ANY(@eventIds) ORDER BY start_date",
            new { eventIds })).ToList();
        ViewData["Events"] = events;

        var session = await GetSessionAsync();
        if (session?.SelectedEventId != null)
        {
            ViewData["SelectedEventID"] = session.SelectedEventId.Value;

            if (user.TeamNumber != null)
            {
                var teamMatchCount = await conn.ExecuteScalarAsync<long>("""
                    SELECT COUNT(*) FROM event_teams
                    JOIN teams ON teams.id = event_teams.team_id
                    WHERE event_teams.event_id = @eventId AND teams.team_number = @teamNumber
                    """, new { eventId = session.SelectedEventId.Value, teamNumber = user.TeamNumber.Value });
                if (teamMatchCount == 0)
                {
                    ViewData["EventWarning"] = "Your team is not listed for this event yet.";
                }
            }
        }
    }

    protected async Task HydrateEventSummaryDataAsync(int eventId, CancellationToken ct)
    {
        await using var conn = await Db.OpenAsync(ct);

        var eventName = await conn.ExecuteScalarAsync<string?>(
            "SELECT name FROM events WHERE id = @eventId", new { eventId });
        if (eventName != null)
        {
            ViewData["SelectedEventName"] = eventName;
            ViewData["SelectedEventID"] = eventId;
        }

        ViewData["EventTeamsCount"] = await conn.ExecuteScalarAsync<long>(
            "SELECT COUNT(*) FROM event_teams WHERE event_id = @eventId", new { eventId });

        ViewData["EventMatchesCount"] = await conn.ExecuteScalarAsync<long>(
            "SELECT COUNT(*) FROM matches WHERE event_id = @eventId", new { eventId });

        ViewData["EventTeams"] = (await conn.QueryAsync<EventTeamRow>("""
            SELECT teams.team_number, teams.name
            FROM event_teams
            JOIN teams ON teams.id = event_teams.team_id
            WHERE event_teams.event_id = @eventId
            ORDER BY teams.team_number
            """, new { eventId })).ToList();

        var user = await GetSessionUserAsync();
        if (user?.TeamNumber != null)
        {
            var userTeamCount = await conn.ExecuteScalarAsync<long>("""
                SELECT COUNT(*) FROM event_teams
                JOIN teams ON teams.id = event_teams.team_id
                WHERE event_teams.event_id = @eventId AND teams.team_number = @teamNumber
                """, new { eventId, teamNumber = user.TeamNumber.Value });
            if (userTeamCount == 0)
            {
                ViewData["EventWarning"] = "Your team is not listed for this event yet.";
            }
        }
    }

    /// <summary>
    /// TBA key format: {year}{event_code}, e.g. "2026mndu" -> "mndu".
    /// FIRST API expects lowercase event codes.
    /// </summary>
    protected static string ExtractEventCode(string tbaKey)
    {
        var match = System.Text.RegularExpressions.Regex.Match(tbaKey, @"^\d{4}(.+)$");
        return match.Success ? match.Groups[1].Value.ToLowerInvariant() : "";
    }

    /// <summary>
    /// Server-driven full navigation — the Unpoly analog of htmx's HX-Redirect.
    /// Emits a tt:navigate event via X-Up-Events (tt-unpoly.js turns it into
    /// window.location) and skips fragment rendering with X-Up-Target: :none.
    /// </summary>
    protected ContentResult UpNavigate(string to)
    {
        Response.Headers["X-Up-Events"] =
            $$"""[{"type":"tt:navigate","url":"{{to}}"}]""";
        Response.Headers["X-Up-Target"] = ":none";
        return Content("", "text/html; charset=utf-8");
    }

    /// <summary>
    /// Failure fragment for the auth forms, wrapped so its root matches the
    /// up-target (#form-response). On success the endpoints navigate instead.
    /// </summary>
    protected ContentResult AuthError(string message)
    {
        var html = $"""
            <div id="form-response" class="fade-swap">
                <div class="bg-red-900/20 border border-red-500 text-red-300 px-4 py-3 rounded mb-4" role="alert">
                    <div class="flex items-center gap-2">
                        <svg class="w-5 h-5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>
                        </svg>
                        <span class="block sm:inline font-medium">{System.Net.WebUtility.HtmlEncode(message)}</span>
                    </div>
                </div>
            </div>
            """;
        return Content(html, "text/html; charset=utf-8");
    }

    protected static int? ParseRequiredInt(string? value)
    {
        var trimmed = (value ?? "").Trim();
        return int.TryParse(trimmed, out var parsed) ? parsed : null;
    }
}
