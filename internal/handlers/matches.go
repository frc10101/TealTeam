package handlers

import (
	"context"
	"os"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/frc10101/TealTeam/internal/frc"
	"github.com/gin-gonic/gin"
)

// MatchDisplay represents match data prepared for display
type MatchDisplay struct {
	Description  string
	MatchNumber  int
	StartTime    time.Time
	TimeDisplay  string
	TimeStatus   string // "past", "current", "upcoming"
	RedTeams     []int
	BlueTeams    []int
	MinutesUntil int
}

// HandleMatchSchedule returns current matches in the 30-minute window
func (h *Handler) HandleMatchSchedule(c *gin.Context) {
	if !h.hasDB() {
		h.renderPartial(c, "match_schedule", map[string]any{
			"ScheduleMessage": "Database unavailable.",
		})
		return
	}

	user, _ := h.GetSessionUser(c)
	if user == nil {
		h.renderPartial(c, "match_schedule", map[string]any{
			"ScheduleMessage": "Sign in to view match schedule.",
		})
		return
	}

	session, err := h.GetSession(c)
	if err != nil || session.SelectedEventID == nil {
		h.renderPartial(c, "match_schedule", map[string]any{
			"ScheduleMessage": "Select an event to view live matches.",
		})
		return
	}

	eventID := *session.SelectedEventID

	// Get event details from database
	var event struct {
		ID        int
		TBAKey    *string
		Name      string
		StartDate *time.Time
	}

	if err := h.db.WithContext(c.Request.Context()).
		Table("events").
		Select("id, tba_key, name, start_date").
		Where("id = ?", eventID).
		Scan(&event).Error; err != nil {
		h.log.Error("failed to fetch event", "error", err)
		h.renderPartial(c, "match_schedule", map[string]any{
			"ScheduleMessage": "Unable to load selected event.",
		})
		return
	}

	if event.TBAKey == nil || *event.TBAKey == "" {
		h.renderPartial(c, "match_schedule", map[string]any{
			"ScheduleMessage": "Selected event is missing schedule data.",
		})
		return
	}

	// Extract event code from TBA key (format: {year}{event_code})
	eventCode := extractEventCode(*event.TBAKey)
	if eventCode == "" {
		h.log.Error("failed to extract event code from TBA key", "tba_key", *event.TBAKey)
		h.renderPartial(c, "match_schedule", map[string]any{
			"ScheduleMessage": "Unable to determine event code for schedule lookup.",
		})
		return
	}

	// Get current season
	season := 2026
	if seasonEnv := strings.TrimSpace(os.Getenv("FIRST_SEASON")); seasonEnv != "" {
		if parsed, err := strconv.Atoi(seasonEnv); err == nil {
			season = parsed
		}
	}

	// Fetch match schedule from FRC API
	username := strings.TrimSpace(os.Getenv("FIRST_API_USERNAME"))
	key := strings.TrimSpace(os.Getenv("FIRST_API_KEY"))
	if username == "" || key == "" {
		h.renderPartial(c, "match_schedule", map[string]any{
			"ScheduleMessage": "FIRST API credentials are not configured.",
		})
		return
	}

	client := frc.NewClient(username, key)
	ctx, cancel := context.WithTimeout(c.Request.Context(), 15*time.Second)
	defer cancel()

	// Fetch qualification matches
	matches, err := client.GetMatchSchedule(ctx, season, eventCode, nil)
	if err != nil {
		h.log.Error("failed to fetch match schedule", "error", err, "event_code", eventCode)
		h.renderPartial(c, "match_schedule", map[string]any{
			"ScheduleMessage": "Could not fetch match schedule from FIRST API.",
		})
		return
	}

	// Filter matches within 15-minute window (past and future)
	now := time.Now()
	windowStart := now.Add(-15 * time.Minute)
	windowEnd := now.Add(15 * time.Minute)

	var displayMatches []MatchDisplay
	for _, match := range matches {
		startTime, err := time.Parse(time.RFC3339, match.StartTime)
		if err != nil {
			h.log.Warn("failed to parse match start time", "match", match.MatchNumber, "time", match.StartTime)
			continue
		}

		// Filter by time window
		if startTime.Before(windowStart) || startTime.After(windowEnd) {
			continue
		}

		// Separate teams by alliance
		var redTeams, blueTeams []int
		for _, team := range match.Teams {
			if strings.HasPrefix(team.Station, "Red") {
				redTeams = append(redTeams, team.TeamNumber)
			} else if strings.HasPrefix(team.Station, "Blue") {
				blueTeams = append(blueTeams, team.TeamNumber)
			}
		}

		// Determine time status
		timeStatus := "upcoming"
		minutesUntil := int(startTime.Sub(now).Minutes())
		if startTime.Before(now) {
			timeStatus = "past"
		} else if minutesUntil <= 5 {
			timeStatus = "current"
		}

		displayMatches = append(displayMatches, MatchDisplay{
			Description:  match.Description,
			MatchNumber:  match.MatchNumber,
			StartTime:    startTime,
			TimeDisplay:  startTime.Format("3:04 PM"),
			TimeStatus:   timeStatus,
			RedTeams:     redTeams,
			BlueTeams:    blueTeams,
			MinutesUntil: minutesUntil,
		})
	}

	data := map[string]any{
		"Matches":   displayMatches,
		"EventName": event.Name,
	}

	h.renderPartial(c, "match_schedule", data)
}

// extractEventCode extracts the event code from a TBA key
// TBA key format: {year}{event_code} (e.g., "2026mndu" -> "mndu")
func extractEventCode(tbaKey string) string {
	// Remove year prefix (first 4 digits)
	re := regexp.MustCompile(`^\d{4}(.+)$`)
	matches := re.FindStringSubmatch(tbaKey)
	if len(matches) > 1 {
		return strings.ToUpper(matches[1])
	}
	return ""
}
