# Team Page Implementation & Field Analysis

## Overview
This document provides a comprehensive analysis of the team page implementation, field definitions, and how team data is fetched and displayed throughout the TealTeam codebase.

---

## 1. Team Page Implementation

### 1.1 Team Page Handler - [internal/handlers/team.go](internal/handlers/team.go)

**Main Entry Points:**

- **`HandleTeamPage()`** - Renders the team search and lookup page
  - Gets the team number from query parameter
  - Calls `hydrateTeamLookupData()` to fetch team info from database
  - Calls `hydrateEventSelectionData()` to populate available events
  - Renders the "team" template

- **`HandleTeamSearch()`** - HTMX endpoint returning team info fragment
  - Accepts team number via query parameter
  - Returns HTML fragment via `renderPartial(c, "team_info", data)`
  - Used for real-time team search functionality

- **`HandleTeamEventData()`** - HTMX endpoint returning team data for specific event
  - Accepts `team` and `event_id` query parameters
  - Calls `hydrateTeamEventData()` to fetch stats and scouting info
  - Returns HTML fragment via `renderPartial(c, "team_data", data)`

**Key Helper Functions:**

- **`hydrateTeamLookupData()`** - Fetches team from database and prepares data
  - Database query: `SELECT * FROM teams WHERE team_number = ?`
  - Populates: TeamNumber, TeamName, Team object
  - Calls `hydrateTeamEventsData()` to find associated events

- **`hydrateTeamEventsData()`** - Fetches events for a team
  - Lists events from database by team association
  - Falls back to TBA API sync if no local events found
  - Returns event list with ID and Name

- **`hydrateTeamEventData()`** - Aggregates all data for team at specific event
  - Fetches **TeamEventStats** from `team_event_stats` table
  - Fetches all **ScoutingData** for the team at the event
  - Calculates aggregations:
    - Most common starting position
    - Most common defense rating
    - Most common traversal
    - Most common scoring strategy
    - Most common hang level
    - Most common auto hang configuration
    - Most common hang position
    - Alliance color distribution

**Data Flow:**
```
User searches for team
  ↓
HandleTeamSearch() / HandleTeamPage()
  ↓
hydrateTeamLookupData()
  ↓
Database: SELECT FROM teams
  ↓
hydrateTeamEventsData()
  ↓
If no events: SyncTeamForUser() [TBA API]
  ↓
User selects event
  ↓
HandleTeamEventData()
  ↓
hydrateTeamEventData()
  ↓
Database: SELECT FROM team_event_stats
Database: SELECT FROM scouting_data
  ↓
Render team_data.html with stats
```

### 1.2 Team Page Template - [web/templates/pages/team.html](web/templates/pages/team.html)

**Structure:**
- Team search form with number input
- Real-time search via HTMX to `/hx/teams/search`
- Container for team info partial (team_info.html)
- Container for team data partial (team_data.html)

**Key Features:**
- Error message display for invalid teams
- HTMX integration for dynamic team lookup
- Auto-loads team_info template when TeamNumber is set

### 1.3 Team Info Partial - [web/templates/partials/team_info.html](web/templates/partials/team_info.html)

**Displays:**
- Team header (number, name, school)
- Location info (city, state, country)
- Team metadata grid:
  - Rookie Year
  - Nickname
  - Website link
  - Team Motto
- Event selection dropdown
- Dynamic event loading via HTMX change trigger

### 1.4 Team Data Partial - [web/templates/partials/team_data.html](web/templates/partials/team_data.html)

**Displays multiple stat sections:**

1. **Event Summary Section:**
   - Rank (yellow badge)
   - Record (W-L-T)
   - Matches Played
   - Qualification Average (Qual Avg)
   - Disqualifications (DQ count)

2. **OPR Stats Section:**
   - OPR (Offensive Power Rating)
   - DPR (Defensive Power Rating)
   - CCWM (Calculated Contribution to Winning Margin)

3. **Component OPRs Section:**
   - AutoOPR (autonomous period performance)
   - TeleopOPR (teleoperation period performance)
   - EndgameOPR (endgame period performance)

4. **District Points Section:**
   - Qual Points (qualification round points)
   - Elim Points (elimination/playoff points)
   - Award Points (award points earned)
   - Alliance Points (alliance contribution)
   - Total Points (highlighted)

5. **Autos Section:**
   - Most Common Start position
   - Display of auto hang data from scouting

6. **Scouting Statistics Section:**
   - Most common defense rating
   - Most common traversal type
   - Most common scoring strategy
   - Most common hang level
   - Most common hang position
   - Alliance color distribution pie chart

---

## 2. Field Definitions & References

### 2.1 TeamEventStats Model - [internal/models/models.go](internal/models/models.go)

```go
type TeamEventStats struct {
    ID             int       `json:"id" gorm:"column:id;primaryKey"`
    TeamID         int       `json:"team_id" gorm:"column:team_id"`
    EventID        int       `json:"event_id" gorm:"column:event_id"`
    OPR            *float64  `json:"opr,omitempty" gorm:"column:opr"`
    DPR            *float64  `json:"dpr,omitempty" gorm:"column:dpr"`
    CCWM           *float64  `json:"ccwm,omitempty" gorm:"column:ccwm"`
    AutoOPR        *float64  `json:"auto_opr,omitempty" gorm:"column:auto_opr"`
    TeleopOPR      *float64  `json:"teleop_opr,omitempty" gorm:"column:teleop_opr"`
    EndgameOPR     *float64  `json:"endgame_opr,omitempty" gorm:"column:endgame_opr"`
    Rank           *int      `json:"rank,omitempty" gorm:"column:rank"`
    MatchesPlayed  int       `json:"matches_played" gorm:"column:matches_played"`
    QualAverage    *float64  `json:"qual_average,omitempty" gorm:"column:qual_average"`
    Wins           int       `json:"wins" gorm:"column:wins"`
    Losses         int       `json:"losses" gorm:"column:losses"`
    Ties           int       `json:"ties" gorm:"column:ties"`
    DQCount        int       `json:"dq_count" gorm:"column:dq_count"`
    QualPoints     *int64    `json:"qual_points,omitempty" gorm:"column:qual_points"`
    ElimPoints     *int64    `json:"elim_points,omitempty" gorm:"column:elim_points"`
    AwardPoints    *int64    `json:"award_points,omitempty" gorm:"column:award_points"`
    AlliancePoints *int64    `json:"alliance_points,omitempty" gorm:"column:alliance_points"`
    TotalPoints    *int64    `json:"total_points,omitempty" gorm:"column:total_points"`
    CreatedAt      time.Time `json:"created_at" gorm:"column:created_at"`
    UpdatedAt      time.Time `json:"updated_at" gorm:"column:updated_at"`
}
```

### 2.2 ScoutingData Model - [internal/models/models.go](internal/models/models.go)

```go
type ScoutingData struct {
    ID               int        `json:"id" gorm:"column:id;primaryKey"`
    EventID          int        `json:"event_id" gorm:"column:event_id"`
    TeamID           int        `json:"team_id" gorm:"column:team_id"`
    AllianceColor    string     `json:"alliance_color" gorm:"column:alliance_color"`
    Notes            *string    `json:"notes,omitempty" gorm:"column:notes"`
    StartingPosition *string    `json:"starting_position,omitempty" gorm:"column:starting_position"`
    DefenseRating    *string    `json:"defense_rating,omitempty" gorm:"column:defense_rating"`
    ScoringStrategy  *string    `json:"scoring_strategy,omitempty" gorm:"column:scoring_strategy"`
    ShootingSpeed    *string    `json:"shooting_speed,omitempty" gorm:"column:shooting_speed"`
    Capacity         *string    `json:"capacity,omitempty" gorm:"column:capacity"`
    Defendability    *string    `json:"defendability,omitempty" gorm:"column:defendability"`
    Traversal        *string    `json:"traversal,omitempty" gorm:"column:traversal"`
    HangLevel        *string    `json:"hang_level,omitempty" gorm:"column:hang_level"`
    AutoHang         *string    `json:"auto_hang,omitempty" gorm:"column:auto_hang"`
    HangPosition     *string    `json:"hang_position,omitempty" gorm:"column:hang_position"`
    ScoutedAt        *time.Time `json:"scouted_at,omitempty" gorm:"column:scouted_at"`
    ScouterID        *int       `json:"scouter_id,omitempty" gorm:"column:scouter_id"`
    CreatedAt        time.Time  `json:"created_at" gorm:"column:created_at"`
    UpdatedAt        time.Time  `json:"updated_at" gorm:"column:updated_at"`
}
```

### 2.3 "Auto" Fields Analysis

**Current "Auto" Fields:**

1. **AutoOPR** (TeamEventStats)
   - Type: `*float64`
   - Column: `auto_opr`
   - Source: TBA API - Component OPR data
   - Display: team_data.html - Component OPRs section

2. **AutoHang** (ScoutingData)
   - Type: `*string`
   - Column: `auto_hang`
   - Source: Scouting form submission
   - Display: Calculated as "Most Common Auto Hang"
   - Values: Categorical options

**Historical "Auto" Fields (Removed):**

1. **auto_path_* fields**
   - Removed via: `migrations/0002_remove_auto_path_fields.sql`
   - Previous columns: `auto_path_data` (JSONB), `auto_path_image_url` (TEXT)
   - Was part of: `scouting_data` and `scouting_submissions` tables
   - Previous table: `auto_paths` (completely dropped)
   - Reason: Legacy feature removal - documented in AUTO_PATH_REMOVAL_RECORD.md

2. **auto_score, teleop_score, endgame_score fields**
   - Removed via: `migrations/0003_remove_score_fields.sql`
   - Previous columns: Unused scoring fields
   - Reason: Never captured in submission form, unused data

3. **auto_tower_level, auto_hand** and other uncaptured fields
   - Removed via: `migrations/0004_remove_uncaptured_fields.sql`
   - Reason: Not part of current scouting form schema

### 2.4 "Qual" Fields Analysis

**Current "Qual" Fields:**

1. **QualAverage** (TeamEventStats)
   - Type: `*float64`
   - Column: `qual_average`
   - Source: TBA API - Rankings endpoint
   - Display: team_data.html - Primary Stats section
   - Meaning: Average score per qualification round

2. **QualPoints** (TeamEventStats)
   - Type: `*int64`
   - Column: `qual_points`
   - Source: TBA API - District Points endpoint
   - Display: team_data.html - District Points section
   - Meaning: Points earned during qualification rounds

### 2.5 "Pionts" Typo Search

**Result: NO MATCHES FOUND** ✓

The typo "pionts" does not appear anywhere in the codebase. All instances use correct spelling "points".

---

## 3. How Team Data is Fetched and Displayed

### 3.1 Data Fetching Pipeline

**Source 1: TBA (The Blue Alliance) API - Via [internal/frc/team_stats_sync.go](internal/frc/team_stats_sync.go)**

1. **OPR Stats** - Fetched via: `GetEventOPRs()`
   - Endpoint: TBA API `/event/{event_key}/oprs`
   - Fields: OPR, DPR, CCWM

2. **Component OPRs** - Fetched via: `GetEventComponentOPRs()`
   - Endpoint: TBA API `/event/{event_key}/coprs`
   - Fields: AutoOPR, TeleopOPR, EndgameOPR
   - Contains breakdown by scoring phase

3. **Rankings** - Fetched via: `GetEventRankings()`
   - Endpoint: TBA API `/event/{event_key}/rankings`
   - Fields: Rank, QualAverage, Record (W-L-T), DQCount, QualPoints, ElimPoints, AwardPoints, AlliancePoints, TotalPoints

4. **Match Data** - Fetched via: `GetEventMatches()`
   - Endpoint: TBA API `/event/{event_key}/matches`
   - Used for match schedule and results

**Source 2: Local Database - Scouting Data**

- Field: `scouting_data` table
- Populated: Via scouting form submissions
- Captures: Starting position, defense rating, traversal, scoring strategy, hang level, auto hang, hang position, alliance color

**Source 3: Team Information**

- Field: `teams` table
- Synced: From TBA API on team lookup
- Contains: Team basic info, school, location, rookie year, nickname, website, motto

### 3.2 Data Sync Flow

**Sync Process:** [internal/frc/team_stats_sync.go](internal/frc/team_stats_sync.go) - `SyncTeamStatsForEvent()`

```
SyncAllTeamStats()
├─ Gets all active events
├─ For each event:
│  ├─ GetEventOPRs() [TBA API]
│  ├─ GetEventComponentOPRs() [TBA API]
│  ├─ GetEventRankings() [TBA API]
│  └─ For each team at event:
│     └─ Upsert into team_event_stats table
└─ SyncEventMatches() [TBA API]
   └─ Upsert into matches table
```

**Upsert Logic:**
- DB: `ON CONFLICT (team_id, event_id) DO UPDATE`
- Updates: OPR, DPR, CCWM, AutoOPR, TeleopOPR, EndgameOPR, all ranking/district fields
- Only updates changed fields

### 3.3 Data Display Flow

**Step 1: Team Lookup**
```
GET /team?team=10101
  ↓
hydrateTeamLookupData()
  ↓
SELECT * FROM teams WHERE team_number = 10101
```

**Step 2: Event Selection (via HTMX)**
```
GET /hx/teams/search?team=10101
  ↓
GetEventsForTeam()
  ↓
SELECT event_id FROM event_teams WHERE team_id = ?
SELECT * FROM events WHERE id IN (...)
  ↓
Render team_info.html partial
```

**Step 3: Event Data Display (via HTMX)**
```
GET /hx/teams/data?team=10101&event_id=5
  ↓
hydrateTeamEventData()
  ├─ SELECT * FROM team_event_stats WHERE team_id = ? AND event_id = ?
  ├─ SELECT * FROM scouting_data WHERE team_id = ? AND event_id = ?
  ├─ Calculate aggregations (most common values)
  └─ Populate data map
  ↓
Render team_data.html partial
```

### 3.4 Display Locations

| Field | Display Location | Section |
|-------|------------------|---------|
| QualAverage | team_data.html | Primary Stats (blue card) |
| QualPoints | team_data.html | District Points (gray cards) |
| AutoOPR | team_data.html | Component OPRs (yellow card) |
| AutoHang | team_data.html | Autos section (calculated) |
| Rank | team_data.html | Primary Stats (yellow card) |
| Wins/Losses/Ties | team_data.html | Primary Stats (Record) |
| MatchesPlayed | team_data.html | Primary Stats |
| DQCount | team_data.html | Primary Stats (red card if > 0) |
| OPR, DPR, CCWM | team_data.html | OPR Stats section |
| TeleopOPR, EndgameOPR | team_data.html | Component OPRs |
| ElimPoints, AwardPoints, AlliancePoints, TotalPoints | team_data.html | District Points section |
| StartingPosition | team_data.html | Autos section |
| DefenseRating | team_data.html | Qualitative Notes |
| Traversal | team_data.html | Qualitative Notes |
| ScoringStrategy | team_data.html | Qualitative Notes |
| HangLevel | team_data.html | Qualitative Notes |
| HangPosition | team_data.html | Qualitative Notes |
| AllianceColor | team_data.html | Alliance Distribution chart |

### 3.5 Data Aggregation Examples

From [internal/handlers/team.go](internal/handlers/team.go) - `hydrateTeamEventData()`:

```go
// Most common starting position
var mostCommonPos string
var maxCount int
for pos, cnt := range startingPositions {
    if cnt > maxCount {
        maxCount = cnt
        mostCommonPos = pos
    }
}
data["MostCommonStartPos"] = mostCommonPos

// Most common defense rating (similar pattern for all fields)
for rating, cnt := range defenseRatings {
    if cnt > maxCount {
        maxCount = cnt
        mostCommonDefense = rating
    }
}
```

---

## 4. Database Schema

### 4.1 team_event_stats Table
```sql
CREATE TABLE team_event_stats (
    id SERIAL PRIMARY KEY,
    team_id INTEGER NOT NULL,
    event_id INTEGER NOT NULL,
    opr DECIMAL(10,2),
    dpr DECIMAL(10,2),
    ccwm DECIMAL(10,2),
    auto_opr DECIMAL(10,2),
    teleop_opr DECIMAL(10,2),
    endgame_opr DECIMAL(10,2),
    rank INTEGER,
    matches_played INTEGER,
    qual_average DECIMAL(10,2),
    wins INTEGER,
    losses INTEGER,
    ties INTEGER,
    dq_count INTEGER,
    qual_points INTEGER,
    elim_points INTEGER,
    award_points INTEGER,
    alliance_points INTEGER,
    total_points INTEGER,
    created_at TIMESTAMP,
    updated_at TIMESTAMP,
    UNIQUE(team_id, event_id)
);
```

### 4.2 scouting_data Table
```sql
CREATE TABLE scouting_data (
    id SERIAL PRIMARY KEY,
    event_id INTEGER NOT NULL,
    team_id INTEGER NOT NULL,
    alliance_color VARCHAR(10) NOT NULL,
    notes TEXT,
    starting_position VARCHAR(20),
    defense_rating VARCHAR(20),
    scoring_strategy VARCHAR(50),
    shooting_speed VARCHAR(20),
    capacity VARCHAR(20),
    defendability TEXT,
    traversal VARCHAR(20),
    hang_level VARCHAR(10),
    auto_hang VARCHAR(10),
    hang_position VARCHAR(20),
    scouted_at TIMESTAMP,
    scouter_id INTEGER,
    created_at TIMESTAMP,
    updated_at TIMESTAMP
);
```

---

## 5. Summary of Findings

### Key Results:

✅ **Team page implementation:** Fully functional handler and template system
- Main handler: `handlers/team.go` with 3 main endpoints
- Templates: `pages/team.html`, `partials/team_info.html`, `partials/team_data.html`

✅ **"Auto" fields:**
- Current: AutoOPR (float64), AutoHang (string)
- Removed: auto_path_*, auto_score, auto_tower_level, auto_hand
- Source: TBA API (AutoOPR) + Scouting form (AutoHang)

✅ **"Qual" fields:**
- Current: QualAverage (float64), QualPoints (int64)
- Source: TBA API Rankings and District Points endpoints
- Display: Primary Stats and District Points sections

✅ **No "pionts" typo:** Zero matches in entire codebase

✅ **Data fetching:** Three sources integrated:
- TBA API (team stats, OPRs, rankings)
- Local scouting database (ScoutingData)
- Team information (Teams)

✅ **Data sync:** Automated via `team_stats_sync.go` with adaptive intervals (2min during events, 3hrs otherwise)

✅ **Data aggregation:** Local calculation of most common values from scouting data

---

## 6. Migration History

### Removed Features (Documented):

1. **0002_remove_auto_path_fields.sql**
   - Dropped: `auto_paths` table
   - Removed: `auto_path_data`, `auto_path_image_url` columns from scouting tables
   - Reference: AUTO_PATH_REMOVAL_RECORD.md

2. **0003_remove_score_fields.sql**
   - Removed: `auto_score`, `teleop_score`, `endgame_score` columns
   - Reason: Never captured in form

3. **0004_remove_uncaptured_fields.sql**
   - Removed: `throughput`, `hub_auto_count`, `hub_teleop_count`, `hub_endgame_count`, `penalties_caused`, `scouter_name`, `auto_tower_level`, `auto_hand`, `scoring_rating`, `endgame_tower_level`, `endgame_hang`
   - Reason: Not part of current scouting form schema

---

## 7. API Integration

### TBA Client - [internal/frc/tba_client.go](internal/frc/tba_client.go)

**Relevant Structures:**

```go
type ComponentOPRData struct {
    AutoOPRs   map[string]float64 `json:"auto_opr"`
    TeleopOPRs map[string]float64 `json:"teleop_opr"`
    EndgameOPRs map[string]float64 `json:"endgame_opr"`
}

type RankingInfo struct {
    Rank           int     `json:"rank"`
    QualAverage    float64 `json:"qual_average"`
    QualPoints     int     `json:"qual_points"`
    ElimPoints     int     `json:"elim_points"`
    AwardPoints    int     `json:"award_points"`
    AlliancePoints int     `json:"alliance_points"`
    TotalPoints    int     `json:"total_points"`
}
```

---

## Document Metadata

- **Last Updated:** 2026-03-04
- **Scope:** Complete team page implementation analysis
- **Coverage:** Handlers, templates, models, database schema, API integration, data fetching
- **Status:** All "auto", "qual" fields identified and documented
- **Typo Check:** "pionts" - NOT FOUND (0 matches)
