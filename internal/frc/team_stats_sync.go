package frc

import (
	"context"
	"errors"
	"fmt"
	"log"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/frc10101/TealTeam/internal/models"
	"gorm.io/gorm"
	"gorm.io/gorm/clause"
)

// SyncConfig holds configuration for team stats syncing
type SyncConfig struct {
	TBAAuthKey           string
	IntervalDuringEvent  time.Duration
	IntervalBetweenEvent time.Duration
}

// LoadSyncConfig loads sync configuration from environment variables
func LoadSyncConfig() SyncConfig {
	return SyncConfig{
		TBAAuthKey:           strings.TrimSpace(os.Getenv("TBA_AUTH_KEY")),
		IntervalDuringEvent:  2 * time.Minute,
		IntervalBetweenEvent: 3 * time.Hour,
	}
}

// TeamStatsSyncer handles periodic syncing of team statistics from TBA and FIRST APIs
type TeamStatsSyncer struct {
	db        *gorm.DB
	config    SyncConfig
	tbaClient *TBAClient
	stopCh    chan struct{}
}

// NewTeamStatsSyncer creates a new team stats syncer
func NewTeamStatsSyncer(db *gorm.DB, config SyncConfig) *TeamStatsSyncer {
	return &TeamStatsSyncer{
		db:        db,
		config:    config,
		tbaClient: NewTBAClient(config.TBAAuthKey),
		stopCh:    make(chan struct{}),
	}
}

// IsEventActive checks if an event is currently active (happening now or has matches scheduled)
func IsEventActive(event models.Event) bool {
	now := time.Now()

	if event.StartDate != nil && event.EndDate != nil {
		return now.After(*event.StartDate) && now.Before(*event.EndDate)
	}

	return false
}

// GetActiveEventsWithTeams fetches all teams and their active events
func (s *TeamStatsSyncer) GetActiveEventsWithTeams(ctx context.Context) (map[int][]models.Event, error) {
	type result struct {
		TeamID int
		Event  models.Event
	}

	var results []result

	// Get all event-team associations with event details
	if err := s.db.WithContext(ctx).
		Table("event_teams").
		Select("event_teams.team_id, events.*").
		Joins("JOIN events ON events.id = event_teams.event_id").
		Where("events.start_date <= NOW() AND events.end_date >= NOW()").
		Scan(&results).Error; err != nil {
		return nil, err
	}

	eventsByTeam := make(map[int][]models.Event)
	for _, r := range results {
		eventsByTeam[r.TeamID] = append(eventsByTeam[r.TeamID], r.Event)
	}

	return eventsByTeam, nil
}

// SyncTeamStatsForEvent syncs statistics for all teams at a specific event from TBA
func (s *TeamStatsSyncer) SyncTeamStatsForEvent(ctx context.Context, eventID int, eventTBAKey string) error {
	if s.config.TBAAuthKey == "" {
		return errors.New("TBA_AUTH_KEY not configured")
	}

	// Fetch OPR data
	oprData, err := s.tbaClient.GetEventOPRs(ctx, eventTBAKey)
	if err != nil {
		log.Printf("⚠️  Failed to fetch OPR data for event %s: %v", eventTBAKey, err)
		return err
	}

	// Fetch component OPR data
	componentData, err := s.tbaClient.GetEventComponentOPRs(ctx, eventTBAKey)
	if err != nil {
		log.Printf("⚠️  Failed to fetch component OPR data for event %s: %v", eventTBAKey, err)
		// Not critical, continue without component data
	}

	// Fetch rankings
	rankings, err := s.tbaClient.GetEventRankings(ctx, eventTBAKey)
	if err != nil {
		log.Printf("⚠️  Failed to fetch rankings for event %s: %v", eventTBAKey, err)
		return err
	}

	// Build lookup map for rankings
	rankingsByTeamKey := make(map[string]RankingInfo)
	for _, ranking := range rankings {
		rankingsByTeamKey[ranking.TeamKey] = ranking
	}

	// Get all teams at this event
	var eventTeams []struct {
		TeamID     int
		TBAKey     *string
		TeamNumber int
	}

	if err := s.db.WithContext(ctx).
		Table("event_teams").
		Select("teams.id as team_id, teams.tba_key, teams.team_number").
		Joins("JOIN teams ON teams.id = event_teams.team_id").
		Where("event_teams.event_id = ?", eventID).
		Scan(&eventTeams).Error; err != nil {
		return fmt.Errorf("failed to fetch event teams: %w", err)
	}

	// Update stats for each team
	for _, et := range eventTeams {
		stats := models.TeamEventStats{
			TeamID:  et.TeamID,
			EventID: eventID,
		}

		// Get TBA key for lookups
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

		// Upsert the stats row
		if err := s.db.WithContext(ctx).
			Clauses(clause.OnConflict{
				Columns:   []clause.Column{{Name: "team_id"}, {Name: "event_id"}},
				DoUpdates: clause.AssignmentColumns([]string{"opr", "dpr", "ccwm", "auto_opr", "teleop_opr", "endgame_opr", "rank", "matches_played", "qual_average", "avg_match_points", "wins", "losses", "ties", "dq_count", "qual_points", "elim_points", "award_points", "alliance_points", "total_points", "updated_at"}),
			}).
			Create(&stats).Error; err != nil {
			log.Printf("⚠️  Failed to upsert team stats (team %d, event %d): %v", et.TeamID, eventID, err)
			continue
		}
	}

	log.Printf("✅ Synced team stats for %d teams at event %d", len(eventTeams), eventID)
	return nil
}

// SyncEventMatches syncs match schedule and results for an event from TBA.
func (s *TeamStatsSyncer) SyncEventMatches(ctx context.Context, eventID int, eventTBAKey string) error {
	if s.config.TBAAuthKey == "" {
		return errors.New("TBA_AUTH_KEY not configured")
	}

	matches, err := s.tbaClient.GetEventMatches(ctx, eventTBAKey)
	if err != nil {
		log.Printf("⚠️  Failed to fetch matches for event %s: %v", eventTBAKey, err)
		return err
	}

	for _, match := range matches {
		compLevel := strings.ToLower(strings.TrimSpace(match.CompLevel))
		if compLevel == "" {
			compLevel = "qm"
		}

		matchType := compLevel
		matchNumber := normalizeMatchNumber(compLevel, match.SetNumber, match.MatchNumber)
		winningAlliance := ""
		if match.Alliances.Red.Score >= 0 && match.Alliances.Blue.Score >= 0 {
			if match.Alliances.Red.Score > match.Alliances.Blue.Score {
				winningAlliance = "red"
			} else if match.Alliances.Blue.Score > match.Alliances.Red.Score {
				winningAlliance = "blue"
			}
		}

		played := match.ActualTime > 0 || (match.ScoreBreakdown != nil && match.Alliances.Red.Score >= 0 && match.Alliances.Blue.Score >= 0)

		record := dbMatch{
			EventID:         eventID,
			MatchNumber:     matchNumber,
			MatchType:       matchType,
			RedScore:        match.Alliances.Red.Score,
			BlueScore:       match.Alliances.Blue.Score,
			Played:          played,
			TBAKey:          match.Key,
			CompLevel:       compLevel,
			SetNumber:       match.SetNumber,
			ScheduledTime:   unixToTimePtr(match.ScheduledTime),
			ActualTime:      unixToTimePtr(match.ActualTime),
			WinningAlliance: winningAlliance,
		}

		if err := s.db.WithContext(ctx).
			Clauses(clause.OnConflict{
				Columns:   []clause.Column{{Name: "event_id"}, {Name: "match_number"}, {Name: "match_type"}},
				DoUpdates: clause.AssignmentColumns([]string{"red_score", "blue_score", "played", "tba_key", "comp_level", "set_number", "scheduled_time", "actual_time", "winning_alliance", "updated_at"}),
			}).
			Create(&record).Error; err != nil {
			log.Printf("⚠️  Failed to upsert match %s for event %d: %v", match.Key, eventID, err)
		}
	}

	log.Printf("✅ Synced %d matches for event %d", len(matches), eventID)
	return nil
}

// SyncAllTeamStats syncs team statistics for all active events
func (s *TeamStatsSyncer) SyncAllTeamStats(ctx context.Context) error {
	// Get all active events
	var activeEvents []models.Event
	now := time.Now()
	season := seasonFromEnv()

	if err := s.db.WithContext(ctx).
		Where("start_date <= ? AND end_date >= ?", now, now).
		Find(&activeEvents).Error; err != nil {
		return fmt.Errorf("failed to fetch active events: %w", err)
	}

	if len(activeEvents) == 0 {
		log.Println("📊 No active events found, syncing all upcoming events from the past week")

		// If no active events, sync recent/future events
		weekAgo := now.AddDate(0, 0, -7)
		nextWeek := now.AddDate(0, 0, 7)

		if err := s.db.WithContext(ctx).
			Where("end_date >= ? AND start_date <= ?", weekAgo, nextWeek).
			Find(&activeEvents).Error; err != nil {
			return fmt.Errorf("failed to fetch recent events: %w", err)
		}
	}

	// Sync stats for each event
	for _, event := range activeEvents {
		if event.TBAKey == nil || *event.TBAKey == "" {
			log.Printf("⚠️  Event %d has no TBA key, skipping", event.ID)
			continue
		}

		eventKey := normalizeTBAEventKey(*event.TBAKey, season)
		if eventKey == "" {
			log.Printf("⚠️  Event %d has invalid TBA key, skipping", event.ID)
			continue
		}

		if err := s.SyncTeamStatsForEvent(ctx, event.ID, eventKey); err != nil {
			log.Printf("⚠️  Failed to sync stats for event %d: %v", event.ID, err)
			continue
		}

		if err := s.SyncEventMatches(ctx, event.ID, eventKey); err != nil {
			log.Printf("⚠️  Failed to sync matches for event %d: %v", event.ID, err)
			continue
		}
	}

	return nil
}

// DetermineSyncInterval determines whether we should use fast or slow sync interval
func (s *TeamStatsSyncer) DetermineSyncInterval() time.Duration {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	// Check if any events are active right now
	var activeEventCount int64
	now := time.Now()

	s.db.WithContext(ctx).
		Table("events").
		Where("start_date <= ? AND end_date >= ?", now, now).
		Count(&activeEventCount)

	if activeEventCount > 0 {
		log.Printf("📊 %d active events found, using fast sync interval (%v)", activeEventCount, s.config.IntervalDuringEvent)
		return s.config.IntervalDuringEvent
	}

	// Check if events are happening within the next 24 hours
	nextDay := now.Add(24 * time.Hour)
	var upcomingCount int64
	s.db.WithContext(ctx).
		Table("events").
		Where("start_date > ? AND start_date <= ?", now, nextDay).
		Count(&upcomingCount)

	if upcomingCount > 0 {
		log.Printf("📊 %d events starting in next 24 hours, using fast sync interval (%v)", upcomingCount, s.config.IntervalDuringEvent)
		return s.config.IntervalDuringEvent
	}

	log.Printf("📊 No active events, using slow sync interval (%v)", s.config.IntervalBetweenEvent)
	return s.config.IntervalBetweenEvent
}

// Start begins the background sync loop (non-blocking)
func (s *TeamStatsSyncer) Start() {
	go s.syncLoop()
	log.Println("🔄 Team stats sync loop started")
}

// Stop stops the background sync loop
func (s *TeamStatsSyncer) Stop() {
	close(s.stopCh)
	log.Println("🛑 Team stats sync loop stopped")
}

// syncLoop is the main background goroutine for syncing team stats
func (s *TeamStatsSyncer) syncLoop() {
	// Initial sync immediately
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	if err := s.SyncAllTeamStats(ctx); err != nil {
		log.Printf("⚠️  Initial team stats sync failed: %v", err)
	}
	cancel()

	// Start with default interval
	currentInterval := s.config.IntervalBetweenEvent
	ticker := time.NewTicker(currentInterval)
	defer ticker.Stop()

	for {
		select {
		case <-s.stopCh:
			log.Println("🛑 Team stats sync loop exiting")
			return
		case <-ticker.C:
			// Determine current interval based on event status
			newInterval := s.DetermineSyncInterval()

			// If interval changed, reset ticker
			if newInterval != currentInterval {
				ticker.Reset(newInterval)
				currentInterval = newInterval
			}

			// Perform sync with timeout
			ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
			if err := s.SyncAllTeamStats(ctx); err != nil {
				log.Printf("⚠️  Team stats sync failed: %v", err)
			}
			cancel()
		}
	}
}

type dbMatch struct {
	ID              int        `gorm:"column:id;primaryKey"`
	EventID         int        `gorm:"column:event_id"`
	MatchNumber     int        `gorm:"column:match_number"`
	MatchType       string     `gorm:"column:match_type"`
	RedScore        int        `gorm:"column:red_score"`
	BlueScore       int        `gorm:"column:blue_score"`
	Played          bool       `gorm:"column:played"`
	TBAKey          string     `gorm:"column:tba_key"`
	CompLevel       string     `gorm:"column:comp_level"`
	SetNumber       int        `gorm:"column:set_number"`
	ScheduledTime   *time.Time `gorm:"column:scheduled_time"`
	ActualTime      *time.Time `gorm:"column:actual_time"`
	WinningAlliance string     `gorm:"column:winning_alliance"`
}

func (dbMatch) TableName() string { return "matches" }

func seasonFromEnv() int {
	season := defaultSeason
	if seasonEnv := strings.TrimSpace(os.Getenv("FIRST_SEASON")); seasonEnv != "" {
		if parsed, err := strconv.Atoi(seasonEnv); err == nil {
			season = parsed
		}
	}
	return season
}

func normalizeTBAEventKey(raw string, season int) string {
	key := strings.ToLower(strings.TrimSpace(raw))
	if key == "" {
		return ""
	}
	seasonPrefix := strconv.Itoa(season)
	if strings.HasPrefix(key, seasonPrefix) {
		return key
	}
	return seasonPrefix + key
}

func normalizeMatchNumber(compLevel string, setNumber int, matchNumber int) int {
	if compLevel == "qm" || setNumber <= 0 {
		return matchNumber
	}
	return (setNumber * 100) + matchNumber
}

func unixToTimePtr(ts int64) *time.Time {
	if ts <= 0 {
		return nil
	}
	parsed := time.Unix(ts, 0).UTC()
	return &parsed
}

// Helper function to convert value to pointer
func toPtr[T any](v T) *T {
	return &v
}
