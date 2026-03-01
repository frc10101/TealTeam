package handlers

import (
	"fmt"
	"net/http"
	"strconv"
	"time"

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

	h.hydrateEventSelectionData(c, user, data)

	return http.StatusOK, ""
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

	data := map[string]any{
		"TeamNumber": team.TeamNumber,
		"TeamName":   team.Name,
	}

	h.hydrateTeamEventData(c, team.ID, eventID, data)

	// Render the partial template
	h.renderPartial(c, "team_data", data)
}

// hydrateTeamEventData adds team performance data for a specific event to the data map
func (h *Handler) hydrateTeamEventData(c *gin.Context, teamID int, eventID int, data map[string]any) {
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
			var totalAuto, totalTeleop, totalEndgame int
			var totalHubAuto, totalHubTeleop, totalHubEndgame int
			var totalPenalties int
			startingPositions := make(map[string]int)
			defenseRatings := make(map[string]int)
			traversals := make(map[string]int)
			scoringStrategies := make(map[string]int)
			hangLevels := make(map[string]int)
			autoHangs := make(map[string]int)
			hangPositions := make(map[string]int)
			allianceColors := make(map[string]int)

			for _, sd := range scoutingData {
				totalAuto += sd.AutoScore
				totalTeleop += sd.TeleopScore
				totalEndgame += sd.EndgameScore
				totalHubAuto += sd.HubAutoCount
				totalHubTeleop += sd.HubTeleopCount
				totalHubEndgame += sd.HubEndgameCount
				totalPenalties += sd.PenaltiesCaused

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
			}

			count := len(scoutingData)
			data["AvgAutoScore"] = float64(totalAuto) / float64(count)
			data["AvgTeleopScore"] = float64(totalTeleop) / float64(count)
			data["AvgEndgameScore"] = float64(totalEndgame) / float64(count)
			data["AvgTotalScore"] = float64(totalAuto+totalTeleop+totalEndgame) / float64(count)
			data["AvgHubAuto"] = float64(totalHubAuto) / float64(count)
			data["AvgHubTeleop"] = float64(totalHubTeleop) / float64(count)
			data["AvgHubEndgame"] = float64(totalHubEndgame) / float64(count)
			data["AvgPenalties"] = float64(totalPenalties) / float64(count)

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

			// Get qualitative data (most recent or mode)
			latestData := scoutingData[0]
			data["LatestNotes"] = latestData.Notes
			data["ShootingSpeed"] = latestData.ShootingSpeed
			data["Capacity"] = latestData.Capacity
			data["Defendability"] = latestData.Defendability
			data["ScoringStrategy"] = latestData.ScoringStrategy
			data["Throughput"] = latestData.Throughput

			// Collect all notes from the competition
			type NoteEntry struct {
				Note        string
				ScouterName string
				ScoutedAt   *time.Time
				MatchIndex  int
			}
			var notes []NoteEntry
			for i, sd := range scoutingData {
				if sd.Notes != nil && *sd.Notes != "" {
					notes = append(notes, NoteEntry{
						Note:        *sd.Notes,
						ScouterName: valueOrNA(sd.ScouterName),
						ScoutedAt:   sd.ScoutedAt,
						MatchIndex:  i + 1,
					})
				}
			}
			data["AllNotes"] = notes
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
