# Team Stats Display Guide

## Overview

The Teams page now displays comprehensive TBA (The Blue Alliance) statistics for each team at selected events, automatically synced every 2 minutes during events.

## How to Access

### 1. Navigate to Teams Page
```
/teams
```

### 2. Search for a Team
- Enter team number (e.g., `10101`)
- Click "Search Team"

### 3. Select Event
- Choose an event from the dropdown
- Team stats automatically load

## What's Displayed

### Primary Performance Metrics (Top Row)

| Metric | What it means | Color |
|--------|---------------|-------|
| **Rank** | Team's placement at the event | Yellow |
| **OPR** | Offensive Power Rating - expected points contributed | Teal |
| **DPR** | Defensive Power Rating - points prevented | Orange |
| **CCWM** | Calculated Contribution to Winning Margin (OPR - DPR) | Green |

### Record & Participation (Second Row)

| Metric | What it means |
|--------|---------------|
| **Record** | Win-Loss-Tie record (W-L-T format) |
| **Matches Played** | Total matches in qualification |
| **Qual Average** | Average score per qualification match |
| **Disqualifications** | Number of DQ events (red if > 0) |

### Component OPRs (Breakdown by Phase)

Shows how much the team contributes in each phase:
- **Auto OPR** (yellow) - Autonomous period contributions
- **Teleop OPR** (blue) - Driver-controlled period contributions  
- **Endgame OPR** (purple) - Endgame phase contributions

### District Points (Ranking Points in Multi-Event Districts)

- **Qual Points** - Points from qualification rounds
- **Elim Points** - Points from playoff matches
- **Award Points** - Points from awards (team character, etc.)
- **Alliance Points** - Points from playoff alliance contribution
- **Total Points** - Sum of all district points (highlighted in teal)

## Column Organization

```
┌─────────────────────────────────────────────────────────────┐
│              Primary Metrics (4 columns)                    │
│  Rank  │   OPR   │   DPR   │   CCWM                         │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│           Record & Participation (4 columns)                │
│ Record │ Matches │ Avg Qual │ Disqualifications            │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│           Component OPRs (3 columns)                        │
│ Auto OPR │ Teleop OPR │ Endgame OPR                        │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│         District Points (5 columns)                         │
│ Qual │ Elim │ Awards │ Alliance │ Total (highlighted)      │
└─────────────────────────────────────────────────────────────┘
```

## Data Freshness

- **Last Updated**: Displayed at bottom of stats section
- **Update Frequency**: 
  - Every 2 minutes during events
  - Every 3 hours between events
- **Source**: The Blue Alliance (TBA) API

## Understanding the Metrics

### OPR (Offensive Power Rating)
- Predicted points a team contributes per match
- Higher is better
- Calculated by TBA using statistical analysis

### DPR (Defensive Power Rating)  
- predicted points a team prevents per match
- Lower is better (negative means they help opponents score less)
- Defensive capability indicator

### CCWM (Contribution to Winning Margin)
- OPR - DPR = net value to winning
- The true measure of a team's impact on match outcomes
- Higher is better (positive is good)

### Component OPRs
- Breakdown of OPR by game phase
- Helps identify team strengths/weaknesses
- Example: High Auto OPR = strong autonomous routine

### District Points
- Used for ranking in multi-event districts
- Cumulative across season events
- Higher totals indicate consistent performance

## Contextual Data Above

The page also shows **Scouted Performance** from your team's scouting submissions:
- Average scores (Auto, Teleop, Endgame)
- Starting positions
- Most common strategies
- Accuracy ratings (Low / Medium / High)
- Recent match history

### Notes (Team-Private)
- Notes submitted by your team are displayed in the "Notes from Competition" section
- **Notes are private to your team** — other teams cannot see notes your scouts wrote
- Notes are sorted by most recent first, with match index and timestamp

### Past Events
- Use the "Fetch Past Events" button on the team info panel to sync historical event data from the FIRST API
- Allows viewing team performance at previous competitions

## Responsive Design

- **Mobile** (< 768px): Stats stack vertically, metrics in 2-column grid
- **Tablet** (768px - 1024px): 4-column grid for primary metrics
- **Desktop** (> 1024px): Full 4-5 column layouts with optimal spacing

## Color Legend

| Color | Meaning |
|-------|---------|
| Teal | Positive/Strong | 
| Green | Excellent/Net Positive |
| Yellow | Neutral/Rank |
| Orange | Defensive/Opposite of Offensive |
| Blue | Informational |
| Purple | Endgame/Late Phase |
| Red | Warning/Issues (DQ) |

## Example Interpretation

```
Team 10101 at Event XYZ
Rank: 12 | OPR: 45.23 | DPR: 8.15 | CCWM: 37.08
Record: 9-3-0 | Matches: 12 | Avg: 87.5

Component OPRs:
Auto: 8.5 | Teleop: 25.1 | Endgame: 11.6

Interpretation:
- Strong team (#12 rank)
- High offensive output (45.23 OPR)
- Solid defensive capability (8.15 DPR)
- Excellent match outcomes (37 point net contribution)
- Great auto routine, solid endgame
```

## Troubleshooting

### Stats show "N/A" or not displayed
- Ensure event has TBA key set (from FIRST API sync)
- Check that syncer is running (logs should show sync messages)
- Verify TBA_AUTH_KEY environment variable is set

### Data seems stale
- Check "Last updated" timestamp
- If older than expected interval:
  - Event may have concluded (switched to slow sync)
  - Check application logs for sync errors
  - Verify TBA API availability

### Missing Component OPRs
- Not all events have component OPR data
- Fall back to main OPR/DPR metrics
- Component data requires more detailed match records
