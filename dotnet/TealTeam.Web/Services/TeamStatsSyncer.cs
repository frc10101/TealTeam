using Dapper;
using TealTeam.Web.Data;

namespace TealTeam.Web.Services;

/// <summary>
/// Background loop syncing team statistics and match results from TBA.
/// Fast cadence during (or just before) active events, slow cadence otherwise.
/// Port of internal/frc/team_stats_sync.go as an IHostedService.
/// </summary>
public class TeamStatsSyncer(Db db, TbaStatsSync tbaStats, ILogger<TeamStatsSyncer> logger) : BackgroundService
{
    private static readonly TimeSpan IntervalDuringEvent = TimeSpan.FromMinutes(2);
    private static readonly TimeSpan IntervalBetweenEvents = TimeSpan.FromHours(3);

    private readonly string _tbaAuthKey = (Environment.GetEnvironmentVariable("TBA_AUTH_KEY") ?? "").Trim();

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        if (_tbaAuthKey == "")
        {
            logger.LogWarning("TBA_AUTH_KEY not configured, team stats sync disabled");
            return;
        }

        logger.LogInformation("team stats sync loop started");
        var tbaClient = new TbaClient(_tbaAuthKey);

        // Initial sync immediately.
        await RunSyncOnce(tbaClient, stoppingToken);

        var currentInterval = IntervalBetweenEvents;
        while (!stoppingToken.IsCancellationRequested)
        {
            try
            {
                await Task.Delay(currentInterval, stoppingToken);
            }
            catch (OperationCanceledException)
            {
                break;
            }

            currentInterval = await DetermineSyncIntervalAsync(stoppingToken);
            await RunSyncOnce(tbaClient, stoppingToken);
        }

        logger.LogInformation("team stats sync loop stopped");
    }

    private async Task RunSyncOnce(TbaClient tbaClient, CancellationToken stoppingToken)
    {
        try
        {
            using var cts = CancellationTokenSource.CreateLinkedTokenSource(stoppingToken);
            cts.CancelAfter(TimeSpan.FromMinutes(2));
            await SyncAllTeamStatsAsync(tbaClient, cts.Token);
        }
        catch (OperationCanceledException) when (stoppingToken.IsCancellationRequested)
        {
            // Shutting down.
        }
        catch (Exception ex)
        {
            logger.LogWarning(ex, "team stats sync failed");
        }
    }

    private async Task SyncAllTeamStatsAsync(TbaClient tbaClient, CancellationToken ct)
    {
        var season = FirstApiClient.SeasonFromEnvironment();
        var now = DateTime.UtcNow;

        List<(int Id, string? TbaKey)> activeEvents;
        await using (var conn = await db.OpenAsync(ct))
        {
            activeEvents = (await conn.QueryAsync<(int, string?)>(
                "SELECT id, tba_key FROM events WHERE start_date <= @now AND end_date >= @now",
                new { now })).ToList();

            if (activeEvents.Count == 0)
            {
                logger.LogInformation("no active events found, syncing recent/upcoming events");
                var weekAgo = now.AddDays(-7);
                var nextWeek = now.AddDays(7);
                activeEvents = (await conn.QueryAsync<(int, string?)>(
                    "SELECT id, tba_key FROM events WHERE end_date >= @weekAgo AND start_date <= @nextWeek",
                    new { weekAgo, nextWeek })).ToList();
            }
        }

        foreach (var (eventId, rawKey) in activeEvents)
        {
            if (string.IsNullOrWhiteSpace(rawKey))
            {
                logger.LogWarning("event {Event} has no TBA key, skipping", eventId);
                continue;
            }

            var eventKey = TbaStatsSync.NormalizeTbaEventKey(rawKey, season);
            if (eventKey == "")
            {
                logger.LogWarning("event {Event} has invalid TBA key, skipping", eventId);
                continue;
            }

            try
            {
                await tbaStats.SyncTeamStatsForEventAsync(tbaClient, eventId, eventKey, ct);
            }
            catch (Exception ex)
            {
                logger.LogWarning(ex, "failed to sync stats for event {Event}", eventId);
                continue;
            }

            try
            {
                await tbaStats.SyncEventMatchesAsync(tbaClient, eventId, eventKey, ct);
            }
            catch (Exception ex)
            {
                logger.LogWarning(ex, "failed to sync matches for event {Event}", eventId);
            }
        }
    }

    private async Task<TimeSpan> DetermineSyncIntervalAsync(CancellationToken ct)
    {
        try
        {
            using var cts = CancellationTokenSource.CreateLinkedTokenSource(ct);
            cts.CancelAfter(TimeSpan.FromSeconds(5));
            var now = DateTime.UtcNow;

            await using var conn = await db.OpenAsync(cts.Token);
            var activeCount = await conn.ExecuteScalarAsync<long>(
                "SELECT COUNT(*) FROM events WHERE start_date <= @now AND end_date >= @now", new { now });
            if (activeCount > 0)
            {
                logger.LogInformation("{Count} active events found, using fast sync interval", activeCount);
                return IntervalDuringEvent;
            }

            var nextDay = now.AddHours(24);
            var upcomingCount = await conn.ExecuteScalarAsync<long>(
                "SELECT COUNT(*) FROM events WHERE start_date > @now AND start_date <= @nextDay", new { now, nextDay });
            if (upcomingCount > 0)
            {
                logger.LogInformation("{Count} events starting in next 24 hours, using fast sync interval", upcomingCount);
                return IntervalDuringEvent;
            }
        }
        catch (Exception ex) when (ex is not OperationCanceledException || !ct.IsCancellationRequested)
        {
            logger.LogWarning(ex, "failed to determine sync interval, using slow interval");
        }

        return IntervalBetweenEvents;
    }
}
