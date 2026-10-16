# FTC BIOBUZZ beta-test plan

Status: **plan only — nothing implemented.**

**Scope decision (2026-10-15): a separate, FTC-only branch, `ftc-biobuzz`, for one
test event. It is never merged back into `rebuild`.** The migration doc
(`docs/FTC_BIOBUZZ_MIGRATION.md`) describes the long-term dual-program design.
This plan deliberately takes the short path the migration doc allows: "an
FTC-only build". Anything learned at the event goes back into that design as
notes, not as code.

Source: *2026-2027 FIRST Tech Challenge Competition Manual, BIOBUZZ presented
by RTX, version TU05* (fetched 2026-10-15). Rules change through Team Updates,
so the schema must record the manual version it was built against, and someone
must re-check sections 10.5 and 13.6 before each event.

---

## 1. Game facts that drive the design

| Fact | Manual | Consequence for TealTeam |
| --- | --- | --- |
| 2 alliances of **2 teams** | §8, §10 | Match has 4 robots, not 6 |
| Championship playoffs use **3-robot alliances** | §15.3 | Not needed on this one-event branch. The long-term design still needs a variable-size `Alliance` |
| 30 s AUTO, 8 s transition, 2:00 TELEOP (2:30 total) | §10.1, §10.4 | Form must be done within one field reset |
| FLOWER scoring and all-NECTAR entry unlock at **1:00 left** | §10.1, G410, G426 | Endgame = last 60 s, not the last 20–30 s |
| 5 or 6 qualification matches per team | §13.6.1 | Little data per robot — each observation matters more than in FRC |
| **SURROGATE** matches: marked `*`, always the team's 3rd match, no ranking effect | §13.6.2 | Import strips the `*` and reports which slots were surrogates; scouting that robot's play is still useful |
| Playoffs: double elimination, no backups | §13.7, T703 | No "backup robot" concept needed |

### Scoring (Table 10-2)

| Action | AUTO | TELEOP | Attributable to one robot? |
| --- | --- | --- | --- |
| LEAVE (off the perimeter wall) | 3 | – | **Yes** |
| PARK (partly in LOADING ZONE) | 5 | 5 | **Yes** |
| HIVE TIP | 20 | 20 | No: both robots fill the CELL |
| POLLEN/NECTAR left in upward CELL at end | – | 2 each | No |
| Bottom NECTAR Bonus (per FLOWER) | – | 5 | Mostly (whoever placed the bottom NECTAR) |
| Element in an **owned** FLOWER (top NECTAR is yours) | – | 2 each | No: ownership is an alliance result |
| Element in your GARDEN | – | 1 each | Partly |

Ranking points: WIN 3, TIE 1, SWARM 1 (alliance LEAVE + PARK ≥ 16), POLLINATOR 1
(≥ 4 TIPS), POLLINATOR 2 (≥ 7 TIPS). These thresholds apply to ordinary events;
Regional and FIRST Championship thresholds are still TBA. RANKING SCORE is the average RP. Tiebreakers in order:
average match points minus fouls, then average TIPS, then average AUTO points (§13.6.3).

Fouls: MINOR = +5 and MAJOR = +20 **credited to the opponent**. Cards follow
§10.6.

### What this means for scouting

1. **Robot-level points are small and certain; alliance-level points are large
   and shared.** LEAVE and PARK (max 13 per robot) can be counted directly. The
   points that decide matches (TIPS at 20 each, FLOWER ownership) come from both
   robots. So scouts should record each robot's *contribution* (launches into the
   CELL, FLOWER placements), and the app should treat derived points as
   estimates.
2. **The RP picture favors pick-list fields over raw score.** A partner who
   reliably LEAVEs and PARKs in AUTO alone gets an alliance most of the way to
   SWARM (3+3+5+5 = 16 if both do it). Tip contribution feeds both POLLINATOR
   RPs and the second tiebreaker.
3. **The manual does not say how many elements tip a HIVE.** It only says
   "enough". Until we measure it on a real field, a launch count cannot be
   turned into TIP points, so launch counters start at `points_each: 0`.

---

## 2. What the branch decision settles, and what is still open

**Settled by choosing a throwaway branch:**
- **No dual-program model.** There is no `program` column and no schema registry, and FRC
  behaviour on this branch does not need to keep working.
- **No data migration.** The event runs on a fresh database and freshly wiped
  tablets, so changing the serialized formats (outbox, QR, snapshot) needs no upgrade path.
- **No FTC upstream adapter.** The schedule is seeded by hand, and TBA/FIRST
  sync is switched off.
- **Fixed 2-robot alliances.** Championship 3-robot alliances do not matter for
  one qualifier-level event.

**Still open (owner: human):**

| # | Decision | Recommendation |
| --- | --- | --- |
| D2 | Which event, and on what date? | Sets every deadline in §5. If a league meet is available, use it, because the stakes are lowest |
| D3 | Scouts per match | **One per robot (4).** This is how the current assignment model already works. If only 2 scouts are available, assign each one a single robot per match and accept partial coverage; do not build a 2-robot form for a single event |
| D4 | Starting-location options | Watch about 10 early-season match videos and pick 3–4 locations a scout can name |
| D5 | Elements needed to TIP a HIVE | Optional. Launch weights start at 0, and the lead scout can raise them at the event through the existing runtime point-weight overrides (L12) without touching the schema |
| D6 | Is the app primary, or a shadow next to paper? | **Primary, with a paper fallback kit.** Running both in parallel doubles the work for a small FTC team, and a single test is worth more if the app does the real job |

---

## 3. Schema — replaces `crates/tt-core/seasons/2026.json` on the branch

On the branch, the FRC `2026.json` is simply replaced. Set `"name": "BIOBUZZ"`,
keep `version: 1`, and record `TU05` in `_comment`. All fields use the existing types (`select`, `counter`, `toggle`, `text`). Point
values are given only where a robot directly earns them under Table 10-2. Every
other field starts at 0, as the migration doc requires. Counter caps are first
guesses and must be checked against match video (§5, B0).

**match**
- `start_location`: select, options to be set by D4, 0 pts, required.
- `robot_status`: select, `played` / `no_show` / `no_movement` /
  `disabled` / `tipped_over` / `lost_connection`, 0 pts, required. This replaces
  the FRC `no_show` toggle, as the migration doc asks.

**auto** (first 30 s)
- `auto_leave`: toggle, **3**.
- `auto_launch_made`: counter 0–15. Launches that land in the upward CELL. 0 pts (D5).
- `auto_launch_missed`: counter 0–15, 0 pts.
- `auto_flower_pollen_pickup`: toggle. Did the robot take POLLEN from a FLOWER's bottom opening? 0 pts.
- `auto_park`: toggle, **5**.

**teleop**
- `launch_made`: counter 0–60, 0 pts (D5).
- `launch_missed`: counter 0–60, 0 pts.
- `garden_delivered`: counter 0–30. Elements pushed or placed into the robot's own GARDEN. **1 each** (only a lower bound, because the opponent can remove them).

**endgame** (last 60 s)
- `flower_nectar_placed`: counter 0–8. The alliance has 8 NECTAR. 0 pts, because ownership decides the value.
- `flower_pollen_placed`: counter 0–20, 0 pts.
- `flower_role`: select, `none` / `attempted` / `placed_bottom_nectar` / `took_ownership`, 0 pts. Scouts may struggle to judge "top-most" from the stands, so this field is part of the B0 test.
- `teleop_park`: toggle, **5**.

**play style**
- `defense_played`: select, `none` / `some` / `primary_role`.
- `defended_against`: select, `not_defended` / `slowed` / `shut_down`.
- `fouls_observed`: counter 0–10, **0 pts**. The scout counts what they saw; official fouls come from the score sheet.
- `rule_incidents`: text, optional. For example "held 5+ elements" (G407), "touched opponent NECTAR" (G408), "NECTAR in flower early" (G410), "hit the HIVE" (G417), "pin" (G421).

**notes**
- `notes`: text, private.

Each of these fields still needs an entry in the migration doc's schema review
table. That table goes in the PR description.

---

## 4. Build work on the `ftc-biobuzz` branch

| Step | Work | Done when |
| --- | --- | --- |
| F0 | Commit or stash the untracked files on `rebuild` first, so the branch starts from a known state. Branch `ftc-biobuzz` from that commit. Run `./check.sh` and save the result as the baseline | Baseline saved; branch point recorded in this file |
| F1 | **Alliance size.** Add one constant, `ALLIANCE_SIZE = 2`, in `tt-core`. Replace the 10 `[Option<i32>; 3]` sites with it, and fix slot iteration, `partners_of`, `alliance_of`, assignment coverage, the keypad, the match picker, templates and fixtures (17 files). Do not write a literal `2` anywhere else. Bump the snapshot, outbox and QR format version so that a stale FRC tablet is refused rather than misread | `cargo test` is green with 4-robot fixtures; a page shows 2 red and 2 blue teams |
| F2 | **Schema.** Replace `2026.json` with the BIOBUZZ fields from §3, and update the tests that name FRC field keys | Schema parse, valid-payload and invalid-payload tests pass; the form completes at phone width |
| F3 | **Manual seed.** Write a small import, either in `tt-load` or as a lead-scout page. It reads a CSV with `match,red1,red2,blue1,blue2` plus a roster, strips the `*` surrogate marker, lists the surrogate slots in the import report, and rejects each bad row by name | A malformed CSV is rejected row by row; the real schedule imports cleanly |
| F4 | **Turn off what is FRC-only.** Disable TBA/FIRST sync and its triggers: the boot fetch, the S6 phone-tether watcher and the S7 lobby fetch. Remove OPR/DPR/CCWM, predictions and TBA rankings from navigation; their routes return "not available on this build" | No upstream request is made during a full rehearsal; no FRC statistic is visible |
| F5 | **Pick list.** Show per-robot LEAVE/PARK rates, average launches, FLOWER role counts and incidents, all labeled "scout estimate" | The lead scout can rank the event's teams from this view |
| F6 | Pin the commit, tag it `ftc-biobuzz-beta-1`, and build and install it on the Pi | The tag exists; the Pi reports that commit |

Estimate: F1 is the largest step and the riskiest. Do it first, and on its own commit.

---

## 5. Test plan, working back from the event

### T-3 weeks — B0: desk test on match video
- Two scouts score the **same robot** independently on about 10 recorded BIOBUZZ matches, at real speed, on phones.
- Measure:
  - completion time after the match ends: the target is under 60 s;
  - agreement between the two scouts: ±2 on launches, exact on toggles;
  - fields left blank or guessed (watch `flower_role` and `start_location`).
- **Ground truth:** for each alliance, add up the scouted LEAVE and PARK points and compare them with the official score breakdown. Do the same for CELL launches against the official TIP count.
- **Gate:** schema frozen at `version: 1`; D4 answered. If launch agreement is poor, swap in a `launch_volume` select (none/few/many).

### T-1 week — B1: dress rehearsal
- Use the real Pi and wired setup from `docs/EVENT_DAY_RUNBOOK.md`, real phones and the real scouts, with a fake 12-team schedule seeded through F3.
- Walk through every failure path once:
  - offline draft recovery;
  - QR handoff;
  - a refused observation, then a lead-scout correction;
  - pulling the power;
  - backup → wipe → restore.
- **Gate:** no lost observations; every match is covered; the pick list is built from the rehearsal data.

### Event day — B2
- Seed from the official schedule as soon as it is posted (no later than 15 min before quals, §13.6.1). Check every match against the printed sheet.
- Start with one scout on one match, check the result, then the full crew.
- Paper fallback kit: printed forms that mirror the schema, used only if the app is down. Note every switch to paper.
- Do not change the schema during the event. Weight overrides are allowed.
- Take a backup after every qualification round.

### After the event
- Export the observations and the final pick list.
- Run a 20-minute debrief with the scouts and lead scout.
- Write `docs/FTC_BETA_FINDINGS.md` **on `rebuild`**, covering:
  - what worked;
  - fields scouts misunderstood;
  - app bugs versus scouting-strategy issues;
  - the measured completion times;
  - whether the app's pick list matched the picks actually made.
- Freeze the branch: keep the tag and do not merge. Keep the exported data and the schedule CSV as fixtures for the future dual-program build.

---

## 6. Risks

| Risk | Mitigation |
| --- | --- |
| F1 misses a 3-slot assumption, and a page shows a ghost third robot or skips coverage | Grep gate in F1, 4-robot fixtures, and a manual click-through of every page in B1 |
| A tablet still holds FRC drafts or snapshots | Wipe all devices before B1. The format-version bump in F1 makes stale data fail loudly |
| Something still reaches TBA/FIRST, for example the S6 tether trigger | F4 gate: rehearse with traffic logging on and confirm zero upstream calls |
| Launch counts are unreliable when the HIVE spills | B0 agreement data; fall back to `launch_volume` |
| The schedule is posted late or changes | F3 import is fast and re-runnable; keep the paper kit |
| A Team Update changes scoring before the event | Re-read §10.5 and §13.6 the week before; weights can be changed at runtime |

---

## 7. What counts as a successful test

- Every qualification match had its scouted robots recorded with no data loss.
- Scouts finished within the field reset at least 90% of the time.
- B0 ground-truth checks were within target.
- The lead scout used the app's pick list in alliance selection, or wrote down why not.
- `FTC_BETA_FINDINGS.md` exists and is specific enough to drive the long-term
  dual-program design in the migration doc.
