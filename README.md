# TealTeam

An FRC scouting and analytics application for team 10101 — manual scouting observations merged with official FIRST and Blue Alliance competition data, served from a Raspberry Pi at the event on a LAN with no reliable internet.

## Status: rebuilding

**There is no implementation in this repository right now.** Three ports previously existed against one shared PostgreSQL schema — Go/Gin, ASP.NET Core, and Rust/axum — plus a Render deployment. All of them have been retired.

They were retired deliberately. Every feature cost three times what it should, and all three were shaped around a server-centric architecture that the next version deliberately moves away from. Rather than refactor one port into a shape it was not built for, the working knowledge was written down and the code was deleted.

Everything needed to rebuild is in `docs/`.

## Start here

| Document | What it is |
| --- | --- |
| **[docs/ACTION_ITEMS.md](docs/ACTION_ITEMS.md)** | **The work list.** Both source documents merged into one dependency-ordered set of tasks, in four phases, with open decisions and dropped items. Start here. |
| [docs/REBUILD_SPEC.md](docs/REBUILD_SPEC.md) | The authoritative rebuild specification. Domain, full data model, complete route surface, business rules, upstream API behavior, front-end conventions, configuration, and a catalog of the defects not to repeat. Written so the app can be rebuilt from an empty directory. |
| [RefurbishInstructions.md](RefurbishInstructions.md) | The forward plan: what should be *different* — offline-first client, season-driven schemas, sync design, chat, and the reasoning behind each decision. |

Work from the action items. Read the rebuild spec for what the app *did*, and the refurbish plan for why the target design is what it is. Where the two disagree, the refurbish plan wins — it was written knowing this system's flaws.

### Supporting reference

- [docs/FRC_API_Calls.md](docs/FRC_API_Calls.md) — FIRST and TBA endpoint catalog with payload examples
- [docs/TBA_SCHEMA_FIX_SUMMARY.md](docs/TBA_SCHEMA_FIX_SUMMARY.md) — TBA schema variance across seasons; read before writing deserializers
- [docs/TIMEZONE_HANDLING.md](docs/TIMEZONE_HANDLING.md) — event timezone rules
- [docs/DataPoints.md](docs/DataPoints.md) — what is collected and what is derived
- [docs/TEAM_STATS_DISPLAY.md](docs/TEAM_STATS_DISPLAY.md) — what the team page shows and why it matters
- [docs/PREDICTIONS_REIMPLEMENTATION.md](docs/PREDICTIONS_REIMPLEMENTATION.md) — the removed OPR/DPR prediction feature and its formulas
- [docs/TEALTEAM_DETAILED_OVERVIEW_SOURCE.md](docs/TEALTEAM_DETAILED_OVERVIEW_SOURCE.md) — dense domain synthesis (its stack and deployment sections describe the retired implementation)

## Target stack

Rust + axum + Askama + sqlx + Unpoly + Tailwind, over SQLite, on a Raspberry Pi 5 at the event. One server, and it lives at the event — no cloud tier.

The one structural rule to get right on the first commit: split the workspace into a pure `tt-core` / `tt-templates` layer that compiles to `wasm32`, behind a `Repo` trait, with a CI job enforcing it. See [REBUILD_SPEC.md §9](docs/REBUILD_SPEC.md#9-crate-layout-for-the-rebuild). Retrofitting that split was the single largest item in the refurbish plan; building it in costs almost nothing.

## Recovering the retired code

Nothing is lost — the last commit containing all three implementations is the one immediately before the retirement commit on `main`.

```sh
git log --oneline -- rust/tealteam-web       # find the retirement commit
git show <retirement-commit>~1 --stat        # see the full retired tree
git checkout <retirement-commit>~1 -- rust/tealteam-web   # restore a subtree
```

Useful paths in that tree: `rust/tealteam-web/src/` (the most complete port), `migrations/*.sql` (the full schema), `web/tailwind/input.css` (the design system), `rust/tealteam-web/static/js/` (`device.js`, `tt-unpoly.js`).
