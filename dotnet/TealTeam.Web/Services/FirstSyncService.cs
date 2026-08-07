using System.Globalization;
using Dapper;
using TealTeam.Web.Data;

namespace TealTeam.Web.Services;

public class SyncSkippedException() : Exception("first sync skipped");

public record SyncResult(int Season, int Events, int Teams, int EventTeams);

/// <summary>
/// Pulls FIRST Events API data into events, teams, and event_teams.
/// Port of internal/frc/sync.go.
/// </summary>
public class FirstSyncService(Db db, TbaStatsSync tbaStats, ILogger<FirstSyncService> logger)
{
    private const string DefaultCountry = "USA";

    public async Task SyncOnBootAsync(CancellationToken ct = default)
    {
        if (string.Equals(Environment.GetEnvironmentVariable("FIRST_SYNC_ON_BOOT"), "false",
                StringComparison.OrdinalIgnoreCase))
        {
            logger.LogInformation("FIRST sync skipped (FIRST_SYNC_ON_BOOT=false)");
            return;
        }

        try
        {
            using var cts = CancellationTokenSource.CreateLinkedTokenSource(ct);
            cts.CancelAfter(TimeSpan.FromSeconds(60));
            var result = await SyncNowAsync(cts.Token);
            logger.LogInformation("FIRST sync complete: events={Events} teams={Teams} event_teams={EventTeams}",
                result.Events, result.Teams, result.EventTeams);
        }
        catch (SyncSkippedException)
        {
            logger.LogInformation("FIRST sync skipped (missing FIRST_API_USERNAME or FIRST_API_KEY)");
        }
        catch (Exception ex)
        {
            logger.LogWarning(ex, "FIRST sync failed");
        }
    }

    public async Task<SyncResult> SyncNowAsync(CancellationToken ct = default)
    {
        var client = FirstApiClient.FromEnvironment() ?? throw new SyncSkippedException();
        var season = FirstApiClient.SeasonFromEnvironment();

        var eventCodeFilter = (Environment.GetEnvironmentVariable("FIRST_EVENT_CODE") ?? "").Trim();
        var teamFilter = (Environment.GetEnvironmentVariable("FIRST_TEAM_NUMBER") ?? "").Trim();
        var countryFilter = (Environment.GetEnvironmentVariable("FIRST_COUNTRY") ?? "").Trim();
        if (countryFilter == "") countryFilter = DefaultCountry;

        var filters = new Dictionary<string, string>();
        if (eventCodeFilter != "") filters["eventCode"] = eventCodeFilter;
        if (teamFilter != "") filters["teamNumber"] = teamFilter;

        logger.LogInformation("FIRST sync starting (season {Season})", season);
        var events = await client.GetSeasonEventsAsync(season, filters, ct);

        if (eventCodeFilter == "" && teamFilter == "" && countryFilter != "")
        {
            events = events.Where(e => string.Equals(e.Country, countryFilter, StringComparison.OrdinalIgnoreCase)).ToList();
        }

        var eventIds = await UpsertEventsAsync(events, ct);
        logger.LogInformation("FIRST events synced: {Count}", eventIds.Count);

        var uniqueTeams = new HashSet<int>();
        var eventTeamCount = 0;
        foreach (var (eventCode, eventId) in eventIds)
        {
            List<FirstTeam> teams;
            try
            {
                teams = await client.GetEventTeamsAsync(season, eventCode, ct);
            }
            catch (Exception ex)
            {
                logger.LogWarning(ex, "teams fetch failed ({EventCode})", eventCode);
                continue;
            }

            var teamIds = new Dictionary<int, int>();
            foreach (var team in teams)
            {
                try
                {
                    var id = await UpsertTeamAsync(team, ct);
                    teamIds[team.TeamNumber] = id;
                    uniqueTeams.Add(id);
                }
                catch (Exception ex)
                {
                    logger.LogWarning(ex, "team upsert failed (team {Team})", team.TeamNumber);
                }
            }

            foreach (var teamId in teamIds.Values)
            {
                try
                {
                    await UpsertEventTeamAsync(eventId, teamId, ct);
                }
                catch (Exception ex)
                {
                    logger.LogWarning(ex, "event_teams upsert failed (event {Event}, team {Team})", eventId, teamId);
                }
            }

            eventTeamCount += teamIds.Count;
            logger.LogInformation("FIRST teams synced for {EventCode}: {Count}", eventCode, teamIds.Count);
        }

        Connectivity.RecordSuccessfulSync();
        return new SyncResult(season, eventIds.Count, uniqueTeams.Count, eventTeamCount);
    }

    /// <summary>
    /// Pulls FIRST Events API data for a specific team (called on sign-in/sign-up),
    /// then launches a background TBA stats sync for that team's events.
    /// </summary>
    public async Task<SyncResult> SyncTeamForUserAsync(int teamNumber, CancellationToken ct = default)
    {
        var client = FirstApiClient.FromEnvironment() ?? throw new SyncSkippedException();
        var season = FirstApiClient.SeasonFromEnvironment();

        var filters = new Dictionary<string, string> { ["teamNumber"] = teamNumber.ToString() };

        logger.LogInformation("FIRST team sync starting for team {Team} (season {Season})", teamNumber, season);
        var events = await client.GetSeasonEventsAsync(season, filters, ct);
        if (events.Count == 0)
        {
            logger.LogWarning("no events found for team {Team} in season {Season}", teamNumber, season);
            return new SyncResult(season, 0, 0, 0);
        }

        var eventIds = await UpsertEventsAsync(events, ct);
        logger.LogInformation("FIRST events synced for team {Team}: {Count}", teamNumber, eventIds.Count);

        var teamId = 0;
        var uniqueTeams = new HashSet<int>();
        var eventTeamCount = 0;

        foreach (var (eventCode, eventId) in eventIds)
        {
            List<FirstTeam> teams;
            try
            {
                teams = await client.GetEventTeamsAsync(season, eventCode, ct);
            }
            catch (Exception ex)
            {
                logger.LogWarning(ex, "teams fetch failed ({EventCode})", eventCode);
                continue;
            }

            foreach (var team in teams)
            {
                int id;
                try
                {
                    id = await UpsertTeamAsync(team, ct);
                }
                catch (Exception ex)
                {
                    logger.LogWarning(ex, "team upsert failed (team {Team})", team.TeamNumber);
                    continue;
                }

                uniqueTeams.Add(id);
                if (team.TeamNumber == teamNumber) teamId = id;

                try
                {
                    await UpsertEventTeamAsync(eventId, id, ct);
                    eventTeamCount++;
                }
                catch (Exception ex)
                {
                    logger.LogWarning(ex, "event_teams upsert failed (event {Event}, team {Team})", eventId, id);
                }
            }
        }

        if (teamId == 0)
        {
            throw new InvalidOperationException($"team {teamNumber} not found in any events");
        }

        logger.LogInformation(
            "FIRST team sync complete for team {Team}: events={Events} teams={Teams} event_teams={EventTeams}",
            teamNumber, eventIds.Count, uniqueTeams.Count, eventTeamCount);

        // Sync TBA stats for the team's events in the background (detached from the request).
        var idsSnapshot = eventIds.Values.ToList();
        _ = Task.Run(() => SyncTeamTbaStatsAsync(idsSnapshot), CancellationToken.None);

        Connectivity.RecordSuccessfulSync();
        return new SyncResult(season, eventIds.Count, 1, eventTeamCount);
    }

    private async Task SyncTeamTbaStatsAsync(List<int> eventIds)
    {
        var tbaKey = (Environment.GetEnvironmentVariable("TBA_AUTH_KEY") ?? "").Trim();
        if (tbaKey == "")
        {
            logger.LogInformation("TBA_AUTH_KEY not configured, skipping TBA stats sync");
            return;
        }

        var tbaClient = new TbaClient(tbaKey);
        foreach (var eventId in eventIds)
        {
            try
            {
                await using var conn = await db.OpenAsync();
                var eventTbaKey = await conn.ExecuteScalarAsync<string?>(
                    "SELECT tba_key FROM events WHERE id = @eventId", new { eventId });
                if (string.IsNullOrWhiteSpace(eventTbaKey))
                {
                    logger.LogWarning("event {Event} has no TBA key, skipping stats sync", eventId);
                    continue;
                }

                using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
                await tbaStats.SyncTeamStatsForEventAsync(tbaClient, eventId, eventTbaKey, cts.Token);
            }
            catch (Exception ex)
            {
                logger.LogWarning(ex, "TBA stats sync failed for event {Event}", eventId);
            }
        }
    }

    private async Task<Dictionary<string, int>> UpsertEventsAsync(List<FirstEvent> events, CancellationToken ct)
    {
        var eventIds = new Dictionary<string, int>();
        foreach (var evt in events)
        {
            var eventCode = evt.EffectiveCode;
            if (eventCode == "")
            {
                logger.LogWarning("event missing code: {Name}", evt.Name);
                continue;
            }

            try
            {
                var id = await UpsertEventAsync(evt, ct);
                eventIds[eventCode] = id;
            }
            catch (Exception ex)
            {
                logger.LogWarning(ex, "event upsert failed ({EventCode})", eventCode);
            }
        }

        return eventIds;
    }

    private async Task<int> UpsertEventAsync(FirstEvent evt, CancellationToken ct)
    {
        var startDate = ParseEventDate(evt.DateStart);
        var endDate = ParseEventDate(evt.DateEnd);
        var eventCode = evt.EffectiveCode;
        if (eventCode == "") throw new InvalidOperationException("missing event code");

        var location = string.Join(", ",
            new[] { evt.Venue, evt.City, evt.StateProv, evt.Country }
                .Select(p => p.Trim()).Where(p => p != ""));

        // TBA key: {year}{event_code_lowercase}, e.g. 2026mabil
        var tbaKey = $"{startDate.Year}{eventCode.ToLowerInvariant()}";

        await using var conn = await db.OpenAsync(ct);

        // events.tba_key has no unique constraint, so mirror GORM's
        // FirstOrCreate: select first, then update or insert.
        var existingId = await conn.ExecuteScalarAsync<int?>(
            "SELECT id FROM events WHERE tba_key = @tbaKey LIMIT 1", new { tbaKey });

        var args = new
        {
            name = evt.Name,
            location,
            startDate,
            endDate,
            tbaKey,
            eventType = evt.Type,
            districtKey = evt.DistrictCode,
            week = evt.WeekNumber,
        };

        if (existingId != null)
        {
            await conn.ExecuteAsync("""
                UPDATE events SET name = @name, location = @location, start_date = @startDate,
                    end_date = @endDate, event_type = @eventType, district_key = @districtKey,
                    week = @week, updated_at = CURRENT_TIMESTAMP
                WHERE id = @id
                """, new { args.name, args.location, args.startDate, args.endDate, args.eventType, args.districtKey, args.week, id = existingId.Value });
            return existingId.Value;
        }

        return await conn.ExecuteScalarAsync<int>("""
            INSERT INTO events (name, location, start_date, end_date, tba_key, event_type, district_key, week)
            VALUES (@name, @location, @startDate, @endDate, @tbaKey, @eventType, @districtKey, @week)
            RETURNING id
            """, args);
    }

    public async Task<int> UpsertTeamAsync(FirstTeam team, CancellationToken ct)
    {
        var name = team.NameShort.Trim();
        if (name == "") name = team.NameFull.Trim();

        await using var conn = await db.OpenAsync(ct);

        var existingId = await conn.ExecuteScalarAsync<int?>(
            "SELECT id FROM teams WHERE team_number = @teamNumber LIMIT 1",
            new { teamNumber = team.TeamNumber });

        var args = new
        {
            teamNumber = team.TeamNumber,
            name,
            school = team.SchoolName,
            city = team.City,
            state = team.StateProv,
            tbaKey = $"frc{team.TeamNumber}",
            nickname = team.NameShort,
            schoolName = team.SchoolName,
            country = team.Country,
            rookieYear = team.RookieYear,
            website = team.Website,
        };

        if (existingId != null)
        {
            await conn.ExecuteAsync("""
                UPDATE teams SET name = @name, school = @school, city = @city, state = @state,
                    tba_key = @tbaKey, nickname = @nickname, school_name = @schoolName,
                    country = @country, rookie_year = @rookieYear, website = @website,
                    updated_at = CURRENT_TIMESTAMP
                WHERE id = @id
                """, new { args.name, args.school, args.city, args.state, args.tbaKey, args.nickname, args.schoolName, args.country, args.rookieYear, args.website, id = existingId.Value });
            return existingId.Value;
        }

        return await conn.ExecuteScalarAsync<int>("""
            INSERT INTO teams (team_number, name, school, city, state, tba_key, nickname, school_name, country, rookie_year, website)
            VALUES (@teamNumber, @name, @school, @city, @state, @tbaKey, @nickname, @schoolName, @country, @rookieYear, @website)
            RETURNING id
            """, args);
    }

    private async Task UpsertEventTeamAsync(int eventId, int teamId, CancellationToken ct)
    {
        await using var conn = await db.OpenAsync(ct);
        await conn.ExecuteAsync("""
            INSERT INTO event_teams (event_id, team_id) VALUES (@eventId, @teamId)
            ON CONFLICT (event_id, team_id) DO NOTHING
            """, new { eventId, teamId });
    }

    internal static DateTime ParseEventDate(string value)
    {
        var trimmed = value.Trim().Trim('"');
        if (trimmed == "") throw new FormatException("empty date");

        string[] formats = { "yyyy-MM-dd", "yyyy-MM-dd'T'HH:mm:ssK", "yyyy-MM-dd'T'HH:mm:ss" };
        if (DateTime.TryParseExact(trimmed, formats, CultureInfo.InvariantCulture,
                DateTimeStyles.AdjustToUniversal | DateTimeStyles.AssumeUniversal, out var parsed))
        {
            return DateTime.SpecifyKind(parsed, DateTimeKind.Utc);
        }

        throw new FormatException($"unparsable date: {trimmed}");
    }
}
