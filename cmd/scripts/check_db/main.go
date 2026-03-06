package main

import (
	"fmt"
	"log"
	"os"

	"gorm.io/driver/postgres"
	"gorm.io/gorm"
)

type Team struct {
	ID         uint
	TeamNumber string
}

type TeamEventStats struct {
	ID            uint
	TeamNumber    string
	MatchesPlayed int
	QualAverage   float64
	Rank          int
}

func main() {
	dbURL := os.Getenv("DATABASE_URL")
	if dbURL == "" {
		dbURL = "postgres://user:password@localhost:5432/yourdb?sslmode=disable"
	}

	db, err := gorm.Open(postgres.Open(dbURL), &gorm.Config{})
	if err != nil {
		log.Fatalf("Failed to connect: %v", err)
	}

	// Check teams
	var teams []Team
	db.Table("teams").Select("id, team_number").Limit(5).Scan(&teams)
	fmt.Println("Teams in database:")
	for _, t := range teams {
		fmt.Printf("  ID: %d, Number: %s\n", t.ID, t.TeamNumber)
	}

	// Check team_event_stats
	var stats []TeamEventStats
	db.Table("team_event_stats").Select("id, team_number, matches_played, qual_average, rank").Limit(5).Scan(&stats)
	fmt.Println("\nTeam Event Stats:")
	for _, s := range stats {
		fmt.Printf("  Team %s: Rank %d, Matches %d, QualAvg %.1f\n", s.TeamNumber, s.Rank, s.MatchesPlayed, s.QualAverage)
	}

	// Check for Team 6328 specifically
	var team6328 TeamEventStats
	db.Table("team_event_stats").Select("team_number, matches_played, qual_average, rank").Where("team_number = ?", "6328").Scan(&team6328)
	fmt.Printf("\nTeam 6328: Rank %d, Matches %d, QualAvg %.1f\n", team6328.Rank, team6328.MatchesPlayed, team6328.QualAverage)
}
