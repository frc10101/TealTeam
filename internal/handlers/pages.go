package handlers

import (
	"net/http"

	"github.com/frc10101/TealTeam/internal/models"
	"github.com/gin-gonic/gin"
)

// HandleIndex renders the home page
func (h *Handler) HandleIndex(c *gin.Context) {
	// Redirect non-root paths to 404
	if c.Request.URL.Path != "/" {
		http.NotFound(c.Writer, c.Request)
		return
	}

	// Get current user if authenticated
	user, _ := h.GetSessionUser(c)

	data := map[string]any{
		"Title":   "Home",
		"Message": "TealTeam Scouting",
		"User":    user,
	}

	h.hydrateEventSelectionData(c, user, data)

	h.render(c, "index", data)
}

func (h *Handler) hydrateEventSelectionData(c *gin.Context, user *models.User, data map[string]any) {
	if user == nil || !h.hasDB() {
		return
	}

	// Get event IDs available to this user (filtered by team registration)
	eventIDs, err := h.GetAvailableEventsForUser(c.Request.Context(), user)
	if err != nil || len(eventIDs) == 0 {
		return
	}

	var events []struct {
		ID   int
		Name string
	}

	if err := h.db.WithContext(c.Request.Context()).
		Table("events").
		Select("id, name").
		Where("id IN ?", eventIDs).
		Order("start_date").
		Scan(&events).Error; err == nil {
		data["Events"] = events
	}

	session, err := h.GetSession(c)
	if err == nil && session.SelectedEventID != nil {
		data["SelectedEventID"] = *session.SelectedEventID

		if user.TeamNumber != nil {
			var teamMatchCount int64
			_ = h.db.WithContext(c.Request.Context()).
				Table("event_teams").
				Joins("JOIN teams ON teams.id = event_teams.team_id").
				Where("event_teams.event_id = ? AND teams.team_number = ?", *session.SelectedEventID, *user.TeamNumber).
				Count(&teamMatchCount).Error
			if teamMatchCount == 0 {
				data["EventWarning"] = "Your team is not listed for this event yet."
			}
		}
	}
}

func (h *Handler) hydrateEventSummaryData(c *gin.Context, eventID int, data map[string]any) {
	if !h.hasDB() {
		return
	}

	// Fetch event name
	var event struct {
		Name string
	}
	if err := h.db.WithContext(c.Request.Context()).
		Table("events").
		Select("name").
		Where("id = ?", eventID).
		Scan(&event).Error; err == nil {
		data["SelectedEventName"] = event.Name
		data["SelectedEventID"] = eventID
	}

	// Count teams in event
	var teamCount int64
	if err := h.db.WithContext(c.Request.Context()).
		Table("event_teams").
		Where("event_id = ?", eventID).
		Count(&teamCount).Error; err == nil {
		data["EventTeamsCount"] = teamCount
	}

	// Count matches in event
	var matchCount int64
	if err := h.db.WithContext(c.Request.Context()).
		Table("matches").
		Where("event_id = ?", eventID).
		Count(&matchCount).Error; err == nil {
		data["EventMatchesCount"] = matchCount
	}

	// Fetch teams for the event
	var teams []struct {
		TeamNumber int
		Name       string
	}
	if err := h.db.WithContext(c.Request.Context()).
		Table("event_teams").
		Joins("JOIN teams ON teams.id = event_teams.team_id").
		Select("teams.team_number, teams.name").
		Where("event_teams.event_id = ?", eventID).
		Order("teams.team_number").
		Scan(&teams).Error; err == nil {
		data["EventTeams"] = teams
	}

	// Check if user's team is in the event
	user, _ := h.GetSessionUser(c)
	if user != nil && user.TeamNumber != nil {
		var userTeamCount int64
		_ = h.db.WithContext(c.Request.Context()).
			Table("event_teams").
			Joins("JOIN teams ON teams.id = event_teams.team_id").
			Where("event_teams.event_id = ? AND teams.team_number = ?", eventID, *user.TeamNumber).
			Count(&userTeamCount).Error
		if userTeamCount == 0 {
			data["EventWarning"] = "Your team is not listed for this event yet."
		}
	}
}

// HandleSubmissionPage renders the submission page
func (h *Handler) HandleSubmissionPage(c *gin.Context) {
	// Require authentication
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}

	data := h.buildSubmissionPageData(c, user)
	h.render(c, "submission", data)
}

// HandleSelectEvent updates the selected event for the current session
func (h *Handler) HandleSelectEvent(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}
	if !h.hasDB() {
		if c.GetHeader("HX-Request") == "true" {
			data := map[string]any{
				"User":       user,
				"EventError": "Database unavailable",
			}
			h.renderPartial(c, "event_selection", data)
			return
		}
		http.Error(c.Writer, "Database unavailable", http.StatusServiceUnavailable)
		return
	}

	selectedEventID, err := parseRequiredInt(c, "event_id")
	if err != nil {
		if c.GetHeader("HX-Request") == "true" {
			data := map[string]any{
				"User":       user,
				"EventError": err.Error(),
			}
			h.hydrateEventSelectionData(c, user, data)
			h.renderPartial(c, "event_selection", data)
			return
		}
		http.Error(c.Writer, err.Error(), http.StatusBadRequest)
		return
	}

	session, err := h.GetSession(c)
	if err != nil {
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}

	if err := h.db.WithContext(c.Request.Context()).
		Model(&models.Session{}).
		Where("session_id = ?", session.SessionID).
		Update("selected_event_id", selectedEventID).Error; err != nil {
		if c.GetHeader("HX-Request") == "true" {
			data := map[string]any{
				"User":       user,
				"EventError": "Failed to save event selection",
			}
			h.hydrateEventSelectionData(c, user, data)
			h.renderPartial(c, "event_selection", data)
			return
		}
		http.Error(c.Writer, "Failed to save event selection", http.StatusInternalServerError)
		return
	}

	// TODO: If the user's team is in the event, hydrate match schedule and related data.

	if c.GetHeader("HX-Request") == "true" {
		data := map[string]any{
			"User":         user,
			"EventUpdated": true,
		}
		h.hydrateEventSelectionData(c, user, data)
		// Load event summary data
		h.hydrateEventSummaryData(c, selectedEventID, data)
		// Trigger match schedule reload via HTMX event
		c.Header("HX-Trigger", "eventSelected")
		h.renderPartial(c, "event_selection", data)
		return
	}

	http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
}

// HandleAdminViewer renders the lead scout panel
func (h *Handler) HandleAdminViewer(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}
	if !user.IsAdmin && !user.IsLeadScout {
		http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
		return
	}

	data := map[string]any{
		"Title":       "Lead Scout Panel",
		"Description": "Approve submissions, review rankings, and coordinate match strategy.",
		"User":        user,
	}

	if h.hasDB() {
		session, err := h.GetSession(c)
		if err == nil && session.SelectedEventID != nil {
			data["SelectedEventID"] = *session.SelectedEventID
			sortKey := c.Query("team_sort")
			data["TeamRankingSort"] = sortKey
			// Load full team point rankings
			if teamRankings, err := h.loadTeamPointRankings(c, *session.SelectedEventID, sortKey); err == nil {
				data["TeamRankings"] = teamRankings
			}
			// Load pick list teams for the selected event
			if teams, err := h.loadPickListTeams(c, *session.SelectedEventID); err == nil {
				data["PickListTeams"] = teams
			}
		}
		pending, err := h.loadPendingSubmissions(c, user)
		if err == nil {
			data["PendingSubmissions"] = pending
		}
	}

	h.render(c, "admin_viewer", data)
}

func (h *Handler) HandleSignIn(c *gin.Context) {
	// Redirect if already logged in
	user, _ := h.GetSessionUser(c)
	if user != nil {
		http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
		return
	}

	data := map[string]any{
		"Title":       "Sign In",
		"Description": "Access scouting data, analytics, and team management features.",
	}
	h.render(c, "signin", data)
}

func (h *Handler) HandleSignUp(c *gin.Context) {
	// Redirect if already logged in
	user, _ := h.GetSessionUser(c)
	if user != nil {
		http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
		return
	}

	data := map[string]any{
		"Title":       "Sign Up",
		"Description": "Join your team's scouting network and start collecting match data.",
	}
	h.render(c, "signup", data)
}
