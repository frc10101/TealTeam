# TealTeam.Web — ASP.NET Core port

ASP.NET Core (.NET 10) port of the Go/Gin TealTeam FRC scouting server, built
for a local LAN server with wired-ethernet clients (scouting tablets/laptops
at a competition). It is a faithful port: same routes, same HTMX-driven UI,
same PostgreSQL schema and SQL migrations, same session cookie — the Go app
and this app can run side by side against the same database.

## Stack decisions

- **ASP.NET Core MVC + Razor views, not Blazor.** The original app is
  server-rendered HTML plus HTMX fragments. MVC controllers returning views
  and partial views map 1:1 onto the Gin handlers and Go templates. Blazor
  Server would replace the stateless HTTP model with a persistent SignalR
  circuit per client — worse on a Pi-class LAN server with many tablets and
  flaky connectivity, and it would have required rewriting the entire UI.
- **.NET (Core), not legacy .NET Framework.** .NET Framework 4.x is
  Windows-only and can't run on the Raspberry Pi / Docker deployment this
  project targets. Kestrel binds `0.0.0.0:$PORT` so LAN clients can connect.
- **Dapper + Npgsql, not EF Core.** The Go code is hand-written SQL via GORM;
  Dapper keeps the queries nearly identical and the footprint small.
- **HTMX is vendored locally** (`wwwroot/static/js/htmx.min.js`) instead of
  loaded from unpkg, so the UI works on an offline event LAN.

## Layout

```
Program.cs              cmd/web/main.go (startup, env, migrations, boot sync)
Data/                   internal/db (connection factory, SQL migration runner)
Models/                 internal/models + handler row types
Services/               internal/frc (FIRST client, TBA client, sync loops),
                        sessions/auth, scouting point weights
Controllers/            internal/handlers (same routes)
Views/Pages, Partials   web/templates/pages, partials (Razor)
wwwroot/static          web/static + built Tailwind CSS + vendored HTMX
```

SQL migrations are shared with the Go app: `../../migrations/*.sql` is copied
into the build output and applied through the same `schema_migrations` table.

## Configuration

Same environment variables as the Go app (loaded from `.env` in this directory
or the repo root):

- `DATABASE_URL` — `postgres://user:pass@host:5432/db?sslmode=disable`
- `PORT` — default `8080`
- `TEALTEAM_ENV` — `test` (default; resets migration history on boot like the
  Go app's `-env=test`) or `prod`
- `FIRST_API_USERNAME`, `FIRST_API_KEY`, `FIRST_SEASON`, `FIRST_SYNC_ON_BOOT`,
  `FIRST_EVENT_CODE`, `FIRST_TEAM_NUMBER`, `FIRST_COUNTRY`
- `TBA_AUTH_KEY` — enables the background team stats/matches sync loop

## Run

```sh
# start postgres (from repo root)
docker compose up -d db

# run the app
cd dotnet/TealTeam.Web
dotnet run                       # http://0.0.0.0:8080

# LAN clients connect to http://<server-ip>:8080
```

Rebuild the Tailwind CSS after editing views (from the repo root):

```sh
npx tailwindcss -i ./web/tailwind/input.css \
  -o ./dotnet/TealTeam.Web/wwwroot/static/css/site.css --minify
```

## Publish for the LAN server / Pi

```sh
dotnet publish -c Release -o out                      # host architecture
dotnet publish -c Release -r linux-arm64 -o out-pi \
  --self-contained                                     # Raspberry Pi (no .NET install needed)
```

The publish output contains `migrations/` and `wwwroot/`; copy the folder to
the server and run `./TealTeam.Web` with the environment variables above.
