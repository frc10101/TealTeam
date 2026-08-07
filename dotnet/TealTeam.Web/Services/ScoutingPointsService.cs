using Dapper;
using TealTeam.Web.Data;
using TealTeam.Web.Models;

namespace TealTeam.Web.Services;

/// <summary>
/// Configurable scouting point weights used for team ranking scores.
/// Port of internal/handlers/scouting_points.go.
/// </summary>
public class ScoutingPointsService(Db db, ILogger<ScoutingPointsService> logger)
{
    // metric_key -> option_key -> points
    private static Dictionary<string, Dictionary<string, int>> DefaultConfig() => new()
    {
        ["defense_rating"] = new() { ["high"] = 5, ["mid"] = 3, ["low"] = 1 },
        ["traversal"] = new() { ["trench"] = 3, ["bump"] = 2 },
        ["shooting_speed"] = new() { ["fast"] = 4, ["medium"] = 2, ["slow"] = 1 },
        ["capacity"] = new() { ["high"] = 4, ["medium"] = 2, ["low"] = 1 },
        ["scoring_strategy"] = new() { ["scoring"] = 4, ["defending"] = 3, ["passing"] = 2 },
        ["hang_level"] = new() { ["none"] = 0, ["l1"] = 2, ["l2"] = 4, ["l3"] = 6 },
        ["auto_hang"] = new() { ["yes"] = 3, ["no"] = 0 },
        ["hang_position"] = new() { ["left"] = 1, ["center"] = 2, ["right"] = 1 },
        ["starting_position"] = new() { ["left"] = 1, ["center"] = 2, ["right"] = 1 },
    };

    private static readonly (string Key, string Label, string[] Options)[] SectionOrder =
    {
        ("defense_rating", "Defense Rating", new[] { "high", "mid", "low" }),
        ("traversal", "Traversal", new[] { "trench", "bump" }),
        ("shooting_speed", "Shooting Speed", new[] { "fast", "medium", "slow" }),
        ("capacity", "Capacity", new[] { "high", "medium", "low" }),
        ("scoring_strategy", "Scoring Strategy", new[] { "scoring", "defending", "passing" }),
        ("hang_level", "Hang Level", new[] { "none", "l1", "l2", "l3" }),
        ("auto_hang", "Auto Hang", new[] { "yes", "no" }),
        ("hang_position", "Hang Position", new[] { "left", "center", "right" }),
        ("starting_position", "Starting Position", new[] { "left", "center", "right" }),
    };

    public static string NormalizeOption(string value) => value.Trim().ToLowerInvariant();

    public static string OptionLabel(string optionKey)
        => string.Join(" ", optionKey.Split('_')
            .Where(p => p != "")
            .Select(p => char.ToUpperInvariant(p[0]) + p[1..]));

    public static string BuildWeightFieldName(string metricKey, string optionKey)
        => $"weight_{metricKey}__{optionKey}";

    public static int CalculateScoutingPoints(ScoutingMetricRow row,
        Dictionary<string, Dictionary<string, int>> cfg)
    {
        var total = 0;
        total += Get(cfg, "defense_rating", row.DefenseRating);
        total += Get(cfg, "traversal", row.Traversal);
        total += Get(cfg, "shooting_speed", row.ShootingSpeed);
        total += Get(cfg, "capacity", row.Capacity);
        total += Get(cfg, "scoring_strategy", row.ScoringStrategy);
        total += Get(cfg, "hang_level", row.HangLevel);
        total += Get(cfg, "auto_hang", row.AutoHang);
        total += Get(cfg, "hang_position", row.HangPosition);
        total += Get(cfg, "starting_position", row.StartingPosition);
        return total;

        static int Get(Dictionary<string, Dictionary<string, int>> cfg, string metric, string option)
            => cfg.TryGetValue(metric, out var options) &&
               options.TryGetValue(NormalizeOption(option), out var points) ? points : 0;
    }

    public async Task<Dictionary<string, Dictionary<string, int>>> LoadEffectiveConfigAsync(CancellationToken ct = default)
    {
        var config = DefaultConfig();
        try
        {
            await using var conn = await db.OpenAsync(ct);
            var rows = await conn.QueryAsync<(string MetricKey, string OptionKey, int Points)>(
                "SELECT metric_key, option_key, points FROM scouting_point_weights");
            foreach (var row in rows)
            {
                var metricKey = NormalizeOption(row.MetricKey);
                var optionKey = NormalizeOption(row.OptionKey);
                if (config.TryGetValue(metricKey, out var options))
                {
                    options[optionKey] = row.Points;
                }
            }
        }
        catch (Exception ex)
        {
            logger.LogWarning(ex, "failed to load scouting point config; using defaults");
        }

        return config;
    }

    public async Task<List<ScoutingPointSection>> LoadSectionsAsync(CancellationToken ct = default)
    {
        var cfg = await LoadEffectiveConfigAsync(ct);
        var sections = new List<ScoutingPointSection>();
        foreach (var (key, label, optionKeys) in SectionOrder)
        {
            var options = new List<ScoutingPointOption>();
            foreach (var optionKey in optionKeys)
            {
                if (cfg.TryGetValue(key, out var metricOptions) &&
                    metricOptions.TryGetValue(optionKey, out var points))
                {
                    options.Add(new ScoutingPointOption { Key = optionKey, Points = points });
                }
            }

            if (options.Count > 0)
            {
                sections.Add(new ScoutingPointSection { Key = key, Label = label, Options = options });
            }
        }

        return sections;
    }

    public async Task PersistSectionsAsync(List<ScoutingPointSection> sections, CancellationToken ct = default)
    {
        await using var conn = await db.OpenAsync(ct);
        await using var tx = await conn.BeginTransactionAsync(ct);
        foreach (var section in sections)
        {
            foreach (var option in section.Options)
            {
                await conn.ExecuteAsync("""
                    INSERT INTO scouting_point_weights (metric_key, option_key, points)
                    VALUES (@metricKey, @optionKey, @points)
                    ON CONFLICT (metric_key, option_key)
                    DO UPDATE SET points = EXCLUDED.points, updated_at = CURRENT_TIMESTAMP
                    """, new { metricKey = section.Key, optionKey = option.Key, points = option.Points }, tx);
            }
        }

        await tx.CommitAsync(ct);
    }

    public static List<ScoutingPointSection>? ParseSectionsFromForm(IFormCollection form, List<ScoutingPointSection> baseSections)
    {
        var parsed = new List<ScoutingPointSection>();
        foreach (var section in baseSections)
        {
            var parsedOptions = new List<ScoutingPointOption>();
            foreach (var option in section.Options)
            {
                var fieldName = BuildWeightFieldName(section.Key, option.Key);
                var value = ((string?)form[fieldName] ?? "").Trim();
                var parsedValue = option.Points;
                if (value != "")
                {
                    if (!int.TryParse(value, out var intValue)) return null;
                    if (intValue is < -100 or > 100) return null;
                    parsedValue = intValue;
                }
                parsedOptions.Add(new ScoutingPointOption { Key = option.Key, Points = parsedValue });
            }
            parsed.Add(new ScoutingPointSection { Key = section.Key, Label = section.Label, Options = parsedOptions });
        }

        return parsed;
    }
}
