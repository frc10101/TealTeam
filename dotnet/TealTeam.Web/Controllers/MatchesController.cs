using System.Globalization;
using Dapper;
using Microsoft.AspNetCore.Mvc;
using TealTeam.Web.Data;
using TealTeam.Web.Models;
using TealTeam.Web.Services;

namespace TealTeam.Web.Controllers;

/// <summary>Match schedule HTMX fragment. Port of internal/handlers/matches.go.</summary>
public class MatchesController(Db db, SessionService sessions, ILogger<MatchesController> logger)
    : AppController(db, sessions)
{
    [HttpGet("/hx/matches/schedule")]
    public async Task<IActionResult> MatchSchedule()
    {
        var user = await GetSessionUserAsync();
        if (user == null)
        {
            ViewData["ScheduleMessage"] = "Sign in to view match schedule.";
            return Fragment("_MatchSchedule");
        }

        var session = await GetSessionAsync();
        if (session == null)
        {
            ViewData["ScheduleMessage"] = "Session error. Please refresh the page.";
            return Fragment("_MatchSchedule");
        }

        if (session.SelectedEventId == null)
        {
            logger.LogInformation("no selected event in session for user {UserId}", session.UserId);
            ViewData["ScheduleMessage"] = "Select an event to view live matches.";
            return Fragment("_MatchSchedule");
        }

        var eventId = session.SelectedEventId.Value;
        var ct = HttpContext.RequestAborted;

        string? eventName;
        string? tbaKey;
        await using (var conn = await Db.OpenAsync(ct))
        {
            var eventRow = await conn.QuerySingleOrDefaultAsync<(string Name, string? TbaKey)>(
                "SELECT name, tba_key FROM events WHERE id = @eventId", new { eventId });
            if (eventRow == default)
            {
                ViewData["ScheduleMessage"] = "Unable to load selected event.";
                return Fragment("_MatchSchedule");
            }
            (eventName, tbaKey) = eventRow;
        }

        if (string.IsNullOrEmpty(tbaKey))
        {
            ViewData["ScheduleMessage"] = "Selected event is missing schedule data.";
            return Fragment("_MatchSchedule");
        }

        var eventCode = ExtractEventCode(tbaKey);
        if (eventCode == "")
        {
            logger.LogError("failed to extract event code from TBA key {TbaKey}", tbaKey);
            ViewData["ScheduleMessage"] = "Unable to determine event code for schedule lookup.";
            return Fragment("_MatchSchedule");
        }

        var client = FirstApiClient.FromEnvironment();
        if (client == null)
        {
            ViewData["ScheduleMessage"] = "FIRST API credentials are not configured.";
            return Fragment("_MatchSchedule");
        }

        var season = FirstApiClient.SeasonFromEnvironment();

        // FIRST schedule endpoint requires at least one filter parameter.
        var filters = new Dictionary<string, string>();
        if (user.TeamNumber is > 0)
        {
            filters["teamNumber"] = user.TeamNumber.Value.ToString();
        }
        else
        {
            filters["tournamentLevel"] = "Qualification";
        }

        List<ScheduleMatch> matches;
        try
        {
            using var cts = CancellationTokenSource.CreateLinkedTokenSource(ct);
            cts.CancelAfter(TimeSpan.FromSeconds(15));
            matches = await client.GetMatchScheduleAsync(season, eventCode, filters, cts.Token);
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to fetch match schedule for {EventCode}", eventCode);

            var staleMatches = await LoadStaleMatchesFromDbAsync(eventId, ct);
            if (staleMatches.Count > 0)
            {
                ViewData["Matches"] = staleMatches;
                ViewData["EventName"] = eventName;
                ViewData["ScheduleMessage"] = Connectivity.IsInternetUnavailable(ex)
                    ? "Offline: showing cached schedule from local DB."
                    : "Using cached schedule data from local DB.";
                return Fragment("_MatchSchedule");
            }

            ViewData["ScheduleMessage"] = Connectivity.IsInternetUnavailable(ex)
                ? "No internet connection available. Connect uplink and retry manual sync."
                : "Could not fetch match schedule from FIRST API.";
            return Fragment("_MatchSchedule");
        }

        // Only show matches within a ±15 minute window.
        var now = DateTime.Now;
        var windowStart = now.AddMinutes(-15);
        var windowEnd = now.AddMinutes(15);

        var displayMatches = new List<MatchDisplay>();
        foreach (var match in matches)
        {
            if (!DateTime.TryParse(match.StartTime, CultureInfo.InvariantCulture,
                    DateTimeStyles.RoundtripKind, out var startTime))
            {
                logger.LogWarning("failed to parse match start time: match {Match} time {Time}",
                    match.MatchNumber, match.StartTime);
                continue;
            }

            if (startTime < windowStart || startTime > windowEnd)
            {
                continue;
            }

            var redTeams = new List<int>();
            var blueTeams = new List<int>();
            foreach (var matchTeam in match.Teams)
            {
                if (matchTeam.Station.StartsWith("Red")) redTeams.Add(matchTeam.TeamNumber);
                else if (matchTeam.Station.StartsWith("Blue")) blueTeams.Add(matchTeam.TeamNumber);
            }

            var minutesUntil = (int)(startTime - now).TotalMinutes;
            var timeStatus = "upcoming";
            if (minutesUntil < -15) timeStatus = "past";
            else if (minutesUntil <= 5) timeStatus = "current";

            displayMatches.Add(new MatchDisplay
            {
                Description = match.Description,
                MatchNumber = match.MatchNumber,
                StartTime = startTime,
                TimeDisplay = startTime.ToLocalTime().ToString("ddd MMM d, h:mm tt", CultureInfo.InvariantCulture),
                TimeStatus = timeStatus,
                RedTeams = redTeams,
                BlueTeams = blueTeams,
                MinutesUntil = minutesUntil,
            });
        }

        displayMatches.Sort((a, b) => a.StartTime.CompareTo(b.StartTime));

        ViewData["Matches"] = displayMatches;
        ViewData["EventName"] = eventName;
        return Fragment("_MatchSchedule");
    }

    private async Task<List<MatchDisplay>> LoadStaleMatchesFromDbAsync(int eventId, CancellationToken ct)
    {
        try
        {
            await using var conn = await Db.OpenAsync(ct);
            var dbMatches = (await conn.QueryAsync<(int MatchNumber, DateTime? ScheduledTime)>(
                "SELECT match_number, scheduled_time FROM matches WHERE event_id = @eventId ORDER BY scheduled_time",
                new { eventId })).ToList();

            var now = DateTime.UtcNow;
            var display = new List<MatchDisplay>();
            foreach (var (matchNumber, scheduledTime) in dbMatches)
            {
                var item = new MatchDisplay
                {
                    Description = $"Match {matchNumber}",
                    MatchNumber = matchNumber,
                    TimeStatus = "upcoming",
                };

                if (scheduledTime != null)
                {
                    item.StartTime = scheduledTime.Value;
                    item.TimeDisplay = scheduledTime.Value.ToLocalTime()
                        .ToString("ddd MMM d, h:mm tt", CultureInfo.InvariantCulture);
                    var minutesUntil = (int)(scheduledTime.Value - now).TotalMinutes;
                    item.MinutesUntil = minutesUntil;
                    if (minutesUntil < -15) item.TimeStatus = "past";
                    else if (minutesUntil <= 5) item.TimeStatus = "current";
                }

                display.Add(item);
            }

            return display;
        }
        catch (Exception ex)
        {
            logger.LogWarning(ex, "failed to load stale matches from DB for event {Event}", eventId);
            return new List<MatchDisplay>();
        }
    }
}
