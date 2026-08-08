using Dapper;
using Microsoft.AspNetCore.Mvc;
using TealTeam.Web.Data;
using TealTeam.Web.Services;

namespace TealTeam.Web.Controllers;

/// <summary>Home page, event selection, and event summary fragment.</summary>
public class PagesController(Db db, SessionService sessions, ILogger<PagesController> logger)
    : AppController(db, sessions)
{
    [HttpGet("/")]
    public async Task<IActionResult> Index()
    {
        var user = await GetSessionUserAsync();

        ViewData["Title"] = "Home";
        ViewData["Message"] = "TealTeam Scouting";
        ViewData["User"] = user;

        try
        {
            await HydrateEventSelectionDataAsync(user, HttpContext.RequestAborted);
        }
        catch (Exception ex)
        {
            logger.LogWarning(ex, "failed to hydrate event selection data");
        }

        return Page("Index");
    }

    [HttpGet("/help")]
    public async Task<IActionResult> Help()
    {
        ViewData["Title"] = "Help";
        ViewData["User"] = await GetSessionUserAsync();
        return Page("Help");
    }

    [HttpPost("/api/events/select")]
    public async Task<IActionResult> SelectEvent()
    {
        var user = await GetSessionUserAsync();
        if (user == null)
        {
            return Redirect("/sign-in");
        }

        var selectedEventId = ParseRequiredInt(Request.Form["event_id"]);
        if (selectedEventId == null)
        {
            if (IsUnpoly)
            {
                ViewData["User"] = user;
                ViewData["EventError"] = "event_id is required";
                await HydrateEventSelectionDataAsync(user, HttpContext.RequestAborted);
                return Fragment("_EventSelection");
            }
            return BadRequest("event_id is required");
        }

        var session = await GetSessionAsync();
        if (session == null)
        {
            return Redirect("/sign-in");
        }

        try
        {
            await using var conn = await Db.OpenAsync(HttpContext.RequestAborted);
            await conn.ExecuteAsync(
                "UPDATE sessions SET selected_event_id = @eventId WHERE session_id = @sessionId",
                new { eventId = selectedEventId.Value, sessionId = session.SessionId });
            logger.LogInformation("event selection updated: session {SessionId} event {EventId}",
                session.SessionId, selectedEventId.Value);
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to update selected event (session {SessionId}, event {EventId})",
                session.SessionId, selectedEventId);
            if (IsUnpoly)
            {
                ViewData["User"] = user;
                ViewData["EventError"] = "Failed to save event selection";
                await HydrateEventSelectionDataAsync(user, HttpContext.RequestAborted);
                return Fragment("_EventSelection");
            }
            return StatusCode(500, "Failed to save event selection");
        }

        if (IsUnpoly)
        {
            ViewData["User"] = user;
            ViewData["EventUpdated"] = true;
            await HydrateEventSelectionDataAsync(user, HttpContext.RequestAborted);
            await HydrateEventSummaryDataAsync(selectedEventId.Value, HttpContext.RequestAborted);
            // The event-selection form re-emits eventSelected/reload-matches
            // client-side via up-on-inserted, so no server trigger is needed.
            return Fragment("_EventSelection");
        }

        return Redirect("/");
    }

    [HttpGet("/hx/events/summary")]
    public async Task<IActionResult> EventSummary([FromQuery(Name = "event_id")] string? eventIdRaw)
    {
        var user = await GetSessionUserAsync();
        if (user == null)
        {
            return Redirect("/sign-in");
        }

        ViewData["User"] = user;

        if (string.IsNullOrEmpty(eventIdRaw))
        {
            var session = await GetSessionAsync();
            if (session?.SelectedEventId != null)
            {
                eventIdRaw = session.SelectedEventId.Value.ToString();
            }
        }

        if (string.IsNullOrEmpty(eventIdRaw))
        {
            return Fragment("_EventSummary");
        }

        if (!int.TryParse(eventIdRaw, out var eventId))
        {
            ViewData["EventSummaryError"] = "Invalid event ID";
            return Fragment("_EventSummary");
        }

        await HydrateEventSummaryDataAsync(eventId, HttpContext.RequestAborted);
        return Fragment("_EventSummary");
    }
}
