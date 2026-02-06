package handlers

import (
	"net/http"

	"github.com/gin-gonic/gin"
)

// HandleIndex renders the home page
func (h *Handler) HandleIndex(c *gin.Context) {
	// Redirect non-root paths to 404
	if c.Request.URL.Path != "/" {
		http.NotFound(c.Writer, c.Request)
		return
	}

	// Get current user if authenticated
	user, _ := h.GetSessionUser(c)

	data := map[string]any{
		"Title":   "Home",
		"Message": "Welcome to Your Application",
		"User":    user,
	}

	h.render(c, "index", data)
}

// HandleSubmissionPage renders the submission page
func (h *Handler) HandleSubmissionPage(c *gin.Context) {
	// Get current user if authenticated
	user, _ := h.GetSessionUser(c)

	data := map[string]any{
		"Title":       "Scouting Submission",
		"Description": "Submit scouting data for competitions",
		"User":        user,
	}

	h.render(c, "submission", data)
}

func (h *Handler) HandleSignIn(c *gin.Context) {
	// Redirect if already logged in
	user, _ := h.GetSessionUser(c)
	if user != nil {
		http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
		return
	}

	data := map[string]any{
		"Title":       "Sign In",
		"Description": "Sign in to access higher level features.",
	}
	h.render(c, "signin", data)
}

func (h *Handler) HandleSignUp(c *gin.Context) {
	// Redirect if already logged in
	user, _ := h.GetSessionUser(c)
	if user != nil {
		http.Redirect(c.Writer, c.Request, "/", http.StatusSeeOther)
		return
	}

	data := map[string]any{
		"Title":       "Sign Up",
		"Description": "Create an account to get started.",
	}
	h.render(c, "signup", data)
}

// TODO: Add more page handlers here
// func (h *Handler) HandleYourPage(w http.ResponseWriter, r *http.Request) {
//     data := map[string]any{
//         "Title": "Your Page Title",
//     }
//     h.render(w, "yourpage", data)
// }
