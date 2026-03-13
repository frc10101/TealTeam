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
	AccuracyRating   string
}

type scoutingData struct {
	ID               int       `gorm:"column:id;primaryKey"`
	EventID          int       `gorm:"column:event_id"`
	TeamID           int       `gorm:"column:team_id"`
	AllianceColor    string    `gorm:"column:alliance_color"`
	Notes            string    `gorm:"column:notes"`
	StartingPosition string    `gorm:"column:starting_position"`
	DefenseRating    string    `gorm:"column:defense_rating"`
	Traversal        string    `gorm:"column:traversal"`
	ScoringStrategy  string    `gorm:"column:scoring_strategy"`
	ShootingSpeed    string    `gorm:"column:shooting_speed"`
	Capacity         string    `gorm:"column:capacity"`
	Defendability    string    `gorm:"column:defendability"`
	HangLevel        string    `gorm:"column:hang_level"`
	AutoHang         string    `gorm:"column:auto_hang"`
	HangPosition     string    `gorm:"column:hang_position"`
	AccuracyRating   string    `gorm:"column:accuracy_rating"`
	ScoutedAt        time.Time `gorm:"column:scouted_at"`
	ScouterID        *int      `gorm:"column:scouter_id"`
	SubmittingTeamID *int      `gorm:"column:submitting_team_id"`
}

func (scoutingData) TableName() string { return "scouting_data" }

type scoutingSubmission struct {
	ID               int       `gorm:"column:id;primaryKey"`
	EventID          int       `gorm:"column:event_id"`
	TeamID           int       `gorm:"column:team_id"`
	AllianceColor    string    `gorm:"column:alliance_color"`
	Notes            string    `gorm:"column:notes"`
	StartingPosition string    `gorm:"column:starting_position"`
	DefenseRating    string    `gorm:"column:defense_rating"`
	Traversal        string    `gorm:"column:traversal"`
	ScoringStrategy  string    `gorm:"column:scoring_strategy"`
	ShootingSpeed    string    `gorm:"column:shooting_speed"`
	Capacity         string    `gorm:"column:capacity"`
	Defendability    string    `gorm:"column:defendability"`
	HangLevel        string    `gorm:"column:hang_level"`
	AutoHang         string    `gorm:"column:auto_hang"`
	HangPosition     string    `gorm:"column:hang_position"`
	AccuracyRating   string    `gorm:"column:accuracy_rating"`
	ScoutedAt        time.Time `gorm:"column:scouted_at"`
	ScouterID        *int      `gorm:"column:scouter_id"`
	SubmittingTeamID *int      `gorm:"column:submitting_team_id"`
	Status           string    `gorm:"column:status"`
	RejectionReason  string    `gorm:"column:rejection_reason"`
	CreatedAt        time.Time `gorm:"column:created_at"`
}

func (scoutingSubmission) TableName() string { return "scouting_submissions" }

func (h *Handler) buildSubmissionPageData(c *gin.Context, user *models.User) map[string]any {
	data := map[string]any{
		"Title":       "Scouting Submission",
		"Description": "Collect and submit match data for teams at competitions.",
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

		// Load rejected submissions for this user
		var rejected []struct {
			ID              int
			EventName       string
			TeamNumber      int
			TeamName        string
			RejectionReason string
			Notes           string
			CreatedAt       time.Time
		}
		if err := h.db.WithContext(ctx).
			Table("scouting_submissions").
			Select("scouting_submissions.id, events.name as event_name, teams.team_number, teams.name as team_name, scouting_submissions.rejection_reason, scouting_submissions.notes, scouting_submissions.created_at").
			Joins("JOIN events ON events.id = scouting_submissions.event_id").
			Joins("JOIN teams ON teams.id = scouting_submissions.team_id").
			Where("scouting_submissions.scouter_id = ? AND scouting_submissions.status = ?", user.ID, "rejected").
			Order("scouting_submissions.created_at DESC").
			Scan(&rejected).Error; err == nil && len(rejected) > 0 {
			data["RejectedSubmissions"] = rejected
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

	// Get the scouter's team ID for submission tracking
	var submittingTeamID *int
	if user.TeamNumber != nil && *user.TeamNumber > 0 {
		var team struct {
			ID int
		}
		if err := h.db.WithContext(ctx).
			Table("teams").
			Select("id").
			Where("team_number = ?", *user.TeamNumber).
			First(&team).Error; err == nil {
			submittingTeamID = &team.ID
		}
	}

	submission := scoutingSubmission{
		EventID:          input.EventID,
		TeamID:           input.TeamID,
		AllianceColor:    input.AllianceColor,
		Notes:            input.Notes,
		StartingPosition: input.StartingPosition,
		DefenseRating:    input.DefenseRating,
		Traversal:        input.Traversal,
		ScoringStrategy:  input.TeleopStrategy,
		ShootingSpeed:    input.ShootingSpeed,
		Capacity:         input.Capacity,
		Defendability:    input.Defendability,
		HangLevel:        input.HangLevel,
		AutoHang:         input.AutoHang,
		HangPosition:     input.HangPosition,
		AccuracyRating:   input.AccuracyRating,
		ScoutedAt:        time.Now().UTC(),
		ScouterID:        &user.ID,
		SubmittingTeamID: submittingTeamID,
	}

	if err := h.db.WithContext(ctx).Create(&submission).Error; err != nil {
		h.log.Error("failed to create scouting submission", "event_id", input.EventID, "team_id", input.TeamID, "scouter_id", user.ID, "error", err)
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

// HandleResubmit updates a rejected submission with new data and resets it to pending
func (h *Handler) HandleResubmit(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}

	if !h.hasDB() {
		http.Error(c.Writer, "Database not connected", http.StatusServiceUnavailable)
		return
	}

	submissionID := c.Param("id")
	if submissionID == "" {
		http.Error(c.Writer, "Submission ID is required", http.StatusBadRequest)
		return
	}

	ctx := c.Request.Context()

	// Verify the submission belongs to this user and is rejected
	var existing scoutingSubmission
	if err := h.db.WithContext(ctx).
		Where("id = ? AND scouter_id = ? AND status = ?", submissionID, user.ID, "rejected").
		First(&existing).Error; err != nil {
		http.Error(c.Writer, "Submission not found or not editable", http.StatusNotFound)
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

	// Update the rejected submission and reset to pending
	updates := map[string]any{
		"event_id":          input.EventID,
		"team_id":           input.TeamID,
		"alliance_color":    input.AllianceColor,
		"notes":             input.Notes,
		"starting_position": input.StartingPosition,
		"defense_rating":    input.DefenseRating,
		"traversal":         input.Traversal,
		"scoring_strategy":  input.TeleopStrategy,
		"shooting_speed":    input.ShootingSpeed,
		"capacity":          input.Capacity,
		"defendability":     input.Defendability,
		"hang_level":        input.HangLevel,
		"auto_hang":         input.AutoHang,
		"hang_position":     input.HangPosition,
		"accuracy_rating":   input.AccuracyRating,
		"scouted_at":        time.Now().UTC(),
		"status":            "pending",
		"rejection_reason":  "",
	}

	if err := h.db.WithContext(ctx).
		Table("scouting_submissions").
		Where("id = ?", submissionID).
		Updates(updates).Error; err != nil {
		if c.GetHeader("HX-Request") == "true" {
			data := h.buildSubmissionPageData(c, user)
			data["SubmissionError"] = "Failed to resubmit"
			h.renderPartial(c, "submission_panel", data)
			return
		}
		http.Error(c.Writer, "Failed to resubmit", http.StatusInternalServerError)
		return
	}

	if c.GetHeader("HX-Request") == "true" {
		data := h.buildSubmissionPageData(c, user)
		data["SubmissionSuccess"] = "Submission updated and requeued for review!"
		h.renderPartial(c, "submission_panel", data)
		return
	}

	http.Redirect(c.Writer, c.Request, "/submission", http.StatusSeeOther)
}

// HandleEditRejectedSubmission loads a rejected submission into the form for editing
func (h *Handler) HandleEditRejectedSubmission(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}

	if !h.hasDB() {
		http.Error(c.Writer, "Database not connected", http.StatusServiceUnavailable)
		return
	}

	submissionID := c.Param("id")
	if submissionID == "" {
		http.Error(c.Writer, "Submission ID is required", http.StatusBadRequest)
		return
	}

	ctx := c.Request.Context()

	var sub struct {
		ID               int
		EventID          int
		TeamID           int
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
		RejectionReason  string
		EventName        string
		TeamNumber       int
		TeamName         string
	}

	if err := h.db.WithContext(ctx).
		Table("scouting_submissions").
		Select("scouting_submissions.*, events.name as event_name, teams.team_number, teams.name as team_name").
		Joins("JOIN events ON events.id = scouting_submissions.event_id").
		Joins("JOIN teams ON teams.id = scouting_submissions.team_id").
		Where("scouting_submissions.id = ? AND scouting_submissions.scouter_id = ? AND scouting_submissions.status = ?", submissionID, user.ID, "rejected").
		First(&sub).Error; err != nil {
		c.String(http.StatusNotFound, "<div class=\"text-red-400 text-sm\">Submission not found or not editable.</div>")
		return
	}

	data := h.buildSubmissionPageData(c, user)
	data["EditSubmission"] = sub
	data["EditSubmissionID"] = sub.ID
	data["RejectionNotice"] = sub.RejectionReason

	h.render(c, "submission", data)
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
	// Traversal comes as multiple checkbox values - join them with comma
	traversalValues := c.PostFormArray("traversal")
	for i, v := range traversalValues {
		traversalValues[i] = strings.ToLower(strings.TrimSpace(v))
	}
	input.Traversal = strings.Join(traversalValues, ",")
	input.ShootingSpeed = strings.ToLower(strings.TrimSpace(c.PostForm("shooting_speed")))
	input.Capacity = strings.ToLower(strings.TrimSpace(c.PostForm("capacity")))
	input.Defendability = strings.TrimSpace(c.PostForm("defendability"))
	input.TeleopStrategy = strings.ToLower(strings.TrimSpace(c.PostForm("teleop_strategy")))
	input.HangLevel = strings.ToLower(strings.TrimSpace(c.PostForm("hang_level")))
	input.AutoHang = strings.ToLower(strings.TrimSpace(c.PostForm("auto_hang")))
	input.HangPosition = strings.ToLower(strings.TrimSpace(c.PostForm("hang_position")))
	input.AccuracyRating = strings.ToLower(strings.TrimSpace(c.PostForm("accuracy_rating")))

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
		c.String(http.StatusServiceUnavailable, `<option value="" disabled selected>Database not connected</option>`)
		return
	}

	eventIDStr := c.Query("event_id")
	if eventIDStr == "" {
		c.String(http.StatusBadRequest, `<option value="" disabled selected>Select event first</option>`)
		return
	}

	eventID, err := strconv.Atoi(eventIDStr)
	if err != nil {
		c.String(http.StatusBadRequest, `<option value="" disabled selected>Invalid event selection</option>`)
		return
	}

	forceRefresh := strings.EqualFold(strings.TrimSpace(c.Query("force_refresh")), "true") || strings.TrimSpace(c.Query("force_refresh")) == "1"

	ctx := c.Request.Context()

	// Get event code from database
	var event struct {
		TBAKey *string
	}
	if err := h.db.WithContext(ctx).Table("events").Select("tba_key").Where("id = ?", eventID).Scan(&event).Error; err != nil {
		c.String(http.StatusInternalServerError, `<option value="" disabled selected>Failed to load event</option>`)
		return
	}

	eventCode := ""
	if event.TBAKey != nil {
		eventCode = extractEventCode(strings.TrimSpace(*event.TBAKey))
	}
	if eventCode == "" {
		c.String(http.StatusBadRequest, `<option value="" disabled selected>Selected event is missing API code</option>`)
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

	_ = query.Scan(&teams).Error

	renderOptions := func(prefix string, sourceTeams []struct {
		ID         int
		TeamNumber int
		Name       string
	}) string {
		html := `<option value="" disabled selected>Select team</option>`
		if prefix != "" {
			html += fmt.Sprintf(`<option value="" disabled>%s</option>`, prefix)
		}
		for _, team := range sourceTeams {
			html += fmt.Sprintf(`<option value="%d">%d - %s</option>`, team.ID, team.TeamNumber, team.Name)
		}
		if len(sourceTeams) == 0 {
			html += `<option value="" disabled>No teams available for this event</option>`
		}
		return html
	}

	if !forceRefresh {
		if len(teams) > 0 {
			// We have teams in the database, render them
			c.Header("Content-Type", "text/html; charset=utf-8")
			c.String(http.StatusOK, renderOptions("", teams))
			return
		}
	}

	// No teams in database, try FIRST API
	username := strings.TrimSpace(os.Getenv("FIRST_API_USERNAME"))
	key := strings.TrimSpace(os.Getenv("FIRST_API_KEY"))
	if username == "" || key == "" {
		c.String(http.StatusInternalServerError, `<option value="" disabled selected>FIRST API credentials not configured</option>`)
		return
	}

	season := 2026
	client := frc.NewClient(username, key)
	firstTeams, err := client.GetEventTeams(ctx, season, eventCode)
	if err != nil {
		if len(teams) > 0 {
			c.Header("Content-Type", "text/html; charset=utf-8")
			c.String(http.StatusOK, renderOptions("Using cached teams (stale)", teams))
			return
		}
		if frc.IsInternetUnavailable(err) {
			c.String(http.StatusServiceUnavailable, `<option value="" disabled selected>No internet connection. Connect LAN uplink and retry sync.</option>`)
			return
		}
		c.String(http.StatusInternalServerError, `<option value="" disabled selected>Failed to fetch teams from FIRST API</option>`)
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
