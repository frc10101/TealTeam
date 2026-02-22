package handlers

import (
	"net/http"

	"github.com/gin-gonic/gin"
)

// HandleCoachViewer renders the drive coach panel
func (h *Handler) HandleCoachViewer(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}
	if !user.IsAdmin && !user.IsCoach {
		http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
		return
	}

	data := map[string]any{
		"Title":       "Drive Coach Panel",
		"Description": "Analyze match data, plan strategies, and review team performance.",
		"User":        user,
	}

	if h.hasDB() {
		// TODO: Load coach-specific data (match insights, team stats, strategy notes, etc.)
		// pending, err := h.loadPendingSubmissions(c)
		// if err == nil {
		// 	data["PendingSubmissions"] = pending
		// }
	}

	h.render(c, "coach_viewer", data)
}
