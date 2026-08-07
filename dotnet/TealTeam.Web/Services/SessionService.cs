using System.Security.Cryptography;
using Dapper;
using TealTeam.Web.Data;
using TealTeam.Web.Models;

namespace TealTeam.Web.Services;

/// <summary>
/// Cookie + database session handling, compatible with the Go app's
/// sessions table and session_id cookie. Port of the session logic in
/// internal/handlers/auth.go.
/// </summary>
public class SessionService(Db db, ILogger<SessionService> logger)
{
    public const string CookieName = "session_id";
    public static readonly TimeSpan Duration = TimeSpan.FromHours(24);
    public const int BcryptCost = 12;

    public static string HashPassword(string password)
        => BCrypt.Net.BCrypt.HashPassword(password, BcryptCost);

    public static bool CheckPasswordHash(string password, string hash)
    {
        try
        {
            return BCrypt.Net.BCrypt.Verify(password, hash);
        }
        catch
        {
            return false;
        }
    }

    public static string GenerateSessionId()
    {
        var bytes = RandomNumberGenerator.GetBytes(32);
        return Convert.ToBase64String(bytes).Replace('+', '-').Replace('/', '_');
    }

    public async Task<Session?> GetSessionAsync(HttpContext ctx)
    {
        if (!ctx.Request.Cookies.TryGetValue(CookieName, out var sessionId) || string.IsNullOrEmpty(sessionId))
        {
            return null;
        }

        await using var conn = await db.OpenAsync(ctx.RequestAborted);
        var session = await conn.QuerySingleOrDefaultAsync<Session>(
            "SELECT session_id, user_id, selected_event_id, expires_at, created_at FROM sessions WHERE session_id = @sessionId",
            new { sessionId });
        if (session == null)
        {
            return null;
        }

        if (DateTime.UtcNow > session.ExpiresAt)
        {
            await conn.ExecuteAsync("DELETE FROM sessions WHERE session_id = @sessionId", new { sessionId });
            return null;
        }

        return session;
    }

    public async Task<User?> GetSessionUserAsync(HttpContext ctx)
    {
        var session = await GetSessionAsync(ctx);
        if (session == null)
        {
            return null;
        }

        await using var conn = await db.OpenAsync(ctx.RequestAborted);
        return await conn.QuerySingleOrDefaultAsync<User>(
            "SELECT * FROM users WHERE id = @id", new { id = session.UserId });
    }

    public async Task<string> CreateSessionAsync(int userId, CancellationToken ct = default)
    {
        var sessionId = GenerateSessionId();
        var expiresAt = DateTime.UtcNow.Add(Duration);

        await using var conn = await db.OpenAsync(ct);
        await conn.ExecuteAsync(
            "INSERT INTO sessions (session_id, user_id, expires_at) VALUES (@sessionId, @userId, @expiresAt)",
            new { sessionId, userId, expiresAt });
        return sessionId;
    }

    public static void SetSessionCookie(HttpContext ctx, string sessionId)
    {
        ctx.Response.Cookies.Append(CookieName, sessionId, new CookieOptions
        {
            Path = "/",
            MaxAge = Duration,
            HttpOnly = true,
            Secure = false, // Plain HTTP on the LAN, same as the Go app.
            SameSite = SameSiteMode.Lax,
        });
    }

    public static void ClearSessionCookie(HttpContext ctx)
    {
        ctx.Response.Cookies.Append(CookieName, "", new CookieOptions
        {
            Path = "/",
            MaxAge = TimeSpan.Zero,
            HttpOnly = true,
            Secure = false,
            SameSite = SameSiteMode.Lax,
        });
    }

    public async Task DeleteSessionAsync(string sessionId, CancellationToken ct = default)
    {
        await using var conn = await db.OpenAsync(ct);
        await conn.ExecuteAsync("DELETE FROM sessions WHERE session_id = @sessionId", new { sessionId });
    }

    public async Task CleanupExpiredSessionsAsync(CancellationToken ct = default)
    {
        await using var conn = await db.OpenAsync(ct);
        var count = await conn.ExecuteAsync(
            "DELETE FROM sessions WHERE expires_at < @now", new { now = DateTime.UtcNow });
        if (count > 0)
        {
            logger.LogInformation("cleaned up expired sessions: {Count}", count);
        }
    }
}
