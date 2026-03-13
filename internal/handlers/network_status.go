package handlers

import (
	"context"
	"net/http"
	"time"

	"github.com/frc10101/TealTeam/internal/frc"
	"github.com/gin-gonic/gin"
)

func classifyNetworkState(s frc.NetworkStatusSnapshot) string {
	if !s.InternetReachable {
		return "offline"
	}
	if !s.LastAPIErrorAt.IsZero() && (s.LastAPISuccessAt.IsZero() || s.LastAPIErrorAt.After(s.LastAPISuccessAt)) {
		return "api-error"
	}
	return "internet-ok"
}

func formatStatusTime(t time.Time) string {
	if t.IsZero() {
		return "Never"
	}
	return t.Local().Format("Jan 2, 3:04:05 PM")
}

func (h *Handler) buildNetworkStatusData(snapshot frc.NetworkStatusSnapshot) map[string]any {
	status := classifyNetworkState(snapshot)
	statusLabel := "Offline"
	statusClass := "bg-red-900/40 text-red-200 border-red-600"

	switch status {
	case "internet-ok":
		statusLabel = "Internet OK"
		statusClass = "bg-teal-900/40 text-teal-200 border-teal-600"
	case "api-error":
		statusLabel = "API Error"
		statusClass = "bg-amber-900/40 text-amber-200 border-amber-600"
	}

	return map[string]any{
		"NetworkStatus":      status,
		"NetworkStatusLabel": statusLabel,
		"NetworkStatusClass": statusClass,
		"LastSyncText":       formatStatusTime(snapshot.LastSuccessfulSync),
		"LastAPISuccessText": formatStatusTime(snapshot.LastAPISuccessAt),
		"LastAPIErrorText":   formatStatusTime(snapshot.LastAPIErrorAt),
		"LastAPIError":       snapshot.LastAPIError,
		"InternetError":      snapshot.InternetError,
	}
}

// HandleNetworkStatus returns JSON network status for diagnostics.
func (h *Handler) HandleNetworkStatus(c *gin.Context) {
	ctx, cancel := context.WithTimeout(c.Request.Context(), 2*time.Second)
	defer cancel()
	_ = frc.RefreshConnectivity(ctx)

	snapshot := frc.GetNetworkStatusSnapshot()
	c.JSON(http.StatusOK, map[string]any{
		"status": classifyNetworkState(snapshot),
		"data":   snapshot,
	})
}

// HandleNetworkStatusBadge returns HTMX-rendered status badge.
func (h *Handler) HandleNetworkStatusBadge(c *gin.Context) {
	ctx, cancel := context.WithTimeout(c.Request.Context(), 2*time.Second)
	defer cancel()
	_ = frc.RefreshConnectivity(ctx)

	snapshot := frc.GetNetworkStatusSnapshot()
	h.renderPartial(c, "network_status_badge", h.buildNetworkStatusData(snapshot))
}
