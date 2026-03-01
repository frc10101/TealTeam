# Team Statistics Syncing System

## Overview

The TealTeam application now includes an automated background service that keeps team statistics synchronized with The Blue Alliance (TBA) API. This ensures your database always has the most current:

- **OPR** (Offensive Power Rating)
- **DPR** (Defensive Power Rating) 
- **CCWM** (Calculated Contribution to Winning Margin)
- **Component OPRs** (Auto, Teleop, Endgame breakdowns)
- **Team Rankings** (Rank, W-L-T record, qualification scores)
- **District Points** (Qualification, Elimination, Award, Alliance points)

## Architecture

### Components

#### 1. **TBA Client** (`internal/frc/tba_client.go`)
- RESTful client for The Blue Alliance API v3
- Handles authentication via `X-TBA-Auth-Key` header
- Provides methods for fetching OPR data, rankings, and event information

#### 2. **Team Stats Syncer** (`internal/frc/team_stats_sync.go`)
- Background goroutine that orchestrates periodic syncing
- Smart scheduling: 
  - **2 minutes** during active events
  - **3 hours** between events
- Automatically detects active/upcoming events
- Handles API retries and timeouts

#### 3. **Database Integration**
- Synced data stored in `team_event_stats` table
- Upsert operations prevent duplicates
- Updated `TeamEventStats` model includes all relevant fields

### Smart Scheduling

The syncer intelligently determines when to sync based on event status:

1. **During Active Events**: Updates every 2 minutes (matches happening now)
2. **Before Events**: Updates every 2 minutes if events start within 24 hours
3. **Between Events**: Updates every 3 hours (falls back to scanning historical/upcoming events)

This approach minimizes API calls while ensuring data freshness during competition.

## Configuration

### Environment Variables

Add these to your `.env` file:

```bash
# The Blue Alliance Auth Key (REQUIRED for syncing)
TBA_AUTH_KEY="your-auth-key-here"

# Optional: Adjust sync intervals (in code, values are hardcoded)
# IntervalDuringEvent: 2 * time.Minute
# IntervalBetweenEvent: 3 * time.Hour
```

### Getting Your TBA Auth Key

1. Visit [The Blue Alliance - Account Page](https://www.thebluealliance.com/account)
2. Sign in with your TBA account
3. Copy your API Auth Key
4. Set it in your environment: `export TBA_AUTH_KEY="your-key"`

## Implementation Details

### Initialization Flow

```
main.go
  ├─ Load configuration with LoadSyncConfig()
  ├─ Create syncer with NewTeamStatsSyncer()
  ├─ Start background loop with syncer.Start()
  └─ Stop on shutdown with syncer.Stop()
```

### Sync Process

```
SyncLoop (every 2-180 minutes depending on event status)
  ├─ DetermineSyncInterval()
  │   └─ Check if events active/upcoming (24h window)
  ├─ SyncAllTeamStats()
  │   └─ For each active event:
  │       ├─ GetEventOPRs() - Fetch OPR/DPR/CCWM
  │       ├─ GetEventComponentOPRs() - Fetch component breakdowns
  │       ├─ GetEventRankings() - Fetch team rankings
  │       └─ Upsert stats to database
  └─ reschedule based on new interval
```

## Database Changes

### New Fields Added to `team_event_stats`

The migration includes these columns (already in schema):

```sql
CREATE TABLE team_event_stats (
    ...
    qual_points INTEGER,           -- Qualification points
    elim_points INTEGER,           -- Elimination points  
    award_points INTEGER,          -- Award points
    alliance_points INTEGER,       -- Alliance contribution points
    total_points INTEGER,          -- Total district points
    ...
);
```

### Upserted Data

On conflict (duplicate team_id, event_id):
- Updates OPR, DPR, CCWM
- Updates component OPRs
- Updates ranking info
- Updates timestamp
- Preserves historical data (INSERT only fails, UPDATE succeeds)

## API Data Fetched Per Team Per Event

From `GET /event/{event_key}/oprs`:
```json
{
  "oprs": { "frc1234": 45.23, ... },
  "dprs": { "frc1234": 12.45, ... },
  "ccwms": { "frc1234": 32.78, ... }
}
```

From `GET /event/{event_key}/coprs`:
```json
{
  "auto_opr": { "frc1234": 8.5, ... },
  "teleop_opr": { "frc1234": 25.1, ... },
  "endgame_opr": { "frc1234": 11.6, ... }
}
```

From `GET /event/{event_key}/rankings`:
```json
{
  "rankings": [
    {
      "team_key": "frc1234",
      "rank": 1,
      "matches_played": 14,
      "qual_average": 87.5,
      "record": { "wins": 12, "losses": 2, "ties": 0 },
      "dq": 0,
      "qual_points": 140,
      "elim_points": 20,
      "award_points": 5,
      "alliance_points": 10,
      "total_points": 175
    },
    ...
  ]
}
```

## Error Handling

- **Network failures**: Logged with warning, retried on next cycle
- **Missing TBA key**: Sync disabled with warning on startup
- **Partial failures**: One team's failure doesn't block others
- **Timeout**: 2-minute timeout per sync cycle prevents hanging

## Monitoring

### Logs

Watch for these in your application logs:

```
✅ Synced team stats for 40 teams at event 1
📊 1 active events found, using fast sync interval (2m0s)
⚠️  Failed to fetch OPR data for event 2024xx_xyz: 404 not found
🔄 Team stats sync loop started
🛑 Team stats sync loop stopped
```

### Database Query

Check synced data:

```sql
SELECT e.name, t.team_number, stats.opr, stats.dpr, stats.rank
FROM team_event_stats stats
JOIN teams t ON t.id = stats.team_id
JOIN events e ON e.id = stats.event_id
WHERE e.id = 1
ORDER BY stats.rank;
```

## Customization

### Adjusting Sync Intervals

Edit `internal/frc/team_stats_sync.go`:

```go
SyncConfig{
    TBAAuthKey:           os.Getenv("TBA_AUTH_KEY"),
    IntervalDuringEvent:  2 * time.Minute,        // Change here
    IntervalBetweenEvent: 3 * time.Hour,          // Change here
}
```

### Adding More Data Points

The system is extensible. To add new fields:

1. Add to `team_event_stats` table schema
2. Add to `TeamEventStats` struct in models.go
3. Extract from TBA API response in `SyncTeamStatsForEvent()`
4. Update upsert clause with new field names

## Troubleshooting

### "Team stats sync disabled" warning

**Problem**: Sync not running on startup
**Solution**: Set `TBA_AUTH_KEY` environment variable

### Old data persisting

**Problem**: Stats not updating despite sync running
**Solution**: 
- Verify `TBA_AUTH_KEY` is valid (test at https://www.thebluealliance.com/api/v3/team/frc1690)
- Check logs for API errors
- Verify events have `tba_key` field set (from FIRST sync)

### High API usage

**Problem**: Too many API calls
**Solution**: Increase `IntervalBetweenEvent` or implement request caching

## Dependencies

- `gorm.io/gorm` - Database operations
- `net/http` - HTTP client for TBA API
- `encoding/json` - JSON parsing

## Future Enhancements

Potential additions:

- [ ] Cache TBA responses with ETag support (conditional requests)
- [ ] Store raw API responses for historical analysis
- [ ] Add Zebra MotionWorks tracking data import
- [ ] Implement Redis caching for frequently accessed stats
- [ ] Create API endpoint to trigger manual sync
- [ ] Add Prometheus metrics for sync performance
- [ ] Support for multiple seasons/years

## References

- [The Blue Alliance API Docs](https://www.thebluealliance.com/api/v3/docs)
- [Team Statistics and OPR Explained](https://www.thebluealliance.com/frc/about)
- [FRC Game Analysis Guide](https://www.chiefdelphi.com/)
