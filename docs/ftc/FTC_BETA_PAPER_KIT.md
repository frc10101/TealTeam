# FTC BIOBUZZ paper scouting sheet

Printable fallback for the `ftc-biobuzz` branch (see `docs/ftc/FTC_BETA_PLAN.md`
§3 for the schema this mirrors, and §5 "Event day" for when to use it). Use
only when the app is down. **One sheet per robot per match.** Print enough
copies for a full day before Fri 10-23 (§5). Print `docs/event/print/scouting-sheet.pdf` (one page, built from the season schema; edit the .docx if needed); the paper-switch log is on `docs/event/print/event-checklists.pdf`.

Field order below matches the schema in §3 exactly, so a sheet can be typed
into the app field-by-field with no re-reading.

---

## Header

| Scout name | Match # | Team # | Alliance | Time paper started |
| --- | --- | --- | --- | --- |
| | | ☐ Red ☐ Blue | | : |

---

## match

**robot_status** (pick one)

☐ played ☐ no_show ☐ no_movement ☐ disabled ☐ tipped_over ☐ lost_connection

---

## auto (first 30 s)

**auto_leave** — robot left the perimeter wall

☐ yes ☐ no

**auto_launched** — every element launched at the HIVE, in or not; tick one
box as each leaves the robot (0–20)

```
[  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]
```

**auto_flower_pollen_pickup** — took POLLEN from a FLOWER's bottom opening

☐ yes ☐ no

**auto_park** — partly in LOADING ZONE at end of AUTO

☐ yes ☐ no

---

## teleop (all 2:00)

**launched** — every element launched at the HIVE, in or not, endgame
included (0–60)

```
[  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]
[  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]
[  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]
```

**launch_accuracy** — after the match, auto and teleop together; leave blank
if it never launched (pick one)

☐ most ☐ about_half ☐ few

**garden_delivered** — elements pushed/placed into the robot's own GARDEN,
endgame included (0–30)

```
[  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]
[  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]
```

---

## endgame (last 60 s)

**flower_nectar_placed** — FLOWERS open at 1:00 left (0–8)

```
[  ][  ][  ][  ][  ]   [  ][  ][  ]
```

**flower_pollen_placed** (0–20)

```
[  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]
```

**flower_role** — the most it did; bottom NECTAR = its NECTAR went into an
empty FLOWER. Do not judge who owns the top (pick one)

☐ none ☐ attempted ☐ placed ☐ placed_bottom_nectar

**teleop_park** — partly in LOADING ZONE at end of match

☐ yes ☐ no

---

## play style

**defense_played** (pick one)

☐ none ☐ some ☐ primary_role

**defended_against** (pick one)

☐ not_defended ☐ slowed ☐ shut_down

**driver_skill** — against the other three robots in this match (pick one)

☐ struggling ☐ average ☐ strong ☐ excellent

**fouls_observed** — fouls the scout saw, not the official score sheet (0–10)

```
[  ][  ][  ][  ][  ]   [  ][  ][  ][  ][  ]
```

**rule_incidents** (optional free text — e.g. "held 5+ elements" G407, "touched
opponent NECTAR" G408, "NECTAR in flower early" G410, "hit the HIVE" G417, "pin" G421)

```




```

---

## notes

**notes** (private, free text)

```




```

---

## Entering paper sheets later

After the app is back up, the lead scout — not individual scouts — types each
paper sheet into the app, one sheet per robot/match, in the field order above.
While entering a sheet that was filled out on paper, the lead scout writes
`PAPER` in that observation's `rule_incidents` field (in addition to any real
incident text), so findings can count how many observations switched to
paper and when. Keep the physical sheets after entry; they are the only
record if an entry is lost.
