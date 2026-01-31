package handlers

import (
	"database/sql"
	"html/template"
	"log"
	"net/http"
	"path/filepath"
)

// Handler holds dependencies for HTTP handlers
type Handler struct {
	db        *sql.DB
	templates map[string]*template.Template
	partials  *template.Template
}

// New creates a new Handler with the given database connection
func New(db *sql.DB) *Handler {
	templates := make(map[string]*template.Template)

	// Parse layout
	layoutFile := filepath.Join("web", "templates", "layout.html")

	// Parse each page template with the layout
	pages := []string{"index", "example"}
	for _, page := range pages {
		pageFile := filepath.Join("web", "templates", "pages", page+".html")
		tmpl, err := template.ParseFiles(layoutFile, pageFile)
		if err != nil {
			log.Fatalf("Failed to parse template %s: %v", page, err)
		}
		templates[page] = tmpl
	}

	// Parse partial templates for HTMX responses
	partials, err := template.ParseGlob(filepath.Join("web", "templates", "partials", "*.html"))
	if err != nil {
		log.Fatalf("Failed to parse partial templates: %v", err)
	}

	return &Handler{
		db:        db,
		templates: templates,
		partials:  partials,
	}
}

// render executes a page template with the layout
func (h *Handler) render(w http.ResponseWriter, page string, data any) {
	tmpl, ok := h.templates[page]
	if !ok {
		log.Printf("Template not found: %s", page)
		http.Error(w, "Internal Server Error", http.StatusInternalServerError)
		return
	}

	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	if err := tmpl.ExecuteTemplate(w, "base", data); err != nil {
		log.Printf("Template error: %v", err)
		http.Error(w, "Internal Server Error", http.StatusInternalServerError)
	}
}

// renderPartial executes a partial template (for HTMX responses)
func (h *Handler) renderPartial(w http.ResponseWriter, name string, data any) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	if err := h.partials.ExecuteTemplate(w, name, data); err != nil {
		log.Printf("Template error: %v", err)
		http.Error(w, "Internal Server Error", http.StatusInternalServerError)
	}
}

// hasDB returns true if database is available
func (h *Handler) hasDB() bool {
	return h.db != nil
}
