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

	// Get current user if authenticated
	user, _ := h.GetSessionUser(r)

	data := map[string]any{
		"Title":   "Home",
		"Message": "Welcome to Your Application",
		"User":    user,
	}

	h.render(w, "index", data)
}

// HandleSubmissionPage renders the submission page
func (h *Handler) HandleSubmissionPage(w http.ResponseWriter, r *http.Request) {
	// Get current user if authenticated
	user, _ := h.GetSessionUser(r)

	data := map[string]any{
		"Title":       "Example Page",
		"Description": "This page demonstrates HTMX integration",
		"User":        user,
	}

	h.render(w, "example", data)
}

func (h *Handler) HandleSignIn(w http.ResponseWriter, r *http.Request) {
	// Redirect if already logged in
	user, _ := h.GetSessionUser(r)
	if user != nil {
		http.Redirect(w, r, "/", http.StatusSeeOther)
		return
	}

	data := map[string]any{
		"Title":       "Sign In",
		"Description": "Sign in to access higher level features.",
	}
	h.render(w, "signin", data)
}

// TODO: Add more page handlers here
// func (h *Handler) HandleYourPage(w http.ResponseWriter, r *http.Request) {
//     data := map[string]any{
//         "Title": "Your Page Title",
//     }
//     h.render(w, "yourpage", data)
// }
