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

	fmt.Print("=== Manual TBA Data Sync ===\n")

	// Step 1: Find event (try to find any event that might be Week 0)
	var events []models.Event
	db.Find(&events)

	if len(events) == 0 {
		log.Fatal("No events found in database")
	}

	fmt.Printf("Found %d events in database:\n", len(events))
	for i, e := range events {
		fmt.Printf("%d. %s", i+1, e.Name)
		if e.TBAKey != nil {
			fmt.Printf(" (TBA Key: %s)", *e.TBAKey)
		}
		fmt.Printf("\n")
	}

	// For now, target the first event and update it
	event := events[0]
	correctTBAKey := "2026week0"

	fmt.Printf("\n🔄 Updating event '%s' with TBA key: %s\n", event.Name, correctTBAKey)

	// Update the event with correct TBA key
	db.Model(&event).Update("tba_key", correctTBAKey)

	fmt.Print("✅ Event updated\n")

	// Step 2: Sync team stats for this event from TBA
	fmt.Printf("🔄 Syncing team stats from TBA for event: %s\n", correctTBAKey)

	syncer := frc.NewTeamStatsSyncer(db, frc.SyncConfig{
		TBAAuthKey: authKey,
	})

	ctx := context.Background()

	// Sync stats
	if err := syncer.SyncTeamStatsForEvent(ctx, int(event.ID), correctTBAKey); err != nil {
		log.Printf("⚠️  Error syncing team stats: %v", err)
	} else {
		fmt.Println("✅ Team stats synced")
	}

	// Sync matches
	if err := syncer.SyncEventMatches(ctx, int(event.ID), correctTBAKey); err != nil {
		log.Printf("⚠️  Error syncing matches: %v", err)
	} else {
		fmt.Println("✅ Matches synced")
	}

	// Step 3: Query and display team 6328 stats
	fmt.Print("\n=== Team 6328 Data After Sync ===\n")

	var team6328 models.Team
	if err := db.Where("team_number = ?", 6328).First(&team6328).Error; err != nil {
		log.Printf("Could not find team 6328: %v", err)
		return
	}

	var stats models.TeamEventStats
	if err := db.Where("team_id = ? AND event_id = ?", team6328.ID, event.ID).First(&stats).Error; err != nil {
		log.Printf("Could not find stats for team 6328 at event: %v", err)
		return
	}

	fmt.Printf("Team: 6328 (%s)\n", team6328.Name)
	fmt.Printf("Event: %s\n\n", event.Name)
	fmt.Printf("Matches Played: %d\n", stats.MatchesPlayed)
	fmt.Printf("Record: %d-%d-%d\n", stats.Wins, stats.Losses, stats.Ties)
	fmt.Printf("Rank: %v\n", stats.Rank)
	if stats.OPR != nil {
		fmt.Printf("OPR: %.2f\n", *stats.OPR)
	}
	if stats.DPR != nil {
		fmt.Printf("DPR: %.2f\n", *stats.DPR)
	}
	if stats.CCWM != nil {
		fmt.Printf("CCWM: %.2f\n", *stats.CCWM)
	}
	if stats.AutoOPR != nil {
		fmt.Printf("Auto OPR: %.2f\n", *stats.AutoOPR)
	}
	if stats.TeleopOPR != nil {
		fmt.Printf("Teleop OPR: %.2f\n", *stats.TeleopOPR)
	}
	if stats.EndgameOPR != nil {
		fmt.Printf("Endgame OPR: %.2f\n", *stats.EndgameOPR)
	}
	if stats.QualAverage != nil {
		fmt.Printf("Qual Average: %.2f\n", *stats.QualAverage)
	}
	if stats.QualPoints != nil {
		fmt.Printf("Qual Points: %d\n", *stats.QualPoints)
	}

	// Count matches for team 6328
	var matchCount int64
	db.Table("matches").
		Where("event_id = ? AND (red_score >= 0 AND blue_score >= 0)", event.ID).
		Count(&matchCount)

	fmt.Printf("\nTotal matches in event: %d\n", matchCount)

	fmt.Println("\n✅ Sync complete!")
}
