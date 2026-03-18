# Team Stats Display Guide

## Overview

The `/teams` page displays synced event performance data from TBA plus local scouting aggregates.

## How to Use

1. Open `/teams`.
2. Search for a team number.
3. Select an event.
4. Review cards and scouting summaries in the team data panel.

## Stats Sections

### Performance Cards

- Rank
- OPR
- DPR
- CCWM

### Record and Qualification

- W-L-T record
- Matches played
- Qual average
- Average match points
- DQ count

### Component OPRs

- Auto OPR
- Teleop OPR
- Endgame OPR

### Points Breakdown

- Qual points
- Elim points
- Award points
- Alliance points
- Total points

### Scouting Aggregate Fields

- Most common starting position
- Most common defense rating
- Most common traversal
- Most common scoring strategy
- Most common hang level
- Most common auto hang
- Most common hang position
- Most common accuracy rating
- Alliance color distribution

### Notes

- Notes are team-private and filtered by submitting team ownership.

## Data Freshness

- During active events: sync target is every 2 minutes.
- Between events: sync target is every 3 hours.
- Source data quality depends on valid event TBA keys and API availability.

## Troubleshooting

### Missing stats

- Check `TBA_AUTH_KEY` is configured.
- Confirm event has a valid `tba_key`.
- Check server logs for sync warnings.

### Stale stats

- Check last sync logs.
- Trigger FIRST refresh with `POST /api/frc/sync` (admin/lead-scout) for event/team graph updates.
- Verify TBA sync loop is running.
