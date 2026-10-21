# Dev environment (DEV1)

`cargo run -- dev` starts a local server on a throwaway database full of fake
but realistic data, so you can click around as any role, and checks the access
and data-sharing rules on that data first.

```sh
cargo run -- dev              # seed if missing, check, serve on http://localhost:8080
cargo run -- dev --fresh      # wipe ./dev-data first
cargo run -- dev --check      # seed, run every check, print each PASS/FAIL, exit 0/1, don't serve
cargo run -- dev --port 9000  # start at 9000 (default 8080); the next free port if taken
cargo dev --check             # the alias in .cargo/config.toml: cargo run -p tt-web -- dev
```

A bare `cargo run` works because the root `Cargo.toml` sets
`default-members = ["crates/tt-web"]` (the workspace also has `tt-load`).
`./check.sh` and CI pass `--workspace` or `-p` on every build, test and clippy
line, so they still cover every crate.

## What keeps it away from real data

- Its data is `./dev-data/` (gitignored), always. It reads no `.env` and
  ignores `DATABASE_URL`.
- It refuses to start if `TEALTEAM_ENV=prod` is in the process environment, or
  if `dev-data` is, or is inside, `/srv/tealteam` (symlinks followed).
- `--fresh` deletes only a directory it made (it holds `.tealteam-dev`).
- No upstream: no FIRST/TBA fetching, no tether watch, no backups.
- It listens on 127.0.0.1 only, and every page has a **DEV DATA** bar (and an
  `x-dev-data` header). Cookies work over plain http.

## The data

Event `2026devevt` ("Dev Regional"), running today: 14 teams (ours is
10101, the other team is 4455), 21 qualification matches of six robots, the
first 10 played with scores. Every observation is written through the same
`Repo` calls the app uses, and the seed is deterministic (ids from insertion
order, record ids from a counter).

Every password is `dev`.

| Name | Email | Team | Role |
| --- | --- | --- | --- |
| Ada Admin | admin@dev.test | 10101 | admin |
| Lena Lead | lead@dev.test | 10101 | lead scout |
| Carl Coach | coach@dev.test | 10101 | drive coach |
| Sam Scout | scout1@dev.test | 10101 | scout; owns the registered tablet "Stands Left" |
| Sofia Scout | scout2@dev.test | 10101 | scout; has a refused entry waiting |
| Sid Scout | scout3@dev.test | 10101 | scout; has a declined observation |
| Sana Scout | scout4@dev.test | 10101 | scout; has an observation put back to waiting (G3) |
| Sol Signedout | signedout@dev.test | 10101 | scout signed out everywhere (token epoch bumped) |
| Lou Otherlead | lead2@dev.test | 4455 | lead scout of the other team |
| Oz Otherscout | scout5@dev.test | 4455 | scout of the other team; has a refused entry waiting |
| Ola Otherscout | scout6@dev.test | 4455 | scout of the other team |
| Nat Noteam | noteam@dev.test | none | scout with no team |

Also seeded: observations approved, waiting and declined by both teams, with
notes you can search for (`N10101-q3-t3102` is team 10101's note in match 3
about robot 3102, `N4455-...` the other team's); both teams approved notes on
the robots in matches 1 to 3; assignments for the unplayed matches (one to the
tablet); a pick list for each team; team stats; one refused entry for each team.

## What the checks cover

About 600, each printed as `PASS`/`FAIL  [who] what`. They drive the real
router (`startup::router`, through tower `oneshot`) as each user.

- **Route x role** (`routes` in `dev/checks.rs`): every route in the router,
  for signed-out, scout, lead, drive coach, admin, no-team user and the other
  team's lead. Pages answer 200, a guard turns the signed-out to `/sign-in` and
  the signed-in to `/`, the sync routes answer 401/403. Requests that would
  change data are sent only as users who are refused, or with an id that is not
  there; the tables are compared before and after to prove nothing changed. A
  test reads the router's source and fails if a route has no row.
- **Data sharing** (U13 and friends): each team reads only its own notes on
  `/notes` and a team page; a lead reviewing or reading a refused entry of the
  other team does not see its notes; a scout sees only their own declines; each
  lead is handed only their team's pick list.
- **Sync** (S2, S3, S10): a pull and a snapshot as each team carry only the
  replicated tables, only the viewer's team's notes and pick list, and nothing
  of sessions, hashes, emails, devices, the token key or refused entries; a pull
  scoped to another event carries nothing.
- **Admin viewer** (U17): no hash, session id or email in it; `sessions` and
  `token_key` are neither listed nor reachable by name.
- **Tokens** (C9, T1): a token with its key works, even from the downloadable
  page; without its key, with another device's key, from another device, signed
  by another key, or from before the user was signed out everywhere, it is
  refused on pull and push; a token is not a session; a scout's token is
  refused on lead-only routes.
- **Writes**: a scout cannot save a robot not in the match, a match not on the
  schedule, or the same robot twice; a push says why it refuses; only a lead
  approves, declines (with a reason) and undoes a review (G3), shown on a
  canary observation that ends as it began.
- **Accounts** (G1): the only admin stays one; a lead cannot change roles or
  passwords; an admin can make a scout a coach and take it back; an expired
  session is sent to sign in.
- **Sign in/out**: every user signs in with `dev`, a wrong password does not,
  and signing out ends the session.

Known and not asserted: the lead's queue lists every team's waiting
observations, and a decline reason is not redacted in a pull or snapshot
([ACTION_ITEMS.md](ACTION_ITEMS.md), R1 findings; open decision 4 makes both
moot while TealTeam is Team 10101 only).

## Adding a check

- **A new route**: add a row to `routes()` in `crates/tt-web/src/dev/checks.rs`
  with its `Access`; the `every_route_has_a_row_in_the_matrix` test fails until
  you do. Pass `probe_allowed = false` if even an empty request from an allowed
  user would change data.
- **A new rule**: add it to `sharing`, `sync_scope`, `tokens`, `writes`,
  `admin_viewer` or `accounts`, using `self.expect(who, what, pass, detail)`.
  If it needs data the seed lacks, add it to `dev/seed.rs` (a marker string in a
  note makes leaks easy to look for). A check must leave the data as it found
  it.
- A FAIL is a finding. If the app is wrong, fix it with a regression test where
  the rule lives; if the expectation is, fix the check and cite where the rule
  is stated.

`cargo test` runs all of it once on a temporary directory:
`dev::checks::tests::the_dev_environment_passes_its_own_checks`.
