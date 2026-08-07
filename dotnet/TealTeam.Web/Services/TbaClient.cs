using System.Text.Json;
using System.Text.Json.Serialization;

namespace TealTeam.Web.Services;

// DTOs and client for The Blue Alliance API v3 (port of internal/frc/tba_client.go)

public class OprData
{
    [JsonPropertyName("oprs")] public Dictionary<string, double> Oprs { get; set; } = new();
    [JsonPropertyName("dprs")] public Dictionary<string, double> Dprs { get; set; } = new();
    [JsonPropertyName("ccwms")] public Dictionary<string, double> Ccwms { get; set; } = new();
}

/// <summary>
/// Dynamic component OPR breakdown: component-name -> teamKey -> value.
/// Includes fallback matching for season schema variance.
/// </summary>
public class ComponentOprData
{
    public Dictionary<string, Dictionary<string, double>> Components { get; set; } = new();

    private Dictionary<string, double>? ComponentMap(string[] preferredNames, string[] containsAll)
    {
        if (Components.Count == 0) return null;

        var lookup = Components.ToDictionary(
            kv => kv.Key.Trim().ToLowerInvariant(), kv => kv.Value);

        foreach (var name in preferredNames)
        {
            if (lookup.TryGetValue(name.Trim().ToLowerInvariant(), out var values))
            {
                return values;
            }
        }

        // Fallback for new seasons: pick the largest component map matching all tokens.
        Dictionary<string, double>? best = null;
        var bestSize = -1;
        foreach (var (name, values) in lookup)
        {
            if (containsAll.All(name.Contains) && values.Count > bestSize)
            {
                best = values;
                bestSize = values.Count;
            }
        }

        return best;
    }

    public (double? Auto, double? Teleop, double? Endgame) TeamPhaseOprs(string teamKey)
    {
        var autoMap = ComponentMap(
            new[] { "totalAutoPoints", "autoPoints", "Hub Auto Points" },
            new[] { "auto", "points" });
        var teleopMap = ComponentMap(
            new[] { "totalTeleopPoints", "teleopPoints", "Hub Teleop Points" },
            new[] { "teleop", "points" });
        var endgameMap = ComponentMap(
            new[] { "endGameTowerPoints", "endgamePoints", "Hub Endgame Points" },
            new[] { "endgame", "points" });

        double? auto = autoMap != null && autoMap.TryGetValue(teamKey, out var a) ? a : null;
        double? teleop = teleopMap != null && teleopMap.TryGetValue(teamKey, out var t) ? t : null;
        double? endgame = endgameMap != null && endgameMap.TryGetValue(teamKey, out var e) ? e : null;
        return (auto, teleop, endgame);
    }
}

public class RankingRecord
{
    [JsonPropertyName("wins")] public int Wins { get; set; }
    [JsonPropertyName("losses")] public int Losses { get; set; }
    [JsonPropertyName("ties")] public int Ties { get; set; }
}

public class RankingInfo
{
    [JsonPropertyName("team_key")] public string TeamKey { get; set; } = "";
    [JsonPropertyName("rank")] public int Rank { get; set; }
    [JsonPropertyName("matches_played")] public int MatchesPlayed { get; set; }
    [JsonPropertyName("qual_average")] public double? QualAverage { get; set; }
    [JsonPropertyName("extra_stats")] public List<double> ExtraStats { get; set; } = new();
    [JsonPropertyName("sort_orders")] public List<double> SortOrders { get; set; } = new();
    [JsonPropertyName("record")] public RankingRecord Record { get; set; } = new();
    [JsonPropertyName("dq")] public int Dq { get; set; }
    [JsonPropertyName("qual_points")] public int? QualPoints { get; set; }
    [JsonPropertyName("elim_points")] public int? ElimPoints { get; set; }
    [JsonPropertyName("award_points")] public int? AwardPoints { get; set; }
    [JsonPropertyName("alliance_points")] public int? AlliancePoints { get; set; }
    [JsonPropertyName("tie_points")] public int? TiePoints { get; set; }
    [JsonPropertyName("total_points")] public int? TotalPoints { get; set; }

    // Fallback extraction for year-specific ranking schema variance.
    public double? EffectiveQualAverage()
        => QualAverage ?? (SortOrders.Count > 0 ? SortOrders[0] : null);

    public double? EffectiveAvgMatchPoints()
        => SortOrders.Count > 1 ? SortOrders[1] : null;

    public long? EffectiveTotalPoints()
    {
        if (TotalPoints != null) return TotalPoints.Value;
        if (ExtraStats.Count > 0) return (long)Math.Round(ExtraStats[0]);
        return null;
    }

    public long? EffectiveQualPoints()
        => QualPoints != null ? QualPoints.Value : EffectiveTotalPoints();

    public long? EffectiveElimPoints() => ElimPoints;
    public long? EffectiveAwardPoints() => AwardPoints;
    public long? EffectiveAlliancePoints() => AlliancePoints;
}

public class TbaAlliance
{
    [JsonPropertyName("team_keys")] public List<string> Teams { get; set; } = new();
    [JsonPropertyName("score")] public int Score { get; set; }
}

public class TbaAlliances
{
    [JsonPropertyName("red")] public TbaAlliance Red { get; set; } = new();
    [JsonPropertyName("blue")] public TbaAlliance Blue { get; set; } = new();
}

public class MatchInfo
{
    [JsonPropertyName("key")] public string Key { get; set; } = "";
    [JsonPropertyName("event_key")] public string EventKey { get; set; } = "";
    [JsonPropertyName("comp_level")] public string CompLevel { get; set; } = "";
    [JsonPropertyName("set_number")] public int SetNumber { get; set; }
    [JsonPropertyName("match_number")] public int MatchNumber { get; set; }
    [JsonPropertyName("alliances")] public TbaAlliances Alliances { get; set; } = new();
    [JsonPropertyName("actual_time")] public long ActualTime { get; set; }
    [JsonPropertyName("predicted_time")] public long PredictedTime { get; set; }
    [JsonPropertyName("scheduled_time")] public long ScheduledTime { get; set; }
    [JsonPropertyName("score_breakdown")] public JsonElement? ScoreBreakdown { get; set; }
}

public class TbaClient
{
    private const string BaseUrl = "https://www.thebluealliance.com/api/v3";

    private static readonly HttpClient Http = new() { Timeout = TimeSpan.FromSeconds(20) };
    private static readonly JsonSerializerOptions JsonOptions = new() { PropertyNameCaseInsensitive = true };

    private readonly string _authKey;

    public TbaClient(string authKey)
    {
        _authKey = authKey;
    }

    private async Task<T> GetJsonAsync<T>(string path, CancellationToken ct)
    {
        if (!Connectivity.ShouldSkipConnectivityCheck(BaseUrl))
        {
            try
            {
                await Connectivity.EnsureInternetForBaseUrlAsync(BaseUrl, ct);
            }
            catch (InternetUnavailableException ex)
            {
                Connectivity.RecordApiError(ex);
                throw;
            }
        }

        var endpoint = BaseUrl + path;
        Exception? lastError = null;
        for (var attempt = 0; attempt < Connectivity.ApiRetryMaxAttempts; attempt++)
        {
            try
            {
                using var req = new HttpRequestMessage(HttpMethod.Get, endpoint);
                req.Headers.TryAddWithoutValidation("X-TBA-Auth-Key", _authKey);
                req.Headers.TryAddWithoutValidation("Accept", "application/json");

                using var resp = await Http.SendAsync(req, ct);
                if (!resp.IsSuccessStatusCode)
                {
                    var body = await resp.Content.ReadAsStringAsync(ct);
                    var err = new HttpRequestException(
                        $"tba api {path} returned {(int)resp.StatusCode}: {body[..Math.Min(body.Length, 4096)]}");
                    Connectivity.RecordApiError(err);
                    lastError = err;
                    if (attempt < Connectivity.ApiRetryMaxAttempts - 1 &&
                        Connectivity.ShouldRetryStatusCode((int)resp.StatusCode))
                    {
                        await Task.Delay(Connectivity.BackoffDelay(attempt), ct);
                        continue;
                    }
                    throw err;
                }

                await using var stream = await resp.Content.ReadAsStreamAsync(ct);
                var result = await JsonSerializer.DeserializeAsync<T>(stream, JsonOptions, ct)
                    ?? throw new JsonException($"tba api {path} returned empty payload");
                Connectivity.RecordApiSuccess();
                return result;
            }
            catch (HttpRequestException ex) when (attempt < Connectivity.ApiRetryMaxAttempts - 1 && ex.StatusCode == null)
            {
                Connectivity.RecordApiError(ex);
                lastError = ex;
                await Task.Delay(Connectivity.BackoffDelay(attempt), ct);
            }
        }

        var finalErr = lastError ?? new HttpRequestException($"tba api {path} retries exhausted");
        Connectivity.RecordApiError(finalErr);
        throw finalErr;
    }

    public Task<OprData> GetEventOprsAsync(string eventKey, CancellationToken ct = default)
        => GetJsonAsync<OprData>($"/event/{eventKey}/oprs", ct);

    public async Task<ComponentOprData?> GetEventComponentOprsAsync(string eventKey, CancellationToken ct = default)
    {
        var raw = await GetJsonAsync<Dictionary<string, Dictionary<string, double>>>($"/event/{eventKey}/coprs", ct);
        return new ComponentOprData { Components = raw };
    }

    private class RankingsResponse
    {
        [JsonPropertyName("rankings")] public List<RankingInfo> Rankings { get; set; } = new();
    }

    public async Task<List<RankingInfo>> GetEventRankingsAsync(string eventKey, CancellationToken ct = default)
        => (await GetJsonAsync<RankingsResponse>($"/event/{eventKey}/rankings", ct)).Rankings;

    public Task<List<MatchInfo>> GetEventMatchesAsync(string eventKey, CancellationToken ct = default)
        => GetJsonAsync<List<MatchInfo>>($"/event/{eventKey}/matches", ct);
}
