package handlers

import (
	"context"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/frc10101/TealTeam/internal/models"
	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

func postFormContext(t *testing.T, form url.Values) *gin.Context {
	t.Helper()

	gin.SetMode(gin.TestMode)
	w := httptest.NewRecorder()
	c, _ := gin.CreateTestContext(w)
	req := httptest.NewRequest(http.MethodPost, "/", strings.NewReader(form.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	c.Request = req

	return c
}

func TestParseRequiredInt(t *testing.T) {
	t.Run("success", func(t *testing.T) {
		c := postFormContext(t, url.Values{"event_id": {"42"}})

		got, err := parseRequiredInt(c, "event_id")
		if err != nil {
			t.Fatalf("expected no error, got %v", err)
		}
		if got != 42 {
			t.Fatalf("expected 42, got %d", got)
		}
	})

	t.Run("missing value", func(t *testing.T) {
		c := postFormContext(t, url.Values{})

		_, err := parseRequiredInt(c, "event_id")
		if err == nil || err.Error() != "event_id is required" {
			t.Fatalf("expected required error, got %v", err)
		}
	})

	t.Run("non-numeric", func(t *testing.T) {
		c := postFormContext(t, url.Values{"event_id": {"abc"}})

		_, err := parseRequiredInt(c, "event_id")
		if err == nil || err.Error() != "event_id must be a number" {
			t.Fatalf("expected number error, got %v", err)
		}
	})
}

func TestExtractEventCode(t *testing.T) {
	tests := []struct {
		name   string
		input  string
		output string
	}{
		{name: "normal lowercase key", input: "2026mndu", output: "mndu"},
		{name: "already uppercase", input: "2026TXAUS", output: "txaus"},
		{name: "missing year prefix", input: "mndu", output: ""},
		{name: "year only", input: "2026", output: ""},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := extractEventCode(tt.input)
			if got != tt.output {
				t.Fatalf("expected %q, got %q", tt.output, got)
			}
		})
	}
}

func TestGetMatchStatus(t *testing.T) {
	now := time.Date(2026, time.March, 19, 12, 0, 0, 0, time.UTC)

	t.Run("played is completed", func(t *testing.T) {
		match := &MatchWithTeams{Played: true}
		if got := GetMatchStatus(match, now); got != MatchStatusCompleted {
			t.Fatalf("expected %q, got %q", MatchStatusCompleted, got)
		}
	})

	t.Run("nil scheduled time is upcoming", func(t *testing.T) {
		match := &MatchWithTeams{Played: false, ScheduledTime: nil}
		if got := GetMatchStatus(match, now); got != MatchStatusUpcoming {
			t.Fatalf("expected %q, got %q", MatchStatusUpcoming, got)
		}
	})

	t.Run("future outside window is upcoming", func(t *testing.T) {
		ts := now.Add(10 * time.Minute)
		match := &MatchWithTeams{ScheduledTime: &ts}
		if got := GetMatchStatus(match, now); got != MatchStatusUpcoming {
			t.Fatalf("expected %q, got %q", MatchStatusUpcoming, got)
		}
	})

	t.Run("within active window is in progress", func(t *testing.T) {
		ts := now.Add(4 * time.Minute)
		match := &MatchWithTeams{ScheduledTime: &ts}
		if got := GetMatchStatus(match, now); got != MatchStatusInProgress {
			t.Fatalf("expected %q, got %q", MatchStatusInProgress, got)
		}
	})

	t.Run("past end window is completed", func(t *testing.T) {
		ts := now.Add(-16 * time.Minute)
		match := &MatchWithTeams{ScheduledTime: &ts}
		if got := GetMatchStatus(match, now); got != MatchStatusCompleted {
			t.Fatalf("expected %q, got %q", MatchStatusCompleted, got)
		}
	})
}

func TestHandlerNoDBGuards(t *testing.T) {
	h := &Handler{}
	ctx := context.Background()

	if h.hasDB() {
		t.Fatalf("expected hasDB false when db is nil")
	}

	if _, err := h.GetMatchesForTeam(ctx, 1, 1); err == nil {
		t.Fatalf("expected error when db is nil")
	}

	if _, err := h.GetCurrentMatchForTeam(ctx, 1, 1); err == nil {
		t.Fatalf("expected error when db is nil")
	}

	if _, _, _, err := h.GetMatchesForTeamByStatus(ctx, 1, 1); err == nil {
		t.Fatalf("expected error when db is nil")
	}

	if _, err := h.GetEventsForTeam(ctx, 10101); err == nil {
		t.Fatalf("expected error when db is nil")
	}

	if _, err := h.GetAvailableEventsForUser(ctx, &models.User{}); err == nil {
		t.Fatalf("expected error when db is nil")
	}
}

func TestHttpErrorAndHasDB(t *testing.T) {
	err := httpError("boom")
	if err == nil || err.Error() != "boom" {
		t.Fatalf("expected static error message, got %v", err)
	}

	h := &Handler{db: &gorm.DB{}}
	if !h.hasDB() {
		t.Fatalf("expected hasDB true when db is set")
	}
}
