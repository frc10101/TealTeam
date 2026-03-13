# Predictions Analysis — Reimplementation Guide

This document captures the removed predictions feature and the steps to reimplement it.

## What Was Removed

Match score predictions based on OPR/DPR stats, displayed in both the Coach Viewer and Drive Coach Dashboard.

### Affected Files

1. **`internal/handlers/coach.go`**
   - `driveCoachMatch` struct had `PredictedRedScore float64` and `PredictedBlueScore float64` fields
   - `predictAllianceScore(ourOPRSum, opponentDPRSum float64) float64` — averaged OPR sum with opponent DPR sum: `(ourOPRSum + opponentDPRSum) / 2`
   - `loadDriveCoachMatches()` accumulated per-alliance OPR/DPR sums and called `predictAllianceScore()` for each match

2. **`internal/handlers/drive_coach.go`**
   - `DriveCoachMatch` struct had `PredictedRedScore float64`, `PredictedBlueScore float64`, `PredictedWinner string`
   - `calculateAllianceScore(teams []TeamWithStats) float64` — summed OPR values for all teams in the alliance (returns 0 if no OPR data)
   - `getDriveCoachMatches()` called `calculateAllianceScore()` for unplayed matches and set `PredictedWinner` based on which score was higher

3. **`web/templates/partials/drive_coach_matches.html`**
   - Displayed a "Prediction" box with `Red {{printf "%.1f" .PredictedRedScore}}` and `Blue {{printf "%.1f" .PredictedBlueScore}}`

## Algorithm Details

### Coach Viewer (`coach.go`) — FIRST API Path
```
predictAllianceScore(ourOPRSum, opponentDPRSum) = (ourOPRSum + opponentDPRSum) / 2
```
- Accumulated OPR and DPR for each alliance from team stats joined against `team_event_stats`
- Red predicted = predictAllianceScore(redOPRSum, blueDPRSum)
- Blue predicted = predictAllianceScore(blueOPRSum, redDPRSum)

### Drive Coach (`drive_coach.go`) — TBA Path
```
calculateAllianceScore(teams) = sum of OPR for all teams where OPR > 0
```
- Simpler approach: just sum the OPRs (OPR already represents expected point contribution)
- After calculating both scores, set PredictedWinner to "red", "blue", or "tie"

## Steps to Reimplement

### 1. Add prediction fields back to structs

In `internal/handlers/coach.go`, add to `driveCoachMatch`:
```go
PredictedRedScore  float64
PredictedBlueScore float64
```

In `internal/handlers/drive_coach.go`, add to `DriveCoachMatch`:
```go
PredictedRedScore  float64
PredictedBlueScore float64
PredictedWinner    string // "red", "blue", or "tie"
```

### 2. Restore prediction functions

In `coach.go`, add:
```go
func predictAllianceScore(ourOPRSum, opponentDPRSum float64) float64 {
    return (ourOPRSum + opponentDPRSum) / 2
}
```

In `drive_coach.go`, add:
```go
func calculateAllianceScore(teams []TeamWithStats) float64 {
    totalOPR := 0.0
    count := 0
    for _, team := range teams {
        if team.OPR != nil && *team.OPR > 0 {
            totalOPR += *team.OPR
            count++
        }
    }
    if count == 0 {
        return 0
    }
    return totalOPR
}
```

### 3. Call prediction functions in match-building loops

In `coach.go` `loadDriveCoachMatches()`, after assembling alliance teams:
```go
// Accumulate redOPRSum, redDPRSum, blueOPRSum, blueDPRSum from team stats
entry.PredictedRedScore = predictAllianceScore(redOPRSum, blueDPRSum)
entry.PredictedBlueScore = predictAllianceScore(blueOPRSum, redDPRSum)
```

In `drive_coach.go` `getDriveCoachMatches()`, after populating alliance rosters for unplayed matches:
```go
if !match.Played {
    dcMatch.PredictedRedScore = calculateAllianceScore(dcMatch.RedTeams)
    dcMatch.PredictedBlueScore = calculateAllianceScore(dcMatch.BlueTeams)
    if dcMatch.PredictedRedScore > dcMatch.PredictedBlueScore {
        dcMatch.PredictedWinner = "red"
    } else if dcMatch.PredictedBlueScore > dcMatch.PredictedRedScore {
        dcMatch.PredictedWinner = "blue"
    } else {
        dcMatch.PredictedWinner = "tie"
    }
}
```

### 4. Restore template display

In `web/templates/partials/drive_coach_matches.html`, add inside the match card header area:
```html
<div class="rounded-lg border border-gray-700 bg-gray-900/60 px-3 py-2 text-right">
    <div class="text-xs uppercase tracking-wide text-gray-500">Prediction</div>
    <div class="mt-1 text-sm font-semibold text-red-300">Red {{printf "%.1f" .PredictedRedScore}}</div>
    <div class="text-sm font-semibold text-blue-300">Blue {{printf "%.1f" .PredictedBlueScore}}</div>
</div>
```

### 5. Future improvements to consider

- Incorporate DPR into the Drive Coach prediction (currently only uses OPR)
- Unify the two prediction algorithms (coach.go vs drive_coach.go use different formulas)
- Add win probability percentage instead of just raw scores
- Use component OPRs for more granular predictions
- Consider match history / recent form weighting
