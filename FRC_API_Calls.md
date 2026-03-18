# FRC Events API Integration Guide

**Version:** 3.0  
**Base URL:** `https://frc-api.firstinspires.org/v3.0`  
**Authentication Required:** All endpoints  
**Token Request:** https://frc-events.firstinspires.org/services/API

---

## Authentication

All requests must include the following header:

```
Authorization: Basic <base64_encoded_token>
```

**Token Creation:**
1. Combine your username and authorization key with a colon: `username:authorizationKey`
2. Base64 encode the string: `echo -n "username:key" | base64`
3. Use the result in the Authorization header

**Example (placeholder values only):**
```
username: your-first-username
key: your-first-api-key
encoded: <base64("your-first-username:your-first-api-key")>
```

Never commit real credentials to documentation, source, or logs.

---

## Request Headers

### Cache Control Headers
- **`If-Modified-Since`**: Returns 304 if no changes since provided date
- **`FMS-OnlyModifiedSince`**: Returns only modified records since provided date
- **`Accept`**: `application/json` or `application/xml`

### Response Headers
- **`Last-Modified`**: Timestamp of last data modification (save this for next request)
- **`Cache-Control`**: Caching guidance
- **`Content-Type`**: Response format

---

## Call Schedule Strategy

### 1. Initial Population (Run Once)
These calls populate your database with baseline data at the start of the season or when setting up the app.

### 2. Periodic Updates (Daily/Weekly)
These calls keep your data fresh when no active events are occurring.

### 3. Event Active (Every 5-15 minutes)
These calls run frequently during an active event you're scouting.

### 4. On-Demand (User Triggered)
These calls run when users select specific teams or matches.

---

## API Endpoints by Use Case

## 1. INITIAL POPULATION CALLS

### 1.1 Get Current Season
**Endpoint:** `GET /season`  
**Schedule:** Once at application start  
**Purpose:** Get the current FRC season year  

**Response:**
```json
{
  "seasonsFRC": 2026,
  "seasonsYear": 2026,
  "eventCount": 0,
  "gameName": "REEFSCAPE",
  "kickoffDate": "2026-01-04"
}
```

**Database Action:** Store current season year for all subsequent calls

---

### 1.2 List All Events for Season
**Endpoint:** `GET /{season}/events`  
**Schedule:** Once at season start, weekly during season  
**Parameters:**
- `{season}` - Year (e.g., 2026)

**Query Parameters (optional):**
- `eventCode` - Filter by specific event code
- `teamNumber` - Events a team is attending
- `districtCode` - Events in a specific district
- `excludeDistrict` - true/false

**Response:**
```json
{
  "Events": [
    {
      "eventCode": "MNDU",
      "name": "Minnesota North Star Regional",
      "type": "Regional",
      "districtCode": null,
      "venue": "Duluth Entertainment Convention Center",
      "city": "Duluth",
      "stateprov": "Minnesota",
      "country": "USA",
      "dateStart": "2026-03-05",
      "dateEnd": "2026-03-07",
      "address": "350 Harbor Dr, Duluth, MN 55802",
      "website": "http://northstarregional.org",
      "webcasts": [],
      "timezone": "America/Chicago"
    }
  ],
  "eventCount": 1
}
```

**Database Action:**
- Store all events with codes, names, dates, locations
- Create events table with fields: event_code, name, type, city, state, country, date_start, date_end, venue, timezone
- Use this for event selector dropdown in scouting forms

---

### 1.3 Get Teams at Specific Event
**Endpoint:** `GET /{season}/teams`  
**Schedule:** Once per event during pre-season, daily during event  
**Parameters:**
- `{season}` - Year (e.g., 2026)

**Query Parameters:**
- `eventCode` - Required to get teams at specific event
- `teamNumber` - Get specific team
- `districtCode` - Get teams in district
- `state` - Teams from specific state
- `page` - Page number for pagination

**Response:**
```json
{
  "teams": [
    {
      "teamNumber": 2220,
      "nameFull": "Blue Twilight",
      "nameShort": "Blue Twilight",
      "schoolName": "Heritage Christian Academy/etc",
      "city": "Maple Grove",
      "stateProv": "Minnesota",
      "country": "USA",
      "rookieYear": 2007,
      "robotName": "TBD",
      "districtCode": null,
      "website": "http://www.blueTwilightRobotics.org"
    }
  ],
  "teamCountTotal": 1,
  "teamCountPage": 1,
  "pageCurrent": 1,
  "pageTotal": 1
}
```

**Database Action:**
- Store teams attending each event in `event_teams` junction table
- Store team details in `teams` table: team_number, name_full, name_short, city, state, country, rookie_year
- Use for team selector dropdown in scouting forms

---

## 2. EVENT ACTIVE CALLS (During Competition)

### 2.1 Get Match Schedule
**Endpoint:** `GET /{season}/schedule/{eventCode}`  
**Schedule:** Every 15-30 minutes during event  
**Parameters:**
- `{season}` - Year (2026)
- `{eventCode}` - Event code (e.g., "MNDU")

**Query Parameters (optional):**
- `tournamentLevel` - qual, playoff
- `teamNumber` - Matches for specific team
- `matchNumber` - Specific match
- `start` - Start match number
- `end` - End match number

**Response:**
```json
{
  "Schedule": [
    {
      "description": "Qualification 1",
      "tournamentLevel": "Qualification",
      "matchNumber": 1,
      "startTime": "2026-03-05T09:00:00",
      "field": "Main",
      "teams": [
        {
          "teamNumber": 2220,
          "station": "Red1",
          "surrogate": false,
          "dq": false
        },
        {
          "teamNumber": 1816,
          "station": "Red2",
          "surrogate": false,
          "dq": false
        },
        {
          "teamNumber": 4181,
          "station": "Red3",
          "surrogate": false,
          "dq": false
        },
        {
          "teamNumber": 2220,
          "station": "Blue1",
          "surrogate": false,
          "dq": false
        },
        {
          "teamNumber": 5837,
          "station": "Blue2",
          "surrogate": false,
          "dq": false
        },
        {
          "teamNumber": 3130,
          "station": "Blue3",
          "surrogate": false,
          "dq": false
        }
      ]
    }
  ]
}
```

**Database Action:**
- Store/update match schedule in `matches` table
- Store team assignments in `match_teams` table
- Display upcoming matches for scouting assignment
- Use to validate match data entry (ensure team was actually in that match)

---

### 2.2 Get Match Results
**Endpoint:** `GET /{season}/matches/{eventCode}`  
**Schedule:** Every 5-10 minutes during active event  
**Parameters:**
- `{season}` - Year (2026)
- `{eventCode}` - Event code

**Query Parameters (optional):**
- `tournamentLevel` - qual, playoff
- `teamNumber` - Matches for specific team
- `matchNumber` - Specific match
- `start`, `end` - Match range

**Response:**
```json
{
  "Matches": [
    {
      "description": "Qualification 1",
      "tournamentLevel": "Qualification",
      "matchNumber": 1,
      "startTime": "2026-03-05T09:00:00",
      "actualStartTime": "2026-03-05T09:02:34",
      "postResultTime": "2026-03-05T09:05:12",
      "scoreRedFinal": 124,
      "scoreRedFoul": 10,
      "scoreRedAuto": 35,
      "scoreBlueFinal": 98,
      "scoreBlueFoul": 0,
      "scoreBlueAuto": 28,
      "teams": [
        {
          "teamNumber": 2220,
          "station": "Red1",
          "dq": false
        }
      ]
    }
  ]
}
```

**Database Action:**
- Update matches with scores and actual times
- Use for displaying completed match data
- Cross-reference with scouting submissions
- Calculate team statistics

---

### 2.3 Get Score Details (Detailed Breakdown)
**Endpoint:** `GET /{season}/scores/{eventCode}/{tournamentLevel}`  
**Schedule:** Every 10-15 minutes during event  
**Parameters:**
- `{season}` - Year (2026)
- `{eventCode}` - Event code
- `{tournamentLevel}` - "qual" or "playoff"

**Query Parameters (optional):**
- `teamNumber` - Scores for specific team
- `matchNumber` - Specific match
- `start`, `end` - Match range

**Response (2026 REEFSCAPE Specific):**
```json
{
  "MatchScores": [
    {
      "matchLevel": "Qualification",
      "matchNumber": 1,
      "Alliances": [
        {
          "alliance": "Red",
          "totalPoints": 124,
          "autoPoints": 35,
          "teleopPoints": 79,
          "endgamePoints": 10,
          "foulPoints": 0,
          "adjustPoints": 0,
          "rp": 2,
          "autoTowerRobot1": "Level2",
          "autoTowerRobot2": "Level1",
          "autoTowerRobot3": "None",
          "autoTowerPoints": 35,
          "endGameTowerRobot1": "Level3",
          "endGameTowerRobot2": "Level2",
          "endGameTowerRobot3": "Level1",
          "endGameTowerPoints": 10,
          "hubScore": {
            "autoCount": 5,
            "autoPoints": 20,
            "teleopCount": 12,
            "teleopPoints": 48,
            "endgameCount": 2,
            "endgamePoints": 8,
            "shift1Count": 3,
            "shift1Points": 12,
            "totalCount": 22,
            "totalPoints": 88
          },
          "energizedAchieved": true,
          "superchargedAchieved": false,
          "traversalAchieved": true,
          "minorFoulCount": 0,
          "majorFoulCount": 0
        },
        {
          "alliance": "Blue",
          "totalPoints": 98
        }
      ]
    }
  ]
}
```

**Database Action:**
- Store detailed scoring breakdown per match/alliance
- Use for advanced analytics and OPR calculations
- Display in match detail views
- Analyze specific game element performance

---

### 2.4 Get Event Rankings
**Endpoint:** `GET /{season}/rankings/{eventCode}`  
**Schedule:** Every 10-15 minutes during event  
**Parameters:**
- `{season}` - Year (2026)
- `{eventCode}` - Event code

**Query Parameters (optional):**
- `teamNumber` - Get specific team's ranking
- `top` - Get top N teams

**Response:**
```json
{
  "Rankings": [
    {
      "rank": 1,
      "teamNumber": 2220,
      "sortOrder1": 2.45,
      "sortOrder2": 145.2,
      "sortOrder3": 98.3,
      "sortOrder4": 47.8,
      "sortOrder5": 15,
      "sortOrder6": 0,
      "wins": 8,
      "losses": 2,
      "ties": 0,
      "qualAverage": 145.2,
      "dq": 0,
      "matchesPlayed": 10
    }
  ]
}
```

**Database Action:**
- Store current rankings
- Display on dashboard
- Use for strategic alliance selection planning
- Track ranking point accumulation

---

### 2.5 Get Alliance Selections
**Endpoint:** `GET /{season}/alliances/{eventCode}`  
**Schedule:** Once after alliance selection, then as needed  
**Parameters:**
- `{season}` - Year (2026)
- `{eventCode}` - Event code

**Response:**
```json
{
  "Alliances": [
    {
      "number": 1,
      "captain": 2220,
      "round1": 1816,
      "round2": 4181,
      "round3": null,
      "backup": null,
      "backupReplaced": null
    }
  ],
  "count": 8
}
```

**Database Action:**
- Store alliance selections
- Display playoff bracket
- Track backup team usage

---

### 2.6 Get Awards
**Endpoint:** `GET /{season}/awards/{eventCode}`  
**Schedule:** End of event or daily  
**Parameters:**
- `{season}` - Year (2026)
- `{eventCode}` - Event code

**Query Parameters (optional):**
- `teamNumber` - Awards for specific team

**Response:**
```json
{
  "Awards": [
    {
      "awardId": 1,
      "teamNumber": 2220,
      "awardName": "Regional Winner",
      "series": 0,
      "person": null
    }
  ],
  "awardCount": 1
}
```

**Database Action:**
- Store awards won by teams
- Display team achievements

---

## 3. PERIODIC UPDATE CALLS

### 3.1 Get District Listings
**Endpoint:** `GET /{season}/districts`  
**Schedule:** Once per season  
**Parameters:**
- `{season}` - Year (2026)

**Response:**
```json
{
  "districts": [
    {
      "code": "FIM",
      "name": "FIRST In Michigan"
    }
  ],
  "districtCount": 1
}
```

**Database Action:**
- Store district codes and names
- Use for filtering events and teams

---

### 3.2 Get All Teams (Full List)
**Endpoint:** `GET /{season}/teams`  
**Schedule:** Once per season, or when needed  
**Parameters:**
- `{season}` - Year (2026)

**Query Parameters:**
- `page` - Page number (pagination required for full list)

**Response:** Same as 1.3 (Get Teams at Event)

**Database Action:**
- Populate full teams database
- Use for team lookup and autocomplete

---

## 4. UTILITY/ANCILLARY CALLS

### 4.1 Get Event List By Type
**Endpoint:** `GET /{season}/events`  
**Query Parameters:**
- `eventCode` - Specific event
- `teamNumber` - Events team is attending
- `districtCode` - District events only
- `excludeDistrict` - Exclude district events

Use to populate event selectors with filtered lists.

---

### 4.2 Avatar/Logo Endpoints
**Endpoint:** `GET /{season}/avatars`  
**Purpose:** Get team avatar images

Team avatars can be displayed in the UI using:
```
https://frc-api.firstinspires.org/{season}/avatars/{teamNumber}?size=small
```

Sizes: `small`, `medium`, `large`

---

## IMPLEMENTATION NOTES

### Caching Strategy

```go
type APICache struct {
    LastModified string
    Data         interface{}
    Endpoint     string
}

// On first request
resp := makeRequest(endpoint)
cache := APICache{
    LastModified: resp.Header.Get("Last-Modified"),
    Data:         resp.Body,
    Endpoint:     endpoint,
}
saveCache(cache)

// On subsequent requests
req.Header.Set("If-Modified-Since", cache.LastModified)
resp := makeRequest(endpoint)
if resp.StatusCode == 304 {
    // Use cached data
    return cache.Data
}
// Update cache with new data
```

### Incremental Updates

Use `FMS-OnlyModifiedSince` instead of `If-Modified-Since` to get only changed records:

```go
req.Header.Set("FMS-OnlyModifiedSince", lastCheck)
resp := makeRequest(endpoint)
// resp contains only modified records since lastCheck
mergeIntoDatabase(resp.Body)
```

---

## CALL PRIORITY SCHEDULE

### Application Startup
1. `GET /season` - Get current season
2. `GET /{season}/events` - Get all events
3. `GET /{season}/districts` - Get districts

### Event Setup (Before/Early in Event)
1. `GET /{season}/teams?eventCode={code}` - Get teams at event
2. `GET /{season}/schedule/{eventCode}` - Get qualification schedule

### During Event (Every 5-10 minutes)
1. `GET /{season}/matches/{eventCode}` - Get match results
2. `GET /{season}/scores/{eventCode}/qual` - Get score details
3. `GET /{season}/rankings/{eventCode}` - Get rankings

### After Qualification (Once)
1. `GET /{season}/alliances/{eventCode}` - Get alliance selections
2. `GET /{season}/schedule/{eventCode}?tournamentLevel=playoff` - Get playoff schedule

### During Playoffs (Every 5-10 minutes)
1. `GET /{season}/matches/{eventCode}?tournamentLevel=playoff` - Playoff results
2. `GET /{season}/scores/{eventCode}/playoff` - Playoff score details

### End of Event
1. `GET /{season}/awards/{eventCode}` - Get awards

---

## ERROR HANDLING

### HTTP Status Codes
- **200 OK** - Success
- **304 Not Modified** - Use cached data
- **400 Bad Request** - Invalid parameters
- **401 Unauthorized** - Invalid/missing auth token
- **404 Not Found** - Event not found
- **500 Internal Server Error** - Server issue
- **501 Not Implemented** - Invalid API pattern
- **503 Service Unavailable** - Temporary overload

### Retry Strategy
```go
func callAPIWithRetry(endpoint string, maxRetries int) (*Response, error) {
    for i := 0; i < maxRetries; i++ {
        resp, err := makeRequest(endpoint)
        if err == nil && resp.StatusCode < 500 {
            return resp, nil
        }
        if resp.StatusCode == 503 {
            retryAfter := resp.Header.Get("Retry-After")
            if retryAfter != "" {
                waitDuration := parseRetryAfter(retryAfter)
                time.Sleep(waitDuration)
                continue
            }
        }
        time.Sleep(time.Duration(i+1) * time.Second * 5) // Exponential backoff
    }
    return nil, errors.New("max retries exceeded")
}
```

---

## RATE LIMITING

The FRC Events API does not publish explicit rate limits, but best practices:

1. **Use caching headers** - Always send `If-Modified-Since` or `FMS-OnlyModifiedSince`
2. **Minimum intervals**:
   - Active matches: 5 minutes
   - Between matches: 10-15 minutes
   - No active event: Daily/weekly
3. **Avoid polling loops** faster than 5 minutes
4. **Handle 503 errors** with exponential backoff

---

## INTEGRATION WITH SCOUTING APP

### Database Schema Requirements

Based on API calls, you'll need tables for:

```sql
-- Events
CREATE TABLE events (
    event_code VARCHAR(10) PRIMARY KEY,
    season INTEGER NOT NULL,
    name VARCHAR(255),
    type VARCHAR(50),
    date_start DATE,
    date_end DATE,
    city VARCHAR(100),
    state VARCHAR(100),
    country VARCHAR(100),
    venue VARCHAR(255),
    timezone VARCHAR(50)
);

-- Teams
CREATE TABLE teams (
    team_number INTEGER PRIMARY KEY,
    name_full VARCHAR(255),
    name_short VARCHAR(100),
    school_name VARCHAR(255),
    city VARCHAR(100),
    state_prov VARCHAR(100),
    country VARCHAR(100),
    rookie_year INTEGER,
    robot_name VARCHAR(100),
    district_code VARCHAR(10)
);

-- Event Teams (Junction)
CREATE TABLE event_teams (
    event_code VARCHAR(10),
    team_number INTEGER,
    PRIMARY KEY (event_code, team_number),
    FOREIGN KEY (event_code) REFERENCES events(event_code),
    FOREIGN KEY (team_number) REFERENCES teams(team_number)
);

-- Matches
CREATE TABLE matches (
    match_id SERIAL PRIMARY KEY,
    event_code VARCHAR(10),
    tournament_level VARCHAR(20),
    match_number INTEGER,
    description VARCHAR(100),
    start_time TIMESTAMP,
    actual_start_time TIMESTAMP,
    score_red_final INTEGER,
    score_red_auto INTEGER,
    score_red_foul INTEGER,
    score_blue_final INTEGER,
    score_blue_auto INTEGER,
    score_blue_foul INTEGER,
    FOREIGN KEY (event_code) REFERENCES events(event_code)
);

-- Match Teams
CREATE TABLE match_teams (
    match_id INTEGER,
    team_number INTEGER,
    alliance VARCHAR(10),
    station VARCHAR(10),
    surrogate BOOLEAN,
    dq BOOLEAN,
    PRIMARY KEY (match_id, team_number),
    FOREIGN KEY (match_id) REFERENCES matches(match_id),
    FOREIGN KEY (team_number) REFERENCES teams(team_number)
);

-- Match Score Details (2026 Game Specific)
CREATE TABLE match_score_details (
    match_id INTEGER,
    alliance VARCHAR(10),
    auto_tower_robot1 VARCHAR(20),
    auto_tower_robot2 VARCHAR(20),
    auto_tower_robot3 VARCHAR(20),
    auto_tower_points INTEGER,
    endgame_tower_robot1 VARCHAR(20),
    endgame_tower_robot2 VARCHAR(20),
    endgame_tower_robot3 VARCHAR(20),
    endgame_tower_points INTEGER,
    hub_auto_count INTEGER,
    hub_teleop_count INTEGER,
    hub_total_points INTEGER,
    energized_achieved BOOLEAN,
    supercharged_achieved BOOLEAN,
    traversal_achieved BOOLEAN,
    ranking_points INTEGER,
    PRIMARY KEY (match_id, alliance),
    FOREIGN KEY (match_id) REFERENCES matches(match_id)
);

-- Rankings
CREATE TABLE rankings (
    event_code VARCHAR(10),
    team_number INTEGER,
    rank INTEGER,
    wins INTEGER,
    losses INTEGER,
    ties INTEGER,
    qual_average DECIMAL(10,2),
    sort_order_1 DECIMAL(10,2),
    sort_order_2 DECIMAL(10,2),
    dq INTEGER,
    matches_played INTEGER,
    updated_at TIMESTAMP,
    PRIMARY KEY (event_code, team_number),
    FOREIGN KEY (event_code) REFERENCES events(event_code),
    FOREIGN KEY (team_number) REFERENCES teams(team_number)
);

-- Alliance Selections
CREATE TABLE alliances (
    event_code VARCHAR(10),
    alliance_number INTEGER,
    captain INTEGER,
    round1_pick INTEGER,
    round2_pick INTEGER,
    round3_pick INTEGER,
    backup INTEGER,
    backup_replaced INTEGER,
    PRIMARY KEY (event_code, alliance_number),
    FOREIGN KEY (event_code) REFERENCES events(event_code)
);

-- Awards
CREATE TABLE awards (
    award_id SERIAL PRIMARY KEY,
    event_code VARCHAR(10),
    team_number INTEGER,
    award_name VARCHAR(255),
    series INTEGER,
    person VARCHAR(255),
    FOREIGN KEY (event_code) REFERENCES events(event_code),
    FOREIGN KEY (team_number) REFERENCES teams(team_number)
);

-- API Cache
CREATE TABLE api_cache (
    endpoint VARCHAR(500) PRIMARY KEY,
    last_modified VARCHAR(100),
    last_checked TIMESTAMP,
    data JSONB
);
```

### Populating Dropdowns

**Event Selector:**
```sql
SELECT event_code, name, date_start, date_end 
FROM events 
WHERE season = 2026 
ORDER BY date_start;
```

**Team Selector (for specific event):**
```sql
SELECT t.team_number, t.name_short 
FROM teams t
JOIN event_teams et ON t.team_number = et.team_number
WHERE et.event_code = ?
ORDER BY t.team_number;
```

---

## COMPLEMENTARY: Blue Alliance API

Your DataPoints.md references The Blue Alliance API which provides additional analytics. Consider using **both APIs**:

- **FRC Events API**: Official real-time match data
- **Blue Alliance API**: OPR, DPR, CCWM, predictions, historical data

The Blue Alliance API is documented at: https://www.thebluealliance.com/apidocs/v3

---

## TESTING

### Test with Season 2024 Data
Use 2024 data for testing (complete season):
```
GET https://frc-api.firstinspires.org/v3.0/2024/events
GET https://frc-api.firstinspires.org/v3.0/2024/matches/MNDU
```

### Mock Event Code
Some events for testing:
- `MNDU` - Minnesota North Star Regional
- `CAOC` - Orange County Regional
- `MIWMI` - West Michigan District Event

---

## SUPPORT & RESOURCES

- **API Token Request**: https://frc-events.firstinspires.org/services/API
- **Documentation**: https://frc-api-docs.firstinspires.org/
- **FMS Forum**: FIRST Tech Challenge TeamForge site
- **Status Page**: Check for API availability issues

## SUMMARY: MINIMUM VIABLE CALLS

For a basic scouting app, you **must** implement:

1. ✅ `GET /season` - Current season
2. ✅ `GET /{season}/events` - Event list
3. ✅ `GET /{season}/teams?eventCode={code}` - Teams at event
4. ✅ `GET /{season}/schedule/{eventCode}` - Match schedule
5. ✅ `GET /{season}/matches/{eventCode}` - Match results

**Nice to have:**
- `GET /{season}/rankings/{eventCode}` - Rankings
- `GET /{season}/scores/{eventCode}/qual` - Detailed scores
- `GET /{season}/alliances/{eventCode}` - Alliance selections
