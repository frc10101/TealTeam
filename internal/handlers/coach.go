package handlers

import (
	"context"
	"log"
	"net/http"
	"net/url"
	"os"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/frc10101/TealTeam/internal/frc"

	"github.com/gin-gonic/gin"
)

type driveCoachTeam struct {
	TeamNumber int
	OPR        *float64
	DPR        *float64
}

type driveCoachMatch struct {
	Description    string
	MatchNumber    int
	MatchType      string
	StartTime      *time.Time
	TimeDisplay    string
	Status         string
	StatusLabel    string
	StatusClass    string
	MinutesFromNow int
	RedTeams       []driveCoachTeam
	BlueTeams      []driveCoachTeam
	OurAlliance    string
	OurPartners    []int
}

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
		"Description": "Quick match schedule and alliance partners.",
		"User":        user,
	}

	if h.hasDB() {
		h.hydrateEventSelectionData(c, user, data)

		session, err := h.GetSession(c)
		if err == nil && session.SelectedEventID != nil {
			selectedEventID := *session.SelectedEventID
			data["SelectedEventID"] = selectedEventID
			h.hydrateEventSummaryData(c, selectedEventID, data)

			matches, summary, loadErr := h.loadDriveCoachMatches(c.Request.Context(), selectedEventID, user.TeamNumber)
			if loadErr != nil {
				data["DriveCoachError"] = loadErr.Error()
			} else {
				data["DriveCoachMatches"] = matches
				data["DriveCoachSummary"] = summary
			}
		}
	}

	h.render(c, "coach_viewer", data)
}

// HandleDriveCoachMatches returns just the matches partial for HTMX updates
func (h *Handler) HandleDriveCoachMatches(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		c.String(http.StatusUnauthorized, "Authentication required")
		return
	}
	if !user.IsAdmin && !user.IsCoach {
		c.String(http.StatusForbidden, "Access denied")
		return
	}

	data := map[string]any{
		"User": user,
	}

	if h.hasDB() {
		session, err := h.GetSession(c)
		if err == nil && session.SelectedEventID != nil {
			selectedEventID := *session.SelectedEventID
			matches, summary, loadErr := h.loadDriveCoachMatches(c.Request.Context(), selectedEventID, user.TeamNumber)
			if loadErr != nil {
				data["DriveCoachError"] = loadErr.Error()
			} else {
				data["DriveCoachMatches"] = matches
				data["DriveCoachSummary"] = summary
			}
		}
	}

	h.renderPartial(c, "drive_coach_matches", data)
}

func (h *Handler) loadDriveCoachMatches(ctx context.Context, eventID int, userTeamNumber *int) ([]driveCoachMatch, map[string]any, error) {
	if userTeamNumber == nil {
		return nil, nil, httpError("Your account is missing a team number. Add one in Account settings.")
	}

	var event struct {
		ID       int
		Name     string
		TBAKey   *string
		Timezone *string
	}

	if err := h.db.WithContext(ctx).
		Table("events").
		Select("id, name, tba_key, timezone").
		Where("id = ?", eventID).
		Scan(&event).Error; err != nil {
		return nil, nil, httpError("Unable to load selected event")
	}

	if event.TBAKey == nil || strings.TrimSpace(*event.TBAKey) == "" {
		return nil, nil, httpError("Selected event is missing schedule data")
	}

	eventCode := extractEventCode(*event.TBAKey)
	if eventCode == "" {
		return nil, nil, httpError("Unable to determine event code for schedule lookup")
	}

	season := 2026
	if seasonEnv := strings.TrimSpace(os.Getenv("FIRST_SEASON")); seasonEnv != "" {
		if parsed, err := strconv.Atoi(seasonEnv); err == nil {
			season = parsed
		}
	}

	username := strings.TrimSpace(os.Getenv("FIRST_API_USERNAME"))
	key := strings.TrimSpace(os.Getenv("FIRST_API_KEY"))
	if username == "" || key == "" {
		return nil, nil, httpError("FIRST API credentials are not configured")
	}

	filters := url.Values{}
	if *userTeamNumber > 0 {
		filters.Set("teamNumber", strconv.Itoa(*userTeamNumber))
	} else {
		filters.Set("tournamentLevel", "Qualification")
	}

	client := frc.NewClient(username, key)
	apiCtx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()

	rawMatches, err := client.GetMatchSchedule(apiCtx, season, eventCode, filters)
	if err != nil {
		if frc.IsInternetUnavailable(err) {
			return nil, nil, httpError("No internet connection available. Connect network uplink and try manual sync again.")
		}
		return nil, nil, httpError("Could not fetch match schedule from FIRST API")
	}

	h.log.Info("fetched matches from FIRST API",
		"event_code", eventCode,
		"team_number", *userTeamNumber,
		"match_count", len(rawMatches))

	teamNumbers := map[int]struct{}{}
	for _, match := range rawMatches {
		for _, team := range match.Teams {
			teamNumbers[team.TeamNumber] = struct{}{}
		}
	}

	statsByTeam := map[int]driveCoachTeam{}
	if len(teamNumbers) > 0 {
		teamNums := make([]int, 0, len(teamNumbers))
		for teamNumber := range teamNumbers {
			teamNums = append(teamNums, teamNumber)
		}

		var rows []struct {
			TeamNumber int
			OPR        *float64
			DPR        *float64
		}

		_ = h.db.WithContext(ctx).
			Table("teams").
			Select("teams.team_number, team_event_stats.opr, team_event_stats.dpr").
			Joins("LEFT JOIN team_event_stats ON team_event_stats.team_id = teams.id AND team_event_stats.event_id = ?", eventID).
			Where("teams.team_number IN ?", teamNums).
			Scan(&rows).Error

		for _, row := range rows {
			statsByTeam[row.TeamNumber] = driveCoachTeam{TeamNumber: row.TeamNumber, OPR: row.OPR, DPR: row.DPR}
		}
	}

	now := time.Now()
	results := make([]driveCoachMatch, 0, len(rawMatches))
	currentMatches := 0
	upcomingMatches := 0
	completedMatches := 0

	for _, match := range rawMatches {
		entry := driveCoachMatch{
			Description: match.Description,
			MatchNumber: match.MatchNumber,
			MatchType:   strings.ToLower(match.TournamentLevel),
			Status:      "upcoming",
			StatusLabel: "Upcoming",
			StatusClass: "border-teal-600 bg-teal-900/20",
		}

		h.log.Info("processing match",
			"match_number", match.MatchNumber,
			"start_time_raw", match.StartTime,
			"description", match.Description)

		// Try parsing with timezone first (RFC3339), then without timezone
		var startTime time.Time
		var err error

		startTime, err = time.Parse(time.RFC3339, match.StartTime)
		if err != nil {
			// Try parsing without timezone - FIRST API sometimes returns this format
			startTime, err = time.Parse("2006-01-02T15:04:05", match.StartTime)
			if err == nil {
				// Use event timezone if available, otherwise assume UTC
				var loc *time.Location
				if event.Timezone != nil && *event.Timezone != "" {
					loc, err = time.LoadLocation(*event.Timezone)
					if err != nil {
						// Invalid timezone, fall back to UTC
						loc = time.UTC
						log.Printf("Invalid timezone %s for event %s: %v", *event.Timezone, event.Name, err)
					}
				} else {
					// No timezone specified, assume UTC
					loc = time.UTC
				}

				startTime = time.Date(startTime.Year(), startTime.Month(), startTime.Day(),
					startTime.Hour(), startTime.Minute(), startTime.Second(), 0, loc)
			}
		}

		if err == nil {
			entry.StartTime = &startTime
			// Format display with day/date and time in event's timezone
			entry.TimeDisplay = startTime.Format("Mon Jan 2, 3:04 PM MST")
			entry.MinutesFromNow = int(time.Until(startTime).Minutes() * -1)

			// Status logic:
			// - Completed: match started more than 15 minutes ago (< -15)
			// - Current: match window is -15 to +15 minutes from start time
			// - Upcoming: match hasn't started yet (more than 15 minutes away)
			minutesUntilStart := int(startTime.Sub(now).Minutes())

			if minutesUntilStart < -15 {
				// Match started more than 15 minutes ago - it's completed
				entry.Status = "completed"
				entry.StatusLabel = "Completed"
				entry.StatusClass = "border-gray-700 bg-gray-900/40"
				completedMatches++
			} else if minutesUntilStart >= -15 && minutesUntilStart <= 15 {
				// Match is happening now or very soon (within ±15 minutes)
				entry.Status = "current"
				entry.StatusLabel = "Current Match"
				entry.StatusClass = "border-yellow-500 bg-yellow-900/20 ring-2 ring-yellow-500/40"
				currentMatches++
			} else {
				// Match is upcoming (more than 15 minutes in the future)
				upcomingMatches++
			}

			h.log.Info("processed match status",
				"match_number", entry.MatchNumber,
				"start_time", startTime.Format(time.RFC3339),
				"minutes_until_start", minutesUntilStart,
				"status", entry.Status,
				"time_display", entry.TimeDisplay)
		} else {
			h.log.Error("failed to parse match start time",
				"match_number", entry.MatchNumber,
				"start_time_raw", match.StartTime,
				"error", err)
		}

		for _, team := range match.Teams {
			teamStats, ok := statsByTeam[team.TeamNumber]
			if !ok {
				teamStats = driveCoachTeam{TeamNumber: team.TeamNumber}
			}

			if strings.HasPrefix(team.Station, "Red") {
				entry.RedTeams = append(entry.RedTeams, teamStats)
				if team.TeamNumber == *userTeamNumber {
					entry.OurAlliance = "Red"
				}
			} else if strings.HasPrefix(team.Station, "Blue") {
				entry.BlueTeams = append(entry.BlueTeams, teamStats)
				if team.TeamNumber == *userTeamNumber {
					entry.OurAlliance = "Blue"
				}
			}
		}

		if entry.OurAlliance == "Red" {
			for _, team := range entry.RedTeams {
				if team.TeamNumber != *userTeamNumber {
					entry.OurPartners = append(entry.OurPartners, team.TeamNumber)
				}
			}
		} else if entry.OurAlliance == "Blue" {
			for _, team := range entry.BlueTeams {
				if team.TeamNumber != *userTeamNumber {
					entry.OurPartners = append(entry.OurPartners, team.TeamNumber)
				}
			}
		}

		results = append(results, entry)
	}

	sort.Slice(results, func(i, j int) bool {
		if results[i].StartTime == nil {
			return false
		}
		if results[j].StartTime == nil {
			return true
		}
		return results[i].StartTime.Before(*results[j].StartTime)
	})

	summary := map[string]any{
		"EventName":        event.Name,
		"TeamNumber":       *userTeamNumber,
		"TotalMatches":     len(results),
		"CurrentMatches":   currentMatches,
		"UpcomingMatches":  upcomingMatches,
		"CompletedMatches": completedMatches,
	}

	h.log.Info("drive coach matches loaded",
		"total", len(results),
		"current", currentMatches,
		"upcoming", upcomingMatches,
		"completed", completedMatches)

	return results, summary, nil
}

type staticError string

func (e staticError) Error() string { return string(e) }

func httpError(msg string) error {
	return staticError(msg)
}
