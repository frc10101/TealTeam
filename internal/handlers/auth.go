package handlers

import (
	"crypto/rand"
	"encoding/base64"
	"errors"
	"fmt"
	"log"
	"net/http"
	"strings"
	"time"

	"github.com/frc10101/TealTeam/internal/models"
	"github.com/gin-gonic/gin"
	"golang.org/x/crypto/bcrypt"
	"gorm.io/gorm"
)

const (
	sessionCookieName = "session_id"
	sessionDuration   = 24 * time.Hour
	bcryptCost        = 12
)

// AuthResponse represents the response from authentication endpoints
type AuthResponse struct {
	Success  bool   `json:"success"`
	Message  string `json:"message"`
	Redirect string `json:"redirect,omitempty"`
}

// generateSessionID creates a cryptographically secure random session ID
func generateSessionID() (string, error) {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return base64.URLEncoding.EncodeToString(b), nil
}

// HashPassword hashes a password using bcrypt
func HashPassword(password string) (string, error) {
	bytes, err := bcrypt.GenerateFromPassword([]byte(password), bcryptCost)
	return string(bytes), err
}

// CheckPasswordHash compares a password with a hash
func CheckPasswordHash(password, hash string) bool {
	err := bcrypt.CompareHashAndPassword([]byte(hash), []byte(password))
	return err == nil
}

// HandleLogin processes login requests
func (h *Handler) HandleLogin(c *gin.Context) {
	// Parse form data
	email := strings.TrimSpace(c.PostForm("email"))
	password := c.PostForm("password")

	// Validate input
	if email == "" || password == "" {
		h.sendAuthResponse(c, false, "Email and password are required", "")
		return
	}

	// Check if database is available
	if !h.hasDB() {
		h.sendAuthResponse(c, false, "Database unavailable", "")
		return
	}

	// Look up user by email
	var user models.User
	err := h.db.Where("email = ?", email).First(&user).Error

	if errors.Is(err, gorm.ErrRecordNotFound) {
		// User not found - use generic error message to prevent user enumeration
		h.sendAuthResponse(c, false, "Invalid email or password", "")
		return
	} else if err != nil {
		log.Printf("Database error during login: %v", err)
		h.sendAuthResponse(c, false, "An error occurred. Please try again.", "")
		return
	}

	// Verify password
	if !CheckPasswordHash(password, user.PasswordHash) {
		h.sendAuthResponse(c, false, "Invalid email or password", "")
		return
	}

	// Create session
	sessionID, err := generateSessionID()
	if err != nil {
		log.Printf("Failed to generate session ID: %v", err)
		h.sendAuthResponse(c, false, "Failed to create session", "")
		return
	}

	// Store session in database
	expiresAt := time.Now().Add(sessionDuration)
	session := models.Session{
		SessionID: sessionID,
		UserID:    user.ID,
		ExpiresAt: expiresAt,
	}
	if err := h.db.Create(&session).Error; err != nil {
		log.Printf("Failed to store session: %v", err)
		h.sendAuthResponse(c, false, "Failed to create session", "")
		return
	}

	// Update last login time
	if err := h.db.Model(&models.User{}).Where("id = ?", user.ID).Update("last_login", time.Now()).Error; err != nil {
		log.Printf("Failed to update last login: %v", err)
	}

	// Set session cookie
	http.SetCookie(c.Writer, &http.Cookie{
		Name:     sessionCookieName,
		Value:    sessionID,
		Path:     "/",
		MaxAge:   int(sessionDuration.Seconds()),
		HttpOnly: true,  // Prevent JavaScript access
		Secure:   false, // Set to true in production with HTTPS
		SameSite: http.SameSiteLaxMode,
	})

	// Send success response with redirect
	h.sendAuthResponse(c, true, "Login successful", "/")
}

// HandleSignup processes user registration requests
func (h *Handler) HandleSignup(c *gin.Context) {
	name := strings.TrimSpace(c.PostForm("name"))
	email := strings.TrimSpace(c.PostForm("email"))
	password := c.PostForm("password")
	confirmPassword := c.PostForm("confirm-password")
	teamNumber := strings.TrimSpace(c.PostForm("team-number"))

	if name == "" || email == "" || password == "" || confirmPassword == "" {
		h.sendAuthResponse(c, false, "All fields are required", "")
		return
	}

	if password != confirmPassword {
		h.sendAuthResponse(c, false, "Passwords do not match", "")
		return
	}

	if len(password) < 8 {
		h.sendAuthResponse(c, false, "Password must be at least 8 characters long", "")
		return
	}

	if !strings.Contains(email, "@") || !strings.Contains(email, ".") {
		h.sendAuthResponse(c, false, "Invalid email format", "")
		return
	}

	if !h.hasDB() {
		h.sendAuthResponse(c, false, "Database unavailable", "")
		return
	}

	var existingUser models.User
	err := h.db.Select("id").Where("email = ?", email).First(&existingUser).Error
	if err == nil {
		// User already exists
		h.sendAuthResponse(c, false, "An account with this email already exists", "")
		return
	} else if !errors.Is(err, gorm.ErrRecordNotFound) {
		log.Printf("Database error checking existing user: %v", err)
		h.sendAuthResponse(c, false, "An error occurred. Please try again.", "")
		return
	}

	passwordHash, err := HashPassword(password)
	if err != nil {
		log.Printf("Failed to hash password: %v", err)
		h.sendAuthResponse(c, false, "Failed to create account. Please try again.", "")
		return
	}

	user := models.User{
		Name:         name,
		Email:        email,
		PasswordHash: passwordHash,
		Role:         "user",
		CreatedAt:    time.Now(),
		UpdatedAt:    time.Now(),
	}

	if err := h.db.Create(&user).Error; err != nil {
		log.Printf("Failed to create user: %v", err)
		if strings.Contains(err.Error(), "duplicate") || strings.Contains(err.Error(), "unique") {
			h.sendAuthResponse(c, false, "An account with this email already exists", "")
		} else {
			h.sendAuthResponse(c, false, "Failed to create account. Please try again.", "")
		}
		return
	}

	if teamNumber != "" {
		log.Printf("User %d (%s) signed up with team number: %s", user.ID, email, teamNumber)
	}

	sessionID, err := generateSessionID()
	if err != nil {
		log.Printf("Failed to generate session ID: %v", err)
		h.sendAuthResponse(c, true, "Account created! Redirecting to sign in...", "/sign-in")
		return
	}

	expiresAt := time.Now().Add(sessionDuration)
	session := models.Session{
		SessionID: sessionID,
		UserID:    user.ID,
		ExpiresAt: expiresAt,
	}
	if err := h.db.Create(&session).Error; err != nil {
		log.Printf("Failed to store session: %v", err)
		h.sendAuthResponse(c, true, "Account created! Redirecting to sign in...", "/sign-in")
		return
	}

	http.SetCookie(c.Writer, &http.Cookie{
		Name:     sessionCookieName,
		Value:    sessionID,
		Path:     "/",
		MaxAge:   int(sessionDuration.Seconds()),
		HttpOnly: true,
		Secure:   false,
		SameSite: http.SameSiteLaxMode,
	})

	h.sendAuthResponse(c, true, "Account created successfully!", "/")
}

func (h *Handler) HandleLogout(c *gin.Context) {
	cookie, err := c.Request.Cookie(sessionCookieName)
	if err == nil && h.hasDB() {
		if err := h.db.Where("session_id = ?", cookie.Value).Delete(&models.Session{}).Error; err != nil {
			log.Printf("Failed to delete session: %v", err)
		}
	}

	http.SetCookie(c.Writer, &http.Cookie{
		Name:     sessionCookieName,
		Value:    "",
		Path:     "/",
		MaxAge:   -1,
		HttpOnly: true,
		Secure:   false,
		SameSite: http.SameSiteLaxMode,
	})

	c.Header("HX-Redirect", "/sign-in")
	h.sendAuthResponse(c, true, "Logged out successfully", "/sign-in")
}

func (h *Handler) GetSessionUser(c *gin.Context) (*models.User, error) {
	if !h.hasDB() {
		return nil, fmt.Errorf("database unavailable")
	}

	cookie, err := c.Request.Cookie(sessionCookieName)
	if err != nil {
		return nil, fmt.Errorf("no session cookie")
	}

	var session models.Session
	err = h.db.Where("session_id = ?", cookie.Value).First(&session).Error
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, fmt.Errorf("invalid session")
	} else if err != nil {
		return nil, fmt.Errorf("database error: %w", err)
	}

	if time.Now().After(session.ExpiresAt) {
		_ = h.db.Where("session_id = ?", cookie.Value).Delete(&models.Session{}).Error
		return nil, fmt.Errorf("session expired")
	}

	var user models.User
	if err := h.db.Where("id = ?", session.UserID).First(&user).Error; err != nil {
		return nil, fmt.Errorf("user not found: %w", err)
	}

	return &user, nil
}

func (h *Handler) sendAuthResponse(c *gin.Context, success bool, message string, redirect string) {
	c.Header("Content-Type", "text/html; charset=utf-8")

	if redirect != "" {
		c.Header("HX-Redirect", redirect)
	}

	if !success {
		// Return error HTML
		fmt.Fprintf(c.Writer, `<div class="bg-red-900/20 border border-red-500 text-red-300 px-4 py-3 rounded mb-4" role="alert">
			<div class="flex items-center gap-2">
				<svg class="w-5 h-5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
					<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>
				</svg>
				<span class="block sm:inline font-medium">%s</span>
			</div>
		</div>`, message)
	} else {
		// Return success HTML
		fmt.Fprintf(c.Writer, `<div class="bg-green-900/20 border border-green-500 text-green-300 px-4 py-3 rounded mb-4" role="alert">
			<div class="flex items-center gap-2">
				<svg class="w-5 h-5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
					<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"></path>
				</svg>
				<span class="block sm:inline font-medium">%s</span>
			</div>
		</div>`, message)
	}
}

func (h *Handler) CleanupExpiredSessions() error {
	if !h.hasDB() {
		return fmt.Errorf("database unavailable")
	}

	result := h.db.Where("expires_at < ?", time.Now()).Delete(&models.Session{})
	if result.Error != nil {
		return fmt.Errorf("failed to cleanup sessions: %w", result.Error)
	}

	rowsAffected := result.RowsAffected
	if rowsAffected > 0 {
		log.Printf("Cleaned up %d expired sessions", rowsAffected)
	}

	return nil
}
