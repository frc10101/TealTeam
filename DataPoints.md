# Data Points for Scouting

## Manual Scouting Data (Collected by Scouts)
| Field | Values | Notes |
|-------|--------|-------|
| Starting Position | Left, Right, Center | Pre-match setup |
| Team Number | Integer | FRC team number |
| Auto Path | Visual Path Map | Interactive sketch of autonomous movement |
| Scoring | Likert Scale | Subjective performance rating |
| Standard Dev of Stats Per Match | Percentage | Consistency metric |
| End Game Hang | 0, 1, 2, 3 | Climbing level achieved |
| Auto Hand | 0, 1, 2, 3 | Autonomous game piece handling |
| Defence | Low, Mid, High | Defensive capability rating |
| Throughput | Low, Mid, High | Game piece cycling speed |
| Scoring Strategy | Passer, Stealer, Scorer | Play style |
| Traversal | Trench, Bump | Robot mobility type |

---

## The Blue Alliance API Data (API v3.12.0)

**API Base URL:** `https://www.thebluealliance.com/api/v3`  
**Authentication:** Requires `X-TBA-Auth-Key` header (get key from [Account Page](https://www.thebluealliance.com/account))

### Team Information
| Endpoint | Data Available |
|----------|---------------|
| `/team/{team_key}` | Team number, nickname, name, school_name, city, state_prov, country, rookie_year, motto, website |
| `/team/{team_key}/simple` | Condensed team info |
| `/team/{team_key}/years_participated` | Array of years team competed |
| `/team/{team_key}/robots` | Robot name per year |
| `/team/{team_key}/awards` | All awards won (name, type, event, year, recipients) |
| `/team/{team_key}/awards/{year}` | Awards for specific year |
| `/team/{team_key}/media/{year}` | Team photos, robot images, videos |
| `/team/{team_key}/districts` | District history |

### Event Statistics (Most Useful for Scouting)
| Endpoint | Data Available |
|----------|---------------|
| `/event/{event_key}/oprs` | **OPR** (Offensive Power Rating), **DPR** (Defensive Power Rating), **CCWM** (Calculated Contribution to Winning Margin) per team |
| `/event/{event_key}/coprs` | **Component OPRs** - Breakdown by scoring element (year-specific: auto points, teleop points, endgame, etc.) |
| `/event/{event_key}/rankings` | Team rank, matches_played, qual_average, W-L-T record, sort_orders (tiebreakers), DQ count |
| `/event/{event_key}/predictions` | TBA-generated match predictions, win probabilities |
| `/event/{event_key}/insights` | Event-wide statistics (avg scores, success rates for game-specific actions) |
| `/event/{event_key}/alliances` | Playoff alliance picks, captain/picks order, backup teams |
| `/event/{event_key}/district_points` | District point breakdown (qual_points, elim_points, award_points, alliance_points) |

### Match Data
| Endpoint | Data Available |
|----------|---------------|
| `/event/{event_key}/matches` | All matches with scores, alliances, times, videos |
| `/match/{match_key}` | Full match details including score_breakdown |
| `/match/{match_key}/simple` | Basic match info (teams, scores, winner) |
| `/match/{match_key}/timeseries` | Real-time match data (2018 only) |
| `/match/{match_key}/zebra_motionworks` | **Robot tracking data** - X/Y positions over time for all 6 robots |

### Score Breakdown Fields (2026 Season)

#### 2026 Game
- `autoTowerRobot1/2/3` (None, Level1, Level2, Level3)
- `autoTowerPoints`
- `endGameTowerRobot1/2/3` (None, Level1, Level2, Level3)
- `endGameTowerPoints`
- `totalTowerPoints`
- `hubScore` object:
  - `autoCount`, `autoPoints`
  - `teleopCount`, `teleopPoints`
  - `endgameCount`, `endgamePoints`
  - `shift1-4Count`, `shift1-4Points`
  - `transitionCount`, `transitionPoints`
  - `totalCount`, `totalPoints`
- `energizedAchieved` (boolean - ranking point)
- `superchargedAchieved` (boolean - ranking point)
- `traversalAchieved` (boolean - ranking point)
- `minorFoulCount`, `majorFoulCount`, `foulPoints`
- `g206Penalty` (boolean)
- `rp` (ranking points), `totalPoints`
- `totalAutoPoints`, `totalTeleopPoints`

### Team Performance at Events
| Endpoint | Data Available |
|----------|---------------|
| `/team/{team_key}/event/{event_key}/matches` | All matches for team at event |
| `/team/{team_key}/event/{event_key}/awards` | Awards won at event |
| `/team/{team_key}/event/{event_key}/status` | Qual ranking, alliance selection, playoff status |
| `/team/{team_key}/events/{year}/statuses` | Status at all events in a year |

### District Data
| Endpoint | Data Available |
|----------|---------------|
| `/district/{district_key}/rankings` | District rankings with point breakdowns |
| `/district/{district_key}/events` | All events in district |
| `/district/{district_key}/teams` | All teams in district |
| `/district/{district_key}/insights` | District-wide statistics |

### Zebra MotionWorks (Robot Tracking)
Available for select events - provides real-time robot position data:
- `times[]` - Timestamps (0.1 second intervals)
- `alliances.red[].xs[]`, `alliances.red[].ys[]` - X/Y coordinates for red robots
- `alliances.blue[].xs[]`, `alliances.blue[].ys[]` - X/Y coordinates for blue robots

**Use cases:** Heat maps, path analysis, defensive positioning, cycle time calculation

---

## Key Metrics Summary

### From Blue Alliance API
| Metric | Source | Description |
|--------|--------|-------------|
| **OPR** | `/event/{key}/oprs` | Offensive Power Rating - expected point contribution |
| **DPR** | `/event/{key}/oprs` | Defensive Power Rating - points prevented |
| **CCWM** | `/event/{key}/oprs` | OPR - DPR = net contribution to win margin |
| **Component OPRs** | `/event/{key}/coprs` | Year-specific breakdown (auto OPR, teleop OPR, etc.) |
| **Team Rank** | `/event/{key}/rankings` | Official event ranking |
| **W-L-T Record** | `/event/{key}/rankings` | Win-Loss-Tie record |
| **Match Schedule** | `/event/{key}/matches` | Match times, opponents, scores |
| **Alliance Partners** | `/match/{key}` | Who team plays with/against |
| **Score Breakdown** | `/match/{key}` | Detailed scoring by category |

### Calculated from API Data
| Metric | Calculation |
|--------|-------------|
| Consistency | Standard deviation of match scores |
| Auto Reliability | % of matches with successful auto |
| Endgame Success Rate | % of matches with climb/park |
| Cycle Time | Zebra tracking data analysis |
| Defense Effectiveness | DPR relative to event average |

---

## API Response Caching
TBA provides `Cache-Control` and `ETag` headers for efficient caching:
- Use `If-None-Match` header with previous `ETag` to check for updates
- Responses include `max-age` for cache validity period
- 304 responses indicate no changes since last request