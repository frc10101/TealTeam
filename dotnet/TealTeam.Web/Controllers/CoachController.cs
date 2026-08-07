using System.Globalization;
using Dapper;
using Microsoft.AspNetCore.Mvc;
using TealTeam.Web.Data;
using TealTeam.Web.Models;
using TealTeam.Web.Services;

namespace TealTeam.Web.Controllers;

/// <summary>Drive coach panel. Port of internal/handlers/coach.go.</summary>
public class CoachController(Db db, SessionService sessions, ILogger<CoachController> logger)
    : AppController(db, sessions)
{
    [HttpGet("/drive-coach")]
    public async Task<IActionResult> CoachViewer()
    {
        var user = await GetSessionUserAsync();
        if (user == null)
        {
            return Redirect("/sign-in");
        }
        if (!user.IsAdmin && !user.IsCoach)
        {
            return Redirect("/");
        }

        ViewData["Title"] = "Drive Coach Panel";
        ViewData["Description"] = "Quick match schedule and alliance partners.";
        ViewData["User"] = user;

        var ct = HttpContext.RequestAborted;
        try
        {
            await HydrateEventSelectionDataAsync(user, ct);

            var session = await GetSessionAsync();
            if (session == null)
            {
                ViewData["DriveCoachError"] = "Unable to read your session. Please sign out and sign in again.";
            }
            else if (session.SelectedEventId == null)
            {
                ViewData["DriveCoachInfo"] = "Select an event above to load your team schedule.";
            }
            else
            {
                var selectedEventId = session.SelectedEventId.Value;
                ViewData["SelectedEventID"] = selectedEventId;
                await HydrateEventSummaryDataAsync(selectedEventId, ct);
                await LoadDriveCoachDataAsync(selectedEventId, user.TeamNumber, ct);
            }
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to load drive coach panel");
            ViewData["DriveCoachError"] = "Database unavailable. Unable to load drive coach schedule.";
        }

        return Page("CoachViewer");
    }

    [HttpGet("/hx/drive-coach/matches")]
    public async Task<IActionResult> DriveCoachMatches()
    {
        var user = await GetSessionUserAsync();
        if (user == null)
        {
            return Unauthorized("Authentication required");
        }
        if (!user.IsAdmin && !user.IsCoach)
        {
            return StatusCode(403, "Access denied");
        }

        ViewData["User"] = user;
        ViewData["DriveCoachUpdatedAt"] = DateTime.Now.ToString("h:mm:ss tt", CultureInfo.InvariantCulture);

        try
        {
            var session = await GetSessionAsync();
            if (session == null)
            {
                ViewData["DriveCoachError"] = "Unable to read your session. Please refresh and try again.";
            }
            else if (session.SelectedEventId == null)
            {
                ViewData["DriveCoachInfo"] = "Select an event above to load your team schedule.";
            }
            else
            {
                await LoadDriveCoachDataAsync(session.SelectedEventId.Value, user.TeamNumber, HttpContext.RequestAborted);
            }
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to load drive coach matches");
            ViewData["DriveCoachError"] = "Database unavailable. Unable to load drive coach schedule.";
        }

        return Fragment("_DriveCoachMatches");
    }

    private async Task LoadDriveCoachDataAsync(int eventId, int? userTeamNumber, CancellationToken ct)
    {
        try
        {
            var (matches, summary) = await LoadDriveCoachMatchesAsync(eventId, userTeamNumber, ct);
            ViewData["DriveCoachMatches"] = matches;
            ViewData["DriveCoachSummary"] = summary;
            if (matches.Count == 0)
            {
                ViewData["DriveCoachInfo"] = "No matches were returned for your team at the selected event yet.";
            }
        }
        catch (DriveCoachException ex)
        {
            ViewData["DriveCoachError"] = ex.Message;
        }
    }

    private class DriveCoachException(string message) : Exception(message);

    private async Task<(List<DriveCoachMatch> Matches, Dictionary<string, object> Summary)> LoadDriveCoachMatchesAsync(
        int eventId, int? userTeamNumber, CancellationToken ct)
    {
        if (userTeamNumber == null)
        {
            throw new DriveCoachException("Your account is missing a team number. Add one in Account settings.");
        }

        string eventName;
        string? tbaKey, timezone;
        await using (var conn = await Db.OpenAsync(ct))
        {
            var eventRow = await conn.QuerySingleOrDefaultAsync<(string Name, string? TbaKey, string? Timezone)>(
                "SELECT name, tba_key, timezone FROM events WHERE id = @eventId", new { eventId });
            if (eventRow == default)
            {
                throw new DriveCoachException("Unable to load selected event");
            }
            (eventName, tbaKey, timezone) = eventRow;
        }

        if (string.IsNullOrWhiteSpace(tbaKey))
        {
            throw new DriveCoachException("Selected event is missing schedule data");
        }

        var eventCode = ExtractEventCode(tbaKey);
        if (eventCode == "")
        {
            throw new DriveCoachException("Unable to determine event code for schedule lookup");
        }

        var client = FirstApiClient.FromEnvironment()
            ?? throw new DriveCoachException("FIRST API credentials are not configured");
        var season = FirstApiClient.SeasonFromEnvironment();

        var filters = new Dictionary<string, string>();
        if (userTeamNumber.Value > 0)
        {
            filters["teamNumber"] = userTeamNumber.Value.ToString();
        }
        else
        {
            filters["tournamentLevel"] = "Qualification";
        }

        List<ScheduleMatch> rawMatches;
        try
        {
            using var cts = CancellationTokenSource.CreateLinkedTokenSource(ct);
            cts.CancelAfter(TimeSpan.FromSeconds(15));
            rawMatches = await client.GetMatchScheduleAsync(season, eventCode, filters, cts.Token);
        }
        catch (Exception ex)
        {
            if (Connectivity.IsInternetUnavailable(ex))
            {
                throw new DriveCoachException(
                    "No internet connection available. Connect network uplink and try manual sync again.");
            }
            throw new DriveCoachException("Could not fetch match schedule from FIRST API");
        }

        logger.LogInformation("fetched matches from FIRST API: event {EventCode} team {Team} count {Count}",
            eventCode, userTeamNumber, rawMatches.Count);

        // Load OPR/DPR stats for all teams appearing in the schedule.
        var teamNumbers = rawMatches.SelectMany(m => m.Teams.Select(t => t.TeamNumber)).Distinct().ToList();
        var statsByTeam = new Dictionary<int, DriveCoachTeam>();
        if (teamNumbers.Count > 0)
        {
            await using var conn = await Db.OpenAsync(ct);
            var rows = await conn.QueryAsync<(int TeamNumber, double? Opr, double? Dpr)>("""
                SELECT teams.team_number, team_event_stats.opr, team_event_stats.dpr
                FROM teams
                LEFT JOIN team_event_stats ON team_event_stats.team_id = teams.id
                    AND team_event_stats.event_id = @eventId
                WHERE teams.team_number = ANY(@teamNumbers)
                """, new { eventId, teamNumbers });
            foreach (var (teamNumber, opr, dpr) in rows)
            {
                statsByTeam[teamNumber] = new DriveCoachTeam { TeamNumber = teamNumber, Opr = opr, Dpr = dpr };
            }
        }

        TimeZoneInfo eventTz = TimeZoneInfo.Utc;
        if (!string.IsNullOrEmpty(timezone))
        {
            try
            {
                eventTz = TimeZoneInfo.FindSystemTimeZoneById(timezone);
            }
            catch (Exception ex)
            {
                logger.LogWarning(ex, "invalid timezone {Timezone} for event {Event}", timezone, eventName);
            }
        }

        var now = DateTimeOffset.Now;
        var results = new List<DriveCoachMatch>();
        int currentMatches = 0, upcomingMatches = 0, completedMatches = 0;

        foreach (var match in rawMatches)
        {
            var entry = new DriveCoachMatch
            {
                Description = match.Description,
                MatchNumber = match.MatchNumber,
                MatchType = match.TournamentLevel.ToLowerInvariant(),
            };

            // Parse with timezone (RFC3339) first, then without; FIRST sometimes
            // returns local times without offsets — interpret in the event timezone.
            DateTimeOffset? startTime = null;
            if (DateTimeOffset.TryParseExact(match.StartTime, "yyyy-MM-dd'T'HH:mm:ssK",
                    CultureInfo.InvariantCulture, DateTimeStyles.None, out var withTz) &&
                (match.StartTime.EndsWith("Z") || match.StartTime.Contains('+') || match.StartTime.Count(c => c == '-') > 2))
            {
                startTime = withTz;
            }
            else if (DateTime.TryParseExact(match.StartTime, "yyyy-MM-dd'T'HH:mm:ss",
                         CultureInfo.InvariantCulture, DateTimeStyles.None, out var naive))
            {
                var offset = eventTz.GetUtcOffset(naive);
                startTime = new DateTimeOffset(naive, offset);
            }
            else if (DateTimeOffset.TryParse(match.StartTime, CultureInfo.InvariantCulture,
                         DateTimeStyles.RoundtripKind, out var parsed))
            {
                startTime = parsed;
            }

            if (startTime != null)
            {
                entry.StartTime = startTime.Value.UtcDateTime;
                entry.TimeDisplay = startTime.Value.ToString("ddd MMM d, h:mm tt", CultureInfo.InvariantCulture);
                entry.MinutesFromNow = (int)-(startTime.Value - now).TotalMinutes;

                // Status: completed (< -15 min), current (±15 min), upcoming otherwise.
                var minutesUntilStart = (int)(startTime.Value - now).TotalMinutes;
                if (minutesUntilStart < -15)
                {
                    entry.Status = "completed";
                    entry.StatusLabel = "Completed";
                    entry.StatusClass = "border-gray-700 bg-gray-900/40";
                    completedMatches++;
                }
                else if (minutesUntilStart <= 15)
                {
                    entry.Status = "current";
                    entry.StatusLabel = "Current Match";
                    entry.StatusClass = "border-yellow-500 bg-yellow-900/20 ring-2 ring-yellow-500/40";
                    currentMatches++;
                }
                else
                {
                    upcomingMatches++;
                }
            }
            else
            {
                logger.LogError("failed to parse match start time: match {Match} raw {Raw}",
                    entry.MatchNumber, match.StartTime);
            }

            foreach (var matchTeam in match.Teams)
            {
                var teamStats = statsByTeam.GetValueOrDefault(matchTeam.TeamNumber,
                    new DriveCoachTeam { TeamNumber = matchTeam.TeamNumber });

                if (matchTeam.Station.StartsWith("Red"))
                {
                    entry.RedTeams.Add(teamStats);
                    if (matchTeam.TeamNumber == userTeamNumber.Value) entry.OurAlliance = "Red";
                }
                else if (matchTeam.Station.StartsWith("Blue"))
                {
                    entry.BlueTeams.Add(teamStats);
                    if (matchTeam.TeamNumber == userTeamNumber.Value) entry.OurAlliance = "Blue";
                }
            }

            var ourTeams = entry.OurAlliance switch
            {
                "Red" => entry.RedTeams,
                "Blue" => entry.BlueTeams,
                _ => null,
            };
            if (ourTeams != null)
            {
                entry.OurPartners = ourTeams
                    .Where(t => t.TeamNumber != userTeamNumber.Value)
                    .Select(t => t.TeamNumber)
                    .ToList();
            }

            results.Add(entry);
        }

        results.Sort((a, b) =>
        {
            if (a.StartTime == null) return 1;
            if (b.StartTime == null) return -1;
            return a.StartTime.Value.CompareTo(b.StartTime.Value);
        });

        var summary = new Dictionary<string, object>
        {
            ["EventName"] = eventName,
            ["TeamNumber"] = userTeamNumber.Value,
            ["TotalMatches"] = results.Count,
            ["CurrentMatches"] = currentMatches,
            ["UpcomingMatches"] = upcomingMatches,
            ["CompletedMatches"] = completedMatches,
        };

        logger.LogInformation("drive coach matches loaded: total {Total} current {Current} upcoming {Upcoming} completed {Completed}",
            results.Count, currentMatches, upcomingMatches, completedMatches);

        return (results, summary);
    }
}
