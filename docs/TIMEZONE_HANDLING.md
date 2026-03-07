# Event Timezone Handling

## Overview

FRC events happen across different time zones (Pacific, Mountain, Central, Eastern, etc.). To display accurate match times and status:

1. **Database stores timezone** - Each event has an IANA timezone identifier (e.g., `America/Los_Angeles`)
2. **API times are parsed correctly** - FIRST API returns times in event's local timezone (without offset)
3. **Display shows event time** - Match times shown in the event's timezone with zone abbreviation

## Setup

### 1. Run Migration

Apply the database migration to add timezone column:

```bash
# Via psql
psql $DATABASE_URL -f migrations/0006_add_event_timezone.sql

# Or via Docker
docker-compose exec web make migrate
```

### 2. Populate Timezones

Run the script to infer timezones from event locations:

```bash
# Set DATABASE_URL
export DATABASE_URL="postgres://user:pass@localhost:5432/teal_team?sslmode=disable"

# Run the script
go run cmd/scripts/set_event_timezones/main.go
```

The script will:
- Find all events without timezone
- Parse location string (e.g., "San Jose, CA USA")
- Extract state/province abbreviation  
- Map to IANA timezone identifier
- Update the database

### 3. Manual Updates

For events the script can't handle, update manually:

```sql
UPDATE events 
SET timezone = 'America/Los_Angeles' 
WHERE name = 'Silicon Valley Regional';
```

## How It Works

### Time Parsing (coach.go)

```go
// 1. Try RFC3339 format first (includes timezone)
startTime, err = time.Parse(time.RFC3339, "2026-02-21T13:30:00-08:00")

// 2. Fallback: parse without timezone, apply event's timezone
if err != nil {
    startTime, err = time.Parse("2006-01-02T15:04:05", "2026-02-21T13:30:00")
    if err == nil {
        loc, _ := time.LoadLocation(event.Timezone) // e.g., "America/Los_Angeles"
        startTime = time.Date(..., loc)
    }
}

// 3. Display in event's timezone
entry.TimeDisplay = startTime.Format("Mon Jan 2, 3:04 PM MST")
// Example: "Fri Feb 21, 1:30 PM PST"
```

### Status Detection

Match status is based on **actual time**, not server's local time:

- **Completed**: Started more than 15 minutes ago
- **Current**: Within ±15 minutes of start time  
- **Upcoming**: Starts in more than 15 minutes

## Timezone Reference

Common FRC regions:

| Region | States | IANA Timezone |
|--------|--------|---------------|
| Eastern | NY, PA, MI, OH, FL, GA, MA, etc. | `America/New_York` |
| Central | IL, TX, MN, WI, MO, etc. | `America/Chicago` |
| Mountain | CO, UT, MT, WY | `America/Denver` |
| Arizona | AZ | `America/Phoenix` (no DST) |
| Pacific | CA, WA, OR, NV | `America/Los_Angeles` |
| Alaska | AK | `America/Anchorage` |
| Hawaii | HI | `Pacific/Honolulu` |
| Ontario | ON | `America/Toronto` |
| Quebec | QC | `America/Montreal` |
| British Columbia | BC | `America/Vancouver` |

## Troubleshooting

### Events show wrong timezone

Check the stored timezone:

```sql
SELECT name, location, timezone FROM events WHERE id = 123;
```

Update if incorrect:

```sql
UPDATE events SET timezone = 'America/Chicago' WHERE id = 123;
```

### Times still show incorrectly

1. **Check Docker timezone** - Ensure container has timezone data:
   ```bash
   docker-compose exec web date
   ```

2. **Verify timezone database** - Ensure Go can load timezones:
   ```bash
   docker-compose exec web ls -al /usr/share/zoneinfo/America/
   ```

3. **Check logs** - Look for "Invalid timezone" warnings:
   ```bash
   docker-compose logs web | grep timezone
   ```

## Testing

Update test data to include timezones:

```go
event := Event{
    Name:     "Silicon Valley Regional",
    Location: stringPtr("San Jose, CA USA"),
    Timezone: stringPtr("America/Los_Angeles"),
}
```

Run tests:

```bash
go test ./internal/handlers/... -v
```
