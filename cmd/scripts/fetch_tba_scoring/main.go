package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"os"
	"strings"

	"github.com/frc10101/TealTeam/internal/frc"
)

// ScoreBreakdown holds component scores for 2026 Reefscape season
type ScoreBreakdown struct {
	AutoFuel    int `json:"auto_fuel"`
	AutoTower   int `json:"auto_tower"`
	TeleopFuel  int `json:"teleop_fuel"`
	TeleopTower int `json:"teleop_tower"`
	EndgameNet  int `json:"endgame_net"`
	EndgameCage int `json:"endgame_cage"`
	EndgamePark int `json:"endgame_park"`
	TotalPoints int `json:"total_points"`
}

// TeamEventScoring holds aggregated scoring stats for a team at an event
type TeamEventScoring struct {
	TeamNumber      int
	AvgMatchScore   float64
	AvgAutoScore    float64
	AvgTeleopScore  float64
	AvgEndgameScore float64
	HighestScore    int
	LowestScore     int
	MatchesPlayed   int
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

	client := frc.NewTBAClient(authKey)
	ctx := context.Background()

	eventKey := "2026week0"

	fmt.Println("╔════════════════════════════════════════════════════════════════════╗")
	fmt.Println("║          TBA Scoring Data Fetcher & Analysis Tool                 ║")
	fmt.Print("╚════════════════════════════════════════════════════════════════════╝\n")

	// Fetch matches and parse scores
	fmt.Printf("📊 Fetching matches for event: %s\n", eventKey)
	matches, err := client.GetEventMatches(ctx, eventKey)
	if err != nil {
		log.Fatalf("Failed to fetch matches: %v", err)
	}
	fmt.Printf("✅ Got %d matches\n\n", len(matches))

	// Parse scoring data
	teamScores := make(map[int]*TeamEventScoring)

	fmt.Println("Parsing score breakdowns...")
	for _, match := range matches {
		// Parse Red Alliance scores
		if match.ScoreBreakdown != nil {
			parseAllianceScores(match.ScoreBreakdown, match.Alliances.Red.Teams, match.Alliances.Red.Score, teamScores)
			parseAllianceScores(match.ScoreBreakdown, match.Alliances.Blue.Teams, match.Alliances.Blue.Score, teamScores)
		}
	}

	fmt.Printf("✅ Analyzed %d teams\n\n", len(teamScores))

	// Display scoring summary table
	fmt.Println("╔═════════════════════════════════════════════════════════════════════════════════════════════════════════╗")
	fmt.Printf("║ %-6s %-12s %-12s %-12s %-12s %-10s ║\n",
		"Team", "Avg Match", "Avg Auto", "Avg Teleop", "Avg Endgame", "Highest")
	fmt.Println("╠═════════════════════════════════════════════════════════════════════════════════════════════════════════╣")

	for _, scoring := range teamScores {
		fmt.Printf("║ %-6d %-12.1f %-12.1f %-12.1f %-12.1f %-10d ║\n",
			scoring.TeamNumber,
			scoring.AvgMatchScore,
			scoring.AvgAutoScore,
			scoring.AvgTeleopScore,
			scoring.AvgEndgameScore,
			scoring.HighestScore)
	}
	fmt.Print("╚═════════════════════════════════════════════════════════════════════════════════════════════════════════╝\n")

	// Show Team 6328 in detail
	if team6328, ok := teamScores[6328]; ok {
		fmt.Println("📍 Team 6328 Detailed Scoring Analysis:")
		fmt.Printf("   Matches Played: %d\n", team6328.MatchesPlayed)
		fmt.Printf("   Average Match Score: %.1f\n", team6328.AvgMatchScore)
		fmt.Printf("   Average Auto Score: %.1f\n", team6328.AvgAutoScore)
		fmt.Printf("   Average Teleop Score: %.1f\n", team6328.AvgTeleopScore)
		fmt.Printf("   Average Endgame Score: %.1f\n", team6328.AvgEndgameScore)
		fmt.Printf("   Highest Score: %d\n", team6328.HighestScore)
		fmt.Printf("   Lowest Score: %d\n", team6328.LowestScore)
	}

	fmt.Println("\n✅ Scoring data analysis complete!")
	fmt.Println("\n💡 Tips:")
	fmt.Println("   - Use these metrics to identify team strengths (auto, teleop, endgame)")
	fmt.Println("   - Compare avg scores across teams to find partnerships")
	fmt.Println("   - Track scoring trends across events")
}

// parseAllianceScores extracts scoring data from match score breakdown
func parseAllianceScores(scoreData interface{}, teamKeys []string, allianceScore int, teamScores map[int]*TeamEventScoring) {
	// The score breakdown format varies by year/game
	// For 2026 Reefscape, we need to parse the component scores

	// Convert team keys to team numbers
	avgScorePerTeam := float64(allianceScore) / float64(len(teamKeys))

	for _, teamKey := range teamKeys {
		teamNumStr := strings.TrimPrefix(teamKey, "frc")
		var teamNum int
		fmt.Sscanf(teamNumStr, "%d", &teamNum)

		if _, ok := teamScores[teamNum]; !ok {
			teamScores[teamNum] = &TeamEventScoring{
				TeamNumber: teamNum,
			}
		}

		scoring := teamScores[teamNum]
		scoring.MatchesPlayed++
		scoring.AvgMatchScore = (scoring.AvgMatchScore*float64(scoring.MatchesPlayed-1) + avgScorePerTeam) / float64(scoring.MatchesPlayed)

		if allianceScore > scoring.HighestScore {
			scoring.HighestScore = allianceScore
		}
		if scoring.LowestScore == 0 || allianceScore < scoring.LowestScore {
			scoring.LowestScore = allianceScore
		}
	}

	// Try to parse detailed breakdown if available
	if breakdown, ok := scoreData.(map[string]interface{}); ok {
		// Parse red and blue breakdown separately
		if red, ok := breakdown["red"].(map[string]interface{}); ok {
			parseComponentScores(red, teamKeys, teamScores)
		}
		if blue, ok := breakdown["blue"].(map[string]interface{}); ok {
			parseComponentScores(blue, teamKeys, teamScores)
		}
	}
}

// parseComponentScores breaks down scoring by phase (auto, teleop, endgame)
func parseComponentScores(breakdown map[string]interface{}, teamKeys []string, teamScores map[int]*TeamEventScoring) {
	avalianceSize := float64(len(teamKeys))

	// Extract component scores
	autoScore := getIntFromMap(breakdown, "auto_fuel") + getIntFromMap(breakdown, "auto_tower")
	teleopScore := getIntFromMap(breakdown, "teleop_fuel") + getIntFromMap(breakdown, "teleop_tower")
	endgameScore := getIntFromMap(breakdown, "endgame_net") + getIntFromMap(breakdown, "endgame_cage") + getIntFromMap(breakdown, "endgame_park")

	// Distribute scores evenly across alliance teams
	avgAutoScore := float64(autoScore) / avalianceSize
	avgTeleopScore := float64(teleopScore) / avalianceSize
	avgEndgameScore := float64(endgameScore) / avalianceSize

	for _, teamKey := range teamKeys {
		teamNumStr := strings.TrimPrefix(teamKey, "frc")
		var teamNum int
		fmt.Sscanf(teamNumStr, "%d", &teamNum)

		if scoring, ok := teamScores[teamNum]; ok {
			// Update running averages
			n := float64(scoring.MatchesPlayed)
			scoring.AvgAutoScore = (scoring.AvgAutoScore*(n-1) + avgAutoScore) / n
			scoring.AvgTeleopScore = (scoring.AvgTeleopScore*(n-1) + avgTeleopScore) / n
			scoring.AvgEndgameScore = (scoring.AvgEndgameScore*(n-1) + avgEndgameScore) / n
		}
	}
}

// getIntFromMap safely gets an int from a map[string]interface{}
func getIntFromMap(m map[string]interface{}, key string) int {
	if val, ok := m[key]; ok {
		switch v := val.(type) {
		case float64:
			return int(v)
		case int:
			return v
		case json.Number:
			if i, err := v.Int64(); err == nil {
				return int(i)
			}
		}
	}
	return 0
}

// Helper function to convert int64 to *int64
func toInt64Ptr(v int64) *int64 {
	return &v
}
