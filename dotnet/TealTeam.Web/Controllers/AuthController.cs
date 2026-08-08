using Dapper;
using Microsoft.AspNetCore.Mvc;
using TealTeam.Web.Data;
using TealTeam.Web.Models;
using TealTeam.Web.Services;

namespace TealTeam.Web.Controllers;

/// <summary>Port of internal/handlers/auth.go and the sign-in/sign-up pages.</summary>
public class AuthController(Db db, SessionService sessions, FirstSyncService firstSync, ILogger<AuthController> logger)
    : AppController(db, sessions)
{
    [HttpGet("/sign-in")]
    public async Task<IActionResult> SignIn()
    {
        var user = await GetSessionUserAsync();
        if (user != null)
        {
            return Redirect("/");
        }

        ViewData["Title"] = "Sign In";
        ViewData["Description"] = "Access scouting data, analytics, and team management features.";
        return Page("SignIn");
    }

    [HttpGet("/sign-up")]
    public async Task<IActionResult> SignUp()
    {
        var user = await GetSessionUserAsync();
        if (user != null)
        {
            return Redirect("/");
        }

        ViewData["Title"] = "Sign Up";
        ViewData["Description"] = "Join your team's scouting network and start collecting match data.";
        return Page("SignUp");
    }

    [HttpPost("/api/auth/login")]
    public async Task<IActionResult> Login()
    {
        var email = ((string?)Request.Form["email"] ?? "").Trim();
        var password = (string?)Request.Form["password"] ?? "";

        if (email == "" || password == "")
        {
            return AuthError("Email and password are required");
        }

        User? user;
        try
        {
            await using var conn = await Db.OpenAsync(HttpContext.RequestAborted);
            user = await conn.QuerySingleOrDefaultAsync<User>(
                "SELECT * FROM users WHERE email = @email", new { email });
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "database error during login");
            return AuthError("An error occurred. Please try again.");
        }

        // Generic error message to prevent user enumeration.
        if (user == null || !SessionService.CheckPasswordHash(password, user.PasswordHash))
        {
            return AuthError("Invalid email or password");
        }

        string sessionId;
        try
        {
            sessionId = await Sessions.CreateSessionAsync(user.Id, HttpContext.RequestAborted);
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to create session");
            return AuthError("Failed to create session");
        }

        try
        {
            await using var conn = await Db.OpenAsync(HttpContext.RequestAborted);
            await conn.ExecuteAsync("UPDATE users SET last_login = @now WHERE id = @id",
                new { now = DateTime.UtcNow, id = user.Id });
        }
        catch (Exception ex)
        {
            logger.LogWarning(ex, "failed to update last login for user {UserId}", user.Id);
        }

        SessionService.SetSessionCookie(HttpContext, sessionId);

        if (user.TeamNumber is > 0)
        {
            var teamNumber = user.TeamNumber.Value;
            _ = Task.Run(async () =>
            {
                try
                {
                    using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(60));
                    await firstSync.SyncTeamForUserAsync(teamNumber, cts.Token);
                }
                catch (Exception ex)
                {
                    logger.LogError(ex, "failed to sync team data on login (team {Team})", teamNumber);
                }
            });
        }

        return UpNavigate("/");
    }

    [HttpPost("/api/auth/signup")]
    public async Task<IActionResult> Signup()
    {
        var name = ((string?)Request.Form["name"] ?? "").Trim();
        var email = ((string?)Request.Form["email"] ?? "").Trim();
        var password = (string?)Request.Form["password"] ?? "";
        var confirmPassword = (string?)Request.Form["confirm-password"] ?? "";
        var teamNumberRaw = ((string?)Request.Form["team-number"] ?? "").Trim();
        var leadScout = ((string?)Request.Form["lead-scout"] ?? "").Trim() != "";
        var coach = ((string?)Request.Form["coach"] ?? "").Trim() != "";

        if (name == "" || email == "" || password == "" || confirmPassword == "")
        {
            return AuthError("All fields are required");
        }

        if (password != confirmPassword)
        {
            return AuthError("Passwords do not match");
        }

        if (password.Length < 8)
        {
            return AuthError("Password must be at least 8 characters long");
        }

        if (!email.Contains('@') || !email.Contains('.'))
        {
            return AuthError("Invalid email format");
        }

        try
        {
            await using var conn = await Db.OpenAsync(HttpContext.RequestAborted);
            var existingId = await conn.ExecuteScalarAsync<int?>(
                "SELECT id FROM users WHERE email = @email LIMIT 1", new { email });
            if (existingId != null)
            {
                return AuthError("An account with this email already exists");
            }
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "database error checking existing user");
            return AuthError("An error occurred. Please try again.");
        }

        var passwordHash = SessionService.HashPassword(password);

        int? parsedTeamNumber = null;
        if (teamNumberRaw != "")
        {
            if (int.TryParse(teamNumberRaw, out var value))
            {
                parsedTeamNumber = value;
            }
            else
            {
                logger.LogWarning("invalid team number on signup: {Email} {TeamNumber}", email, teamNumberRaw);
            }
        }

        int userId;
        try
        {
            await using var conn = await Db.OpenAsync(HttpContext.RequestAborted);
            userId = await conn.ExecuteScalarAsync<int>("""
                INSERT INTO users (name, email, password_hash, team_number, role, is_lead_scout, is_coach)
                VALUES (@name, @email, @passwordHash, @teamNumber, 'user', @leadScout, @coach)
                RETURNING id
                """, new { name, email, passwordHash, teamNumber = parsedTeamNumber, leadScout, coach });
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to create user {Email}", email);
            var msg = ex.Message.Contains("duplicate") || ex.Message.Contains("unique")
                ? "An account with this email already exists"
                : "Failed to create account. Please try again.";
            return AuthError(msg);
        }

        if (parsedTeamNumber is > 0)
        {
            logger.LogInformation("user signed up: {UserId} {Email} team {Team}", userId, email, parsedTeamNumber);
            var teamNumber = parsedTeamNumber.Value;
            _ = Task.Run(async () =>
            {
                try
                {
                    using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(60));
                    var result = await firstSync.SyncTeamForUserAsync(teamNumber, cts.Token);
                    logger.LogInformation("synced team on signup: team {Team} events={Events} teams={Teams} event_teams={EventTeams}",
                        teamNumber, result.Events, result.Teams, result.EventTeams);
                }
                catch (Exception ex)
                {
                    logger.LogError(ex, "failed to sync team on signup (team {Team})", teamNumber);
                }
            });
        }

        string sessionId;
        try
        {
            sessionId = await Sessions.CreateSessionAsync(userId, HttpContext.RequestAborted);
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to create session on signup");
            return UpNavigate("/sign-in");
        }

        SessionService.SetSessionCookie(HttpContext, sessionId);
        return UpNavigate("/");
    }

    [HttpPost("/api/auth/logout")]
    public async Task<IActionResult> Logout()
    {
        if (Request.Cookies.TryGetValue(SessionService.CookieName, out var sessionId) && !string.IsNullOrEmpty(sessionId))
        {
            try
            {
                await Sessions.DeleteSessionAsync(sessionId, HttpContext.RequestAborted);
            }
            catch (Exception ex)
            {
                logger.LogWarning(ex, "failed to delete session on logout");
            }
        }

        SessionService.ClearSessionCookie(HttpContext);        return UpNavigate("/sign-in");
    }

    [HttpGet("/account")]
    public async Task<IActionResult> Account()
    {
        var user = await GetSessionUserAsync();
        if (user == null)
        {
            return Redirect("/sign-in");
        }

        ViewData["Title"] = "Account Settings";
        ViewData["User"] = user;
        return Page("Account");
    }

    [HttpPost("/api/account/change-password")]
    public async Task<IActionResult> ChangePassword()
    {
        var user = await GetSessionUserAsync();
        if (user == null)
        {
            return PasswordChangeResponse(false, "Please log in to change your password");
        }

        var currentPassword = (string?)Request.Form["current-password"] ?? "";
        var newPassword = (string?)Request.Form["new-password"] ?? "";
        var confirmPassword = (string?)Request.Form["confirm-password"] ?? "";

        if (currentPassword == "" || newPassword == "" || confirmPassword == "")
        {
            return PasswordChangeResponse(false, "All fields are required");
        }

        if (newPassword.Length < 8)
        {
            return PasswordChangeResponse(false, "New password must be at least 8 characters long");
        }

        if (newPassword != confirmPassword)
        {
            return PasswordChangeResponse(false, "New passwords do not match");
        }

        if (currentPassword == newPassword)
        {
            return PasswordChangeResponse(false, "New password must be different from your current password");
        }

        if (!SessionService.CheckPasswordHash(currentPassword, user.PasswordHash))
        {
            return PasswordChangeResponse(false, "Current password is incorrect");
        }

        try
        {
            var newPasswordHash = SessionService.HashPassword(newPassword);
            await using var conn = await Db.OpenAsync(HttpContext.RequestAborted);
            await conn.ExecuteAsync("UPDATE users SET password_hash = @hash WHERE id = @id",
                new { hash = newPasswordHash, id = user.Id });
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to update password for user {UserId}", user.Id);
            return PasswordChangeResponse(false, "Failed to update password. Please try again.");
        }

        logger.LogInformation("password changed for user {UserId} ({Email})", user.Id, user.Email);
        return PasswordChangeResponse(true, "Password changed successfully!");
    }

    private ContentResult PasswordChangeResponse(bool success, string message)
    {
        var encoded = System.Net.WebUtility.HtmlEncode(message);
        // Wrapped so the response root matches up-target="#password-response".
        // On success, up-on-inserted resets the form (Unpoly does not run
        // <script> in swapped fragments).
        var html = success
            ? $"""
               <div id="password-response" up-on-inserted="document.getElementById('change-password-form').reset()">
                   <div class="bg-green-900/20 border border-green-500 text-green-300 px-4 py-3 rounded" role="alert">
                       <div class="flex items-center gap-2">
                           <svg class="w-5 h-5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                               <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"></path>
                           </svg>
                           <span class="block sm:inline font-medium">{encoded}</span>
                       </div>
                   </div>
               </div>
               """
            : $"""
               <div id="password-response">
                   <div class="bg-red-900/20 border border-red-500 text-red-300 px-4 py-3 rounded" role="alert">
                       <div class="flex items-center gap-2">
                           <svg class="w-5 h-5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                               <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>
                           </svg>
                           <span class="block sm:inline font-medium">{encoded}</span>
                       </div>
                   </div>
               </div>
               """;

        return Content(html, "text/html; charset=utf-8");
    }
}
