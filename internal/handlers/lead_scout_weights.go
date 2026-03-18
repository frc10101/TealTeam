package handlers

import (
	"net/http"

	"github.com/gin-gonic/gin"
)

func (h *Handler) HandleLeadScoutWeightsPage(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}
	if !user.IsAdmin && !user.IsLeadScout {
		http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
		return
	}

	sections := h.loadScoutingPointSections(c)
	sortSectionsByLabel(sections)

	data := map[string]any{
		"Title":       "Point Weight Settings",
		"Description": "Adjust category point values used for team ranking scores.",
		"User":        user,
		"Sections":    sections,
		"Updated":     c.Query("updated") == "1",
		"Error":       c.Query("error"),
	}

	h.render(c, "lead_scout_weights", data)
}

func (h *Handler) HandleLeadScoutWeightsUpdate(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}
	if !user.IsAdmin && !user.IsLeadScout {
		http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
		return
	}

	baseSections := h.loadScoutingPointSections(c)
	parsedSections, err := parsePointSectionsFromForm(c, baseSections)
	if err != nil {
		c.Redirect(http.StatusSeeOther, "/lead-scout/weights?error=Invalid+weight+value")
		return
	}

	if err := h.persistScoutingPointSections(c, parsedSections); err != nil {
		h.log.Error("failed to persist scouting point weights", "error", err)
		c.Redirect(http.StatusSeeOther, "/lead-scout/weights?error=Failed+to+save+weights")
		return
	}

	// Rankings are computed using current weights during page load, so a redirect is enough to refresh.
	c.Redirect(http.StatusSeeOther, "/lead-scout?team_sort=points")
}
