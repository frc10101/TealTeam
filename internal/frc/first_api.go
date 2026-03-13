package frc

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"time"
)

const defaultBaseURL = "https://frc-api.firstinspires.org/v3.0"

// Client wraps access to the FIRST Events API.
type Client struct {
	baseURL    string
	authHeader string
	httpClient *http.Client
}

// NewClient builds a FIRST Events API client using Basic auth.
func NewClient(username, key string) *Client {
	creds := fmt.Sprintf("%s:%s", username, key)
	encoded := base64.StdEncoding.EncodeToString([]byte(creds))

	return &Client{
		baseURL:    defaultBaseURL,
		authHeader: "Basic " + encoded,
		httpClient: &http.Client{Timeout: 20 * time.Second},
	}
}

func (c *Client) getJSON(ctx context.Context, path string, query url.Values, out any) error {
	if !shouldSkipConnectivityCheck(c.baseURL) {
		if err := ensureInternetConnectivity(ctx); err != nil {
			recordAPIError(err)
			return fmt.Errorf("first api preflight failed: %w", err)
		}
	}

	endpoint := c.baseURL + path
	if len(query) > 0 {
		endpoint += "?" + query.Encode()
	}

	for attempt := 0; attempt < apiRetryMaxAttempts; attempt++ {
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint, nil)
		if err != nil {
			recordAPIError(err)
			return err
		}

		req.Header.Set("Authorization", c.authHeader)
		req.Header.Set("Accept", "application/json")

		resp, err := c.httpClient.Do(req)
		if err != nil {
			recordAPIError(err)
			if attempt < apiRetryMaxAttempts-1 && shouldRetryRequestError(err) {
				if sleepErr := sleepWithContext(ctx, backoffDelay(attempt)); sleepErr != nil {
					recordAPIError(sleepErr)
					return sleepErr
				}
				continue
			}
			return err
		}

		func() {
			defer resp.Body.Close()
			if resp.StatusCode < 200 || resp.StatusCode >= 300 {
				body, _ := io.ReadAll(io.LimitReader(resp.Body, 4096))
				err = fmt.Errorf("first api %s returned %d: %s", path, resp.StatusCode, string(body))
				return
			}

			dec := json.NewDecoder(resp.Body)
			err = dec.Decode(out)
		}()

		if err == nil {
			recordAPISuccess()
			return nil
		}

		recordAPIError(err)
		if attempt < apiRetryMaxAttempts-1 && resp != nil && shouldRetryStatusCode(resp.StatusCode) {
			if sleepErr := sleepWithContext(ctx, backoffDelay(attempt)); sleepErr != nil {
				recordAPIError(sleepErr)
				return sleepErr
			}
			continue
		}

		return err
	}

	finalErr := fmt.Errorf("first api %s retries exhausted", path)
	recordAPIError(finalErr)
	return finalErr
}

// EventsResponse matches the /{season}/events response.
type EventsResponse struct {
	Events []Event `json:"Events"`
}

// Event represents a FIRST event summary.
type Event struct {
	EventCode    string `json:"eventCode"`
	Code         string `json:"code"`
	Name         string `json:"name"`
	Type         string `json:"type"`
	DistrictCode string `json:"districtCode"`
	Venue        string `json:"venue"`
	City         string `json:"city"`
	StateProv    string `json:"stateprov"`
	Country      string `json:"country"`
	DateStart    string `json:"dateStart"`
	DateEnd      string `json:"dateEnd"`
	WeekNumber   int    `json:"weekNumber"`
}

// GetSeasonEvents lists events for a season, with optional filters.
func (c *Client) GetSeasonEvents(ctx context.Context, season int, filters url.Values) ([]Event, error) {
	var resp EventsResponse
	path := fmt.Sprintf("/%d/events", season)
	if err := c.getJSON(ctx, path, filters, &resp); err != nil {
		return nil, err
	}
	return resp.Events, nil
}

// TeamsResponse matches the /{season}/teams response.
type TeamsResponse struct {
	Teams []Team `json:"teams"`
}

// Team represents a FIRST team summary.
type Team struct {
	TeamNumber int    `json:"teamNumber"`
	NameFull   string `json:"nameFull"`
	NameShort  string `json:"nameShort"`
	SchoolName string `json:"schoolName"`
	City       string `json:"city"`
	StateProv  string `json:"stateProv"`
	Country    string `json:"country"`
	RookieYear int    `json:"rookieYear"`
	Website    string `json:"website"`
}

// GetEventTeams lists teams attending an event.
func (c *Client) GetEventTeams(ctx context.Context, season int, eventCode string) ([]Team, error) {
	var resp TeamsResponse
	path := fmt.Sprintf("/%d/teams", season)
	q := url.Values{}
	q.Set("eventCode", eventCode)
	if err := c.getJSON(ctx, path, q, &resp); err != nil {
		return nil, err
	}
	return resp.Teams, nil
}

// ScheduleResponse matches the /{season}/schedule/{eventCode} response.
type ScheduleResponse struct {
	Schedule []ScheduleMatch `json:"Schedule"`
}

// ScheduleMatch represents a match in the schedule.
type ScheduleMatch struct {
	Description     string              `json:"description"`
	TournamentLevel string              `json:"tournamentLevel"`
	MatchNumber     int                 `json:"matchNumber"`
	StartTime       string              `json:"startTime"` // ISO 8601 format
	Field           string              `json:"field"`
	Teams           []ScheduleMatchTeam `json:"teams"`
}

// ScheduleMatchTeam represents a team in a match.
type ScheduleMatchTeam struct {
	TeamNumber int    `json:"teamNumber"`
	Station    string `json:"station"` // e.g., "Red1", "Blue2"
	Surrogate  bool   `json:"surrogate"`
	DQ         bool   `json:"dq"`
}

// GetMatchSchedule retrieves the match schedule for an event.
func (c *Client) GetMatchSchedule(ctx context.Context, season int, eventCode string, filters url.Values) ([]ScheduleMatch, error) {
	var resp ScheduleResponse
	path := fmt.Sprintf("/%d/schedule/%s", season, eventCode)
	if err := c.getJSON(ctx, path, filters, &resp); err != nil {
		return nil, err
	}
	return resp.Schedule, nil
}
