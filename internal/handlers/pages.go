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
		"Message": "Welcome to Your Application",
		"User":    user,
	}

	if user != nil && h.hasDB() {
		var events []struct {
			ID   int
			Name string
		}

		if err := h.db.WithContext(c.Request.Context()).
			Table("events").
			Select("id, name").
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

	h.render(c, "index", data)
}

// HandleSubmissionPage renders the submission page
func (h *Handler) HandleSubmissionPage(c *gin.Context) {
	// Require authentication
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}

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

		var teams []struct {
			ID         int
			TeamNumber int
			Name       string
		}

		if err := h.db.WithContext(c.Request.Context()).
			Table("teams").
			Select("id, team_number, name").
			Order("team_number").
			Scan(&teams).Error; err == nil {
			data["Teams"] = teams
		}

		var events []struct {
			ID   int
			Name string
		}

		if err := h.db.WithContext(c.Request.Context()).
			Table("events").
			Select("id, name").
			Order("start_date").
			Scan(&events).Error; err == nil {
			data["Events"] = events
		}
	}

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
		http.Error(c.Writer, "Database unavailable", http.StatusServiceUnavailable)
		return
	}

	selectedEventID, err := parseRequiredInt(c, "event_id")
	if err != nil {
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
		http.Error(c.Writer, "Failed to save event selection", http.StatusInternalServerError)
		return
	}

	// TODO: If the user's team is in the event, hydrate match schedule and related data.

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
			if topTeams, userTeam, aroundTeams, err := h.loadRankingSnapshot(c, *session.SelectedEventID, user.TeamNumber); err == nil {
				data["TopRankedTeams"] = topTeams
				data["UserRankedTeam"] = userTeam
				data["AroundRankedTeams"] = aroundTeams
			}
		}
		pending, err := h.loadPendingSubmissions(c)
		if err == nil {
			data["PendingSubmissions"] = pending
		}
		teams, err := h.loadPickListTeams(c)
		if err == nil {
			data["PickListTeams"] = teams
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
		"Description": "Sign in to access higher level features.",
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
		"Description": "Create an account to get started.",
	}
	h.render(c, "signup", data)
}

// TODO: Add more page handlers here
// func (h *Handler) HandleYourPage(w http.ResponseWriter, r *http.Request) {
//     data := map[string]any{
//         "Title": "Your Page Title",
//     }
//     h.render(w, "yourpage", data)
// }
