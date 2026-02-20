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
	endpoint := c.baseURL + path
	if len(query) > 0 {
		endpoint += "?" + query.Encode()
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint, nil)
	if err != nil {
		return err
	}

	req.Header.Set("Authorization", c.authHeader)
	req.Header.Set("Accept", "application/json")

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 4096))
		return fmt.Errorf("first api %s returned %d: %s", path, resp.StatusCode, string(body))
	}

	dec := json.NewDecoder(resp.Body)
	return dec.Decode(out)
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
