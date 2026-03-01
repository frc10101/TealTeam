package handlers

import (
	"html/template"
	"log"
	"net/http"
	"path/filepath"

	"github.com/gin-gonic/gin"
	"gorm.io/gorm"
)

// Handler holds dependencies for HTTP handlers
type Handler struct {
	db        *gorm.DB
	templates map[string]*template.Template
	partials  *template.Template
}

// New creates a new Handler with the given database connection
func New(db *gorm.DB) *Handler {
	templates := make(map[string]*template.Template)

	// Template functions for pagination
	funcMap := template.FuncMap{
		"add": func(a, b int) int {
			return a + b
		},
		"subtract": func(a, b int) int {
			return a - b
		},
		"divide": func(a, b int) int {
			if b == 0 {
				return 0
			}
			return a / b
		},
	}

	// Parse layout
	layoutFile := filepath.Join("web", "templates", "layout.html")

	partialFiles, err := filepath.Glob(filepath.Join("web", "templates", "partials", "*.html"))
	if err != nil {
		log.Fatalf("Failed to list partial templates: %v", err)
	}

	// Parse each page template with the layout (and partials)
	pages := []string{"index", "submission", "submission_detail", "db_viewer", "admin_viewer", "signin", "signup", "team", "account"}
	for _, page := range pages {
		pageFile := filepath.Join("web", "templates", "pages", page+".html")
		files := append([]string{layoutFile, pageFile}, partialFiles...)
		tmpl, err := template.New("").Funcs(funcMap).ParseFiles(files...)
		if err != nil {
			log.Fatalf("Failed to parse template %s: %v", page, err)
		}
		templates[page] = tmpl
	}

	// Parse partial templates for HTMX responses
	partials, err := template.New("").Funcs(funcMap).ParseGlob(filepath.Join("web", "templates", "partials", "*.html"))
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
func (h *Handler) render(c *gin.Context, page string, data any) {
	tmpl, ok := h.templates[page]
	if !ok {
		log.Printf("Template not found: %s", page)
		http.Error(c.Writer, "Internal Server Error", http.StatusInternalServerError)
		return
	}

	c.Header("Content-Type", "text/html; charset=utf-8")
	if err := tmpl.ExecuteTemplate(c.Writer, "base", data); err != nil {
		log.Printf("Template error: %v", err)
		http.Error(c.Writer, "Internal Server Error", http.StatusInternalServerError)
	}
}

// renderPartial executes a partial template (for HTMX responses)
func (h *Handler) renderPartial(c *gin.Context, name string, data any) {
	c.Header("Content-Type", "text/html; charset=utf-8")
	if err := h.partials.ExecuteTemplate(c.Writer, name, data); err != nil {
		log.Printf("Template error: %v", err)
		http.Error(c.Writer, "Internal Server Error", http.StatusInternalServerError)
	}
}

// hasDB returns true if database is available
func (h *Handler) hasDB() bool {
	return h.db != nil
}
