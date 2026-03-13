package handlers

import (
	"database/sql"
	"errors"
	"fmt"
	"net/http"
	"sort"
	"strings"

	"github.com/frc10101/TealTeam/internal/frc"
	"github.com/frc10101/TealTeam/internal/models"
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

type pendingSubmissionRow struct {
	ID         int
	EventName  string
	TeamNumber int
	TeamName   string
	ScoutName  string
	FlagLabel  string
	FlagClass  string
}

type pickListTeam struct {
	TeamNumber int
}

type scoutingMetricRow struct {
	TeamID           int
	DefenseRating    string
	Traversal        string
	ShootingSpeed    string
	Capacity         string
	ScoringStrategy  string
	HangLevel        string
	AutoHang         string
	HangPosition     string
	StartingPosition string
	CreatedAt        string
}

type teamPointSummary struct {
	TeamID     int
	TeamNumber int
	TeamName   string
	Rank       *int
	Points     int
	Matches    int
	Strategy   string
}

// canAccessSubmission checks if a user has permission to access a submission
func (h *Handler) canAccessSubmission(c *gin.Context, user *models.User, submissionID string) bool {
	if user.IsAdmin {
		return true // Admins can access all submissions
	}

	if !user.IsLeadScout {
		return false // Only admins and lead scouts can access
	}

	// Lead scouts can only access submissions from their own team
	if user.TeamNumber == nil || *user.TeamNumber <= 0 {
		return false // Lead scout must have a team number
	}

	var submittingTeamNumber *int
	if err := h.db.WithContext(c.Request.Context()).
		Table("scouting_submissions").
		Select("teams.team_number").
		Joins("JOIN teams ON teams.id = scouting_submissions.submitting_team_id").
		Where("scouting_submissions.id = ?", submissionID).
		Scan(&submittingTeamNumber).Error; err != nil {
		return false
	}

	if submittingTeamNumber == nil {
		return false // No submitting team found
	}

	return *submittingTeamNumber == *user.TeamNumber
}

func (h *Handler) loadPendingSubmissions(c *gin.Context, user *models.User) ([]pendingSubmissionRow, error) {
	var rows []struct {
		ID         int
		EventName  string
		TeamNumber int
		TeamName   string
		ScoutName  sql.NullString
		Notes      sql.NullString
	}

	query := h.db.WithContext(c.Request.Context()).
		Table("scouting_submissions").
		Select("scouting_submissions.id, events.name as event_name, teams.team_number, teams.name as team_name, users.name as scout_name, scouting_submissions.notes").
		Joins("JOIN events ON events.id = scouting_submissions.event_id").
		Joins("JOIN teams ON teams.id = scouting_submissions.team_id").
		Joins("LEFT JOIN users ON users.id = scouting_submissions.scouter_id").
		Where("scouting_submissions.status = ?", "pending")

	// Filter by lead scout's team - only see submissions from their team
	if !user.IsAdmin && user.IsLeadScout {
		if user.TeamNumber != nil && *user.TeamNumber > 0 {
			query = query.Joins("JOIN teams AS submitting_team ON submitting_team.id = scouting_submissions.submitting_team_id").
				Where("submitting_team.team_number = ?", *user.TeamNumber)
		}
	}
	// Admins see all submissions

	if err := query.
		Order("scouting_submissions.created_at").
		Scan(&rows).Error; err != nil {
		return nil, err
	}

	submissions := make([]pendingSubmissionRow, 0, len(rows))
	for _, row := range rows {
		scoutName := "Unknown"
		if row.ScoutName.Valid && strings.TrimSpace(row.ScoutName.String) != "" {
			scoutName = row.ScoutName.String
		}

		flagLabel := "Clean"
		flagClass := "text-teal-300"
		if strings.TrimSpace(row.Notes.String) == "" {
			flagLabel = "Missing notes"
			flagClass = "text-yellow-300"
		}

		submissions = append(submissions, pendingSubmissionRow{
			ID:         row.ID,
			EventName:  row.EventName,
			TeamNumber: row.TeamNumber,
			TeamName:   row.TeamName,
			ScoutName:  scoutName,
			FlagLabel:  flagLabel,
			FlagClass:  flagClass,
		})
	}

	return submissions, nil
}

func (h *Handler) loadPickListTeams(c *gin.Context, eventID int) ([]pickListTeam, error) {
	var teams []pickListTeam
	if err := h.db.WithContext(c.Request.Context()).
		Table("teams").
		Select("team_number").
		Joins("JOIN event_teams ON event_teams.team_id = teams.id").
		Where("event_teams.event_id = ?", eventID).
		Order("team_number").
		Scan(&teams).Error; err != nil {
		return nil, err
	}
	return teams, nil
}

func (h *Handler) loadTeamPointRankings(c *gin.Context, eventID int, sortKey string) ([]teamPointSummary, error) {
	var teams []struct {
		TeamID     int
		TeamNumber int
		TeamName   string
		Rank       sql.NullInt64
	}

	if err := h.db.WithContext(c.Request.Context()).
		Table("teams").
		Select("teams.id as team_id, teams.team_number, teams.name as team_name, team_event_stats.rank").
		Joins("JOIN event_teams ON event_teams.team_id = teams.id").
		Joins("LEFT JOIN team_event_stats ON team_event_stats.team_id = teams.id AND team_event_stats.event_id = event_teams.event_id").
		Where("event_teams.event_id = ?", eventID).
		Order("teams.team_number").
		Scan(&teams).Error; err != nil {
		return nil, err
	}

	var metrics []scoutingMetricRow
	if err := h.db.WithContext(c.Request.Context()).
		Table("scouting_data").
		Select(`scouting_data.team_id,
			scouting_data.defense_rating,
			scouting_data.traversal,
			scouting_data.shooting_speed,
			scouting_data.capacity,
			scouting_data.scoring_strategy,
			scouting_data.hang_level,
			scouting_data.auto_hang,
			scouting_data.hang_position,
			scouting_data.starting_position,
			scouting_data.created_at`).
		Where("scouting_data.event_id = ?", eventID).
		Scan(&metrics).Error; err != nil && !errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, err
	}

	pointsByTeam := make(map[int]int)
	matchesByTeam := make(map[int]int)
	pointConfig := h.loadEffectiveScoutingPointConfig(c)
	// Track strategy frequency and most recent for each team
	type strategyInfo struct {
		count     int
		createdAt string
	}
	strategiesByTeam := make(map[int]map[string]strategyInfo)
	for _, row := range metrics {
		pointsByTeam[row.TeamID] += calculateScoutingPointsWithConfig(row, pointConfig)
		matchesByTeam[row.TeamID]++
		// Track strategy if it's not empty
		if row.ScoringStrategy != "" {
			if strategiesByTeam[row.TeamID] == nil {
				strategiesByTeam[row.TeamID] = make(map[string]strategyInfo)
			}
			current := strategiesByTeam[row.TeamID][row.ScoringStrategy]
			current.count++
			// Keep the most recent created_at for this strategy
			if row.CreatedAt > current.createdAt {
				current.createdAt = row.CreatedAt
			}
			strategiesByTeam[row.TeamID][row.ScoringStrategy] = current
		}
	}

	// Determine most common (or most recent if tie) strategy for each team
	mostCommonStrategyByTeam := make(map[int]string)
	for teamID, strategies := range strategiesByTeam {
		var bestStrategy string
		var bestCount int
		var bestCreatedAt string
		for strategy, info := range strategies {
			// Choose this strategy if it has more occurrences, or if tied, if it's more recent
			if info.count > bestCount || (info.count == bestCount && info.createdAt > bestCreatedAt) {
				bestStrategy = strategy
				bestCount = info.count
				bestCreatedAt = info.createdAt
			}
		}
		mostCommonStrategyByTeam[teamID] = bestStrategy
	}

	summaries := make([]teamPointSummary, 0, len(teams))
	for _, team := range teams {
		var rankPtr *int
		if team.Rank.Valid {
			rankValue := int(team.Rank.Int64)
			rankPtr = &rankValue
		}

		summaries = append(summaries, teamPointSummary{
			TeamID:     team.TeamID,
			TeamNumber: team.TeamNumber,
			TeamName:   team.TeamName,
			Rank:       rankPtr,
			Points:     pointsByTeam[team.TeamID],
			Matches:    matchesByTeam[team.TeamID],
			Strategy:   mostCommonStrategyByTeam[team.TeamID],
		})
	}

	sortKey = strings.ToLower(strings.TrimSpace(sortKey))
	if sortKey == "" {
		sortKey = "rank"
	}

	sort.SliceStable(summaries, func(i, j int) bool {
		left := summaries[i]
		right := summaries[j]

		switch sortKey {
		case "points":
			if left.Points != right.Points {
				return left.Points > right.Points
			}
		case "name":
			leftName := strings.ToLower(strings.TrimSpace(left.TeamName))
			rightName := strings.ToLower(strings.TrimSpace(right.TeamName))
			if leftName != rightName {
				return leftName < rightName
			}
		case "number":
			if left.TeamNumber != right.TeamNumber {
				return left.TeamNumber < right.TeamNumber
			}
		default:
			if left.Rank != nil || right.Rank != nil {
				if left.Rank == nil {
					return false
				}
				if right.Rank == nil {
					return true
				}
				if *left.Rank != *right.Rank {
					return *left.Rank < *right.Rank
				}
			}
		}

		if left.TeamNumber != right.TeamNumber {
			return left.TeamNumber < right.TeamNumber
		}
		return left.TeamName < right.TeamName
	})

	return summaries, nil
}

func (h *Handler) HandleApproveSubmission(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil || (!user.IsAdmin && !user.IsLeadScout) {
		http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
		return
	}

	id := c.Param("id")
	if id == "" {
		http.Error(c.Writer, "Submission ID is required", http.StatusBadRequest)
		return
	}

	// Check if user has access to this submission
	if !h.canAccessSubmission(c, user, id) {
		http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
		return
	}

	ctx := c.Request.Context()

	var submission scoutingSubmission
	if err := h.db.WithContext(ctx).
		Where("id = ?", id).
		First(&submission).Error; err != nil {
		http.Error(c.Writer, "Submission not found", http.StatusNotFound)
		return
	}

	// Get the team number for the FIRST API update
	var team struct {
		TeamNumber int
	}
	if err := h.db.WithContext(ctx).
		Table("teams").
		Select("team_number").
		Where("id = ?", submission.TeamID).
		First(&team).Error; err != nil {
		http.Error(c.Writer, "Team not found", http.StatusInternalServerError)
		return
	}

	approved := scoutingData{
		EventID:          submission.EventID,
		TeamID:           submission.TeamID,
		AllianceColor:    submission.AllianceColor,
		Notes:            submission.Notes,
		StartingPosition: submission.StartingPosition,
		DefenseRating:    submission.DefenseRating,
		Traversal:        submission.Traversal,
		ScoringStrategy:  submission.ScoringStrategy,
		ShootingSpeed:    submission.ShootingSpeed,
		Capacity:         submission.Capacity,
		Defendability:    submission.Defendability,
		HangLevel:        submission.HangLevel,
		AutoHang:         submission.AutoHang,
		HangPosition:     submission.HangPosition,
		AccuracyRating:   submission.AccuracyRating,
		ScoutedAt:        submission.ScoutedAt,
		ScouterID:        submission.ScouterID,
		SubmittingTeamID: submission.SubmittingTeamID,
	}

	tx := h.db.WithContext(ctx).Begin()
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

	// After successful approval, sync the team's information from FIRST API
	go func() {
		_, err := frc.SyncTeamForUser(ctx, h.db, team.TeamNumber)
		if err != nil {
			// Log the error but don't fail the request - the submission was already approved
			fmt.Printf("warning: failed to sync team %d after approval: %v\n", team.TeamNumber, err)
		}
	}()

	// Redirect back to lead-scout page
	if c.GetHeader("HX-Request") == "true" {
		c.Header("HX-Redirect", "/lead-scout")
	} else {
		c.Redirect(http.StatusSeeOther, "/lead-scout")
	}
}

func (h *Handler) HandleDeclineSubmission(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil || (!user.IsAdmin && !user.IsLeadScout) {
		http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
		return
	}

	id := c.Param("id")
	if id == "" {
		http.Error(c.Writer, "Submission ID is required", http.StatusBadRequest)
		return
	}

	// Check if user has access to this submission
	if !h.canAccessSubmission(c, user, id) {
		http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
		return
	}

	reason := strings.TrimSpace(c.PostForm("reason"))
	if reason == "" {
		reason = strings.TrimSpace(c.GetHeader("HX-Prompt"))
	}

	if err := h.db.WithContext(c.Request.Context()).
		Table("scouting_submissions").
		Where("id = ?", id).
		Updates(map[string]any{
			"status":           "rejected",
			"rejection_reason": reason,
		}).Error; err != nil {
		http.Error(c.Writer, "Failed to decline submission", http.StatusInternalServerError)
		return
	}

	// Redirect back to lead-scout page
	if c.GetHeader("HX-Request") == "true" {
		c.Header("HX-Redirect", "/lead-scout")
	} else {
		c.Redirect(http.StatusSeeOther, "/lead-scout")
	}
}

type submissionDetailRow struct {
	ID               int
	EventName        string
	TeamNumber       int
	TeamName         string
	ScoutName        string
	AllianceColor    string
	AutoScore        int
	TeleopScore      int
	EndgameScore     int
	Notes            string
	StartingPosition string
	DefenseRating    string
	Traversal        string
	Throughput       string
	ScoringStrategy  string
	ShootingSpeed    string
	Capacity         string
	Defendability    string
	HangLevel        string
	AutoHang         string
	HangPosition     string
	AccuracyRating   string
	FlagLabel        string
	FlagClass        string
	CreatedAt        string
}

func (h *Handler) HandleViewSubmission(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil || (!user.IsAdmin && !user.IsLeadScout) {
		http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
		return
	}

	id := c.Param("id")
	if id == "" {
		http.Error(c.Writer, "Submission ID is required", http.StatusBadRequest)
		return
	}

	// Check if user has access to this submission
	if !h.canAccessSubmission(c, user, id) {
		http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
		return
	}

	ctx := c.Request.Context()

	var submission struct {
		ID               int
		EventName        string
		TeamNumber       int
		TeamName         string
		ScoutName        sql.NullString
		AllianceColor    string
		Notes            string
		StartingPosition string
		DefenseRating    string
		Traversal        string
		ScoringStrategy  string
		ShootingSpeed    string
		Capacity         string
		Defendability    string
		HangLevel        string
		AutoHang         string
		HangPosition     string
		AccuracyRating   string
		CreatedAt        string
	}

	if err := h.db.WithContext(ctx).
		Table("scouting_submissions").
		Select(`scouting_submissions.id, 
			events.name as event_name, 
			teams.team_number, 
			teams.name as team_name, 
			users.name as scout_name,
			scouting_submissions.alliance_color,
			scouting_submissions.notes,
			scouting_submissions.starting_position,
			scouting_submissions.defense_rating,
			scouting_submissions.traversal,
			scouting_submissions.scoring_strategy,
			scouting_submissions.shooting_speed,
			scouting_submissions.capacity,
			scouting_submissions.defendability,
			scouting_submissions.hang_level,
			scouting_submissions.auto_hang,
			scouting_submissions.hang_position,
			scouting_submissions.accuracy_rating,
			TO_CHAR(scouting_submissions.created_at, 'YYYY-MM-DD HH24:MI:SS') as created_at`).
		Joins("JOIN events ON events.id = scouting_submissions.event_id").
		Joins("JOIN teams ON teams.id = scouting_submissions.team_id").
		Joins("LEFT JOIN users ON users.id = scouting_submissions.scouter_id").
		Where("scouting_submissions.id = ?", id).
		First(&submission).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			data := map[string]any{
				"Title": "Submission Details",
				"User":  user,
			}
			h.render(c, "submission_detail", data)
			return
		}
		http.Error(c.Writer, "Failed to load submission", http.StatusInternalServerError)
		return
	}

	scoutName := "Unknown"
	if submission.ScoutName.Valid && strings.TrimSpace(submission.ScoutName.String) != "" {
		scoutName = submission.ScoutName.String
	}

	flagLabel := "Clean"
	flagClass := "text-teal-300"
	if strings.TrimSpace(submission.Notes) == "" {
		flagLabel = "Missing notes"
		flagClass = "text-yellow-300"
	}

	detail := submissionDetailRow{
		ID:               submission.ID,
		EventName:        submission.EventName,
		TeamNumber:       submission.TeamNumber,
		TeamName:         submission.TeamName,
		ScoutName:        scoutName,
		AllianceColor:    submission.AllianceColor,
		Notes:            submission.Notes,
		StartingPosition: submission.StartingPosition,
		DefenseRating:    submission.DefenseRating,
		Traversal:        submission.Traversal,
		ScoringStrategy:  submission.ScoringStrategy,
		ShootingSpeed:    submission.ShootingSpeed,
		Capacity:         submission.Capacity,
		Defendability:    submission.Defendability,
		HangLevel:        submission.HangLevel,
		AutoHang:         submission.AutoHang,
		HangPosition:     submission.HangPosition,
		AccuracyRating:   submission.AccuracyRating,
		FlagLabel:        flagLabel,
		FlagClass:        flagClass,
		CreatedAt:        submission.CreatedAt,
	}

	data := map[string]any{
		"Title":      "Submission Details",
		"User":       user,
		"Submission": detail,
	}

	h.render(c, "submission_detail", data)
}
