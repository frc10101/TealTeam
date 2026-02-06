package middleware

import (
	"net/http"

	"github.com/gin-gonic/gin"
)

// AuthChecker is an interface for checking authentication
type AuthChecker interface {
	GetSessionUser(c *gin.Context) (interface{}, error)
}

// RequireAuth middleware requires authentication to access a route
func RequireAuth(checker AuthChecker) gin.HandlerFunc {
	return func(c *gin.Context) {
		user, err := checker.GetSessionUser(c)
		if err != nil || user == nil {
			// Check if this is an HTMX request
			if c.GetHeader("HX-Request") == "true" {
				// For HTMX requests, send a redirect header
				c.Header("HX-Redirect", "/sign-in")
				c.Status(http.StatusUnauthorized)
				return
			}
			// For regular requests, redirect to sign-in page
			http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
			return
		}

		c.Next()
	}
}

// TODO: Add more middleware as needed
// Examples:
// - CORS middleware
// - Rate limiting middleware
// - Request ID middleware
