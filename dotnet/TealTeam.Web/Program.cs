using TealTeam.Web.Data;
using TealTeam.Web.Services;

// Load .env for local development, mirroring godotenv (ignored if missing).
// Checks the app directory first, then the repo root when run from dotnet/TealTeam.Web.
LoadDotEnv(".env");
LoadDotEnv("../../.env");

var builder = WebApplication.CreateBuilder(args);

// Environment: "test" (local/LAN, resets migration history on boot like the Go
// app) or "prod". Set via TEALTEAM_ENV; defaults to test.
var appEnv = (Environment.GetEnvironmentVariable("TEALTEAM_ENV") ?? "test").Trim().ToLowerInvariant();
if (appEnv != "test" && appEnv != "prod")
{
    Console.Error.WriteLine($"invalid environment: {appEnv}");
    return 1;
}

// Bind Kestrel to all interfaces so LAN clients (tablets over ethernet/wifi)
// can reach the server. PORT defaults to 8080, matching the Go app.
var port = Environment.GetEnvironmentVariable("PORT");
if (string.IsNullOrWhiteSpace(port)) port = "8080";
builder.WebHost.UseUrls($"http://0.0.0.0:{port}");

// Resolve DATABASE_URL with the same precedence as cmd/web/main.go.
var databaseUrl = appEnv switch
{
    "prod" => Environment.GetEnvironmentVariable("RENDER_DATABASE_URL")
              ?? Environment.GetEnvironmentVariable("DATABASE_URL"),
    _ => Environment.GetEnvironmentVariable("DATABASE_URL")
         ?? "postgres://user:password@127.0.0.1:5432/yourdb?sslmode=disable",
};

if (appEnv == "prod" && string.IsNullOrWhiteSpace(databaseUrl))
{
    Console.Error.WriteLine("production mode requires RENDER_DATABASE_URL or DATABASE_URL environment variable");
    return 1;
}

builder.Services.AddControllersWithViews();
builder.Services.AddSingleton(new Db(databaseUrl!));
builder.Services.AddSingleton<SessionService>();
builder.Services.AddSingleton<ScoutingPointsService>();
builder.Services.AddSingleton<TbaStatsSync>();
builder.Services.AddSingleton<FirstSyncService>();
builder.Services.AddHostedService<TeamStatsSyncer>();

var app = builder.Build();

var logger = app.Services.GetRequiredService<ILogger<Program>>();
logger.LogInformation("running in {Env} mode", appEnv.ToUpperInvariant());

// Migrations + boot sync. Like the Go app, the server still starts if the
// database is unavailable; DB-backed pages degrade.
var db = app.Services.GetRequiredService<Db>();
var dbAvailable = false;
try
{
    await using (var conn = await db.OpenAsync())
    {
        dbAvailable = true;
    }
}
catch (Exception ex)
{
    logger.LogWarning(ex, "database connection failed, running without database");
}

if (dbAvailable)
{
    logger.LogInformation("database connected successfully");

    if (appEnv == "test")
    {
        await MigrationRunner.ResetAsync(db);
    }

    var migrationsDir = ResolveMigrationsDir(app.Environment.ContentRootPath);
    await MigrationRunner.ApplyAsync(db, migrationsDir, logger);

    var firstSync = app.Services.GetRequiredService<FirstSyncService>();
    await firstSync.SyncOnBootAsync();
}

app.UseStaticFiles(); // wwwroot served at root; assets live under wwwroot/static
app.MapControllers();

logger.LogInformation("server listening on http://0.0.0.0:{Port}", port);
app.Run();
return 0;

static void LoadDotEnv(string path)
{
    if (!File.Exists(path)) return;
    foreach (var rawLine in File.ReadAllLines(path))
    {
        var line = rawLine.Trim();
        if (line == "" || line.StartsWith('#')) continue;
        var idx = line.IndexOf('=');
        if (idx <= 0) continue;
        var key = line[..idx].Trim();
        var value = line[(idx + 1)..].Trim().Trim('"');
        if (Environment.GetEnvironmentVariable(key) == null)
        {
            Environment.SetEnvironmentVariable(key, value);
        }
    }
}

static string ResolveMigrationsDir(string contentRoot)
{
    // Copied to the output directory at build time; fall back to the repo copy in dev.
    string[] candidates =
    {
        Path.Combine(AppContext.BaseDirectory, "migrations"),
        Path.Combine(contentRoot, "migrations"),
        Path.Combine(contentRoot, "..", "..", "migrations"),
    };
    foreach (var candidate in candidates)
    {
        if (Directory.Exists(candidate)) return candidate;
    }
    return "migrations";
}
