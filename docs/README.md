# TealTeam documentation

Three tiers. The directory a document sits in tells you whether it describes the
system being built, a fact that outlives any implementation, or the code that was
deleted on 2026-08-26.

## Plan of record — `docs/`

What the project is doing. Keep these current; if one is wrong, fix it.

| Document | What it is |
| --- | --- |
| **[ACTION_ITEMS.md](ACTION_ITEMS.md)** | **The work list.** Both source documents merged into one dependency-ordered set of items across four phases, with open decisions and dropped items. Start here, and use its IDs in commits. |
| [REBUILD_SPEC.md](REBUILD_SPEC.md) | The authoritative specification: domain, full data model, complete route surface, business rules, upstream API behavior, front-end conventions, configuration, and §12 — the 13 defects not to repeat. Written so the app can be rebuilt from an empty directory. |
| [REFURBISH_PLAN.md](REFURBISH_PLAN.md) | The forward plan: what should be *different* — offline-first client, season-driven schemas, sync design, chat, and the reasoning behind each decision. |
| [DEV_ENVIRONMENT.md](DEV_ENVIRONMENT.md) | `cargo run -- dev` (DEV1): a throwaway server on fake data with every role, and the checks that hold the access and data-sharing rules to it. |
| [SCOUTING_ASSISTANT.md](SCOUTING_ASSISTANT.md) | Feasibility and design of the question box that answers from our data (AI1): where a model can run with no uplink, how answers stay grounded, and how to turn it on. |
| [REFURBISH_NOTES.txt](REFURBISH_NOTES.txt) | The team's raw notes, verbatim, that the refurbish plan was worked up from. Source material, not a decision record. |

The spec is the *backward* record — what the app did and why. The refurbish plan
is the *forward* one. Where they disagree the refurbish plan wins: it was written
knowing this system's flaws.

## Reference — `docs/reference/`

Facts about the world outside this repository. True regardless of what the code
looks like, and unlikely to need rewriting.

| Document | What it is |
| --- | --- |
| [FRC_API_CALLS.md](reference/FRC_API_CALLS.md) | FIRST and TBA endpoint catalog with payload examples |
| [TBA_SCHEMA_FIX_SUMMARY.md](reference/TBA_SCHEMA_FIX_SUMMARY.md) | TBA schema variance across seasons. **Read before writing a deserializer** |
| [TIMEZONE_HANDLING.md](reference/TIMEZONE_HANDLING.md) | Event timezone rules |

## Historical — `docs/legacy/`

Descriptions of the implementations retired on 2026-08-26. Kept for the domain
knowledge in them, not as a guide to the current code — **nothing here describes
what is in this repository now.** Every file carries a banner saying so and, where
one exists, a pointer to the live equivalent.

| Document | What it is |
| --- | --- |
| [DATA_POINTS.md](legacy/DATA_POINTS.md) | What was collected and what was derived |
| [TEAM_STATS_DISPLAY.md](legacy/TEAM_STATS_DISPLAY.md) | What the team page showed and why it mattered |
| [PREDICTIONS_REIMPLEMENTATION.md](legacy/PREDICTIONS_REIMPLEMENTATION.md) | The removed OPR/DPR prediction feature and its formulas |
| [MATCH_DETECTION.md](legacy/MATCH_DETECTION.md) | One surviving diagram of the match detection flow |
| [TEALTEAM_OVERVIEW_SOURCE.md](legacy/TEALTEAM_OVERVIEW_SOURCE.md) | Dense synthesis of the pre-rebuild documentation corpus |

## Adding a document

Put it in the tier that matches its lifetime, not its subject. A document about
the TBA API is `reference/` if it describes the API and `docs/` if it describes
what we do about it. When something in `docs/` stops being true, it moves to
`legacy/` with a banner — it does not get quietly deleted, and it does not stay
where a reader will mistake it for current.
