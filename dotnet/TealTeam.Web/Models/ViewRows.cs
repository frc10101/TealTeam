namespace TealTeam.Web.Models;

// Row/display types shared between controllers and Razor views. These mirror
// the ad-hoc structs the Go handlers projected query results into.

public record EventOption(int Id, string Name);

public record EventTeamRow(int TeamNumber, string Name);

public class PendingSubmissionRow
{
    public int Id { get; set; }
    public string EventName { get; set; } = "";
    public int TeamNumber { get; set; }
    public string TeamName { get; set; } = "";
    public string ScoutName { get; set; } = "Unknown";
    public string FlagLabel { get; set; } = "Clean";
    public string FlagClass { get; set; } = "text-teal-300";
}

public class PickListTeam
{
    public int TeamNumber { get; set; }
    public string TeamName { get; set; } = "";
    public int? Rank { get; set; }
}

public class TeamPointSummary
{
    public int TeamId { get; set; }
    public int TeamNumber { get; set; }
    public string TeamName { get; set; } = "";
    public int? Rank { get; set; }
    public int Points { get; set; }
    public int Matches { get; set; }
}

public class ScoutingMetricRow
{
    public int TeamId { get; set; }
    public string DefenseRating { get; set; } = "";
    public string Traversal { get; set; } = "";
    public string ShootingSpeed { get; set; } = "";
    public string Capacity { get; set; } = "";
    public string ScoringStrategy { get; set; } = "";
    public string HangLevel { get; set; } = "";
    public string AutoHang { get; set; } = "";
    public string HangPosition { get; set; } = "";
    public string StartingPosition { get; set; } = "";
}

public class SubmissionDetailRow
{
    public int Id { get; set; }
    public string EventName { get; set; } = "";
    public int TeamNumber { get; set; }
    public string TeamName { get; set; } = "";
    public string ScoutName { get; set; } = "Unknown";
    public string AllianceColor { get; set; } = "";
    public string Notes { get; set; } = "";
    public string StartingPosition { get; set; } = "";
    public string DefenseRating { get; set; } = "";
    public string Traversal { get; set; } = "";
    public string ScoringStrategy { get; set; } = "";
    public string ShootingSpeed { get; set; } = "";
    public string Capacity { get; set; } = "";
    public string Defendability { get; set; } = "";
    public string HangLevel { get; set; } = "";
    public string AutoHang { get; set; } = "";
    public string HangPosition { get; set; } = "";
    public string FlagLabel { get; set; } = "Clean";
    public string FlagClass { get; set; } = "text-teal-300";
    public string CreatedAt { get; set; } = "";
}

public class MatchDisplay
{
    public string Description { get; set; } = "";
    public int MatchNumber { get; set; }
    public DateTime StartTime { get; set; }
    public string TimeDisplay { get; set; } = "";
    public string TimeStatus { get; set; } = "upcoming"; // "past", "current", "upcoming"
    public List<int> RedTeams { get; set; } = new();
    public List<int> BlueTeams { get; set; } = new();
    public int MinutesUntil { get; set; }
}

public class DriveCoachTeam
{
    public int TeamNumber { get; set; }
    public double? Opr { get; set; }
    public double? Dpr { get; set; }
}

public class DriveCoachMatch
{
    public string Description { get; set; } = "";
    public int MatchNumber { get; set; }
    public string MatchType { get; set; } = "";
    public DateTime? StartTime { get; set; }
    public string TimeDisplay { get; set; } = "";
    public string Status { get; set; } = "upcoming";
    public string StatusLabel { get; set; } = "Upcoming";
    public string StatusClass { get; set; } = "border-teal-600 bg-teal-900/20";
    public int MinutesFromNow { get; set; }
    public List<DriveCoachTeam> RedTeams { get; set; } = new();
    public List<DriveCoachTeam> BlueTeams { get; set; } = new();
    public string OurAlliance { get; set; } = "";
    public List<int> OurPartners { get; set; } = new();
}

public class TableInfo
{
    public string Name { get; set; } = "";
    public long RowCount { get; set; }
}

public class ColumnInfo
{
    public string Name { get; set; } = "";
    public string Type { get; set; } = "";
    public bool Nullable { get; set; }
    public string? Default { get; set; }
}

public class NoteEntry
{
    public string Note { get; set; } = "";
    public DateTime? ScoutedAt { get; set; }
    public int MatchIndex { get; set; }
}

public class RankingRow
{
    public int Rank { get; set; }
    public int TeamNumber { get; set; }
    public double QualAverage { get; set; }
}

public class ScoutingPointOption
{
    public string Key { get; set; } = "";
    public int Points { get; set; }
}

public class ScoutingPointSection
{
    public string Key { get; set; } = "";
    public string Label { get; set; } = "";
    public List<ScoutingPointOption> Options { get; set; } = new();
}
