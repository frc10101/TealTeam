# tealteam-web — Rust/axum port

Rust port of the TealTeam FRC scouting server, built for a local LAN server with
wired-ethernet clients (scouting tablets/laptops at a competition). It is a
faithful port of the Go and ASP.NET Core apps: same routes, same Unpoly-driven UI,
the **same PostgreSQL schema and SQL migrations**, and the same session cookie —
all three implementations can run side by side against one database.

## Stack decisions

- **axum + Askama, not a heavier framework.** The app is server-rendered HTML
  plus Unpoly fragments. axum handlers returning rendered Askama templates map
  1:1 onto the Gin/MVC handlers. Askama is compile-time-checked templating (the
  closest analog to Razor), so a malformed template is a build error, not a
  runtime 500.
- **sqlx with runtime queries (`query`/`query_as`), not the compile-time macros.**
  The schema is owned by the shared `migrations/` directory and created at
  boot, so there is no compile-time database to check against. Hand-written SQL
  keeps the queries identical to the Go/C# ports.
- **bcrypt** hashes are compatible with the Go and C# apps — a user created in
  any of the three can log into the others.
- **rustls** (no OpenSSL) for `reqwest` and `sqlx`, so the static binary has no
  system TLS dependency.
- **Unpoly is vendored locally** (`static/js/unpoly.min.js` + `.css`) so the UI
  works on an offline event LAN. `static/js/tt-unpoly.js` is a small glue layer
  that bridges the two places Unpoly differs from the previous HTMX fragments:
  server-driven navigation (an `X-Up-Events: tt:navigate` response header, the
  analog of HTMX's `HX-Redirect`) and the `[tt-src]` / `[tt-change]` self-load
  helpers for regions whose responses are bare inner fragments.

## Layout

```
src/main.rs            startup: env, migrations, boot sync, Kestrel-equivalent bind
src/db.rs              SQL migration runner (schema_migrations, shared with Go/C#)
src/models.rs          entity structs (sqlx::FromRow)
src/session.rs         cookie + DB sessions, bcrypt
src/connectivity.rs    internet/API health tracking for offline-aware behavior
src/first_api.rs       FIRST Events API client
src/tba.rs             The Blue Alliance API client
src/tba_stats_sync.rs  shared stats/matches upsert
src/first_sync.rs      FIRST sync (boot + on-login)
src/stats_syncer.rs    background TBA sync loop (tokio task)
src/scouting_points.rs configurable ranking point weights
src/handlers/*         one module per controller, same routes
templates/*            Askama templates (layout + pages + partials)
static/*               vendored Unpoly, tt-unpoly.js glue, site JS/CSS, device.js
```

Migrations are shared with the other ports: `../../migrations/*.sql` is applied
through the same `schema_migrations` table.

## Configuration

Same environment variables as the Go/C# apps (loaded from `.env` in this
directory or the repo root):

- `DATABASE_URL` — `postgres://user:pass@host:5432/db?sslmode=disable`
- `PORT` — default `8080`
- `TEALTEAM_ENV` — `test` (default; resets migration history on boot) or `prod`
- `FIRST_API_USERNAME`, `FIRST_API_KEY`, `FIRST_SEASON`, `FIRST_SYNC_ON_BOOT`,
  `FIRST_EVENT_CODE`, `FIRST_TEAM_NUMBER`, `FIRST_COUNTRY`
- `TBA_AUTH_KEY` — enables the background team stats/matches sync loop

## Run

```sh
# start postgres (from repo root)
docker compose up -d db

# run the app
cd rust/tealteam-web
cargo run                        # http://0.0.0.0:8080

# LAN clients connect to http://<server-ip>:8080
```

Rebuild the Tailwind CSS after editing templates (from the repo root):

```sh
npx tailwindcss -i ./web/tailwind/input.css \
  -o ./rust/tealteam-web/static/css/site.css --minify
```

## Build for the LAN server / Pi

```sh
cargo build --release                                   # host architecture
# Raspberry Pi (aarch64), with the target + linker installed:
cargo build --release --target aarch64-unknown-linux-gnu
```

The release binary is self-contained (rustls, no OpenSSL). Copy the binary plus
the `templates/` are compiled in, but `static/` and the repo `migrations/` must
be reachable at runtime — deploy the binary alongside `static/` and a copy of
`migrations/` (the binary looks next to itself first, then the repo layout).
