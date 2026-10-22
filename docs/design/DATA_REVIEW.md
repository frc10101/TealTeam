# Data review for alliance selection

*2026-10-21. Item **DR1** in `ACTION_ITEMS.md`, row **F14** in
`FTC_BETA_PLAN.md`. Built on the FTC beta branch (`ftc-biobuzz`) for the
Athens Qualifier (Sat 2026-10-24).*

Before alliance selection a lead scout and the drive coach have minutes,
not hours, to answer: *who is good, who is steady, who is better than whom
at what, and how sure are we?* This doc covers the pages built for that,
where their design comes from, and what is still to do.

The graph's design follows **AdvantageScope**, the FRC log viewer, since many
students already know it. Two of its tabs map onto scouting data:

| AdvantageScope | What it does there | Here |
| --- | --- | --- |
| **Line graph** | Fields over time; left and right axes for numbers on different scales; a discrete strip for on/off values; click a time to read every field there | **Graph** (`/graph`, U21): a team's metrics match by match. Tap a match to read every line there. **New:** a metric whose numbers are over three times bigger or smaller than the first one's goes on a right-hand axis |
| **Statistics** | Count, mean, median, quartiles, standard deviation and a histogram per field; a reference field that others are measured against | **Compare** (`/compare`, new): each team's spread per metric, with a box plot on a scale the row shares, and an optional reference team every other column is measured against |

---

## 1. Compare (`/compare`)

A column per team (up to 6, colour-coded as on Graph) and a row per thing
scouts record, in the order of the form.

- **Moving teams in and out (user, 2026-10-21):**
  - **Remove** on each team's column takes it out.
  - **+ Add team** opens a search of the event's teams, by number or name,
    best scout estimate first, each with its estimate, place and observation
    count; tap one, or press Enter for the top match.
  - The limit is 6 teams, which fit a Chromebook screen. Adding a 7th takes
    the last column's place, never our own team or the one measured against,
    and the page says which team made room.
- **Every number comes with its spread.** "8.0" alone hides whether a team
  scored 8, 8, 8 or 0, 8, 16. Each cell shows the per-match average, then the
  median, range, ± one standard deviation and how many matches it rests on.
- **Box plots on a shared scale.** Under each number, a line for the whole
  range, a box for the middle half, a tick at the median. Every box in a row
  is on the same scale, so steady and streaky teams look different at a
  glance.
- **The best in each row is marked.** Higher is better except where it is
  not: fouls, DPR and official rank are lower-is-better, and the row says so.
  No mark when only one team has a value or all are equal.
- **Measure against a team.** When our own team is among those chosen, the
  others are measured against it unless "Nobody" is picked (user,
  2026-10-21). Any other chosen team can be picked instead, such as our first
  pick. Every other cell then shows how far ahead (green) or behind (red) it is: in
  points for counts, in percentage points for yes/no rates.
- **Last 3 or 5 matches.** Like AdvantageScope's time range: how a team plays
  now, not on its first match.
- **Each kind of field is read its own way:**
  - counts, and choices that carry points, are spreads;
  - yes/no fields are rates, "67% (2 of 3)";
  - choices without points (robot status, driver skill, defense) are tallies,
    "Played 11 · Disabled 1", since they have no "higher";
  - notes are not compared; each column header links to the team page.
- **Official numbers close the table:** rank, OPR, DPR and CCWM, once a
  results pull has filled them.
- **Rows nobody recorded anything in are left out**, so the table is only as
  long as the data.
- **Thin data is flagged.** A team with fewer than 3 scouted matches is
  marked "thin", as on Rankings.

**Ways in:**
- **Rankings:** tick teams, then **Compare ticked**.
- **Team page:** **Compare with the best here** puts that team first, as the
  reference, beside the best by scout estimate.
- **Pick list:** **Compare** takes the teams not yet crossed off, in pick
  order.
- **Graph:** **Compare**, and Compare has **Graph these** back.

Every choice is in the URL, so a comparison can be bookmarked or sent to the
drive coach. It is a read-only page, so phones may open it (device roles,
Open decision 1). At 1280 px the table fits; at phone width it scrolls
sideways inside its card, with the row names held in place.

**Checked:**
- unit tests for the spread maths, the window, the table's rows, the best
  mark, fouls being lower-is-better, the reference deltas and empty rows
  being left out;
- router tests for the page, its sources line, the sign-in guard and the
  links from Rankings, the team page and Graph;
- `tests/browser/compare.mjs` in Chromium at 1280 and 390 px: defaults,
  unticking a team, a reference, the last 3 matches, Rankings → Compare, the
  team page → Compare, no page-wide sideways scroll;
- the role click-through (`role-split.mjs`) now opens Compare on a laptop
  and on two phone sizes.

## 2. Rankings by what scouts recorded

Built 2026-10-21 (was "next" item 5). Rankings gets a column for each thing
scouts count, after the scout estimate: the team's average per match, from
the same numbers as Compare. Each column header sorts, best first (fouls:
fewest first, and the header says so). **Columns** picks which: any count,
any choice that carries points, and any yes/no as how often ("67%"). The
choice and the sort are in the URL.

On a phone the headers are hidden, so one **Sort by** dropdown sorts, and
each card shows the metric it is sorted by.

## 3. Graph (`/graph`)

Unchanged apart from the right-hand axis: a yes/no (0–1) beside points
(10–40) no longer lies flat along the bottom. The page says which metric is
on the right. `tests/browser/graph.mjs` checks both cases.

---

## 4. Next, after Athens

In order of value for selection, from what the beta shows:

1. **Choose a metric's axis.** In AdvantageScope the user picks left or right;
   here it is automatic. Let a tap on a metric chip move it.
2. **Rolling average on Graph.** A 3-match average line, to see a trend through
   noisy matches (AdvantageScope's derivative and integral options are the
   closest it has).
3. **On/off fields as a strip under the graph**, like AdvantageScope's
   discrete fields: parked or not, disabled or not, per match.
4. **A distribution view:** a histogram of one metric for every team at the
   event, to see where a team sits in the field, not just against 6 others.
5. ~~**Rankings by any metric**~~: built, section 2.
6. **Compare offline:** move it to the pages the device can build with no
   server (C6), as the team page and Graph already are. Built in `tt-pages`
   so this is a registration in `tt-client`, not a rewrite.
7. **Bring it to `rebuild`** with the FTC findings.

## 5. Questions for the team

1. **Is 6 teams the right width?** Eight would match Graph's colours but
   gets cramped on a Chromebook screen.
2. ~~Should "measure against" default to our own team when we are among the
   teams chosen?~~ Yes (2026-10-21), built.
