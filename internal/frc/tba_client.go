package frc

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"time"
)

const tbaBaseURL = "https://www.thebluealliance.com/api/v3"

// TBAClient wraps access to The Blue Alliance API.
type TBAClient struct {
	baseURL    string
	authKey    string
	httpClient *http.Client
}

// NewTBAClient creates a new TBA API client with the provided auth key.
func NewTBAClient(authKey string) *TBAClient {
	return &TBAClient{
		baseURL:    tbaBaseURL,
		authKey:    authKey,
		httpClient: &http.Client{Timeout: 20 * time.Second},
	}
}

func (c *TBAClient) getJSON(ctx context.Context, path string, query url.Values, out any) error {
	endpoint := c.baseURL + path
	if len(query) > 0 {
		endpoint += "?" + query.Encode()
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint, nil)
	if err != nil {
		return err
	}

	req.Header.Set("X-TBA-Auth-Key", c.authKey)
	req.Header.Set("Accept", "application/json")

	resp, err := c.httpClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 4096))
		return fmt.Errorf("tba api %s returned %d: %s", path, resp.StatusCode, string(body))
	}

	dec := json.NewDecoder(resp.Body)
	return dec.Decode(out)
}

// OPRData contains OPR, DPR, and CCWM statistics for all teams at an event.
type OPRData struct {
	OPRs map[string]float64 `json:"oprs"`
	DPRs map[string]float64 `json:"dprs"`
	CCWMs map[string]float64 `json:"ccwms"`
}

// GetEventOPRs fetches OPR, DPR, and CCWM data for an event.
func (c *TBAClient) GetEventOPRs(ctx context.Context, eventKey string) (*OPRData, error) {
	var data OPRData
	path := fmt.Sprintf("/event/%s/oprs", eventKey)
	if err := c.getJSON(ctx, path, nil, &data); err != nil {
		return nil, err
	}
	return &data, nil
}

// ComponentOPRData contains component OPR breakdown per team.
type ComponentOPRData struct {
	AutoOPRs   map[string]float64 `json:"auto_opr"`
	TeleopOPRs map[string]float64 `json:"teleop_opr"`
	EndgameOPRs map[string]float64 `json:"endgame_opr"`
}

// GetEventComponentOPRs fetches component OPR breakdown for an event.
func (c *TBAClient) GetEventComponentOPRs(ctx context.Context, eventKey string) (*ComponentOPRData, error) {
	var data ComponentOPRData
	path := fmt.Sprintf("/event/%s/coprs", eventKey)
	if err := c.getJSON(ctx, path, nil, &data); err != nil {
		return nil, err
	}
	return &data, nil
}

// RankingInfo contains ranking information for a team at an event.
type RankingInfo struct {
	TeamKey      string `json:"team_key"`
	Rank         int    `json:"rank"`
	MatchesPlayed int   `json:"matches_played"`
	QualAverage  float64 `json:"qual_average"`
	Record       struct {
		Wins   int `json:"wins"`
		Losses int `json:"losses"`
		Ties   int `json:"ties"`
	} `json:"record"`
	Dq         int `json:"dq"`
	QualPoints int `json:"qual_points"`
	ElimPoints int `json:"elim_points"`
	AwardPoints int `json:"award_points"`
	AlliancePoints int `json:"alliance_points"`
	TiePoints int `json:"tie_points"`
	TotalPoints int `json:"total_points"`
}

// EventRankings fetches rankings for an event.
func (c *TBAClient) GetEventRankings(ctx context.Context, eventKey string) ([]RankingInfo, error) {
	var data struct {
		Rankings []RankingInfo `json:"rankings"`
	}
	path := fmt.Sprintf("/event/%s/rankings", eventKey)
	if err := c.getJSON(ctx, path, nil, &data); err != nil {
		return nil, err
	}
	return data.Rankings, nil
}

// EventInfo contains event metadata.
type EventInfo struct {
	Key       string     `json:"key"`
	Name      string     `json:"name"`
	EventCode string     `json:"event_code"`
	Year      int        `json:"year"`
	StartDate string     `json:"start_date"`
	EndDate   string     `json:"end_date"`
	Timezone  string     `json:"timezone"`
	Official  bool       `json:"official"`
	Playoff   string     `json:"playoff"`
}

// GetEvent fetches detailed event information.
func (c *TBAClient) GetEvent(ctx context.Context, eventKey string) (*EventInfo, error) {
	var event EventInfo
	path := fmt.Sprintf("/event/%s", eventKey)
	if err := c.getJSON(ctx, path, nil, &event); err != nil {
		return nil, err
	}
	return &event, nil
}

// MatchInfo contains match information including timing.
type MatchInfo struct {
	Key          string    `json:"key"`
	EventKey     string    `json:"event_key"`
	CompLevel    string    `json:"comp_level"`
	SetNumber    int       `json:"set_number"`
	MatchNumber  int       `json:"match_number"`
	Alliances    struct {
		Red struct {
			Teams []string `json:"team_keys"`
			Score int      `json:"score"`
		} `json:"red"`
		Blue struct {
			Teams []string `json:"team_keys"`
			Score int      `json:"score"`
		} `json:"blue"`
	} `json:"alliances"`
	ActualTime int64  `json:"actual_time"`
	PredictedTime int64  `json:"predicted_time"`
	ScheduledTime int64  `json:"scheduled_time"`
	ScoreBreakdown interface{} `json:"score_breakdown"`
}

// GetEventMatches fetches all matches for an event.
func (c *TBAClient) GetEventMatches(ctx context.Context, eventKey string) ([]MatchInfo, error) {
	var matches []MatchInfo
	path := fmt.Sprintf("/event/%s/matches", eventKey)
	if err := c.getJSON(ctx, path, nil, &matches); err != nil {
		return nil, err
	}
	return matches, nil
}
