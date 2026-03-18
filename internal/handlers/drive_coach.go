package handlers

import (
	"context"
	"fmt"
	"net/http"
	"time"

	"github.com/frc10101/TealTeam/internal/models"
	"github.com/gin-gonic/gin"
)

// DriveCoachMatch represents match data with team details and predictions
type DriveCoachMatch struct {
	MatchNumber      int
	MatchType        string
	ScheduledTime    *time.Time
	TimeDisplay      string
	Status           string // "completed", "current", "upcoming"
	MinutesUntil     int
	Played           bool
	UserTeamAlliance string // "red" or "blue"

	// Alliance rosters
	RedTeams  []TeamWithStats
	BlueTeams []TeamWithStats

	// Actual scores (if played)
	RedScore  int
	BlueScore int

	WinningAlliance string // actual winner if played
}

// TeamWithStats includes team info and relevant stats
type TeamWithStats struct {
	TeamNumber int
	TeamName   string
	OPR        *float64
	DPR        *float64
	CCWM       *float64
	Rank       *int
}

// HandleDriveCoach renders the drive coach dashboard
func (h *Handler) HandleDriveCoach(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}

	// Get selected event
	session, err := h.GetSession(c)
	if err != nil || session.SelectedEventID == nil {
		h.render(c, "drive_coach", map[string]any{
			"Title":       "Drive Coach Dashboard",
			"Description": "Match schedule and alliance info",
			"User":        user,
			"Error":       "Please select an event to view match schedule.",
		})
		return
	}

	eventID := *session.SelectedEventID

	// Get event details
	var event models.Event
	if err := h.db.WithContext(c.Request.Context()).
		Where("id = ?", eventID).
		First(&event).Error; err != nil {
		h.render(c, "drive_coach", map[string]any{
			"Title":       "Drive Coach Dashboard",
			"Description": "Match schedule and alliance info",
			"User":        user,
			"Error":       "Unable to load event details.",
		})
		return
	}

	// Fetch matches for this event with team assignments from TBA
	matches, err := h.getDriveCoachMatches(c.Request.Context(), eventID, user.TeamNumber)
	if err != nil {
		h.log.Error("failed to fetch drive coach matches", "error", err)
		h.render(c, "drive_coach", map[string]any{
			"Title":       "Drive Coach Dashboard",
			"Description": "Match schedule and alliance info",
			"User":        user,
			"Event":       event,
			"Error":       "Unable to load match schedule.",
		})
		return
	}

	data := map[string]any{
		"Title":       "Drive Coach Dashboard",
		"Description": "Match schedule and alliance info",
		"User":        user,
		"Event":       event,
		"Matches":     matches,
		"UserTeam":    user.TeamNumber,
	}

	h.render(c, "drive_coach", data)
}

// getDriveCoachMatches fetches and enriches match data with team stats
func (h *Handler) getDriveCoachMatches(ctx context.Context, eventID int, userTeamNumber *int) ([]DriveCoachMatch, error) {
	// Get all matches for the event
	var dbMatches []models.Match
	if err := h.db.WithContext(ctx).
		Where("event_id = ?", eventID).
		Order("match_number ASC").
		Find(&dbMatches).Error; err != nil {
		return nil, err
	}

	// Get event details for TBA key
	var event models.Event
	if err := h.db.WithContext(ctx).Where("id = ?", eventID).First(&event).Error; err != nil {
		return nil, err
	}

	// Get team stats for all teams at this event
	teamStatsMap, err := h.getTeamStatsForEvent(ctx, eventID)
	if err != nil {
		h.log.Warn("failed to load team stats", "error", err)
		teamStatsMap = make(map[int]TeamWithStats)
	}

	// Fetch alliance assignments from TBA matches
	matchTeamsMap, err := h.getMatchTeamsFromTBA(ctx, event.TBAKey)
	if err != nil {
		h.log.Warn("failed to load match teams from TBA", "error", err)
		matchTeamsMap = make(map[int]MatchTeams)
	}

	// Build enriched match list
	var matches []DriveCoachMatch
	now := time.Now()

	for _, match := range dbMatches {
		dcMatch := DriveCoachMatch{
			MatchNumber:   match.MatchNumber,
			MatchType:     match.MatchType,
			ScheduledTime: match.ScheduledTime,
			Played:        match.Played,
			RedScore:      match.RedScore,
			BlueScore:     match.BlueScore,
		}
		if match.Played {
			switch {
			case match.RedScore > match.BlueScore:
				dcMatch.WinningAlliance = "red"
			case match.BlueScore > match.RedScore:
				dcMatch.WinningAlliance = "blue"
			default:
				dcMatch.WinningAlliance = "tie"
			}
		}

		// Determine match status from schedule window even when played flag is stale.
		dcMatch.Status = "upcoming"
		if match.Played {
			dcMatch.Status = "completed"
		} else if match.ScheduledTime != nil {
			minutesUntil := int(match.ScheduledTime.Sub(now).Minutes())
			dcMatch.MinutesUntil = minutesUntil

			if minutesUntil <= 5 && minutesUntil >= -15 {
				dcMatch.Status = "current"
			} else if minutesUntil < -15 {
				dcMatch.Status = "completed"
			}

			// Include weekday + date + time to make cross-day schedules obvious.
			dcMatch.TimeDisplay = match.ScheduledTime.In(time.Local).Format("Mon Jan 2, 3:04 PM")
		}

		// Get team assignments
		teams, ok := matchTeamsMap[match.MatchNumber]
		if ok {
			// Populate red alliance
			for _, teamNum := range teams.RedTeams {
				teamStats, hasStats := teamStatsMap[teamNum]
				if !hasStats {
					teamStats = TeamWithStats{TeamNumber: teamNum}
				}
				dcMatch.RedTeams = append(dcMatch.RedTeams, teamStats)
			}

			// Populate blue alliance
			for _, teamNum := range teams.BlueTeams {
				teamStats, hasStats := teamStatsMap[teamNum]
				if !hasStats {
					teamStats = TeamWithStats{TeamNumber: teamNum}
				}
				dcMatch.BlueTeams = append(dcMatch.BlueTeams, teamStats)
			}

			// Determine user's alliance
			if userTeamNumber != nil {
				for _, ts := range dcMatch.RedTeams {
					if ts.TeamNumber == *userTeamNumber {
						dcMatch.UserTeamAlliance = "red"
						break
					}
				}
				for _, ts := range dcMatch.BlueTeams {
					if ts.TeamNumber == *userTeamNumber {
						dcMatch.UserTeamAlliance = "blue"
						break
					}
				}
			}

		}

		matches = append(matches, dcMatch)
	}

	return matches, nil
}

// MatchTeams holds team numbers for a match
type MatchTeams struct {
	RedTeams  []int
	BlueTeams []int
}

// getMatchTeamsFromTBA fetches team assignments from TBA
func (h *Handler) getMatchTeamsFromTBA(ctx context.Context, tbaKey *string) (map[int]MatchTeams, error) {
	if tbaKey == nil || *tbaKey == "" {
		return nil, fmt.Errorf("no TBA key available")
	}

	// This would ideally use the TBA client from frc package
	// For now, we'll return empty - you'll need to integrate with TBA sync
	result := make(map[int]MatchTeams)

	// TODO: Integrate with TBA client to fetch match details
	// For now, return empty map - matches will show without team details

	return result, nil
}

// getTeamStatsForEvent fetches team stats for all teams at an event
func (h *Handler) getTeamStatsForEvent(ctx context.Context, eventID int) (map[int]TeamWithStats, error) {
	var stats []struct {
		TeamID     int
		TeamNumber int
		TeamName   string
		OPR        *float64
		DPR        *float64
		CCWM       *float64
		Rank       *int
	}

	err := h.db.WithContext(ctx).
		Table("team_event_stats").
		Select(`
			team_event_stats.team_id,
			teams.team_number,
			teams.name as team_name,
			team_event_stats.opr,
			team_event_stats.dpr,
			team_event_stats.ccwm,
			team_event_stats.rank
		`).
		Joins("JOIN teams ON teams.id = team_event_stats.team_id").
		Where("team_event_stats.event_id = ?", eventID).
		Scan(&stats).Error

	if err != nil {
		return nil, err
	}

	result := make(map[int]TeamWithStats)
	for _, s := range stats {
		result[s.TeamNumber] = TeamWithStats{
			TeamNumber: s.TeamNumber,
			TeamName:   s.TeamName,
			OPR:        s.OPR,
			DPR:        s.DPR,
			CCWM:       s.CCWM,
			Rank:       s.Rank,
		}
	}

	return result, nil
}
