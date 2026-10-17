# FTC BIOBUZZ beta: schema review

This is the schema review for `crates/tt-core/seasons/2026.json` on the
`ftc-biobuzz` branch (step F2 of `docs/FTC_BETA_PLAN.md`). The columns answer
the checklist in `docs/FTC_BIOBUZZ_MIGRATION.md` ("Schema review checklist").
It was built against the *2026-2027 FTC Competition Manual, BIOBUZZ, TU05*.

How unknown answers are stored, for every field kind:

- An untouched counter posts its rendered minimum, so it is stored as **0**.
- An unticked toggle is stored as **false**.
- An unanswered select is **omitted**. Only `robot_status` is required.
- An empty text field is **omitted**.

The "Ranking" column gives the schema's point value. A lead scout can retune
any value on the day through the runtime overrides (L12).

| Field | What it measures | Support | Scout can enter it live? | Independent of official score? | Bounds / options | Ranking | When unknown |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `robot_status` | Whether the robot played the match, and if not, why | Migration doc (it replaces the FRC `no_show`); team decision | Yes: one tap, usually "Played" | Yes; the score sheet does not record it | `played` / `no_show` / `no_movement` / `disabled` / `tipped_over` / `lost_connection` | No (all 0) | Required, so the form is refused without it |
| `auto_leave` | The robot left the perimeter wall in AUTO | Table 10-2 LEAVE | Yes, at the end of AUTO | Overlaps with the alliance LEAVE total; this field gives the per-robot split | true / false | Yes, **3** | false |
| `auto_launch_made` | AUTO launches that land in the upward CELL | Table 10-2 (HIVE TIP); §1 of the plan | Yes, tap per launch | Yes; the official score counts TIPs, not launches | 0–15, a first guess (B0 checks it against video) | Defer, 0 (D5: TIP threshold unknown) | 0 |
| `auto_launch_missed` | AUTO launches that miss the CELL | Team decision (accuracy) | Yes, tap per launch | Yes | 0–15 | No, 0 | 0 |
| `auto_flower_pollen_pickup` | The robot took POLLEN from a FLOWER's bottom opening in AUTO | Team decision (auto capability) | Yes | Yes | true / false | No, 0 | false |
| `auto_park` | The robot ended AUTO partly in the LOADING ZONE | Table 10-2 PARK (AUTO) | Yes, at the end of AUTO | Overlaps with the alliance PARK total; this field gives the per-robot split | true / false | Yes, **5** | false |
| `launch_made` | TELEOP launches that land in the upward CELL | Table 10-2 (HIVE TIP) | Yes, tap per launch | Yes; the official score counts TIPs | 0–60, a first guess for 2:00 TELEOP | Defer, 0 (D5) | 0 |
| `launch_missed` | TELEOP launches that miss the CELL | Team decision (accuracy) | Yes | Yes | 0–60 | No, 0 | 0 |
| `garden_delivered` | Elements this robot pushed or placed into its own GARDEN | Table 10-2 GARDEN (1 each) | Yes, tap per element | Partly; the official GARDEN total is per alliance, and opponents can remove elements, so this is a lower bound | 0–30 | Yes, **1 each** | 0 |
| `flower_nectar_placed` | NECTAR this robot placed in FLOWERS (endgame, last 60 s) | Table 10-2; G410, G426 | Yes | Yes; the score sheet records ownership, not who placed what | 0–8 (the alliance has 8 NECTAR) | Defer, 0 (ownership decides the value) | 0 |
| `flower_pollen_placed` | POLLEN this robot placed in FLOWERS (endgame) | Table 10-2 | Yes | Yes | 0–20 | Defer, 0 | 0 |
| `flower_role` | This robot's part in the FLOWER fight | Table 10-2 (bottom NECTAR bonus, ownership) | Mostly; "top-most" is hard to judge from the stands, so B0 checks it | Partly; ownership is an alliance result | `none` / `attempted` / `placed_bottom_nectar` / `took_ownership` | Defer, 0 | Omitted |
| `teleop_park` | The robot ended the match partly in the LOADING ZONE | Table 10-2 PARK (TELEOP) | Yes, at the buzzer | Overlaps with the alliance PARK total; this field gives the per-robot split | true / false | Yes, **5** | false |
| `defense_played` | How much defense the robot played | Team decision (pick list) | Yes, after the match | Yes | `none` / `some` / `primary_role` | No, 0 | Omitted |
| `defended_against` | How much opponents' defense slowed this robot | Team decision (pick list) | Yes, after the match | Yes | `not_defended` / `slowed` / `shut_down` | No, 0 | Omitted |
| `fouls_observed` | Fouls the scout saw this robot commit | §10.6; team decision | Yes, tap per foul | No; official fouls come from the score sheet. This count is only what the scout saw | 0–10 | No, **0** (no guessed negative points) | 0 |
| `rule_incidents` | Short free text naming the rules involved (G407, G408, G410, G417, G421) | Manual G4xx rules | Yes, typed after the match | Yes | Text, up to 500 characters | Never (text is not scored) | Omitted. Visible only to the scout's own team, like all text fields |
| `notes` | Anything the numbers miss | Team decision | Yes, typed after the match | Yes | Text, up to 2000 characters | Never (text is not scored) | Omitted. **Private**: only the scout's own team can read it (U13) |

`start_location` is deliberately not scouted (decision D4).
