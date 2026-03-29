# TealTeam Detailed Overview (Source Synthesis for Downstream AI)

## Document Intent

This document is a high-detail source synthesis of TealTeam based on the current repository documentation corpus.
It is designed as an intermediate input for another AI system that will produce a final, human-readable artifact aligned to team initiative documentation guidelines.

This is intentionally dense and comprehensive rather than narrative-polished.

## How to Use This File

- Treat this as a canonical fact pool and system context snapshot.
- Prefer this file for structure and coverage checks.
- Use the source mapping section to trace statements back to originating docs.
- If conflicts arise, prioritize code behavior, then README/runtime docs, then historical notes.

## One-Sentence Definition

TealTeam is an FRC scouting and analytics platform that combines manual scouting submissions with synchronized FIRST and Blue Alliance competition data to support scouts, lead scouts, coaches, and admins with event-specific team intelligence.

## Project Purpose and Outcome Targets

### Primary Purpose

- Capture qualitative scouting observations during events.
- Synchronize official and community competition data from external APIs.
- Aggregate and display team/event performance indicators for strategic decisions.

### Practical Outcomes Supported

- Faster pre-match and in-event strategy decisions.
- Team-level data continuity across event lifecycle stages.
- Role-based workflows for data entry, review, and consumption.
- Better alliance planning through combined quantitative and qualitative signals.

### Problem Space Addressed

- Scouting data is fragmented when only manual or only API-sourced data is used.
- Event operations require rapid refresh during active windows and lower overhead outside those windows.
- Team users need privacy boundaries for notes while sharing common performance baselines.

## Intended Users and Role Context

### Scout

- Submits qualitative observations per team/event context.
- Uses submission workflows with event/team selection and structured form fields.

### Lead Scout

- Reviews pending scouting submissions.
- Approves (promotes into canonical scouting data) or declines entries.

### Coach / Drive Coach

- Consumes team/event insights and match context.
- Uses match status categories and team performance summaries.

### Admin

- Has elevated operations access, including manual sync trigger endpoint.
- Supports environment and data integrity during event operations.

## Scope Boundaries (Current State)

### In Scope

- Event/team sync from FIRST.
- Team stats and match sync from TBA.
- Server-rendered pages and HTMX fragments.
- Session-backed authentication and role-aware handlers.
- Team-private notes visibility constraints.

### Explicitly Retired or Removed

- Auto-path scouting feature is retired and not active in current schema/flows.
- Match prediction UI/logic was removed and documented for potential reimplementation.

## Architecture Summary

### System Style

- Server-rendered web app with progressive enhancement via HTMX.
- Gin router and handler-layer orchestration.
- PostgreSQL as operational store for auth, competition graph, submissions, and stats.
- External API integration with FIRST Events and TBA.

### Runtime Boot Sequence

1. Application starts from cmd/web main entrypoint.
2. Environment and DB URL are resolved by mode (test/prod behavior differs).
3. DB connects and migrations auto-apply.
4. FIRST sync runs on boot unless disabled.
5. TBA background sync starts when auth key is present.
6. HTTP routes (pages, API, HTMX) are served.

### Key Runtime Modes

- test mode (default): local/dev assumptions and migration-history reset behavior.
- prod mode: Render-oriented database URL resolution.

## Technical Stack and Delivery Model

### Core Stack

- Backend: Go + Gin.
- Rendering: html/template and HTMX partial responses.
- Frontend assets: Tailwind CSS and TypeScript.
- Database: PostgreSQL.
- Deployment: Docker runtime and Render blueprint.

### Local and Event Infrastructure

- Local docker-compose stack for development.
- Raspberry Pi compose profile for event operations.
- Service/timer model for auto-start and URL refresh in Pi mode.

## Route and Interaction Model

### Full Page Routes (Representative)

- /
- /submission
- /teams
- /lead-scout
- /drive-coach
- /account
- /sign-in
- /sign-up

### API Routes (Representative)

- auth login/signup/logout
- account password update
- event selection
- manual FIRST sync trigger

### HTMX Fragment Routes (Representative)

- event summary fragments
- team search/data fragments
- match schedule fragments
- lead-scout approve/decline actions

## Core Data Domains

### Identity and Sessions

- users
- sessions

### Competition Graph and Event Context

- events
- teams
- event_teams

### Performance and Match Intelligence

- team_event_stats
- matches
- awards
- zebra-related telemetry table surface is documented in architecture notes

### Scouting Intake and Canonicalized Data

- scouting_submissions
- scouting_data

### Operational Metadata

- schema_migrations

## Data Inputs and Outputs

### Manual Scouting Input Fields (Current)

- event
- team
- alliance color
- starting position
- defense rating
- traversal
- shooting speed
- capacity
- defendability note
- teleop strategy
- hang level
- auto hang
- hang position
- notes

### Synced Metrics from TBA/FIRST

- OPR, DPR, CCWM
- component OPRs (auto/teleop/endgame)
- ranking and W-L-T
- qualification average and average match points
- ranking point family fields (qual/elim/award/alliance/total)
- match schedule and results
- event metadata including timezone when available

### Aggregated Team Page Outputs

- most common values for key scouting dimensions
- alliance color distribution
- privacy-filtered notes set

## Sync and Freshness Strategy

### FIRST Sync

- Runs on startup unless explicitly disabled.
- Can be manually triggered by authorized roles.
- Supports filtering by event code, team number, and country context.

### TBA Background Sync

- Enabled only when TBA auth key exists.
- Active event cadence target: every 2 minutes.
- Inactive window cadence target: every 3 hours.
- Event-level failures are isolated and logged without collapsing the loop.

### Team-Scoped Auth-Time Sync

- Signup/login may trigger team-specific sync path.
- FIRST team event graph sync happens first.
- Asynchronous TBA follow-up runs for that team's event set when key is configured.

## Critical Data and Schema Reliability Notes

### TBA Schema Variability Handling

Fixes documented in repository resolve schema mismatch classes:

- Dynamic component OPR names are now interpreted via map-based parsing and heuristics.
- Ranking points and related values use fallback extraction from arrays where direct fields are null.
- Average match points capture was added as first-class stored metric.
- Match data persistence was added in comprehensive sync script path.

### Practical Impact of Fixes

- Prevented silent metric nulling for component OPRs.
- Restored ranking-related field population for modern TBA schemas.
- Populated matches table where it had previously remained empty in one sync path.
- Improved team page analytical completeness.

## Team Page Behavior and Decision-Support Value

### Search and Event Selection Flow

- Team search resolves canonical team and event options.
- If local event links are missing, system attempts team sync and retries lookup.
- Event selection loads synced stats + approved scouting data for synthesis.

### Displayed Data Families

- Performance cards (rank/OPR/DPR/CCWM).
- W-L-T and qualification profile.
- Component OPR breakdown.
- Points family fields.
- Scouting aggregate insights.
- Team-private notes.

### Privacy Rule

- Competition notes are scoped to the submitting team context.
- Cross-team notes visibility is intentionally blocked.

## Match and Time Semantics

### Match Context Sources

- Match schedule/results from TBA sync.
- Event/team relationship from FIRST sync and DB associations.

### Match Status Classification

- Completed, current/in-progress, upcoming categories are determined via scheduled/actual time windows.

### Timezone Handling

- Events store IANA timezone IDs.
- Time parsing attempts timezone-aware parse first, then local parse with event timezone assignment.
- Display format shows event local time with abbreviation.
- Includes operational scripts/process for timezone backfill and manual correction.

## Deployment and Operations

### Render Production Model

- Dockerized service deployment via render blueprint.
- Production startup command targets prod env mode.
- Managed postgres connection injected by platform env var.
- FIRST/TBA keys are manual secret configuration items.

### Local Development Model

- Dockerized Postgres + local app run.
- Asset build/watch via npm scripts.
- Go run/build and migrations via startup path.

### Raspberry Pi Event Mode

- First-boot automation script prepares environment and runtime.
- Systemd service enables autostart.
- URL refresh timer supports quick discovery checks.
- Boot mode transitions to event-oriented defaults after first successful launch.
- Optional LCD display integration supported, with graceful degradation if unavailable.

## Operational Dependencies and Degradation Behavior

### Required/Important Environment Variables

- database URL
- FIRST API username/key
- TBA auth key
- season and sync behavior flags
- app port

### Degradation Rules

- Missing TBA key: app runs without periodic TBA stats/match sync.
- Missing FIRST credentials: FIRST sync paths skip cleanly.
- DB issues: service may start but DB-backed features degrade.

## Governance, Access, and Security-Adjacent Notes

### Access Control Shape

- Session-backed auth gate for protected actions.
- Manual sync endpoint restricted to elevated roles.

### Data Visibility Boundaries

- Team-private note visibility enforced in team data rendering.

### Credential Hygiene

- Documentation and examples should only use placeholders for API credentials.
- Real keys should never appear in docs, source, or logs.

## Known Historical/Design Notes

### Predictions Feature

- Previously existed in coach/drive-coach surfaces.
- Removed in current state.
- Reimplementation guide exists with former formulas and reintegration steps.

### Auto Path Feature

- Explicitly retired in current codebase and migration chain.
- Reintroduction would require full schema + handler + UI + docs updates.

## Risks, Constraints, and Assumptions for Downstream Documentation

### Risks to Capture

- External API schema drift remains a recurring integration risk.
- Event-time correctness depends on timezone data quality.
- Sync completeness depends on key availability and event key validity.

### Constraints to Preserve

- Team notes privacy boundary is non-negotiable behavior.
- Role-gated operations must stay explicit.
- Cadence choices balance freshness vs operational overhead.

### Assumptions This Summary Uses

- Repository docs reflect intended current behavior unless contradicted by code/runtime.
- Migration chain references in modern docs supersede legacy historical references.

## Suggested Structure for the Next AI (Target Initiative Artifact)

Use this sequence when producing the final readable guideline-aligned document:

1. Mission and strategic objective.
2. User personas and role responsibilities.
3. Platform architecture and integration boundaries.
4. Data lifecycle from intake to decision support.
5. Operational model (startup, sync loops, deployment modes).
6. Privacy, security posture, and governance controls.
7. Reliability and known failure/degradation behaviors.
8. Current exclusions (retired features) and roadmap candidates.
9. Appendix with data fields, key routes, and environment dependencies.

## Source Coverage Matrix

This section verifies that all discovered markdown docs were incorporated.

- README.md: project identity, stack, routes, env, startup, deployment, migration chain.
- ARCHITECTURE.md: architecture narrative, startup flow, route model, domain data map.
- DataPoints.md: scouting input fields, synced data points, aggregate outputs, privacy.
- FRC_API_Calls.md: FIRST API endpoint catalog, scheduling strategy, caching/retry guidance.
- TEAM_PAGE_ANALYSIS.md: /teams flow, dependencies, privacy behavior, aggregate rendering.
- TEAM_STATS_SYNC.md: TBA sync architecture, cadence, startup conditions, error behavior.
- TEAM_STATS_DISPLAY.md: /teams stats display sections, freshness expectations, troubleshooting.
- TBA_SCHEMA_FIX_SUMMARY.md: schema mismatch root causes, fixes, migration and validation outcomes.
- SIGNUP_DATA_SYNC.md: signup/login-triggered team sync and async follow-up behavior.
- PREDICTIONS_REIMPLEMENTATION.md: removed predictions feature and reimplementation details.
- docs/ARCHITECTURE_DIAGRAM.md: UI-server-data-api flow and bootstrap sequence.
- docs/TIMEZONE_HANDLING.md: timezone storage/parsing/display and operational backfill method.
- docs/PI_EVENT_BOOT.md: Raspberry Pi first boot and autostart operational model.
- AUTO_PATH_REMOVAL_RECORD.md: retired auto path scope and reintroduction checklist.
- internal/handlers/MATCH_DETECTION.md: match/event context derivation and status categorization notes.

## Terminology Normalization Notes for Downstream AI

- FIRST means FIRST Events API integration domain.
- TBA means The Blue Alliance API integration domain.
- Team event stats refers to per-team, per-event synchronized performance metrics.
- Scouting submissions are pending intake; scouting data is approved/canonicalized output.

## Final Synthesis Statement

TealTeam is a hybrid scouting intelligence system for FRC operations that deliberately combines controlled human observations, role-based review workflows, and continuously refreshed external competition telemetry into a unified event decision-support surface, while enforcing team note privacy and supporting both cloud deployment and headless on-site event infrastructure.
