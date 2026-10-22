# The dev environment (F13)

A local server on a throwaway database full of fake data, and the access-control
checks run against that data. Use it to click around as any role, and to find out
whether a change broke who may see or do what, before it reaches a Pi.

```
cargo run -- dev            # or: cargo dev
```

| Command | What it does |
| --- | --- |
| `cargo run -- dev` | Seeds `./dev-data/` if it is missing, runs the checks, prints the user table, serves. |
| `cargo run -- dev --fresh` | Wipes the dev data first, so it starts from the seed again. |
| `cargo run -- dev --check` | Seeds, runs the checks, prints every PASS and FAIL and a total, and exits 0 or 1 without serving. |
| `cargo run -- dev --port N` | Serves on exactly port N. Without it: 8080, or the next free port. |

`cargo dev` is the same as `cargo run -- dev` (`.cargo/config.toml`). A bare
`cargo run` picks the server because the workspace's `default-members` is
`crates/tt-web`; `tt-load` is the other binary. `./check.sh` and CI always say
`--workspace` or `-p`, so nothing they build or test changed.

## What makes it safe

- Its data is `./dev-data/` (git-ignored), under the folder you run it in. It
  never reads `DATABASE_URL`, and it reads no `.env`.
- It refuses to start if `TEALTEAM_ENV=prod` is in the environment, or if its
  data folder is, or links into, `/srv/tealteam`.
- It makes no network request. FIRST and TBA are off on this build, the FTC
  Events key is not read (no "Pull from FTC Events" button), the backups and the
  tether watch are not started, and `ASK_BACKEND` is not read, so `/ask` says it
  is not set up.
- It listens on `127.0.0.1` only. Every account's password is `dev`.
- Every response carries the header `X-Dev-Data: DEV DATA - not production`, and
  the terminal says so at startup.

## The fake event

`DEVEVT` ("Dev Qualifier", America/Chicago, today): 14 teams, ours `10101`,
another scouting team `10202`, and 12 more (`10001`-`10012`); 21 qualification
matches of 4 robots, one with a surrogate (`*`). The schedule goes in through the
same `schedule::import` the lead's page runs. Q1-Q12 are played, with scores,
ranks, records, and OPR/DPR/CCWM worked out as an FTC pull leaves them (F7, F10),
so the rankings, graph, team and drive coach pages have data. Q13-Q21 are unplayed
and assigned (the last slot of Q21 to the tablet).

Accounts (all password `dev`; the terminal prints this table):

| Name | Email | Role | Team |
| --- | --- | --- | --- |
| Ada Admin | admin@dev.test | admin | 10101 |
| Lena Lead | lead@dev.test | lead scout | 10101 |
| Cole Coach | coach@dev.test | drive coach | 10101 |
| Sam, Priya, Quinn, Rae Scout | scout1..4@dev.test | scouts (Sam has the tablet) | 10101 |
| Sid Signedout | signedout@dev.test | scout, signed out everywhere (T1) | 10101 |
| Lou Otherlead | lead2@dev.test | lead scout | 10202 |
| Bea, Bo Otherscout | scoutb1, scoutb2@dev.test | scouts | 10202 |
| Nat Noteam | noteam@dev.test | scout | none |
| Nell Noteamlead | noteamlead@dev.test | lead scout | none |

Data: observations approved, waiting, declined (with a reason), and one approved
then undone (G3), by both teams; notes marked `DEVNOTE-10101`, `DEVNOTE-10202`,
`DEVNOTE-NOTEAM` so a leak is easy to grep; two refused entries waiting for the
lead (one per team); a pick list for each team; a tablet registered to Sam.

## What the checks cover

About 680 checks. Each prints `PASS` or `FAIL`, the user, the route, and the
rule. They drive the real router in-process, as each seeded user, on a **copy**
of the dev database (`VACUUM INTO`, deleted after), so they can write without
touching the data you serve.

- **Route x role.** Every route in `startup::router` (a check fails if one is not
  in the list) for signed-out, scout, coach, lead, admin, a user with no team,
  the other team's lead and scout: 200, a redirect to sign-in or home, 401, 403
  or 404 as the extractor says. Admin pages refuse non-admins; lead actions
  refuse scouts and coaches; the drive coach page is the coach's and the
  admin's; the pick list and `/ask` are the lead's, the coach's and the admin's.
- **Notes (U13).** Only the team that wrote a note reads it, on `/notes`, team
  pages, the review page of every observation, refused entries, the lead pages,
  the downloadable scouting page, the pull, the stream, the snapshot, and the
  `/ask` sheet. A user with no team reads none. A scout is told of their own
  declines only.
- **Sync (S3, S10).** The pull and the stream carry only observations,
  assignments and the caller's own pick list; the snapshot has empty
  `sessions`, `token_key`, `token_epochs`, `devices`, `refused_entries`,
  `pick_list_docs`, no emails or hashes, and only the caller's notes and
  pick list.
- **`/ask` (F11).** Off in dev. Leads, coaches and admins see "not set up";
  scouts are sent home; the sheet builder gives each user only their team's
  approved notes (never a waiting or declined one), and a user with no team none.
- **Admin viewer (U17).** `sessions` and `token_key` are not listed or readable;
  email and password hash are masked.
- **Tokens (T1).** A token needs its device key and its own tablet; another
  holder's key is refused; a token signs no one in to a page; a scout's token
  cannot push a bundle; Sid's token from before the sign-out is refused and one
  issued after works; a lead signing a scout out ends their session and token.
- **Writes.** A push refuses a robot not in the match, an unscheduled match,
  answers that do not fit the form, and keeps each refusal for the lead; it
  records under the caller. A scout, coach or teamless scout cannot approve,
  decline or undo; the lead can; assignments, sign-out and the pick list follow
  the roles; a pick list merge lands only in the caller's team's list; a bad
  schedule import changes nothing.
- **Upstream off.** No FTC client, no button, `POST /api/ftc/pull` says no key is
  set, `/api/frc/sync` and `/api/upstream/key` are 404, no assistant backend.

Two things the checks do **not** assert, because the code and the docs say they
are allowed: any lead can review, assign for, or sign out another team's scouts
(ACTION_ITEMS.md, "who reviews what"; moot while only team 10101 uses a server),
and a decline reason travels in the pull to other teams (the R1 `review_note`
finding, likewise).

## Adding a check

- A new route: add a line to `ROUTES` in `crates/tt-web/src/dev/checks.rs`
  with who may use it. The first check fails until every route in the router is
  listed.
- A new rule: add a `h.check(user, route, what, ok, detail)` call in the fitting
  section of `checks.rs`. Say where the rule is stated in the comment. A new fact
  to check against goes in `crates/tt-web/src/dev/seed.rs`.
- The integration test `crates/tt-web/tests/dev_check.rs` runs the whole report
  in `cargo test`, so `./check.sh` covers it.

A FAIL is either a real hole or a wrong expectation. Find out which. For a hole,
fix the app with a regression test; for a wrong expectation, fix the check and
cite where the rule is stated.
