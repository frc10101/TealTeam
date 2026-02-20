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

	record := dbEvent{
		Name:        evt.Name,
		Location:    location,
		StartDate:   startDate,
		EndDate:     endDate,
		TBAKey:      eventCode,
		EventType:   evt.Type,
		DistrictKey: evt.DistrictCode,
		Week:        evt.WeekNumber,
	}

	result := db.WithContext(ctx).
		Where("tba_key = ?", eventCode).
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
