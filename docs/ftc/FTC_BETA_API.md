# FTC beta: upstream APIs for the Athens Qualifier

This covers one event only, on the `ftc-biobuzz` branch. Nothing here needs to
generalise to other events or to FRC. Checked on 2026-10-17 against the FTC
Events API's OpenAPI spec and the live APIs.

## The event

| | |
| --- | --- |
| Official name | **Athens Qualifier** (the only Athens event in Alabama this season, so there is no "Qualifier 1") |
| Event code | **`USALATQ`**; season path segment **`2026`** |
| Date | Saturday 2026-10-24, one field, capacity 14 teams |
| Venue | Athens Renaissance School, 601 S Clinton St, Athens, AL |
| Time zone | `America/Chicago` |
| Teams listed by FTCScout on 10-17 | 7842, 18664, 19459, 21830, 24148, 24378, 25114, 27305, 30403, 31017, 35722 (11; re-check the morning of the event) |
| Schedule | not published yet (it is posted on the day, at least 15 min before quals) |

## Sources

**FTC Events API** (official, primary)
- Base URL: `https://ftc-api.firstinspires.org/v2.0`
- Auth: `Authorization: Basic base64(username:key)`. The key comes from
  registering at https://ftc-events.firstinspires.org/services/API/register.
  **We do not have one yet.**
- Caching: send `If-Modified-Since` with the `Last-Modified` value from the last
  response to get a 304 when nothing changed. `FMS-OnlyModifiedSince` instead
  returns only the records that changed. Never send both. Empty results carry no
  `Last-Modified`.
- No documented rate limit, only a 503 with `Retry-After`.
- Matches still being played are never returned.

**FTCScout** (unofficial mirror; no key)
- REST: `https://api.ftcscout.org/rest/v1`
- GraphQL: `https://api.ftcscout.org/graphql`
- It may lag the official API.

**The Blue Alliance does not cover FTC.** Every TBA call has no FTC equivalent there.

## Endpoints for USALATQ

All on `https://ftc-api.firstinspires.org/v2.0/2026`.

| Need | Endpoint | Response model (key fields) |
| --- | --- | --- |
| Event | `GET /events?eventCode=USALATQ` | `events[]`: `code`, `name`, `timezone`, `dateStart`, `dateEnd`, `venue`, `published` |
| Teams | `GET /teams?eventCode=USALATQ` (paged; one page here) | `teams[]`: `teamNumber`, `nameShort`, `nameFull`, `city`, `stateProv`; plus `pageTotal` |
| Schedule and results | `GET /schedule/USALATQ/qual/hybrid` | `schedule[]`: `matchNumber`, `startTime`, `actualStartTime`, `scoreRedFinal`, `scoreBlueFinal`, `redWins`, `blueWins`, and `teams[]` with `teamNumber`, `station` (`Red1`…`Blue2`), `surrogate`, `noShow`, `dq` |
| Per-alliance score sheet | `GET /scores/USALATQ/qual` | `matchScores[]` of `ScoreDetailModel_2026`. The example is in `docs/ftc/USALATQ_scores_qual.example.json` |
| Rankings | `GET /rankings/USALATQ` | `rankings[]`: `rank`, `teamNumber`, `sortOrder1..6`, `wins`, `losses`, `ties`, `qualAverage`, `matchesPlayed` |
| Playoffs (optional) | `GET /schedule/USALATQ/playoff/hybrid`, `GET /alliances/USALATQ` | same models |

The hybrid schedule makes `/schedule` and `/matches` unnecessary: it returns
the schedule with results merged in, and nulls for unplayed matches. It also
flags surrogates, which the CSV import (F3) reads from a `*`.

## The score sheet (`ScoreDetailAllianceModel_2026`)

One object per alliance per match. `docs/ftc/USALATQ_scores_qual.example.json` is a
**made-up** match. It uses exactly the spec's field set, and its points are
consistent with Table 10-2. No real 2026 match exists yet. Points that come
from one robot are the only ones we can check a scout against:

| Score sheet field | Our scouting field | Check |
| --- | --- | --- |
| `autoRobot1Leave`, `autoRobot2Leave` | `auto_leave` | exact, per robot |
| `autoRobot1Park`, `autoRobot2Park` | `auto_park` | exact, per robot |
| `teleopRobot1Park`, `teleopRobot2Park` | `teleop_park` | exact, per robot |
| `autoHiveTips`, `teleopHiveTips` | sum of both robots' `auto_launched` / `launched`, scaled by `launch_accuracy` | rough: it gives the TIP threshold (plan D5) |
| `teleopGardenPollenNectar` | sum of `garden_delivered` | lower bound |
| `minorFouls`, `majorFouls` | `fouls_observed` | the scout sees the opponent's fouls credited the other way |
| `swarmRP`, `pollinator1RP`, `pollinator2RP` | none | for the pick list's RP picture |

Unverified until the first real match:
- `Robot1`/`Robot2` probably follow stations `Red1`/`Red2`.
- Flower arrays are probably listed bottom to top.
- `teleopHivePoints` is assumed to include the 2-point POLLEN/NECTAR left in the CELL.
- `foulPointsCommitted` is assumed to be added to the *other* alliance's
  `totalPoints`, as FTCScout's naming suggests.
- `team` is 0 or unused.

## What changes in our code

The app calls 7 upstream endpoints today. F4 switched all of them off with
`UPSTREAM_ENABLED = false`.

| Today (FRC) | Where | For Athens |
| --- | --- | --- |
| FIRST `GET /v3.0/{season}/events` | `tt-upstream/src/first.rs:201` → `sync::sync_events` | **Change:** base URL to `ftc-api…/v2.0`, season 2026, `FIRST_EVENT_CODE=USALATQ`. The envelope `{"events": [...]}` and field names differ slightly from FRC's `Events` |
| FIRST `GET /v3.0/{season}/teams?eventCode=` | `first.rs:226` → `store_roster` | **Change:** same client; FTC uses `nameShort`/`nameFull` and the same paging |
| TBA `/event/{key}/matches` | `tba.rs:326` → `store_matches`, and via `project.rs` | **Replace** with FTC `/schedule/USALATQ/qual/hybrid` and map it into `MatchRecord` (2 per alliance by `station`; `-1`/null for unplayed) |
| TBA `/event/{key}/rankings` | `tba.rs:321` → `store_stats` | **Replace** with FTC `/rankings/USALATQ` (rank, W-L-T, `qualAverage`) |
| TBA `/event/{key}/oprs` | `tba.rs:312` | **Drop**: off in F4. Could be computed locally later |
| TBA `/event/{key}/coprs` | `tba.rs:316` | **Drop** |
| TBA `/events/{year}` (key lookup) | `tba.rs:333` | **Drop**: one event, key fixed |
| *(new)* FTC `/scores/USALATQ/qual` | none | **Add if wanted**: store raw in the upstream log, then use the per-robot LEAVE/PARK booleans as the B0 ground-truth check |

Plumbing that also changes:
- **Upstream log:** `upstream.api` has `CHECK (api IN ('first','tba'))`
  (migration 0003). Either log FTC responses as `'first'` (the FTC API is
  FIRST's) or add `'ftc'` with migration 0011.
- **Team keys:** `team_key(n)` = `"frc{n}"` and `team_number()` drops any other
  prefix (`tt-core/src/upstream.rs:392/401`). The CSV import (F3) already stores
  FTC teams under `frc…` keys. Keep that; renaming is not worth it for one event.
- **Event key:** with code `USALATQ`, `tba_key()` makes `2026usalatq`. Use that
  same code in the F3 import so the two paths agree.
- **Tablet courier (S7):** it is TBA-only and stays off. No FTC path is needed on the tablet.
- **Secrets:** F7 uses its own `FTC_API_USERNAME` / `FTC_API_KEY`, so FIRST's stay unread while F4 has FIRST off.

## Recommendation

The F3 CSV import already covers the event. Paste the schedule from the
event's page when it is posted. The smallest useful upstream change is:

1. Register for an FTC Events API key now. Approval can take days.
2. **One optional pull:** FTC `events` + `teams` + `schedule/qual/hybrid` + `rankings`
   for `USALATQ`, behind a switch, with the CSV import as the fallback. That is
   about one day of work, and it would reverse part of F4.
3. Log `/scores/USALATQ/qual` raw for after-event analysis (B0 ground truth,
   TIP threshold). It does not need to be in the app before the event.

At the event, the Pi's only uplink is the lead's phone tether, and that tether
watcher is also off in F4. So even with (2) built, the CSV paste stays the plan
of record.

## Getting a key and switching it on (F7)

F7 built the optional pull in (2): the event, teams, qualification hybrid
schedule, rankings, and the qualification score sheets (log only) for USALATQ.

1. Register at https://ftc-events.firstinspires.org/services/API/register with
   the team's email. FIRST emails a username and an API key; approval can
   take days.
2. On the Pi, put both in `.env` as `FTC_API_USERNAME=` and `FTC_API_KEY=`, then
   restart `tt-web`. `FTC_EVENT_CODE` defaults to `USALATQ`.
3. Plug in the phone tether, open **Lead scout → Schedule**, and press **Pull
   from FTC Events**. The page says what the pull stored, which matches it
   removed, and which it kept because they have observations. A 401 means the
   key is wrong; "busy" means FTC sent a 503 and asked us to wait.

Without both values there is no button and no request. Nothing polls in the
background: a pull happens only when someone presses the button, so a pasted
schedule is never replaced behind the lead's back. A second press sends
`If-Modified-Since` and changes nothing if FTC has nothing new. The pull and
the CSV paste use the same event key (`2026usalatq`), so either can follow the
other. The CSV paste stays the plan of record.
