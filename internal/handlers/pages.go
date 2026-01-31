package handlers

import (
	"net/http"
)

// HandleIndex renders the home page
func (h *Handler) HandleIndex(w http.ResponseWriter, r *http.Request) {
	// Redirect non-root paths to 404
	if r.URL.Path != "/" {
		http.NotFound(w, r)
		return
	}

	data := map[string]any{
		"Title":   "Home",
		"Message": "Welcome to Your Application",
	}

	h.render(w, "index", data)
}

// HandleExamplePage renders the example page
func (h *Handler) HandleExamplePage(w http.ResponseWriter, r *http.Request) {
	data := map[string]any{
		"Title":       "Example Page",
		"Description": "This page demonstrates HTMX integration",
	}

	h.render(w, "example", data)
}

// TODO: Add more page handlers here
// func (h *Handler) HandleYourPage(w http.ResponseWriter, r *http.Request) {
//     data := map[string]any{
//         "Title": "Your Page Title",
//     }
//     h.render(w, "yourpage", data)
// }
