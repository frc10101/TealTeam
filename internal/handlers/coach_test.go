package handlers

import (
	"testing"
	"time"
)

func TestMatchStatusDetection(t *testing.T) {
	now := time.Now()

	tests := []struct {
		name           string
		matchTime      time.Time
		expectedStatus string
		expectedLabel  string
	}{
		{
			name:           "Match from last week",
			matchTime:      now.Add(-7 * 24 * time.Hour),
			expectedStatus: "completed",
			expectedLabel:  "Completed",
		},
		{
			name:           "Match from yesterday",
			matchTime:      now.Add(-24 * time.Hour),
			expectedStatus: "completed",
			expectedLabel:  "Completed",
		},
		{
			name:           "Match 30 minutes ago",
			matchTime:      now.Add(-30 * time.Minute),
			expectedStatus: "completed",
			expectedLabel:  "Completed",
		},
		{
			name:           "Match 20 minutes ago",
			matchTime:      now.Add(-20 * time.Minute),
			expectedStatus: "completed",
			expectedLabel:  "Completed",
		},
		{
			name:           "Match 16 minutes ago",
			matchTime:      now.Add(-16 * time.Minute),
			expectedStatus: "completed",
			expectedLabel:  "Completed",
		},
		{
			name:           "Match 10 minutes ago (current)",
			matchTime:      now.Add(-10 * time.Minute),
			expectedStatus: "current",
			expectedLabel:  "Current Match",
		},
		{
			name:           "Match 5 minutes ago (current)",
			matchTime:      now.Add(-5 * time.Minute),
			expectedStatus: "current",
			expectedLabel:  "Current Match",
		},
		{
			name:           "Match right now (current)",
			matchTime:      now,
			expectedStatus: "current",
			expectedLabel:  "Current Match",
		},
		{
			name:           "Match in 5 minutes (current)",
			matchTime:      now.Add(5 * time.Minute),
			expectedStatus: "current",
			expectedLabel:  "Current Match",
		},
		{
			name:           "Match in 10 minutes (current)",
			matchTime:      now.Add(10 * time.Minute),
			expectedStatus: "current",
			expectedLabel:  "Current Match",
		},
		{
			name:           "Match in 15 minutes (current)",
			matchTime:      now.Add(15 * time.Minute),
			expectedStatus: "current",
			expectedLabel:  "Current Match",
		},
		{
			name:           "Match in 20 minutes (upcoming)",
			matchTime:      now.Add(20 * time.Minute),
			expectedStatus: "upcoming",
			expectedLabel:  "Upcoming",
		},
		{
			name:           "Match in 1 hour (upcoming)",
			matchTime:      now.Add(1 * time.Hour),
			expectedStatus: "upcoming",
			expectedLabel:  "Upcoming",
		},
		{
			name:           "Match tomorrow (upcoming)",
			matchTime:      now.Add(24 * time.Hour),
			expectedStatus: "upcoming",
			expectedLabel:  "Upcoming",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var status, statusLabel, statusClass string
			minutesUntilStart := int(tt.matchTime.Sub(now).Minutes())

			// This is the exact logic from coach.go
			if minutesUntilStart < -15 {
				status = "completed"
				statusLabel = "Completed"
				statusClass = "border-gray-700 bg-gray-900/40"
			} else if minutesUntilStart >= -15 && minutesUntilStart <= 15 {
				status = "current"
				statusLabel = "Current Match"
				statusClass = "border-yellow-500 bg-yellow-900/20 ring-2 ring-yellow-500/40"
			} else {
				status = "upcoming"
				statusLabel = "Upcoming"
				statusClass = "border-teal-600 bg-teal-900/20"
			}

			if status != tt.expectedStatus {
				t.Errorf("For %s: expected status %q, got %q (minutesUntilStart=%d)",
					tt.name, tt.expectedStatus, status, minutesUntilStart)
			}
			if statusLabel != tt.expectedLabel {
				t.Errorf("For %s: expected label %q, got %q",
					tt.name, tt.expectedLabel, statusLabel)
			}
			if status == "completed" && statusClass != "border-gray-700 bg-gray-900/40" {
				t.Errorf("For %s: completed match has wrong class: %q", tt.name, statusClass)
			}
		})
	}
}
