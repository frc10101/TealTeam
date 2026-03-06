package main

import (
	"context"
	"fmt"
	"log"
	"os"
	"strings"

	"github.com/frc10101/TealTeam/internal/frc"
	"github.com/frc10101/TealTeam/internal/models"
	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/clause"
)

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
	fmt.Println("║          TBA Rankings Data Fetcher & Database Updater              ║")
	fmt.Print("╚════════════════════════════════════════════════════════════════════╝\n")

	// Fetch event info
	fmt.Printf("📋 Fetching event: %s\n", eventKey)
	event, err := client.GetEvent(ctx, eventKey)
	if err != nil {
		log.Fatalf("Failed to fetch event: %v", err)
	}
	fmt.Printf("✅ Event: %s\n", event.Name)
	fmt.Printf("   Date: %s\n\n", event.StartDate)

	// Fetch rankings
	fmt.Printf("🏆 Fetching rankings from TBA...\n")
	rankings, err := client.GetEventRankings(ctx, eventKey)
	if err != nil {
		log.Fatalf("Failed to fetch rankings: %v", err)
	}
	fmt.Printf("✅ Got %d teams in rankings\n\n", len(rankings))

	// Find or create event in database
	var dbEvent models.Event
	result := db.Where("name LIKE ?", "%"+strings.TrimSpace(event.Name)+"%").First(&dbEvent)
	if result.Error != nil {
		if result.Error == gorm.ErrRecordNotFound {
			fmt.Println("⚠️  Event not found in database, attempting to create...")
			tbaCopy := eventKey
			dbEvent = models.Event{
				Name:   event.Name,
				TBAKey: &tbaCopy,
			}
			if err := db.Create(&dbEvent).Error; err != nil {
				log.Fatalf("Failed to create event: %v", err)
			}
			fmt.Printf("✅ Created event: %s\n\n", dbEvent.Name)
		} else {
			log.Fatalf("Database error: %v", err)
		}
	} else {
		// Update event with correct TBA key if not set
		if dbEvent.TBAKey == nil || *dbEvent.TBAKey != eventKey {
			fmt.Printf("🔄 Updating event TBA key from %v to %s\n", dbEvent.TBAKey, eventKey)
			db.Model(&dbEvent).Update("tba_key", eventKey)
		}
	}

	// Display rankings table
	fmt.Println("╔═══════════════════════════════════════════════════════════════════════════════════════════╗")
	fmt.Printf("║ %-5s %-6s %-6s %-6s %-7s %-8s %-9s %-6s %-8s ║\n",
		"Rank", "Team", "Wins", "Loss", "Ties", "DQ", "Avg Pts", "Played", "Ranking")
	fmt.Println("╠═══════════════════════════════════════════════════════════════════════════════════════════╣")

	// Map to track team sync results
	synchedTeams := 0
	failedTeams := 0

	for _, ranking := range rankings {
		// Extract team number from team key (e.g., "frc6328" -> 6328)
		teamNumStr := strings.TrimPrefix(ranking.TeamKey, "frc")

		// Display ranking
		fmt.Printf("║ %-5d %-6s %-6d %-6d %-7d %-8d %-9.2f %-6d %-8d ║\n",
			ranking.Rank,
			teamNumStr,
			ranking.Record.Wins,
			ranking.Record.Losses,
			ranking.Record.Ties,
			ranking.Dq,
			func() float64 {
				if v := ranking.EffectiveQualAverage(); v != nil {
					return *v
				}
				return 0
			}(),
			ranking.MatchesPlayed,
			func() int64 {
				if v := ranking.EffectiveTotalPoints(); v != nil {
					return *v
				}
				return 0
			}())

		// Find team in database and update stats
		var team models.Team
		if err := db.Where("team_number = ?", teamNumStr).First(&team).Error; err != nil {
			if err == gorm.ErrRecordNotFound {
				failedTeams++
				continue // Skip teams not in database
			}
		}

		// Update or create team_event_stats
		stats := models.TeamEventStats{
			TeamID:         team.ID,
			EventID:        int(dbEvent.ID),
			Rank:           &ranking.Rank,
			Wins:           ranking.Record.Wins,
			Losses:         ranking.Record.Losses,
			Ties:           ranking.Record.Ties,
			DQCount:        ranking.Dq,
			QualAverage:    ranking.EffectiveQualAverage(),
			AvgMatchPoints: ranking.EffectiveAvgMatchPoints(),
			QualPoints:     ranking.EffectiveQualPoints(),
			ElimPoints:     ranking.EffectiveElimPoints(),
			AwardPoints:    ranking.EffectiveAwardPoints(),
			AlliancePoints: ranking.EffectiveAlliancePoints(),
			TotalPoints:    ranking.EffectiveTotalPoints(),
			// Calculate MatchesPlayed from record
			MatchesPlayed: ranking.Record.Wins + ranking.Record.Losses + ranking.Record.Ties,
		}

		// Upsert stats
		if err := db.Clauses(clause.OnConflict{
			Columns: []clause.Column{{Name: "team_id"}, {Name: "event_id"}},
			DoUpdates: clause.AssignmentColumns([]string{
				"rank", "wins", "losses", "ties", "dq_count", "qual_average", "avg_match_points",
				"qual_points", "elim_points", "award_points", "alliance_points",
				"total_points", "matches_played", "updated_at",
			}),
		}).Create(&stats).Error; err != nil {
			failedTeams++
			fmt.Printf("  ⚠️  Failed to update stats for team %s: %v\n", teamNumStr, err)
		} else {
			synchedTeams++
		}
	}

	fmt.Print("╚═══════════════════════════════════════════════════════════════════════════════════════════╝\n")

	// Fetch OPR data
	fmt.Printf("📊 Fetching OPR data from TBA...\n")
	oprData, err := client.GetEventOPRs(ctx, eventKey)
	if err != nil {
		fmt.Printf("⚠️  Could not fetch OPR data: %v\n", err)
	} else {
		fmt.Printf("✅ Got OPR data for event\n\n")

		// Update stats with OPR data
		updateCount := 0
		for teamNum := range oprData.OPRs {
			var team models.Team
			if err := db.Where("tba_key = ? OR team_number = ?", teamNum, strings.TrimPrefix(teamNum, "frc")).First(&team).Error; err == nil {
				opr := oprData.OPRs[teamNum]
				dpr := oprData.DPRs[teamNum]
				ccwm := oprData.CCWMs[teamNum]

				db.Model(&models.TeamEventStats{}).
					Where("team_id = ? AND event_id = ?", team.ID, dbEvent.ID).
					Updates(map[string]interface{}{
						"opr":  opr,
						"dpr":  dpr,
						"ccwm": ccwm,
					})
				updateCount++
			}
		}
		fmt.Printf("✅ Updated OPR data for %d teams\n\n", updateCount)
	}

	// Fetch component OPR data
	fmt.Printf("🎯 Fetching component OPR data from TBA...\n")
	componentData, err := client.GetEventComponentOPRs(ctx, eventKey)
	if err != nil {
		fmt.Printf("⚠️  Could not fetch component OPR data: %v\n\n", err)
	} else {
		fmt.Printf("✅ Got component OPR data for event\n\n")

		// Update stats with component OPR data
		updateCount := 0
		for _, ranking := range rankings {
			teamNum := ranking.TeamKey
			var team models.Team
			if err := db.Where("tba_key = ? OR team_number = ?", teamNum, strings.TrimPrefix(teamNum, "frc")).First(&team).Error; err == nil {
				autoOPR, teleopOPR, endgameOPR := componentData.TeamPhaseOPRs(teamNum)
				updates := map[string]interface{}{}
				if autoOPR != nil {
					updates["auto_opr"] = *autoOPR
				}
				if teleopOPR != nil {
					updates["teleop_opr"] = *teleopOPR
				}
				if endgameOPR != nil {
					updates["endgame_opr"] = *endgameOPR
				}
				if len(updates) == 0 {
					continue
				}

				db.Model(&models.TeamEventStats{}).
					Where("team_id = ? AND event_id = ?", team.ID, dbEvent.ID).
					Updates(updates)
				updateCount++
			}
		}
		fmt.Printf("✅ Updated component OPR data for %d teams\n\n", updateCount)
	}

	// Summary
	fmt.Println("╔════════════════════════════════════════════════════════════════════╗")
	fmt.Println("║                          SYNC SUMMARY                              ║")
	fmt.Println("╠════════════════════════════════════════════════════════════════════╣")
	fmt.Printf("║ Event: %-61s║\n", dbEvent.Name)
	fmt.Printf("║ Teams synced: %-52d║\n", synchedTeams)
	fmt.Printf("║ Teams failed: %-52d║\n", failedTeams)
	fmt.Printf("║ Total teams in ranking: %-44d║\n", len(rankings))
	fmt.Print("╚════════════════════════════════════════════════════════════════════╝\n")

	// Show sample team (6328)
	fmt.Println("📍 Verifying Team 6328 stats:")
	var team6328 models.Team
	if err := db.Where("team_number = ?", 6328).First(&team6328).Error; err != nil {
		fmt.Printf("  Team 6328 not in database\n")
	} else {
		var stats models.TeamEventStats
		if err := db.Where("team_id = ? AND event_id = ?", team6328.ID, dbEvent.ID).First(&stats).Error; err != nil {
			fmt.Printf("  Stats not found for team 6328\n")
		} else {
			fmt.Printf("  ✅ Team 6328 (rank %v): %d-%d-%d, %d matches played\n",
				stats.Rank, stats.Wins, stats.Losses, stats.Ties, stats.MatchesPlayed)
			if stats.OPR != nil {
				fmt.Printf("     OPR: %.2f, DPR: %.2f, CCWM: %.2f\n", *stats.OPR, *stats.DPR, *stats.CCWM)
			}
		}
	}

	fmt.Println("\n✅ Data sync complete!")
}

// Helper function to convert int64 to *int64
func toInt64Ptr(v int64) *int64 {
	return &v
}
