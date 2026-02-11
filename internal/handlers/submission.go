package handlers

import (
	"context"
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
)

type scoutingFormInput struct {
	EventID          int
	TeamID           int
	AllianceColor    string
	AlliancePosition int
	MatchType        string
	Notes            string
	StartingPosition string
	AutoPathData     string
	DefenseRating    string
	Traversal        string
	ShootingSpeed    string
	Capacity         string
	Defendability    string
	TeleopStrategy   string
	HangLevel        string
	AutoHang         string
	HangPosition     string
}

type scoutingAPIMetrics struct {
	AutoScore    int
	TeleopScore  int
	EndgameScore int
}

type scoutingData struct {
	ID               int       `gorm:"column:id;primaryKey"`
	MatchID          int       `gorm:"column:match_id"`
	TeamID           int       `gorm:"column:team_id"`
	AllianceColor    string    `gorm:"column:alliance_color"`
	AlliancePosition int       `gorm:"column:alliance_position"`
	AutoScore        int       `gorm:"column:auto_score"`
	TeleopScore      int       `gorm:"column:teleop_score"`
	EndgameScore     int       `gorm:"column:endgame_score"`
	Notes            string    `gorm:"column:notes"`
	StartingPosition string    `gorm:"column:starting_position"`
	AutoPathData     string    `gorm:"column:auto_path_data;type:jsonb"`
	DefenseRating    string    `gorm:"column:defense_rating"`
	Traversal        string    `gorm:"column:traversal"`
	Throughput       string    `gorm:"column:throughput"`
	ScoringStrategy  string    `gorm:"column:scoring_strategy"`
	ShootingSpeed    string    `gorm:"column:shooting_speed"`
	Capacity         string    `gorm:"column:capacity"`
	Defendability    string    `gorm:"column:defendability"`
	HangLevel        string    `gorm:"column:hang_level"`
	AutoHang         string    `gorm:"column:auto_hang"`
	HangPosition     string    `gorm:"column:hang_position"`
	ScoutedAt        time.Time `gorm:"column:scouted_at"`
	ScouterID        *int      `gorm:"column:scouter_id"`
}

func (scoutingData) TableName() string { return "scouting_data" }

type matchInfo struct {
	ID          int
	MatchNumber int
}

func (h *Handler) HandleSubmission(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}

	if !h.hasDB() {
		http.Error(c.Writer, "Database not connected", http.StatusServiceUnavailable)
		return
	}

	input, err := parseScoutingForm(c)
	if err != nil {
		http.Error(c.Writer, err.Error(), http.StatusBadRequest)
		return
	}

	ctx := c.Request.Context()
	match, err := h.findNextMatchForTeam(ctx, input.EventID, input.MatchType, input.TeamID, input.AllianceColor, input.AlliancePosition)
	if err != nil {
		http.Error(c.Writer, err.Error(), http.StatusBadRequest)
		return
	}

	metrics, err := h.deriveScoringMetrics(ctx, input.EventID, match.ID, input.TeamID)
	if err != nil {
		http.Error(c.Writer, err.Error(), http.StatusBadRequest)
		return
	}

	submission := scoutingData{
		MatchID:          match.ID,
		TeamID:           input.TeamID,
		AllianceColor:    input.AllianceColor,
		AlliancePosition: input.AlliancePosition,
		AutoScore:        metrics.AutoScore,
		TeleopScore:      metrics.TeleopScore,
		EndgameScore:     metrics.EndgameScore,
		Notes:            input.Notes,
		StartingPosition: input.StartingPosition,
		AutoPathData:     input.AutoPathData,
		DefenseRating:    input.DefenseRating,
		Traversal:        input.Traversal,
		Throughput:       "",
		ScoringStrategy:  input.TeleopStrategy,
		ShootingSpeed:    input.ShootingSpeed,
		Capacity:         input.Capacity,
		Defendability:    input.Defendability,
		HangLevel:        input.HangLevel,
		AutoHang:         input.AutoHang,
		HangPosition:     input.HangPosition,
		ScoutedAt:        time.Now().UTC(),
		ScouterID:        &user.ID,
	}

	if err := h.db.WithContext(ctx).Create(&submission).Error; err != nil {
		http.Error(c.Writer, fmt.Sprintf("Failed to save submission: %v", err), http.StatusInternalServerError)
		return
	}

	http.Redirect(c.Writer, c.Request, "/submission", http.StatusSeeOther)
}

func parseScoutingForm(c *gin.Context) (scoutingFormInput, error) {
	input := scoutingFormInput{}

	eventID, err := parseRequiredInt(c, "event_id")
	if err != nil {
		return input, err
	}
	teamID, err := parseRequiredInt(c, "team_id")
	if err != nil {
		return input, err
	}
	alliancePosition, err := parseRequiredInt(c, "alliance_position")
	if err != nil {
		return input, err
	}

	input.EventID = eventID
	input.TeamID = teamID
	input.AlliancePosition = alliancePosition
	input.AllianceColor = strings.ToLower(strings.TrimSpace(c.PostForm("alliance_color")))
	input.MatchType = strings.ToLower(strings.TrimSpace(c.PostForm("match_type")))
	input.Notes = strings.TrimSpace(c.PostForm("notes"))
	input.StartingPosition = strings.ToLower(strings.TrimSpace(c.PostForm("starting_position")))
	input.AutoPathData = strings.TrimSpace(c.PostForm("auto_path_data"))
	input.DefenseRating = strings.ToLower(strings.TrimSpace(c.PostForm("defense_rating")))
	input.Traversal = strings.ToLower(strings.TrimSpace(c.PostForm("traversal")))
	input.ShootingSpeed = strings.ToLower(strings.TrimSpace(c.PostForm("shooting_speed")))
	input.Capacity = strings.ToLower(strings.TrimSpace(c.PostForm("capacity")))
	input.Defendability = strings.TrimSpace(c.PostForm("defendability"))
	input.TeleopStrategy = strings.ToLower(strings.TrimSpace(c.PostForm("teleop_strategy")))
	input.HangLevel = strings.ToLower(strings.TrimSpace(c.PostForm("hang_level")))
	input.AutoHang = strings.ToLower(strings.TrimSpace(c.PostForm("auto_hang")))
	input.HangPosition = strings.ToLower(strings.TrimSpace(c.PostForm("hang_position")))

	if input.AllianceColor == "" {
		return input, fmt.Errorf("alliance_color is required")
	}
	if input.MatchType == "" {
		return input, fmt.Errorf("match_type is required")
	}
	if input.StartingPosition == "" {
		return input, fmt.Errorf("starting_position is required")
	}

	return input, nil
}

func parseRequiredInt(c *gin.Context, field string) (int, error) {
	value := strings.TrimSpace(c.PostForm(field))
	if value == "" {
		return 0, fmt.Errorf("%s is required", field)
	}
	parsed, err := strconv.Atoi(value)
	if err != nil {
		return 0, fmt.Errorf("%s must be a number", field)
	}
	return parsed, nil
}

func (h *Handler) findNextMatchForTeam(ctx context.Context, eventID int, matchType string, teamID int, allianceColor string, alliancePosition int) (*matchInfo, error) {
	var scoutedCount int64
	if err := h.db.WithContext(ctx).
		Table("scouting_data").
		Joins("JOIN matches ON matches.id = scouting_data.match_id").
		Where("scouting_data.team_id = ? AND matches.event_id = ? AND matches.match_type = ?", teamID, eventID, matchType).
		Count(&scoutedCount).Error; err != nil {
		return nil, fmt.Errorf("failed to check existing submissions: %w", err)
	}

	var matches []matchInfo
	if err := h.db.WithContext(ctx).
		Table("matches").
		Select("id, match_number").
		Where("event_id = ? AND match_type = ?", eventID, matchType).
		Order("match_number").
		Find(&matches).Error; err != nil {
		return nil, fmt.Errorf("failed to load matches: %w", err)
	}
	if len(matches) == 0 {
		return nil, fmt.Errorf("no matches available for the selected event")
	}

	startIndex := int(scoutedCount)
	if startIndex < 0 {
		startIndex = 0
	}

	for i := startIndex; i < len(matches); i++ {
		match := matches[i]

		var existingCount int64
		if err := h.db.WithContext(ctx).
			Table("scouting_data").
			Where("match_id = ? AND team_id = ?", match.ID, teamID).
			Count(&existingCount).Error; err != nil {
			return nil, fmt.Errorf("failed to validate team match: %w", err)
		}
		if existingCount > 0 {
			continue
		}

		if err := h.db.WithContext(ctx).
			Table("scouting_data").
			Where("match_id = ? AND alliance_color = ? AND alliance_position = ?", match.ID, allianceColor, alliancePosition).
			Count(&existingCount).Error; err != nil {
			return nil, fmt.Errorf("failed to validate alliance slot: %w", err)
		}
		if existingCount > 0 {
			continue
		}

		return &match, nil
	}

	return nil, fmt.Errorf("no available match slots for this team")
}

func (h *Handler) deriveScoringMetrics(ctx context.Context, eventID int, matchID int, teamID int) (scoutingAPIMetrics, error) {
	// TODO: Pull from FIRST schedule + TBA stats.
	// For now, return zeroed scores until API integration is implemented.
	return scoutingAPIMetrics{}, nil
}
