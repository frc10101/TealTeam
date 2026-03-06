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
	authKey := strings.TrimSpace(os.Getenv("TBA_AUTH_KEY"))
	if authKey == "" {
		log.Fatal("TBA_AUTH_KEY environment variable not set")
	}

	databaseURL := os.Getenv("DATABASE_URL")
	if databaseURL == "" {
		databaseURL = "postgres://user:password@localhost:5432/yourdb?sslmode=disable"
	}

	// Connect to database
	db, err := gorm.Open(postgres.Open(databaseURL), &gorm.Config{})
	if err != nil {
		log.Printf("Note: Could not connect to database: %v\n", err)
		log.Println("Using hardcoded test event keys instead...")
		testWithEventKeys(authKey, []string{"2026txho1", "2026txho2", "2026ne", "2026pnw"})
		return
	}

	// Query events from database
	var events []models.Event
	db.Find(&events)

	if len(events) == 0 {
		log.Println("No events found in database, using defaults...")
		testWithEventKeys(authKey, []string{"2026txho1", "2026txho2", "2026ne", "2026pnw"})
		return
	}

	fmt.Printf("Found %d events in database:\n\n", len(events))

	eventKeys := []string{}
	for _, e := range events {
		fmt.Printf("- Event: %s\n", e.Name)
		if e.TBAKey != nil {
			fmt.Printf("  TBA Key: %s\n", *e.TBAKey)
			eventKeys = append(eventKeys, *e.TBAKey)
		}
	}

	testWithEventKeys(authKey, eventKeys)
}

func testWithEventKeys(authKey string, eventKeys []string) {
	client := frc.NewTBAClient(authKey)
	ctx := context.Background()

	fmt.Printf("\n=== Testing %d event keys ===\n", len(eventKeys))

	for _, eventKey := range eventKeys {
		fmt.Printf("\n🔍 Testing event key: %s\n", eventKey)
		event, err := client.GetEvent(ctx, eventKey)
		if err != nil {
			fmt.Printf("  ❌ Not found: %v\n\n", err)
			continue
		}

		fmt.Printf("✅ Found event: %s\n", event.Name)
		fmt.Printf("   Start: %s, End: %s\n", event.StartDate, event.EndDate)

		// Get rankings for this event
		rankings, err := client.GetEventRankings(ctx, eventKey)
		if err != nil {
			fmt.Printf("  Error fetching rankings: %v\n", err)
			continue
		}
		fmt.Printf("   Found %d teams with rankings\n", len(rankings))

		// Find team 6328 or show first few teams
		found := false
		for _, r := range rankings {
			if strings.Contains(r.TeamKey, "6328") {
				fmt.Printf("\n   🎯 Team 6328 Found!\n")
				fmt.Printf("      Matches Played (from TBA rankings): %d\n", r.MatchesPlayed)
				fmt.Printf("      Record: %d-%d-%d (total: %d)\n", r.Record.Wins, r.Record.Losses, r.Record.Ties, r.Record.Wins+r.Record.Losses+r.Record.Ties)
				found = true
				break
			}
		}

		if !found && len(rankings) > 0 {
			r := rankings[0]
			fmt.Printf("\n   (First team in rankings for reference)\n")
			fmt.Printf("      Team: %s\n", r.TeamKey)
			fmt.Printf("      Matches Played: %d\n", r.MatchesPlayed)
			fmt.Printf("      Record: %d-%d-%d\n", r.Record.Wins, r.Record.Losses, r.Record.Ties)
		}

		// Get matches for comparison
		matches, err := client.GetEventMatches(ctx, eventKey)
		if err != nil {
			fmt.Printf("  Error fetching matches: %v\n", err)
			continue
		}
		fmt.Printf("   Total matches in event: %d\n", len(matches))

		// Show match breakdown by type
		matchesByType := make(map[string]int)
		playedByType := make(map[string]int)
		for _, match := range matches {
			matchesByType[match.CompLevel]++
			if match.Alliances.Red.Score >= 0 && match.Alliances.Blue.Score >= 0 {
				playedByType[match.CompLevel]++
			}
		}

		fmt.Printf("   Match breakdown:\n")
		for _, compLevel := range []string{"qm", "ef", "sf", "f"} {
			if count, ok := matchesByType[compLevel]; ok && count > 0 {
				played := playedByType[compLevel]
				fmt.Printf("     - %s: %d total, %d played\n", compLevel, count, played)
			}
		}

		fmt.Printf("\n")
	}
}
