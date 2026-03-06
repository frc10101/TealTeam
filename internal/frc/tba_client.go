package frc

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"math"
	"net/http"
	"net/url"
	"strings"
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

// ComponentOPRData contains dynamic component OPR breakdown per team.
// TBA returns a map of component-name -> map[teamKey]value.
type ComponentOPRData struct {
	Components map[string]map[string]float64
}

func (d *ComponentOPRData) UnmarshalJSON(data []byte) error {
	var raw map[string]map[string]float64
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	d.Components = raw
	return nil
}

func (d *ComponentOPRData) componentMap(preferredNames []string, containsAll []string) map[string]float64 {
	if d == nil || len(d.Components) == 0 {
		return nil
	}

	lookup := make(map[string]map[string]float64, len(d.Components))
	for name, values := range d.Components {
		lookup[strings.ToLower(strings.TrimSpace(name))] = values
	}

	for _, name := range preferredNames {
		if values, ok := lookup[strings.ToLower(strings.TrimSpace(name))]; ok {
			return values
		}
	}

	// Fallback for new seasons: pick the largest component map that matches tokens.
	var best map[string]float64
	bestSize := -1
	for name, values := range lookup {
		match := true
		for _, token := range containsAll {
			if !strings.Contains(name, token) {
				match = false
				break
			}
		}
		if match && len(values) > bestSize {
			best = values
			bestSize = len(values)
		}
	}

	return best
}

// TeamPhaseOPRs returns best-effort auto/teleop/endgame point contributions for a team.
func (d *ComponentOPRData) TeamPhaseOPRs(teamKey string) (auto *float64, teleop *float64, endgame *float64) {
	autoMap := d.componentMap(
		[]string{"totalAutoPoints", "autoPoints", "Hub Auto Points"},
		[]string{"auto", "points"},
	)
	teleopMap := d.componentMap(
		[]string{"totalTeleopPoints", "teleopPoints", "Hub Teleop Points"},
		[]string{"teleop", "points"},
	)
	endgameMap := d.componentMap(
		[]string{"endGameTowerPoints", "endgamePoints", "Hub Endgame Points"},
		[]string{"endgame", "points"},
	)

	if autoMap != nil {
		if v, ok := autoMap[teamKey]; ok {
			auto = &v
		}
	}
	if teleopMap != nil {
		if v, ok := teleopMap[teamKey]; ok {
			teleop = &v
		}
	}
	if endgameMap != nil {
		if v, ok := endgameMap[teamKey]; ok {
			endgame = &v
		}
	}

	return auto, teleop, endgame
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
	QualAverage  *float64 `json:"qual_average"`
	ExtraStats   []float64 `json:"extra_stats"`
	SortOrders   []float64 `json:"sort_orders"`
	Record       struct {
		Wins   int `json:"wins"`
		Losses int `json:"losses"`
		Ties   int `json:"ties"`
	} `json:"record"`
	Dq         int `json:"dq"`
	QualPoints *int `json:"qual_points"`
	ElimPoints *int `json:"elim_points"`
	AwardPoints *int `json:"award_points"`
	AlliancePoints *int `json:"alliance_points"`
	TiePoints *int `json:"tie_points"`
	TotalPoints *int `json:"total_points"`
}

// EffectiveQualAverage returns qual_average or falls back to first sort order (year-specific ranking score).
func (r RankingInfo) EffectiveQualAverage() *float64 {
	if r.QualAverage != nil {
		return r.QualAverage
	}
	if len(r.SortOrders) > 0 {
		v := r.SortOrders[0]
		return &v
	}
	return nil
}

// EffectiveAvgMatchPoints returns average match points from sort_orders[1] if available.
func (r RankingInfo) EffectiveAvgMatchPoints() *float64 {
	if len(r.SortOrders) > 1 {
		v := r.SortOrders[1]
		return &v
	}
	return nil
}

// EffectiveTotalPoints returns total points, falling back to first extra stat (often total ranking points).
func (r RankingInfo) EffectiveTotalPoints() *int64 {
	if r.TotalPoints != nil {
		v := int64(*r.TotalPoints)
		return &v
	}
	if len(r.ExtraStats) > 0 {
		v := int64(math.Round(r.ExtraStats[0]))
		return &v
	}
	return nil
}

// EffectiveQualPoints returns qual points or falls back to effective total points when only extra_stats are provided.
func (r RankingInfo) EffectiveQualPoints() *int64 {
	if r.QualPoints != nil {
		v := int64(*r.QualPoints)
		return &v
	}
	return r.EffectiveTotalPoints()
}

func optionalIntToInt64Ptr(v *int) *int64 {
	if v == nil {
		return nil
	}
	out := int64(*v)
	return &out
}

func (r RankingInfo) EffectiveElimPoints() *int64 {
	return optionalIntToInt64Ptr(r.ElimPoints)
}

func (r RankingInfo) EffectiveAwardPoints() *int64 {
	return optionalIntToInt64Ptr(r.AwardPoints)
}

func (r RankingInfo) EffectiveAlliancePoints() *int64 {
	return optionalIntToInt64Ptr(r.AlliancePoints)
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
