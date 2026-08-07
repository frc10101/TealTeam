using Dapper;
using Microsoft.AspNetCore.Mvc;
using TealTeam.Web.Data;
using TealTeam.Web.Models;
using TealTeam.Web.Services;

namespace TealTeam.Web.Controllers;

/// <summary>Admin database viewer. Port of internal/handlers/db_viewer.go.</summary>
public class DbViewerController(Db db, SessionService sessions, ILogger<DbViewerController> logger)
    : AppController(db, sessions)
{
    // Sensitive columns never shown in the viewer.
    private static readonly Dictionary<string, string[]> SensitiveColumns = new()
    {
        ["users"] = new[] { "password_hash" },
    };

    [HttpGet("/development/db")]
    public async Task<IActionResult> DbViewer([FromQuery] string? tab, [FromQuery] string? table)
    {
        var user = await GetSessionUserAsync();
        if (user == null || !user.IsAdmin)
        {
            return Redirect("/");
        }

        ViewData["Title"] = "Database Viewer";
        ViewData["DBConnected"] = true;
        ViewData["User"] = user;
        ViewData["ActiveTab"] = string.IsNullOrEmpty(tab) ? "core" : tab;

        try
        {
            ViewData["Tables"] = await GetTableListAsync(HttpContext.RequestAborted);
        }
        catch (Exception ex)
        {
            ViewData["Error"] = $"Failed to get tables: {ex.Message}";
        }

        if (!string.IsNullOrEmpty(table))
        {
            ViewData["SelectedTable"] = table;
        }

        return Page("DbViewer");
    }

    [HttpGet("/hx/development/db/table/{name}")]
    public async Task<IActionResult> TableContent(string name, [FromQuery] int offset = 0, [FromQuery] int limit = 50)
    {
        if (string.IsNullOrEmpty(name))
        {
            return BadRequest("Table name is required");
        }

        if (limit <= 0) limit = 50;
        if (offset < 0) offset = 0;

        try
        {
            await LoadTableDataAsync(name, offset, limit, HttpContext.RequestAborted);
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "failed to get table data for {Table}", name);
            return StatusCode(500, $"Failed to get table data: {ex.Message}");
        }

        return Fragment("_DbTableContent");
    }

    private async Task<List<TableInfo>> GetTableListAsync(CancellationToken ct)
    {
        await using var conn = await Db.OpenAsync(ct);
        var tableNames = (await conn.QueryAsync<string>("""
            SELECT table_name FROM information_schema.tables
            WHERE table_schema = 'public' AND table_type = 'BASE TABLE'
            ORDER BY table_name
            """)).ToList();

        var tables = new List<TableInfo>();
        foreach (var name in tableNames)
        {
            long rowCount = 0;
            try
            {
                // Table name is validated against information_schema above.
                rowCount = await conn.ExecuteScalarAsync<long>($"SELECT COUNT(*) FROM \"{name}\"");
            }
            catch
            {
                // Count failures leave 0, matching the Go behavior.
            }
            tables.Add(new TableInfo { Name = name, RowCount = rowCount });
        }

        return tables;
    }

    private async Task LoadTableDataAsync(string tableName, int offset, int limit, CancellationToken ct)
    {
        await using var conn = await Db.OpenAsync(ct);

        var allowed = await conn.ExecuteScalarAsync<long>("""
            SELECT COUNT(*) FROM information_schema.tables
            WHERE table_schema = 'public' AND table_type = 'BASE TABLE' AND table_name = @tableName
            """, new { tableName });
        if (allowed == 0)
        {
            throw new InvalidOperationException("invalid table name");
        }

        ViewData["SelectedTable"] = tableName;
        ViewData["Offset"] = offset;
        ViewData["Limit"] = limit;

        var columns = (await conn.QueryAsync<(string Name, string Type, string Nullable, string? Default)>("""
            SELECT column_name AS name, data_type AS type, is_nullable AS nullable, column_default AS "default"
            FROM information_schema.columns
            WHERE table_schema = 'public' AND table_name = @tableName
            ORDER BY ordinal_position
            """, new { tableName }))
            .Select(c => new ColumnInfo { Name = c.Name, Type = c.Type, Nullable = c.Nullable == "YES", Default = c.Default })
            .ToList();

        var excluded = SensitiveColumns.GetValueOrDefault(tableName, Array.Empty<string>()).ToHashSet();
        var displayColumns = columns.Where(c => !excluded.Contains(c.Name)).ToList();
        ViewData["Columns"] = displayColumns;

        ViewData["TotalRows"] = await conn.ExecuteScalarAsync<long>($"SELECT COUNT(*) FROM \"{tableName}\"");

        var orderBy = columns.Count > 0 ? $" ORDER BY \"{columns[0].Name}\"" : "";
        var rows = (await conn.QueryAsync(
            $"SELECT * FROM \"{tableName}\"{orderBy} LIMIT @limit OFFSET @offset",
            new { limit, offset })).ToList();

        var rowData = new List<List<string>>();
        foreach (var row in rows)
        {
            var dict = (IDictionary<string, object?>)row;
            var rowValues = new List<string>();
            foreach (var col in displayColumns)
            {
                var value = dict.TryGetValue(col.Name, out var v) ? v : null;
                rowValues.Add(FormatValue(value));
            }
            rowData.Add(rowValues);
        }
        ViewData["Rows"] = rowData;
    }

    private static string FormatValue(object? v) => v switch
    {
        null => "",
        byte[] bytes => System.Text.Encoding.UTF8.GetString(bytes),
        string s => s,
        _ => v.ToString() ?? "",
    };
}
