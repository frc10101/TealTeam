package handlers

import (
	"net/http"
	"strconv"

	"github.com/gin-gonic/gin"
)

// HandleEventSummary returns event summary data as an HTML fragment
func (h *Handler) HandleEventSummary(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}

	if !h.hasDB() {
		data := map[string]any{
			"EventSummaryError": "Database unavailable",
		}
		h.renderPartial(c, "event_summary", data)
		return
	}

	eventIDStr := c.Query("event_id")
	if eventIDStr == "" {
		// Try to get event_id from session
		session, err := h.GetSession(c)
		if err == nil && session.SelectedEventID != nil {
			eventIDStr = strconv.Itoa(*session.SelectedEventID)
		}
	}

	if eventIDStr == "" {
		data := map[string]any{
			"User": user,
		}
		h.renderPartial(c, "event_summary", data)
		return
	}

	eventID, err := strconv.Atoi(eventIDStr)
	if err != nil {
		data := map[string]any{
			"EventSummaryError": "Invalid event ID",
		}
		h.renderPartial(c, "event_summary", data)
		return
	}

	data := map[string]any{
		"User": user,
	}

	h.hydrateEventSummaryData(c, eventID, data)
	h.renderPartial(c, "event_summary", data)
}
