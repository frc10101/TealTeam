package frc

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"
)

func TestNewTBAClient(t *testing.T) {
	authKey := "test-auth-key"

	client := NewTBAClient(authKey)

	if client.baseURL != tbaBaseURL {
		t.Errorf("expected baseURL %s, got %s", tbaBaseURL, client.baseURL)
	}

	if client.authKey != authKey {
		t.Errorf("expected authKey %s, got %s", authKey, client.authKey)
	}

	if client.httpClient == nil {
		t.Error("http client should not be nil")
	}
}

func TestGetEventOPRs(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			t.Errorf("expected GET, got %s", r.Method)
		}

		if r.URL.Path != "/api/v3/event/2024mide/oprs" {
			t.Errorf("expected path /api/v3/event/2024mide/oprs, got %s", r.URL.Path)
		}

		authHeader := r.Header.Get("X-TBA-Auth-Key")
		if authHeader != "test-key" {
			t.Errorf("expected auth key test-key, got %s", authHeader)
		}

		if r.Header.Get("Accept") != "application/json" {
			t.Error("Accept header should be application/json")
		}

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		fmt.Fprint(w, `{
			"oprs": {
				"frc7405": 45.5,
				"frc5881": 52.3,
				"frc3476": 48.1
			},
			"dprs": {
				"frc7405": 12.3,
				"frc5881": 10.1,
				"frc3476": 14.2
			},
			"ccwms": {
				"frc7405": 2.1,
				"frc5881": 3.2,
				"frc3476": 1.8
			}
		}`)
	}))
	defer server.Close()

	client := NewTBAClient("test-key")
	client.baseURL = server.URL + "/api/v3"

	data, err := client.GetEventOPRs(context.Background(), "2024mide")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(data.OPRs) != 3 {
		t.Errorf("expected 3 OPR entries, got %d", len(data.OPRs))
	}

	if opr, ok := data.OPRs["frc7405"]; !ok || opr != 45.5 {
		t.Errorf("expected OPR for frc7405 to be 45.5, got %v", opr)
	}

	if len(data.DPRs) != 3 {
		t.Errorf("expected 3 DPR entries, got %d", len(data.DPRs))
	}

	if dpr, ok := data.DPRs["frc5881"]; !ok || dpr != 10.1 {
		t.Errorf("expected DPR for frc5881 to be 10.1, got %v", dpr)
	}

	if len(data.CCWMs) != 3 {
		t.Errorf("expected 3 CCWM entries, got %d", len(data.CCWMs))
	}
}

func TestGetEventOPRsError(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusNotFound)
		fmt.Fprint(w, "Event not found")
	}))
	defer server.Close()

	client := NewTBAClient("test-key")
	client.baseURL = server.URL + "/api/v3"

	data, err := client.GetEventOPRs(context.Background(), "invalid-event")
	if err == nil {
		t.Error("expected error for not found response")
	}
	if data != nil {
		t.Error("data should be nil when error occurs")
	}
}

func TestGetEventComponentOPRs(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/v3/event/2024mide/coprs" {
			t.Errorf("expected path /api/v3/event/2024mide/coprs, got %s", r.URL.Path)
		}

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		fmt.Fprint(w, `{
			"totalAutoPoints": {
				"frc7405": 15.5,
				"frc5881": 18.3
			},
			"totalTeleopPoints": {
				"frc7405": 22.1,
				"frc5881": 25.4
			},
			"endGameTowerPoints": {
				"frc7405": 8.5,
				"frc5881": 9.1
			}
		}`)
	}))
	defer server.Close()

	client := NewTBAClient("test-key")
	client.baseURL = server.URL + "/api/v3"

	data, err := client.GetEventComponentOPRs(context.Background(), "2024mide")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(data.Components) != 3 {
		t.Errorf("expected 3 component maps, got %d", len(data.Components))
	}

	auto, teleop, endgame := data.TeamPhaseOPRs("frc7405")
	if auto == nil || *auto != 15.5 {
		t.Errorf("expected auto value 15.5, got %v", auto)
	}
	if teleop == nil || *teleop != 22.1 {
		t.Errorf("expected teleop value 22.1, got %v", teleop)
	}
	if endgame == nil || *endgame != 8.5 {
		t.Errorf("expected endgame value 8.5, got %v", endgame)
	}
}

func TestGetEventRankings(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/v3/event/2024mide/rankings" {
			t.Errorf("expected path /api/v3/event/2024mide/rankings, got %s", r.URL.Path)
		}

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		fmt.Fprint(w, `{
			"rankings": [
				{
					"team_key": "frc7405",
					"rank": 1,
					"matches_played": 10,
					"qual_average": null,
					"sort_orders": [95.5, 171.2],
					"extra_stats": [180],
					"record": {
						"wins": 9,
						"losses": 1,
						"ties": 0
					}
				},
				{
					"team_key": "frc5881",
					"rank": 2,
					"matches_played": 10,
					"qual_average": 88.2,
					"sort_orders": [88.2, 154.5],
					"extra_stats": [165],
					"record": {
						"wins": 8,
						"losses": 2,
						"ties": 0
					}
				}
			]
		}`)
	}))
	defer server.Close()

	client := NewTBAClient("test-key")
	client.baseURL = server.URL + "/api/v3"

	rankings, err := client.GetEventRankings(context.Background(), "2024mide")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(rankings) != 2 {
		t.Errorf("expected 2 rankings, got %d", len(rankings))
	}

	ranking1 := rankings[0]
	if ranking1.TeamKey != "frc7405" {
		t.Errorf("expected team key frc7405, got %s", ranking1.TeamKey)
	}
	if ranking1.Rank != 1 {
		t.Errorf("expected rank 1, got %d", ranking1.Rank)
	}
	if ranking1.Record.Wins != 9 {
		t.Errorf("expected 9 wins, got %d", ranking1.Record.Wins)
	}
	if ranking1.QualAverage != nil {
		t.Errorf("expected nil qual average, got %v", ranking1.QualAverage)
	}
	if v := ranking1.EffectiveQualAverage(); v == nil || *v != 95.5 {
		t.Errorf("expected effective qual average 95.5, got %v", v)
	}
	if pts := ranking1.EffectiveTotalPoints(); pts == nil || *pts != 180 {
		t.Errorf("expected effective total points 180, got %v", pts)
	}

	ranking2 := rankings[1]
	if ranking2.TeamKey != "frc5881" {
		t.Errorf("expected team key frc5881, got %s", ranking2.TeamKey)
	}
	if ranking2.Rank != 2 {
		t.Errorf("expected rank 2, got %d", ranking2.Rank)
	}
}

func TestGetEvent(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/v3/event/2024mide" {
			t.Errorf("expected path /api/v3/event/2024mide, got %s", r.URL.Path)
		}

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		fmt.Fprint(w, `{
			"key": "2024mide",
			"name": "Michigan District Event",
			"event_code": "mide",
			"year": 2024,
			"start_date": "2024-03-01",
			"end_date": "2024-03-03",
			"timezone": "America/Detroit",
			"official": true
		}`)
	}))
	defer server.Close()

	client := NewTBAClient("test-key")
	client.baseURL = server.URL + "/api/v3"

	event, err := client.GetEvent(context.Background(), "2024mide")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if event.Key != "2024mide" {
		t.Errorf("expected key 2024mide, got %s", event.Key)
	}
	if event.Name != "Michigan District Event" {
		t.Errorf("expected name Michigan District Event, got %s", event.Name)
	}
	if event.Year != 2024 {
		t.Errorf("expected year 2024, got %d", event.Year)
	}
	if !event.Official {
		t.Error("expected event to be official")
	}
}

func TestGetEventMatches(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/v3/event/2024mide/matches" {
			t.Errorf("expected path /api/v3/event/2024mide/matches, got %s", r.URL.Path)
		}

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		fmt.Fprint(w, `[
			{
				"key": "2024mide_qm1",
				"event_key": "2024mide",
				"comp_level": "qm",
				"set_number": 1,
				"match_number": 1,
				"alliances": {
					"red": {
						"team_keys": ["frc7405", "frc5881", "frc3476"],
						"score": 120
					},
					"blue": {
						"team_keys": ["frc1690", "frc2410", "frc3476"],
						"score": 115
					}
				},
				"scheduled_time": 1709251800000,
				"predicted_time": 1709251800000,
				"actual_time": 1709251800000
			},
			{
				"key": "2024mide_qm2",
				"event_key": "2024mide",
				"comp_level": "qm",
				"set_number": 1,
				"match_number": 2,
				"alliances": {
					"red": {
						"team_keys": ["frc1690", "frc2410", "frc7405"],
						"score": 125
					},
					"blue": {
						"team_keys": ["frc5881", "frc3476", "frc1234"],
						"score": 110
					}
				},
				"scheduled_time": 1709252400000,
				"predicted_time": 1709252400000,
				"actual_time": 1709252400000
			}
		]`)
	}))
	defer server.Close()

	client := NewTBAClient("test-key")
	client.baseURL = server.URL + "/api/v3"

	matches, err := client.GetEventMatches(context.Background(), "2024mide")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(matches) != 2 {
		t.Errorf("expected 2 matches, got %d", len(matches))
	}

	match1 := matches[0]
	if match1.Key != "2024mide_qm1" {
		t.Errorf("expected key 2024mide_qm1, got %s", match1.Key)
	}
	if match1.CompLevel != "qm" {
		t.Errorf("expected comp level qm, got %s", match1.CompLevel)
	}
	if match1.MatchNumber != 1 {
		t.Errorf("expected match number 1, got %d", match1.MatchNumber)
	}
	if len(match1.Alliances.Red.Teams) != 3 {
		t.Errorf("expected 3 red teams, got %d", len(match1.Alliances.Red.Teams))
	}
	if match1.Alliances.Red.Score != 120 {
		t.Errorf("expected red score 120, got %d", match1.Alliances.Red.Score)
	}
	if match1.Alliances.Blue.Score != 115 {
		t.Errorf("expected blue score 115, got %d", match1.Alliances.Blue.Score)
	}

	match2 := matches[1]
	if match2.Key != "2024mide_qm2" {
		t.Errorf("expected key 2024mide_qm2, got %s", match2.Key)
	}
	if match2.MatchNumber != 2 {
		t.Errorf("expected match number 2, got %d", match2.MatchNumber)
	}
}

func TestGetEventMatchesError(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusInternalServerError)
		fmt.Fprint(w, "Internal server error")
	}))
	defer server.Close()

	client := NewTBAClient("test-key")
	client.baseURL = server.URL + "/api/v3"

	matches, err := client.GetEventMatches(context.Background(), "2024mide")
	if err == nil {
		t.Error("expected error for 500 response")
	}
	if matches != nil {
		t.Error("matches should be nil when error occurs")
	}
}

func TestGetEventRankingsInvalidJSON(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		fmt.Fprint(w, `{invalid json`)
	}))
	defer server.Close()

	client := NewTBAClient("test-key")
	client.baseURL = server.URL + "/api/v3"

	rankings, err := client.GetEventRankings(context.Background(), "2024mide")
	if err == nil {
		t.Error("expected error for invalid JSON")
	}
	if rankings != nil {
		t.Error("rankings should be nil when error occurs")
	}
}

// Integration Tests - These test against real The Blue Alliance API endpoints
// Requires TBA_AUTH_KEY environment variable

func skipIfNoInternetTBA(t *testing.T, err error) {
	t.Helper()
	if IsInternetUnavailable(err) {
		t.Skipf("skipping integration test: internet unavailable (%v)", err)
	}
}

func TestGetEventOPRsIntegration(t *testing.T) {
	authKey := os.Getenv("TBA_AUTH_KEY")
	if authKey == "" {
		t.Skip("skipping integration test: TBA_AUTH_KEY not set")
	}

	client := NewTBAClient(authKey)
	ctx := context.Background()

	// Try a 2026 event or recent 2025 events
	eventKeys := []string{"2026alhu", "2026micha", "2025micity", "2025michdist"}

	var found bool
	for _, eventKey := range eventKeys {
		data, err := client.GetEventOPRs(ctx, eventKey)

		if err != nil {
			t.Logf("Event %s not available: %v", eventKey, err)
			continue
		}

		if len(data.OPRs) == 0 {
			t.Logf("Event %s has no OPR data", eventKey)
			continue
		}

		t.Logf("✓ Retrieved OPR data for %s", eventKey)
		t.Logf("  Teams in event: %d", len(data.OPRs))
		found = true
		break
	}

	if !found {
		t.Logf("⊘ Could not find event with OPR data (season may not have started)")
	}
}

func TestGetEventRankingsIntegration(t *testing.T) {
	authKey := os.Getenv("TBA_AUTH_KEY")
	if authKey == "" {
		t.Skip("skipping integration test: TBA_AUTH_KEY not set")
	}

	client := NewTBAClient(authKey)
	ctx := context.Background()

	// Try a 2026 event or recent 2025 events
	eventKeys := []string{"2026alhu", "2026micha", "2025micity", "2025michdist"}

	var found bool
	for _, eventKey := range eventKeys {
		rankings, err := client.GetEventRankings(ctx, eventKey)

		if err != nil {
			t.Logf("Event %s not available: %v", eventKey, err)
			continue
		}

		if len(rankings) == 0 {
			t.Logf("Event %s has no rankings", eventKey)
			continue
		}

		ranking := rankings[0]
		if ranking.TeamKey == "" {
			t.Logf("Event %s has invalid ranking data", eventKey)
			continue
		}

		t.Logf("✓ Retrieved rankings for %s", eventKey)
		t.Logf("  Total teams ranked: %d", len(rankings))
		t.Logf("  Top team: %s (Rank %d)", ranking.TeamKey, ranking.Rank)
		found = true
		break
	}

	if !found {
		t.Logf("⊘ Could not find event with ranking data (season may not have started)")
	}
}

func TestGetEventMatchesIntegration(t *testing.T) {
	authKey := os.Getenv("TBA_AUTH_KEY")
	if authKey == "" {
		t.Skip("skipping integration test: TBA_AUTH_KEY not set")
	}

	client := NewTBAClient(authKey)
	ctx := context.Background()

	// Try a 2026 event or recent 2025 events
	eventKeys := []string{"2026alhu", "2026micha", "2025micity", "2025michdist"}

	var found bool
	for _, eventKey := range eventKeys {
		matches, err := client.GetEventMatches(ctx, eventKey)

		if err != nil {
			t.Logf("Event %s not available: %v", eventKey, err)
			continue
		}

		if len(matches) == 0 {
			t.Logf("Event %s has no matches", eventKey)
			continue
		}

		match := matches[0]
		if match.Key == "" {
			t.Logf("Event %s has invalid match data", eventKey)
			continue
		}

		t.Logf("✓ Retrieved matches for %s", eventKey)
		t.Logf("  Total matches: %d", len(matches))
		t.Logf("  First match: %s", match.Key)
		found = true
		break
	}

	if !found {
		t.Logf("⊘ Could not find event with match data (season may not have started)")
	}
}

func TestGetEventIntegration(t *testing.T) {
	authKey := os.Getenv("TBA_AUTH_KEY")
	if authKey == "" {
		t.Skip("skipping integration test: TBA_AUTH_KEY not set")
	}

	client := NewTBAClient(authKey)
	ctx := context.Background()

	// Try a 2026 event or recent 2025 events
	eventKeys := []string{"2026alhu", "2026micha", "2025micity", "2025michdist"}

	var found bool
	for _, eventKey := range eventKeys {
		event, err := client.GetEvent(ctx, eventKey)

		if err != nil {
			t.Logf("Event %s not available: %v", eventKey, err)
			continue
		}

		if event == nil {
			t.Logf("Event %s returned nil", eventKey)
			continue
		}

		if event.Key == "" {
			t.Logf("Event %s has invalid key", eventKey)
			continue
		}

		t.Logf("✓ Retrieved event data for %s", eventKey)
		t.Logf("  Event: %s (%d)", event.Name, event.Year)
		found = true
		break
	}

	if !found {
		t.Logf("⊘ Could not find valid event (season may not have started)")
	}
}

func TestTBAAuthenticationIntegration(t *testing.T) {
	authKey := os.Getenv("TBA_AUTH_KEY")
	if authKey == "" {
		t.Skip("skipping integration test: TBA_AUTH_KEY not set")
	}

	// Test with valid credentials - verify we can make a successful API call
	client := NewTBAClient(authKey)
	ctx := context.Background()

	// Try to access an event that exists
	event, err := client.GetEvent(ctx, "2026alhu")
	if err != nil {
		skipIfNoInternetTBA(t, err)
		t.Fatalf("Authentication failed with valid TBA_AUTH_KEY: %v", err)
	}

	if event == nil {
		t.Fatal("Expected event data but got nil with valid TBA_AUTH_KEY")
	}

	if event.Key == "" {
		t.Fatal("Event data is present but key is empty")
	}

	t.Logf("✓ TBA authentication successful")
	t.Logf("  Authenticated user retrieved: %s", event.Key)
}
