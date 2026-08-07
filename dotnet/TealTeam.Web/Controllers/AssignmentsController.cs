using Dapper;
using Microsoft.AspNetCore.Mvc;
using TealTeam.Web.Data;
using TealTeam.Web.Models;
using TealTeam.Web.Services;

namespace TealTeam.Web.Controllers;

/// <summary>
/// Per-match robot assignments: the lead scout assigns a scout or device to
/// each robot slot for each upcoming match. Assignees see their robot
/// pre-filled on the scouting form. Devices heartbeat to stay "online".
/// </summary>
public class AssignmentsController(Db db, SessionService sessions, ILogger<AssignmentsController> logger)
    : AppController(db, sessions)
{
    public const string DeviceCookieName = "device_uuid";
    private static readonly TimeSpan OnlineWindow = TimeSpan.FromMinutes(3);

    // ── View model types ──────────────────────────────────────────────────

    /// <summary>One match with its six robot assignment slots.</summary>
    public class MatchAssignmentRow
    {
        public int MatchId { get; set; }
        public int MatchNumber { get; set; }
        public string MatchType { get; set; } = "";
        public bool Played { get; set; }
        public DateTime? ScheduledTime { get; set; }
        public List<SlotAssignment> Slots { get; set; } = new();

        public string Label => MatchType switch
        {
            "qm" or "" => $"Q{MatchNumber}",
            "sf"       => $"SF{MatchNumber}",
            "f"        => $"F{MatchNumber}",
            _          => $"M{MatchNumber}",
        };
    }

    /// <summary>One robot slot within a match (red/blue, position 1-3).</summary>
    public class SlotAssignment
    {
        public string Alliance { get; set; } = "";
        public int Position { get; set; }
        public int? TeamId { get; set; }
        public int TeamNumber { get; set; }
        public string TeamName { get; set; } = "";
        public int? AssignmentId { get; set; }
        public int? ScouterId { get; set; }
        public string? ScouterName { get; set; }
        public int? DeviceId { get; set; }
        public string? DeviceName { get; set; }
        public bool AssigneeOnline { get; set; }
    }

    public class ScoutOption
    {
        public int Id { get; set; }
        public string Name { get; set; } = "";
        public bool Online { get; set; }
    }

    public class DeviceOption
    {
        public int Id { get; set; }
        public string Name { get; set; } = "";
        public bool Online { get; set; }
        public DateTime? LastSeenAt { get; set; }
    }

    // ── Internal DB row types ─────────────────────────────────────────────

    private class MatchDbRow
    {
        public int Id { get; set; }
        public int MatchNumber { get; set; }
        public string MatchType { get; set; } = "";
        public bool Played { get; set; }
        public DateTime? ScheduledTime { get; set; }
        public int? Red1 { get; set; }
        public int? Red2 { get; set; }
        public int? Red3 { get; set; }
        public int? Blue1 { get; set; }
        public int? Blue2 { get; set; }
        public int? Blue3 { get; set; }
    }

    private class AssignmentDbRow
    {
        public int Id { get; set; }
        public int MatchId { get; set; }
        public int TeamId { get; set; }
        public int? ScouterId { get; set; }
        public string? ScouterName { get; set; }
        public int? DeviceId { get; set; }
        public string? DeviceName { get; set; }
        public bool AssigneeOnline { get; set; }
    }

    private class TeamLookupRow
    {
        public int TeamNumber { get; set; }
        public int TeamId { get; set; }
        public string TeamName { get; set; } = "";
    }

    // ── Pages ─────────────────────────────────────────────────────────────

    [HttpGet("/lead-scout/assignments")]
    public async Task<IActionResult> AssignmentsPage()
    {
        var user = await GetSessionUserAsync();
        if (user == null) return Redirect("/sign-in");
        if (!user.IsAdmin && !user.IsLeadScout) return Redirect("/");

        ViewData["Title"] = "Match Assignments";
        ViewData["Description"] = "Assign scouts or devices to each robot slot per match. Assignees get a pre-filled scouting form.";
        ViewData["User"] = user;

        var session = await GetSessionAsync();
        if (session?.SelectedEventId == null)
        {
            ViewData["AssignmentsInfo"] = "Select an event on the home page first.";
            return Page("Assignments");
        }

        ViewData["SelectedEventID"] = session.SelectedEventId.Value;
        await HydrateAssignmentDataAsync(user, session.SelectedEventId.Value, HttpContext.RequestAborted);
        return Page("Assignments");
    }

    // ── Data hydration ────────────────────────────────────────────────────

    private async Task HydrateAssignmentDataAsync(User lead, int eventId, CancellationToken ct)
    {
        await using var conn = await Db.OpenAsync(ct);
        var onlineCutoff = DateTime.UtcNow - OnlineWindow;

        ViewData["SelectedEventName"] = await conn.ExecuteScalarAsync<string?>(
            "SELECT name FROM events WHERE id = @eventId", new { eventId }) ?? "";

        // Matches for this event
        var matches = (await conn.QueryAsync<MatchDbRow>("""
            SELECT id, match_number, match_type, played, scheduled_time,
                   red1, red2, red3, blue1, blue2, blue3
            FROM matches
            WHERE event_id = @eventId
            ORDER BY match_number, match_type
            """, new { eventId })).ToList();

        // Team number → id+name for all teams at this event
        var teamLookup = (await conn.QueryAsync<TeamLookupRow>("""
            SELECT teams.team_number, teams.id AS team_id, teams.name AS team_name
            FROM teams
            JOIN event_teams ON teams.id = event_teams.team_id
            WHERE event_teams.event_id = @eventId
            """, new { eventId })).ToDictionary(r => r.TeamNumber);

        // All existing assignments for this event (grouped by match)
        var assignmentsByMatch = (await conn.QueryAsync<AssignmentDbRow>("""
            SELECT sa.id, sa.match_id, sa.team_id, sa.scouter_id, u.name AS scouter_name,
                   sa.device_id,
                   COALESCE(NULLIF(d.name, ''), 'Device ' || LEFT(d.device_uuid, 8)) AS device_name,
                   (d.last_seen_at >= @onlineCutoff) IS TRUE AS assignee_online
            FROM scout_assignments sa
            JOIN matches m ON m.id = sa.match_id
            LEFT JOIN users u ON u.id = sa.scouter_id
            LEFT JOIN devices d ON d.id = sa.device_id
            WHERE m.event_id = @eventId
            """, new { eventId, onlineCutoff }))
            .GroupBy(a => a.MatchId)
            .ToDictionary(g => g.Key, g => g.ToDictionary(a => a.TeamId));

        // Build view model rows
        ViewData["MatchRows"] = matches.Select(m =>
        {
            var byTeam = assignmentsByMatch.TryGetValue(m.Id, out var d) ? d : new();
            return new MatchAssignmentRow
            {
                MatchId = m.Id,
                MatchNumber = m.MatchNumber,
                MatchType = m.MatchType,
                Played = m.Played,
                ScheduledTime = m.ScheduledTime,
                Slots = BuildSlots(m, teamLookup, byTeam),
            };
        }).ToList();

        // Scout options (users on the lead's team; everyone if lead has no team)
        var scoutWhere = lead.TeamNumber != null ? "WHERE users.team_number = @teamNumber" : "";
        ViewData["ScoutOptions"] = (await conn.QueryAsync<ScoutOption>($"""
            SELECT users.id, users.name,
                   EXISTS (SELECT 1 FROM sessions s
                           WHERE s.user_id = users.id AND s.expires_at > @now) AS online
            FROM users {scoutWhere}
            ORDER BY users.name
            """, new { teamNumber = lead.TeamNumber, now = DateTime.UtcNow })).ToList();

        ViewData["DeviceOptions"] = (await conn.QueryAsync<DeviceOption>("""
            SELECT id, COALESCE(NULLIF(name, ''), 'Device ' || SUBSTRING(device_uuid, 1, 8)) AS name,
                   (last_seen_at >= @onlineCutoff) IS TRUE AS online, last_seen_at
            FROM devices
            ORDER BY last_seen_at DESC NULLS LAST
            """, new { onlineCutoff })).ToList();
    }

    private static List<SlotAssignment> BuildSlots(
        MatchDbRow m,
        Dictionary<int, TeamLookupRow> teamLookup,
        Dictionary<int, AssignmentDbRow> byTeam)
    {
        var slots = new List<SlotAssignment>();
        foreach (var (alliance, pos, teamNumber) in new (string, int, int?)[]
        {
            ("red",  1, m.Red1), ("red",  2, m.Red2), ("red",  3, m.Red3),
            ("blue", 1, m.Blue1), ("blue", 2, m.Blue2), ("blue", 3, m.Blue3),
        })
        {
            int? teamId = null;
            var teamName = "TBD";
            if (teamNumber.HasValue && teamLookup.TryGetValue(teamNumber.Value, out var t))
            {
                teamId = t.TeamId;
                teamName = t.TeamName;
            }

            var a = teamId.HasValue && byTeam.TryGetValue(teamId.Value, out var row) ? row : null;
            slots.Add(new SlotAssignment
            {
                Alliance = alliance,
                Position = pos,
                TeamId = teamId,
                TeamNumber = teamNumber ?? 0,
                TeamName = teamName,
                AssignmentId = a?.Id,
                ScouterId = a?.ScouterId,
                ScouterName = a?.ScouterName,
                DeviceId = a?.DeviceId,
                DeviceName = a?.DeviceName,
                AssigneeOnline = a?.AssigneeOnline ?? false,
            });
        }
        return slots;
    }

    // ── Assignment mutations ──────────────────────────────────────────────

    /// <summary>
    /// Set or clear a single robot slot. Assignee value is "u:{userId}" or
    /// "d:{deviceId}"; empty string clears the assignment.
    /// </summary>
    [HttpPost("/hx/assignments/set")]
    public async Task<IActionResult> SetAssignment()
    {
        var user = await GetSessionUserAsync();
        if (user == null || (!user.IsAdmin && !user.IsLeadScout)) return Unauthorized();

        var session = await GetSessionAsync();
        if (session?.SelectedEventId == null) return BadRequest("No event selected");
        var eventId = session.SelectedEventId.Value;

        var matchId = ParseRequiredInt(Request.Form["match_id"]);
        var teamId  = ParseRequiredInt(Request.Form["team_id"]);
        if (matchId == null) return BadRequest("match_id is required");
        if (teamId == null)  return BadRequest("team_id is required");

        var assignee = ((string?)Request.Form["assignee"] ?? "").Trim();
        var ct = HttpContext.RequestAborted;

        try
        {
            await using var conn = await Db.OpenAsync(ct);
            if (assignee == "")
            {
                await conn.ExecuteAsync(
                    "DELETE FROM scout_assignments WHERE match_id = @matchId AND team_id = @teamId",
                    new { matchId = matchId.Value, teamId = teamId.Value });
            }
            else
            {
                int? scouterId = null, deviceId = null;
                if (assignee.StartsWith("u:") && int.TryParse(assignee[2..], out var uid)) scouterId = uid;
                else if (assignee.StartsWith("d:") && int.TryParse(assignee[2..], out var did)) deviceId = did;
                else return BadRequest("Invalid assignee");

                await conn.ExecuteAsync("""
                    INSERT INTO scout_assignments (match_id, team_id, event_id, scouter_id, device_id, assigned_by)
                    VALUES (@matchId, @teamId, @eventId, @scouterId, @deviceId, @assignedBy)
                    ON CONFLICT (match_id, team_id) DO UPDATE SET
                        scouter_id = EXCLUDED.scouter_id, device_id = EXCLUDED.device_id,
                        assigned_by = EXCLUDED.assigned_by, updated_at = CURRENT_TIMESTAMP
                    """, new { matchId = matchId.Value, teamId = teamId.Value, eventId, scouterId, deviceId, assignedBy = user.Id });
            }
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to set assignment (match {Match}, team {Team})", matchId, teamId);
            return StatusCode(500, "Failed to save assignment");
        }

        await HydrateAssignmentDataAsync(user, eventId, ct);
        ViewData["SelectedEventID"] = eventId;
        return Fragment("_AssignmentTable");
    }

    /// <summary>
    /// Round-robin distribute unassigned slots across upcoming unplayed matches.
    /// Pool defaults to all online scouts + devices; can be filtered via form.
    /// </summary>
    [HttpPost("/hx/assignments/auto")]
    public async Task<IActionResult> AutoDistribute()
    {
        var user = await GetSessionUserAsync();
        if (user == null || (!user.IsAdmin && !user.IsLeadScout)) return Unauthorized();

        var session = await GetSessionAsync();
        if (session?.SelectedEventId == null) return BadRequest("No event selected");
        var eventId = session.SelectedEventId.Value;
        var ct = HttpContext.RequestAborted;

        var pool = (Request.HasFormContentType ? Request.Form["assignees"] : default)
            .Select(v => (v ?? "").Trim()).Where(v => v != "").ToList();

        try
        {
            await using var conn = await Db.OpenAsync(ct);

            if (pool.Count == 0)
            {
                var onlineCutoff = DateTime.UtcNow - OnlineWindow;
                var scoutWhere = user.TeamNumber != null ? "AND users.team_number = @teamNumber" : "";
                var scoutIds = await conn.QueryAsync<int>($"""
                    SELECT DISTINCT users.id FROM users
                    JOIN sessions s ON s.user_id = users.id AND s.expires_at > @now
                    WHERE TRUE {scoutWhere}
                    """, new { now = DateTime.UtcNow, teamNumber = user.TeamNumber });
                pool.AddRange(scoutIds.Select(id => $"u:{id}"));

                var deviceIds = await conn.QueryAsync<int>(
                    "SELECT id FROM devices WHERE last_seen_at >= @onlineCutoff", new { onlineCutoff });
                pool.AddRange(deviceIds.Select(id => $"d:{id}"));
            }

            if (pool.Count > 0)
            {
                // Unassigned slots = teams in upcoming unplayed matches with no assignment yet
                var unassigned = (await conn.QueryAsync<(int MatchId, int TeamId, int EventId)>("""
                    SELECT sa_existing.match_id, teams_slot.team_id, @eventId AS event_id
                    FROM (
                        SELECT m.id AS match_id, t.id AS team_id
                        FROM matches m
                        CROSS JOIN LATERAL (
                            SELECT teams.id
                            FROM teams
                            WHERE teams.team_number IN (m.red1, m.red2, m.red3, m.blue1, m.blue2, m.blue3)
                        ) t(id)
                        WHERE m.event_id = @eventId AND m.played = FALSE
                          AND (m.red1 IS NOT NULL OR m.blue1 IS NOT NULL)
                    ) teams_slot
                    LEFT JOIN scout_assignments sa_existing
                        ON sa_existing.match_id = teams_slot.match_id
                        AND sa_existing.team_id = teams_slot.team_id
                    WHERE sa_existing.id IS NULL
                    ORDER BY teams_slot.match_id, teams_slot.team_id
                    """, new { eventId })).ToList();

                for (var i = 0; i < unassigned.Count; i++)
                {
                    var (matchId, teamId, evId) = unassigned[i];
                    var assignee = pool[i % pool.Count];
                    int? scouterId = null, deviceId = null;
                    if (assignee.StartsWith("u:") && int.TryParse(assignee[2..], out var uid)) scouterId = uid;
                    else if (assignee.StartsWith("d:") && int.TryParse(assignee[2..], out var did)) deviceId = did;
                    else continue;

                    await conn.ExecuteAsync("""
                        INSERT INTO scout_assignments (match_id, team_id, event_id, scouter_id, device_id, assigned_by)
                        VALUES (@matchId, @teamId, @eventId, @scouterId, @deviceId, @assignedBy)
                        ON CONFLICT (match_id, team_id) DO NOTHING
                        """, new { matchId, teamId, eventId = evId, scouterId, deviceId, assignedBy = user.Id });
                }
            }
            else
            {
                ViewData["AssignmentsInfo"] = "No online scouts or devices to distribute to.";
            }
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "auto-distribute failed (event {Event})", eventId);
            return StatusCode(500, "Auto-distribute failed");
        }

        await HydrateAssignmentDataAsync(user, eventId, ct);
        ViewData["SelectedEventID"] = eventId;
        return Fragment("_AssignmentTable");
    }

    [HttpPost("/hx/assignments/clear-all")]
    public async Task<IActionResult> ClearAll()
    {
        var user = await GetSessionUserAsync();
        if (user == null || (!user.IsAdmin && !user.IsLeadScout)) return Unauthorized();

        var session = await GetSessionAsync();
        if (session?.SelectedEventId == null) return BadRequest("No event selected");
        var eventId = session.SelectedEventId.Value;

        await using (var conn = await Db.OpenAsync(HttpContext.RequestAborted))
        {
            await conn.ExecuteAsync(
                "DELETE FROM scout_assignments WHERE event_id = @eventId", new { eventId });
        }

        await HydrateAssignmentDataAsync(user, eventId, HttpContext.RequestAborted);
        ViewData["SelectedEventID"] = eventId;
        return Fragment("_AssignmentTable");
    }

    [HttpPost("/hx/assignments/clear-match/{matchId:int}")]
    public async Task<IActionResult> ClearMatch(int matchId)
    {
        var user = await GetSessionUserAsync();
        if (user == null || (!user.IsAdmin && !user.IsLeadScout)) return Unauthorized();

        var session = await GetSessionAsync();
        if (session?.SelectedEventId == null) return BadRequest("No event selected");
        var eventId = session.SelectedEventId.Value;

        await using (var conn = await Db.OpenAsync(HttpContext.RequestAborted))
        {
            await conn.ExecuteAsync(
                "DELETE FROM scout_assignments WHERE match_id = @matchId", new { matchId });
        }

        await HydrateAssignmentDataAsync(user, eventId, HttpContext.RequestAborted);
        ViewData["SelectedEventID"] = eventId;
        return Fragment("_AssignmentTable");
    }

    // ── Device management ─────────────────────────────────────────────────

    [HttpPost("/hx/devices/{id:int}/rename")]
    public async Task<IActionResult> RenameDevice(int id)
    {
        var user = await GetSessionUserAsync();
        if (user == null || (!user.IsAdmin && !user.IsLeadScout)) return Unauthorized();

        var name = ((string?)Request.Form["name"] ?? "").Trim();
        if (name == "") return BadRequest("Name is required");

        await using (var conn = await Db.OpenAsync(HttpContext.RequestAborted))
        {
            await conn.ExecuteAsync(
                "UPDATE devices SET name = @name, updated_at = CURRENT_TIMESTAMP WHERE id = @id",
                new { name, id });
        }

        var session = await GetSessionAsync();
        if (session?.SelectedEventId != null)
        {
            ViewData["SelectedEventID"] = session.SelectedEventId.Value;
            await HydrateAssignmentDataAsync(user, session.SelectedEventId.Value, HttpContext.RequestAborted);
        }
        ViewData["User"] = user;
        return Fragment("_DeviceList");
    }

    /// <summary>
    /// Device heartbeat: upserts the persistent browser UUID and stamps
    /// last_seen_at. Called by device.js on load and every 60 s.
    /// </summary>
    [HttpPost("/api/device/heartbeat")]
    public async Task<IActionResult> Heartbeat()
    {
        if (!Request.Cookies.TryGetValue(DeviceCookieName, out var uuid) || string.IsNullOrEmpty(uuid))
            return BadRequest(new { error = "no device id" });

        uuid = uuid.Trim();
        if (uuid.Length is < 8 or > 64) return BadRequest(new { error = "invalid device id" });

        var user = await GetSessionUserAsync();
        try
        {
            await using var conn = await Db.OpenAsync(HttpContext.RequestAborted);
            await conn.ExecuteAsync("""
                INSERT INTO devices (device_uuid, team_number, last_seen_at)
                VALUES (@uuid, @teamNumber, @now)
                ON CONFLICT (device_uuid) DO UPDATE SET
                    last_seen_at = EXCLUDED.last_seen_at,
                    team_number = COALESCE(devices.team_number, EXCLUDED.team_number),
                    updated_at = CURRENT_TIMESTAMP
                """, new { uuid, teamNumber = user?.TeamNumber, now = DateTime.UtcNow });
        }
        catch (Exception ex)
        {
            logger.LogWarning(ex, "device heartbeat failed");
            return StatusCode(500, new { error = "heartbeat failed" });
        }

        return Json(new { status = "ok" });
    }
}
