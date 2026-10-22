# TealTeam documentation

The directory a document sits in tells you whether it describes the system being
built, a fact that outlives any implementation, or the code that was deleted on
2026-08-26. Within `docs/` itself, the tables below group documents by what you
would open them for.

## Plan of record — `docs/`

What the project is doing. Keep these current; if one is wrong, fix it.

| Document | What it is |
| --- | --- |
| **[ACTION_ITEMS.md](ACTION_ITEMS.md)** | **The work list.** Both source documents merged into one dependency-ordered set of items across four phases, with open decisions and dropped items. Start here, and use its IDs in commits. |
| [REBUILD_SPEC.md](REBUILD_SPEC.md) | The authoritative specification: domain, full data model, complete route surface, business rules, upstream API behavior, front-end conventions, configuration, and §12 — the 13 defects not to repeat. Written so the app can be rebuilt from an empty directory. |
| [REFURBISH_PLAN.md](REFURBISH_PLAN.md) | The forward plan: what should be *different* — offline-first client, season-driven schemas, sync design, chat, and the reasoning behind each decision. |
| [REFURBISH_NOTES.txt](REFURBISH_NOTES.txt) | The team's raw notes, verbatim, that the refurbish plan was worked up from. Source material, not a decision record. |

The spec is the *backward* record — what the app did and why. The refurbish plan
is the *forward* one. Where they disagree the refurbish plan wins: it was written
knowing this system's flaws.

### Feature designs

One document per larger item, linked from its row in ACTION_ITEMS.

| Document | What it is |
| --- | --- |
| [DATA_REVIEW.md](DATA_REVIEW.md) | DR1: comparing teams side by side for alliance selection. |
| [SCOUTING_ASSISTANT.md](SCOUTING_ASSISTANT.md) | AI1: asking a question and having it answered from our own data. |

### Running it

| Document | What it is |
| --- | --- |
| [SETUP_GUIDE.md](SETUP_GUIDE.md) | A fresh Pi and the team's laptops to event-ready, for someone who is not a developer. |
| [EVENT_DAY_RUNBOOK.md](EVENT_DAY_RUNBOOK.md) | One page for the day itself. Printed from `print/`. |
| [PI_NETWORK.md](PI_NETWORK.md) | The wired event LAN the Pi provides. |
| [PI_STORAGE.md](PI_STORAGE.md) | The database on the Pi's SSD, and its backups. |
| [print/](print/) | The event papers (runbook, checklists, Chromebook card, scouting sheet) as .docx to edit and .pdf to print; `print/build.py` rebuilds them from the docs above. |

### Developing and testing it

| Document | What it is |
| --- | --- |
| [DEV_ENVIRONMENT.md](DEV_ENVIRONMENT.md) | `cargo run -- dev`: a local server on fake data for every role, and the access-control checks run on it. |
| [LOAD_TEST.md](LOAD_TEST.md) | Q3: thirty devices saving and syncing against a real server for two hours. |

### FTC beta — this branch only

`ftc-biobuzz` is a throwaway FTC branch for the Athens Qualifier (2026-10-24). These
documents do not exist on `rebuild` except the plan and the migration playbook.

| Document | What it is |
| --- | --- |
| **[FTC_BETA_PLAN.md](FTC_BETA_PLAN.md)** | The plan: scope, the F-rows, the season schema, event-day steps. |
| [FTC_BIOBUZZ_MIGRATION.md](FTC_BIOBUZZ_MIGRATION.md) | The long-term design for running FRC and FTC from one codebase, and the beta-test playbook. |
| [FTC_BETA_SCHEMA_REVIEW.md](FTC_BETA_SCHEMA_REVIEW.md) | Review of the BIOBUZZ season schema, `crates/tt-core/seasons/2026.json`. |
| [FTC_BETA_API.md](FTC_BETA_API.md) | The FTC Events API for this one event, and how its payloads map. |
| [FTC_BETA_PAPER_KIT.md](FTC_BETA_PAPER_KIT.md) | The paper scouting sheet for when the app is down. |
| [FTC_BETA_REHEARSAL_SCHEDULE.csv](FTC_BETA_REHEARSAL_SCHEDULE.csv) | The fake schedule the rehearsal and tests load. Code reads this path. |
| [ftc/](ftc/) | Sample FTC Events API payloads. |

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

Give every new document a row in the matching table above. Before moving or
renaming one, `git grep` its file name: code comments, deploy scripts, server
error messages and `include_str!` name these paths.
