using Dapper;
using Microsoft.AspNetCore.Mvc;
using TealTeam.Web.Data;
using TealTeam.Web.Models;
using TealTeam.Web.Services;

namespace TealTeam.Web.Controllers;

/// <summary>
/// Lead scout robot assignments: assign each robot at the selected event to a
/// signed-in scout or a registered device. Assignments pre-fill the scouting
/// form for the assignee. Devices identify themselves with a persistent
/// browser UUID and heartbeat while connected.
/// </summary>
public class AssignmentsController(Db db, SessionService sessions, ILogger<AssignmentsController> logger)
    : AppController(db, sessions)
{
    public const string DeviceCookieName = "device_uuid";
    private static readonly TimeSpan OnlineWindow = TimeSpan.FromMinutes(3);

    // Row shapes shared with the assignment views.
    public class AssignmentRow
    {
        public int TeamId { get; set; }
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

    [HttpGet("/lead-scout/assignments")]
    public async Task<IActionResult> AssignmentsPage()
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

        ViewData["Title"] = "Scout Assignments";
        ViewData["Description"] = "Assign each robot to a scout or a designated device. Assignees get a pre-filled scouting form.";
        ViewData["User"] = user;

        var session = await GetSessionAsync();
        if (session?.SelectedEventId == null)
        {
            ViewData["AssignmentsInfo"] = "Select an event on the home page first — assignments are per event.";
            return Page("Assignments");
        }

        ViewData["SelectedEventID"] = session.SelectedEventId.Value;
        await HydrateAssignmentDataAsync(user, session.SelectedEventId.Value, HttpContext.RequestAborted);
        return Page("Assignments");
    }

    private async Task HydrateAssignmentDataAsync(User lead, int eventId, CancellationToken ct)
    {
        await using var conn = await Db.OpenAsync(ct);
        var onlineCutoff = DateTime.UtcNow - OnlineWindow;

        ViewData["SelectedEventName"] = await conn.ExecuteScalarAsync<string?>(
            "SELECT name FROM events WHERE id = @eventId", new { eventId }) ?? "";

        ViewData["AssignmentRows"] = (await conn.QueryAsync<AssignmentRow>("""
            SELECT teams.id AS team_id, teams.team_number, teams.name AS team_name,
                   sa.id AS assignment_id, sa.scouter_id, users.name AS scouter_name,
                   sa.device_id, devices.name AS device_name,
                   (devices.last_seen_at >= @onlineCutoff) IS TRUE AS assignee_online
            FROM teams
            JOIN event_teams ON event_teams.team_id = teams.id
            LEFT JOIN scout_assignments sa ON sa.event_id = event_teams.event_id AND sa.team_id = teams.id
            LEFT JOIN users ON users.id = sa.scouter_id
            LEFT JOIN devices ON devices.id = sa.device_id
            WHERE event_teams.event_id = @eventId
            ORDER BY teams.team_number
            """, new { eventId, onlineCutoff })).ToList();

        // Scouts: users on the lead's team (or everyone if the lead has no
        // team). "Online" means they have an unexpired session.
        var scoutSql = lead.TeamNumber != null
            ? "WHERE users.team_number = @teamNumber"
            : "";
        ViewData["ScoutOptions"] = (await conn.QueryAsync<ScoutOption>($"""
            SELECT users.id, users.name,
                   EXISTS (SELECT 1 FROM sessions s WHERE s.user_id = users.id AND s.expires_at > @now) AS online
            FROM users
            {scoutSql}
            ORDER BY users.name
            """, new { teamNumber = lead.TeamNumber, now = DateTime.UtcNow })).ToList();

        ViewData["DeviceOptions"] = (await conn.QueryAsync<DeviceOption>("""
            SELECT id, COALESCE(NULLIF(name, ''), 'Device ' || SUBSTRING(device_uuid, 1, 8)) AS name,
                   (last_seen_at >= @onlineCutoff) IS TRUE AS online, last_seen_at
            FROM devices
            ORDER BY last_seen_at DESC NULLS LAST
            """, new { onlineCutoff })).ToList();
    }

    /// <summary>Assign a robot. Assignee value is "u:{userId}" or "d:{deviceId}"; empty clears.</summary>
    [HttpPost("/hx/assignments/set")]
    public async Task<IActionResult> SetAssignment()
    {
        var user = await GetSessionUserAsync();
        if (user == null || (!user.IsAdmin && !user.IsLeadScout))
        {
            return Unauthorized();
        }

        var session = await GetSessionAsync();
        if (session?.SelectedEventId == null)
        {
            return BadRequest("No event selected");
        }
        var eventId = session.SelectedEventId.Value;

        var teamId = ParseRequiredInt(Request.Form["team_id"]);
        if (teamId == null)
        {
            return BadRequest("team_id is required");
        }

        var assignee = ((string?)Request.Form["assignee"] ?? "").Trim();
        var ct = HttpContext.RequestAborted;

        try
        {
            await using var conn = await Db.OpenAsync(ct);
            if (assignee == "")
            {
                await conn.ExecuteAsync(
                    "DELETE FROM scout_assignments WHERE event_id = @eventId AND team_id = @teamId",
                    new { eventId, teamId = teamId.Value });
            }
            else
            {
                int? scouterId = null, deviceId = null;
                if (assignee.StartsWith("u:") && int.TryParse(assignee[2..], out var uid)) scouterId = uid;
                else if (assignee.StartsWith("d:") && int.TryParse(assignee[2..], out var did)) deviceId = did;
                else return BadRequest("Invalid assignee");

                await conn.ExecuteAsync("""
                    INSERT INTO scout_assignments (event_id, team_id, scouter_id, device_id, assigned_by)
                    VALUES (@eventId, @teamId, @scouterId, @deviceId, @assignedBy)
                    ON CONFLICT (event_id, team_id) DO UPDATE SET
                        scouter_id = EXCLUDED.scouter_id, device_id = EXCLUDED.device_id,
                        assigned_by = EXCLUDED.assigned_by, updated_at = CURRENT_TIMESTAMP
                    """, new { eventId, teamId = teamId.Value, scouterId, deviceId, assignedBy = user.Id });
            }
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to set assignment (event {Event}, team {Team})", eventId, teamId);
            return StatusCode(500, "Failed to save assignment");
        }

        await HydrateAssignmentDataAsync(user, eventId, ct);
        ViewData["SelectedEventID"] = eventId;
        return Fragment("_AssignmentTable");
    }

    /// <summary>Round-robins unassigned robots across the selected online scouts/devices.</summary>
    [HttpPost("/hx/assignments/auto")]
    public async Task<IActionResult> AutoDistribute()
    {
        var user = await GetSessionUserAsync();
        if (user == null || (!user.IsAdmin && !user.IsLeadScout))
        {
            return Unauthorized();
        }

        var session = await GetSessionAsync();
        if (session?.SelectedEventId == null)
        {
            return BadRequest("No event selected");
        }
        var eventId = session.SelectedEventId.Value;
        var ct = HttpContext.RequestAborted;

        // Checked assignees come through as repeated "assignees" form values
        // (the button may also post an empty body — distribute to everyone online).
        var pool = (Request.HasFormContentType ? Request.Form["assignees"] : default)
            .Select(v => (v ?? "").Trim())
            .Where(v => v != "")
            .ToList();

        try
        {
            await using var conn = await Db.OpenAsync(ct);
            if (pool.Count == 0)
            {
                // Default pool: online scouts on the lead's team, then online devices.
                var onlineCutoff = DateTime.UtcNow - OnlineWindow;
                var scoutSql = user.TeamNumber != null ? "AND users.team_number = @teamNumber" : "";
                var scoutIds = await conn.QueryAsync<int>($"""
                    SELECT DISTINCT users.id FROM users
                    JOIN sessions s ON s.user_id = users.id AND s.expires_at > @now
                    WHERE TRUE {scoutSql}
                    """, new { now = DateTime.UtcNow, teamNumber = user.TeamNumber });
                pool.AddRange(scoutIds.Select(id => $"u:{id}"));

                var deviceIds = await conn.QueryAsync<int>(
                    "SELECT id FROM devices WHERE last_seen_at >= @onlineCutoff", new { onlineCutoff });
                pool.AddRange(deviceIds.Select(id => $"d:{id}"));
            }

            if (pool.Count > 0)
            {
                var unassigned = (await conn.QueryAsync<int>("""
                    SELECT teams.id FROM teams
                    JOIN event_teams ON event_teams.team_id = teams.id
                    LEFT JOIN scout_assignments sa ON sa.event_id = event_teams.event_id AND sa.team_id = teams.id
                    WHERE event_teams.event_id = @eventId AND sa.id IS NULL
                    ORDER BY teams.team_number
                    """, new { eventId })).ToList();

                for (var i = 0; i < unassigned.Count; i++)
                {
                    var assignee = pool[i % pool.Count];
                    int? scouterId = null, deviceId = null;
                    if (assignee.StartsWith("u:") && int.TryParse(assignee[2..], out var uid)) scouterId = uid;
                    else if (assignee.StartsWith("d:") && int.TryParse(assignee[2..], out var did)) deviceId = did;
                    else continue;

                    await conn.ExecuteAsync("""
                        INSERT INTO scout_assignments (event_id, team_id, scouter_id, device_id, assigned_by)
                        VALUES (@eventId, @teamId, @scouterId, @deviceId, @assignedBy)
                        ON CONFLICT (event_id, team_id) DO NOTHING
                        """, new { eventId, teamId = unassigned[i], scouterId, deviceId, assignedBy = user.Id });
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
        if (user == null || (!user.IsAdmin && !user.IsLeadScout))
        {
            return Unauthorized();
        }

        var session = await GetSessionAsync();
        if (session?.SelectedEventId == null)
        {
            return BadRequest("No event selected");
        }
        var eventId = session.SelectedEventId.Value;

        await using (var conn = await Db.OpenAsync(HttpContext.RequestAborted))
        {
            await conn.ExecuteAsync("DELETE FROM scout_assignments WHERE event_id = @eventId", new { eventId });
        }

        await HydrateAssignmentDataAsync(user, eventId, HttpContext.RequestAborted);
        ViewData["SelectedEventID"] = eventId;
        return Fragment("_AssignmentTable");
    }

    [HttpPost("/hx/devices/{id:int}/rename")]
    public async Task<IActionResult> RenameDevice(int id)
    {
        var user = await GetSessionUserAsync();
        if (user == null || (!user.IsAdmin && !user.IsLeadScout))
        {
            return Unauthorized();
        }

        var name = ((string?)Request.Form["name"] ?? "").Trim();
        if (name == "")
        {
            return BadRequest("Name is required");
        }

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
    /// last_seen_at. Called by device.js on load and every 60s while open.
    /// </summary>
    [HttpPost("/api/device/heartbeat")]
    public async Task<IActionResult> Heartbeat()
    {
        if (!Request.Cookies.TryGetValue(DeviceCookieName, out var uuid) || string.IsNullOrEmpty(uuid))
        {
            return BadRequest(new { error = "no device id" });
        }

        // Sanity-limit: browser-generated UUIDs only.
        uuid = uuid.Trim();
        if (uuid.Length is < 8 or > 64)
        {
            return BadRequest(new { error = "invalid device id" });
        }

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
