using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TealTeam.Web.Services;

// DTOs for the FIRST Events API (port of internal/frc/first_api.go)

public class FirstEvent
{
    [JsonPropertyName("eventCode")] public string EventCode { get; set; } = "";
    [JsonPropertyName("code")] public string Code { get; set; } = "";
    [JsonPropertyName("name")] public string Name { get; set; } = "";
    [JsonPropertyName("type")] public string Type { get; set; } = "";
    [JsonPropertyName("districtCode")] public string DistrictCode { get; set; } = "";
    [JsonPropertyName("venue")] public string Venue { get; set; } = "";
    [JsonPropertyName("city")] public string City { get; set; } = "";
    [JsonPropertyName("stateprov")] public string StateProv { get; set; } = "";
    [JsonPropertyName("country")] public string Country { get; set; } = "";
    [JsonPropertyName("dateStart")] public string DateStart { get; set; } = "";
    [JsonPropertyName("dateEnd")] public string DateEnd { get; set; } = "";
    [JsonPropertyName("weekNumber")] public int WeekNumber { get; set; }

    public string EffectiveCode =>
        !string.IsNullOrWhiteSpace(EventCode) ? EventCode.Trim() : Code.Trim();
}

public class FirstTeam
{
    [JsonPropertyName("teamNumber")] public int TeamNumber { get; set; }
    [JsonPropertyName("nameFull")] public string NameFull { get; set; } = "";
    [JsonPropertyName("nameShort")] public string NameShort { get; set; } = "";
    [JsonPropertyName("schoolName")] public string SchoolName { get; set; } = "";
    [JsonPropertyName("city")] public string City { get; set; } = "";
    [JsonPropertyName("stateProv")] public string StateProv { get; set; } = "";
    [JsonPropertyName("country")] public string Country { get; set; } = "";
    [JsonPropertyName("rookieYear")] public int RookieYear { get; set; }
    [JsonPropertyName("website")] public string Website { get; set; } = "";
}

public class ScheduleMatchTeam
{
    [JsonPropertyName("teamNumber")] public int TeamNumber { get; set; }
    [JsonPropertyName("station")] public string Station { get; set; } = "";
    [JsonPropertyName("surrogate")] public bool Surrogate { get; set; }
    [JsonPropertyName("dq")] public bool Dq { get; set; }
}

public class ScheduleMatch
{
    [JsonPropertyName("description")] public string Description { get; set; } = "";
    [JsonPropertyName("tournamentLevel")] public string TournamentLevel { get; set; } = "";
    [JsonPropertyName("matchNumber")] public int MatchNumber { get; set; }
    [JsonPropertyName("startTime")] public string StartTime { get; set; } = "";
    [JsonPropertyName("field")] public string Field { get; set; } = "";
    [JsonPropertyName("teams")] public List<ScheduleMatchTeam> Teams { get; set; } = new();
}

/// <summary>
/// Client for the FIRST Events API v3 using Basic auth, with connectivity
/// preflight and bounded retries. Port of internal/frc/first_api.go.
/// </summary>
public class FirstApiClient
{
    private const string DefaultBaseUrl = "https://frc-api.firstinspires.org/v3.0";

    private static readonly HttpClient Http = new() { Timeout = TimeSpan.FromSeconds(20) };
    private static readonly JsonSerializerOptions JsonOptions = new() { PropertyNameCaseInsensitive = true };

    private readonly string _baseUrl;
    private readonly string _authHeader;

    public FirstApiClient(string username, string key, string? baseUrl = null)
    {
        _baseUrl = baseUrl ?? DefaultBaseUrl;
        _authHeader = "Basic " + Convert.ToBase64String(Encoding.UTF8.GetBytes($"{username}:{key}"));
    }

    /// <summary>Builds a client from environment config, or null if credentials are missing.</summary>
    public static FirstApiClient? FromEnvironment()
    {
        var username = (Environment.GetEnvironmentVariable("FIRST_API_USERNAME") ?? "").Trim();
        var key = (Environment.GetEnvironmentVariable("FIRST_API_KEY") ?? "").Trim();
        if (username == "" || key == "") return null;
        return new FirstApiClient(username, key);
    }

    public static int SeasonFromEnvironment()
    {
        var seasonEnv = (Environment.GetEnvironmentVariable("FIRST_SEASON") ?? "").Trim();
        return int.TryParse(seasonEnv, out var parsed) ? parsed : 2026;
    }

    private async Task<T> GetJsonAsync<T>(string path, IDictionary<string, string>? query, CancellationToken ct)
    {
        if (!Connectivity.ShouldSkipConnectivityCheck(_baseUrl))
        {
            try
            {
                await Connectivity.EnsureInternetForBaseUrlAsync(_baseUrl, ct);
            }
            catch (InternetUnavailableException ex)
            {
                Connectivity.RecordApiError(ex);
                throw;
            }
        }

        var endpoint = _baseUrl + path;
        if (query is { Count: > 0 })
        {
            endpoint += "?" + string.Join("&", query.Select(kv =>
                $"{Uri.EscapeDataString(kv.Key)}={Uri.EscapeDataString(kv.Value)}"));
        }

        Exception? lastError = null;
        for (var attempt = 0; attempt < Connectivity.ApiRetryMaxAttempts; attempt++)
        {
            try
            {
                using var req = new HttpRequestMessage(HttpMethod.Get, endpoint);
                req.Headers.TryAddWithoutValidation("Authorization", _authHeader);
                req.Headers.TryAddWithoutValidation("Accept", "application/json");

                using var resp = await Http.SendAsync(req, ct);
                if (!resp.IsSuccessStatusCode)
                {
                    var body = await resp.Content.ReadAsStringAsync(ct);
                    var err = new HttpRequestException(
                        $"first api {path} returned {(int)resp.StatusCode}: {body[..Math.Min(body.Length, 4096)]}");
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
                    ?? throw new JsonException($"first api {path} returned empty payload");
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

        var finalErr = lastError ?? new HttpRequestException($"first api {path} retries exhausted");
        Connectivity.RecordApiError(finalErr);
        throw finalErr;
    }

    private class EventsResponse
    {
        [JsonPropertyName("Events")] public List<FirstEvent> Events { get; set; } = new();
    }

    public async Task<List<FirstEvent>> GetSeasonEventsAsync(int season, IDictionary<string, string>? filters, CancellationToken ct = default)
        => (await GetJsonAsync<EventsResponse>($"/{season}/events", filters, ct)).Events;

    private class TeamsResponse
    {
        [JsonPropertyName("teams")] public List<FirstTeam> Teams { get; set; } = new();
    }

    public async Task<List<FirstTeam>> GetEventTeamsAsync(int season, string eventCode, CancellationToken ct = default)
        => (await GetJsonAsync<TeamsResponse>($"/{season}/teams",
            new Dictionary<string, string> { ["eventCode"] = eventCode }, ct)).Teams;

    private class ScheduleResponse
    {
        [JsonPropertyName("Schedule")] public List<ScheduleMatch> Schedule { get; set; } = new();
    }

    public async Task<List<ScheduleMatch>> GetMatchScheduleAsync(int season, string eventCode, IDictionary<string, string>? filters, CancellationToken ct = default)
        => (await GetJsonAsync<ScheduleResponse>($"/{season}/schedule/{eventCode}", filters, ct)).Schedule;
}
