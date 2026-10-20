# FTC BIOBUZZ beta: schema review

This is the schema review for `crates/tt-core/seasons/2026.json` on the
`ftc-biobuzz` branch (step F2 of `docs/FTC_BETA_PLAN.md`, revised in the F8
accuracy pass). The columns answer
the checklist in `docs/FTC_BIOBUZZ_MIGRATION.md` ("Schema review checklist").
It was built against the *2026-2027 FTC Competition Manual, BIOBUZZ, TU05*.

How unknown answers are stored, for every field kind:

- An untouched counter posts its rendered minimum, so it is stored as **0**.
- An unticked toggle is stored as **false**.
- An unanswered select is **omitted**. Only `robot_status` is required.
- An empty text field is **omitted**.

F8 held every field to one test, made without the B0 video desk test: can one
scout in the stands, watching one robot, see it reliably and finish the form
within one field reset (about 60 s)? The "Why (F8)" column gives the answer for
each field. Three decisions follow from it:

- **Launches are counted at the robot, not at the HIVE.** The F2 form split
  made and missed. That needs the scout to judge each element's landing in an
  upward CELL that both alliance robots fill at once and that spills when it
  TIPs, during volleys that leave the robot faster than two counters can be
  tapped. An element leaving the robot is one thing to see, where the scout's
  eyes already are. So each period has one count, and landing is a single
  three-way `launch_accuracy` choice after the match. A `launch_volume`
  select (none/few/many) was rejected: with only 5–6 matches per team, a
  count is what separates a 15-launch robot from a 35-launch one on the pick
  list. It stays the B0 fallback.
- **FLOWER ownership is not asked.** Whose NECTAR is top-most is decided by
  the last placement before the buzzer, by either robot of either alliance,
  while the scout is watching PARK. The score sheet's FLOWER arrays record it
  exactly per alliance. The scout records what this robot put in instead.
- **No field needs the scout to time the 1:00 mark.** FLOWERS only score in
  the last 60 s, so the endgame section is the FLOWER counters plus PARK;
  launches and GARDEN count across all of TELEOP, endgame included.

The "Ranking" column gives the schema's point value. A lead scout can retune
any value on the day through the runtime overrides (L12).

| Field | What it measures | Support | Scout can enter it live? | Independent of official score? | Bounds / options | Ranking | When unknown | Why (F8) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `robot_status` | Whether the robot played the match, and if not, why | Migration doc (it replaces the FRC `no_show`); team decision | Yes: one tap, usually "Played" | Yes; the score sheet does not record it | `played` / `no_show` / `no_movement` / `disabled` / `tipped_over` / `lost_connection` | No (all 0) | Required, so the form is refused without it | Kept. Each state is visible from the stands, and it explains a low match on the pick list |
| `auto_leave` | The robot left the perimeter wall in AUTO | Table 10-2 LEAVE | Yes, at the end of AUTO | Overlaps with the alliance LEAVE total; this field gives the per-robot split | true / false | Yes, **3** | false | Kept. One robot, one moment, checkable per robot against `autoRobotNLeave` |
| `auto_launched` | Every element the robot launched at the HIVE in AUTO, in or not | Table 10-2 (HIVE TIP); §1 of the plan | Yes, tap as each element leaves the robot | Yes; the official score counts TIPs, not launches | 0–20, a ceiling for a 30 s AUTO, not an expected value | Defer, 0 (D5: TIP threshold unknown) | 0 | Replaces `auto_launch_made` and `auto_launch_missed`: counting at the robot is one observation, landing in a shared, spilling CELL is not |
| `auto_flower_pollen_pickup` | The robot took POLLEN from a FLOWER's bottom opening in AUTO | Team decision (auto capability) | Yes | Yes | true / false | No, 0 | false | Kept. A drive to a FLOWER is easy to see, and it marks an AUTO that collects more than its preload |
| `auto_park` | The robot ended AUTO partly in the LOADING ZONE | Table 10-2 PARK (AUTO) | Yes, at the end of AUTO | Overlaps with the alliance PARK total; this field gives the per-robot split | true / false | Yes, **5** | false | Kept. Checkable per robot against `autoRobotNPark` |
| `launched` | Every element the robot launched at the HIVE in TELEOP, endgame included | Table 10-2 (HIVE TIP) | Yes, as in AUTO | Yes; the official score counts TIPs | 0–60, a ceiling for 2:00 TELEOP | Defer, 0 (D5) | 0 | Replaces `launch_made` and `launch_missed`, for the same reason as `auto_launched` |
| `launch_accuracy` | How many of the robot's launches landed in the upward CELL, judged once for the whole match | Table 10-2 (HIVE TIP); team decision (accuracy) | Yes, after the match | Yes | `most` / `about_half` / `few` | No, 0 | Omitted (blank when the robot never launched) | New. Keeps an accuracy signal without a per-element landing call. With the counts it gives a rough TIP contribution |
| `garden_delivered` | Elements this robot pushed or placed into its own GARDEN, all of TELEOP | Table 10-2 GARDEN (1 each) | Yes, tap per element | Partly; the official GARDEN total is per alliance, and opponents can remove elements, so this is a lower bound | 0–30 | Yes, **1 each** | 0 | Kept. The robot earns these points directly. Help text now says endgame counts too |
| `flower_nectar_placed` | NECTAR this robot placed in FLOWERS (they open at 1:00 left) | Table 10-2; G410, G426 | Yes | Yes; the score sheet records ownership, not who placed what | 0–8 (the alliance has 8 NECTAR) | Defer, 0 (ownership decides the value) | 0 | Kept. A placement is slow and deliberate, so it can be counted |
| `flower_pollen_placed` | POLLEN this robot placed in FLOWERS | Table 10-2 | Yes | Yes | 0–20 | Defer, 0 | 0 | Kept, as for NECTAR |
| `flower_role` | The most this robot did at a FLOWER | Table 10-2 (bottom NECTAR bonus) | Yes, after the match | Yes; the score sheet does not say which robot placed what | `none` / `attempted` / `placed` / `placed_bottom_nectar` | Defer, 0 | Omitted | `took_ownership` dropped: "top-most" is decided at the buzzer, by any robot, while the scout watches PARK, and the score sheet records it exactly. `placed` added so a robot that placed only POLLEN has an answer. Bottom NECTAR (its NECTAR into an empty FLOWER) is seen at the moment of placing |
| `teleop_park` | The robot ended the match partly in the LOADING ZONE | Table 10-2 PARK (TELEOP) | Yes, at the buzzer | Overlaps with the alliance PARK total; this field gives the per-robot split | true / false | Yes, **5** | false | Kept. Checkable per robot against `teleopRobotNPark` |
| `defense_played` | How much defense the robot played | Team decision (pick list) | Yes, after the match | Yes | `none` / `some` / `primary_role` | No, 0 | Omitted | Kept. Three coarse steps agree better between scouts than a finer scale |
| `defended_against` | How much opponents' defense slowed this robot | Team decision (pick list) | Yes, after the match | Yes | `not_defended` / `slowed` / `shut_down` | No, 0 | Omitted | Kept. Explains a low launch count |
| `driver_skill` | How well the robot was driven, against the other three in the match | Team decision (pick list); the FRC form's field | Yes, after the match | Yes | `struggling` / `average` / `strong` / `excellent` | No, 0 | Omitted | New. With 2-robot alliances the partner's driving matters more than in FRC, and FRC scouts know this choice. Anchored to the match so scouts rate the same way |
| `fouls_observed` | Fouls the scout saw this robot commit | §10.6; team decision | Yes, tap per foul | No; official fouls come from the score sheet. This count is only what the scout saw | 0–10 | No, **0** (no guessed negative points) | 0 | Kept, unscored: a scout cannot see every call, but a repeat offender still shows |
| `rule_incidents` | Short free text naming the rules involved (G407, G408, G410, G417, G421) | Manual G4xx rules | Yes, typed after the match | Yes | Text, up to 500 characters | Never (text is not scored) | Omitted. Visible only to the scout's own team, like all text fields | Kept. Optional, so it costs no time when nothing happened |
| `notes` | Anything the numbers miss | Team decision | Yes, typed after the match | Yes | Text, up to 2000 characters | Never (text is not scored) | Omitted. **Private**: only the scout's own team can read it (U13) | Kept |

`start_location` is deliberately not scouted (decision D4).
