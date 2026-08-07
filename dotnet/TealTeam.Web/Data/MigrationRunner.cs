using Dapper;

namespace TealTeam.Web.Data;

/// <summary>
/// Applies SQL migrations from a directory, tracked in the schema_migrations
/// table. Direct port of internal/db/migrate.go so the same database can be
/// shared with the Go app.
/// </summary>
public static class MigrationRunner
{
    public static async Task ApplyAsync(Db db, string dir, ILogger logger)
    {
        if (!Directory.Exists(dir))
        {
            throw new DirectoryNotFoundException($"migrations directory not found: {dir}");
        }

        await using var conn = await db.OpenAsync();

        await conn.ExecuteAsync("""
            CREATE TABLE IF NOT EXISTS schema_migrations (
                id SERIAL PRIMARY KEY,
                filename VARCHAR(255) UNIQUE NOT NULL,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            """);

        var files = Directory.GetFiles(dir, "*.sql")
            .Select(Path.GetFileName)
            .Where(f => f != null)
            .Cast<string>()
            .OrderBy(f => f, StringComparer.Ordinal)
            .ToList();

        foreach (var file in files)
        {
            var count = await conn.ExecuteScalarAsync<long>(
                "SELECT COUNT(*) FROM schema_migrations WHERE filename = @file", new { file });
            if (count > 0)
            {
                continue;
            }

            var content = await File.ReadAllTextAsync(Path.Combine(dir, file));
            if (string.IsNullOrWhiteSpace(content))
            {
                continue;
            }

            await using var tx = await conn.BeginTransactionAsync();
            try
            {
                await conn.ExecuteAsync(content, transaction: tx);
                await conn.ExecuteAsync(
                    "INSERT INTO schema_migrations (filename) VALUES (@file)", new { file }, tx);
                await tx.CommitAsync();
                logger.LogInformation("applied migration {File}", file);
            }
            catch (Exception ex)
            {
                await tx.RollbackAsync();
                throw new InvalidOperationException($"failed to apply migration {file}", ex);
            }
        }
    }

    /// <summary>Clears migration history (test databases only).</summary>
    public static async Task ResetAsync(Db db)
    {
        await using var conn = await db.OpenAsync();
        await conn.ExecuteAsync("DROP TABLE IF EXISTS schema_migrations");
    }
}
