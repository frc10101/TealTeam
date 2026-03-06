package main

import (
	"fmt"
	"log"
	"os"

	"github.com/frc10101/TealTeam/internal/models"
	"gorm.io/driver/postgres"
	"gorm.io/gorm"
)

func main() {
	databaseURL := os.Getenv("DATABASE_URL")
	if databaseURL == "" {
		databaseURL = "host=localhost user=postgres password=postgres dbname=teal_team_dev port=5432 sslmode=disable"
	}

	db, err := gorm.Open(postgres.Open(databaseURL), &gorm.Config{})
	if err != nil {
		log.Fatalf("Failed to connect to database: %v", err)
	}

	// Find event
	var event models.Event
	if err := db.Where("tba_key = ?", "2026week0").First(&event).Error; err != nil {
		log.Fatal("Event not found:", err)
	}

	// Find team 6328
	var team models.Team
	if err := db.Where("team_number = ?", 6328).First(&team).Error; err != nil {
		log.Fatal("Team not found:", err)
	}

	// Get stats
	var stats models.TeamEventStats
	if err := db.Where("team_id = ? AND event_id = ?", team.ID, event.ID).First(&stats).Error; err != nil {
		if err == gorm.ErrRecordNotFound {
			fmt.Println("❌ No stats found for Team 6328")
			return
		}
		log.Fatal("Query error:", err)
	}

	fmt.Println("✅ Team 6328 Statistics:")
	fmt.Println()
	fmt.Printf("Event: %s\n", event.Name)
	fmt.Println()

	fmt.Println("=== RANKING DATA ===")
	if stats.Rank != nil {
		fmt.Printf("Rank: %d\n", *stats.Rank)
	} else {
		fmt.Printf("Rank: nil\n")
	}
	fmt.Printf("Matches Played: %d\n", stats.MatchesPlayed)
	fmt.Printf("Record: %d-%d-%d\n", stats.Wins, stats.Losses, stats.Ties)
	if stats.QualAverage != nil {
		fmt.Printf("Qual Average: %.4f\n", *stats.QualAverage)
	} else {
		fmt.Printf("Qual Average: nil\n")
	}
	if stats.AvgMatchPoints != nil {
		fmt.Printf("Avg Match Points: %.2f\n", *stats.AvgMatchPoints)
	} else {
		fmt.Printf("Avg Match Points: nil\n")
	}
	if stats.QualPoints != nil {
		fmt.Printf("Qual Points: %d\n", *stats.QualPoints)
	} else {
		fmt.Printf("Qual Points: nil\n")
	}
	if stats.TotalPoints != nil {
		fmt.Printf("Total Points: %d\n", *stats.TotalPoints)
	} else {
		fmt.Printf("Total Points: nil\n")
	}
	fmt.Println()

	fmt.Println("=== OPR DATA ===")
	if stats.OPR != nil {
		fmt.Printf("Overall OPR: %.4f\n", *stats.OPR)
	} else {
		fmt.Printf("Overall OPR: nil\n")
	}
	if stats.AutoOPR != nil {
		fmt.Printf("Auto OPR: %.4f\n", *stats.AutoOPR)
	} else {
		fmt.Printf("Auto OPR: nil\n")
	}
	if stats.TeleopOPR != nil {
		fmt.Printf("Teleop OPR: %.4f\n", *stats.TeleopOPR)
	} else {
		fmt.Printf("Teleop OPR: nil\n")
	}
	if stats.EndgameOPR != nil {
		fmt.Printf("Endgame OPR: %.4f\n", *stats.EndgameOPR)
	} else {
		fmt.Printf("Endgame OPR: nil\n")
	}
	if stats.DPR != nil {
		fmt.Printf("DPR: %.4f\n", *stats.DPR)
	} else {
		fmt.Printf("DPR: nil\n")
	}
	if stats.CCWM != nil {
		fmt.Printf("CCWM: %.4f\n", *stats.CCWM)
	} else {
		fmt.Printf("CCWM: nil\n")
	}
	fmt.Println()

	// Check all teams in event
	var statsList []models.TeamEventStats
	if err := db.Where("event_id = ?", event.ID).Find(&statsList).Error; err != nil {
		log.Fatal("Error finding stats:", err)
	}

	fmt.Printf("Total teams synced for this event: %d\n", len(statsList))
}
