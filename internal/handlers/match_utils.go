package handlers

import (
	"context"
	"fmt"
	"time"

	"github.com/frc10101/TealTeam/internal/models"
	"gorm.io/gorm"
)

// MatchWithTeams includes match details with team context
type MatchWithTeams struct {
	ID            int
	MatchNumber   int
	MatchType     string
	ScheduledTime *time.Time
	ActualTime    *time.Time
	RedScore      int
	BlueScore     int
	Played        bool
	TeamID        int
	AllianceColor string
	Position      int
}

// MatchStatus represents the status of a match
type MatchStatus string

const (
	MatchStatusUpcoming    MatchStatus = "upcoming"
	MatchStatusInProgress  MatchStatus = "in_progress"
	MatchStatusCompleted   MatchStatus = "completed"
)

// GetMatchesForTeam retrieves all matches at an event, ordered by time
// Match participation is determined by scheduled_time and event, not scouting history
func (h *Handler) GetMatchesForTeam(ctx context.Context, eventID int, teamID int) ([]MatchWithTeams, error) {
	if !h.hasDB() {
		return nil, fmt.Errorf("database not available")
	}

	var matches []MatchWithTeams
	// Query matches for the event - team participation via schedule, not scouting data
	err := h.db.WithContext(ctx).
		Table("matches").
		Select(`
			matches.id,
			matches.match_number,
			matches.match_type,
			matches.scheduled_time,
			matches.actual_time,
			matches.red_score,
			matches.blue_score,
			matches.played,
			? as team_id,
			'' as alliance_color,
			0 as position
		`, teamID).
		Where("matches.event_id = ?", eventID).
		Order("matches.match_number").
		Scan(&matches).Error

	if err != nil && err != gorm.ErrRecordNotFound {
		return nil, fmt.Errorf("failed to query matches: %w", err)
	}

	return matches, nil
}

// GetCurrentMatchForTeam finds the currently active or next upcoming match based on time
// Uses scheduled_time to determine current match, no scouting data dependency
func (h *Handler) GetCurrentMatchForTeam(ctx context.Context, eventID int, teamID int) (*MatchWithTeams, error) {
	if !h.hasDB() {
		return nil, fmt.Errorf("database not available")
	}

	now := time.Now()

	// Get all matches for the event
	matches, err := h.GetMatchesForTeam(ctx, eventID, teamID)
	if err != nil {
		return nil, err
	}

	if len(matches) == 0 {
		return nil, fmt.Errorf("no matches available for this event")
	}

	// Find currently in-progress match (within 5 min before to 15 min after scheduled time)
	for _, match := range matches {
		if match.Played {
			continue
		}

		if match.ScheduledTime == nil {
			continue
		}

		matchWindow := match.ScheduledTime.Add(-5 * time.Minute)
		matchEnd := match.ScheduledTime.Add(15 * time.Minute)

		if now.After(matchWindow) && now.Before(matchEnd) {
			// This match is currently in progress
			return &match, nil
		}
	}

	// If no match is currently in progress, return the next upcoming match
	for _, match := range matches {
		if match.Played {
			continue
		}

		if match.ScheduledTime == nil {
			// No scheduled time, treat as upcoming
			return &match, nil
		}

		matchWindow := match.ScheduledTime.Add(-5 * time.Minute)
		if now.Before(matchWindow) {
			// This is a future match (before the 5-minute early window)
			return &match, nil
		}
	}

	return nil, fmt.Errorf("no upcoming matches available for this event")
}

// GetMatchStatus returns the current status of a match based on time
func GetMatchStatus(match *MatchWithTeams, now time.Time) MatchStatus {
	if match.Played {
		return MatchStatusCompleted
	}

	if match.ScheduledTime == nil {
		return MatchStatusUpcoming
	}

	// Match is in progress if within 5 min before to 15 min after scheduled time
	matchWindow := match.ScheduledTime.Add(-5 * time.Minute)
	matchEnd := match.ScheduledTime.Add(15 * time.Minute)

	if now.Before(matchWindow) {
		return MatchStatusUpcoming
	} else if now.Before(matchEnd) {
		return MatchStatusInProgress
	}

	return MatchStatusCompleted
}

// GetMatchesForTeamAtEvent retrieves matches grouped by status
// This helps for UI filtering - show current/next matches prominently
func (h *Handler) GetMatchesForTeamByStatus(ctx context.Context, eventID int, teamID int) (
	current *MatchWithTeams,
	upcoming []MatchWithTeams,
	completed []MatchWithTeams,
	err error,
) {
	matches, err := h.GetMatchesForTeam(ctx, eventID, teamID)
	if err != nil {
		return nil, nil, nil, err
	}

	now := time.Now()
	upcoming = []MatchWithTeams{}
	completed = []MatchWithTeams{}

	for i, match := range matches {
		status := GetMatchStatus(&match, now)
		switch status {
		case MatchStatusInProgress:
			if current == nil {
				current = &matches[i]
			}
		case MatchStatusUpcoming:
			upcoming = append(upcoming, match)
		case MatchStatusCompleted:
			completed = append(completed, match)
		}
	}

	return current, upcoming, completed, nil
}


// GetEventsForTeam retrieves events that a team is attending
// Filters by team_number in users table and event_teams registration
func (h *Handler) GetEventsForTeam(ctx context.Context, teamNumber int) ([]int, error) {
	if !h.hasDB() {
		return nil, fmt.Errorf("database not available")
	}

	var eventIDs []int
	err := h.db.WithContext(ctx).
		Table("event_teams").
		Select("DISTINCT event_teams.event_id").
		Joins("JOIN teams ON teams.id = event_teams.team_id").
		Where("teams.team_number = ?", teamNumber).
		Scan(&eventIDs).Error

	if err != nil && err != gorm.ErrRecordNotFound {
		return nil, fmt.Errorf("failed to query events for team: %w", err)
	}

	return eventIDs, nil
}

// GetAvailableEventsForUser returns events that:
// 1. The user's team (if they have one) is attending, OR
// 2. Events they can select (if they're admin/coach)
func (h *Handler) GetAvailableEventsForUser(ctx context.Context, user *models.User) ([]int, error) {
	if !h.hasDB() {
		return nil, fmt.Errorf("database not available")
	}

	var eventIDs []int

	// If user has a team number, filter to events their team attends
	if user.TeamNumber != nil && *user.TeamNumber > 0 {
		events, err := h.GetEventsForTeam(ctx, *user.TeamNumber)
		if err != nil {
			return nil, err
		}
		return events, nil
	}

	// Otherwise return all events (for coaches/admins without a team)
	err := h.db.WithContext(ctx).
		Table("events").
		Select("id").
		Order("start_date").
		Scan(&eventIDs).Error

	if err != nil && err != gorm.ErrRecordNotFound {
		return nil, fmt.Errorf("failed to query available events: %w", err)
	}

	return eventIDs, nil
}
