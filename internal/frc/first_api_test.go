package frc

import (
	"context"
	"encoding/base64"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"testing"
)

func TestNewClient(t *testing.T) {
	username := "testuser"
	key := "testkey"

	client := NewClient(username, key)

	if client.baseURL != defaultBaseURL {
		t.Errorf("expected baseURL %s, got %s", defaultBaseURL, client.baseURL)
	}

	expected := base64.StdEncoding.EncodeToString([]byte(username + ":" + key))
	if client.authHeader != "Basic "+expected {
		t.Errorf("auth header not correctly formatted")
	}

	if client.httpClient == nil {
		t.Error("http client should not be nil")
	}
}

func TestGetSeasonEvents(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			t.Errorf("expected GET, got %s", r.Method)
		}

		if r.URL.Path != "/2024/events" {
			t.Errorf("expected path /2024/events, got %s", r.URL.Path)
		}

		authHeader := r.Header.Get("Authorization")
		if authHeader == "" {
			t.Error("Authorization header missing")
		}

		if r.Header.Get("Accept") != "application/json" {
			t.Error("Accept header should be application/json")
		}

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		fmt.Fprint(w, `{
			"Events": [
				{
					"eventCode": "mide",
					"code": "mide",
					"name": "Michigan District Event",
					"type": "District",
					"city": "Detroit",
					"stateprov": "MI",
					"country": "USA",
					"dateStart": "2024-03-01",
					"dateEnd": "2024-03-03",
					"weekNumber": 1
				}
			]
		}`)
	}))
	defer server.Close()

	client := NewClient("user", "key")
	client.baseURL = server.URL

	events, err := client.GetSeasonEvents(context.Background(), 2024, url.Values{})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(events) != 1 {
		t.Errorf("expected 1 event, got %d", len(events))
	}

	event := events[0]
	if event.EventCode != "mide" {
		t.Errorf("expected event code mide, got %s", event.EventCode)
	}
	if event.Name != "Michigan District Event" {
		t.Errorf("expected name Michigan District Event, got %s", event.Name)
	}
	if event.City != "Detroit" {
		t.Errorf("expected city Detroit, got %s", event.City)
	}
}

func TestGetSeasonEventsWithFilters(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		query := r.URL.Query()
		if query.Get("eventCode") != "mide" {
			t.Errorf("expected eventCode query param, got %s", query.Get("eventCode"))
		}

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		fmt.Fprint(w, `{
			"Events": [
				{
					"eventCode": "mide",
					"code": "mide",
					"name": "Michigan District Event",
					"type": "District",
					"city": "Detroit",
					"stateprov": "MI",
					"country": "USA",
					"dateStart": "2024-03-01",
					"dateEnd": "2024-03-03",
					"weekNumber": 1
				}
			]
		}`)
	}))
	defer server.Close()

	client := NewClient("user", "key")
	client.baseURL = server.URL

	filters := url.Values{}
	filters.Set("eventCode", "mide")

	events, err := client.GetSeasonEvents(context.Background(), 2024, filters)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(events) != 1 {
		t.Errorf("expected 1 event, got %d", len(events))
	}
}

func TestGetSeasonEventsError(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusUnauthorized)
		fmt.Fprint(w, "Unauthorized")
	}))
	defer server.Close()

	client := NewClient("user", "wrongkey")
	client.baseURL = server.URL

	events, err := client.GetSeasonEvents(context.Background(), 2024, url.Values{})
	if err == nil {
		t.Error("expected error for unauthorized response")
	}
	if events != nil {
		t.Error("events should be nil when error occurs")
	}
}

func TestGetEventTeams(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/2024/teams" {
			t.Errorf("expected path /2024/teams, got %s", r.URL.Path)
		}

		query := r.URL.Query()
		if query.Get("eventCode") != "mide" {
			t.Errorf("expected eventCode filter, got %s", query.Get("eventCode"))
		}

		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		fmt.Fprint(w, `{
			"teams": [
				{
					"teamNumber": 7405,
					"nameFull": "Teal Team 7405",
					"nameShort": "Teal Team",
					"schoolName": "School Name",
					"city": "Detroit",
					"stateProv": "MI",
					"country": "USA",
					"rookieYear": 2019,
					"website": "https://tealteam7405.com"
				},
				{
					"teamNumber": 5881,
					"nameFull": "Resistance 5881",
					"nameShort": "Resistance",
					"schoolName": "Other School",
					"city": "Ann Arbor",
					"stateProv": "MI",
					"country": "USA",
					"rookieYear": 2015,
					"website": "https://example.com"
				}
			]
		}`)
	}))
	defer server.Close()

	client := NewClient("user", "key")
	client.baseURL = server.URL

	teams, err := client.GetEventTeams(context.Background(), 2024, "mide")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(teams) != 2 {
		t.Errorf("expected 2 teams, got %d", len(teams))
	}

	team := teams[0]
	if team.TeamNumber != 7405 {
		t.Errorf("expected team number 7405, got %d", team.TeamNumber)
	}
	if team.NameFull != "Teal Team 7405" {
		t.Errorf("expected name Teal Team 7405, got %s", team.NameFull)
	}
	if team.City != "Detroit" {
		t.Errorf("expected city Detroit, got %s", team.City)
	}

	team2 := teams[1]
	if team2.TeamNumber != 5881 {
		t.Errorf("expected team number 5881, got %d", team2.TeamNumber)
	}
	if team2.NameShort != "Resistance" {
		t.Errorf("expected short name Resistance, got %s", team2.NameShort)
	}
}

func TestGetEventTeamsInvalidJSON(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		fmt.Fprint(w, `{invalid json}`)
	}))
	defer server.Close()

	client := NewClient("user", "key")
	client.baseURL = server.URL

	teams, err := client.GetEventTeams(context.Background(), 2024, "mide")
	if err == nil {
		t.Error("expected error for invalid JSON")
	}
	if teams != nil {
		t.Error("teams should be nil when error occurs")
	}
}

// Integration Tests - These test against real FIRST API endpoints
// Requires FIRST_API_USERNAME and FIRST_API_KEY environment variables

func TestGetSeasonEventsIntegration(t *testing.T) {
	username := os.Getenv("FIRST_API_USERNAME")
	key := os.Getenv("FIRST_API_KEY")

	if username == "" || key == "" {
		t.Skip("skipping integration test: FIRST_API_USERNAME or FIRST_API_KEY not set")
	}

	client := NewClient(username, key)
	ctx := context.Background()

	// Test getting 2024 events
	events, err := client.GetSeasonEvents(ctx, 2024, url.Values{})
	if err != nil {
		t.Fatalf("GetSeasonEvents failed: %v", err)
	}

	if len(events) == 0 {
		t.Fatal("expected at least one event, got none")
	}

	// Verify event has expected fields
	event := events[0]
	if event.Code == "" {
		t.Error("event code should not be empty")
	}
	if event.Name == "" {
		t.Error("event name should not be empty")
	}

	t.Logf("✓ Retrieved %d events from FIRST API", len(events))
	t.Logf("  First event: %s (%s)", event.Name, event.Code)
}

func TestGetEventTeamsIntegration(t *testing.T) {
	username := os.Getenv("FIRST_API_USERNAME")
	key := os.Getenv("FIRST_API_KEY")

	if username == "" || key == "" {
		t.Skip("skipping integration test: FIRST_API_USERNAME or FIRST_API_KEY not set")
	}

	client := NewClient(username, key)
	ctx := context.Background()

	// First get a valid event code
	events, err := client.GetSeasonEvents(ctx, 2024, url.Values{})
	if err != nil {
		t.Fatalf("GetSeasonEvents failed: %v", err)
	}

	if len(events) == 0 {
		t.Fatal("no events available to test teams")
	}

	// Use the first available event
	eventCode := events[0].Code
	teams, err := client.GetEventTeams(ctx, 2024, eventCode)

	if err != nil {
		t.Fatalf("GetEventTeams failed for event %s: %v", eventCode, err)
	}

	if len(teams) == 0 {
		t.Logf("⊘ No teams found at event %s (this is valid)", eventCode)
		return
	}

	// Verify team has expected fields
	team := teams[0]
	if team.TeamNumber == 0 {
		t.Error("team number should not be 0")
	}
	if team.NameFull == "" && team.NameShort == "" {
		t.Error("team should have a name")
	}

	t.Logf("✓ Retrieved %d teams from event %s", len(teams), eventCode)
	t.Logf("  First team: %s (Team %d)", team.NameFull, team.TeamNumber)
}

func TestAuthenticationIntegration(t *testing.T) {
	username := os.Getenv("FIRST_API_USERNAME")
	key := os.Getenv("FIRST_API_KEY")

	if username == "" || key == "" {
		t.Skip("skipping integration test: FIRST_API_USERNAME or FIRST_API_KEY not set")
	}

	// Test with valid credentials
	client := NewClient(username, key)
	ctx := context.Background()

	_, err := client.GetSeasonEvents(ctx, 2024, url.Values{})
	if err != nil {
		t.Fatalf("Authentication failed with valid credentials: %v", err)
	}

	// Test with invalid credentials
	badClient := NewClient("baduser", "badkey")
	_, err = badClient.GetSeasonEvents(ctx, 2024, url.Values{})
	if err == nil {
		t.Error("expected authentication to fail with invalid credentials")
	}

	t.Log("✓ Authentication test passed")
}

func TestGetEventTeamsNotFound(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusNotFound)
		fmt.Fprint(w, "Event not found")
	}))
	defer server.Close()

	client := NewClient("user", "key")
	client.baseURL = server.URL

	teams, err := client.GetEventTeams(context.Background(), 2024, "invalid")
	if err == nil {
		t.Error("expected error for 404 response")
	}
	if teams != nil {
		t.Error("teams should be nil when error occurs")
	}
}
