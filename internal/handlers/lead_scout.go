package handlers

import (
    "database/sql"
    "fmt"
    "net/http"
    "strings"

    "github.com/gin-gonic/gin"
)

type pendingSubmissionRow struct {
    ID         int
    MatchLabel string
    TeamNumber int
    TeamName   string
    ScoutName  string
    FlagLabel  string
    FlagClass  string
}

type pickListTeam struct {
    TeamNumber int
}

func (h *Handler) loadPendingSubmissions(c *gin.Context) ([]pendingSubmissionRow, error) {
    var rows []struct {
        ID           int
        MatchNumber  int
        MatchType    string
        TeamNumber   int
        TeamName     string
        ScoutName    sql.NullString
        AutoPathData sql.NullString
        Notes        sql.NullString
    }

    if err := h.db.WithContext(c.Request.Context()).
        Table("scouting_submissions").
        Select("scouting_submissions.id, matches.match_number, matches.match_type, teams.team_number, teams.name as team_name, users.name as scout_name, CAST(scouting_submissions.auto_path_data AS text) as auto_path_data, scouting_submissions.notes").
        Joins("JOIN matches ON matches.id = scouting_submissions.match_id").
        Joins("JOIN teams ON teams.id = scouting_submissions.team_id").
        Joins("LEFT JOIN users ON users.id = scouting_submissions.scouter_id").
        Order("scouting_submissions.created_at").
        Scan(&rows).Error; err != nil {
        return nil, err
    }

    submissions := make([]pendingSubmissionRow, 0, len(rows))
    for _, row := range rows {
        matchLabel := formatMatchLabel(row.MatchType, row.MatchNumber)
        scoutName := "Unknown"
        if row.ScoutName.Valid && strings.TrimSpace(row.ScoutName.String) != "" {
            scoutName = row.ScoutName.String
        }

        flagLabel := "Clean"
        flagClass := "text-teal-300"
        if strings.TrimSpace(row.AutoPathData.String) == "" {
            flagLabel = "Missing auto note"
            flagClass = "text-yellow-300"
        } else if strings.TrimSpace(row.Notes.String) == "" {
            flagLabel = "Missing notes"
            flagClass = "text-yellow-300"
        }

        submissions = append(submissions, pendingSubmissionRow{
            ID:         row.ID,
            MatchLabel: matchLabel,
            TeamNumber: row.TeamNumber,
            TeamName:   row.TeamName,
            ScoutName:  scoutName,
            FlagLabel:  flagLabel,
            FlagClass:  flagClass,
        })
    }

    return submissions, nil
}

func formatMatchLabel(matchType string, matchNumber int) string {
    normalized := strings.ToLower(strings.TrimSpace(matchType))
    switch normalized {
    case "qualification":
        return fmt.Sprintf("Q%d", matchNumber)
    case "playoff":
        return fmt.Sprintf("P%d", matchNumber)
    default:
        return fmt.Sprintf("M%d", matchNumber)
    }
}

func (h *Handler) loadPickListTeams(c *gin.Context) ([]pickListTeam, error) {
    var teams []pickListTeam
    if err := h.db.WithContext(c.Request.Context()).
        Table("teams").
        Select("team_number").
        Order("team_number").
        Scan(&teams).Error; err != nil {
        return nil, err
    }
    return teams, nil
}

func (h *Handler) HandleApproveSubmission(c *gin.Context) {
    user, err := h.GetSessionUser(c)
    if err != nil || user == nil || !user.IsAdmin {
        http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
        return
    }

    id := c.Param("id")
    if id == "" {
        http.Error(c.Writer, "Submission ID is required", http.StatusBadRequest)
        return
    }

    var submission scoutingSubmission
    if err := h.db.WithContext(c.Request.Context()).
        Where("id = ?", id).
        First(&submission).Error; err != nil {
        http.Error(c.Writer, "Submission not found", http.StatusNotFound)
        return
    }

    approved := scoutingData{
        MatchID:          submission.MatchID,
        TeamID:           submission.TeamID,
        AllianceColor:    submission.AllianceColor,
        AlliancePosition: submission.AlliancePosition,
        AutoScore:        submission.AutoScore,
        TeleopScore:      submission.TeleopScore,
        EndgameScore:     submission.EndgameScore,
        Notes:            submission.Notes,
        StartingPosition: submission.StartingPosition,
        AutoPathData:     submission.AutoPathData,
        DefenseRating:    submission.DefenseRating,
        Traversal:        submission.Traversal,
        Throughput:       submission.Throughput,
        ScoringStrategy:  submission.ScoringStrategy,
        ShootingSpeed:    submission.ShootingSpeed,
        Capacity:         submission.Capacity,
        Defendability:    submission.Defendability,
        HangLevel:        submission.HangLevel,
        AutoHang:         submission.AutoHang,
        HangPosition:     submission.HangPosition,
        ScoutedAt:        submission.ScoutedAt,
        ScouterID:        submission.ScouterID,
    }

    tx := h.db.WithContext(c.Request.Context()).Begin()
    if err := tx.Create(&approved).Error; err != nil {
        tx.Rollback()
        http.Error(c.Writer, "Failed to approve submission", http.StatusInternalServerError)
        return
    }
    if err := tx.Delete(&scoutingSubmission{}, submission.ID).Error; err != nil {
        tx.Rollback()
        http.Error(c.Writer, "Failed to remove pending submission", http.StatusInternalServerError)
        return
    }
    if err := tx.Commit().Error; err != nil {
        http.Error(c.Writer, "Failed to finalize approval", http.StatusInternalServerError)
        return
    }

    c.Status(http.StatusNoContent)
}

func (h *Handler) HandleDeclineSubmission(c *gin.Context) {
    user, err := h.GetSessionUser(c)
    if err != nil || user == nil || !user.IsAdmin {
        http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
        return
    }

    id := c.Param("id")
    if id == "" {
        http.Error(c.Writer, "Submission ID is required", http.StatusBadRequest)
        return
    }

    if err := h.db.WithContext(c.Request.Context()).
        Where("id = ?", id).
        Delete(&scoutingSubmission{}).Error; err != nil {
        http.Error(c.Writer, "Failed to decline submission", http.StatusInternalServerError)
        return
    }

    c.Status(http.StatusNoContent)
}

