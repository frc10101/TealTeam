# Go + HTMX + Tailwind CSS Web Application Template

A minimal, server-rendered web application template using **Go**, **HTMX**, and **Tailwind CSS**. This is a starter kit for building dashboards, admin panels, internal tools, CRUD applications, and data-driven websites.

## 🏗️ Stack

| Layer | Technology | Why |
|-------|------------|-----|
| **Backend** | Go (net/http) | Fast, simple, no framework overhead |
| **Templates** | Go html/template | Server-rendered HTML, built-in security |
| **Interactivity** | HTMX | Dynamic UIs without writing JavaScript |
| **Styling** | Tailwind CSS | Utility-first, compiled for production |
| **Client JS** | Vanilla JS | Minimal, only for UI polish |
| **Database** | PostgreSQL | Optional, but template-ready |

### Why This Stack?

- **Server-rendered by default**: Fast initial loads, SEO-friendly, works without JS
- **HTMX for interactivity**: Get SPA-like UX with HTML responses instead of JSON
- **No build complexity**: No webpack, no bundlers, no transpilers (except Tailwind CLI)
- **Boring technology**: Well-understood patterns, easy to debug, easy to hire for

## 📁 Directory Structure

```
├── cmd/
│   └── web/
│       └── main.go           # Application entry point
├── internal/
│   ├── handlers/             # HTTP handlers (pages + HTMX fragments)
│   ├── db/                   # Database connection helpers
│   ├── models/               # Plain Go structs (no ORM)
│   └── middleware/           # Logging, recovery, etc.
├── migrations/
│   └── 0001_init.sql         # Database schema
├── web/
│   ├── templates/
│   │   ├── layout.html       # Main layout wrapper
│   │   ├── pages/            # Full page templates
│   │   └── partials/         # HTMX fragment templates
│   ├── static/
│   │   ├── css/site.css      # Generated Tailwind CSS
│   │   └── js/site.js        # Minimal client-side JS
│   └── tailwind/
│       └── input.css         # Tailwind source with @apply components
├── docker-compose.yml        # PostgreSQL for local dev
├── package.json              # Tailwind CLI scripts
├── tailwind.config.js        # Tailwind configuration
├── Makefile                  # Common dev commands
└── README.md
```

## 🚀 Quick Start

### Prerequisites

- Go 1.22+
- Node.js 18+ (for Tailwind CLI)
- PostgreSQL (optional, via Docker)

### 1. Clone and Setup

```bash
# Clone the template
git clone https://github.com/yourusername/yourproject.git
cd yourproject

# Install Tailwind dependencies
npm install

# Copy environment file
cp .env.example .env
```

### 2. Build CSS

```bash
# Build once
npm run css:build

# Or watch for changes during development
npm run css:watch
```

### 3. Run the Server

```bash
# Run directly
go run ./cmd/web

# Or with hot reload (install air first)
go install github.com/cosmtrek/air@latest
air
```

### 4. Open in Browser

Visit [http://localhost:8080](http://localhost:8080)

### Optional: Start PostgreSQL

```bash
# Start database
docker-compose up -d

# Run migrations
psql postgres://user:password@localhost:5432/yourdb -f migrations/0001_init.sql

# Or use make
make db-up
make migrate
```

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

## 🧪 Development Commands

```bash
# Using Make
make dev        # Run with hot reload
make run        # Run directly
make css        # Build CSS once
make css-watch  # Watch CSS changes
make db-up      # Start PostgreSQL
make db-down    # Stop PostgreSQL
make db-reset   # Reset database
make migrate    # Run migrations
make test       # Run tests

# Using npm
npm run css:build   # Build CSS
npm run css:watch   # Watch CSS

# Using Go directly
go run ./cmd/web
go test ./...
go build -o bin/server ./cmd/web
```

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
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `PORT` | Server port | `8080` |
| `DATABASE_URL` | PostgreSQL connection string | - |

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

## License

MIT License - Use this template for any project.
