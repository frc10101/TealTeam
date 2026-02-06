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
	roundsPerTeam      = 5 // Each team plays this many matches
	minMatchSeparation = 3 // Minimum matches between a team's appearances
	defaultDBURL       = "postgres://user:password@localhost:5432/yourdb?sslmode=disable"
)

// Match represents a single FRC match with 3v3 alliances
type Match struct {
	Number    int
	MatchType string
	RedTeams  [3]int // Team IDs for red alliance (positions 1, 2, 3)
	BlueTeams [3]int // Team IDs for blue alliance (positions 1, 2, 3)
	RedScore  int
	BlueScore int
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

	// Create events
	log.Printf("📅 Creating %d events...", numCompetitions)
	competitionIDs, err := createCompetitions(db)
	if err != nil {
		return fmt.Errorf("failed to create events: %w", err)
	}

	// Create teams
	log.Printf("👥 Creating %d teams...", teamsPerComp)
	teamIDs, err := createTeams(db, teamsPerComp)
	if err != nil {
		return fmt.Errorf("failed to create teams: %w", err)
	}

	// For each event, generate FRC-style match schedule
	for i, compID := range competitionIDs {
		log.Printf("\n🏆 Event %d: Generating FRC match schedule...", i+1)

		// Associate teams with event
		if err := associateTeamsWithCompetition(db, compID, teamIDs); err != nil {
			return fmt.Errorf("failed to associate teams with event: %w", err)
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
	tables := []string{"zebra_data", "awards", "auto_paths", "team_event_stats", "scouting_data", "matches", "event_teams", "events", "teams"}
	for _, table := range tables {
		_, err := db.Exec(fmt.Sprintf("DELETE FROM %s", table))
		if err != nil {
			log.Printf("  Note: Could not clear %s (may not exist)", table)
		}
	}
	return nil
}

// Competition TBA keys and event types
var eventTypes = []string{"regional", "district", "championship"}
var districtKeys = []string{"2026fit", "2026fim", "2026fma", "2026ne", "2026pnw"}

func createCompetitions(db *sql.DB) ([]int, error) {
	var ids []int

	for i := 0; i < numCompetitions; i++ {
		name := competitionNames[i%len(competitionNames)]
		if i >= len(competitionNames) {
			name = fmt.Sprintf("Competition %d", i+1)
		}

		startDate := time.Now().AddDate(0, i, 0)
		endDate := startDate.AddDate(0, 0, 2)

		// Generate TBA-style key
		tbaKey := fmt.Sprintf("2026txho%d", i+1)
		eventType := eventTypes[i%len(eventTypes)]
		districtKey := districtKeys[i%len(districtKeys)]
		week := i + 1

		var id int
		err := db.QueryRow(`
			INSERT INTO events (name, location, start_date, end_date, tba_key, event_type, district_key, week)
			VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
			RETURNING id
		`, name, fmt.Sprintf("Venue %d", i+1), startDate, endDate, tbaKey, eventType, districtKey, week).Scan(&id)

		if err != nil {
			return nil, err
		}
		ids = append(ids, id)
		log.Printf("  Created competition: %s (ID: %d, TBA: %s)", name, id, tbaKey)
	}

	return ids, nil
}

// Team mottos for seed data
var teamMottos = []string{
	"Building the future, one robot at a time",
	"Innovation through collaboration",
	"Dream it. Build it. Win it.",
	"Engineering excellence",
	"Gracious professionalism in action",
	"Coopertition makes us stronger",
	"More than robots",
	"STEM education for all",
}

var countries = []string{"USA", "USA", "USA", "USA", "Canada", "Mexico", "Israel", "Turkey"}

func createTeams(db *sql.DB, count int) ([]int, error) {
	var ids []int

	for i := 0; i < count; i++ {
		// Generate realistic FRC team numbers (typically 1-9999)
		teamNumber := 100 + i*100 + rand.Intn(99)
		name := generateTeamName()
		nickname := name // Use same as name for nickname
		school := fmt.Sprintf("High School %d", i+1)
		schoolName := fmt.Sprintf("%s Technical Academy", name)
		city := fmt.Sprintf("City %d", i%10+1)
		state := []string{"CA", "TX", "NY", "FL", "IL", "PA", "OH", "MI", "GA", "NC"}[i%10]
		country := countries[i%len(countries)]
		tbaKey := fmt.Sprintf("frc%d", teamNumber)
		rookieYear := 2010 + rand.Intn(16) // 2010-2025
		motto := teamMottos[i%len(teamMottos)]
		website := fmt.Sprintf("https://team%d.org", teamNumber)

		var id int
		err := db.QueryRow(`
			INSERT INTO teams (team_number, name, school, city, state, tba_key, nickname, school_name, country, rookie_year, motto, website)
			VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
			RETURNING id
		`, teamNumber, name, school, city, state, tbaKey, nickname, schoolName, country, rookieYear, motto, website).Scan(&id)

		if err != nil {
			return nil, err
		}
		ids = append(ids, id)
	}

	log.Printf("  Created %d teams with TBA data", count)
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
			INSERT INTO event_teams (event_id, team_id)
			VALUES ($1, $2)
			ON CONFLICT (event_id, team_id) DO NOTHING
		`, compID, teamID)

		if err != nil {
			return err
		}
	}

	// Create team_event_stats for each team at this competition
	for _, teamID := range teamIDs {
		opr := 20.0 + rand.Float64()*40.0       // OPR: 20-60
		dpr := 5.0 + rand.Float64()*20.0        // DPR: 5-25
		ccwm := opr - dpr                       // CCWM = OPR - DPR
		autoOpr := 5.0 + rand.Float64()*15.0    // Auto OPR: 5-20
		teleopOpr := 10.0 + rand.Float64()*25.0 // Teleop OPR: 10-35
		endgameOpr := 2.0 + rand.Float64()*10.0 // Endgame OPR: 2-12

		_, err := db.Exec(`
			INSERT INTO team_event_stats (team_id, event_id, opr, dpr, ccwm, auto_opr, teleop_opr, endgame_opr,
				rank, matches_played, qual_average, wins, losses, ties, dq_count,
				qual_points, elim_points, award_points, alliance_points, total_points)
			VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
		`, teamID, compID, opr, dpr, ccwm, autoOpr, teleopOpr, endgameOpr,
			rand.Intn(30)+1,          // rank: 1-30
			roundsPerTeam,            // matches_played
			30.0+rand.Float64()*30.0, // qual_average: 30-60
			rand.Intn(5),             // wins
			rand.Intn(5),             // losses
			rand.Intn(2),             // ties
			rand.Intn(2),             // dq_count
			rand.Intn(20)+5,          // qual_points
			rand.Intn(30),            // elim_points
			rand.Intn(10),            // award_points
			rand.Intn(16),            // alliance_points
			rand.Intn(60)+20)         // total_points

		if err != nil {
			log.Printf("Warning: Could not insert team_event_stats: %v", err)
		}
	}

	log.Printf("   📊 Created team_event_stats for %d teams", len(teamIDs))
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

// Scouting data enums
var startingPositions = []string{"left", "center", "right"}
var towerLevels = []string{"none", "level1", "level2", "level3"}
var defenseRatings = []string{"low", "mid", "high"}
var throughputRatings = []string{"low", "mid", "high"}
var scoringStrategies = []string{"passer", "stealer", "scorer"}
var traversalTypes = []string{"trench", "bump"}

func insertMatches(db *sql.DB, compID int, matches []Match) error {
	for _, match := range matches {
		// Generate 2026 game-specific score breakdown
		redAutoTower := rand.Intn(15)
		redEndgameTower := rand.Intn(20)
		redHubAuto := rand.Intn(5)
		redHubTeleop := rand.Intn(20)
		redHubEndgame := rand.Intn(8)
		redEnergized := rand.Float32() < 0.4
		redSupercharged := rand.Float32() < 0.25
		redTraversal := rand.Float32() < 0.3
		redMinorFouls := rand.Intn(5)
		redMajorFouls := rand.Intn(2)
		redFoulPoints := redMinorFouls*2 + redMajorFouls*5
		redRp := 0
		if redEnergized {
			redRp++
		}
		if redSupercharged {
			redRp++
		}
		redAutoPoints := redAutoTower + redHubAuto*3
		redTeleopPoints := redHubTeleop*2 + redHubEndgame*4 + redEndgameTower

		blueAutoTower := rand.Intn(15)
		blueEndgameTower := rand.Intn(20)
		blueHubAuto := rand.Intn(5)
		blueHubTeleop := rand.Intn(20)
		blueHubEndgame := rand.Intn(8)
		blueEnergized := rand.Float32() < 0.4
		blueSupercharged := rand.Float32() < 0.25
		blueTraversal := rand.Float32() < 0.3
		blueMinorFouls := rand.Intn(5)
		blueMajorFouls := rand.Intn(2)
		blueFoulPoints := blueMinorFouls*2 + blueMajorFouls*5
		blueRp := 0
		if blueEnergized {
			blueRp++
		}
		if blueSupercharged {
			blueRp++
		}
		blueAutoPoints := blueAutoTower + blueHubAuto*3
		blueTeleopPoints := blueHubTeleop*2 + blueHubEndgame*4 + blueEndgameTower

		// Calculate total scores
		redTotal := redAutoPoints + redTeleopPoints + redFoulPoints
		blueTotal := blueAutoPoints + blueTeleopPoints + blueFoulPoints

		winningAlliance := ""
		if redTotal > blueTotal {
			winningAlliance = "red"
			redRp += 2
		} else if blueTotal > redTotal {
			winningAlliance = "blue"
			blueRp += 2
		} else {
			redRp++
			blueRp++
		}

		tbaKey := fmt.Sprintf("2026txho_qm%d", match.Number)
		scheduledTime := time.Now().Add(time.Duration(match.Number) * time.Hour)

		// Insert match with all new fields
		var matchID int
		err := db.QueryRow(`
			INSERT INTO matches (event_id, match_number, match_type, red_score, blue_score, played,
				tba_key, comp_level, set_number, scheduled_time, winning_alliance,
				red_auto_tower_points, red_endgame_tower_points, red_hub_auto_count, red_hub_auto_points,
				red_hub_teleop_count, red_hub_teleop_points, red_hub_endgame_count, red_hub_endgame_points,
				red_hub_total_count, red_hub_total_points, red_energized_achieved, red_supercharged_achieved,
				red_traversal_achieved, red_minor_foul_count, red_major_foul_count, red_foul_points,
				red_rp, red_total_auto_points, red_total_teleop_points,
				blue_auto_tower_points, blue_endgame_tower_points, blue_hub_auto_count, blue_hub_auto_points,
				blue_hub_teleop_count, blue_hub_teleop_points, blue_hub_endgame_count, blue_hub_endgame_points,
				blue_hub_total_count, blue_hub_total_points, blue_energized_achieved, blue_supercharged_achieved,
				blue_traversal_achieved, blue_minor_foul_count, blue_major_foul_count, blue_foul_points,
				blue_rp, blue_total_auto_points, blue_total_teleop_points)
			VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
				$12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30,
				$31, $32, $33, $34, $35, $36, $37, $38, $39, $40, $41, $42, $43, $44, $45, $46, $47, $48, $49)
			RETURNING id
		`, compID, match.Number, match.MatchType, redTotal, blueTotal, true,
			tbaKey, "qm", 1, scheduledTime, winningAlliance,
			redAutoTower, redEndgameTower, redHubAuto, redHubAuto*3,
			redHubTeleop, redHubTeleop*2, redHubEndgame, redHubEndgame*4,
			redHubAuto+redHubTeleop+redHubEndgame, redHubAuto*3+redHubTeleop*2+redHubEndgame*4,
			redEnergized, redSupercharged, redTraversal, redMinorFouls, redMajorFouls, redFoulPoints,
			redRp, redAutoPoints, redTeleopPoints,
			blueAutoTower, blueEndgameTower, blueHubAuto, blueHubAuto*3,
			blueHubTeleop, blueHubTeleop*2, blueHubEndgame, blueHubEndgame*4,
			blueHubAuto+blueHubTeleop+blueHubEndgame, blueHubAuto*3+blueHubTeleop*2+blueHubEndgame*4,
			blueEnergized, blueSupercharged, blueTraversal, blueMinorFouls, blueMajorFouls, blueFoulPoints,
			blueRp, blueAutoPoints, blueTeleopPoints).Scan(&matchID)

		if err != nil {
			return fmt.Errorf("failed to insert match %d: %w", match.Number, err)
		}

		// Insert red alliance teams with scouting data
		for pos, teamID := range match.RedTeams {
			if teamID == 0 {
				continue
			}
			if err := insertMatchTeam(db, matchID, teamID, "red", pos+1); err != nil {
				return fmt.Errorf("failed to insert red team: %w", err)
			}
		}

		// Insert blue alliance teams with scouting data
		for pos, teamID := range match.BlueTeams {
			if teamID == 0 {
				continue
			}
			if err := insertMatchTeam(db, matchID, teamID, "blue", pos+1); err != nil {
				return fmt.Errorf("failed to insert blue team: %w", err)
			}
		}
	}

	log.Printf("   ✅ Inserted %d matches with full 2026 score breakdowns", len(matches))
	return nil
}

// insertMatchTeam inserts a team's scouting data for a specific match
func insertMatchTeam(db *sql.DB, matchID, teamID int, allianceColor string, position int) error {
	// Generate random scouting data
	startingPos := startingPositions[rand.Intn(len(startingPositions))]
	autoTowerLevel := towerLevels[rand.Intn(len(towerLevels))]
	endgameTowerLevel := towerLevels[rand.Intn(len(towerLevels))]
	autoHand := rand.Intn(4)          // 0-3
	endgameHang := rand.Intn(4)       // 0-3
	scoringRating := rand.Intn(5) + 1 // 1-5 Likert
	defenseRating := defenseRatings[rand.Intn(len(defenseRatings))]
	throughput := throughputRatings[rand.Intn(len(throughputRatings))]
	scoringStrategy := scoringStrategies[rand.Intn(len(scoringStrategies))]
	traversal := traversalTypes[rand.Intn(len(traversalTypes))]

	// Hub scoring contribution
	hubAutoCount := rand.Intn(3)
	hubTeleopCount := rand.Intn(8)
	hubEndgameCount := rand.Intn(4)

	autoScore := rand.Intn(30)
	teleopScore := rand.Intn(50)
	endgameScore := rand.Intn(25)
	penaltiesCaused := rand.Intn(3)

	// Generate auto path data as JSON
	autoPathData := generateAutoPathJSON(startingPos)

	scouterName := fmt.Sprintf("Scouter %d", rand.Intn(10)+1)

	_, err := db.Exec(`
		INSERT INTO scouting_data (match_id, team_id, alliance_color, alliance_position,
			auto_score, teleop_score, endgame_score, scouter_name,
			starting_position, auto_path_data, auto_tower_level, auto_hand,
			scoring_rating, endgame_tower_level, endgame_hang,
			defense_rating, throughput, scoring_strategy, traversal,
			hub_auto_count, hub_teleop_count, hub_endgame_count, penalties_caused,
			scouted_at)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24)
	`, matchID, teamID, allianceColor, position,
		autoScore, teleopScore, endgameScore, scouterName,
		startingPos, autoPathData, autoTowerLevel, autoHand,
		scoringRating, endgameTowerLevel, endgameHang,
		defenseRating, throughput, scoringStrategy, traversal,
		hubAutoCount, hubTeleopCount, hubEndgameCount, penaltiesCaused,
		time.Now())

	return err
}

// generateAutoPathJSON creates a simple auto path as JSON
func generateAutoPathJSON(startingPos string) string {
	// Generate a path with 5-10 waypoints
	numPoints := 5 + rand.Intn(6)

	// Starting coordinates based on position
	var startX, startY float64
	switch startingPos {
	case "left":
		startX, startY = 1.5, 4.0
	case "center":
		startX, startY = 8.0, 4.0
	case "right":
		startX, startY = 14.5, 4.0
	}

	path := fmt.Sprintf(`{"points": [{"x": %.2f, "y": %.2f, "t": 0}`, startX, startY)

	currentX, currentY := startX, startY
	for i := 1; i < numPoints; i++ {
		// Move randomly but generally forward
		currentX += rand.Float64()*2.0 - 0.5
		currentY += rand.Float64()*1.5 - 0.75
		// Keep within field bounds (roughly 16m x 8m)
		if currentX < 0 {
			currentX = 0
		}
		if currentX > 16 {
			currentX = 16
		}
		if currentY < 0 {
			currentY = 0
		}
		if currentY > 8 {
			currentY = 8
		}
		path += fmt.Sprintf(`, {"x": %.2f, "y": %.2f, "t": %.1f}`, currentX, currentY, float64(i)*0.5)
	}

	path += `]}`
	return path
}

func printSummary(db *sql.DB) {
	log.Println("\n📊 Seed Summary:")

	var compCount, teamCount, matchCount, matchTeamCount int

	db.QueryRow("SELECT COUNT(*) FROM events").Scan(&compCount)
	db.QueryRow("SELECT COUNT(*) FROM teams").Scan(&teamCount)
	db.QueryRow("SELECT COUNT(*) FROM matches").Scan(&matchCount)
	db.QueryRow("SELECT COUNT(*) FROM scouting_data").Scan(&matchTeamCount)

	log.Printf("   Events: %d", compCount)
	log.Printf("   Teams: %d", teamCount)
	log.Printf("   Matches: %d", matchCount)
	log.Printf("   Match-Team assignments: %d", matchTeamCount)

	// Print example matches
	log.Println("\n📋 Example Match Schedule (first 5 matches of Event 1):")

	rows, err := db.Query(`
		SELECT m.match_number, m.red_score, m.blue_score
		FROM matches m
		WHERE m.event_id = 1
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
			FROM scouting_data sd
			JOIN teams t ON sd.team_id = t.id
			JOIN matches m ON sd.match_id = m.id
			WHERE m.match_number = $1 AND m.event_id = 1
			ORDER BY sd.alliance_color DESC, sd.alliance_position
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
	log.Println("\n📊 Team Appearance Stats (Event 1):")
	rows, err = db.Query(`
		SELECT t.team_number, 
		       COUNT(*) as appearances,
		       SUM(CASE WHEN sd.alliance_color = 'red' THEN 1 ELSE 0 END) as red_count,
		       SUM(CASE WHEN sd.alliance_color = 'blue' THEN 1 ELSE 0 END) as blue_count
		FROM scouting_data sd
		JOIN teams t ON sd.team_id = t.id
		JOIN matches m ON sd.match_id = m.id
		WHERE m.event_id = 1
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
