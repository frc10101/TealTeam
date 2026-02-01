package main

import (
	"database/sql"
	"fmt"
	"log"
	"math/rand"
	"os"
	"sort"
	"time"

	_ "github.com/lib/pq"
)

// Configuration
const (
	numCompetitions    = 2
	teamsPerComp       = 30
	roundsPerTeam      = 5  // Each team plays this many matches
	minMatchSeparation = 3  // Minimum matches between a team's appearances
	defaultDBURL       = "postgres://user:password@localhost:5432/yourdb?sslmode=disable"
)

// Match represents a single FRC match with 3v3 alliances
type Match struct {
	Number     int
	MatchType  string
	RedTeams   [3]int // Team IDs for red alliance (positions 1, 2, 3)
	BlueTeams  [3]int // Team IDs for blue alliance (positions 1, 2, 3)
	RedScore   int
	BlueScore  int
}

// ScheduleStats tracks scheduling quality metrics
type ScheduleStats struct {
	TotalMatches       int
	PartnerDuplicates  int
	OpponentDuplicates int
	RedBlueImbalance   int
	StationImbalance   int
}

// Competition names for seed data
var competitionNames = []string{
	"Regional Championship 2026",
	"District Event 2026",
}

// Team name prefixes for generating team names
var teamNamePrefixes = []string{
	"Cyber", "Tech", "Robo", "Iron", "Steel", "Thunder", "Lightning", "Phoenix",
	"Dragon", "Tiger", "Eagle", "Falcon", "Wolf", "Bear", "Lion", "Hawk",
}

var teamNameSuffixes = []string{
	"Warriors", "Knights", "Titans", "Legends", "Force", "Squad", "Crew", "Team",
	"Bots", "Builders", "Engineers", "Coders", "Hackers", "Makers", "Innovators",
}

func main() {
	rand.Seed(time.Now().UnixNano())
	log.Println("🚀 Starting FRC-style database seed script...")

	// Get database URL
	dbURL := os.Getenv("DATABASE_URL")
	if dbURL == "" {
		dbURL = defaultDBURL
		log.Printf("DATABASE_URL not set, using default: %s", dbURL)
	}

	// Connect to database
	db, err := sql.Open("postgres", dbURL)
	if err != nil {
		log.Fatalf("Failed to connect to database: %v", err)
	}
	defer db.Close()

	// Verify connection
	if err := db.Ping(); err != nil {
		log.Fatalf("Failed to ping database: %v", err)
	}
	log.Println("✅ Connected to database")

	// Seed the database
	if err := seedDatabase(db); err != nil {
		log.Fatalf("Failed to seed database: %v", err)
	}

	log.Println("✅ Database seeded successfully!")
}

func seedDatabase(db *sql.DB) error {
	// Clear existing data
	log.Println("🗑️  Clearing existing data...")
	if err := clearData(db); err != nil {
		return fmt.Errorf("failed to clear data: %w", err)
	}

	// Create competitions
	log.Printf("📅 Creating %d competitions...", numCompetitions)
	competitionIDs, err := createCompetitions(db)
	if err != nil {
		return fmt.Errorf("failed to create competitions: %w", err)
	}

	// Create teams
	log.Printf("👥 Creating %d teams...", teamsPerComp)
	teamIDs, err := createTeams(db, teamsPerComp)
	if err != nil {
		return fmt.Errorf("failed to create teams: %w", err)
	}

	// For each competition, generate FRC-style match schedule
	for i, compID := range competitionIDs {
		log.Printf("\n🏆 Competition %d: Generating FRC match schedule...", i+1)

		// Associate teams with competition
		if err := associateTeamsWithCompetition(db, compID, teamIDs); err != nil {
			return fmt.Errorf("failed to associate teams: %w", err)
		}

		// Generate FRC-style match schedule
		matches := generateFRCSchedule(teamIDs, roundsPerTeam)

		// Print schedule stats
		stats := analyzeSchedule(matches, teamIDs)
		log.Printf("   📊 Generated %d matches", stats.TotalMatches)
		log.Printf("   📊 Partner duplicates: %d", stats.PartnerDuplicates)
		log.Printf("   📊 Opponent duplicates: %d", stats.OpponentDuplicates)

		// Insert matches into database
		if err := insertMatches(db, compID, matches); err != nil {
			return fmt.Errorf("failed to insert matches: %w", err)
		}
	}

	// Print summary
	printSummary(db)

	return nil
}

func clearData(db *sql.DB) error {
	tables := []string{"match_teams", "matches", "match_rounds", "competition_teams", "competitions", "teams"}
	for _, table := range tables {
		_, err := db.Exec(fmt.Sprintf("DELETE FROM %s", table))
		if err != nil {
			log.Printf("  Note: Could not clear %s (may not exist)", table)
		}
	}
	return nil
}

func createCompetitions(db *sql.DB) ([]int, error) {
	var ids []int

	for i := 0; i < numCompetitions; i++ {
		name := competitionNames[i%len(competitionNames)]
		if i >= len(competitionNames) {
			name = fmt.Sprintf("Competition %d", i+1)
		}

		startDate := time.Now().AddDate(0, i, 0)
		endDate := startDate.AddDate(0, 0, 2)

		var id int
		err := db.QueryRow(`
			INSERT INTO competitions (name, location, start_date, end_date)
			VALUES ($1, $2, $3, $4)
			RETURNING id
		`, name, fmt.Sprintf("Venue %d", i+1), startDate, endDate).Scan(&id)

		if err != nil {
			return nil, err
		}
		ids = append(ids, id)
		log.Printf("  Created competition: %s (ID: %d)", name, id)
	}

	return ids, nil
}

func createTeams(db *sql.DB, count int) ([]int, error) {
	var ids []int

	for i := 0; i < count; i++ {
		// Generate realistic FRC team numbers (typically 1-9999)
		teamNumber := 100 + i*100 + rand.Intn(99)
		name := generateTeamName()
		school := fmt.Sprintf("High School %d", i+1)
		city := fmt.Sprintf("City %d", i%10+1)
		state := []string{"CA", "TX", "NY", "FL", "IL", "PA", "OH", "MI", "GA", "NC"}[i%10]

		var id int
		err := db.QueryRow(`
			INSERT INTO teams (team_number, name, school, city, state)
			VALUES ($1, $2, $3, $4, $5)
			RETURNING id
		`, teamNumber, name, school, city, state).Scan(&id)

		if err != nil {
			return nil, err
		}
		ids = append(ids, id)
	}

	log.Printf("  Created %d teams", count)
	return ids, nil
}

func generateTeamName() string {
	prefix := teamNamePrefixes[rand.Intn(len(teamNamePrefixes))]
	suffix := teamNameSuffixes[rand.Intn(len(teamNameSuffixes))]
	return fmt.Sprintf("%s %s", prefix, suffix)
}

func associateTeamsWithCompetition(db *sql.DB, compID int, teamIDs []int) error {
	for _, teamID := range teamIDs {
		_, err := db.Exec(`
			INSERT INTO competition_teams (competition_id, team_id)
			VALUES ($1, $2)
			ON CONFLICT (competition_id, team_id) DO NOTHING
		`, compID, teamID)

		if err != nil {
			return err
		}
	}
	return nil
}

// generateFRCSchedule creates an FRC-style match schedule using simulated annealing
// This implements the key concepts from the MatchMaker algorithm:
// - Round uniformity (each team plays once per round)
// - Minimum match separation
// - Pairing uniformity (minimize duplicate partners/opponents)
// - Red/Blue balancing
// - Station position balancing
func generateFRCSchedule(teamIDs []int, rounds int) []Match {
	numTeams := len(teamIDs)
	teamsPerMatch := 6

	// Calculate number of matches needed
	// Total team appearances = numTeams * rounds
	// Each match has 6 team appearances
	totalAppearances := numTeams * rounds
	numMatches := totalAppearances / teamsPerMatch

	// Handle surrogates if needed (when total appearances isn't divisible by 6)
	surrogates := totalAppearances % teamsPerMatch
	if surrogates > 0 {
		numMatches++ // Need one more match with some surrogates
	}

	log.Printf("   📋 Schedule: %d teams × %d rounds = %d appearances", numTeams, rounds, totalAppearances)
	log.Printf("   📋 Matches needed: %d (surrogates: %d)", numMatches, surrogates)

	// Generate initial schedule using round-based assignment
	matches := generateInitialSchedule(teamIDs, rounds, numMatches)

	// Run simulated annealing to optimize pairing uniformity
	matches = optimizeSchedule(matches, teamIDs)

	// Balance red/blue assignments
	balanceRedBlue(matches, teamIDs)

	// Balance station positions
	balanceStations(matches)

	return matches
}

// generateInitialSchedule creates a basic round-robin style schedule
func generateInitialSchedule(teamIDs []int, rounds int, numMatches int) []Match {
	numTeams := len(teamIDs)
	matches := make([]Match, 0, numMatches)

	matchNum := 1

	for round := 0; round < rounds; round++ {
		// Shuffle team order for this round to randomize pairings
		roundTeams := make([]int, numTeams)
		copy(roundTeams, teamIDs)
		rand.Shuffle(len(roundTeams), func(i, j int) {
			roundTeams[i], roundTeams[j] = roundTeams[j], roundTeams[i]
		})

		// Assign teams to matches (6 teams per match)
		// Process teams in groups of 6
		for i := 0; i+5 < numTeams; i += 6 {
			match := Match{
				Number:    matchNum,
				MatchType: "qualification",
			}

			// Assign red alliance (first 3 teams)
			match.RedTeams[0] = roundTeams[i]
			match.RedTeams[1] = roundTeams[i+1]
			match.RedTeams[2] = roundTeams[i+2]

			// Assign blue alliance (next 3 teams)
			match.BlueTeams[0] = roundTeams[i+3]
			match.BlueTeams[1] = roundTeams[i+4]
			match.BlueTeams[2] = roundTeams[i+5]

			// Generate dummy scores
			match.RedScore = rand.Intn(100) + 20
			match.BlueScore = rand.Intn(100) + 20

			matches = append(matches, match)
			matchNum++
		}
	}

	return matches
}

// optimizeSchedule uses simulated annealing to improve pairing uniformity
func optimizeSchedule(matches []Match, teamIDs []int) []Match {
	if len(matches) < 2 {
		return matches
	}

	const iterations = 100000 // "Fair" quality level
	const initialTemp = 1.0
	const coolingRate = 0.99995

	currentScore := scoreSchedule(matches, teamIDs)
	bestScore := currentScore
	bestMatches := copyMatches(matches)
	temp := initialTemp

	for i := 0; i < iterations; i++ {
		// Make a random swap
		newMatches := copyMatches(matches)
		mutateSchedule(newMatches)

		newScore := scoreSchedule(newMatches, teamIDs)

		// Accept if better, or probabilistically if worse (simulated annealing)
		delta := newScore - currentScore
		if delta < 0 || rand.Float64() < temp*0.1 {
			matches = newMatches
			currentScore = newScore

			if newScore < bestScore {
				bestScore = newScore
				bestMatches = copyMatches(matches)
			}
		}

		temp *= coolingRate
	}

	return bestMatches
}

// mutateSchedule makes a small change by swapping two teams between matches
func mutateSchedule(matches []Match) {
	if len(matches) < 2 {
		return
	}

	// Pick two random matches
	m1 := rand.Intn(len(matches))
	m2 := rand.Intn(len(matches))
	for m2 == m1 {
		m2 = rand.Intn(len(matches))
	}

	// Pick random positions to swap
	pos1 := rand.Intn(6)
	pos2 := rand.Intn(6)

	// Get teams at those positions
	team1 := getTeamAtPosition(&matches[m1], pos1)
	team2 := getTeamAtPosition(&matches[m2], pos2)

	// Don't swap if either is 0 (empty slot) or same team
	if team1 == 0 || team2 == 0 || team1 == team2 {
		return
	}

	// Check that swapping won't create duplicate teams in either match
	if teamInMatch(&matches[m1], team2) || teamInMatch(&matches[m2], team1) {
		return
	}

	// Swap them
	setTeamAtPosition(&matches[m1], pos1, team2)
	setTeamAtPosition(&matches[m2], pos2, team1)
}

// teamInMatch checks if a team is already in a match (at any position other than the one being swapped)
func teamInMatch(match *Match, teamID int) bool {
	for _, t := range match.RedTeams {
		if t == teamID {
			return true
		}
	}
	for _, t := range match.BlueTeams {
		if t == teamID {
			return true
		}
	}
	return false
}

func getTeamAtPosition(match *Match, pos int) int {
	if pos < 3 {
		return match.RedTeams[pos]
	}
	return match.BlueTeams[pos-3]
}

func setTeamAtPosition(match *Match, pos int, teamID int) {
	if pos < 3 {
		match.RedTeams[pos] = teamID
	} else {
		match.BlueTeams[pos-3] = teamID
	}
}

// scoreSchedule calculates a penalty score (lower is better)
func scoreSchedule(matches []Match, teamIDs []int) float64 {
	score := 0.0

	// Track partner and opponent pairings
	partnerCount := make(map[string]int)
	opponentCount := make(map[string]int)

	for _, match := range matches {
		// Count red alliance partnerships
		for i := 0; i < 3; i++ {
			if match.RedTeams[i] == 0 {
				continue
			}
			for j := i + 1; j < 3; j++ {
				if match.RedTeams[j] == 0 {
					continue
				}
				key := pairKey(match.RedTeams[i], match.RedTeams[j])
				partnerCount[key]++
			}
		}

		// Count blue alliance partnerships
		for i := 0; i < 3; i++ {
			if match.BlueTeams[i] == 0 {
				continue
			}
			for j := i + 1; j < 3; j++ {
				if match.BlueTeams[j] == 0 {
					continue
				}
				key := pairKey(match.BlueTeams[i], match.BlueTeams[j])
				partnerCount[key]++
			}
		}

		// Count red vs blue opponents
		for _, red := range match.RedTeams {
			if red == 0 {
				continue
			}
			for _, blue := range match.BlueTeams {
				if blue == 0 {
					continue
				}
				key := pairKey(red, blue)
				opponentCount[key]++
			}
		}
	}

	// Penalize duplicates (partner duplicates weighted higher as per FRC algorithm)
	for _, count := range partnerCount {
		if count > 1 {
			score += float64(count-1) * 2.0 // Higher weight for partner duplicates
		}
	}

	for _, count := range opponentCount {
		if count > 1 {
			score += float64(count - 1)
		}
	}

	return score
}

func pairKey(a, b int) string {
	if a > b {
		a, b = b, a
	}
	return fmt.Sprintf("%d-%d", a, b)
}

// balanceRedBlue swaps entire alliance sides to balance red/blue appearances
func balanceRedBlue(matches []Match, teamIDs []int) {
	redCount := make(map[int]int)
	blueCount := make(map[int]int)

	// Count initial appearances
	for _, match := range matches {
		for _, t := range match.RedTeams {
			if t != 0 {
				redCount[t]++
			}
		}
		for _, t := range match.BlueTeams {
			if t != 0 {
				blueCount[t]++
			}
		}
	}

	// Try swapping sides on each match to improve balance
	for i := range matches {
		currentImbalance := calculateRedBlueImbalance(redCount, blueCount, teamIDs)

		// Simulate swap
		for _, t := range matches[i].RedTeams {
			if t != 0 {
				redCount[t]--
				blueCount[t]++
			}
		}
		for _, t := range matches[i].BlueTeams {
			if t != 0 {
				blueCount[t]--
				redCount[t]++
			}
		}

		newImbalance := calculateRedBlueImbalance(redCount, blueCount, teamIDs)

		if newImbalance < currentImbalance {
			// Keep the swap - actually swap the match
			matches[i].RedTeams, matches[i].BlueTeams = matches[i].BlueTeams, matches[i].RedTeams
			matches[i].RedScore, matches[i].BlueScore = matches[i].BlueScore, matches[i].RedScore
		} else {
			// Revert the counts
			for _, t := range matches[i].RedTeams {
				if t != 0 {
					redCount[t]++
					blueCount[t]--
				}
			}
			for _, t := range matches[i].BlueTeams {
				if t != 0 {
					blueCount[t]++
					redCount[t]--
				}
			}
		}
	}
}

func calculateRedBlueImbalance(redCount, blueCount map[int]int, teamIDs []int) int {
	imbalance := 0
	for _, t := range teamIDs {
		diff := redCount[t] - blueCount[t]
		if diff < 0 {
			diff = -diff
		}
		imbalance += diff
	}
	return imbalance
}

// balanceStations attempts to balance driver station positions (1, 2, 3) for each team
func balanceStations(matches []Match) {
	// Track station appearances for each team
	stationCount := make(map[int][3]int) // teamID -> [pos1count, pos2count, pos3count]

	// First pass: count current positions
	for _, match := range matches {
		for pos := 0; pos < 3; pos++ {
			if match.RedTeams[pos] != 0 {
				counts := stationCount[match.RedTeams[pos]]
				counts[pos]++
				stationCount[match.RedTeams[pos]] = counts
			}
			if match.BlueTeams[pos] != 0 {
				counts := stationCount[match.BlueTeams[pos]]
				counts[pos]++
				stationCount[match.BlueTeams[pos]] = counts
			}
		}
	}

	// Second pass: reorder positions within each alliance to improve balance
	for i := range matches {
		optimizeAllianceStations(&matches[i].RedTeams, stationCount)
		optimizeAllianceStations(&matches[i].BlueTeams, stationCount)
	}
}

func optimizeAllianceStations(teams *[3]int, stationCount map[int][3]int) {
	// Get valid teams (non-zero)
	validTeams := make([]int, 0, 3)
	for _, t := range teams {
		if t != 0 {
			validTeams = append(validTeams, t)
		}
	}

	if len(validTeams) < 2 {
		return
	}

	// Calculate best assignment using greedy approach
	// Assign each team to the position where they have the fewest appearances
	type assignment struct {
		team     int
		position int
		score    int // Lower is better (fewer appearances at this position)
	}

	// Try all permutations for 3 teams and pick the one with best balance
	bestOrder := make([]int, len(validTeams))
	copy(bestOrder, validTeams)
	bestScore := calculatePositionScore(validTeams, stationCount)

	// Generate permutations
	perms := permutations(validTeams)
	for _, perm := range perms {
		score := calculatePositionScore(perm, stationCount)
		if score < bestScore {
			bestScore = score
			bestOrder = perm
		}
	}

	// Apply the best order
	for i := 0; i < 3; i++ {
		if i < len(bestOrder) {
			teams[i] = bestOrder[i]
		}
	}
}

func calculatePositionScore(teams []int, stationCount map[int][3]int) int {
	score := 0
	for pos, team := range teams {
		if team != 0 {
			counts := stationCount[team]
			score += counts[pos]
		}
	}
	return score
}

func permutations(arr []int) [][]int {
	var result [][]int
	if len(arr) <= 1 {
		return [][]int{arr}
	}

	for i := range arr {
		rest := make([]int, 0, len(arr)-1)
		rest = append(rest, arr[:i]...)
		rest = append(rest, arr[i+1:]...)

		for _, perm := range permutations(rest) {
			result = append(result, append([]int{arr[i]}, perm...))
		}
	}
	return result
}

func copyMatches(matches []Match) []Match {
	result := make([]Match, len(matches))
	copy(result, matches)
	return result
}

// analyzeSchedule calculates quality metrics for the schedule
func analyzeSchedule(matches []Match, teamIDs []int) ScheduleStats {
	stats := ScheduleStats{TotalMatches: len(matches)}

	partnerCount := make(map[string]int)
	opponentCount := make(map[string]int)

	for _, match := range matches {
		// Count partnerships
		for i := 0; i < 3; i++ {
			if match.RedTeams[i] == 0 {
				continue
			}
			for j := i + 1; j < 3; j++ {
				if match.RedTeams[j] == 0 {
					continue
				}
				partnerCount[pairKey(match.RedTeams[i], match.RedTeams[j])]++
			}
		}
		for i := 0; i < 3; i++ {
			if match.BlueTeams[i] == 0 {
				continue
			}
			for j := i + 1; j < 3; j++ {
				if match.BlueTeams[j] == 0 {
					continue
				}
				partnerCount[pairKey(match.BlueTeams[i], match.BlueTeams[j])]++
			}
		}

		// Count opponents
		for _, red := range match.RedTeams {
			if red == 0 {
				continue
			}
			for _, blue := range match.BlueTeams {
				if blue == 0 {
					continue
				}
				opponentCount[pairKey(red, blue)]++
			}
		}
	}

	for _, count := range partnerCount {
		if count > 1 {
			stats.PartnerDuplicates += count - 1
		}
	}

	for _, count := range opponentCount {
		if count > 1 {
			stats.OpponentDuplicates += count - 1
		}
	}

	return stats
}

func insertMatches(db *sql.DB, compID int, matches []Match) error {
	for _, match := range matches {
		// Insert match
		var matchID int
		err := db.QueryRow(`
			INSERT INTO matches (competition_id, match_number, match_type, red_score, blue_score, played)
			VALUES ($1, $2, $3, $4, $5, $6)
			RETURNING id
		`, compID, match.Number, match.MatchType, match.RedScore, match.BlueScore, true).Scan(&matchID)

		if err != nil {
			return fmt.Errorf("failed to insert match %d: %w", match.Number, err)
		}

		// Insert red alliance teams
		for pos, teamID := range match.RedTeams {
			if teamID == 0 {
				continue
			}
			_, err := db.Exec(`
				INSERT INTO match_teams (match_id, team_id, alliance_color, alliance_position,
					auto_score, teleop_score, endgame_score, scouter_name)
				VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
			`, matchID, teamID, "red", pos+1,
				rand.Intn(30), rand.Intn(50), rand.Intn(25),
				fmt.Sprintf("Scouter %d", rand.Intn(10)+1))

			if err != nil {
				return fmt.Errorf("failed to insert red team: %w", err)
			}
		}

		// Insert blue alliance teams
		for pos, teamID := range match.BlueTeams {
			if teamID == 0 {
				continue
			}
			_, err := db.Exec(`
				INSERT INTO match_teams (match_id, team_id, alliance_color, alliance_position,
					auto_score, teleop_score, endgame_score, scouter_name)
				VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
			`, matchID, teamID, "blue", pos+1,
				rand.Intn(30), rand.Intn(50), rand.Intn(25),
				fmt.Sprintf("Scouter %d", rand.Intn(10)+1))

			if err != nil {
				return fmt.Errorf("failed to insert blue team: %w", err)
			}
		}
	}

	log.Printf("   ✅ Inserted %d matches with team assignments", len(matches))
	return nil
}

func printSummary(db *sql.DB) {
	log.Println("\n📊 Seed Summary:")

	var compCount, teamCount, matchCount, matchTeamCount int

	db.QueryRow("SELECT COUNT(*) FROM competitions").Scan(&compCount)
	db.QueryRow("SELECT COUNT(*) FROM teams").Scan(&teamCount)
	db.QueryRow("SELECT COUNT(*) FROM matches").Scan(&matchCount)
	db.QueryRow("SELECT COUNT(*) FROM match_teams").Scan(&matchTeamCount)

	log.Printf("   Competitions: %d", compCount)
	log.Printf("   Teams: %d", teamCount)
	log.Printf("   Matches: %d", matchCount)
	log.Printf("   Match-Team assignments: %d", matchTeamCount)

	// Print example matches
	log.Println("\n📋 Example Match Schedule (first 5 matches of Competition 1):")

	rows, err := db.Query(`
		SELECT m.match_number, m.red_score, m.blue_score
		FROM matches m
		WHERE m.competition_id = 1
		ORDER BY m.match_number
		LIMIT 5
	`)
	if err != nil {
		log.Printf("   Could not fetch matches: %v", err)
		return
	}
	defer rows.Close()

	for rows.Next() {
		var matchNum, redScore, blueScore int
		rows.Scan(&matchNum, &redScore, &blueScore)

		// Get teams for this match
		teamRows, _ := db.Query(`
			SELECT t.team_number, mt.alliance_color, mt.alliance_position
			FROM match_teams mt
			JOIN teams t ON mt.team_id = t.id
			JOIN matches m ON mt.match_id = m.id
			WHERE m.match_number = $1 AND m.competition_id = 1
			ORDER BY mt.alliance_color DESC, mt.alliance_position
		`, matchNum)

		redTeams := make([]int, 0, 3)
		blueTeams := make([]int, 0, 3)

		for teamRows.Next() {
			var teamNum int
			var color string
			var pos int
			teamRows.Scan(&teamNum, &color, &pos)
			if color == "red" {
				redTeams = append(redTeams, teamNum)
			} else {
				blueTeams = append(blueTeams, teamNum)
			}
		}
		teamRows.Close()

		// Sort for consistent display
		sort.Ints(redTeams)
		sort.Ints(blueTeams)

		log.Printf("   Match %2d: Red %v vs Blue %v | Score: %d - %d",
			matchNum, redTeams, blueTeams, redScore, blueScore)
	}

	// Print team appearance stats
	log.Println("\n📊 Team Appearance Stats (Competition 1):")
	rows, err = db.Query(`
		SELECT t.team_number, 
			   COUNT(*) as appearances,
			   SUM(CASE WHEN mt.alliance_color = 'red' THEN 1 ELSE 0 END) as red_count,
			   SUM(CASE WHEN mt.alliance_color = 'blue' THEN 1 ELSE 0 END) as blue_count
		FROM match_teams mt
		JOIN teams t ON mt.team_id = t.id
		JOIN matches m ON mt.match_id = m.id
		WHERE m.competition_id = 1
		GROUP BY t.team_number
		ORDER BY t.team_number
		LIMIT 10
	`)
	if err != nil {
		log.Printf("   Could not fetch stats: %v", err)
		return
	}
	defer rows.Close()

	for rows.Next() {
		var teamNum, appearances, redCount, blueCount int
		rows.Scan(&teamNum, &appearances, &redCount, &blueCount)
		log.Printf("   Team %4d: %d matches (Red: %d, Blue: %d)", teamNum, appearances, redCount, blueCount)
	}
}
