package handlers

import (
	"fmt"
	"net/http"
	"strconv"
	"time"

	"github.com/frc10101/TealTeam/internal/frc"
	"github.com/frc10101/TealTeam/internal/models"
	"github.com/gin-gonic/gin"
)

// HandleTeamPage renders the team detail page
func (h *Handler) HandleTeamPage(c *gin.Context) {
	if !h.hasDB() {
		c.JSON(http.StatusServiceUnavailable, gin.H{"error": "Database not available"})
		return
	}

	// Get current user
	user, _ := h.GetSessionUser(c)

	data := map[string]any{
		"Title": "Teams",
		"User":  user,
	}

	teamNumberStr := c.Query("team")
	if teamNumberStr != "" {
		data["TeamSearchValue"] = teamNumberStr

		if _, errMsg := h.hydrateTeamLookupData(c, user, teamNumberStr, data); errMsg != "" {
			data["TeamError"] = errMsg
		}
	}

	h.hydrateEventSelectionData(c, user, data)
	h.render(c, "team", data)
}

// HandleTeamSearch returns HTMX fragment with team information
func (h *Handler) HandleTeamSearch(c *gin.Context) {
	if !h.hasDB() {
		c.JSON(http.StatusServiceUnavailable, gin.H{"error": "Database not available"})
		return
	}

	// Get current user
	user, _ := h.GetSessionUser(c)

	data := map[string]any{
		"User": user,
	}

	statusCode, errMsg := h.hydrateTeamLookupData(c, user, c.Query("team"), data)
	if errMsg != "" {
		c.String(statusCode, "<div class=\"card\"><div class=\"card-body text-center text-red-400 py-8\">%s</div></div>", errMsg)
		return
	}

	// Render the partial template
	h.renderPartial(c, "team_info", data)
}

func (h *Handler) hydrateTeamLookupData(c *gin.Context, user *models.User, teamNumberStr string, data map[string]any) (int, string) {
	if teamNumberStr == "" {
		return http.StatusBadRequest, "Team number is required"
	}

	teamNumber, err := strconv.Atoi(teamNumberStr)
	if err != nil {
		return http.StatusBadRequest, "Invalid team number"
	}

	var team models.Team
	if err := h.db.WithContext(c.Request.Context()).
		Table("teams").
		Where("team_number = ?", teamNumber).
		First(&team).Error; err != nil {
		return http.StatusNotFound, fmt.Sprintf("Team %d not found", teamNumber)
	}

	data["User"] = user
	data["Team"] = team
	data["TeamNumber"] = team.TeamNumber
	data["TeamName"] = team.Name

	// Populate events that this specific team is participating in
	h.hydrateTeamEventsData(c, teamNumber, data)

	return http.StatusOK, ""
}

// hydrateTeamEventsData fetches events for a specific team
// If events are not found locally, it calls the FIRST API to fetch them and sync to database
func (h *Handler) hydrateTeamEventsData(c *gin.Context, teamNumber int, data map[string]any) {
	if !h.hasDB() {
		return
	}

	// Try to get events from database for this team
	eventIDs, err := h.GetEventsForTeam(c.Request.Context(), teamNumber)
	if err != nil || len(eventIDs) == 0 {
		// No events locally, try to sync from FIRST API
		syncErr := h.syncAndLoadTeamEvents(c, teamNumber)
		if syncErr != nil {
			// Sync failed, just continue without events
			data["Events"] = []struct {
				ID   int
				Name string
			}{}
			return
		}
		// After sync, try to get events again
		eventIDs, _ = h.GetEventsForTeam(c.Request.Context(), teamNumber)
	}

	// Fetch event details
	if len(eventIDs) > 0 {
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
	}
}

// syncAndLoadTeamEvents syncs events for a specific team from FIRST API
func (h *Handler) syncAndLoadTeamEvents(c *gin.Context, teamNumber int) error {
	ctx := c.Request.Context()
	_, err := frc.SyncTeamForUser(ctx, h.db, teamNumber)
	return err
}

// HandleTeamEventData returns HTMX fragment with team data for a specific event
func (h *Handler) HandleTeamEventData(c *gin.Context) {
	if !h.hasDB() {
		c.JSON(http.StatusServiceUnavailable, gin.H{"error": "Database not available"})
		return
	}

	// Get team number from query parameter
	teamNumberStr := c.Query("team")
	if teamNumberStr == "" {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Team number is required"})
		return
	}

	teamNumber, err := strconv.Atoi(teamNumberStr)
	if err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid team number"})
		return
	}

	// Get event ID from query parameter
	eventIDStr := c.Query("event_id")
	if eventIDStr == "" {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Event ID is required"})
		return
	}

	eventID, err := strconv.Atoi(eventIDStr)
	if err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid event ID"})
		return
	}

	// Find the team
	var team models.Team
	if err := h.db.WithContext(c.Request.Context()).
		Table("teams").
		Where("team_number = ?", teamNumber).
		First(&team).Error; err != nil {
		c.JSON(http.StatusNotFound, gin.H{"error": "Team not found"})
		return
	}

	// Resolve the viewer's team ID for notes filtering
	var viewerTeamID int
	if user, _ := h.GetSessionUser(c); user != nil && user.TeamNumber != nil {
		var viewerTeam models.Team
		if err := h.db.WithContext(c.Request.Context()).
			Table("teams").
			Where("team_number = ?", *user.TeamNumber).
			First(&viewerTeam).Error; err == nil {
			viewerTeamID = viewerTeam.ID
		}
	}

	data := map[string]any{
		"TeamNumber": team.TeamNumber,
		"TeamName":   team.Name,
	}

	h.hydrateTeamEventData(c, team.ID, eventID, viewerTeamID, data)

	// Render the partial template
	h.renderPartial(c, "team_data", data)
}

// hydrateTeamEventData adds team performance data for a specific event to the data map.
// viewerTeamID is the team ID of the logged-in user; notes are filtered to only show
// entries submitted by that team. Pass 0 to hide all notes.
func (h *Handler) hydrateTeamEventData(c *gin.Context, teamID int, eventID int, viewerTeamID int, data map[string]any) {
	// Get event name
	var event models.Event
	if err := h.db.WithContext(c.Request.Context()).
		Table("events").
		Where("id = ?", eventID).
		First(&event).Error; err == nil {
		data["EventName"] = event.Name
	}

	// Get team event stats (OPR, DPR, CCWM, rank, etc.)
	var stats models.TeamEventStats
	if err := h.db.WithContext(c.Request.Context()).
		Table("team_event_stats").
		Where("team_id = ? AND event_id = ?", teamID, eventID).
		First(&stats).Error; err == nil {
		data["TeamStats"] = stats
	}

	// Get all scouting data for this team at this event
	var scoutingData []models.ScoutingData
	if err := h.db.WithContext(c.Request.Context()).
		Table("scouting_data").
		Where("team_id = ? AND event_id = ?", teamID, eventID).
		Order("scouted_at DESC").
		Find(&scoutingData).Error; err == nil {
		data["ScoutingData"] = scoutingData
		data["ScoutingCount"] = len(scoutingData)

		// Calculate averages
		if len(scoutingData) > 0 {
			startingPositions := make(map[string]int)
			defenseRatings := make(map[string]int)
			traversals := make(map[string]int)
			scoringStrategies := make(map[string]int)
			hangLevels := make(map[string]int)
			autoHangs := make(map[string]int)
			hangPositions := make(map[string]int)
			allianceColors := make(map[string]int)
			accuracyRatings := make(map[string]int)

			for _, sd := range scoutingData {
				if sd.StartingPosition != nil && *sd.StartingPosition != "" {
					startingPositions[*sd.StartingPosition]++
				}
				if sd.DefenseRating != nil && *sd.DefenseRating != "" {
					defenseRatings[*sd.DefenseRating]++
				}
				if sd.Traversal != nil && *sd.Traversal != "" {
					traversals[*sd.Traversal]++
				}
				if sd.ScoringStrategy != nil && *sd.ScoringStrategy != "" {
					scoringStrategies[*sd.ScoringStrategy]++
				}
				if sd.HangLevel != nil && *sd.HangLevel != "" {
					hangLevels[*sd.HangLevel]++
				}
				if sd.AutoHang != nil && *sd.AutoHang != "" {
					autoHangs[*sd.AutoHang]++
				}
				if sd.HangPosition != nil && *sd.HangPosition != "" {
					hangPositions[*sd.HangPosition]++
				}
				// Alliance color distribution
				allianceColors[sd.AllianceColor]++
				if sd.AccuracyRating != nil && *sd.AccuracyRating != "" {
					accuracyRatings[*sd.AccuracyRating]++
				}
			}

			// Alliance color stats
			data["AllianceColorStats"] = allianceColors

			// Most common starting position
			var mostCommonPos string
			var maxCount int
			for pos, cnt := range startingPositions {
				if cnt > maxCount {
					maxCount = cnt
					mostCommonPos = pos
				}
			}
			data["MostCommonStartPos"] = mostCommonPos

			// Most common defense rating
			var mostCommonDefense string
			maxCount = 0
			for rating, cnt := range defenseRatings {
				if cnt > maxCount {
					maxCount = cnt
					mostCommonDefense = rating
				}
			}
			data["MostCommonDefense"] = mostCommonDefense

			// Most common traversal
			var mostCommonTraversal string
			maxCount = 0
			for trav, cnt := range traversals {
				if cnt > maxCount {
					maxCount = cnt
					mostCommonTraversal = trav
				}
			}
			data["MostCommonTraversal"] = mostCommonTraversal

			// Most common scoring strategy
			var mostCommonScoringStrat string
			maxCount = 0
			for strat, cnt := range scoringStrategies {
				if cnt > maxCount {
					maxCount = cnt
					mostCommonScoringStrat = strat
				}
			}
			data["MostCommonScoringStrategy"] = mostCommonScoringStrat

			// Most common hang level
			var mostCommonHangLevel string
			maxCount = 0
			for level, cnt := range hangLevels {
				if cnt > maxCount {
					maxCount = cnt
					mostCommonHangLevel = level
				}
			}
			data["MostCommonHangLevel"] = mostCommonHangLevel

			// Auto hang stats
			data["AutoHangStats"] = autoHangs

			// Most common hang position
			var mostCommonHangPos string
			maxCount = 0
			for pos, cnt := range hangPositions {
				if cnt > maxCount {
					maxCount = cnt
					mostCommonHangPos = pos
				}
			}
			data["MostCommonHangPosition"] = mostCommonHangPos

			// Most common accuracy rating
			var mostCommonAccuracy string
			maxCount = 0
			for rating, cnt := range accuracyRatings {
				if cnt > maxCount {
					maxCount = cnt
					mostCommonAccuracy = rating
				}
			}
			data["MostCommonAccuracy"] = mostCommonAccuracy

			// Get qualitative data (most recent or mode)
			latestData := scoutingData[0]
			data["ShootingSpeed"] = derefStr(latestData.ShootingSpeed)
			data["Capacity"] = derefStr(latestData.Capacity)
			data["Defendability"] = derefStr(latestData.Defendability)
			data["ScoringStrategy"] = derefStr(latestData.ScoringStrategy)

			// Collect notes only from the viewer's own team
			type NoteEntry struct {
				Note       string
				ScoutedAt  *time.Time
				MatchIndex int
			}
			var notes []NoteEntry
			for i, sd := range scoutingData {
				if sd.Notes != nil && *sd.Notes != "" && viewerTeamID > 0 && sd.SubmittingTeamID != nil && *sd.SubmittingTeamID == viewerTeamID {
					notes = append(notes, NoteEntry{
						Note:       *sd.Notes,
						ScoutedAt:  sd.ScoutedAt,
						MatchIndex: i + 1,
					})
				}
			}
			data["AllNotes"] = notes

			// LatestNotes: first note belonging to the viewer's team
			for _, sd := range scoutingData {
				if sd.Notes != nil && *sd.Notes != "" && viewerTeamID > 0 && sd.SubmittingTeamID != nil && *sd.SubmittingTeamID == viewerTeamID {
					data["LatestNotes"] = *sd.Notes
					break
				}
			}
		}
	}
}

// Helper function to get value or "Unknown"
func valueOrNA(s *string) string {
	if s != nil && *s != "" {
		return *s
	}
	return "Unknown"
}

// derefStr safely dereferences a *string, returning "" if nil or empty.
func derefStr(s *string) string {
	if s != nil {
		return *s
	}
	return ""
}

// HandleFetchPastEvents triggers a sync for a team's past events from FIRST API
func (h *Handler) HandleFetchPastEvents(c *gin.Context) {
	if !h.hasDB() {
		c.String(http.StatusServiceUnavailable, "Database not available")
		return
	}

	user, _ := h.GetSessionUser(c)

	teamNumberStr := c.PostForm("team")
	if teamNumberStr == "" {
		c.String(http.StatusBadRequest, "Team number is required")
		return
	}

	teamNumber, err := strconv.Atoi(teamNumberStr)
	if err != nil {
		c.String(http.StatusBadRequest, "Invalid team number")
		return
	}

	// Sync team events from FIRST API
	syncErr := h.syncAndLoadTeamEvents(c, teamNumber)
	if syncErr != nil {
		h.log.Warn("failed to fetch past events", "team", teamNumber, "error", syncErr)
	}

	// Re-render the team info partial with updated data
	data := map[string]any{
		"User": user,
	}

	if _, errMsg := h.hydrateTeamLookupData(c, user, teamNumberStr, data); errMsg != "" {
		c.String(http.StatusInternalServerError, "<div class=\"card\"><div class=\"card-body text-center text-red-400 py-8\">%s</div></div>", errMsg)
		return
	}

	h.renderPartial(c, "team_info", data)
}
