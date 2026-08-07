namespace TealTeam.Web.Models;

// Direct ports of internal/models/models.go. Dapper maps snake_case columns
// onto these properties (DefaultTypeMap.MatchNamesWithUnderscores).

public class User
{
    public int Id { get; set; }
    public string Email { get; set; } = "";
    public string Name { get; set; } = "";
    public string PasswordHash { get; set; } = "";
    public int? TeamNumber { get; set; }
    public string Role { get; set; } = "user";
    public bool IsAdmin { get; set; }
    public bool IsLeadScout { get; set; }
    public bool IsCoach { get; set; }
    public DateTime? LastLogin { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime UpdatedAt { get; set; }
}

public class Session
{
    public string SessionId { get; set; } = "";
    public int UserId { get; set; }
    public int? SelectedEventId { get; set; }
    public DateTime ExpiresAt { get; set; }
    public DateTime CreatedAt { get; set; }
}

public class Team
{
    public int Id { get; set; }
    public int TeamNumber { get; set; }
    public string Name { get; set; } = "";
    public string? School { get; set; }
    public string? City { get; set; }
    public string? State { get; set; }
    public string? TbaKey { get; set; }
    public string? Nickname { get; set; }
    public string? SchoolName { get; set; }
    public string? Country { get; set; }
    public int? RookieYear { get; set; }
    public string? Motto { get; set; }
    public string? Website { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime UpdatedAt { get; set; }
}

public class Event
{
    public int Id { get; set; }
    public string Name { get; set; } = "";
    public string? Location { get; set; }
    public string? Timezone { get; set; }
    public DateTime? StartDate { get; set; }
    public DateTime? EndDate { get; set; }
    public string? TbaKey { get; set; }
    public string? EventType { get; set; }
    public string? DistrictKey { get; set; }
    public int? Week { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime UpdatedAt { get; set; }
}

public class ScoutingData
{
    public int Id { get; set; }
    public int EventId { get; set; }
    public int TeamId { get; set; }
    public string AllianceColor { get; set; } = "";
    public string? Notes { get; set; }
    public string? StartingPosition { get; set; }
    public string? DefenseRating { get; set; }
    public string? ScoringStrategy { get; set; }
    public string? ShootingSpeed { get; set; }
    public string? Capacity { get; set; }
    public string? Defendability { get; set; }
    public string? Traversal { get; set; }
    public string? HangLevel { get; set; }
    public string? AutoHang { get; set; }
    public string? HangPosition { get; set; }
    public string? AccuracyRating { get; set; }
    public DateTime? ScoutedAt { get; set; }
    public int? ScouterId { get; set; }
    public int? SubmittingTeamId { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime UpdatedAt { get; set; }
}

public class ScoutingSubmission
{
    public int Id { get; set; }
    public int EventId { get; set; }
    public int TeamId { get; set; }
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
    public DateTime ScoutedAt { get; set; }
    public int? ScouterId { get; set; }
    public int? SubmittingTeamId { get; set; }
    public DateTime CreatedAt { get; set; }
}

public class Device
{
    public int Id { get; set; }
    public string DeviceUuid { get; set; } = "";
    public string? Name { get; set; }
    public int? TeamNumber { get; set; }
    public DateTime? LastSeenAt { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime UpdatedAt { get; set; }
}

public class ScoutAssignment
{
    public int Id { get; set; }
    public int EventId { get; set; }
    public int TeamId { get; set; }
    public int? ScouterId { get; set; }
    public int? DeviceId { get; set; }
    public int? AssignedBy { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime UpdatedAt { get; set; }
}

public class Match
{
    public int Id { get; set; }
    public int EventId { get; set; }
    public int MatchNumber { get; set; }
    public string MatchType { get; set; } = "";
    public int RedScore { get; set; }
    public int BlueScore { get; set; }
    public bool Played { get; set; }
    public DateTime? ScheduledTime { get; set; }
    public DateTime? ActualTime { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime UpdatedAt { get; set; }
}

public class TeamEventStats
{
    public int Id { get; set; }
    public int TeamId { get; set; }
    public int EventId { get; set; }
    public double? Opr { get; set; }
    public double? Dpr { get; set; }
    public double? Ccwm { get; set; }
    public double? AutoOpr { get; set; }
    public double? TeleopOpr { get; set; }
    public double? EndgameOpr { get; set; }
    public int? Rank { get; set; }
    public int MatchesPlayed { get; set; }
    public double? QualAverage { get; set; }
    public double? AvgMatchPoints { get; set; }
    public int Wins { get; set; }
    public int Losses { get; set; }
    public int Ties { get; set; }
    public int DqCount { get; set; }
    public long? QualPoints { get; set; }
    public long? ElimPoints { get; set; }
    public long? AwardPoints { get; set; }
    public long? AlliancePoints { get; set; }
    public long? TotalPoints { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime UpdatedAt { get; set; }
}

public class PickListEntry
{
    public int Id { get; set; }
    public int TeamNumber { get; set; }
    public int EventId { get; set; }
    public int PickedTeamNumber { get; set; }
    public string? Color { get; set; }
    public bool Crossed { get; set; }
    public int Position { get; set; }
    public DateTime CreatedAt { get; set; }
    public DateTime UpdatedAt { get; set; }
}
