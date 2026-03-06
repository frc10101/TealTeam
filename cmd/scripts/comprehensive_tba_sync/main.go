package main

import (
	"context"
	"fmt"
	"log"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/frc10101/TealTeam/internal/frc"
	"github.com/frc10101/TealTeam/internal/models"
	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/clause"
)

func toInt64Ptr(v int64) *int64 {
	return &v
}

func toPtr[T any](v T) *T {
	return &v
}

type dbMatch struct {
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
	tm := time.Unix(ts, 0).UTC()
	return &tm
}

func main() {
	databaseURL := os.Getenv("DATABASE_URL")
	if databaseURL == "" {
		databaseURL = "postgres://user:password@localhost:5432/yourdb?sslmode=disable"
	}

	authKey := strings.TrimSpace(os.Getenv("TBA_AUTH_KEY"))
	if authKey == "" {
		log.Fatal("TBA_AUTH_KEY environment variable required")
	}

	// Connect to database
	db, err := gorm.Open(postgres.Open(databaseURL), &gorm.Config{})
	if err != nil {
		log.Fatalf("Failed to connect to database: %v", err)
	}

	client := frc.NewTBAClient(authKey)
	ctx := context.Background()
	eventKey := "2026week0"

	fmt.Println("╔════════════════════════════════════════════════════════════════════╗")
	fmt.Println("║       Comprehensive TBA Data Sync: Rankings + Scoring              ║")
	fmt.Print("╚════════════════════════════════════════════════════════════════════╝\n")

	// Step 0: Get or create event
	fmt.Printf("📋 Fetching event: %s\n", eventKey)
	tbaTEvent, err := client.GetEvent(ctx, eventKey)
	if err != nil {
		log.Fatalf("Failed to fetch event from TBA: %v", err)
	}

	var dbEvent models.Event
	if err := db.Where("tba_key = ?", eventKey).First(&dbEvent).Error; err != nil {
		if err == gorm.ErrRecordNotFound {
			tbaCopy := eventKey
			dbEvent = models.Event{
				Name:   tbaTEvent.Name,
				TBAKey: &tbaCopy,
			}
			if err := db.Create(&dbEvent).Error; err != nil {
				log.Fatalf("Failed to create event: %v", err)
			}
		} else {
			log.Fatalf("Failed to query event: %v", err)
		}
	}
	fmt.Printf("✅ Event: %s (ID: %d)\n\n", dbEvent.Name, dbEvent.ID)

	// Step 1: Fetch rankings from TBA (SINGLE SOURCE OF TRUTH)
	fmt.Println("┌─ RANKINGS SYNC ────────────────────────────────────────────────┐")
	fmt.Printf("🏆 Fetching rankings from TBA...\n")
	rankings, err := client.GetEventRankings(ctx, eventKey)
	if err != nil {
		log.Fatalf("Failed to fetch rankings: %v", err)
	}
	fmt.Printf("✅ Got %d teams in rankings from TBA\n\n", len(rankings))

	// Step 2: Create/update all teams from rankings (don't rely on existing DB entries)
	fmt.Println("┌─ SYNCING RANKING DATA ─────────────────────────────────────────┐")
	syncedRankings := 0
	syncedTeamIDs := []int{}

	for _, ranking := range rankings {
		teamNumStr := strings.TrimPrefix(ranking.TeamKey, "frc")
		teamNum, err := strconv.Atoi(teamNumStr)
		if err != nil {
			log.Printf("⚠️  Invalid team number: %s\n", ranking.TeamKey)
			continue
		}

		// Find or CREATE team (don't skip if not found)
		var team models.Team
		result := db.Where("team_number = ?", teamNum).First(&team)

		if result.Error == gorm.ErrRecordNotFound {
			// CREATE the team from TBA data
			tbaKeyCopy := ranking.TeamKey
			team = models.Team{
				TeamNumber: teamNum,
				Name:       fmt.Sprintf("Team %d", teamNum),
				TBAKey:     &tbaKeyCopy,
			}
			if createErr := db.Create(&team).Error; createErr != nil {
				log.Printf("⚠️  Failed to create team %d: %v\n", teamNum, createErr)
				continue
			}
		} else if result.Error != nil {
			log.Printf("⚠️  Error looking up team %d: %v\n", teamNum, result.Error)
			continue
		}

		// Ensure team is linked to event
		var linkCount int64
		db.Table("event_teams").Where("team_id = ? AND event_id = ?", team.ID, dbEvent.ID).Count(&linkCount)
		if linkCount == 0 {
			if linkErr := db.Exec("INSERT INTO event_teams (team_id, event_id) VALUES (?, ?)", team.ID, dbEvent.ID).Error; linkErr != nil {
				log.Printf("⚠️  Failed to link team %d to event: %v\n", teamNum, linkErr)
				continue
			}
		}

		// Sync ranking data
		stats := models.TeamEventStats{
			TeamID:         team.ID,
			EventID:        int(dbEvent.ID),
			Rank:           toPtr(ranking.Rank),
			MatchesPlayed:  ranking.MatchesPlayed,
			QualAverage:    ranking.EffectiveQualAverage(),
			AvgMatchPoints: ranking.EffectiveAvgMatchPoints(),
			Wins:           ranking.Record.Wins,
			Losses:         ranking.Record.Losses,
			Ties:           ranking.Record.Ties,
			DQCount:        ranking.Dq,
			QualPoints:     ranking.EffectiveQualPoints(),
			ElimPoints:     ranking.EffectiveElimPoints(),
			AwardPoints:    ranking.EffectiveAwardPoints(),
			AlliancePoints: ranking.EffectiveAlliancePoints(),
			TotalPoints:    ranking.EffectiveTotalPoints(),
		}

		if upsertErr := db.Clauses(clause.OnConflict{
			Columns: []clause.Column{{Name: "team_id"}, {Name: "event_id"}},
			DoUpdates: clause.AssignmentColumns([]string{
				"rank", "wins", "losses", "ties", "dq_count", "qual_average", "avg_match_points",
				"qual_points", "elim_points", "award_points", "alliance_points",
				"total_points", "matches_played", "updated_at",
			}),
		}).Create(&stats).Error; upsertErr != nil {
			log.Printf("⚠️  Failed to sync ranking for team %d: %v\n", teamNum, upsertErr)
			continue
		}

		syncedRankings++
		syncedTeamIDs = append(syncedTeamIDs, team.ID)
	}
	fmt.Printf("✅ Synced rankings for %d teams\n\n", syncedRankings)

	// Step 3: Fetch and sync match data with scoring
	fmt.Println("┌─ SCORING DATA SYNC ────────────────────────────────────────────┐")
	fmt.Printf("📊 Fetching matches for event: %s\n", eventKey)
	matches, err := client.GetEventMatches(ctx, eventKey)
	if err != nil {
		log.Fatalf("Failed to fetch matches: %v", err)
	}
	fmt.Printf("✅ Got %d matches\n\n", len(matches))

	// Calculate average scores per team
	type scoreData struct {
		total   int
		count   int
		highest int
		lowest  int
	}
	teamScores := make(map[int]scoreData)
	syncedMatches := 0

	for _, match := range matches {
		compLevel := strings.ToLower(strings.TrimSpace(match.CompLevel))
		if compLevel == "" {
			compLevel = "qm"
		}

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
			EventID:         int(dbEvent.ID),
			MatchNumber:     normalizeMatchNumber(compLevel, match.SetNumber, match.MatchNumber),
			MatchType:       compLevel,
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

		if err := db.Clauses(clause.OnConflict{
			Columns:   []clause.Column{{Name: "event_id"}, {Name: "match_number"}, {Name: "match_type"}},
			DoUpdates: clause.AssignmentColumns([]string{"red_score", "blue_score", "played", "tba_key", "comp_level", "set_number", "scheduled_time", "actual_time", "winning_alliance", "updated_at"}),
		}).Create(&record).Error; err != nil {
			log.Printf("⚠️  Failed to upsert match %s: %v", match.Key, err)
		} else {
			syncedMatches++
		}

		// Only count played matches (valid scores)
		if match.Alliances.Red.Score < 0 || match.Alliances.Blue.Score < 0 {
			continue
		}

		// Red alliance scoring
		for _, teamKey := range match.Alliances.Red.Teams {
			teamNumStr := strings.TrimPrefix(teamKey, "frc")
			teamNum, err := strconv.Atoi(teamNumStr)
			if err != nil {
				continue
			}

			current := teamScores[teamNum]
			current.total += match.Alliances.Red.Score
			current.count++
			if match.Alliances.Red.Score > current.highest {
				current.highest = match.Alliances.Red.Score
			}
			if current.lowest == 0 || match.Alliances.Red.Score < current.lowest {
				current.lowest = match.Alliances.Red.Score
			}
			teamScores[teamNum] = current
		}

		// Blue alliance scoring
		for _, teamKey := range match.Alliances.Blue.Teams {
			teamNumStr := strings.TrimPrefix(teamKey, "frc")
			teamNum, err := strconv.Atoi(teamNumStr)
			if err != nil {
				continue
			}

			current := teamScores[teamNum]
			current.total += match.Alliances.Blue.Score
			current.count++
			if match.Alliances.Blue.Score > current.highest {
				current.highest = match.Alliances.Blue.Score
			}
			if current.lowest == 0 || match.Alliances.Blue.Score < current.lowest {
				current.lowest = match.Alliances.Blue.Score
			}
			teamScores[teamNum] = current
		}
	}
	fmt.Printf("✅ Synced %d match rows to database\n\n", syncedMatches)

	// Display scoring summary
	fmt.Println("Scoring Summary:")
	fmt.Println("╔════════════════════════════════════════════════════════════════════════╗")
	fmt.Printf("║ %-6s %-12s %-10s %-10s ║\n", "Team", "Avg Score", "Highest", "Lowest")
	fmt.Println("╠════════════════════════════════════════════════════════════════════════╣")

	for teamNum := range teamScores {
		scores := teamScores[teamNum]
		if scores.count > 0 {
			avgScore := float64(scores.total) / float64(scores.count)
			fmt.Printf("║ %-6d %-12.1f %-10d %-10d ║\n", teamNum, avgScore, scores.highest, scores.lowest)
		}
	}
	fmt.Print("╚════════════════════════════════════════════════════════════════════════╝\n")

	// Step 5: Fetch and sync OPR data
	fmt.Println("┌─ OPR DATA SYNC ────────────────────────────────────────────────┐")
	fmt.Printf("📊 Fetching OPR/DPR/CCWM data from TBA...\n")
	oprData, err := client.GetEventOPRs(ctx, eventKey)
	if err != nil {
		fmt.Printf("⚠️  Could not fetch OPR data: %v\n", err)
	} else {
		syncedOPR := 0
		for _, ranking := range rankings {
			teamNumStr := strings.TrimPrefix(ranking.TeamKey, "frc")

			// Check if OPR data exists for this team key
			opr, hasOPR := oprData.OPRs[ranking.TeamKey]
			dpr, hasDPR := oprData.DPRs[ranking.TeamKey]
			ccwm, hasCCWM := oprData.CCWMs[ranking.TeamKey]

			if !hasOPR && !hasDPR && !hasCCWM {
				continue
			}

			// Find team ID from syncedTeamIDs
			var team models.Team
			if err := db.Where("team_number = ?", teamNumStr).First(&team).Error; err != nil {
				continue
			}

			updateMap := make(map[string]interface{})
			if hasOPR {
				updateMap["opr"] = opr
			}
			if hasDPR {
				updateMap["dpr"] = dpr
			}
			if hasCCWM {
				updateMap["ccwm"] = ccwm
			}

			if err := db.Model(&models.TeamEventStats{}).
				Where("team_id = ? AND event_id = ?", team.ID, dbEvent.ID).
				Updates(updateMap).Error; err != nil {
				log.Printf("⚠️  Failed to update OPR for team %s: %v", teamNumStr, err)
				continue
			}

			syncedOPR++
		}
		fmt.Printf("✅ Synced OPR data for %d teams\n", syncedOPR)
	}

	// Step 6: Fetch and sync component OPR data
	fmt.Println("\n┌─ COMPONENT OPR DATA SYNC ──────────────────────────────────────┐")
	fmt.Printf("🎯 Fetching component OPR data from TBA...\n")
	componentData, err := client.GetEventComponentOPRs(ctx, eventKey)
	if err != nil {
		fmt.Printf("⚠️  Could not fetch component OPR data: %v\n", err)
	} else {
		syncedComponent := 0
		for _, ranking := range rankings {
			teamNumStr := strings.TrimPrefix(ranking.TeamKey, "frc")

			autoOPR, teleopOPR, endgameOPR := componentData.TeamPhaseOPRs(ranking.TeamKey)
			if autoOPR == nil && teleopOPR == nil && endgameOPR == nil {
				continue
			}

			// Find team ID
			var team models.Team
			if err := db.Where("team_number = ?", teamNumStr).First(&team).Error; err != nil {
				continue
			}

			updateMap := make(map[string]interface{})
			if autoOPR != nil {
				updateMap["auto_opr"] = *autoOPR
			}
			if teleopOPR != nil {
				updateMap["teleop_opr"] = *teleopOPR
			}
			if endgameOPR != nil {
				updateMap["endgame_opr"] = *endgameOPR
			}

			if err := db.Model(&models.TeamEventStats{}).
				Where("team_id = ? AND event_id = ?", team.ID, dbEvent.ID).
				Updates(updateMap).Error; err != nil {
				log.Printf("⚠️  Failed to update component OPR for team %s: %v", teamNumStr, err)
				continue
			}

			syncedComponent++
		}
		fmt.Printf("✅ Synced component OPR data for %d teams\n", syncedComponent)
	}

	// Step 7: Summary
	fmt.Println("\n╔════════════════════════════════════════════════════════════════════╗")
	fmt.Println("║                       ✅ SYNC COMPLETE                            ║")
	fmt.Println("╠════════════════════════════════════════════════════════════════════╣")
	fmt.Printf("║ Event: %-62s║\n", dbEvent.Name)
	fmt.Printf("║ Teams synced: %-52d║\n", syncedRankings)
	fmt.Printf("║ Matches analyzed: %-48d║\n", len(matches))
	fmt.Print("╚════════════════════════════════════════════════════════════════════╝\n")

	// Show sample data for Team 6328
	fmt.Println("\n📍 Verification - Team 6328 Stats:")
	if scores, ok := teamScores[6328]; ok {
		avgScore := float64(scores.total) / float64(scores.count)
		fmt.Printf("   Matches: %d\n", scores.count)
		fmt.Printf("   Avg Score: %.1f\n", avgScore)
		fmt.Printf("   Highest: %d\n", scores.highest)
		fmt.Printf("   Lowest: %d\n\n", scores.lowest)
	}

	// Check database for Team 6328
	var team6328Stats models.TeamEventStats
	if err := db.Where("team_id IN (SELECT id FROM teams WHERE team_number = ?) AND event_id = ?", 6328, dbEvent.ID).First(&team6328Stats).Error; err == nil {
		fmt.Printf("   DB - Rank: %v, Matches: %d, W-L-T: %d-%d-%d\n",
			team6328Stats.Rank, team6328Stats.MatchesPlayed,
			team6328Stats.Wins, team6328Stats.Losses, team6328Stats.Ties)
		fmt.Printf("   DB - OPR: %v, DPR: %v, CCWM: %v\n",
			team6328Stats.OPR, team6328Stats.DPR, team6328Stats.CCWM)
	}

	fmt.Println("\n✨ Data sync finished!")
}
