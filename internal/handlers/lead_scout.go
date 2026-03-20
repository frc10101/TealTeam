package handlers

import (
	"database/sql"
	"errors"
	"net/http"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/frc10101/TealTeam/internal/frc"
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
	"gorm.io/gorm/clause"
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
	TeamName   string
	Rank       *int
}

type rankingRow struct {
	Rank        int
	TeamNumber  int
	QualAverage float64
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
}

type teamPointSummary struct {
	TeamID     int
	TeamNumber int
	TeamName   string
	Rank       *int
	Points     int
	Matches    int
}

func (h *Handler) loadPendingSubmissions(c *gin.Context) ([]pendingSubmissionRow, error) {
	var rows []struct {
		ID         int
		EventName  string
		TeamNumber int
		TeamName   string
		ScoutName  sql.NullString
		Notes      sql.NullString
	}

	if err := h.db.WithContext(c.Request.Context()).
		Table("scouting_submissions").
		Select("scouting_submissions.id, events.name as event_name, teams.team_number, teams.name as team_name, users.name as scout_name, scouting_submissions.notes").
		Joins("JOIN events ON events.id = scouting_submissions.event_id").
		Joins("JOIN teams ON teams.id = scouting_submissions.team_id").
		Joins("LEFT JOIN users ON users.id = scouting_submissions.scouter_id").
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
	var rows []struct {
		TeamNumber int
		TeamName   string
		Rank       sql.NullInt64
	}
	if err := h.db.WithContext(c.Request.Context()).
		Table("teams").
		Select("teams.team_number, teams.name as team_name, team_event_stats.rank").
		Joins("JOIN event_teams ON event_teams.team_id = teams.id").
		Joins("LEFT JOIN team_event_stats ON team_event_stats.team_id = teams.id AND team_event_stats.event_id = event_teams.event_id").
		Where("event_teams.event_id = ?", eventID).
		Order("team_number").
		Scan(&rows).Error; err != nil {
		return nil, err
	}

	teams := make([]pickListTeam, 0, len(rows))
	for _, row := range rows {
		var rank *int
		if row.Rank.Valid {
			r := int(row.Rank.Int64)
			rank = &r
		}
		teams = append(teams, pickListTeam{
			TeamNumber: row.TeamNumber,
			TeamName:   row.TeamName,
			Rank:       rank,
		})
	}

	return teams, nil
}

func (h *Handler) loadRankingSnapshot(c *gin.Context, eventID int, teamNumber *int) ([]rankingRow, *rankingRow, []rankingRow, error) {
	var topTeams []rankingRow
	if err := h.db.WithContext(c.Request.Context()).
		Table("team_event_stats").
		Select("team_event_stats.rank, teams.team_number, team_event_stats.qual_average").
		Joins("JOIN teams ON teams.id = team_event_stats.team_id").
		Where("team_event_stats.event_id = ? AND team_event_stats.rank IS NOT NULL", eventID).
		Order("team_event_stats.rank").
		Limit(5).
		Scan(&topTeams).Error; err != nil {
		return nil, nil, nil, err
	}

	if teamNumber == nil {
		return topTeams, nil, nil, nil
	}

	var userTeam rankingRow
	err := h.db.WithContext(c.Request.Context()).
		Table("team_event_stats").
		Select("team_event_stats.rank, teams.team_number, team_event_stats.qual_average").
		Joins("JOIN teams ON teams.id = team_event_stats.team_id").
		Where("team_event_stats.event_id = ? AND teams.team_number = ? AND team_event_stats.rank IS NOT NULL", eventID, *teamNumber).
		First(&userTeam).Error
	if err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return topTeams, nil, nil, nil
		}
		return nil, nil, nil, err
	}

	minRank := userTeam.Rank - 2
	if minRank < 1 {
		minRank = 1
	}
	maxRank := userTeam.Rank + 2

	var aroundTeams []rankingRow
	if err := h.db.WithContext(c.Request.Context()).
		Table("team_event_stats").
		Select("team_event_stats.rank, teams.team_number, team_event_stats.qual_average").
		Joins("JOIN teams ON teams.id = team_event_stats.team_id").
		Where("team_event_stats.event_id = ? AND team_event_stats.rank BETWEEN ? AND ? AND team_event_stats.rank IS NOT NULL", eventID, minRank, maxRank).
		Order("team_event_stats.rank").
		Scan(&aroundTeams).Error; err != nil {
		return nil, nil, nil, err
	}

	filtered := make([]rankingRow, 0, len(aroundTeams))
	for _, team := range aroundTeams {
		if team.TeamNumber == userTeam.TeamNumber {
			continue
		}
		filtered = append(filtered, team)
	}

	return topTeams, &userTeam, filtered, nil
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
			scouting_data.starting_position`).
		Where("scouting_data.event_id = ?", eventID).
		Scan(&metrics).Error; err != nil && !errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, err
	}

	pointsByTeam := make(map[int]int)
	matchesByTeam := make(map[int]int)
	for _, row := range metrics {
		pointsByTeam[row.TeamID] += calculateScoutingPoints(row)
		matchesByTeam[row.TeamID]++
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
		ScoutedAt:        submission.ScoutedAt,
		ScouterID:        submission.ScouterID,
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
			h.log.Warn("failed to sync team after approval", "team", team.TeamNumber, "error", err)
		}
	}()

	// Redirect back to lead-scout page to refresh the rankings and submission list
	c.Header("HX-Redirect", "/lead-scout")
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

	if err := h.db.WithContext(c.Request.Context()).
		Where("id = ?", id).
		Delete(&scoutingSubmission{}).Error; err != nil {
		http.Error(c.Writer, "Failed to decline submission", http.StatusInternalServerError)
		return
	}

	// Redirect back to lead-scout page to refresh the submission list
	c.Header("HX-Redirect", "/lead-scout")
}

type submissionDetailRow struct {
	ID               int
	EventName        string
	TeamNumber       int
	TeamName         string
	ScoutName        string
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

// PickListEntry represents a team in the pick list
type pickListEntryDB struct {
	ID               int `gorm:"primaryKey"`
	TeamNumber       int
	EventID          int
	PickedTeamNumber int
	Color            *string
	Crossed          bool
	Position         int
	CreatedAt        time.Time `gorm:"autoCreateTime"`
	UpdatedAt        time.Time `gorm:"autoUpdateTime"`
}

// TableName specifies the table name for GORM
func (pickListEntryDB) TableName() string {
	return "pick_list_entries"
}

// HandleSavePickListEntry saves a pick list entry to the database
func (h *Handler) HandleSavePickListEntry(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		c.JSON(http.StatusUnauthorized, gin.H{"error": "not authenticated"})
		return
	}

	if user.TeamNumber == nil || *user.TeamNumber == 0 {
		c.JSON(http.StatusBadRequest, gin.H{"error": "user has no team assigned"})
		return
	}

	session, err := h.GetSession(c)
	if err != nil || session == nil || session.SelectedEventID == nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "no event selected"})
		return
	}

	var req struct {
		PickedTeamNumber int     `json:"picked_team_number" binding:"required"`
		Color            *string `json:"color"`
		Crossed          bool    `json:"crossed"`
		Position         int     `json:"position"`
	}

	if err := c.BindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "invalid request"})
		return
	}

	ctx := c.Request.Context()
	entry := pickListEntryDB{
		TeamNumber:       *user.TeamNumber,
		EventID:          *session.SelectedEventID,
		PickedTeamNumber: req.PickedTeamNumber,
		Color:            req.Color,
		Crossed:          req.Crossed,
		Position:         req.Position,
	}

	// Upsert the entry using the database unique constraint
	if err := h.db.WithContext(ctx).
		Clauses(clause.OnConflict{
			Columns:   []clause.Column{{Name: "team_number"}, {Name: "event_id"}, {Name: "picked_team_number"}},
			UpdateAll: true,
		}).
		Create(&entry).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "failed to save entry"})
		return
	}

	c.JSON(http.StatusOK, gin.H{"status": "saved"})
}

// HandleDeletePickListEntry deletes a pick list entry from the database
func (h *Handler) HandleDeletePickListEntry(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		c.JSON(http.StatusUnauthorized, gin.H{"error": "not authenticated"})
		return
	}

	if user.TeamNumber == nil || *user.TeamNumber == 0 {
		c.JSON(http.StatusBadRequest, gin.H{"error": "user has no team assigned"})
		return
	}

	session, err := h.GetSession(c)
	if err != nil || session == nil || session.SelectedEventID == nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "no event selected"})
		return
	}

	pickedTeamNumber := c.Query("team")
	if pickedTeamNumber == "" {
		c.JSON(http.StatusBadRequest, gin.H{"error": "team number required"})
		return
	}

	// Convert to int
	pickedTeamNumberInt, parseErr := strconv.Atoi(pickedTeamNumber)
	if parseErr != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "invalid team number"})
		return
	}

	ctx := c.Request.Context()
	if err := h.db.WithContext(ctx).
		Where("team_number = ? AND event_id = ? AND picked_team_number = ?",
			*user.TeamNumber, *session.SelectedEventID, pickedTeamNumberInt).
		Delete(&pickListEntryDB{}).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "failed to delete entry"})
		return
	}

	c.JSON(http.StatusOK, gin.H{"status": "deleted"})
}

// HandleGetPickList retrieves the team's pick list from the database
func (h *Handler) HandleGetPickList(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		c.JSON(http.StatusUnauthorized, gin.H{"error": "not authenticated"})
		return
	}

	if user.TeamNumber == nil || *user.TeamNumber == 0 {
		c.JSON(http.StatusBadRequest, gin.H{"error": "user has no team assigned"})
		return
	}

	session, err := h.GetSession(c)
	if err != nil || session == nil || session.SelectedEventID == nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "no event selected"})
		return
	}

	var entries []pickListEntryDB
	ctx := c.Request.Context()
	if err := h.db.WithContext(ctx).
		Where("team_number = ? AND event_id = ?",
			*user.TeamNumber, *session.SelectedEventID).
		Order("position").
		Find(&entries).Error; err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "failed to load pick list"})
		return
	}

	// Transform to frontend format
	type entry struct {
		PickedTeamNumber int     `json:"picked_team_number"`
		Color            *string `json:"color"`
		Crossed          bool    `json:"crossed"`
		Position         int     `json:"position"`
	}

	result := make([]entry, len(entries))
	for i, e := range entries {
		result[i] = entry{
			PickedTeamNumber: e.PickedTeamNumber,
			Color:            e.Color,
			Crossed:          e.Crossed,
			Position:         e.Position,
		}
	}

	c.JSON(http.StatusOK, gin.H{"entries": result})
}
