# Team Data Sync on Sign Up & Login

## Overview

When a user signs up or logs in with a team number, the application automatically fetches and stores all relevant data for that team, providing immediate access to:

- **Team Information** (from FIRST API)
  - Team name, school, location, rookie year, website
  - All events the team is attending this season

- **Event Data** (from FIRST API)
  - Event schedules, locations, dates
  - All teams attending each event

- **Team Statistics** (from The Blue Alliance API)
  - OPR, DPR, CCWM metrics
  - Component OPRs (auto/teleop/endgame)
  - Rankings and W-L-T records
  - District points breakdown

## How It Works

### On Sign Up

```
User submits signup form with team number
    ↓
User account created in database
    ↓
Background job triggered: SyncTeamForUser(team_number)
    ├─ Fetch team's events from FIRST API
    ├─ Upsert all events and teams
    ├─ Create event_team relationships
    └─ [Background] Fetch TBA stats for all events
            ├─ Get OPR/DPR/CCWM data
            ├─ Get component OPRs
            ├─ Get rankings and records
            └─ Upsert all team stats
    ↓
Redirect to dashboard with data ready
```

### On Login

Same process as signup - ensures data stays fresh on each session.

## Implementation Details

### Main Function: `SyncTeamForUser()`

**Location**: `internal/frc/sync.go`

**What it does**:
1. Takes team number as input
2. Calls FIRST API to get events for team
3. Upserts events, team, and relationships
4. Triggers background TBA stats sync
5. Returns immediately with results

**Code**:
```go
func SyncTeamForUser(ctx context.Context, db *gorm.DB, teamNumber int) (SyncResult, error)
```

### Helper Function: `syncTeamTBAStatsForUser()`

**Location**: `internal/frc/sync.go`

**What it does**:
1. For each event the team attends:
   - Fetches OPR/DPR/CCWM from TBA
   - Fetches component OPRs breakdown
   - Fetches rankings and record
   - Upserts all team stats
   - Handles missing/partial data gracefully

**Called as**: Background goroutine (non-blocking)

**Timeout**: 30 seconds per event

### Integration Points

#### 1. Sign Up Handler (`internal/handlers/auth.go`)

```go
// In HandleSignup():
if parsedTeamNumber != nil {
    // Sync team data from FIRST API for new team member
    go func() {
        ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
        defer cancel()
        
        result, err := frc.SyncTeamForUser(ctx, h.db, *parsedTeamNumber)
        // ...
    }()
}
```

#### 2. Login Handler (`internal/handlers/auth.go`)

```go
// In HandleLogin():
if user.TeamNumber != nil && *user.TeamNumber > 0 {
    go func() {
        ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
        defer cancel()
        _, err := frc.SyncTeamForUser(ctx, h.db, *user.TeamNumber)
        // ...
    }()
}
```

## Data Fetched

### From FIRST Events API
- Team information (name, school, location, rookie year)
- All events team is attending
- All teams at each event
- Event schedules and details

### From The Blue Alliance API
Per team at each event:
- **OPR**: Offensive Power Rating
- **DPR**: Defensive Power Rating
- **CCWM**: Contribution to Winning Margin
- **Component OPRs**: Auto/Teleop/Endgame breakdown
- **Rankings**: Qualification rank and position
- **Record**: Win-Loss-Tie statistics
- **District Points**: Qualification, Elimination, Award, Alliance contribution

## Error Handling

### Graceful Failures
- Missing TBA key → Sync skipped with log message
- TBA API unavailable → Logged, other data still used
- Event lack TBA key → Skipped, other events processed
- Partial data → Stats created with available fields

### Timeouts
- Total operation: 30-60 seconds
- Individual TBA requests: 20 seconds
- Prevents hanging requests

### Logging
All operations logged with:
- ✅ Success messages
- ⚠️ Warnings for partial failures
- ℹ️ Info for skipped operations

Example logs:
```
📡 FIRST team sync starting for team 10101 (season 2026)
✅ FIRST events synced for team 10101: 3
✅ FIRST team sync complete for team 10101: events=3 teams=47 event_teams=141
✅ Synced TBA stats for 47 teams at event 1
```

## Database Impact

### Tables Updated
- **teams**: Team info (name, school, location, nickname)
- **events**: Event schedule (dates, location, TBA key)
- **event_teams**: Team attendance relationships
- **team_event_stats**: TBA statistics (OPR, rankings, records)

### Data Upsert Strategy
- Insert or update if exists
- Preserves historical data
- Updates only changed fields
- Maintains referential integrity

## Configuration Requirements

### Required
- `FIRST_API_USERNAME` - FIRST API credentials
- `FIRST_API_KEY` - FIRST API credentials

### Optional but Recommended
- `TBA_AUTH_KEY` - For TBA stats (leave empty to disable)

### Set via Environment
```bash
export FIRST_API_USERNAME="your_username"
export FIRST_API_KEY="your_key"
export TBA_AUTH_KEY="your_tba_key"
```

## Performance Notes

### Non-Blocking Design
- User redirected immediately after signup
- Data syncs in background
- No user-facing delays

### Background Processing
- Uses goroutines for concurrency
- Context-based timeout management
- 30-second max wait per sync

### Database Efficiency
- Batch inserts via upsert
- No duplicate work (constraint prevents re-syncing)
- Indexes on event_teams and team_event_stats

## Monitoring & Troubleshooting

### Verify Sync Completed
```sql
-- Check team data is loaded
SELECT e.name, COUNT(DISTINCT t.id) as team_count
FROM events e
JOIN event_teams et ON et.event_id = e.id
JOIN teams t ON t.id = et.team_id
WHERE e.name LIKE '%your_event%'
GROUP BY e.id, e.name;

-- Check TBA stats are populated
SELECT COUNT(*) as stats_count
FROM team_event_stats tes
WHERE tes.opr IS NOT NULL;
```

### Common Issues

**Issue**: TBA stats not populating
- **Cause**: TBA_AUTH_KEY not set
- **Fix**: Add to environment and restart app

**Issue**: Some events missing TBA key
- **Cause**: Event not synced properly from FIRST API
- **Fix**: Run `SyncOnBoot` or manual FIRST sync

**Issue**: Timeout errors in logs
- **Cause**: FIRST/TBA API slow or unreachable
- **Fix**: Check network, verify API availability

## Future Enhancements

- [ ] Manual "Refresh Team Data" button
- [ ] Scheduled background refresh for all teams
- [ ] Webhook support for real-time updates
- [ ] Caching layer for frequently accessed data
- [ ] Incremental sync (only changed data)
- [ ] Historical data retention across seasons
