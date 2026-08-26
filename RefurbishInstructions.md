# TealTeam Refurbish Plan

**Status:** Draft architecture review
**Date:** 2026-08-26
**Source:** Cleaned up and expanded from `RefurbishInstructions.txt`
**Target stack:** Rust + axum 0.7 + Askama 0.12 + sqlx 0.8 + Unpoly + Tailwind, over **SQLite** (`rust/tealteam-web`), on a Raspberry Pi 5 at the event (`docker-compose.pi.yml`, `scripts/pi_*.sh`, [docs/PI_EVENT_BOOT.md](docs/PI_EVENT_BOOT.md))

**Topology:** One server, and it lives at the event. No cloud tier — Render is retired along with the Go and .NET ports, and Postgres with it.

**Port consolidation:** Three implementations currently exist against one shared schema — Go (`cmd/`, `internal/`), .NET (`dotnet/`), and Rust (`rust/tealteam-web`). **Rust is becoming the single source of truth; Go and .NET are being retired.** Everything below assumes that.

---

## How To Read This Document

Each of the five problem areas below is broken into:

| Part | What it contains |
| --- | --- |
| **Issue** | The original problem statement, cleaned up |
| **Proposed solution** | What the original notes proposed, restated precisely |
| **Verdict** | Whether the proposal survives contact with reality |
| **Technology options** | Candidate technologies with honest pros and cons |
| **Recommended architecture** | The actual design, in enough detail to build from |
| **Feasibility notes and better alternatives** | Where the original plan breaks, and what to do instead |
| **Work items** | Discrete, assignable tasks |

---

## Executive Summary

| # | Area | Original proposal | Verdict | Recommended direction |
| --- | --- | --- | --- | --- |
| 1 | Internet access | Pi 5 local server, Ethernet primary + Wi-Fi AP secondary, HaLow to pit, QR transfer | **Partly blocked** | Pi 5 and Ethernet: yes, already half-built. **Wi-Fi AP violates FRC event rules.** Use USB tethering, QR sneakernet, and a phone uplink instead. HaLow only as a Pi-to-Pi backhaul, never for client devices. |
| 2 | Poor UI | Graph view like AdvantageScope, drag-to-add metrics, separate notes menu | **Sound, under-specified** | Correct instinct. Add: assignment-driven team selection instead of lists, tap-to-add as the primary interaction (drag is a desktop metaphor), and a **season-schema-driven form system** — the `scouting_data` table is hardcoded to the 2022 game **in all three ports**. |
| 3 | Poor sync | Timestamped delta sync, courier sync via scout devices, server fan-out | **Sound, and simpler than proposed** | Use an append-only change log, not per-table watermarks — watermarks cannot see deletions and have a commit-ordering race. **Both TBA and FIRST allow direct browser requests**, so no relay server is needed: any client with signal fetches upstream itself and pushes a SQLite bundle to the Pi. ~90% of the tedious data is pulled before the event anyway. |
| 4 | Offline mode | Rewrite the client in Rust + WebAssembly | **Right direction, blocked by coupling** | The Rust port exists but is a *faithful* port — ~187 inline `sqlx::query` calls sit inside handlers and cannot cross to `wasm32`. The work is a ports-and-adapters refactor behind a `Repo` trait, not more Rust. Askama compiles to wasm, so the whole render layer comes along free. A Service Worker is still required — WASM alone does not make anything offline. |
| 5 | Poor communication | Two chat rooms in a collapsible side panel | **Sound, build it in-app** | Build it inside TealTeam rather than adopting Matrix/Mattermost, specifically so messages can be linked to a match and team. Requires an offline outbox, hybrid logical clocks for ordering, and a moderation model, because the users are minors. |

---

## Constraints That Shape Every Decision

These are the invariants. Any solution that violates one of these is dead on arrival.

### C1. You may not run your own Wi-Fi access point in the venue

This is the single most important constraint in this document, and the original notes plan around it incorrectly.

FRC event rules prohibit teams from setting up their own wireless communication in the venue. The historical wording is, in substance: *teams may not set up their own 802.11a/b/g/n/ac (2.4 GHz or 5 GHz) wireless communication, such as access points or ad-hoc networks, in the venue.* In the 2026 manual this lives in **Section 14.3, Wireless Rules (E143)**. The intent is to protect the field's radio spectrum, which is what robots use to stay in communication during matches.

Consequences:

- The "separate Wi-Fi connection point for occasional mobile users" in the original notes is **not permissible** as written.
- Personal hotspots, travel routers, and mesh nodes are all covered by the same intent.
- **Ethernet is explicitly fine.** A switch and a spool of cable is a fully compliant network.
- Enforcement varies. Some FTAs will not notice a low-power AP; some will issue a warning and make you shut it down mid-event. Designing around "we probably won't get caught" means designing a system that can be switched off by a volunteer at the worst possible moment.

> **Action:** Confirm the exact current wording of E143 in the 2026 manual before finalizing the network design, and ask the FTA at your first event what they will tolerate. Design for the compliant path; treat any allowance as a bonus.

### C2. Venue power is unreliable and the Pi has no clock

A Raspberry Pi has **no battery-backed real-time clock**. Without internet, a Pi that loses power comes back believing it is whenever it last shut down. A sync design built on timestamps that runs on a server with a wrong clock will silently corrupt the ordering of every record.

### C3. Devices are phones, and screens are small

Design target is a 360 px-wide viewport in a scout's hand, in a loud arena, possibly with gloves on, with roughly 15 seconds of attention between matches. Not a laptop.

### C4. The game changes every January

`scouting_data` currently has `hang_level`, `auto_hang`, `traversal`, and `hang_position` as columns — 2022 *Rapid React* concepts, hardcoded across all three ports. Every season these become dead columns and someone writes a migration. This is a structural problem, not a data-entry problem, and it is addressed in Section 2C.

### C5. This is a high-school team

Whatever gets built has to be maintainable by students who have not read this document, after the people who wrote it have graduated.

The language question is settled — Rust is the implementation — so this constraint is no longer about technology selection. It is about **onboarding cost**, and it shifts the obligation onto the codebase: the crate boundaries in Section 4 must be obvious, the `Repo` trait must be the only place SQL lives, and CI must fail loudly when someone puts a query somewhere it does not belong. A student should be able to add a feature without first understanding the wasm split.

It also argues for finishing the port retirement quickly. Three implementations means every feature is written three times, and it is the single largest ongoing drag on the team's throughput.

---

## 1. Internet Access and the Local Server

### Issue

The server was hosted in the cloud last season, which caused connection failures at events. Many event venues have no stable internet access. This creates a need for a local server and a way to move data to and from it.

### Proposed Solution

1. Run a primary scouting network on laptops over Ethernet.
2. Add a separate Wi-Fi connection point for occasional mobile users and for quickly pulling information.
3. Both paths need an offline mode that lets a user search existing data without querying the server.
4. Possibly use Wi-Fi HaLow to move data to the pit, with a QR code generator there as a fallback transmission method.
5. Run the server on a Raspberry Pi 5 — easy to set up, powerful enough to handle communications and a small network.

### Verdict

**Partly blocked.** The Pi 5 decision is correct and is already half-implemented in this repo. The Ethernet-primary decision is correct and is the only fully rules-compliant option. The Wi-Fi access point is **not permissible** under C1. The HaLow idea is technically real but does not do what the notes assume it does. QR transfer is the right fallback and should be promoted from "possibly" to "definitely."

### Technology Options — Server Hardware

| Option | Pros | Cons | Verdict |
| --- | --- | --- | --- |
| **Raspberry Pi 5 (8 GB) + NVMe HAT** | ~$120 all-in; already targeted by `docker-compose.pi.yml`; runs Go + Postgres for 50 clients without breaking a sweat; low power, runs off a battery bank; the team already wrote the boot scripts | No RTC (see C2); needs a real 5 V / 5 A USB-C PD supply; SD cards die from Postgres write churn | **Recommended.** Add NVMe or a USB SSD; do not run Postgres on a microSD card. |
| **A spare laptop as the server** | Zero new hardware; has a battery (survives power loss); has an RTC; easier to debug with a screen attached | Someone has to not close the lid; sleep/suspend kills the server mid-event; competes with being used for scouting or strategy | **Good backup.** Keep one imaged and ready as a hot spare. |
| **Mini PC (N100 class)** | 5-10x the CPU; real SSD; real RTC; still fanless and small | ~$150-200; needs mains power, so no battery-bank operation | Overkill for this workload, but a legitimate choice if the Pi proves flaky. |
| **Cloud-only (status quo)** | Zero event-day setup; already deployed to Render | This is the problem being solved, and it adds a tier that has to be kept in sync | **Retire it.** See Section 3 — both upstream APIs are browser-callable, so no cloud relay is needed at all. |

**Recommendation:** Pi 5 (8 GB) + NVMe HAT + a UPS HAT or USB-C battery bank + **an RTC module (DS3231, roughly $5)**. The RTC is not optional given a timestamp-based sync design.

### Technology Options — Getting Data Between Devices

This is where the original plan needs the most revision.

| Transport | Compliant? | Throughput | Client support | Verdict |
| --- | --- | --- | --- | --- |
| **Ethernet switch + cable** | Yes, unambiguously | 1 Gbps | Laptops natively; phones/tablets via a $15 USB-C-to-Ethernet adapter | **Primary transport.** Boring and correct. |
| **USB-C Ethernet adapter for phones** | Yes | 100 Mbps+ | Android: works out of the box. iPhone 15+/iPad USB-C: works. Older Lightning iPhones: needs a powered adapter | **This is the answer to "occasional mobile users"**, and it replaces the Wi-Fi AP. |
| **Wi-Fi AP on the Pi** | **No** — violates C1 | 100+ Mbps | Universal | **Do not build this** as the event path. Keep the capability for practice sessions at the shop, gated behind a config flag that defaults off. |
| **Wi-Fi HaLow (802.11ah, 900 MHz)** | Arguable — outside the named 2.4/5 GHz bands, but it is still 802.11 and an FTA will not know what it is | 150 kbps to ~15 Mbps depending on range and bandwidth | **Zero.** No phone, tablet, or laptop has a HaLow radio | **Not usable for client access.** See below. |
| **QR codes (screen to camera)** | Yes — it is light, not radio | ~2-3 KB per frame, ~10-40 KB/s with animated codes | Any device with a camera | **Build this.** It is the compliant sneakernet and it always works. |
| **USB drive / physical device rotation** | Yes | Unlimited | Universal | Ugly, reliable, worth having as the last resort. |
| **One phone's cellular data, USB-tethered to the Pi** | Yes — a phone being a phone is not "setting up wireless communication in the venue" | Whatever the carrier gives you | The Pi sees it as a USB network interface | **Best answer for intermittent internet.** See Section 3. |

#### On Wi-Fi HaLow specifically

HaLow hardware is real and buyable in 2026 — the Alfa Network AHPI7292S Pi HAT, Morse Micro MM6108 and MM8108 modules, and the Quectel FGH100M-H (available on a Seeed XIAO carrier for around $20-30) all have FCC certification. So the technology is not vaporware.

But the plan in the original notes assumes HaLow can serve scout devices, and it cannot. **No consumer phone, tablet, or laptop has a HaLow radio.** To put a scout's phone on HaLow you would need a HaLow-to-Ethernet bridge box for every single client. That is not a scouting system, that is a hardware project.

Where HaLow *does* work is exactly the narrow case the notes mention in passing: **a point-to-point backhaul link between two fixed boxes** — a Pi in the stands and a Pi (or a bridge) in the pit, where running Ethernet across a public walkway is not allowed. For that, one HaLow link at ~1-5 Mbps over a few hundred meters through a crowd is genuinely good; sub-GHz penetrates bodies and metal far better than 2.4 GHz.

Ranked honestly:

1. **Ethernet from stands to pit**, if the venue will let you tape down a cable. Free, fast, compliant. Ask first.
2. **QR sneakernet.** A scout walks to the pit anyway. Compliant, zero hardware, zero spectrum.
3. **HaLow point-to-point bridge.** ~$80 of hardware, a weekend of setup, and an unresolved rules question. Only worth it if 1 and 2 both fail.
4. LoRa — sometimes suggested for this. At 0.3-50 kbps with strict duty-cycle limits, it can carry "match 34 is starting" and nothing else. Not a data transport.

### Recommended Architecture

```text
                     ┌───────────────────────────────┐
   Before the event   │  TBA / FIRST APIs             │
   ──────────────────▶│  pulled directly on shop wifi │
   bulk load          └───────────────┬───────────────┘
                                      │  straight into the Pi's SQLite
                                      ▼
   ┌──────────────────────────────────────────────────────┐
   │  EVENT SERVER — Raspberry Pi 5                       │
   │  ┌────────────────────────────────────────────────┐  │
   │  │ axum app  :80          Postgres  (internal)    │  │
   │  │ mDNS: tealteam.local   DS3231 RTC              │  │
   │  └────────────────────────────────────────────────┘  │
   │   eth0 ──┐                          usb0 ──┐         │
   └──────────┼─────────────────────────────────┼─────────┘
              │                                 │
     ┌────────▼────────┐            ┌───────────▼──────────┐
     │ Gigabit switch  │            │ Tethered phone       │
     │ (unmanaged, 8pt)│            │ = intermittent uplink│
     └────┬───────┬────┘            └──────────────────────┘
          │       │
   ┌──────▼──┐ ┌──▼─────────────┐
   │ Laptops │ │ Phones/tablets │
   │ (RJ45)  │ │ (USB-C→RJ45)   │
   └─────────┘ └────────────────┘
          │
          │  ── no cable available? ──▶  QR sneakernet to/from the pit
          ▼
   All clients hold a full local copy (SQLite-WASM on OPFS)
   and stay fully usable when unplugged.  See Section 4.
```

Key properties:

- **Every client is offline-capable by default.** The network is an optimization, not a requirement. This is what makes the whole design robust, and it is why Section 4 is the most important section in this document.
- **`tealteam.local` via mDNS/Avahi**, so nobody types an IP address. The repo already has `scripts/pi_show_ip_lcd.py` and `pi_show_ip.sh`, which is evidence this problem has already bitten you. mDNS makes those scripts a fallback rather than the primary path.
- **One physical uplink**, USB-tethered, owned by one designated person. Not everyone's hotspot.

### Feasibility Notes and Better Alternatives

- **Drop the Wi-Fi AP from the event plan.** Keep the code behind `TEALTEAM_ALLOW_AP=false` for shop practice. Budget the money you would have spent on a travel router on USB-C Ethernet adapters instead — about $15 each, and they are also useful for everything else.
- **Buy a 25 ft flat Ethernet cable per client and a roll of gaff tape.** Flat cable under a floor mat is the difference between "the venue said yes" and "the venue said no."
- **Add the DS3231 RTC before anything else.** Two hours of work, five dollars, and it prevents an entire class of sync corruption that would otherwise be nearly impossible to debug at 9 PM on a Saturday.
- **Practice the setup.** The event-day network should be built and torn down at least twice at the shop, timed, by a student who did not design it.

### Work Items

| ID | Task | Effort |
| --- | --- | --- |
| N1 | Add DS3231 RTC to the Pi; configure `hwclock` on boot; verify clock survives a cold power cycle | S |
| N2 | Move Postgres data directory to NVMe/USB SSD; update `docker-compose.pi.yml` | S |
| N3 | Install and configure Avahi; verify `http://tealteam.local` resolves on iOS, Android, macOS, Windows | S |
| N4 | Document and script USB tethering as the Pi uplink (`usb0` interface, route metric) | M |
| N5 | Gate any AP-mode code behind a default-off flag; write the rules rationale into the docs | S |
| N6 | Build the QR transfer path (see Section 3, S5) | L |
| N7 | Write a one-page laminated event-day setup runbook with a photo of the correct cabling | S |
| N8 | Confirm E143 wording for 2026 and record the FTA conversation from your first event | S |

---

## 2. User Interface

### Issue

Several distinct problems were grouped together in the original notes. Separating them:

1. **Team lists do not scale.** With 50+ teams at an event and a need to reach any one of them within seconds, a scrolling list is a liability. Small phone screens magnify the problem.
2. **Data views are unreadable.** Information was condensed to save screen space, which made navigation and comprehension worse rather than better.
3. **Data is duplicated** — the same values appear in multiple places, and sometimes twice in the same place.
4. **The app's organizational structure causes data-entry accidents**, event selection being the worst offender.
5. **Event selection limits what the app can do** beyond just being confusing.
6. **Team metrics need a full redo** — too many hard-to-read data points, and the form changes underneath them.
7. **There is no way to tell when data was entered.**

### Proposed Solution

Build a graph view similar to AdvantageScope: a panel of metrics that can be dragged onto a chart, plus a team selector controlling which teams are displayed. Put notes and other non-quantifiable data in a separate menu. Add clear usage instructions. Data entry is season-dependent.

### Verdict

**Sound instinct, under-specified, and missing the biggest item.** The AdvantageScope-style graph view is the right model for the analysis screens. But problem 7 (no entry timestamps) and the "data entry is season dependent" aside are the two most structurally important lines in the whole section, and they need real architecture, not a UI tweak.

### 2A. Replacing the Team List

The core insight: **a team number is a 4-digit key, and a keypad beats a list of 50 items every time.** But better still is not making the user pick at all.

| Approach | Pros | Cons | Use as |
| --- | --- | --- | --- |
| **Assignment-driven** — the lead scout assigns you a robot; the app shows only that robot | Zero selection cost; eliminates wrong-robot data entry outright; matches the "babysitting" goal from the original notes | Requires the lead-scout assignment system to exist and be kept current | **Primary path** |
| **Match-context chips** — pick the match, get 6 big touch targets | Two taps; naturally correct; readable at arm's length | Only works when scouting a scheduled match | **Secondary path** |
| **Numeric keypad + type-ahead** — type `16`, see `16, 166, 1619, 1678...` narrowing live | Fast for someone who knows the number; no scrolling; big touch targets | Requires knowing the number | **Escape hatch** |
| **Searchable list with fuzzy match** | Familiar | This is what you have; it is the thing that is broken | **Last resort only** |

Recommended interaction, in priority order:

```text
┌─────────────────────────────────────┐
│  YOU ARE SCOUTING                   │   ← assignment-driven, no choice to make
│  ┌───────────────────────────────┐  │
│  │   1678   Q34   RED 2          │  │
│  │   Citrus Circuits             │  │
│  └───────────────────────────────┘  │
│  [ Start scouting ]                 │
│                                     │
│  Not your robot?  [ Change ]  ──────┼──▶  keypad + type-ahead
└─────────────────────────────────────┘
```

For the fuzzy search when it is needed, a hand-rolled prefix scorer over 50-300 teams is roughly 30 lines and beats pulling in a dependency. If you want a library, **Fuse.js** (~12 KB gzipped, no deps) is the reasonable pick; **MiniSearch** is a better fit if you later want to search notes text too.

### 2B. The Graph View

AdvantageScope's model — a field list on the left, drag a field onto a chart, chart updates — is a good target. Charting library options:

| Library | Size (gzip) | Pros | Cons | Verdict |
| --- | --- | --- | --- | --- |
| **uPlot** | ~15 KB | Extremely fast (canvas); handles thousands of points; no framework needed; tiny | Minimal built-in interactivity; you write your own legend, tooltips, and drag targets | **Recommended.** Smallest JS surface to maintain alongside a Rust/Unpoly app — feed it JSON from a handler, or call it from wasm via `wasm-bindgen`. |
| **Apache ECharts** | ~150-400 KB (custom builds smaller) | Brush select, data zoom, drag, rich tooltips all built in; excellent docs | Large; a big API surface for students to learn | **Strong alternative** if you want interactivity for free and can accept the bundle. |
| **Chart.js** | ~60 KB | Easiest to learn; huge community | Slower with many series; awkward for the drag-to-add model | Fine, but you will outgrow it. |
| **D3 / Observable Plot** | ~30-90 KB | Total control; Plot is genuinely concise | Steepest learning curve; easiest to write unmaintainable code in | Only if someone on the team already knows it. |
| **Recharts / Victory / Nivo** | 100 KB+ | Nice defaults | **Require React.** This codebase has no React | **Rejected** — would force a framework decision for a chart. |

**Recommendation: uPlot** for per-match trend lines, plus small hand-written SVG components for the bar and scatter comparisons. SVG for the comparison views is deliberate — 50 bars is nothing, and SVG elements are inspectable, accessible, and stylable with the Tailwind you already use.

#### Drag-and-drop, and why it should not be the primary interaction

HTML5 native drag-and-drop **does not work on touch devices**. If you build drag-to-add with the native API, it will work on the strategist's laptop and be completely dead on every scout's phone.

Options: **SortableJS** (~13 KB, no deps, correct touch support via Pointer Events) is the right library if you want drag. `dnd-kit` and `react-beautiful-dnd` are React-only and therefore out.

But the better alternative is to **question the requirement**. Drag-to-add is a desktop metaphor from a desktop tool. On a phone, tapping a metric chip to toggle it on the chart is faster, more discoverable, needs no tutorial, and cannot be fumbled. Recommended: **tap-to-toggle is the interaction; drag is an enhancement that reorders series on pointer-capable devices only.** This also quietly satisfies the "add clear instructions on how to use it" requirement, because a toggle chip needs no instructions.

```text
┌──────────────────────────────────────────────────────────┐
│ Teams:  [1678 ×] [254 ×] [971 ×]        [+ add team]     │
├──────────────────────────────────────────────────────────┤
│ Metrics:  (tap to toggle)                                │
│  ●Auto pts  ○Teleop pts  ●OPR  ○DPR  ○Cycle time  ○Climb │
├──────────────────────────────────────────────────────────┤
│                                                          │
│   [ uPlot chart — one line per team per active metric ]  │
│                                                          │
├──────────────────────────────────────────────────────────┤
│  Source: scouting (n=12) + TBA           Synced 4m ago   │  ← always show provenance
└──────────────────────────────────────────────────────────┘
```

### 2C. Season-Dependent Data Entry — the structural fix

This is the item that will pay for itself every January.

Right now the schema has season-specific columns baked into `scouting_data`: `hang_level`, `auto_hang`, `hang_position`, `traversal`. Those are 2022 *Rapid React* concepts, and they are hardcoded in [rust/tealteam-web/src/scouting_points.rs](rust/tealteam-web/src/scouting_points.rs), [rust/tealteam-web/src/models.rs](rust/tealteam-web/src/models.rs), and their Go and .NET equivalents.

Every new game currently means a migration, new struct fields, new form HTML, new template branches, and new aggregation code — **times three ports**. The old columns never get removed, which is part of why the data views are cluttered and duplicated. Retiring Go and .NET cuts this by two-thirds; making the season a data file removes it almost entirely.

**Recommended: a schema-driven form and storage system.**

Define a season as data, not code:

```jsonc
// seasons/2026-rebuilt.json
{
  "season": 2026,
  "name": "REBUILT",
  "version": 3,
  "phases": [
    {
      "id": "auto",
      "label": "Autonomous",
      "fields": [
        { "id": "auto_leave",  "label": "Left start zone", "type": "bool",    "widget": "big-toggle" },
        { "id": "auto_scored", "label": "Pieces scored",   "type": "int",     "widget": "counter",
          "min": 0, "max": 12, "weight": 4 }
      ]
    },
    {
      "id": "teleop",
      "label": "Teleop",
      "fields": [
        { "id": "cycles",   "label": "Cycles",  "type": "int",  "widget": "counter", "weight": 3 },
        { "id": "defense",  "label": "Defense", "type": "enum", "widget": "segmented",
          "options": ["none", "some", "heavy"] }
      ]
    }
  ]
}
```

Storage becomes:

```sql
-- scouting_data keeps identity + provenance as real columns,
-- and moves game-specific values into JSONB
ALTER TABLE scouting_data ADD COLUMN schema_version int;
ALTER TABLE scouting_data ADD COLUMN payload jsonb NOT NULL DEFAULT '{}';

-- Hot metrics get generated columns so they stay indexable and typed:
ALTER TABLE scouting_data
  ADD COLUMN auto_points int GENERATED ALWAYS AS
    ((payload->>'auto_scored')::int * 4) STORED;

CREATE INDEX ON scouting_data USING gin (payload jsonb_path_ops);
```

| | Pros | Cons |
| --- | --- | --- |
| **Wide typed columns (status quo)** | Type safety; simple SQL; easy indexes | A migration and a code change every season; dead columns accumulate forever; the form and the schema drift apart |
| **JSONB payload** | New season = a new JSON file, no migration, no deploy; the form renders itself; old seasons stay queryable | Loses column-level constraints; queries are wordier; a typo in the JSON is a runtime error not a compile error |
| **JSONB + generated columns (recommended)** | Flexibility where the game changes, typed and indexed where the app actually queries | Slightly more moving parts; generated columns still need a migration when the *scoring formula* changes |

The generated-column hybrid is the right trade. The schema file also becomes the single source of truth for the form, the validation, the CSV export headers, the metric picker in the graph view, **and** the point weights that `internal/handlers/scouting_points.go` and `lead_scout_weights.go` currently manage separately.

### 2D. Data Provenance and Freshness

Problem 7 — "no clear way for users to determine when data was entered" — is solved by making provenance a first-class, always-visible property rather than a detail page.

Every displayed number carries three facts:

| Fact | Source | Displayed as |
| --- | --- | --- |
| **When was it observed?** | `scouted_at` (already exists on the model) | `Q34 · 2:14 PM` |
| **Who observed it?** | `scouter_id`, `submitting_team_id` (already exist) | avatar or initials |
| **How stale is our copy?** | client-side `last_synced_at` per table | `Synced 4m ago` in the status bar |
| **How much is there?** | count of contributing records | `n=12` next to any average |

The `n=` badge deserves emphasis. An average built from one match and an average built from twelve look identical today, and that is actively misleading to a drive coach making a pick.

### 2E. Deduplication and Event Context

- **One aggregate, one partial.** Build a single `TeamProfile` view model in Go, assembled in one place, and render it through one set of partials that every screen reuses. The duplication in the current UI comes from several handlers each assembling their own slightly different version of "team stats."
- **Event context belongs in a persistent header switcher**, not buried in a settings flow. The `sessions.selected_event_id` column already exists — surface it. Show the current event name at all times, make switching two taps, and make it obvious that switching changes what everything else shows.
- **Allow multi-event and historical views** in analysis screens specifically. The "event selection was very limiting" complaint is really "the app assumes exactly one event exists." Scoping *data entry* to one event is correct and prevents mistakes; scoping *analysis* to one event is what makes it limiting. Split the two.

### 2F. Mobile-First Ground Rules

- Design at 360 px. If it does not work there, it does not work.
- Touch targets 44 × 44 px minimum, 56 px for anything used during a match.
- **Primary navigation at the bottom** (thumb zone), not the top.
- Counters get large `+` / `−` buttons, never a text input with a spinner.
- No horizontal scrolling, ever. Tables become cards below 600 px.
- Destructive actions require a deliberate second action, and there is always an undo.

### Work Items

| ID | Task | Effort |
| --- | --- | --- |
| U1 | Design the season schema format; write `seasons/2026.json` | M |
| U2 | Add `payload jsonb` + `schema_version` to `scouting_data`; backfill existing rows | M |
| U3 | Build the generic schema-driven form renderer (replaces `scouting_form.html` branching) | L |
| U4 | Replace team-list selection with assignment-driven UI + keypad escape hatch | M |
| U5 | Build the graph view: uPlot + tap-to-toggle metric chips + team chips | L |
| U6 | Build the notes panel as a separate, filterable, timestamped view | M |
| U7 | Add provenance badges (`n=`, `scouted_at`, `synced ago`) to every aggregate | S |
| U8 | Consolidate team stats assembly into one `TeamProfile` view model | M |
| U9 | Move event selection into a persistent header switcher; allow multi-event analysis | M |
| U10 | Mobile pass: bottom nav, 44 px targets, card layouts under 600 px | M |

---

## 3. Sync

### Issue

The app syncs data from third-party sources (FIRST Events API, The Blue Alliance) to avoid burdening a small scouting team with collecting it by hand. With an offline system, that data cannot be pulled during the event. The data is tedious to collect and organize manually.

### Proposed Solution

Sync with the internet, then store the data on the local web server. Timestamp each sync and pull only new data since the last one. Part of the sync happens on the user's device, so a scout can leave the venue, get service, sync and store data, come back, reconnect, and push to the server. The server then pushes to all clients.

### Verdict

**Sound, and over-scoped in one place.** The timestamped delta sync and the server fan-out are correct and should be built. The courier-sync path works, but it solves a smaller problem than it appears to, for the reason below — and there is a much easier option the notes do not consider.

### The insight that shrinks the problem

Most of the third-party data does not change during the event:

| Data | Changes during event? | Can be pre-fetched? |
| --- | --- | --- |
| Team list, names, rookie years | No | **Yes** |
| Event list, dates, venues | No | **Yes** |
| Historical OPR / stats from prior events | No | **Yes** |
| Qual match schedule | Published before quals start; rarely edited | **Mostly** |
| Rankings | Yes, after every match | No |
| Match results and scores | Yes | No |
| Playoff alliances and bracket | Yes, Saturday | No |

So a **bulk pre-event pull** — run the night before, over the shop's wifi, into the Pi's Postgres — covers the large, tedious, structural data. What actually needs live sync is rankings and results, which are small (a few KB per refresh), and which are *also displayed on the venue's audience screens*, so a total sync failure is inconvenient rather than fatal.

This reorders the priorities: **build the pre-event bulk pull first.** It is easy, and it delivers most of the value.

### Technology Options — Live Uplink

| Option | Pros | Cons | Verdict |
| --- | --- | --- | --- |
| **Phone USB-tethered to the Pi** | Compliant (C1); continuous; no human in the loop; one person's data plan; the Pi just sees a network interface | Coverage in a metal arena can be poor; drains that phone; someone must remember to plug it in | **Recommended primary.** |
| **Venue guest wifi, when it exists** | Free; sometimes genuinely fine | Frequently absent, captive-portal'd, or saturated | Opportunistic. Try it, do not plan on it. |
| **Courier sync** (scout walks outside, syncs, returns) | Works with zero infrastructure; already in the plan | Human latency measured in tens of minutes; requires a person to leave; needs API credentials or a relay (see below) | **Build, but as the fallback.** |
| **Nothing — pre-event pull only** | Simplest possible | Rankings go stale; playoff data never arrives | Acceptable for a first event. Genuinely. |

#### The credential problem with courier sync

The naive courier design puts FIRST/TBA API keys on scouts' phones so their devices can call the APIs directly. Do not do this — those keys are team credentials, phones get lost, and browser storage is not a secret store.

The resolution is simpler than a relay, and it is the subject of the next section: **both upstream APIs accept browser requests directly**, so a device with signal can fetch for itself. The keys are read-only credentials for public data, which makes that an acceptable trade rather than a compromise:

```text
  TBA / FIRST APIs
        │
        ▼
  TBA / FIRST ──── public APIs, CORS-enabled, ETag-cacheable
        │
        │  scout walks outside, phone gets signal,
        │  pulls a signed delta bundle
        ▼
  Scout's phone (signed SQLite bundle) ──walks back in──▶  Pi  ────▶  all clients
```

No intermediate server, and no designated courier: **any client that finds signal updates everyone.** The details, and the honest accounting of what dropping the cloud costs, follow below.

### Recommended Architecture — Two Logs, No Cloud

**There is no cloud tier.** The only server is the Pi at the event. Render is retired along with the Go and .NET ports.

This is possible because of a fact worth verifying yourself and then relying on: **both upstream APIs are callable directly from a browser.**

```
$ curl -I -X OPTIONS -H "Origin: https://example.com" \
    -H "Access-Control-Request-Method: GET" \
    -H "Access-Control-Request-Headers: X-TBA-Auth-Key" \
    https://www.thebluealliance.com/api/v3/status

  access-control-allow-origin:    https://example.com
  access-control-allow-headers:   X-TBA-Auth-Key
  access-control-allow-methods:   GET, OPTIONS
  access-control-expose-headers:  ETag          ← conditional requests work too

$ curl -I -X OPTIONS -H "Origin: https://example.com" \
    https://frc-api.firstinspires.org/v3.0/2026/teams

  Access-Control-Allow-Origin:   *
  Access-Control-Allow-Methods:  GET
```

TBA reflects the origin, explicitly permits the auth header, and — importantly — **exposes `ETag` to browser JavaScript**, so a client can do conditional polling and spend almost no cellular data. FIRST allows any origin.

So no proxy is needed, and the design collapses to something much simpler:

> **Whichever node currently has internet fetches upstream data. Everything merges on the Pi.**

Upstream fetching stops being a *tier* and becomes a *role* that any node can take:

| Node | Fetches upstream when | How |
| --- | --- | --- |
| **Pi** | On shop wifi the night before; via USB tether at the venue | Native `tba.rs` / `first_api.rs` |
| **Any client** | Whenever its browser has signal — outside the venue, on cellular, at the hotel | **The same `tba.rs`, compiled to wasm** |

That second row is the payoff of the Section 4 split. [tba.rs](rust/tealteam-web/src/tba.rs) and [first_api.rs](rust/tealteam-web/src/first_api.rs) are two of the four modules that already have zero pool references — they compile to `wasm32` unchanged, and `reqwest`'s wasm backend runs them over `fetch`. One implementation, two targets, no proxy, no second codebase.

#### Two logs, two cursors

Upstream data and venue data stay separate streams, because they have opposite ownership:

| | Upstream stream | Venue stream |
| --- | --- | --- |
| Origin | TBA / FIRST, fetched by whoever has signal | Scouts' devices |
| Authority at the venue | **Read-only.** Nobody edits it | Authoritative |
| Conflict rule | Last-write-wins; it is derived public data | Append-only |
| If it is hours stale | Fine. Rankings are just old | Not fine |

Mixing them creates a false dependency where the Pi cannot accept a scouting submission because it is behind on rankings. Keep them apart.

```text
        TBA / FIRST  (public, CORS-enabled, ETag-cacheable)
                 ▲                          ▲
   native fetch  │                          │  wasm fetch, when the
   (shop wifi,   │                          │  device has any signal
    USB tether)  │                          │
   ┌─────────────┴──────────┐   push    ┌───┴──────────────────────┐
   │  PI — SQLite           │ ◀──────── │  Clients — SQLite/OPFS   │
   │  authoritative merge   │ ────────▶ │  full local replica      │
   │  `changes` + `upstream`│   SSE     └──────────────────────────┘
   └────────────────────────┘
```

With an append-only log on each side, `sync_state` is one row per remote — no per-table watermarks, and deletions are representable:

```sql
CREATE TABLE sync_state (
  source     text    PRIMARY KEY,   -- 'upstream' | 'pi'
  cursor     integer NOT NULL,
  applied_at text    NOT NULL
);
```

### Getting Upstream Data In

Because every node runs SQLite, a delta is not JSON with a bespoke importer — it is a small SQLite database holding only the changed rows. Ingest is `ATTACH`:

```sql
ATTACH '/tmp/upstream-84213.sqlite' AS up;

INSERT INTO team_event_stats SELECT * FROM up.team_event_stats
  WHERE true
  ON CONFLICT(team_id, event_id) DO UPDATE SET
    opr = excluded.opr, rank = excluded.rank /* ... */;

UPDATE sync_state SET cursor = (SELECT to_seq FROM up.meta) WHERE source = 'upstream';
DETACH up;
```

**The transport becomes irrelevant.** The same file works whether it arrived over a tethered phone, from a client that had signal, off a USB stick, or reassembled from QR frames. One import routine, many transports, no serialization format to maintain.

#### The transports, in priority order

| Transport | Covers | When |
| --- | --- | --- |
| **1. Pre-event bulk load** | Teams, events, schedules, historical stats — roughly 90% of the tedious data | The night before, Pi on shop wifi. A full snapshot, not a delta. |
| **2. USB-tethered phone → Pi** | Live rankings, results, playoff alliances | Continuously, whenever there is signal. The normal case. |
| **3. Opportunistic client fetch** | Same as 2 | Any scout's device that finds signal — no special hardware, no designated courier |
| **4. QR frames** | Rankings only, a few KB | When nothing else works |
| *(5. Manual entry)* | Rankings typed off the audience display | Genuinely last resort. Five minutes, never fails. |

Transport 3 is worth dwelling on. There is no courier *role* and nobody has to leave the venue on purpose. The client already holds `tba.rs`; when its browser notices connectivity it fetches conditionally, writes into its local SQLite, and pushes on reconnect. A scout who steps into the lobby for two minutes has silently updated everyone.

#### About the API keys

Putting a TBA key on client devices is fine, and I would not contort the architecture to avoid it. Both keys are **read-only credentials for public data** — the realistic worst case if one leaks is rate-limit abuse, not a data breach, and TBA keys are free and revocable from the account dashboard.

Sensible hygiene rather than paranoia:

- The Pi hands the key to **authenticated** clients at sync time; it is not baked into the wasm bundle or committed.
- Only lead-scout-role devices may push an upstream bundle, and the Pi logs who pushed what.
- Rotate the key between seasons.

Since the Pi authenticates the pusher and upstream rows are idempotent last-write-wins, **cryptographic bundle signing is not needed** — the earlier Ed25519 scheme was protecting a trust boundary that no longer exists once there is no third party. If you want defence against a bored student submitting fake rankings, the proportionate answer is role-gating plus an audit trail, and the Pi re-verifying against TBA the next time it has internet itself.

#### What you actually give up by dropping the cloud

Two real losses, both cheaply mitigated. Naming them so nobody is surprised:

| Loss | Mitigation |
| --- | --- |
| **No continuous polling when every device is offline overnight** | Irrelevant. Nothing changes at 3 AM, and the pre-event bulk load covers the gap. |
| **No off-site backup.** The Pi is now the only authoritative copy | The database is a *file*. `rsync` it to the lead scout's laptop between match blocks, plus a USB stick. Every client also holds a full replica — see Backups under Cross-Cutting Concerns. |
| No always-on URL for looking at data from home | Run the same binary locally. It is one file and one binary. |

The upside is that **Postgres disappears entirely.** SQLite on the Pi, SQLite in the browser, one schema, one dialect, one set of queries — the translation boundary noted in Section 4 is not merely contained, it is gone.

### Conflict resolution, by data class

| Data class | Authority | Rule |
| --- | --- | --- |
| TBA/FIRST derived data (rankings, schedules, OPR) | Upstream | **Last-write-wins.** It is derived; the newest copy is correct by definition. |
| Scouting submissions | Append-only | **No conflicts by construction.** A record is keyed by `(event, match, team, scouter, client_record_id)`. Two scouts on the same robot produce two records, both kept, and the lead scout arbitrates. |
| Pick list ordering | Collaborative | **Genuinely conflicting.** See Section 4 — this is the one place a CRDT earns its keep. |
| Lead-scout assignments | Single writer | **Server-authoritative**, with the lead scout's device winning ties. |

Classifying the data this way is what lets you avoid a general-purpose conflict resolution system. Only one of four classes actually needs one.

**Idempotency.** Every client-originated record gets a client-generated **UUIDv7** (time-ordered, so it sorts usefully and indexes well) as `client_record_id`, with a unique constraint. Re-pushing a queued submission after a flaky reconnect is then a no-op rather than a duplicate. This is the single cheapest thing you can do to make offline sync trustworthy.

**Fan-out to clients: SSE vs WebSockets vs polling**

| | Pros | Cons | Verdict |
| --- | --- | --- | --- |
| **Server-Sent Events** | One-directional, which is exactly the shape of the problem; plain HTTP; auto-reconnect with `Last-Event-ID` built into the browser; `axum::response::Sse` with a `tokio` broadcast channel is a few dozen lines | Text only; 6-connection-per-origin limit on HTTP/1.1 (irrelevant here) | **Recommended.** |
| **WebSockets** | Bidirectional; lower per-message overhead; needed for typing indicators and presence | You write your own reconnect, heartbeat, and backoff; more code to get wrong | Adopt only if the chat features in Section 5 demand it. |
| **Polling every 15-30 s** | Simplest possible; stateless; survives anything | Latency; wasteful, though on a LAN nobody cares | **Good enough fallback** — and worth keeping as the degraded mode. |

Start with SSE for push and keep polling as the automatic fallback when the event stream will not stay open.

### QR Transfer — Architecture

Promote this from "possibly" to a real subsystem, because it is the only transport that cannot be taken away from you.

**Payload sizing.** One scouting record is roughly 200-500 bytes of JSON. Compressed with gzip and encoded in QR alphanumeric mode, a single version-40 QR code holds ~4,296 alphanumeric characters — comfortably 8-15 records per frame.

**For larger transfers, use animated QR with fountain coding.** A plain animated sequence fails if the camera misses frame 7; a fountain code (LT or RaptorQ) lets the receiver reconstruct from *any* sufficient subset of frames, so the sender just loops forever and the receiver stops when it has enough. This is how crypto wallets move signed transactions across air gaps, and it is well-proven.

| Layer | Recommended | Alternatives |
| --- | --- | --- |
| Encode (Go) | `skip2/go-qrcode` | `boombuler/barcode` |
| Fountain code | Port `txqr`-style LT coding, or ship plain chunked frames first | RaptorQ if you need the efficiency, which you probably do not |
| Decode (browser) | **`BarcodeDetector` API** where available — native, fast, zero bundle | `zxing-wasm` or `jsQR` as fallback |

**Browser support caveat:** `BarcodeDetector` is available in Chrome and Edge on Android, ChromeOS, and macOS, but **not in Safari on iOS**. Ship `zxing-wasm` (~300 KB, loaded lazily only when the scanner opens) as the fallback so iPhones work. Test on an actual iPhone before the event.

**Ergonomics matter more than the codec.** Show a progress ring (`38 / 60 frames`), keep the screen at full brightness, and give an unmistakable success state. A scanner that silently half-works is worse than one that fails loudly.

### Feasibility Notes and Better Alternatives

- **Build the pre-event bulk pull first.** It is an afternoon of work and it covers the majority of the tedious data the notes are worried about.
- **Courier sync is worth less than it looks.** It only carries rankings and results, and only in tens-of-minutes batches. The USB-tethered phone does the same job continuously for zero human effort. Build the courier path, but build it second, and be honest that it may never get used.
- **Consider a manual rankings entry screen** as the true last resort. A lead scout can type 40 ranking rows off the audience display in five minutes. It is unglamorous and it has never once failed to work.
- **Do not adopt a general sync framework for this.** ElectricSQL and PowerSync are excellent and are discussed in Section 4, but the sync described here — three channels, four data classes, three of which have trivial conflict rules — is a few hundred lines of Go you fully understand. That is the right call for C5.

### Work Items

| ID | Task | Effort |
| --- | --- | --- |
| S1 | *(Pi → SQLite migration is O19; it gates S2–S5)* | — |
| S2 | `upstream` append-only log on the Pi, fed by the existing TBA/FIRST clients | M |
| S3 | Compile `tba.rs` / `first_api.rs` for `wasm32`; client-side conditional fetch with ETags | M |
| S4 | Bundle import on the Pi: role-gate the push, `ATTACH`, upsert, advance cursor, audit-log | M |
| S5 | Pre-event bulk load — full snapshot, one command, verifiable row counts | S |
| S6 | USB tether as the Pi's automatic uplink; pull bundles whenever `usb0` is up | M |
| S7 | Opportunistic client fetch: detect signal, fetch upstream, queue bundle, push on reconnect | M |
| S8 | Add `client_record_id` (UUIDv7) + unique constraint to client-originated tables | S |
| S9 | SSE fan-out endpoint with `Last-Event-ID` resume; polling fallback | M |
| S10 | ETag / conditional requests on the Pi's TBA poller | S |
| S11 | Upstream freshness badges; amber past 20 minutes during quals | S |
| S12 | QR transfer: Rust encoder, browser scanner with `BarcodeDetector` + zxing-wasm fallback | L |
| S13 | Manual rankings entry screen (last-resort path) | S |

---

## 4. Offline Mode and the WASM Client

### Issue

1. Scouts cannot always reach the server.
2. There was confusion about what "offline mode" even was, and what worked while it was active.
3. **An accidental page reload made the page disappear or lose functionality**, because the app could not reach the server to re-render it.
4. The app needs to keep most of its functionality while disconnected, reconnect automatically, sync new data, and resolve conflicts.

### Proposed Solution

Move the app to be mostly or entirely a client-side web app, written in Rust compiled to WebAssembly. The server is then used only to relay between clients and act as a light source of truth.

### Verdict

**Correct, and already half-done — but not in the way it looks.**

`rust/tealteam-web` is a complete axum + Askama + sqlx port, ~6,400 lines, and it is becoming the single implementation (Go and .NET are being retired). That decision is settled and this section assumes it.

What has to be said plainly: **the Rust port does not currently move you any closer to running in the browser.** It is a *faithful* port — same routes, same handler shapes, same inline SQL — so it inherited the Go app's server-shaped architecture along with its behavior. There are roughly **187 `sqlx::query` call sites living directly inside handler functions** (48 in `assignments.rs` alone, 33 in `lead_scout.rs`). None of them can cross to `wasm32-unknown-unknown`, because there is no Postgres in a browser tab and no TCP socket to reach one.

The language was never the blocker. **The coupling is.** The good news is that Rust leaves you far better positioned than Go or C# did, for three specific reasons.

### What already compiles to `wasm32`, unchanged

These modules contain **zero** pool references and are portable as-is:

| File | Lines | Contents |
| --- | --- | --- |
| [tba.rs](rust/tealteam-web/src/tba.rs) | 274 | Blue Alliance client and response types |
| [first_api.rs](rust/tealteam-web/src/first_api.rs) | 254 | FIRST Events client and response types |
| [connectivity.rs](rust/tealteam-web/src/connectivity.rs) | 179 | Connectivity and API health state |
| [models.rs](rust/tealteam-web/src/models.rs) | 152 | Entity structs |

Plus two things that matter more than the line counts:

- **Askama is compile-time and zero-runtime.** All 24 templates become generated Rust code implementing `Display`. The entire rendering layer runs in the browser with no template engine shipped and no changes made. Neither `html/template` nor Razor would have given you this.
- **The view-model logic already inside your handlers is pure.** `MatchAssignmentRow::label()`, `scheduled_display()`, the `scouting_points` weight math — all of it ports untouched.

### What blocks, and why

| Dependency | `wasm32`? | Notes |
| --- | --- | --- |
| `sqlx` (postgres, runtime-tokio) | **No** | No TCP in the browser. This is the whole problem. |
| `axum`, `tower-http` | **No** | Server transport. Stays server-side by design. |
| `tokio` (`features = ["full"]`) | **Partial** | Only `sync`, `time`, `macros` build for wasm. No `net`, no `fs`, no multi-thread runtime. |
| `reqwest` | **Yes** | Has a `wasm32` backend over `fetch` — but your `rustls-tls` feature is native-only, so it needs `cfg`-gated features. |
| `bcrypt` | Compiles | Deliberately slow; irrelevant, because password verification must stay server-side anyway. |
| `askama`, `serde`, `serde_json`, `chrono`, `html-escape`, `once_cell` | **Yes** | Clean. `rand` needs `getrandom`'s `js` feature. |

### Recommended Architecture — Ports and Adapters

Split the single crate into a workspace. The rule is simple: **`tt-core` and `tt-templates` may not depend on `sqlx` or `axum`.** Enforce it in CI with a `wasm32` build.

```text
crates/
  tt-core/        pure domain: models, season schema, scoring, validation,
                  aggregation, view models, conflict rules
                  ── builds for native AND wasm32 ──
  tt-templates/   Askama templates + render functions (depends on tt-core)
                  ── builds for native AND wasm32 ──
  tt-repo/        the `Repo` trait — every query the app can make, as methods
  tt-repo-pg/     sqlx/Postgres impl                      ── server only ──
  tt-repo-sqlite/ SQLite-WASM over OPFS impl              ── browser only ──
  tt-server/      axum routes, sessions, TBA/FIRST sync, sync endpoints
  tt-client/      wasm-bindgen entry, service worker glue, outbox, sync client
```

Handlers become generic over the repository and stop caring where data lives:

```rust
pub async fn team_event_data<R: Repo>(repo: &R, team_id: i32, event_id: i32)
    -> Result<String, AppError>
{
    let profile = repo.team_profile(team_id, event_id).await?;   // swappable
    Ok(TeamDataTemplate { profile }.render()?)                    // shared Askama
}
```

#### Gotcha: `Send` bounds on the trait

This will bite on day two, so plan for it. axum spawns futures across threads and needs `Send`; wasm futures are single-threaded and `!Send`. One trait cannot satisfy both with a plain `async fn`.

Use the [`trait-variant`](https://crates.io/crates/trait-variant) crate, which exists for exactly this:

```rust
#[trait_variant::make(Repo: Send)]      // generates a Send variant for the server
pub trait LocalRepo {
    async fn team_profile(&self, team_id: i32, event_id: i32) -> Result<TeamProfile>;
    async fn matches_for_event(&self, event_id: i32) -> Result<Vec<MatchRow>>;
    // ... one method per query the app makes
}
```

Server code uses `Repo`, wasm code uses `LocalRepo`, and you write the signatures once.

#### Browser storage: use SQLite, not IndexedDB

This is the decision that determines how much of your 187 query sites survive.

| Option | Pros | Cons | Verdict |
| --- | --- | --- | --- |
| **SQLite-WASM on OPFS** (`sqlite-wasm-rs`, or `wa-sqlite` via JS interop) | **Your SQL mostly ports as-is** — real joins, real aggregates, real `ON CONFLICT`; this is literally the "search through data without querying the server" requirement | ~1 MB wasm blob (cached once); Postgres and SQLite dialects diverge in places | **Recommended** |
| **IndexedDB** (`rexie`, `idb`, raw `web-sys`) | Smaller; no extra blob; universally supported | Key-value only — you hand-write every join and aggregate in Rust. Your queries are relational | Fallback only |

Dialect differences you will hit: `NOW()` → `datetime('now')`, `::int` casts → `CAST(… AS INTEGER)`, `SERIAL` → `INTEGER PRIMARY KEY AUTOINCREMENT`. `ON CONFLICT` and `RETURNING` work in both. Keeping the two dialects close is a real, ongoing tax — budget for it, and consider authoring new queries in the portable subset from the start.

### The Migration Path — and why Unpoly makes it easy

Here is the part that makes this genuinely incremental rather than a big-bang rewrite, and it falls out of your recent HTMX → Unpoly swap.

Unpoly fetches HTML fragments over HTTP. It does not care who produced them. So:

**A Service Worker intercepts `/hx/*` fragment requests. When the server is unreachable, it hands the request to the wasm module, which runs the same handler against `tt-repo-sqlite` and returns the same HTML.** Unpoly never knows the difference. No template changes, no UI rewrite, no client-side router.

```text
   Unpoly requests /hx/teams/data?team=1678
            │
            ▼
   ┌─────────────────────┐   online   ┌──────────────────────────┐
   │   Service Worker    │───────────▶│ Pi: axum + tt-repo-pg    │
   │  (routing decision) │            └──────────────────────────┘
   └──────────┬──────────┘
              │ offline / timeout
              ▼
   ┌─────────────────────────────────────────────┐
   │ wasm: same handler fn                       │
   │   tt-core + tt-templates + tt-repo-sqlite   │
   │   → identical HTML fragment                 │
   └─────────────────────────────────────────────┘
```

**Migrate one route at a time**, read-only first, because those are safe and they satisfy the original "search data without querying the server" requirement:

1. `/hx/teams/search`, `/hx/teams/data` — team lookup and profiles
2. `/hx/matches/schedule`, `/hx/drive-coach/matches` — schedules
3. `/hx/events/summary`, `/lead-scout/assignments` — event and assignment views
4. `/submission` (POST) — writes, via the outbox in Section 3
5. Everything else, or never — some routes are fine as online-only

At every point in this migration the app still works. That property is worth more than any architectural elegance, because it means you can stop when you run out of time before kickoff.

### What stays server-only, permanently

Not everything should move, and being explicit about this prevents scope creep:

- **Password verification and session issuance.** Never verify a bcrypt hash client-side.
- **FIRST and TBA API credentials**, and the sync loops that use them.
- **The authoritative database** and all conflict arbitration.
- **Submission approval** — a lead scout approving into `scouting_data` is a source-of-truth write.

### WASM still does not make it offline

This point survives from the original analysis unchanged, and it is the reason the Service Worker is a prerequisite rather than an alternative.

A wasm module fetched over the network is exactly as dead on a failed reload as a JavaScript one. Issue 3 — reload kills the page — is fixed by a Service Worker precaching the shell, not by the compilation target:

```js
// navigation requests fall back to the cached shell when the network fails
self.addEventListener('fetch', (event) => {
  if (event.request.mode === 'navigate') {
    event.respondWith(
      fetch(event.request).catch(() => caches.match('/app-shell.html'))
    );
  }
});
```

Pair it with **debounced form-state persistence** — write the in-progress scouting form to storage on every input, restore on load. Roughly 40 lines, and it means a dropped phone or a fat-fingered reload costs nothing. Do this first, before any of the workspace refactor. It is the single highest-value fix in this document.

### Bundle size

Not a concern in your deployment, and worth stating so nobody relitigates it. Your `[profile.release]` already sets `lto = true` and `strip = true`; add `opt-level = "z"` and run `wasm-opt -Oz` for the wasm target. Expect roughly 300–800 KB gzipped for `tt-core` + Askama + serde, plus ~1 MB for SQLite-WASM.

The Pi is three feet away on gigabit Ethernet and the Service Worker caches the blob once per device per deploy. Public-internet bundle-size reasoning does not apply to a private LAN.

### Naming the connection states

Issue 2 was "confusion about what offline mode was." The fix is to stop treating offline as a mode the user toggles and start treating it as an observed state the app reports. [connectivity.rs](rust/tealteam-web/src/connectivity.rs) and `network_status_badge.html` already exist — extend them to four states:

| State | Chip | Meaning to the user |
| --- | --- | --- |
| `SYNCED` | 🟢 Synced | Everything you've entered is on the server. |
| `SYNCING` | 🔵 Syncing… | Connected, catching up. |
| `OFFLINE` | 🟡 Offline · 4 saved | No server. **Your work is saved.** It will send when you reconnect. |
| `CONFLICT` | 🔴 3 need review | Something needs a human. Tap here. |

The wording matters. "Offline · 4 saved" tells a scout their work is safe. "Offline" alone makes them think it was lost, and then they write it on their hand, which is what happened last season.

There is no offline *toggle*. The app is always local-first; the network is an optimization it uses when present.

### Offline Authentication

Sessions are DB-backed, so a disconnected client cannot currently establish who the user is. Note that **device identity is already solved** — [device.js](rust/tealteam-web/static/js/device.js) puts a permanent UUID in `localStorage`, mirrors it to a 10-year cookie, and heartbeats to the server. That is the hard half.

For user identity offline: on login, issue a short-lived signed token (**PASETO v4.public**, or JWT with EdDSA) carrying `user_id`, `team_number`, and role claims, valid for the event duration. Cache it alongside the device UUID. The client reads claims locally to decide what to show; the server re-verifies on every write.

Trade-off to accept consciously: **an offline token cannot be revoked.** For a three-day event with a known roster that is fine. Keep expiry to 48–72 hours.

### Where CRDTs Actually Belong

Worth being precise about, rather than adopting a sync engine wholesale.

| Library | Fit for TealTeam |
| --- | --- |
| **Yjs** (via `yrs`, the Rust implementation) | **Yes — for the pick list.** `/api/pick-list` already exists, and a shared, reorderable ranking edited by several people during alliance selection is exactly where last-write-wins loses someone's work. `yrs` means it stays in Rust on both sides. |
| **Automerge** | Has a solid Rust core; a reasonable alternative to `yrs` if you want history and branching |
| **Loro** | Rust-native, 1.0 since 2024; smaller ecosystem, no particular reason to reach for it here |
| **ElectricSQL / PowerSync** | Both solve Postgres→client sync, but neither has a Rust/wasm client story that fits this design, and your conflict rules are simpler than what they solve |

**Recommendation:** hand-rolled delta sync (Section 3) for everything, plus `yrs` for the pick list only. Scouting records are append-only and do not conflict; a CRDT there adds metadata overhead to solve a problem you do not have.

### Storage Durability Notes

- Call `navigator.storage.persist()` on first login, or the browser may evict OPFS data under pressure — including a scout's queued submissions.
- Check `navigator.storage.estimate()` and warn before quota gets tight.
- iOS Safari evicts non-persisted site data after ~7 days of non-use. Irrelevant within an event; relevant between them.
- **Never treat the client as the system of record.** The outbox is a queue, not an archive.

### The Endgame — Server as Relay and Source of Truth

Section 4's migration path gets you to "handlers can run in the browser." This section describes where that lands: the client *is* the app, and the server is a pipe with a database behind it.

#### What the server keeps, and what it stops doing

Once the client renders and queries locally, the server has exactly five jobs left:

| Keeps | Why it cannot move |
| --- | --- |
| **Durable source of truth** | The client is a cache. Devices get dropped, wiped, and left in cars. |
| **Relay / fan-out** | Clients cannot reach each other; something has to sit in the middle. |
| **Credential holder** | FIRST and TBA keys, and the sync loops that use them. Never ship these to a device. |
| **Authentication** | Password verification and token issuance. Never verify a hash client-side. |
| **Arbitration** | Approval writes, conflict resolution, anything where "who wins" is a policy decision. |

And it stops doing: rendering HTML, running queries on behalf of the UI, aggregating stats, validating forms, computing point weights. All of that moves into `tt-core` and runs on the device.

The practical effect on the Pi is that it does almost nothing per request — append a row, filter it, broadcast it. Fifty clients stops being a question worth load-testing.

#### The mechanism: an append-only change log

Do not replicate by polling `updated_at` per table. That approach has two failure modes that produce exactly the "one scout's data mysteriously never showed up" bug:

1. **It cannot see deletions.** A deleted row has no `updated_at` to observe.
2. **It has a commit-ordering race.** Sequence numbers are assigned at `INSERT`, but rows become visible at `COMMIT`. A client that reads up to `seq = 100` can permanently miss a `seq = 99` written by a transaction that committed a moment later.

Replace it with a single append-only log that every mutation writes to:

```sql
CREATE TABLE changes (
  seq        bigserial PRIMARY KEY,
  entity     text        NOT NULL,   -- 'scouting_data' | 'matches' | 'assignments' | ...
  entity_pk  text        NOT NULL,
  op         text        NOT NULL,   -- 'upsert' | 'delete'
  payload    jsonb,                  -- null for deletes
  event_id   int,                    -- for scope filtering
  team_scope int,                    -- null = public; else visible to that team only
  actor_id   int,
  hlc        text        NOT NULL,   -- hybrid logical clock
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ON changes (seq) INCLUDE (entity, event_id, team_scope);
```

Clients then track **one cursor**, not one per table. Pull is `seq > cursor`. Deletes are first-class. Ordering is total. Replay and audit come free.

For the commit-ordering race, the simple correct fix at your write rate is a **lag window** — only serve changes older than a couple of seconds:

```sql
SELECT * FROM changes
WHERE seq > $1 AND created_at < now() - interval '2 seconds'
ORDER BY seq LIMIT 500;
```

Two seconds of latency is invisible to a human and eliminates the class of bug entirely. (The rigorous alternative is assigning `seq` under an advisory lock inside the transaction. Do that only if the lag ever becomes a problem, which it will not.)

#### Scoped subscriptions — do not leak other teams' notes

This is the part that is easy to get wrong and expensive to get wrong. A client must receive only the changes it is authorized to see. Your existing `submitting_team_id` privacy rule already establishes the principle; the change log has to enforce it.

A client subscribes with a scope — *"user 12, team 10101, event 42"* — and the server filters:

```rust
fn visible_to(change: &Change, scope: &Scope) -> bool {
    if let Some(eid) = change.event_id {
        if eid != scope.event_id { return false; }
    }
    match change.team_scope {
        None => true,                        // public: teams, matches, rankings
        Some(t) => t == scope.team_number,   // private: notes, assignments, chat
    }
}
```

Some tables never replicate at all — `users`, `sessions`, anything holding a password hash. Make that an explicit allowlist in code, not an omission.

#### The write path

Local-first, optimistic, with the server's echo as the durability signal:

```text
 1. Scout submits the form
 2. tt-core validates          ← the same code the server will run
 3. INSERT into local SQLite, status = 'pending', client_record_id = UUIDv7
 4. Append to outbox
 5. UI updates immediately     ← no network in the critical path
    ──────────── later, whenever a connection exists ────────────
 6. Sync client POSTs the outbox
 7. Server re-validates (same tt-core), appends to `changes`, assigns seq
 8. Change broadcasts to all scoped clients — including the author
 9. Author's client sees its own record return with a server seq
    → status = 'confirmed', outbox entry cleared
```

Step 9 is the whole trick. The client does not trust its own write until it comes back through the log. That single rule gives you idempotent retries (the `client_record_id` unique constraint makes a duplicate push a no-op), a visible pending state, and a clean definition of "saved" for the connection chip in the previous section.

Rejection is rare but must be handled: the server returns the rejected `client_record_id` with a reason, the client marks it `CONFLICT`, and it surfaces in the lead scout's review queue rather than vanishing.

#### Bootstrap: ship a SQLite file, not a million rows

A fresh device should not replay the change log from `seq = 0`. Have the server generate a **snapshot as an actual SQLite database file**, scoped to that client, and let the browser download it and open it directly in OPFS:

```text
GET /api/sync/snapshot?event=42
  → event.sqlite  (a few MB)  +  header: X-Sync-Cursor: 84213

client: write bytes to OPFS → open → resume incremental pull from seq 84213
```

No row-by-row inserts, no JSON parsing, no schema-building on the client. On a LAN this is a sub-second cold start. Regenerate the snapshot on a timer so it never lags far behind the log.

#### Rendering, and an SSR bonus that is genuinely free here

In the endgame the client renders everything and `tt-server` serves a static shell, the wasm bundle, and JSON/SSE. No Askama on the server at all.

But because Askama is compile-time and lives in a shared crate, the server *can* call the exact same render function for a cold first paint — not a reimplementation, literally the same code path. That preserves the "open the URL and it just works" property for a device that has never visited before, at zero duplication cost. A JS SPA cannot offer this cheaply; you can.

Recommended: **client renders by default; keep server-side render of the first page only.** Revisit if maintaining the two entry points ever costs more than it saves.

#### Version handshake — the deploy footgun

The failure this prevents: someone deploys mid-event, and clients running yesterday's wasm keep writing rows the new schema cannot accept, silently.

Every sync response carries a schema version. The client compares it to its own on every exchange:

- **Client older** → refuse to sync, show a blocking "Update required — tap to reload" banner, let the Service Worker fetch the new bundle. Do not silently discard the outbox; flush it first if the schema allows, otherwise export it.
- **Client newer** (a stale Pi) → warn the lead scout loudly.

Cheap to build, and it turns a data-corruption incident into an inconvenience.

#### Decided: the server runs SQLite too

Clients run SQLite, so the Pi does as well. The reasoning, recorded here because it shapes everything else:

| | Postgres on the Pi (rejected) | SQLite on the Pi (chosen) |
| --- | --- | --- |
| Dialect drift | **Two dialects forever.** The ongoing tax flagged earlier in this section | **Gone.** One schema, one set of queries, client and server |
| Snapshot generation | Export and rebuild | `VACUUM INTO` — a file copy |
| Backups | `pg_dump` | Copy a file |
| Pi deployment | A Postgres container to manage | One binary, one file |
| Concurrency | Genuinely high | WAL mode handles ~50 readers and one writer comfortably — which is exactly your shape |
| Off-site copy | Render held one | Gone — replaced by `rsync` to a laptop and client replicas (Section 3) |
| `sqlx` support | Yes | Yes — same library, feature flag |

At 20–50 clients with a low write rate and a single writer process, SQLite is not a compromise, and it deletes the largest ongoing maintenance cost in this design.

The decisive factor is dialect drift. With the cloud tier retired (Section 3), **Postgres disappears from the system entirely** — SQLite on the Pi, SQLite in the browser, one schema, one dialect, one set of queries. There is no translation boundary left to contain.

Two things this obliges you to do: run WAL mode (`PRAGMA journal_mode=WAL`) and serialize writes through one connection, and keep a load test in CI, because "SQLite is fine at this scale" is a claim worth continuously verifying rather than assuming.

### Work Items

| ID | Task | Effort |
| --- | --- | --- |
| O1 | Service Worker + app shell precache + navigation fallback (**fixes the reload bug**) | M |
| O2 | Web App Manifest, icons, installability, `navigator.storage.persist()` | S |
| O3 | Debounced form-state persistence and restore | S |
| O4 | Split into a cargo workspace; move pure code into `tt-core` + `tt-templates` | L |
| O5 | CI job building `tt-core` and `tt-templates` for `wasm32-unknown-unknown` — this is what keeps the boundary honest | S |
| O6 | Define the `Repo` trait with `trait-variant`; extract SQL from handlers into `tt-repo-pg` | XL |
| O7 | `tt-repo-sqlite` over SQLite-WASM/OPFS; port the schema to SQLite dialect | L |
| O8 | Service Worker fragment interception → wasm handler dispatch | M |
| O9 | Migrate read-only `/hx/*` routes to wasm, one at a time | L |
| O10 | Outbox + sync client in `tt-client` (pairs with S2/S4) | L |
| O11 | Four-state connection chip; remove all "offline mode" toggle language | S |
| O12 | Offline auth tokens (PASETO) layered onto the existing device identity | M |
| O13 | Conflict review screen for the lead scout | M |
| O14 | `yrs`-backed collaborative pick list | M |
| O15 | `changes` append-only log + `/api/sync/pull` with lag window | M |
| O16 | Scoped subscription filtering + never-replicate allowlist | M |
| O17 | SQLite snapshot bootstrap (`/api/sync/snapshot`, OPFS import) | M |
| O18 | Schema version handshake + blocking update banner | S |
| O19 | Migrate the Pi to SQLite (WAL, single writer); retire the Postgres container | L |


## 5. Communication

### Issue

Distance and environment make communication hard — scout to scout, and scout to strategist. That communication matters: scouts exchange relevant information that never makes it into a form, and strategists need to ask scouts to watch for specific things during a match.

### Proposed Solution

Two separate chat rooms — one for scout-to-strategist, one for scout-to-scout — in a collapsible side panel with tabs to switch between them. Every user can post in both; no role-based posting restrictions.

### Verdict

**Sound, and worth building in-app rather than adopting.** The open-posting decision is right: role-locked chat in a 15-person team creates more friction than it prevents. Two rooms is the right number — one is noisy, four is dead.

### Build vs. Adopt

The obvious objection is "the team already has Discord." That fails on C1 — Discord needs internet, and the whole premise is that there isn't any. Self-hosted options:

| Option | Pros | Cons | Verdict |
| --- | --- | --- | --- |
| **Build it into TealTeam** | ~400 lines using the SSE channel you're already building; reuses existing auth, roles, and team scoping; **messages can be attached to a match and a team**, which is the actual feature | You own it, including moderation | **Recommended** |
| **Matrix / Synapse on the Pi** | Federated, real clients, E2E encryption | Python, ~1 GB RAM, non-trivial ops; every client needs configuring for a local homeserver; encryption actively fights the moderation requirement below | Too heavy for a Pi that is also running the app |
| **Mattermost** | Solid, self-hostable, good mobile clients | Another ~1 GB container; separate accounts; still can't link a message to a match | Reasonable, but you gain a chat app and lose the integration |
| **Rocket.Chat** | Feature-rich | Heaviest of the three | No |

The deciding factor is context linking. Discord can give you a chat room. It cannot give you a message that renders as *"re: 1678, Q34"* and links straight into that robot's data. That link is the entire reason this belongs in the app.

### Recommended Architecture

**Transport.** Reuse the Section 3 machinery — **SSE down, `POST` up**. No new infrastructure, one connection for both sync and chat, and the browser handles reconnection with `Last-Event-ID`.

Adopt WebSockets only if you decide typing indicators and presence are worth the extra reconnect/heartbeat code. For 15 people in a gym, they are probably not.

**Message model:**

```sql
CREATE TABLE messages (
  id                bigserial PRIMARY KEY,
  client_message_id uuid NOT NULL,           -- UUIDv7, client-generated; idempotent resend
  room              text NOT NULL,           -- 'scouts' | 'strategy'
  event_id          int  NOT NULL REFERENCES events(id),
  author_id         int  NOT NULL REFERENCES users(id),
  team_number       int  NOT NULL,           -- posting team; rooms are scoped per team
  body              text NOT NULL,
  ref_team_id       int  REFERENCES teams(id),   -- optional context link
  ref_match_id      int  REFERENCES matches(id), -- optional context link
  hlc               text NOT NULL,           -- hybrid logical clock, for ordering
  created_at        timestamptz NOT NULL,    -- author's clock (may be wrong)
  received_at       timestamptz NOT NULL DEFAULT now(),  -- server clock (authoritative)
  UNIQUE (client_message_id)
);
CREATE INDEX ON messages (event_id, room, id);
```

**Ordering across offline clients.** A scout composes a message at 2:15 PM with no signal; it arrives at 2:40. Sorting by `created_at` teleports it into the middle of a conversation nobody remembers having. Sorting by `received_at` alone loses the fact that it was a reply to something.

Use a **hybrid logical clock** — a Lamport counter paired with wall time — for canonical ordering, and display both timestamps when they differ by more than a couple of minutes:

```text
  Maya · 2:15 PM  (delivered 2:40)
  1678 dropped a piece on the way out of auto, twice
```

This is also a good place to note that a hybrid logical clock on the server is undermined by a Pi with a wrong clock. See C2.

**Offline behavior.** Messages queue in the same local outbox as scouting submissions and flush through the same code path. Unsent messages render at reduced opacity with a small clock icon. `client_message_id` makes resends idempotent.

**Context linking.** Typing `#1678` or `#Q34` in a message autocompletes and populates `ref_team_id` / `ref_match_id`. Rendered messages then show a chip that navigates directly to that team or match. Strategist requests — *"watch #1678's intake next match"* — become actionable rather than something someone has to remember.

**UI.** The collapsible side panel from the original notes is right for laptops. On a 360 px phone it is not — a side panel there is a full-screen takeover in disguise.

```text
  Laptop / tablet (≥900px)          Phone (<900px)
  ┌──────────────┬──────────┐       ┌──────────────────┐
  │              │ [Scouts] │       │                  │
  │   Main app   │ Strategy │       │    Main app      │
  │              │──────────│       │                  │
  │              │ messages │       │                  │
  │              │          │       ├──────────────────┤
  │              │ [input ] │       │ 🏠  📋  💬²  📊  │ ← badge on nav
  └──────────────┴──────────┘       └──────────────────┘
       collapsible panel                bottom nav → full-screen chat
```

### Safety and Moderation — non-negotiable

The users are minors, and this is a communication tool their school is effectively hosting. Build these in from the start:

- **No direct messages.** Two rooms, both visible to everyone on the team. This removes an entire category of problem.
- **Full message log**, retained. Mentors and lead scouts get a read-only view of everything.
- **No hard delete.** Authors may retract, which tombstones the message and leaves *"message retracted by author"* visible.
- **Rooms are scoped to `(event_id, team_number)`.** This is not just a safety measure — it also prevents accidentally broadcasting your strategy to another team, which the existing `submitting_team_id` privacy rule already establishes as a design principle.
- **Rate limit** posts per user per minute. Someone will discover they can spam it. They always do.

This is also a strong argument *against* Matrix specifically: end-to-end encryption is the correct default for a chat system and directly conflicts with the mentor-oversight requirement here.

### Notifications — a feasibility caveat

Web Push requires a push service reachable over the internet (FCM, APNs, Mozilla's autopush). **With no internet, Web Push does not work.** This is not a limitation you can engineer around.

What does work offline, while the tab or installed PWA is open:

- The **Notification API** for in-page notifications
- `navigator.vibrate()` on Android (**not supported on iOS**)
- An audible alert via the Web Audio API — realistically inaudible in a competition venue
- A count badge on the bottom nav, which is honestly the one that works

For a strategist request that genuinely must land, the reliable channel is the one that already exists: the lead scout walks over. Design the app to make that unnecessary most of the time, not to eliminate it.

### Work Items

| ID | Task | Effort |
| --- | --- | --- |
| M1 | `messages` table, `POST /api/messages`, history endpoint with cursor paging | M |
| M2 | SSE message stream sharing the Section 3 event channel | S |
| M3 | Side panel (desktop) + full-screen view (mobile) with unread badges | M |
| M4 | Offline outbox integration and pending-message rendering | S |
| M5 | Hybrid logical clock ordering; dual-timestamp display for delayed messages | M |
| M6 | `#team` / `#match` autocomplete and context chips | M |
| M7 | Moderation: mentor log view, retract-not-delete, rate limiting | M |

---

## Cross-Cutting Concerns

Items that did not appear in the original notes but that this plan depends on.

### Lead Scout Assignment System — already built, needs finishing

This is the backbone of Section 2's UI simplification, and **most of it already exists.** [rust/tealteam-web/src/handlers/assignments.rs](rust/tealteam-web/src/handlers/assignments.rs) is ~651 lines covering per-match robot slot assignment, auto-distribute, clear-match, clear-all, and device rename. [static/js/device.js](rust/tealteam-web/static/js/device.js) gives every browser a permanent UUID in `localStorage`, mirrors it to a ten-year cookie, and heartbeats so the lead scout can see which physical devices are online within a three-minute window.

That is the hard half, and it is done. What remains:

| Gap | Why it matters |
| --- | --- |
| **Assignments do not push** — clients poll or reload | Push over the Section 3 SSE channel so a scout's device updates the moment they are assigned |
| **The scout side is not locked to the assignment** | This is the piece that eliminates wrong-robot entry. The submission form should open pre-filled and restricted, with a deliberate override |
| **No coverage view** | The lead scout needs at-a-glance: who is assigned, who has submitted, which robots are uncovered |
| **No rotation fairness** | Track matches scouted per person and suggest rotation, rather than making the lead scout remember |
| **Assignments are not available offline** | An assignment a scout cannot see when the network drops is worse than no assignment |

Closing those five gaps is the highest-leverage remaining work in this document. It directly implements the "babysitting" goal from the original notes and removes the team-list problem at its root.

### Time Synchronization

Restating C2 because a timestamp-based design lives or dies on it:

- The Pi needs a **DS3231 RTC module** (~$5). Without it, a power cycle at an offline venue gives you a server whose clock is wrong by hours or days, which silently corrupts every watermark and every hybrid logical clock in this design.
- Clients should compute an offset against the server clock on each sync and record it, so device-clock skew is measurable rather than mysterious.
- Store everything in UTC. The repo already has timezone handling documented in [docs/TIMEZONE_HANDLING.md](docs/TIMEZONE_HANDLING.md) — keep to it.

### Load Testing

Fifty concurrent clients on a Pi is fine in theory. Verify it before you find out in a gym:

- Script 30 simulated clients doing realistic submit-and-sync cycles.
- Measure p95 latency, Postgres connection pool saturation, and SSE connection stability over a two-hour run.
- Test the failure modes deliberately: pull the Ethernet cable mid-submission; kill the Pi's power; fill a client's storage quota.

### Backups

A single Pi holding an event's scouting data with no backup is one dropped crate away from losing a weekend of work.

- `pg_dump` to the NVMe drive every 10 minutes, keeping the last 24 hours.
- Copy to a USB stick between match blocks.
- Every client already holds a full SQLite replica in OPFS — a recovery path worth testing at least once, deliberately, before you need it.

---

## Phased Roadmap

Ordered by value delivered per unit of work, not by document order.

### Phase 0 — Foundations (pre-season)

| Task | Why first |
| --- | --- |
| N1 RTC module | Everything downstream depends on a correct clock |
| N2 NVMe storage | Prevents SD card death mid-event |
| N3 mDNS | Removes the single most common event-day support question |
| S3 `client_record_id` (UUIDv7) | Cheap now, painful to retrofit later |
| N8 Confirm E143 wording | Determines whether the network plan is legal |
| Retire the Go and .NET ports, and Render | Every day all three live, features get written three times |
| O5 `wasm32` CI job | Cheap now; the only thing that keeps the crate boundary honest later |
| O19 Migrate the Pi to SQLite | Decides the dialect before you write 187 queries against the wrong one |

### Phase 1 — Stop the bleeding (highest value)

| Task | Fixes |
| --- | --- |
| O1 Service Worker + navigation fallback | **The reload bug** |
| O3 Form-state persistence | Lost in-progress entries |
| O2 Manifest + persistent storage | Installability, eviction |
| O11 Four-state connection chip | "What does offline mode even do" |
| S5 Pre-event bulk load | Most of the tedious third-party data, before you ever leave the shop |
| Assignment gaps: lock scout to assignment, SSE push, coverage view | Wrong-robot entry, and the team-list problem at its root |

None of this requires the workspace refactor. All of it ships before kickoff.

### Phase 2 — Make it usable

| Task | Fixes |
| --- | --- |
| O4, O5 Cargo workspace split + `wasm32` CI gate | Makes everything after it possible |
| O6 `Repo` trait; extract SQL out of handlers | **The actual blocker.** Largest single item in the plan |
| U1-U3 Season schema system | The January rewrite treadmill |
| U4 Assignment-driven team selection | The 50-team list |
| S2-S4 Upstream log + signed SQLite bundles | Gets TBA data in without the Pi ever touching the internet |
| S6, S9, S10 Tethered uplink + SSE fan-out | Live updates |
| S11 Freshness badges | Stale rankings that look live cause bad picks |
| U7 Provenance badges | "When was this entered?" |
| U10 Mobile pass | Small screens |

### Phase 3 — Make it good

| Task | Fixes |
| --- | --- |
| O7 `tt-repo-sqlite` over SQLite-WASM/OPFS | Real SQL in the browser |
| O8, O9 Service Worker fragment interception; migrate read-only routes to wasm | **Handlers running browser-side** |
| O10 Outbox + sync client | Offline writes |
| O15, O16 Change log + scoped subscriptions | Replaces per-table watermarks; prevents cross-team leaks |
| O17, O18 Snapshot bootstrap + version handshake | Fast cold start; safe mid-event deploys |
| U5, U6 Graph view + notes panel | Unreadable metrics |
| U8, U9 Deduplication + event context | Duplicated data, limiting event selection |
| M1-M7 Chat | Communication |
| O12, O13 Offline auth + conflict review | Role gating offline |
| S5 QR transfer | Pit-to-stands with no cable |
| O14 `yrs` pick list | Collaborative alliance selection |

### Explicitly deferred or dropped

| Item | Why |
| --- | --- |
| Wi-Fi access point on the Pi | Violates FRC event rules (C1). Shop-practice only, default off. |
| Wi-Fi HaLow | No client device supports it. Revisit only as a pit-to-stands Pi-to-Pi bridge, and only if Ethernet and QR both fail. |
| Go and .NET ports | Being retired in favour of Rust. Until they are gone, every feature costs three times what it should. |
| A client-side SPA / Leptos rewrite | Unnecessary. Askama compiles to wasm and Unpoly fetches fragments over HTTP, so the existing UI can be served by wasm handlers without a framework or a client router. |
| IndexedDB as the browser store | Key-value only; you would hand-write every join. SQLite-WASM on OPFS lets your existing SQL mostly port. |
| ElectricSQL / PowerSync | Neither has a Rust/wasm client story that fits this design, and your conflict rules are simpler than what they solve. |
| Web Push notifications | Structurally impossible without internet. |

---

## Open Questions

Decisions that need a human before Phase 2 starts.

1. **Rules.** What is the exact 2026 E143 text, and what will your FTA actually tolerate? This determines the entire network topology.
2. **Devices.** What are scouts actually holding — team-owned tablets, or personal phones? Personal phones means iOS is in scope, which affects `BarcodeDetector`, `navigator.vibrate`, and storage eviction.
3. **Port retirement timeline.** When do Go and .NET actually get deleted, and who does it? Until then every feature in this plan costs three times what it should, and the shared-schema constraint blocks the season-schema redesign in Section 2C.
4. **Season schema.** Who owns writing the JSON schema each January, and what is the deadline relative to Kickoff?
5. **Chat moderation.** Which mentor owns the log review, and what is the retention policy?
6. **Multi-team.** Is TealTeam only for team 10101, or are you sharing it with alliance partners at events? This changes the scoping model, the auth model, and the chat design substantially.
7. **Off-site backup.** With Render retired, the Pi holds the only authoritative copy. Whose laptop receives the between-blocks `rsync`, and who checks that it actually ran?
8. **Scope of the wasm split.** Is the goal every route running browser-side, or just enough for a scout to enter data and search team info offline? The second is far cheaper and probably sufficient — Section 4's migration path lets you stop at any point.

---

## Sources

- [2026 FIRST Robotics Competition Game Manual](https://firstfrc.blob.core.windows.net/frc2026/Manual/HTML/2026GameManual.htm) — Section 14.3, Wireless Rules (E143)
- [FRC Event Rules Manual (2019)](https://firstfrc.blob.core.windows.net/frc2019/EventRules/EventRulesManual.pdf) — historical wording of the venue wireless prohibition
- [FMS Whitepaper](https://fms-manual.readthedocs.io/en/latest/fms-whitepaper/fms-whitepaper.html) — field network and spectrum rationale
- [Evaluating 802.11ah HaLow using the ESP32-S3 and FGH100M-H](https://www.beyondlogic.org/evaluating-802-11ah-halow-using-the-esp32-s3-fgh100m-h/) — HaLow hardware availability and real-world throughput
- [Quectel Wi-Fi HaLow module FCC/CE certifications](https://www.quectel.com/news-and-pr/wi-fi-halow-module-ce-fcc-certifications-morse-micro/) — HaLow module certification status
- [Alfa AHPI7292S Wi-Fi HaLow Pi HAT](https://store.rakwireless.com/products/choose-other-wireless-module-with-wisgate-connect) — Raspberry Pi HaLow hardware
- [Yjs vs Automerge vs Loro: CRDT Libraries 2026](https://www.pkgpulse.com/guides/yjs-vs-automerge-vs-loro-crdt-libraries-2026) — CRDT library comparison
- [Local-First Software in 2026](https://verity.salient.community/research/local-first-software-in-2026.html) — sync engine landscape, ElectricSQL and PowerSync positioning
- [Local-First Software: Principles, Patterns, and Technologies](https://wal.sh/research/local-first) — local-first architecture patterns
- [Wireless Communication Setup (Home and Competition)](https://www.chiefdelphi.com/t/wireless-communication-setup-home-competition/145907) — community discussion of AP restrictions at events
- [Leptos vs Yew vs Dioxus: Rust Frontend Framework Comparison 2026](https://reintech.io/blog/leptos-vs-yew-vs-dioxus-rust-frontend-framework-comparison-2026) — surveyed and not adopted; see "Explicitly deferred"
- [`trait-variant`](https://crates.io/crates/trait-variant) — generating `Send` and non-`Send` variants of one async trait
- [Go Wiki: WebAssembly](https://go.dev/wiki/WebAssembly) — background on wasm binary size, retained for comparison
- Internal: [rust/tealteam-web/README.md](rust/tealteam-web/README.md), [ARCHITECTURE.md](ARCHITECTURE.md), [docs/PI_EVENT_BOOT.md](docs/PI_EVENT_BOOT.md), [docs/TIMEZONE_HANDLING.md](docs/TIMEZONE_HANDLING.md)
