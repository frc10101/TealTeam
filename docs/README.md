# TealTeam documentation

Every document lives in a folder named for what you would open it for. This page
is the only file at the top: start here.

| Folder | What is in it |
| --- | --- |
| [`plan/`](plan/) | What the project is doing: the work list, the spec, the refurbish plan |
| [`design/`](design/) | One design document per larger feature |
| [`event/`](event/) | Setting up the Pi and laptops, and running the event day; [`event/print/`](event/print/) has the papers to print |
| [`dev/`](dev/) | Working on the code: the fake-data dev server, the load test |
| [`ftc/`](ftc/) | The FTC season and the one-event FTC beta |
| [`reference/`](reference/) | Facts about the outside world (FIRST, TBA, timezones) that outlive any implementation |
| [`legacy/`](legacy/) | The implementation retired on 2026-08-26; history, not a guide to the code |

## Find it fast

| I want to… | Open |
| --- | --- |
| See what is done and what is next | [plan/ACTION_ITEMS.md](plan/ACTION_ITEMS.md) |
| Know how a page, route or rule is meant to work | [plan/REBUILD_SPEC.md](plan/REBUILD_SPEC.md) |
| Set up a fresh Pi or the Chromebooks | [event/SETUP_GUIDE.md](event/SETUP_GUIDE.md) |
| Run the server on the day | [event/EVENT_DAY_RUNBOOK.md](event/EVENT_DAY_RUNBOOK.md) |
| Fix the network, the SSD or a backup | [event/PI_NETWORK.md](event/PI_NETWORK.md), [event/PI_STORAGE.md](event/PI_STORAGE.md) |
| Turn on HTTPS (offline mode on the Chromebooks) | [event/HTTPS_SETUP.md](event/HTTPS_SETUP.md) |
| Print the runbook, checklists or scouting sheet | [event/print/](event/print/) |
| Click around on fake data | [dev/DEV_ENVIRONMENT.md](dev/DEV_ENVIRONMENT.md) |
| Call the FIRST or TBA API | [reference/FRC_API_CALLS.md](reference/FRC_API_CALLS.md), then [reference/TBA_SCHEMA_FIX_SUMMARY.md](reference/TBA_SCHEMA_FIX_SUMMARY.md) |
| Work on the FTC beta | [ftc/FTC_BETA_PLAN.md](ftc/FTC_BETA_PLAN.md) |

## Plan of record — `plan/`

What the project is doing. Keep these current; if one is wrong, fix it.

| Document | What it is |
| --- | --- |
| **[ACTION_ITEMS.md](plan/ACTION_ITEMS.md)** | **The work list.** Both source documents merged into one dependency-ordered set of items across four phases, with open decisions and dropped items. Start here, and use its IDs in commits. |
| [REBUILD_SPEC.md](plan/REBUILD_SPEC.md) | The authoritative specification: domain, full data model, complete route surface, business rules, upstream API behavior, front-end conventions, configuration, and §12 — the 13 defects not to repeat. Written so the app can be rebuilt from an empty directory. |
| [REFURBISH_PLAN.md](plan/REFURBISH_PLAN.md) | The forward plan: what should be *different* — offline-first client, season-driven schemas, sync design, chat, and the reasoning behind each decision. |
| [REFURBISH_NOTES.txt](plan/REFURBISH_NOTES.txt) | The team's raw notes, verbatim, that the refurbish plan was worked up from. Source material, not a decision record. |

The spec is the *backward* record — what the app did and why. The refurbish plan
is the *forward* one. Where they disagree the refurbish plan wins: it was written
knowing this system's flaws.

## Feature designs — `design/`

One document per larger item, linked from its row in ACTION_ITEMS.

| Document | What it is |
| --- | --- |
| [DATA_REVIEW.md](design/DATA_REVIEW.md) | DR1: comparing teams side by side for alliance selection. |
| [SCOUTING_ASSISTANT.md](design/SCOUTING_ASSISTANT.md) | AI1: asking a question and having it answered from our own data. |

## Running an event — `event/`

| Document | What it is |
| --- | --- |
| [SETUP_GUIDE.md](event/SETUP_GUIDE.md) | A fresh Pi and the team's laptops to event-ready, for someone who is not a developer. |
| [EVENT_DAY_RUNBOOK.md](event/EVENT_DAY_RUNBOOK.md) | One page for the day itself. Printed from `event/print/`. |
| [PI_NETWORK.md](event/PI_NETWORK.md) | The wired event LAN the Pi provides. |
| [HTTPS_SETUP.md](event/HTTPS_SETUP.md) | Getting the app, then HTTPS with our own CA (Caddy `tls internal`) and trusting it on each device, so the offline features turn on. |
| [PI_STORAGE.md](event/PI_STORAGE.md) | The database on the Pi's SSD, and its backups. |
| [print/](event/print/) | The event papers (runbook, checklists, Chromebook card, scouting sheet) as .docx to edit and .pdf to print; `python3 docs/event/print/build.py` rebuilds them from the docs above. |

## Developing and testing — `dev/`

| Document | What it is |
| --- | --- |
| [DEV_ENVIRONMENT.md](dev/DEV_ENVIRONMENT.md) | `cargo run -- dev`: a local server on fake data for every role, and the access-control checks run on it. |
| [LOAD_TEST.md](dev/LOAD_TEST.md) | Q3: thirty devices saving and syncing against a real server for two hours. |

## FTC — `ftc/`

`ftc-biobuzz` is a throwaway FTC branch for the Athens Qualifier (2026-10-24). These
documents do not exist on `rebuild` except the plan and the migration playbook.

| Document | What it is |
| --- | --- |
| **[FTC_BETA_PLAN.md](ftc/FTC_BETA_PLAN.md)** | The plan: scope, the F-rows, the season schema, event-day steps. |
| [FTC_BIOBUZZ_MIGRATION.md](ftc/FTC_BIOBUZZ_MIGRATION.md) | The long-term design for running FRC and FTC from one codebase, and the beta-test playbook. |
| [FTC_BETA_SCHEMA_REVIEW.md](ftc/FTC_BETA_SCHEMA_REVIEW.md) | Review of the BIOBUZZ season schema, `crates/tt-core/seasons/2026.json`. |
| [FTC_BETA_API.md](ftc/FTC_BETA_API.md) | The FTC Events API for this one event, and how its payloads map. |
| [FTC_BETA_PAPER_KIT.md](ftc/FTC_BETA_PAPER_KIT.md) | The paper scouting sheet for when the app is down. |
| [FTC_BETA_REHEARSAL_SCHEDULE.csv](ftc/FTC_BETA_REHEARSAL_SCHEDULE.csv) | The fake schedule the rehearsal and tests load. Code reads this path. |
| [USALATQ_scores_qual.example.json](ftc/USALATQ_scores_qual.example.json) | A sample FTC Events API payload. |

## Reference — `reference/`

Facts about the world outside this repository. True regardless of what the code
looks like, and unlikely to need rewriting.

| Document | What it is |
| --- | --- |
| [FRC_API_CALLS.md](reference/FRC_API_CALLS.md) | FIRST and TBA endpoint catalog with payload examples |
| [TBA_SCHEMA_FIX_SUMMARY.md](reference/TBA_SCHEMA_FIX_SUMMARY.md) | TBA schema variance across seasons. **Read before writing a deserializer** |
| [TIMEZONE_HANDLING.md](reference/TIMEZONE_HANDLING.md) | Event timezone rules |

## Historical — `legacy/`

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

Put it in the folder that matches what a reader would open it for, and give it a
row in that folder's table here (and in *Find it fast* if people will look for it
often). A document about the TBA API is `reference/` if it describes the API and
`plan/` or `design/` if it describes what we do about it.

When something stops being true, it moves to `legacy/` with a banner saying so
— it does not get quietly deleted, and it does not stay where a reader will
mistake it for current.

Before moving or renaming a document, `git grep` its file name: code comments,
deploy scripts, server error messages, `include_str!` and the event profile name
these paths. Never edit an applied migration to fix one: it changes the
migration's checksum.
