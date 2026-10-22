# FTC BIOBUZZ beta-test plan

Status: **built on `ftc-biobuzz` and frozen there since 2026-10-21** (F1–F17). This copy is the plan as written; the branch's copy has the build table and the rehearsal as done. Work to bring back to `rebuild` after Athens is in `docs/ACTION_ITEMS.md` (DR1, BR1, PK1, LS1).

**Scope decision (2026-10-15): a separate, FTC-only branch, `ftc-biobuzz`, for one
test event: **Alabama Athens Qualifier 1, Saturday 2026-10-24.** It is never merged back into `rebuild`.** The migration doc
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

## 2. Decisions

**Settled by choosing a throwaway branch:**
- **No dual-program model.** There is no `program` column and no schema registry, and FRC
  behaviour on this branch does not need to keep working.
- **No data migration.** The event runs on a fresh database and freshly wiped
  laptops and phones, so changing the serialized formats (outbox, QR, snapshot) needs no upgrade path.
- **No FTC upstream adapter.** The schedule is seeded by hand, and TBA/FIRST
  sync is switched off.
- **Fixed 2-robot alliances.** Championship 3-robot alliances do not matter for
  one qualifier-level event.

**Answered (2026-10-15):**

| # | Decision | Answer → consequence |
| --- | --- | --- |
| D2 | Event | **Alabama Athens Qualifier 1, Sat 2026-10-24.** Nine days, so the schedule in §5 is tight and F5 is cut back to reusing what already exists |
| D3 | Scouts per match | **2–4, varying through the day.** The generator already leaves extra robots open rather than doubling anyone up, but it balances *scout* load, not *team* coverage. F1b fixes that so every team is still seen across its 5–6 matches |
| D4 | Starting location | **Not scouted.** `start_location` is dropped from the schema |
| D5 | HIVE TIP threshold | Not needed before the event. Launch weights start at 0; the lead scout tunes them on the day through the runtime overrides (L12) |
| D6 | Primary or shadow | Primary, with a paper fallback kit |
| D7 | Devices | **Laptops do the work; phones only view data and sync** (2026-10-20, Open decision 1 in `ACTION_ITEMS.md`). Scouts fill in the form, and the lead scout assigns, reviews and imports, on the wired Chromebooks. Phones show the read-only pages (team, rankings, graph, drive coach, pick list) and hold an offline copy. So the form and the lead pages are checked at laptop width (1280 px), and phone width (360–390 px) only for the read-only pages and offline sync |

---

## 3. Schema — replaces `crates/tt-core/seasons/2026.json` on the branch

On the branch, the FRC `2026.json` is simply replaced. Set `"name": "BIOBUZZ"`,
keep `version: 1`, and record `TU05` in `_comment`. All fields use the existing types (`select`, `counter`, `toggle`, `text`). Point
values are given only where a robot directly earns them under Table 10-2. Every
other field starts at 0, as the migration doc requires. Counter caps are first
guesses and must be checked against match video (§5, B0).

**match**
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
- `flower_role`: select, `none` / `attempted` / `placed_bottom_nectar` / `took_ownership`, 0 pts. Scouts may struggle to judge "top-most" from the stands, so B0 checks it.
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
| F1b | **Coverage with 2–4 scouts.** When there are fewer assignees than robots, fill the open slots with the robots whose team has been assigned least so far at the event. Our own team's robot goes last, because we already know it. The existing rule that nobody watches two robots in one match stays | Test: 2 scouts × a 12-team schedule leaves every team observed at least twice; 4 scouts leave nothing open |
| F2 | **Schema.** Replace `2026.json` with the BIOBUZZ fields from §3, and update the tests that name FRC field keys | Schema parse, valid-payload and invalid-payload tests pass; the form completes at laptop width (D7) |
| F3 | **Manual seed.** Write a small import, either in `tt-load` or as a lead-scout page. It reads a CSV with `match,red1,red2,blue1,blue2` plus a roster, strips the `*` surrogate marker, lists the surrogate slots in the import report, and rejects each bad row by name | A malformed CSV is rejected row by row; the real schedule imports cleanly |
| F4 | **Turn off what is FRC-only.** Disable TBA/FIRST sync and its triggers: the boot fetch, the S6 phone-tether watcher and the S7 lobby fetch. Remove OPR/DPR/CCWM, predictions and TBA rankings from navigation; their routes return "not available on this build" | No upstream request is made during a full rehearsal; no FRC statistic is visible |
| F5 | **Pick list, minimal.** Reuse the existing pick list and team pages. The only new work is labeling the schema score "scout estimate" and making sure the team page shows the BIOBUZZ fields. No new analytics before the event | The lead scout can rank the event's teams from the existing views |
| F6 | Pin the commit, tag it `ftc-biobuzz-beta-1`, and build and install it on the Pi | The tag exists; the Pi reports that commit |

Estimate: F1 is the largest step and the riskiest. Do it first, and on its own commit.

---

## 5. Schedule to Sat 2026-10-24

| Day | Work | Gate |
| --- | --- | --- |
| **Thu 10-15** | F0: clean commit, branch, baseline. Start F1 | Baseline saved |
| **Fri 10-16** | Finish F1 (alliance size) and F1b (coverage) | `cargo test` green with 4-robot fixtures |
| **Sat 10-17** | F2 schema. **B0 desk test**: 2 scouts score the same robot on about 10 BIOBUZZ match videos at real speed (see below) | Schema frozen at `version: 1` by Saturday night |
| **Sun 10-18** | F3 schedule import, F4 upstream off | Malformed CSV rejected row by row; zero upstream calls in a logged run |
| **Mon 10-19** | F5 labels, then a click-through: the form and lead pages at laptop width, the read-only pages and offline sync at phone width (D7) | No 3-robot ghost slots anywhere |
| **Tue 10-20** | F6: tag `ftc-biobuzz-beta-1`, install on the Pi | Pi reports the tagged commit |
| **Wed 10-21** | **B1 dress rehearsal** (see below) | No lost observations |
| **Thu 10-22** | Fix B1 findings only. **Code freeze tonight**, then re-tag | Final tag on the Pi |
| **Fri 10-23** | Check for a new Team Update (re-read §10.5 and §13.6). Wipe and charge the laptops and phones, print the paper kit, take a backup, run a restore check | Bag packed |
| **Sat 10-24** | **Event** (see below) | §7 |

If F1 is not green by **Saturday night (10-17)**, stop and fall back to paper for this event. Keep the branch for the next opportunity. Two red days on the riskiest change leave no time for B1.

### B0 — desk test on match video (Sat 10-17)
- Use early-season BIOBUZZ match video. If too little exists, score robot practice on our own field instead.
- Measure:
  - completion time after the match ends: the target is under 60 s;
  - agreement between the two scouts: ±2 on launches, exact on toggles;
  - fields left blank or guessed (watch `flower_role`).
- Where official score breakdowns exist, compare each alliance's scouted LEAVE and PARK points with the official numbers.
- If launch agreement is poor, swap in a `launch_volume` select (none/few/many) **before** the freeze.

### B1 — dress rehearsal (Wed 10-21)
- Use the real Pi and wired setup (`docs/EVENT_DAY_RUNBOOK.md`), the real laptops for scouting and the lead's pages, real phones for viewing data and offline sync (D7), and the scouts who will actually go.
- Seed a fake 12-team schedule through F3. Run it once with 2 scouts and once with 4, to exercise F1b.
- Walk through every failure path once:
  - offline draft recovery;
  - QR handoff;
  - a refused observation, then a lead-scout correction;
  - pulling the power;
  - backup → wipe → restore.

### Event day (Sat 10-24)
- Seed from the official schedule as soon as it is posted (no later than 15 min before quals, §13.6.1). Check every match against the printed sheet.
- Re-run auto-assign whenever the number of scouts on duty changes.
- Start with one scout on one match, check the result, then bring in the full crew.
- Paper fallback kit: printed forms that mirror the schema, used only if the app is down. Note every switch to paper.
- Do not change the schema. Weight overrides are allowed.
- Take a backup after every qualification round.

### After the event
- Export the observations and the final pick list.
- Run a 20-minute debrief with the scouts and lead scout.
- Write `docs/FTC_BETA_FINDINGS.md` **on `rebuild`**, covering:
  - what worked;
  - fields scouts misunderstood;
  - app bugs versus scouting-strategy issues;
  - the measured completion times;
  - coverage per team;
  - whether the app's pick list matched the picks actually made.
- Freeze the branch at its tag; do not merge. Keep the exported data and the schedule CSV as fixtures.

---

## 6. Risks

| Risk | Mitigation |
| --- | --- |
| F1 misses a 3-slot assumption, and a page shows a ghost third robot or skips coverage | Grep gate in F1, 4-robot fixtures, and a manual click-through of every page in B1 |
| A laptop or phone still holds FRC drafts or snapshots | Wipe all devices before B1. The format-version bump in F1 makes stale data fail loudly |
| Something still reaches TBA/FIRST, for example the S6 tether trigger | F4 gate: rehearse with traffic logging on and confirm zero upstream calls |
| Launch counts are unreliable when the HIVE spills | B0 agreement data; fall back to `launch_volume` |
| The schedule is posted late or changes | F3 import is fast and re-runnable; keep the paper kit |
| Nine days is tight; F1 runs long | Hard stop Saturday 10-17 (§5). F5 is already cut back to reuse |
| With 2 scouts, some teams barely get seen | F1b team-coverage ordering; the lead scout checks per-team counts before alliance selection |
| A Team Update changes scoring before the event | Re-read §10.5 and §13.6 the week before; weights can be changed at runtime |

---

## 7. What counts as a successful test

- Every assigned robot was recorded with no data loss, and every team was observed at least twice.
- Scouts finished within the field reset at least 90% of the time.
- B0 ground-truth checks were within target.
- The lead scout used the app's pick list in alliance selection, or wrote down why not.
- `FTC_BETA_FINDINGS.md` exists and is specific enough to drive the long-term
  dual-program design in the migration doc.
