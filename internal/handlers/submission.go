package handlers

import (
	"fmt"
	"net/http"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/frc10101/TealTeam/internal/frc"
	"github.com/frc10101/TealTeam/internal/models"
	"github.com/gin-gonic/gin"
)

type scoutingFormInput struct {
	EventID          int
	TeamID           int
	AllianceColor    string
	Notes            string
	StartingPosition string
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

type scoutingData struct {
	ID               int       `gorm:"column:id;primaryKey"`
	EventID          int       `gorm:"column:event_id"`
	TeamID           int       `gorm:"column:team_id"`
	AllianceColor    string    `gorm:"column:alliance_color"`
	AutoScore        int       `gorm:"column:auto_score"`
	TeleopScore      int       `gorm:"column:teleop_score"`
	EndgameScore     int       `gorm:"column:endgame_score"`
	Notes            string    `gorm:"column:notes"`
	StartingPosition string    `gorm:"column:starting_position"`
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

type scoutingSubmission struct {
	ID               int       `gorm:"column:id;primaryKey"`
	EventID          int       `gorm:"column:event_id"`
	TeamID           int       `gorm:"column:team_id"`
	AllianceColor    string    `gorm:"column:alliance_color"`
	AutoScore        int       `gorm:"column:auto_score"`
	TeleopScore      int       `gorm:"column:teleop_score"`
	EndgameScore     int       `gorm:"column:endgame_score"`
	Notes            string    `gorm:"column:notes"`
	StartingPosition string    `gorm:"column:starting_position"`
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
	CreatedAt        time.Time `gorm:"column:created_at"`
}

func (scoutingSubmission) TableName() string { return "scouting_submissions" }

func (h *Handler) buildSubmissionPageData(c *gin.Context, user *models.User) map[string]any {
	data := map[string]any{
		"Title":       "Scouting Submission",
		"Description": "Submit scouting data for competitions",
		"User":        user,
	}

	if h.hasDB() {
		session, err := h.GetSession(c)
		if err == nil && session.SelectedEventID != nil {
			data["SelectedEventID"] = *session.SelectedEventID
		}

		// Don't load teams on initial page load - they'll be fetched via HTMX when event is selected
		// This prevents showing teams when no event has been selected yet

		// Filter events based on user's team registration
		ctx := c.Request.Context()
		eventIDs, err := h.GetAvailableEventsForUser(ctx, user)
		if err == nil && len(eventIDs) > 0 {
			var events []struct {
				ID        int
				Name      string
				StartDate *time.Time
			}
			if err := h.db.WithContext(ctx).Table("events").
				Select("id, name, start_date").
				Where("id IN ?", eventIDs).
				Order("start_date").
				Scan(&events).Error; err == nil {
				data["Events"] = events
			}
		}
	}

	return data
}

func (h *Handler) HandleSubmission(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}

	if !h.hasDB() {
		if c.GetHeader("HX-Request") == "true" {
			data := h.buildSubmissionPageData(c, user)
			data["SubmissionError"] = "Database not connected"
			h.renderPartial(c, "submission_panel", data)
			return
		}
		http.Error(c.Writer, "Database not connected", http.StatusServiceUnavailable)
		return
	}

	input, err := parseScoutingForm(c)
	if err != nil {
		if c.GetHeader("HX-Request") == "true" {
			data := h.buildSubmissionPageData(c, user)
			data["SubmissionError"] = err.Error()
			h.renderPartial(c, "submission_panel", data)
			return
		}
		http.Error(c.Writer, err.Error(), http.StatusBadRequest)
		return
	}

	ctx := c.Request.Context()

	submission := scoutingSubmission{
		EventID:          input.EventID,
		TeamID:           input.TeamID,
		AllianceColor:    input.AllianceColor,
		AutoScore:        0,
		TeleopScore:      0,
		EndgameScore:     0,
		Notes:            input.Notes,
		StartingPosition: input.StartingPosition,
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
		if c.GetHeader("HX-Request") == "true" {
			data := h.buildSubmissionPageData(c, user)
			data["SubmissionError"] = fmt.Sprintf("Failed to queue submission: %v", err)
			h.renderPartial(c, "submission_panel", data)
			return
		}
		http.Error(c.Writer, fmt.Sprintf("Failed to queue submission: %v", err), http.StatusInternalServerError)
		return
	}

	if c.GetHeader("HX-Request") == "true" {
		data := h.buildSubmissionPageData(c, user)
		data["SubmissionSuccess"] = "Submission queued for team scouting. Thanks for scouting!"
		h.renderPartial(c, "submission_panel", data)
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

	input.EventID = eventID
	input.TeamID = teamID
	input.AllianceColor = strings.ToLower(strings.TrimSpace(c.PostForm("alliance_color")))
	input.Notes = strings.TrimSpace(c.PostForm("notes"))
	input.StartingPosition = strings.ToLower(strings.TrimSpace(c.PostForm("starting_position")))
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

// HandleGetEventTeams returns teams participating in a selected event for HTMX
func (h *Handler) HandleGetEventTeams(c *gin.Context) {
	if !h.hasDB() {
		c.String(http.StatusServiceUnavailable, "Database not connected")
		return
	}

	eventIDStr := c.Query("event_id")
	if eventIDStr == "" {
		c.String(http.StatusBadRequest, "event_id is required")
		return
	}

	eventID, err := strconv.Atoi(eventIDStr)
	if err != nil {
		c.String(http.StatusBadRequest, "event_id must be a number")
		return
	}

	ctx := c.Request.Context()

	// Get event code from database
	var event struct {
		TBAKey string
	}
	if err := h.db.WithContext(ctx).Table("events").Select("tba_key").Where("id = ?", eventID).Scan(&event).Error; err != nil {
		c.String(http.StatusInternalServerError, "Failed to fetch event")
		return
	}

	// Get teams from database first
	var teams []struct {
		ID         int
		TeamNumber int
		Name       string
	}

	query := h.db.WithContext(ctx).
		Table("teams").
		Select("teams.id, teams.team_number, teams.name").
		Joins("JOIN event_teams ON teams.id = event_teams.team_id").
		Where("event_teams.event_id = ?", eventID).
		Order("teams.team_number")

	if err := query.Scan(&teams).Error; err == nil && len(teams) > 0 {
		// We have teams in the database, render them
		html := `<option value="" disabled selected>Select team</option>`
		for _, team := range teams {
			html += fmt.Sprintf(`<option value="%d">%d - %s</option>`, team.ID, team.TeamNumber, team.Name)
		}
		c.Header("Content-Type", "text/html; charset=utf-8")
		c.String(http.StatusOK, html)
		return
	}

	// No teams in database, try FIRST API
	username := strings.TrimSpace(os.Getenv("FIRST_API_USERNAME"))
	key := strings.TrimSpace(os.Getenv("FIRST_API_KEY"))
	if username == "" || key == "" {
		c.String(http.StatusInternalServerError, "FIRST API credentials not configured")
		return
	}

	// Extract season from year in event code if possible
	season := 2026
	client := frc.NewClient(username, key)
	firstTeams, err := client.GetEventTeams(ctx, season, event.TBAKey)
	if err != nil {
		c.String(http.StatusInternalServerError, "Failed to fetch teams from FIRST API")
		return
	}

	// Upsert teams into database and build options list
	html := `<option value="" disabled selected>Select team</option>`
	if len(firstTeams) > 0 {
		for _, firstTeam := range firstTeams {
			// Upsert team into database
			dbTeam := struct {
				ID int
			}{}
			name := strings.TrimSpace(firstTeam.NameShort)
			if name == "" {
				name = strings.TrimSpace(firstTeam.NameFull)
			}

			result := h.db.WithContext(ctx).Table("teams").
				Where("team_number = ?", firstTeam.TeamNumber).
				Assign(map[string]interface{}{
					"team_number": firstTeam.TeamNumber,
					"name":        name,
					"school_name": firstTeam.SchoolName,
					"city":        firstTeam.City,
					"state":       firstTeam.StateProv,
					"country":     firstTeam.Country,
					"rookie_year": firstTeam.RookieYear,
					"website":     firstTeam.Website,
				}).
				FirstOrCreate(&dbTeam)

			if result.Error == nil && dbTeam.ID > 0 {
				html += fmt.Sprintf(`<option value="%d">%d - %s</option>`, dbTeam.ID, firstTeam.TeamNumber, name)
			}
		}
	} else {
		html += `<option value="" disabled>No teams available for this event</option>`
	}

	c.Header("Content-Type", "text/html; charset=utf-8")
	c.String(http.StatusOK, html)
}
