package frc

import (
	"context"
	"errors"
	"fmt"
	"log"
	"net/url"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/frc10101/TealTeam/internal/models"
	"gorm.io/gorm"
	"gorm.io/gorm/clause"
)

const (
	defaultSeason  = 2026
	defaultCountry = "USA"
)

var ErrSyncSkipped = errors.New("first sync skipped")

// SyncResult summarizes a sync run.
type SyncResult struct {
	Season     int `json:"season"`
	Events     int `json:"events"`
	Teams      int `json:"teams"`
	EventTeams int `json:"eventTeams"`
}

// SyncOnBoot pulls FIRST Events API data into events, teams, and event_teams.
func SyncOnBoot(db *gorm.DB) {
	if db == nil {
		return
	}

	if strings.EqualFold(os.Getenv("FIRST_SYNC_ON_BOOT"), "false") {
		log.Println("⏭️  FIRST sync skipped (FIRST_SYNC_ON_BOOT=false)")
		return
	}

	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()

	result, err := SyncNow(ctx, db)
	if err != nil {
		if errors.Is(err, ErrSyncSkipped) {
			log.Println("⏭️  FIRST sync skipped (missing FIRST_API_USERNAME or FIRST_API_KEY)")
			return
		}
		log.Printf("⚠️  FIRST sync failed: %v", err)
		return
	}

	log.Printf("✅ FIRST sync complete: events=%d teams=%d event_teams=%d", result.Events, result.Teams, result.EventTeams)
}

// SyncNow pulls FIRST Events API data using current environment configuration.
func SyncNow(ctx context.Context, db *gorm.DB) (SyncResult, error) {
	if db == nil {
		return SyncResult{}, fmt.Errorf("database unavailable")
	}

	username := strings.TrimSpace(os.Getenv("FIRST_API_USERNAME"))
	key := strings.TrimSpace(os.Getenv("FIRST_API_KEY"))
	if username == "" || key == "" {
		return SyncResult{}, ErrSyncSkipped
	}

	season := defaultSeason
	if seasonEnv := strings.TrimSpace(os.Getenv("FIRST_SEASON")); seasonEnv != "" {
		if parsed, err := strconv.Atoi(seasonEnv); err == nil {
			season = parsed
		}
	}

	client := NewClient(username, key)

	eventCodeFilter := strings.TrimSpace(os.Getenv("FIRST_EVENT_CODE"))
	teamFilter := strings.TrimSpace(os.Getenv("FIRST_TEAM_NUMBER"))
	countryFilter := strings.TrimSpace(os.Getenv("FIRST_COUNTRY"))
	if countryFilter == "" {
		countryFilter = defaultCountry
	}

	filters := url.Values{}
	if eventCodeFilter != "" {
		filters.Set("eventCode", eventCodeFilter)
	}
	if teamFilter != "" {
		filters.Set("teamNumber", teamFilter)
	}

	log.Printf("📡 FIRST sync starting (season %d)", season)
	events, err := client.GetSeasonEvents(ctx, season, filters)
	if err != nil {
		return SyncResult{}, fmt.Errorf("events fetch failed: %w", err)
	}

	if eventCodeFilter == "" && teamFilter == "" && countryFilter != "" {
		filtered := events[:0]
		for _, evt := range events {
			if strings.EqualFold(evt.Country, countryFilter) {
				filtered = append(filtered, evt)
			}
		}
		events = filtered
	}

	eventIDs := make(map[string]int)
	for _, evt := range events {
		eventCode := eventCodeValue(evt)
		if eventCode == "" {
			log.Printf("⚠️  event missing code: %s", evt.Name)
			continue
		}
		id, err := upsertEvent(ctx, db, evt)
		if err != nil {
			log.Printf("⚠️  event upsert failed (%s): %v", eventCode, err)
			continue
		}
		eventIDs[eventCode] = id
	}

	log.Printf("✅ FIRST events synced: %d", len(eventIDs))

	uniqueTeams := make(map[int]struct{})
	eventTeamCount := 0
	for eventCode, eventID := range eventIDs {
		teams, err := client.GetEventTeams(ctx, season, eventCode)
		if err != nil {
			log.Printf("⚠️  teams fetch failed (%s): %v", eventCode, err)
			continue
		}

		teamIDs := make(map[int]int)
		for _, team := range teams {
			id, err := upsertTeam(ctx, db, team)
			if err != nil {
				log.Printf("⚠️  team upsert failed (team %d): %v", team.TeamNumber, err)
				continue
			}
			teamIDs[team.TeamNumber] = id
			uniqueTeams[id] = struct{}{}
		}

		for _, teamID := range teamIDs {
			if err := upsertEventTeam(ctx, db, eventID, teamID); err != nil {
				log.Printf("⚠️  event_teams upsert failed (event %d, team %d): %v", eventID, teamID, err)
			}
		}

		eventTeamCount += len(teamIDs)
		log.Printf("✅ FIRST teams synced for %s: %d", eventCode, len(teamIDs))
	}

	return SyncResult{
		Season:     season,
		Events:     len(eventIDs),
		Teams:      len(uniqueTeams),
		EventTeams: eventTeamCount,
	}, nil
}

// SyncTeamForUser pulls FIRST Events API data for a specific team when they sign in.
// This syncs events, the specific team, and event_team relationships.
// Also fetches TBA statistics for all events the team is attending.
func SyncTeamForUser(ctx context.Context, db *gorm.DB, teamNumber int) (SyncResult, error) {
	if db == nil {
		return SyncResult{}, fmt.Errorf("database unavailable")
	}

	username := strings.TrimSpace(os.Getenv("FIRST_API_USERNAME"))
	key := strings.TrimSpace(os.Getenv("FIRST_API_KEY"))
	if username == "" || key == "" {
		return SyncResult{}, ErrSyncSkipped
	}

	season := defaultSeason
	if seasonEnv := strings.TrimSpace(os.Getenv("FIRST_SEASON")); seasonEnv != "" {
		if parsed, err := strconv.Atoi(seasonEnv); err == nil {
			season = parsed
		}
	}

	client := NewClient(username, key)

	// Filter events by team number to get events this team is attending
	filters := url.Values{}
	filters.Set("teamNumber", strconv.Itoa(teamNumber))

	log.Printf("📡 FIRST team sync starting for team %d (season %d)", teamNumber, season)
	events, err := client.GetSeasonEvents(ctx, season, filters)
	if err != nil {
		return SyncResult{}, fmt.Errorf("events fetch failed: %w", err)
	}

	if len(events) == 0 {
		log.Printf("⚠️  No events found for team %d in season %d", teamNumber, season)
		return SyncResult{
			Season:     season,
			Events:     0,
			Teams:      0,
			EventTeams: 0,
		}, nil
	}

	// Upsert events
	eventIDs := make(map[string]int)
	for _, evt := range events {
		eventCode := eventCodeValue(evt)
		if eventCode == "" {
			log.Printf("⚠️  event missing code: %s", evt.Name)
			continue
		}
		id, err := upsertEvent(ctx, db, evt)
		if err != nil {
			log.Printf("⚠️  event upsert failed (%s): %v", eventCode, err)
			continue
		}
		eventIDs[eventCode] = id
	}

	log.Printf("✅ FIRST events synced for team %d: %d", teamNumber, len(eventIDs))

	// For each event, get all teams and create event_team relationships
	teamID := 0
	uniqueTeams := make(map[int]struct{})
	eventTeamCount := 0

	for eventCode, eventID := range eventIDs {
		teams, err := client.GetEventTeams(ctx, season, eventCode)
		if err != nil {
			log.Printf("⚠️  teams fetch failed (%s): %v", eventCode, err)
			continue
		}

		// Upsert all teams from this event
		for _, team := range teams {
			id, err := upsertTeam(ctx, db, team)
			if err != nil {
				log.Printf("⚠️  team upsert failed (team %d): %v", team.TeamNumber, err)
				continue
			}

			uniqueTeams[id] = struct{}{}

			// If this is our target team, save the ID
			if team.TeamNumber == teamNumber {
				teamID = id
			}

			// Create event_team relationship for all teams
			if err := upsertEventTeam(ctx, db, eventID, id); err != nil {
				log.Printf("⚠️  event_teams upsert failed (event %d, team %d): %v", eventID, id, err)
			} else {
				eventTeamCount++
			}
		}
	}

	if teamID == 0 {
		return SyncResult{
			Season:     season,
			Events:     len(eventIDs),
			Teams:      len(uniqueTeams),
			EventTeams: 0,
		}, fmt.Errorf("team %d not found in any events", teamNumber)
	}

	log.Printf("✅ FIRST team sync complete for team %d: events=%d teams=%d event_teams=%d", teamNumber, len(eventIDs), len(uniqueTeams), eventTeamCount)

	// Sync TBA stats for all events the team is attending (done in background)
	// Use background context instead of the passed context (which may be cancelled)
	go syncTeamTBAStatsForUser(db, teamID, eventIDs)

	return SyncResult{
		Season:     season,
		Events:     len(eventIDs),
		Teams:      1,
		EventTeams: eventTeamCount,
	}, nil
}

// syncTeamTBAStatsForUser syncs TBA statistics for all events a team is attending
// This is called as a background task when a user signs up/logs in with a team
// Uses its own background context to avoid cancellation from HTTP request context
func syncTeamTBAStatsForUser(db *gorm.DB, teamID int, eventIDs map[string]int) {
	// Create background context that won't be cancelled when HTTP request completes
	ctx := context.Background()

	tbaKey := strings.TrimSpace(os.Getenv("TBA_AUTH_KEY"))
	if tbaKey == "" {
		log.Printf("ℹ️  TBA_AUTH_KEY not configured, skipping TBA stats sync for team %d", teamID)
		return
	}

	tbaClient := NewTBAClient(tbaKey)

	// Sync TBA stats for each event
	for eventCode, eventID := range eventIDs {
		// Get event details to find TBA key
		var event models.Event
		if err := db.WithContext(ctx).
			Table("events").
			Where("id = ?", eventID).
			First(&event).Error; err != nil {
			log.Printf("⚠️  Failed to fetch event details for event %d: %v", eventID, err)
			continue
		}

		if event.TBAKey == nil || *event.TBAKey == "" {
			log.Printf("⚠️  Event %d (%s) has no TBA key, skipping stats sync", eventID, eventCode)
			continue
		}

		// Set a reasonable timeout for this operation
		syncCtx, cancel := context.WithTimeout(ctx, 30*time.Second)

		// Fetch OPR data
		oprData, err := tbaClient.GetEventOPRs(syncCtx, *event.TBAKey)
		if err != nil {
			log.Printf("⚠️  Failed to fetch OPR data for event %d: %v", eventID, err)
			cancel()
			continue
		}

		// Fetch component OPR data
		componentData, err := tbaClient.GetEventComponentOPRs(syncCtx, *event.TBAKey)
		if err != nil {
			log.Printf("⚠️  Failed to fetch component OPR data for event %d: %v", eventID, err)
			// Non-critical, continue
		}

		// Fetch rankings
		rankings, err := tbaClient.GetEventRankings(syncCtx, *event.TBAKey)
		if err != nil {
			log.Printf("⚠️  Failed to fetch rankings for event %d: %v", eventID, err)
			cancel()
			continue
		}

		// Build lookup maps
		rankingsByTeamKey := make(map[string]RankingInfo)
		for _, ranking := range rankings {
			rankingsByTeamKey[ranking.TeamKey] = ranking
		}

		// Get all teams at this event and update their stats
		var eventTeams []struct {
			TeamID     int
			TBAKey     *string
			TeamNumber int
		}

		if err := db.WithContext(syncCtx).
			Table("event_teams").
			Select("teams.id as team_id, teams.tba_key, teams.team_number").
			Joins("JOIN teams ON teams.id = event_teams.team_id").
			Where("event_teams.event_id = ?", eventID).
			Scan(&eventTeams).Error; err != nil {
			log.Printf("⚠️  Failed to fetch event teams for event %d: %v", eventID, err)
			cancel()
			continue
		}

		statsUpdated := 0
		for _, et := range eventTeams {
			stats := models.TeamEventStats{
				TeamID:  et.TeamID,
				EventID: eventID,
			}

			tbaKey := et.TBAKey
			if tbaKey == nil {
				tbaKey = toPtr(fmt.Sprintf("frc%d", et.TeamNumber))
			}

			// Set OPR stats
			if opr, ok := oprData.OPRs[*tbaKey]; ok {
				stats.OPR = &opr
			}
			if dpr, ok := oprData.DPRs[*tbaKey]; ok {
				stats.DPR = &dpr
			}
			if ccwm, ok := oprData.CCWMs[*tbaKey]; ok {
				stats.CCWM = &ccwm
			}

			// Set component OPR stats from dynamic COPRs payload.
			if componentData != nil {
				autoOPR, teleopOPR, endgameOPR := componentData.TeamPhaseOPRs(*tbaKey)
				stats.AutoOPR = autoOPR
				stats.TeleopOPR = teleopOPR
				stats.EndgameOPR = endgameOPR
			}

			// Set ranking stats
			if ranking, ok := rankingsByTeamKey[*tbaKey]; ok {
				stats.Rank = &ranking.Rank
				stats.MatchesPlayed = ranking.MatchesPlayed
				stats.QualAverage = ranking.EffectiveQualAverage()
				stats.AvgMatchPoints = ranking.EffectiveAvgMatchPoints()
				stats.Wins = ranking.Record.Wins
				stats.Losses = ranking.Record.Losses
				stats.Ties = ranking.Record.Ties
				stats.DQCount = ranking.Dq
				stats.QualPoints = ranking.EffectiveQualPoints()
				stats.ElimPoints = ranking.EffectiveElimPoints()
				stats.AwardPoints = ranking.EffectiveAwardPoints()
				stats.AlliancePoints = ranking.EffectiveAlliancePoints()
				stats.TotalPoints = ranking.EffectiveTotalPoints()
			}

			// Upsert the stats
			if err := db.WithContext(syncCtx).
				Clauses(clause.OnConflict{
					Columns:   []clause.Column{{Name: "team_id"}, {Name: "event_id"}},
					DoUpdates: clause.AssignmentColumns([]string{"opr", "dpr", "ccwm", "auto_opr", "teleop_opr", "endgame_opr", "rank", "matches_played", "qual_average", "avg_match_points", "wins", "losses", "ties", "dq_count", "qual_points", "elim_points", "award_points", "alliance_points", "total_points", "updated_at"}),
				}).
				Create(&stats).Error; err != nil {
				log.Printf("⚠️  Failed to upsert team stats (team %d, event %d): %v", et.TeamID, eventID, err)
			} else {
				statsUpdated++
			}
		}

		log.Printf("✅ Synced TBA stats for %d teams at event %d", statsUpdated, eventID)
		cancel()
	}
}

type dbEvent struct {
	ID          int       `gorm:"column:id;primaryKey"`
	Name        string    `gorm:"column:name"`
	Location    string    `gorm:"column:location"`
	StartDate   time.Time `gorm:"column:start_date"`
	EndDate     time.Time `gorm:"column:end_date"`
	TBAKey      string    `gorm:"column:tba_key"`
	EventType   string    `gorm:"column:event_type"`
	DistrictKey string    `gorm:"column:district_key"`
	Week        int       `gorm:"column:week"`
}

func (dbEvent) TableName() string { return "events" }

type dbTeam struct {
	ID         int    `gorm:"column:id;primaryKey"`
	TeamNumber int    `gorm:"column:team_number"`
	Name       string `gorm:"column:name"`
	School     string `gorm:"column:school"`
	City       string `gorm:"column:city"`
	State      string `gorm:"column:state"`
	TBAKey     string `gorm:"column:tba_key"`
	Nickname   string `gorm:"column:nickname"`
	SchoolName string `gorm:"column:school_name"`
	Country    string `gorm:"column:country"`
	RookieYear int    `gorm:"column:rookie_year"`
	Website    string `gorm:"column:website"`
}

func (dbTeam) TableName() string { return "teams" }

type dbEventTeam struct {
	ID      int `gorm:"column:id;primaryKey"`
	EventID int `gorm:"column:event_id"`
	TeamID  int `gorm:"column:team_id"`
}

func (dbEventTeam) TableName() string { return "event_teams" }

func upsertEvent(ctx context.Context, db *gorm.DB, evt Event) (int, error) {
	startDate, err := parseEventDate(evt.DateStart)
	if err != nil {
		return 0, fmt.Errorf("parse start date: %w", err)
	}
	endDate, err := parseEventDate(evt.DateEnd)
	if err != nil {
		return 0, fmt.Errorf("parse end date: %w", err)
	}

	eventCode := eventCodeValue(evt)
	if eventCode == "" {
		return 0, fmt.Errorf("missing event code")
	}

	location := joinNonEmpty([]string{evt.Venue, evt.City, evt.StateProv, evt.Country}, ", ")

	// Extract year from date for TBA key format
	year := startDate.Year()

	// Construct TBA key: {year}{event_code_lowercase}
	// Example: 2026mabil, 2026week0, etc.
	tbaKey := fmt.Sprintf("%d%s", year, strings.ToLower(eventCode))

	record := dbEvent{
		Name:        evt.Name,
		Location:    location,
		StartDate:   startDate,
		EndDate:     endDate,
		TBAKey:      tbaKey,
		EventType:   evt.Type,
		DistrictKey: evt.DistrictCode,
		Week:        evt.WeekNumber,
	}

	result := db.WithContext(ctx).
		Where("tba_key = ?", tbaKey).
		Assign(record).
		FirstOrCreate(&record)
	return record.ID, result.Error
}

func upsertTeam(ctx context.Context, db *gorm.DB, team Team) (int, error) {
	name := strings.TrimSpace(team.NameShort)
	if name == "" {
		name = strings.TrimSpace(team.NameFull)
	}

	record := dbTeam{
		TeamNumber: team.TeamNumber,
		Name:       name,
		School:     team.SchoolName,
		City:       team.City,
		State:      team.StateProv,
		TBAKey:     fmt.Sprintf("frc%d", team.TeamNumber),
		Nickname:   team.NameShort,
		SchoolName: team.SchoolName,
		Country:    team.Country,
		RookieYear: team.RookieYear,
		Website:    team.Website,
	}

	result := db.WithContext(ctx).
		Where("team_number = ?", team.TeamNumber).
		Assign(record).
		FirstOrCreate(&record)
	return record.ID, result.Error
}

func upsertEventTeam(ctx context.Context, db *gorm.DB, eventID, teamID int) error {
	record := dbEventTeam{EventID: eventID, TeamID: teamID}
	return db.WithContext(ctx).
		Clauses(clause.OnConflict{DoNothing: true}).
		Create(&record).Error
}

func joinNonEmpty(parts []string, sep string) string {
	filtered := make([]string, 0, len(parts))
	for _, part := range parts {
		trimmed := strings.TrimSpace(part)
		if trimmed != "" {
			filtered = append(filtered, trimmed)
		}
	}
	return strings.Join(filtered, sep)
}

func parseEventDate(value string) (time.Time, error) {
	trimmed := strings.TrimSpace(value)
	trimmed = strings.Trim(trimmed, "\"")
	if trimmed == "" {
		return time.Time{}, fmt.Errorf("empty date")
	}

	if parsed, err := time.Parse("2006-01-02", trimmed); err == nil {
		return parsed, nil
	}
	if parsed, err := time.Parse(time.RFC3339, trimmed); err == nil {
		return parsed, nil
	}
	return time.Parse("2006-01-02T15:04:05", trimmed)
}

func eventCodeValue(evt Event) string {
	if strings.TrimSpace(evt.EventCode) != "" {
		return strings.TrimSpace(evt.EventCode)
	}
	return strings.TrimSpace(evt.Code)
}
