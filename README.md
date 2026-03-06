# TealTeam - FRC Scouting Application

An FRC (FIRST Robotics Competition) scouting application built with **Go**, **HTMX**, and **Tailwind CSS**. Designed for collecting match data, tracking team performance, and integrating with The Blue Alliance API.

## 🏗️ Stack

| Layer | Technology | Why |
|-------|------------|-----|
| **Backend** | Go (net/http) | Fast, simple, no framework overhead |
| **Templates** | Go html/template | Server-rendered HTML, built-in security |
| **Interactivity** | HTMX | Dynamic UIs without writing JavaScript |
| **Styling** | Tailwind CSS | Utility-first, compiled for production |
| **Client TS** | TypeScript | Minimal, only for UI polish |
| **Database** | PostgreSQL | Local Docker for dev, Render for production |
| **Deployment** | Render | Production hosting with managed PostgreSQL |

### Why This Stack?

- **Server-rendered by default**: Fast initial loads, SEO-friendly, works without JS
- **HTMX for interactivity**: Get SPA-like UX with HTML responses instead of JSON
- **No build complexity**: No webpack, no bundlers, no transpilers (except Tailwind CLI)
- **Boring technology**: Well-understood patterns, easy to debug, easy to hire for

## 📁 Directory Structure

```
├── cmd/
│   └── web/
│       └── main.go           # Application entry point (env flag handling)
├── internal/
│   ├── handlers/             # HTTP handlers (pages + HTMX fragments)
│   ├── db/                   # Database connection helpers
│   ├── models/               # Plain Go structs (no ORM)
│   └── middleware/           # Logging, recovery, etc.
├── migrations/
│   ├── 0001_init.sql         # Base schema
│   ├── 0002_*.sql            # Feature migrations
│   └── 0004_scouting_data.sql # Scouting data tables
├── scripts/
│   ├── dev.sh                # Development scripts
│   └── seed.go               # Database seeder with FRC test data
├── web/
│   ├── templates/
│   │   ├── layout.html       # Main layout wrapper
│   │   ├── pages/            # Full page templates
│   │   └── partials/         # HTMX fragment templates
│   ├── static/
│   │   ├── css/site.css      # Generated Tailwind CSS
│   │   └── js/site.ts        # Minimal client-side TypeScript
│   └── tailwind/
│       └── input.css         # Tailwind source with @apply components
├── docker-compose.yml        # Local dev services (PostgreSQL, Adminer)
├── package.json              # Tailwind CLI scripts
├── tailwind.config.js        # Tailwind configuration
├── Makefile                  # Common dev commands
├── DataPoints.md             # FRC scouting data documentation
└── README.md
```

---

## 🚀 Quick Start

### Prerequisites

- Go 1.22+
- Node.js 18+ (for Tailwind CLI)
- Docker (for local PostgreSQL)

### 1. Clone and Setup

```bash
git clone https://github.com/frc10101/TealTeam.git
cd TealTeam

# Install Tailwind dependencies
npm install
```

### 2. Start Local Database

```bash
# Start PostgreSQL in Docker
docker-compose up -d db

# Run all migrations
make migrate
# Or manually:
# psql postgres://user:password@localhost:5432/yourdb -f migrations/0001_init.sql
# psql postgres://user:password@localhost:5432/yourdb -f migrations/0002_...
```

### 3. Build CSS

```bash
# Build once
npm run css:build

# Or watch for changes during development
npm run css:watch
```

### 4. Run the Server

```bash
# Run with TEST database (local Docker) - DEFAULT
go run ./cmd/web

# Or explicitly specify test environment
go run ./cmd/web -env=test

# Run with PRODUCTION database (Render)
go run ./cmd/web -env=prod
```
### Or With Docker Installed 

```bash
#build the enviorment
docker-compose build

#run the containers
docker-compose up -d
```

### 5. Open in Browser

Visit [http://localhost:8080](http://localhost:8080)

---

## 🔧 Environment Selection

The application supports two database environments via the `-env` flag:

### Test Environment (Default)
```bash
go run ./cmd/web -env=test
```
- Uses local Docker PostgreSQL (`localhost:5432`)
- Connection: `postgres://user:password@localhost:5432/yourdb`
- For local development and testing
- Requires `docker-compose up -d db`

### Production Environment
```bash
go run ./cmd/web -env=prod
```
- Uses Render's managed PostgreSQL
- Reads from `RENDER_DATABASE_URL` or `DATABASE_URL` environment variable
- For connecting to production data locally or in deployment
- **Requires** environment variable to be set:
  ```bash
  export DATABASE_URL="postgres://user:pass@host:5432/dbname?sslmode=require"
  go run ./cmd/web -env=prod
  ```

### Environment Variable Priority
1. `RENDER_DATABASE_URL` (Render's auto-injected variable)
2. `DATABASE_URL` (fallback)

---

## 🔄 FIRST Events API Sync

On server startup, the app can populate `events`, `teams`, and `event_teams` from the FIRST Events API. If credentials are missing, sync is skipped.

### Required
- `FIRST_API_USERNAME`
- `FIRST_API_KEY`

### Optional
- `FIRST_SEASON` (default: 2026)
- `FIRST_SYNC_ON_BOOT` (default: true; set to `false` to skip)
- `FIRST_EVENT_CODE` (sync a single event)
- `FIRST_TEAM_NUMBER` (events for a specific team)
- `FIRST_COUNTRY` (default: `USA` when no other filters are set)

### On-Demand Refresh (Admin/Lead Scout)

Trigger a manual refresh (requires an authenticated admin/lead scout session):

```bash
curl -X POST http://localhost:8080/api/frc/sync
```

---

## 🏭 Build Process

### Development Build

```bash
# 1. Start database
docker-compose up -d db

# 2. Run migrations
make migrate

# 3. Seed test data (optional)
make seed

# 4. Build CSS (in separate terminal)
npm run css:watch

# 5. Run server with hot reload
make dev
# Or without hot reload:
make run
```

### Production Build

```bash
# 1. Build CSS for production
npm run css:build

# 2. Build Go binary
CGO_ENABLED=0 go build -o bin/server ./cmd/web

# 3. Run with production database
./bin/server -env=prod
```

### Docker Build

```bash
# Build and run everything in Docker
docker-compose up --build

# Or just build the image
docker build -t tealteam .
```

---

## 📁 Directory Structure

```
├── cmd/
│   └── web/
│       └── main.go           # Application entry point (env flag handling)
├── internal/
│   ├── handlers/             # HTTP handlers (pages + HTMX fragments)
│   ├── db/                   # Database connection helpers
│   ├── models/               # Plain Go structs (no ORM)
│   └── middleware/           # Logging, recovery, etc.
├── migrations/
│   ├── 0001_init.sql         # Base schema
│   ├── 0002_*.sql            # Feature migrations
│   └── 0004_scouting_data.sql # Scouting data tables
├── scripts/
│   ├── dev.sh                # Development scripts
│   └── seed.go               # Database seeder with FRC test data
├── web/
│   ├── templates/
│   │   ├── layout.html       # Main layout wrapper
│   │   ├── pages/            # Full page templates
│   │   └── partials/         # HTMX fragment templates
│   ├── static/
│   │   ├── css/site.css      # Generated Tailwind CSS
│   │   └── js/site.ts        # Minimal client-side TypeScript
│   └── tailwind/
│       └── input.css         # Tailwind source with @apply components
├── docker-compose.yml        # Local dev services (PostgreSQL, Adminer)
├── package.json              # Tailwind CLI scripts
├── tailwind.config.js        # Tailwind configuration
├── Makefile                  # Common dev commands
├── DataPoints.md             # FRC scouting data documentation
└── README.md
```

---

## 🧪 Development Commands

```bash
# Using Make
make dev        # Run with hot reload (test DB)
make run        # Run directly (test DB)
make build      # Build the application
make css        # Build CSS once
make css-watch  # Watch CSS changes
make db-up      # Start PostgreSQL
make db-down    # Stop PostgreSQL
make db-reset   # Reset database
make migrate    # Run migrations
make seed       # Seed database with FRC test data
make test       # Run tests

# Using npm
npm run css:build   # Build CSS
npm run css:watch   # Watch CSS

# Using Go directly
go run ./cmd/web              # Run with test DB (default)
go run ./cmd/web -env=test    # Run with test DB (explicit)
go run ./cmd/web -env=prod    # Run with production DB
go test ./...                 # Run tests
go build -o bin/server ./cmd/web  # Build binary
```

---

## 📖 How It Works

### Full Pages vs HTMX Fragments

This template distinguishes between two types of routes:

#### Full Pages (render with layout)
- Return complete HTML documents
- Include `<html>`, `<head>`, navigation, footer
- Used for initial page loads and direct navigation

```
GET /           → renders layout + index page
GET /example    → renders layout + example page
```

#### HTMX Fragments (HTML only)
- Return HTML fragments WITHOUT layout
- No `<html>`, `<head>`, or navigation
- Designed to be swapped into the DOM by HTMX
- **Always prefixed with `/hx`**

```
GET  /hx/example/table     → returns table HTML
POST /hx/example/item      → creates item, returns updated table
DELETE /hx/example/item/1  → deletes item, returns updated table
```

### Template Structure

```go
// Full page render (with layout)
h.render(w, "index", data)

// HTMX fragment render (no layout)
h.renderPartial(w, "example_table", data)
```

### HTMX Patterns

The template demonstrates these HTMX patterns:

```html
<!-- Load data on page load -->
<div hx-get="/hx/example/table" hx-trigger="load">
    Loading...
</div>

<!-- Submit form, update target -->
<form hx-post="/hx/example/item" hx-target="#items-table">
    <input name="name" required>
    <button type="submit">Add</button>
</form>

<!-- Delete with confirmation -->
<button hx-delete="/hx/example/item/1" 
        hx-target="#items-table"
        hx-confirm="Delete this item?">
    Delete
</button>
```

## 🔧 How to Extend

### Add a New Page

1. **Create the template** in `web/templates/pages/yourpage.html`:

```html
{{define "yourpage"}}
<div>
    <h1>{{.Title}}</h1>
    <!-- Your page content -->
</div>
{{end}}
```

2. **Add the handler** in `internal/handlers/pages.go`:

```go
func (h *Handler) HandleYourPage(w http.ResponseWriter, r *http.Request) {
    data := map[string]any{
        "Title": "Your Page",
    }
    h.render(w, "yourpage", data)
}
```

3. **Register the route** in `cmd/web/main.go`:

```go
mux.HandleFunc("GET /yourpage", h.HandleYourPage)
```

4. **Add navigation link** in `web/templates/layout.html`:

```html
<a href="/yourpage" class="nav-link">Your Page</a>
```

### Add an HTMX Fragment

1. **Create the partial** in `web/templates/partials/your_partial.html`:

```html
{{define "your_partial"}}
<!-- Fragment content - NO layout markup -->
<div class="your-content">
    {{range .Items}}
        <p>{{.Name}}</p>
    {{end}}
</div>
{{end}}
```

2. **Add the handler** in `internal/handlers/htmx.go`:

```go
func (h *Handler) HandleYourFragment(w http.ResponseWriter, r *http.Request) {
    // Fetch data from DB or elsewhere
    data := map[string]any{
        "Items": items,
    }
    h.renderPartial(w, "your_partial", data)
}
```

3. **Register the route** with `/hx` prefix:

```go
mux.HandleFunc("GET /hx/your/fragment", h.HandleYourFragment)
```

4. **Use in a page template**:

```html
<div hx-get="/hx/your/fragment" hx-trigger="load">
    Loading...
</div>
```

### Add a Database-Backed Feature

1. **Add migration** in `migrations/0002_your_feature.sql`:

```sql
CREATE TABLE your_table (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

2. **Add model** in `internal/models/models.go`:

```go
type YourModel struct {
    ID        int       `json:"id"`
    Name      string    `json:"name"`
    CreatedAt time.Time `json:"created_at"`
}
```

3. **Add DB queries** in `internal/db/` or directly in handlers:

```go
func (h *Handler) HandleYourFeature(w http.ResponseWriter, r *http.Request) {
    ctx, cancel := db.WithTimeout(r.Context())
    defer cancel()
    
    rows, err := h.db.QueryContext(ctx, 
        "SELECT id, name, created_at FROM your_table")
    // ... handle rows
}
```

### Add Tailwind Components

Edit `web/tailwind/input.css`:

```css
@layer components {
    .your-component {
        @apply px-4 py-2 bg-blue-500 text-white rounded;
    }
}
```

Then rebuild: `npm run css:build`

## 📝 Architectural Rules

1. **Server-rendered HTML is the default**
2. **HTMX endpoints return HTML fragments ONLY** (no JSON, no layout)
3. **All fragment routes start with `/hx`**
4. **Business logic lives on the server**
5. **Client-side JS is for UI polish only** (no business rules)
6. **Keep the stack boring and maintainable**

## 🚢 Deployment

### Build for Production

```bash
# Build CSS
npm run css:build

# Build Go binary
CGO_ENABLED=0 go build -o bin/server ./cmd/web

# Run locally against production database
./bin/server -env=prod
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `PORT` | Server port | `8080` |
| `DATABASE_URL` | PostgreSQL connection string | - |
| `RENDER_DATABASE_URL` | Render's auto-injected DB URL (takes priority) | - |

### Command Line Flags

| Flag | Description | Default |
|------|-------------|---------|
| `-env` | Environment: `test` (local Docker) or `prod` (Render) | `test` |

### Render Deployment

1. Connect your GitHub repository to Render
2. Set the build command: `npm run css:build && go build -o bin/server ./cmd/web`
3. Set the start command: `./bin/server -env=prod`
4. Add a PostgreSQL database in Render (auto-injects `RENDER_DATABASE_URL`)

### Docker (Optional)

Create a `Dockerfile`:

```dockerfile
FROM golang:1.22-alpine AS builder
WORKDIR /app
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -o server ./cmd/web

FROM alpine:latest
WORKDIR /app
COPY --from=builder /app/server .
COPY --from=builder /app/web ./web
EXPOSE 8080
CMD ["./server"]
```

## 📚 Resources

- [Go net/http documentation](https://pkg.go.dev/net/http)
- [Go html/template documentation](https://pkg.go.dev/html/template)
- [HTMX documentation](https://htmx.org/docs/)
- [Tailwind CSS documentation](https://tailwindcss.com/docs)
- [PostgreSQL documentation](https://www.postgresql.org/docs/)
- [The Blue Alliance API Documentation](https://www.thebluealliance.com/apidocs)

## ⚠️ The Blue Alliance Integration Notes

This application syncs team statistics and match data from The Blue Alliance (TBA) API v3. 

**Important**: TBA's response schema varies by FRC season. For example, 2026+ seasons use dynamic component naming for OPR calculations and nullable ranking fields. The application handles these variations through:

1. **Dynamic Component OPR Parsing** - Matches TBA's component map structure instead of expecting fixed field names
2. **Effective Ranking Helpers** - Falls back to `sort_orders` and `extra_stats` arrays when direct fields are null
3. **Match Persistence** - Automatically syncs match schedules and results to the database

For technical details and troubleshooting TBA integration issues, see [TBA_SCHEMA_FIX_SUMMARY.md](TBA_SCHEMA_FIX_SUMMARY.md).

## License

MIT License - Use this template for any project.
