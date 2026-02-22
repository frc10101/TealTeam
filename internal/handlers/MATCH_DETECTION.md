// MATCH DETECTION & EVENT FILTERING FEATURE
//
// This document explains the match detection and filtering system implemented
// in the TealTeam scouting application.
//
// ===========================================================================
// FEATURE OVERVIEW
// ===========================================================================
//
// The scouting application now provides intelligent match detection and
// event filtering to streamline the scouting workflow:
//
// 1. MATCH DETECTION
//    - Automatically identifies the current match for scouting
//    - Supports both time-based and team participation verification
//    - Filters matches based on team participation
//
// 2. EVENT FILTERING  
//    - Events are filtered based on the user's team number
//    - Users see only events their team is attending
//    - Admin/coach users without a team see all events
//
// ===========================================================================
// IMPLEMENTATION DETAILS
// ===========================================================================
//
// OPTION 1: TIME-BASED MATCH DETECTION
// =====================================
//
// Function: findCurrentMatchForTeam()
// Strategy: Compares the match's scheduled_time (from API/database) with the
//           current server time to determine which match is active
//
// Logic Flow:
// 1. Query all matches where the team is participating
// 2. Compare each match's scheduled_time to current time
// 3. Find match where: (scheduled_time - 5min) < now < (scheduled_time + 15min)
// 4. If no match is currently active, return the next upcoming match
//
// Match Window: ±15 minutes from scheduled_time
// - Match is considered "in progress" if current time falls within this window
// - 5 minutes before to 15 minutes after scheduled time
//
// OPTION 2: TEAM PARTICIPATION VERIFICATION
// ===========================================
//
// Function: TeamInMatch()
// Strategy: Validates that a team is actually participating in the selected match
//           by querying the match's alliance composition
//
// Logic Flow:
// 1. Query scouting_data table for match-team relationships
// 2. Verify the team exists in the match's red or blue alliance
// 3. Return the team's actual alliance color (red/blue)
//
// Data Source:
// - scouting_data table stores the alliance color for each team in each match
// - This is populated during match schedule sync
//
// ===========================================================================
// MATCH FILTERING BY TEAM
// ===========================================================================
//
// Function: GetMatchesForTeam()
// Returns: All matches where the team participates, with status info
//
// Join Strategy:
//   matches
//   ├── JOIN scouting_data (match participation)
//   └── Team information from scouting_data.team_id
//
// Results include:
// - match_number, match_type (qualification, playoff, etc.)
// - scheduled_time, actual_time
// - red_score, blue_score, played status
// - team's alliance_color
//
// ===========================================================================
// EVENT FILTERING BY TEAM
// ===========================================================================
//
// Function: buildSubmissionPageData()
// Modified to: Filter events based on user's team number
//
// Join Strategy (if user has team):
//   events
//   ├── JOIN event_teams (team registration at event)
//   └── JOIN teams (team details)
//   └── WHERE teams.team_number = ?
//
// Result:
// - Users only see events their team is attending
// - Admin/coach users without a team see all events
// - Events are ordered by start_date
//
// ===========================================================================
// MATCH STATUS CATEGORIZATION
// ===========================================================================
//
// Function: GetMatchesForTeamByStatus()
// Returns: Matches organized by status for better UI presentation
//
// Status Categories:
//
// COMPLETED:
// ├── Match has been played (played = true)
// └── OR current time is > (scheduled_time + 15 minutes)
//
// IN_PROGRESS:
// ├── (scheduled_time - 5 min) < now < (scheduled_time + 15 min)
// └── AND match not yet marked as played
//
// UPCOMING:
// ├── now < (scheduled_time - 5 min)
// └── OR scheduled_time is NULL
//
// ===========================================================================
// WORKFLOW INTEGRATION
// ===========================================================================
//
// Scouting Submission Flow:
//
// 1. User selects event
//    └─ Only events their team attends are shown
//
// 2. User selects team (their team if applicable)
//    └─ Pre-populated if user has a team_number
//
// 3. System automatically determines current match
//    ├─ Calls findCurrentMatchForTeam()
//    └─ Returns best match based on timing
//
// 4. System validates team participation
//    ├─ Calls TeamInMatch()
//    └─ Confirms team is in this match
//
// 5. System retrieves actual alliance color
//    ├─ From match participation data
//    └─ User's color preference is validated/overridden
//
// 6. Submission is queued with correct match data
//    └─ Scouting data is associated with correct team/match/alliance
//
// ===========================================================================
// DATABASE QUERIES
// ===========================================================================
//
// Key tables and relationships:
//
//   matches
//   ├── id (primary key)
//   ├── event_id (foreign key → events)
//   ├── match_number
//   ├── match_type (qualification, playoff, etc.)
//   ├── scheduled_time (TIMESTAMP WITH TIME ZONE)
//   ├── actual_time (TIMESTAMP WITH TIME ZONE, NULL if not played)
//   └── played (boolean)
//
//   scouting_data
//   ├── match_id (foreign key → matches)
//   ├── team_id (foreign key → teams)
//   ├── alliance_color (red or blue)
//   ├── starting_position
//   └── various scouting metrics
//
//   event_teams (Many-to-Many)
//   ├── event_id (foreign key → events)
//   └── team_id (foreign key → teams)
//
//   users
//   ├── id (primary key)
//   ├── team_number (optional, INT nullable)
//   ├── email
//   └── various user settings
//
//   events
//   ├── id (primary key)
//   ├── name
//   ├── start_date
//   └── end_date
//
// ===========================================================================
// FUTURE ENHANCEMENTS
// ===========================================================================
//
// 1. Blue Alliance API Integration
//    - Fetch match times automatically
//    - Real-time match status updates
//    - Timezone handling for events across regions
//
// 2. Match Prediction
//    - Suggest likeliest upcoming match based on:
//      - Team's typical match times
//      - Competition schedule patterns
//      - Historical scouting patterns
//
// 3. Multi-Team Support
//    - Allow users to scout for multiple teams
//    - Team switching during event
//    - Role-based match visibility
//
// 4. Queue Management
//    - Persistent scouting queue
//    - Re-queue failed submissions
//    - Batch match assignments
//
// ===========================================================================
