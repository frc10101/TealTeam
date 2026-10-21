# FTC BIOBUZZ migration and beta-test playbook

This document is a handoff for a future AI agent that needs to adapt TealTeam
from its current FRC implementation to an FTC season, specifically BIOBUZZ.
It describes the boundary between a new season scouting schema and the
competition-platform changes that a schema alone cannot solve.

The goal of the first FTC release is a safe qualifier beta: scouts can record
observations, work through assignments, review observations, and build a pick
list without inventing FRC statistics or depending on an unverified FTC API.
Full FTC upstream synchronization is a separate phase.

## Read these files first

Before changing code, read:

1. `README.md`
2. `docs/ACTION_ITEMS.md`
3. `docs/REBUILD_SPEC.md`
4. `crates/tt-core/src/season.rs`
5. `crates/tt-core/src/records.rs`
6. `crates/tt-core/src/upstream.rs`
7. `crates/tt-upstream/src/first.rs`
8. `crates/tt-upstream/src/tba.rs`
9. `crates/tt-web/src/scouting.rs`
10. `crates/tt-repo/src/lib.rs`

Do not assume that a field in the rulebook belongs in the scouting form. The
rulebook defines legal actions and official scoring; the team decides what a
scout can reliably observe while watching a match.

## Current boundary: what is reusable

The season-driven form system is deliberately generic:

- `seasons/*.json` declares sections and fields.
- `select`, `counter`, `toggle`, and `text` fields are rendered generically.
- `tt-core::form` parses and validates submitted answers.
- `SeasonSchema::score` applies optional point values.
- Observations are stored as a JSON payload with a schema version.
- Drafts, review, offline handoff, and record idempotency do not need to know
  the names of the game-specific fields.

An FTC schema can therefore reuse the form machinery if it stays within the
existing field types. A schema change by itself is not enough to make the rest
of the application FTC-compatible.

## Current FRC assumptions that must be removed or isolated

### Match alliance size

`MatchRecord` currently stores three teams on each alliance:

```rust
pub red: [Option<i32>; 3],
pub blue: [Option<i32>; 3],
```

FTC qualification matches use two teams per alliance. The migration must not
silently put FTC data into a six-team structure. That would corrupt picker,
assignment, coverage, partner, and display behavior.

Preferred design:

```rust
pub struct Alliance {
    pub teams: Vec<Option<i32>>,
}

pub struct MatchRecord {
    pub red: Alliance,
    pub blue: Alliance,
    // ...
}
```

If a smaller change is safer for the first beta, make the slot count explicit
in the match record and support both fixed sizes behind a program-specific
constructor. Do not scatter `2` and `3` literals through templates and
handlers.

Search for and review every use of:

- `[Option<i32>; 3]`
- `six robots`
- `red` and `blue` slot iteration
- `partners_of`
- `alliance_of`
- assignment coverage
- match picker and keypad
- fixtures that create six-team matches

### Event and team identity

The repository calls the event natural key `tba_key`, and the upstream code
uses FRC event codes and TBA keys. FTC identifiers may come from a different
provider and may not be TBA keys.

For a durable design, introduce provider-neutral identity:

```text
events:
  program              FRC | FTC
  season               integer
  external_provider    FIRST | TBA | FTC_PROVIDER | MANUAL
  external_event_key   text
```

A short beta may continue using the existing event key column if the event is
seeded manually, but the code must not label a manually chosen FTC key as a
TBA key in user-facing behavior or future integrations.

Numeric team numbers are likely reusable. The metadata mapping, event
participation source, and upstream response shapes are not automatically
reusable.

### FRC-only statistics

The current upstream and analytics code contains FRC concepts including:

- OPR
- DPR
- CCWM
- component OPR
- TBA rankings arrays
- FRC match result shapes
- FRC event and team endpoints

Do not populate those columns with guessed FTC equivalents. For an FTC beta,
hide or disable FRC-only pages and calculations unless an FTC-specific,
validated source exists.

### FRC upstream clients

`tt-upstream/src/first.rs` is an FRC Events API client and
`tt-upstream/src/tba.rs` is a Blue Alliance client. Neither should be pointed
at an FTC endpoint by changing a base URL.

Use an adapter boundary:

```text
FTC provider response
        |
        v
     FtcClient
        |
        v
 Event / Team / MatchRecord
        |
        v
 existing repository, assignments, scouting, and offline code
```

The adapter must be tested against recorded FTC responses before it is enabled
at an event.

## Designing `BIOBUZZ.json`

Create a draft outside the active embedded schema first, for example:

```text
crates/tt-core/seasons/2026-ftc-biobuzz.draft.json
```

Do not overwrite the current FRC schema until the program boundary has been
implemented or the beta is explicitly running as an FTC-only build.

The final schema must use the existing `SeasonSchema` format:

```json
{
  "season": 2026,
  "version": 1,
  "name": "BIOBUZZ",
  "sections": [
    {
      "key": "match",
      "label": "Match context",
      "fields": []
    }
  ]
}
```

### Recommended field categories

Use fields that a scout can observe quickly and consistently:

- Match context and starting area
- Autonomous path or routine
- Autonomous successful scores
- Autonomous failures
- TeleOp successful scores by meaningful game element or location
- Intake, transfer, or cycle failures
- Defense experienced and response to defense
- Endgame attempt and result
- Tipped, disabled, disconnected, or mechanically failed
- Driver control and field mobility
- Short private notes

Prefer separate counters when two game elements have different strategic value.
Prefer a select when the scout must classify one outcome. Avoid making a
rulebook term into a field unless a scout can recognize it from the stands.

### Fields that require special review

Do not copy these FRC fields without redesign:

- `starting_position`: BIOBUZZ locations and legal starting areas may differ.
- `auto_scored`: split by element or action if a single count hides strategy.
- `teleop_scored`: split by meaningful scoring categories if needed.
- `endgame`: use BIOBUZZ-specific outcomes, not `none/partial/full` by habit.
- `defense_rating`: define observable FTC effects, not an FRC scale.
- `penalties`: record observed incidents only until official penalty semantics
  are confirmed; do not assign guessed negative points.
- `no_show`: distinguish no-show, no movement, disable, tip, disconnect, and
  mechanical failure where those distinctions affect alliance selection.
- fixed counter `max` values: derive them from match duration and game
  mechanics, then test them with realistic match recordings.

Start with point values of zero for uncertain metrics. A beta should collect
good observations before it claims to have a correct ranking formula.

### Schema review checklist

For every field, record the answer to these questions in the agent's work log
or pull request description:

| Question | Required answer |
| --- | --- |
| What does the field measure? | A concrete observable action or result |
| Where is it supported? | Rulebook section, team strategy decision, or both |
| Can a scout enter it during a match? | Yes, without pausing or guessing |
| Is it independent of official score data? | Explain any overlap |
| What are valid bounds/options? | Enumerated and justified |
| Should it affect ranking? | Yes, no, or defer |
| What happens when it is unknown? | Omitted, zero, false, or explicit unknown |

## Recommended implementation phases

### Phase 0: establish a safe branch and baseline

1. Confirm the current branch and cleanly record unrelated worktree changes.
2. Run the existing verification command (`./check.sh` or the repository's
   current documented equivalent).
3. Save the baseline test result.
4. Do not replace FRC fixtures or the active season file yet.

### Phase 1: add program-aware domain data

Add the smallest durable representation that prevents FRC and FTC data from
being confused:

- program (`frc` or `ftc`)
- season
- event provider
- external event key
- alliance slot count or a variable-size alliance

Update serialization, SQLite migrations, repository methods, fixtures, and
offline snapshot/bundle formats together. Preserve existing FRC behavior.

Do not use `as any`, silent defaults, or a fallback that turns an FTC event
into an FRC event. Invalid program or slot data must produce an explicit error.

### Phase 2: support FTC match shape

Update, test, and review all code that assumes six robots:

- `MatchRecord`
- match labels and display
- `teams()`
- `partners_of`
- `alliance_of`
- assignment generation
- coverage and recorded-observation checks
- keypad and match picker
- templates
- browser tests
- QR/outbox/snapshot serialization

Add fixtures for:

- an FTC qualification match with four teams
- an unplayed match
- a played match with scores
- a missing or replacement team slot
- an invalid three-team FTC alliance, if the model should reject it

### Phase 3: add manual qualifier seeding

This is the recommended first beta path.

Provide a clearly named import or seed path that accepts:

```text
event metadata
team roster
match schedule
optional scores/results
```

The import should validate:

- event program is FTC
- every match belongs to the event
- team numbers are positive and unique within a roster
- each alliance has the expected number of slots
- match keys are unique
- match numbers and competition levels are valid
- timestamps include the intended event timezone

The import must report every rejected row. It must not silently drop a team or
match and continue with a success-shaped result.

### Phase 4: add the BIOBUZZ schema and form tests

Only after the form design is approved:

1. Add the final schema file.
2. Change the active-season loading mechanism so program and season select the
   correct schema, or explicitly configure an FTC-only beta build.
3. Add tests that parse the schema.
4. Add a realistic payload test.
5. Add invalid select, counter, required-field, and stale-schema tests.
6. Render the form at laptop width (scouts use laptops; phones only view
   data) and verify that a scout can complete it quickly.

If the application still embeds a single `2026.json`, do not put both FRC and
FTC meanings in that one file. Split schemas by program or make selection
explicit.

### Phase 5: hide unsupported analytics

Until FTC data sources and formulas exist:

- do not show FRC OPR/DPR/CCWM as if they were FTC values;
- do not show TBA rankings for an FTC event;
- do not label manually entered standings as official;
- keep team observations and pick-list data available;
- clearly label derived scouting scores as team estimates.

The UI should say when a feature is unavailable rather than rendering zeros
that look like real statistics.

### Phase 6: optional FTC upstream adapter

Only implement this after obtaining the official provider documentation and
recording real responses from the target qualifier.

The adapter should:

1. Fetch one resource at a time.
2. Save a redacted fixture for each response shape.
3. Deserialize into provider-specific structs.
4. Convert to internal domain records.
5. Validate the converted records.
6. Report partial failures explicitly.
7. Work offline from the last successful snapshot.

Do not reuse TBA's endpoint paths, ranking assumptions, or ETag behavior until
the FTC provider is confirmed to support them.

## Qualifier beta runbook

### Before the event

- Pin the exact commit used on the Pi.
- Seed the BIOBUZZ event, roster, and schedule from reviewed data.
- Confirm there are four teams per match.
- Create scout accounts and assignments.
- Test one complete observation from form to review to pick list.
- Test a rejected observation and correction path.
- Test browser draft recovery after losing connectivity.
- Test QR or physical handoff if used.
- Export a database backup and verify that it can be restored.
- Disable FRC upstream sync and unsupported analytics.

### During the event

- Start with one scout and one match.
- Verify the displayed alliance and partner teams against the official schedule.
- Compare the app's match order with the field schedule.
- Record whether a real scout can finish before the next match.
- Monitor storage and sync errors.
- Do not change the schema during the qualifier.
- If data is wrong, stop collection or use a clearly marked manual procedure;
  do not silently edit the schema or database.

### After the event

- Export observations and the final pick list.
- Compare a sample of observations with scout notes.
- Record fields that scouts misunderstood or could not enter in time.
- Separate application bugs from scouting-strategy changes.
- Bump the schema version for a field meaning change.
- Keep the event data and provider fixtures for regression tests.

## Validation commands and acceptance criteria

The future agent should use the repository's documented commands, then run the
smallest focused tests while iterating. At minimum:

```sh
cargo test -p tt-core
cargo test -p tt-templates
cargo test -p tt-repo-sqlite
cargo test -p tt-web
./check.sh
```

Use the exact package/test commands available in the current checkout; do not
claim success if a command was skipped because the environment lacked a
dependency.

The FTC beta is not ready until:

- all current FRC tests still pass;
- FTC schema parsing and form tests pass;
- four-team match tests pass;
- manual event import rejects malformed data visibly;
- assignments cover exactly the teams in each FTC match;
- no FRC-only analytics appear for the FTC event;
- an observation survives reload, offline draft recovery, and approved review;
- backup and restore have been tested;
- a human scout and lead scout approve the workflow.

## Rollback and safety rules

- Keep the FTC work on a branch or feature flag until the beta is accepted.
- Do not overwrite existing FRC event data.
- Do not migrate existing FRC rows by guessing their program.
- Do not delete old schema versions or payloads.
- Do not turn an upstream error into an empty roster or empty schedule.
- Do not calculate official-looking FTC statistics from FRC formulas.
- If a migration is destructive or changes serialized formats, stop and ask for
  approval before applying it.

## Final handoff required from the AI agent

The implementation agent must report:

1. Files changed and why.
2. Whether the build supports FRC and FTC together or FTC-only.
3. The exact match-shape representation.
4. The source and date of every imported event, team, and match record.
5. Which APIs are enabled, disabled, or manually seeded.
6. Which analytics are unavailable.
7. Test commands and results.
8. Known assumptions requiring human confirmation.
9. The rollback procedure.

The agent should end with a clear statement of what is safe to use at a
qualifier and what remains experimental.
