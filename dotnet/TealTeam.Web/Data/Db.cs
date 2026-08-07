using Dapper;
using Npgsql;

namespace TealTeam.Web.Data;

/// <summary>
/// Connection factory over Npgsql. Accepts DATABASE_URL in either
/// postgres://user:pass@host:port/db?sslmode=disable form or a keyword
/// connection string, matching the Go app's configuration.
/// </summary>
public sealed class Db : IAsyncDisposable
{
    private readonly NpgsqlDataSource _dataSource;

    static Db()
    {
        // Map snake_case columns (team_number) to PascalCase properties (TeamNumber).
        DefaultTypeMap.MatchNamesWithUnderscores = true;
    }

    public Db(string databaseUrl)
    {
        var builder = new NpgsqlDataSourceBuilder(ToConnectionString(databaseUrl));
        _dataSource = builder.Build();
    }

    public async Task<NpgsqlConnection> OpenAsync(CancellationToken ct = default)
        => await _dataSource.OpenConnectionAsync(ct);

    public static string ToConnectionString(string databaseUrl)
    {
        if (!databaseUrl.StartsWith("postgres://", StringComparison.OrdinalIgnoreCase) &&
            !databaseUrl.StartsWith("postgresql://", StringComparison.OrdinalIgnoreCase))
        {
            return databaseUrl;
        }

        var uri = new Uri(databaseUrl);
        var userInfo = uri.UserInfo.Split(':', 2);
        var csb = new NpgsqlConnectionStringBuilder
        {
            Host = uri.Host,
            Port = uri.Port > 0 ? uri.Port : 5432,
            Database = uri.AbsolutePath.TrimStart('/'),
            Username = Uri.UnescapeDataString(userInfo[0]),
            Password = userInfo.Length > 1 ? Uri.UnescapeDataString(userInfo[1]) : "",
            MaxPoolSize = 25,
        };

        var query = System.Web.HttpUtility.ParseQueryString(uri.Query);
        var sslMode = query["sslmode"];
        if (!string.IsNullOrEmpty(sslMode))
        {
            csb.SslMode = sslMode.ToLowerInvariant() switch
            {
                "disable" => SslMode.Disable,
                "require" => SslMode.Require,
                "prefer" => SslMode.Prefer,
                "verify-ca" => SslMode.VerifyCA,
                "verify-full" => SslMode.VerifyFull,
                _ => SslMode.Prefer,
            };
        }

        return csb.ConnectionString;
    }

    public ValueTask DisposeAsync() => _dataSource.DisposeAsync();
}
