package handlers

import (
	"context"
	"errors"
	"net/http"
	"time"

	"github.com/frc10101/TealTeam/internal/frc"
	"github.com/gin-gonic/gin"
)

// HandleFRCSync triggers a FIRST Events API sync on demand.
func (h *Handler) HandleFRCSync(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil || (!user.IsAdmin && !user.IsLeadScout) {
		http.Error(c.Writer, "Unauthorized", http.StatusUnauthorized)
		return
	}
	if !h.hasDB() {
		http.Error(c.Writer, "Database unavailable", http.StatusServiceUnavailable)
		return
	}

	ctx, cancel := context.WithTimeout(c.Request.Context(), 90*time.Second)
	defer cancel()

	result, err := frc.SyncNow(ctx, h.db)
	if err != nil {
		if errors.Is(err, frc.ErrSyncSkipped) {
			http.Error(c.Writer, "FIRST API credentials missing", http.StatusBadRequest)
			return
		}
		h.log.Error("FRC sync failed", "error", err)
		http.Error(c.Writer, err.Error(), http.StatusInternalServerError)
		return
	}

	h.log.Info("FRC sync completed", "events", result.Events, "teams", result.Teams, "event_teams", result.EventTeams)
	c.JSON(http.StatusOK, result)
}
