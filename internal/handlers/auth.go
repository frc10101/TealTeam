package handlers

import (
	"context"
	"crypto/rand"
	"encoding/base64"
	"errors"
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/frc10101/TealTeam/internal/frc"
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
		h.log.Error("database error during login", "error", err)
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
		h.log.Error("failed to generate session ID", "error", err)
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
		h.log.Error("failed to store session", "error", err)
		h.sendAuthResponse(c, false, "Failed to create session", "")
		return
	}

	// Update last login time
	if err := h.db.Model(&models.User{}).Where("id = ?", user.ID).Update("last_login", time.Now()).Error; err != nil {
		h.log.Warn("failed to update last login", "user_id", user.ID, "error", err)
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

	// Sync team data if user has a team number
	if user.TeamNumber != nil && *user.TeamNumber > 0 {
		go func() {
			ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
			defer cancel()
			_, err := frc.SyncTeamForUser(ctx, h.db, *user.TeamNumber)
			if err != nil {
				h.log.Error("failed to sync team data on login", "user_id", user.ID, "team", *user.TeamNumber, "error", err)
			}
		}()
	}

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
	leadScout := strings.TrimSpace(c.PostForm("lead-scout")) != ""
	coach := strings.TrimSpace(c.PostForm("coach")) != ""

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
		h.log.Error("database error checking existing user", "error", err)
		h.sendAuthResponse(c, false, "An error occurred. Please try again.", "")
		return
	}

	passwordHash, err := HashPassword(password)
	if err != nil {
		h.log.Error("failed to hash password", "error", err)
		h.sendAuthResponse(c, false, "Failed to create account. Please try again.", "")
		return
	}

	var parsedTeamNumber *int
	if teamNumber != "" {
		if value, err := strconv.Atoi(teamNumber); err == nil {
			parsedTeamNumber = &value
		} else {
			h.log.Warn("invalid team number on signup", "email", email, "team_number", teamNumber)
		}
	}

	user := models.User{
		Name:         name,
		Email:        email,
		PasswordHash: passwordHash,
		TeamNumber:   parsedTeamNumber,
		Role:         "user",
		IsLeadScout:  leadScout,
		IsCoach:      coach,
		CreatedAt:    time.Now(),
		UpdatedAt:    time.Now(),
	}

	if err := h.db.Create(&user).Error; err != nil {
		h.log.Error("failed to create user", "email", email, "error", err)
		if strings.Contains(err.Error(), "duplicate") || strings.Contains(err.Error(), "unique") {
			h.sendAuthResponse(c, false, "An account with this email already exists", "")
		} else {
			h.sendAuthResponse(c, false, "Failed to create account. Please try again.", "")
		}
		return
	}

	if parsedTeamNumber != nil {
		h.log.Info("user signed up", "user_id", user.ID, "email", email, "team", *parsedTeamNumber)

		// Sync team data from FIRST API for new team member
		go func() {
			ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
			defer cancel()

			result, err := frc.SyncTeamForUser(ctx, h.db, *parsedTeamNumber)
			if err != nil {
				h.log.Error("failed to sync team on signup", "team", *parsedTeamNumber, "error", err)
				return
			}
			h.log.Info("synced team on signup", "team", *parsedTeamNumber,
				"events", result.Events, "teams", result.Teams, "event_teams", result.EventTeams)
		}()
	}

	sessionID, err := generateSessionID()
	if err != nil {
		h.log.Error("failed to generate session ID on signup", "error", err)
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
		h.log.Error("failed to store session on signup", "error", err)
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

	// Sync team data if user has a team number
	if parsedTeamNumber != nil && *parsedTeamNumber > 0 {
		go func() {
			ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
			defer cancel()
			_, err := frc.SyncTeamForUser(ctx, h.db, *parsedTeamNumber)
			if err != nil {
				h.log.Error("failed to sync team data on signup", "user_id", user.ID, "team", *parsedTeamNumber, "error", err)
			}
		}()
	}

	h.sendAuthResponse(c, true, "Account created successfully!", "/")
}

func (h *Handler) HandleLogout(c *gin.Context) {
	cookie, err := c.Request.Cookie(sessionCookieName)
	if err == nil && h.hasDB() {
		if err := h.db.Where("session_id = ?", cookie.Value).Delete(&models.Session{}).Error; err != nil {
			h.log.Warn("failed to delete session on logout", "error", err)
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

func (h *Handler) GetSession(c *gin.Context) (*models.Session, error) {
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

	return &session, nil
}

func (h *Handler) GetSessionUser(c *gin.Context) (*models.User, error) {
	session, err := h.GetSession(c)
	if err != nil {
		return nil, err
	}

	var user models.User
	err = h.db.Where("id = ?", session.UserID).First(&user).Error
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return nil, fmt.Errorf("user not found")
	} else if err != nil {
		return nil, fmt.Errorf("database error: %w", err)
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
		h.log.Info("cleaned up expired sessions", "count", rowsAffected)
	}

	return nil
}

// CleanupExpiredSubmissions removes pending submissions older than 20 minutes
func (h *Handler) CleanupExpiredSubmissions() error {
	if !h.hasDB() {
		return fmt.Errorf("database unavailable")
	}

	cutoff := time.Now().Add(-20 * time.Minute)
	result := h.db.Where("created_at < ? AND (status = ? OR status IS NULL)", cutoff, "pending").Delete(&scoutingSubmission{})
	if result.Error != nil {
		return fmt.Errorf("failed to cleanup expired submissions: %w", result.Error)
	}

	if result.RowsAffected > 0 {
		h.log.Info("cleaned up expired submissions", "count", result.RowsAffected)
	}

	return nil
}

// StartBackgroundCleanup starts periodic cleanup of expired sessions and submissions
func (h *Handler) StartBackgroundCleanup(stop <-chan struct{}) {
	ticker := time.NewTicker(5 * time.Minute)
	defer ticker.Stop()

	for {
		select {
		case <-ticker.C:
			if err := h.CleanupExpiredSessions(); err != nil {
				h.log.Error("session cleanup failed", "error", err)
			}
			if err := h.CleanupExpiredSubmissions(); err != nil {
				h.log.Error("submission cleanup failed", "error", err)
			}
		case <-stop:
			return
		}
	}
}

// HandleAccountPage displays the user's account information page
func (h *Handler) HandleAccountPage(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil {
		c.Redirect(http.StatusSeeOther, "/sign-in")
		return
	}

	h.render(c, "account", gin.H{
		"Title": "Account Settings",
		"User":  user,
	})
}

// HandleChangePassword processes password change requests
func (h *Handler) HandleChangePassword(c *gin.Context) {
	user, err := h.GetSessionUser(c)
	if err != nil {
		h.sendPasswordChangeResponse(c, false, "Please log in to change your password")
		return
	}

	currentPassword := c.PostForm("current-password")
	newPassword := c.PostForm("new-password")
	confirmPassword := c.PostForm("confirm-password")

	// Validate input
	if currentPassword == "" || newPassword == "" || confirmPassword == "" {
		h.sendPasswordChangeResponse(c, false, "All fields are required")
		return
	}

	// Validate password length
	if len(newPassword) < 8 {
		h.sendPasswordChangeResponse(c, false, "New password must be at least 8 characters long")
		return
	}

	// Check if passwords match
	if newPassword != confirmPassword {
		h.sendPasswordChangeResponse(c, false, "New passwords do not match")
		return
	}

	// Check if new password is the same as current password
	if currentPassword == newPassword {
		h.sendPasswordChangeResponse(c, false, "New password must be different from your current password")
		return
	}

	// Verify current password
	if !CheckPasswordHash(currentPassword, user.PasswordHash) {
		h.sendPasswordChangeResponse(c, false, "Current password is incorrect")
		return
	}

	// Hash new password
	newPasswordHash, err := HashPassword(newPassword)
	if err != nil {
		h.log.Error("failed to hash new password", "user_id", user.ID, "error", err)
		h.sendPasswordChangeResponse(c, false, "Failed to update password. Please try again.")
		return
	}

	// Update password in database
	if err := h.db.Model(&models.User{}).Where("id = ?", user.ID).Update("password_hash", newPasswordHash).Error; err != nil {
		h.log.Error("failed to update password", "user_id", user.ID, "error", err)
		h.sendPasswordChangeResponse(c, false, "Failed to update password. Please try again.")
		return
	}

	h.log.Info("password changed", "user_id", user.ID, "email", user.Email)
	h.sendPasswordChangeResponse(c, true, "Password changed successfully!")
}

// sendPasswordChangeResponse sends a formatted response for password change operations
func (h *Handler) sendPasswordChangeResponse(c *gin.Context, success bool, message string) {
	c.Header("Content-Type", "text/html; charset=utf-8")

	if !success {
		// Return error HTML
		fmt.Fprintf(c.Writer, `<div class="bg-red-900/20 border border-red-500 text-red-300 px-4 py-3 rounded" role="alert">
			<div class="flex items-center gap-2">
				<svg class="w-5 h-5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
					<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>
				</svg>
				<span class="block sm:inline font-medium">%s</span>
			</div>
		</div>`, message)
	} else {
		// Return success HTML and clear the form
		fmt.Fprintf(c.Writer, `<div class="bg-green-900/20 border border-green-500 text-green-300 px-4 py-3 rounded" role="alert">
			<div class="flex items-center gap-2">
				<svg class="w-5 h-5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
					<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"></path>
				</svg>
				<span class="block sm:inline font-medium">%s</span>
			</div>
		</div>
		<script>
			// Clear the form after successful password change
			document.getElementById('change-password-form').reset();
		</script>`, message)
	}
}
