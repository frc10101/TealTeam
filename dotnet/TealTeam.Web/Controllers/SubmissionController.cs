using Dapper;
using Microsoft.AspNetCore.Mvc;
using TealTeam.Web.Data;
using TealTeam.Web.Models;
using TealTeam.Web.Services;

namespace TealTeam.Web.Controllers;

/// <summary>Scouting submission page + form handling. Port of internal/handlers/submission.go.</summary>
public class SubmissionController(Db db, SessionService sessions, ILogger<SubmissionController> logger)
    : AppController(db, sessions)
{
    public record AssignedTeam(int TeamId, int TeamNumber, string TeamName, int EventId, int? MatchNumber = null);

    public record TeamOption(int Id, int TeamNumber, string Name);

    private async Task BuildSubmissionPageDataAsync(User user)
    {
        ViewData["Title"] = "Scouting Submission";
        ViewData["Description"] = "Submit scouting data for competitions";
        ViewData["User"] = user;

        var ct = HttpContext.RequestAborted;
        var session = await GetSessionAsync();
        if (session?.SelectedEventId != null)
        {
            ViewData["SelectedEventID"] = session.SelectedEventId.Value;
        }

        var eventIds = await GetAvailableEventsForUserAsync(user, ct);
        await using var conn = await Db.OpenAsync(ct);
        if (eventIds.Count > 0)
        {
            ViewData["Events"] = (await conn.QueryAsync<EventOption>(
                "SELECT id, name FROM events WHERE id = ANY(@eventIds) ORDER BY start_date",
                new { eventIds })).ToList();
        }

        // Find the scout's next unplayed match assignment for the selected event
        // (matched by signed-in user OR by this device's permanent UUID).
        Request.Cookies.TryGetValue(AssignmentsController.DeviceCookieName, out var deviceUuid);
        var prefillEventId = session?.SelectedEventId;

        if (prefillEventId != null)
        {
            var assignments = (await conn.QueryAsync<AssignedTeam>("""
                SELECT sa.team_id, teams.team_number, teams.name AS team_name,
                       m.event_id, m.match_number
                FROM scout_assignments sa
                JOIN matches m ON m.id = sa.match_id
                JOIN teams ON teams.id = sa.team_id
                LEFT JOIN devices ON devices.id = sa.device_id
                WHERE m.event_id = @eventId
                  AND m.played = FALSE
                  AND (sa.scouter_id = @userId
                       OR (devices.device_uuid = @deviceUuid AND @deviceUuid IS NOT NULL))
                ORDER BY m.match_number ASC
                """, new
            {
                eventId = prefillEventId.Value,
                userId = user.Id,
                deviceUuid = string.IsNullOrEmpty(deviceUuid) ? null : deviceUuid,
            })).ToList();

            ViewData["PrefillEventID"] = prefillEventId.Value;
            ViewData["AssignedTeams"] = assignments;

            // Server-render team options for the pre-filled event
            ViewData["TeamOptions"] = (await conn.QueryAsync<TeamOption>("""
                SELECT teams.id, teams.team_number, teams.name
                FROM teams
                JOIN event_teams ON teams.id = event_teams.team_id
                WHERE event_teams.event_id = @eventId
                ORDER BY teams.team_number
                """, new { eventId = prefillEventId.Value })).ToList();

            if (assignments.Count > 0)
            {
                ViewData["PrefillTeamID"] = assignments[0].TeamId;
                ViewData["PrefillMatchNumber"] = assignments[0].MatchNumber;
            }
        }
    }

    [HttpGet("/submission")]
    public async Task<IActionResult> SubmissionPage()
    {
        var user = await GetSessionUserAsync();
        if (user == null)
        {
            return Redirect("/sign-in");
        }

        await BuildSubmissionPageDataAsync(user);
        return Page("Submission");
    }

    [HttpPost("/submission")]
    public async Task<IActionResult> Submit()
    {
        var user = await GetSessionUserAsync();
        if (user == null)
        {
            return Redirect("/sign-in");
        }

        var form = Request.Form;
        var eventId = ParseRequiredInt(form["event_id"]);
        var teamId = ParseRequiredInt(form["team_id"]);
        var allianceColor = Field(form, "alliance_color");
        var startingPosition = Field(form, "starting_position");

        string? error = null;
        if (eventId == null) error = "event_id is required";
        else if (teamId == null) error = "team_id is required";
        else if (allianceColor == "") error = "alliance_color is required";
        else if (startingPosition == "") error = "starting_position is required";

        if (error != null)
        {
            if (IsUnpoly)
            {
                await BuildSubmissionPageDataAsync(user);
                ViewData["SubmissionError"] = error;
                return Fragment("_SubmissionPanel");
            }
            return BadRequest(error);
        }

        try
        {
            await using var conn = await Db.OpenAsync(HttpContext.RequestAborted);

            // Attribute the submission to the scout's team so their notes are
            // visible to teammates on the team data page.
            int? submittingTeamId = null;
            if (user.TeamNumber is > 0)
            {
                submittingTeamId = await conn.ExecuteScalarAsync<int?>(
                    "SELECT id FROM teams WHERE team_number = @teamNumber LIMIT 1",
                    new { teamNumber = user.TeamNumber.Value });
            }

            await conn.ExecuteAsync("""
                INSERT INTO scouting_submissions (event_id, team_id, alliance_color, notes, starting_position,
                    defense_rating, traversal, scoring_strategy, shooting_speed, capacity, defendability,
                    hang_level, auto_hang, hang_position, scouted_at, scouter_id, submitting_team_id)
                VALUES (@eventId, @teamId, @allianceColor, @notes, @startingPosition,
                    @defenseRating, @traversal, @scoringStrategy, @shootingSpeed, @capacity, @defendability,
                    @hangLevel, @autoHang, @hangPosition, @scoutedAt, @scouterId, @submittingTeamId)
                """, new
            {
                eventId = eventId!.Value,
                teamId = teamId!.Value,
                allianceColor,
                notes = ((string?)form["notes"] ?? "").Trim(),
                startingPosition,
                defenseRating = Field(form, "defense_rating"),
                traversal = Field(form, "traversal"),
                scoringStrategy = Field(form, "teleop_strategy"),
                shootingSpeed = Field(form, "shooting_speed"),
                capacity = Field(form, "capacity"),
                defendability = ((string?)form["defendability"] ?? "").Trim(),
                hangLevel = Field(form, "hang_level"),
                autoHang = Field(form, "auto_hang"),
                hangPosition = Field(form, "hang_position"),
                scoutedAt = DateTime.UtcNow,
                scouterId = user.Id,
                submittingTeamId,
            });
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to create scouting submission (event {Event}, team {Team}, scouter {Scouter})",
                eventId, teamId, user.Id);
            if (IsUnpoly)
            {
                await BuildSubmissionPageDataAsync(user);
                ViewData["SubmissionError"] = $"Failed to queue submission: {ex.Message}";
                return Fragment("_SubmissionPanel");
            }
            return StatusCode(500, $"Failed to queue submission: {ex.Message}");
        }

        if (IsUnpoly)
        {
            await BuildSubmissionPageDataAsync(user);
            ViewData["SubmissionSuccess"] = "Submission queued for team scouting. Thanks for scouting!";
            return Fragment("_SubmissionPanel");
        }

        return Redirect("/submission");

        static string Field(IFormCollection form, string name)
            => ((string?)form[name] ?? "").Trim().ToLowerInvariant();
    }

    /// <summary>Team options for a selected event, upserting from FIRST API when the local DB is empty.</summary>
    [HttpGet("/submission/event-teams")]
    public async Task<IActionResult> EventTeams([FromQuery(Name = "event_id")] string? eventIdRaw)
    {
        if (string.IsNullOrEmpty(eventIdRaw))
        {
            return BadRequest("event_id is required");
        }

        if (!int.TryParse(eventIdRaw, out var eventId))
        {
            return BadRequest("event_id must be a number");
        }

        var ct = HttpContext.RequestAborted;
        string? eventTbaKey;
        List<(int Id, int TeamNumber, string Name)> teams;
        try
        {
            await using var conn = await Db.OpenAsync(ct);
            eventTbaKey = await conn.ExecuteScalarAsync<string?>(
                "SELECT tba_key FROM events WHERE id = @eventId", new { eventId });

            teams = (await conn.QueryAsync<(int, int, string)>("""
                SELECT teams.id, teams.team_number, teams.name
                FROM teams
                JOIN event_teams ON teams.id = event_teams.team_id
                WHERE event_teams.event_id = @eventId
                ORDER BY teams.team_number
                """, new { eventId })).ToList();
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to fetch event teams for event {Event}", eventId);
            return StatusCode(500, "Failed to fetch event");
        }

        var html = new System.Text.StringBuilder(
            """<option value="" disabled selected>Select team</option>""");

        if (teams.Count > 0)
        {
            foreach (var (id, teamNumber, name) in teams)
            {
                html.Append($"""<option value="{id}">{teamNumber} - {System.Net.WebUtility.HtmlEncode(name)}</option>""");
            }
            return Content(SelectWrap(html), "text/html; charset=utf-8");
        }

        // No teams locally; fall back to the FIRST API and upsert results.
        var client = FirstApiClient.FromEnvironment();
        if (client == null)
        {
            return StatusCode(500, "FIRST API credentials not configured");
        }

        List<FirstTeam> firstTeams;
        try
        {
            firstTeams = await client.GetEventTeamsAsync(FirstApiClient.SeasonFromEnvironment(), eventTbaKey ?? "", ct);
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to fetch teams from FIRST API for event {Event}", eventId);
            return StatusCode(500, "Failed to fetch teams from FIRST API");
        }

        if (firstTeams.Count == 0)
        {
            html.Append("""<option value="" disabled>No teams available for this event</option>""");
            return Content(SelectWrap(html), "text/html; charset=utf-8");
        }

        await using (var conn = await Db.OpenAsync(ct))
        {
            foreach (var firstTeam in firstTeams)
            {
                var name = firstTeam.NameShort.Trim();
                if (name == "") name = firstTeam.NameFull.Trim();

                try
                {
                    var existingId = await conn.ExecuteScalarAsync<int?>(
                        "SELECT id FROM teams WHERE team_number = @teamNumber LIMIT 1",
                        new { teamNumber = firstTeam.TeamNumber });

                    int dbId;
                    if (existingId != null)
                    {
                        await conn.ExecuteAsync("""
                            UPDATE teams SET name = @name, school_name = @schoolName, city = @city, state = @state,
                                country = @country, rookie_year = @rookieYear, website = @website,
                                updated_at = CURRENT_TIMESTAMP
                            WHERE id = @id
                            """, new
                        {
                            name,
                            schoolName = firstTeam.SchoolName,
                            city = firstTeam.City,
                            state = firstTeam.StateProv,
                            country = firstTeam.Country,
                            rookieYear = firstTeam.RookieYear,
                            website = firstTeam.Website,
                            id = existingId.Value,
                        });
                        dbId = existingId.Value;
                    }
                    else
                    {
                        dbId = await conn.ExecuteScalarAsync<int>("""
                            INSERT INTO teams (team_number, name, school_name, city, state, country, rookie_year, website)
                            VALUES (@teamNumber, @name, @schoolName, @city, @state, @country, @rookieYear, @website)
                            RETURNING id
                            """, new
                        {
                            teamNumber = firstTeam.TeamNumber,
                            name,
                            schoolName = firstTeam.SchoolName,
                            city = firstTeam.City,
                            state = firstTeam.StateProv,
                            country = firstTeam.Country,
                            rookieYear = firstTeam.RookieYear,
                            website = firstTeam.Website,
                        });
                    }

                    html.Append($"""<option value="{dbId}">{firstTeam.TeamNumber} - {System.Net.WebUtility.HtmlEncode(name)}</option>""");
                }
                catch (Exception ex)
                {
                    logger.LogWarning(ex, "failed to upsert team {Team}", firstTeam.TeamNumber);
                }
            }
        }

        return Content(SelectWrap(html), "text/html; charset=utf-8");
    }

    // Unpoly matches this response against up-target="#team-id", so the whole
    // <select> is returned (not just <option>s) and swapped as one element.
    private static string SelectWrap(System.Text.StringBuilder options)
        => $"""<select id="team-id" name="team_id" required class="w-full px-4 py-2 bg-white border border-gray-300 rounded-lg text-gray-900 focus:outline-none focus:ring-2 focus:ring-teal-500 focus:border-transparent transition-colors">{options}</select>""";
}
