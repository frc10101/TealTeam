using System.Net.Sockets;

namespace TealTeam.Web.Services;

public class InternetUnavailableException(string message) : Exception(message);

public record NetworkStatusSnapshot
{
    public DateTime CheckedAt { get; init; }
    public bool InternetReachable { get; init; }
    public string InternetError { get; init; } = "";
    public DateTime LastApiSuccessAt { get; init; }
    public DateTime LastApiErrorAt { get; init; }
    public string LastApiError { get; init; } = "";
    public DateTime LastSuccessfulSync { get; init; }
}

/// <summary>
/// Tracks internet/API health for offline-aware behavior at events.
/// Port of internal/frc/connectivity.go.
/// </summary>
public static class Connectivity
{
    private const string ProbeHost = "1.1.1.1";
    private const int ProbePort = 443;
    private static readonly TimeSpan ProbeTimeout = TimeSpan.FromMilliseconds(1500);
    private static readonly TimeSpan CacheTtl = TimeSpan.FromSeconds(3);
    public const int ApiRetryMaxAttempts = 3;

    private static readonly object Lock = new();
    private static DateTime _checkedAt;
    private static bool _ok;
    private static string _errText = "";
    private static DateTime _lastApiSuccessAt;
    private static DateTime _lastApiErrorAt;
    private static string _lastApiError = "";
    private static DateTime _lastSuccessfulSync;

    public static bool IsInternetUnavailable(Exception ex)
        => ex is InternetUnavailableException || ex.InnerException is InternetUnavailableException;

    public static NetworkStatusSnapshot GetSnapshot()
    {
        lock (Lock)
        {
            return new NetworkStatusSnapshot
            {
                CheckedAt = _checkedAt,
                InternetReachable = _ok,
                InternetError = _errText,
                LastApiSuccessAt = _lastApiSuccessAt,
                LastApiErrorAt = _lastApiErrorAt,
                LastApiError = _lastApiError,
                LastSuccessfulSync = _lastSuccessfulSync,
            };
        }
    }

    public static async Task RefreshAsync(CancellationToken ct = default)
    {
        try
        {
            await EnsureInternetAsync(ct);
        }
        catch (InternetUnavailableException)
        {
            // Snapshot already records the failure.
        }
    }

    public static async Task EnsureInternetAsync(CancellationToken ct = default)
    {
        (DateTime checkedAt, bool ok, string errText) cached;
        lock (Lock)
        {
            cached = (_checkedAt, _ok, _errText);
        }

        if (cached.checkedAt != default && DateTime.UtcNow - cached.checkedAt <= CacheTtl)
        {
            if (cached.ok) return;
            throw new InternetUnavailableException(cached.errText);
        }

        await ProbeAsync(ProbeHost, ProbePort, ct);
    }

    public static async Task EnsureInternetForBaseUrlAsync(string baseUrl, CancellationToken ct = default)
    {
        if (!Uri.TryCreate(baseUrl, UriKind.Absolute, out var uri) || string.IsNullOrWhiteSpace(uri.Host))
        {
            await EnsureInternetAsync(ct);
            return;
        }

        var port = uri.Port > 0 ? uri.Port : (uri.Scheme == "http" ? 80 : 443);
        await ProbeAsync(uri.Host, port, ct);
    }

    public static bool ShouldSkipConnectivityCheck(string baseUrl)
    {
        if (!Uri.TryCreate(baseUrl, UriKind.Absolute, out var uri) || string.IsNullOrWhiteSpace(uri.Host))
        {
            return false;
        }

        if (string.Equals(uri.Host, "localhost", StringComparison.OrdinalIgnoreCase))
        {
            return true;
        }

        if (System.Net.IPAddress.TryParse(uri.Host, out var ip))
        {
            var bytes = ip.GetAddressBytes();
            if (System.Net.IPAddress.IsLoopback(ip)) return true;
            if (bytes.Length == 4)
            {
                // RFC1918 private ranges + link-local
                if (bytes[0] == 10) return true;
                if (bytes[0] == 172 && bytes[1] >= 16 && bytes[1] <= 31) return true;
                if (bytes[0] == 192 && bytes[1] == 168) return true;
                if (bytes[0] == 169 && bytes[1] == 254) return true;
            }
        }

        return false;
    }

    private static async Task ProbeAsync(string host, int port, CancellationToken ct)
    {
        try
        {
            using var client = new TcpClient();
            using var cts = CancellationTokenSource.CreateLinkedTokenSource(ct);
            cts.CancelAfter(ProbeTimeout);
            await client.ConnectAsync(host, port, cts.Token);
            RecordConnectivityResult(true, "");
        }
        catch (Exception ex)
        {
            var reason = $"failed to reach {host}:{port} ({ex.Message})";
            RecordConnectivityResult(false, reason);
            throw new InternetUnavailableException(reason);
        }
    }

    private static void RecordConnectivityResult(bool ok, string errText)
    {
        lock (Lock)
        {
            _checkedAt = DateTime.UtcNow;
            _ok = ok;
            _errText = errText;
        }
    }

    public static void RecordApiSuccess()
    {
        lock (Lock)
        {
            var now = DateTime.UtcNow;
            _lastApiSuccessAt = now;
            // A successful API call proves the internet/API path is reachable.
            _checkedAt = now;
            _ok = true;
            _errText = "";
        }
    }

    public static void RecordApiError(Exception? ex)
    {
        if (ex == null) return;
        lock (Lock)
        {
            _lastApiErrorAt = DateTime.UtcNow;
            _lastApiError = ex.Message;
        }
    }

    public static void RecordSuccessfulSync()
    {
        lock (Lock)
        {
            _lastSuccessfulSync = DateTime.UtcNow;
        }
    }

    public static bool ShouldRetryStatusCode(int statusCode)
        => statusCode == 429 || (statusCode >= 500 && statusCode <= 599);

    public static TimeSpan BackoffDelay(int attempt) => attempt switch
    {
        <= 0 => TimeSpan.FromMilliseconds(250),
        1 => TimeSpan.FromMilliseconds(500),
        _ => TimeSpan.FromSeconds(1),
    };
}
