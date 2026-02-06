package handlers

import (
	"crypto/rand"
	"database/sql"
	"encoding/base64"
	"fmt"
	"log"
	"net/http"
	"strings"
	"time"

	"github.com/frc10101/TealTeam/internal/models"
	"golang.org/x/crypto/bcrypt"
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
func (h *Handler) HandleLogin(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	// Parse form data
	email := strings.TrimSpace(r.FormValue("email"))
	password := r.FormValue("password")

	// Validate input
	if email == "" || password == "" {
		h.sendAuthResponse(w, false, "Email and password are required", "")
		return
	}

	// Check if database is available
	if !h.hasDB() {
		h.sendAuthResponse(w, false, "Database unavailable", "")
		return
	}

	// Look up user by email
	var user models.User
	query := `SELECT id, email, name, password_hash, role FROM users WHERE email = $1`
	err := h.db.QueryRow(query, email).Scan(
		&user.ID,
		&user.Email,
		&user.Name,
		&user.PasswordHash,
		&user.Role,
	)

	if err == sql.ErrNoRows {
		// User not found - use generic error message to prevent user enumeration
		h.sendAuthResponse(w, false, "Invalid email or password", "")
		return
	} else if err != nil {
		log.Printf("Database error during login: %v", err)
		h.sendAuthResponse(w, false, "An error occurred. Please try again.", "")
		return
	}

	// Verify password
	if !CheckPasswordHash(password, user.PasswordHash) {
		h.sendAuthResponse(w, false, "Invalid email or password", "")
		return
	}

	// Create session
	sessionID, err := generateSessionID()
	if err != nil {
		log.Printf("Failed to generate session ID: %v", err)
		h.sendAuthResponse(w, false, "Failed to create session", "")
		return
	}

	// Store session in database
	expiresAt := time.Now().Add(sessionDuration)
	_, err = h.db.Exec(
		`INSERT INTO sessions (session_id, user_id, expires_at) VALUES ($1, $2, $3)`,
		sessionID, user.ID, expiresAt,
	)
	if err != nil {
		log.Printf("Failed to store session: %v", err)
		h.sendAuthResponse(w, false, "Failed to create session", "")
		return
	}

	// Update last login time
	_, err = h.db.Exec(
		`UPDATE users SET last_login = $1 WHERE id = $2`,
		time.Now(), user.ID,
	)
	if err != nil {
		log.Printf("Failed to update last login: %v", err)
	}

	// Set session cookie
	http.SetCookie(w, &http.Cookie{
		Name:     sessionCookieName,
		Value:    sessionID,
		Path:     "/",
		MaxAge:   int(sessionDuration.Seconds()),
		HttpOnly: true,  // Prevent JavaScript access
		Secure:   false, // Set to true in production with HTTPS
		SameSite: http.SameSiteLaxMode,
	})

	// Send success response with redirect
	h.sendAuthResponse(w, true, "Login successful", "/")
}

// HandleSignup processes user registration requests
func (h *Handler) HandleSignup(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	name := strings.TrimSpace(r.FormValue("name"))
	email := strings.TrimSpace(r.FormValue("email"))
	password := r.FormValue("password")
	confirmPassword := r.FormValue("confirm-password")
	teamNumber := strings.TrimSpace(r.FormValue("team-number"))

	if name == "" || email == "" || password == "" || confirmPassword == "" {
		h.sendAuthResponse(w, false, "All fields are required", "")
		return
	}

	if password != confirmPassword {
		h.sendAuthResponse(w, false, "Passwords do not match", "")
		return
	}

	if len(password) < 8 {
		h.sendAuthResponse(w, false, "Password must be at least 8 characters long", "")
		return
	}

	if !strings.Contains(email, "@") || !strings.Contains(email, ".") {
		h.sendAuthResponse(w, false, "Invalid email format", "")
		return
	}

	if !h.hasDB() {
		h.sendAuthResponse(w, false, "Database unavailable", "")
		return
	}

	var existingUserID int
	err := h.db.QueryRow(`SELECT id FROM users WHERE email = $1`, email).Scan(&existingUserID)
	if err == nil {
		// User already exists
		h.sendAuthResponse(w, false, "An account with this email already exists", "")
		return
	} else if err != sql.ErrNoRows {
		log.Printf("Database error checking existing user: %v", err)
		h.sendAuthResponse(w, false, "An error occurred. Please try again.", "")
		return
	}

	passwordHash, err := HashPassword(password)
	if err != nil {
		log.Printf("Failed to hash password: %v", err)
		h.sendAuthResponse(w, false, "Failed to create account. Please try again.", "")
		return
	}

	var userID int
	query := `INSERT INTO users (name, email, password_hash, role, created_at, updated_at) 
	          VALUES ($1, $2, $3, $4, $5, $6) 
	          RETURNING id`

	now := time.Now()
	err = h.db.QueryRow(
		query,
		name,
		email,
		passwordHash,
		"user", // Default role
		now,
		now,
	).Scan(&userID)

	if err != nil {
		log.Printf("Failed to create user: %v", err)
		if strings.Contains(err.Error(), "duplicate") || strings.Contains(err.Error(), "unique") {
			h.sendAuthResponse(w, false, "An account with this email already exists", "")
		} else {
			h.sendAuthResponse(w, false, "Failed to create account. Please try again.", "")
		}
		return
	}

	if teamNumber != "" {
		log.Printf("User %d (%s) signed up with team number: %s", userID, email, teamNumber)
	}

	sessionID, err := generateSessionID()
	if err != nil {
		log.Printf("Failed to generate session ID: %v", err)
		h.sendAuthResponse(w, true, "Account created! Redirecting to sign in...", "/sign-in")
		return
	}

	expiresAt := time.Now().Add(sessionDuration)
	_, err = h.db.Exec(
		`INSERT INTO sessions (session_id, user_id, expires_at) VALUES ($1, $2, $3)`,
		sessionID, userID, expiresAt,
	)
	if err != nil {
		log.Printf("Failed to store session: %v", err)
		h.sendAuthResponse(w, true, "Account created! Redirecting to sign in...", "/sign-in")
		return
	}

	http.SetCookie(w, &http.Cookie{
		Name:     sessionCookieName,
		Value:    sessionID,
		Path:     "/",
		MaxAge:   int(sessionDuration.Seconds()),
		HttpOnly: true,
		Secure:   false,
		SameSite: http.SameSiteLaxMode,
	})

	h.sendAuthResponse(w, true, "Account created successfully!", "/")
}

func (h *Handler) HandleLogout(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}

	cookie, err := r.Cookie(sessionCookieName)
	if err == nil && h.hasDB() {
		_, err = h.db.Exec(`DELETE FROM sessions WHERE session_id = $1`, cookie.Value)
		if err != nil {
			log.Printf("Failed to delete session: %v", err)
		}
	}

	http.SetCookie(w, &http.Cookie{
		Name:     sessionCookieName,
		Value:    "",
		Path:     "/",
		MaxAge:   -1,
		HttpOnly: true,
		Secure:   false,
		SameSite: http.SameSiteLaxMode,
	})

	w.Header().Set("HX-Redirect", "/sign-in")
	h.sendAuthResponse(w, true, "Logged out successfully", "/sign-in")
}

func (h *Handler) GetSessionUser(r *http.Request) (*models.User, error) {
	if !h.hasDB() {
		return nil, fmt.Errorf("database unavailable")
	}

	cookie, err := r.Cookie(sessionCookieName)
	if err != nil {
		return nil, fmt.Errorf("no session cookie")
	}

	var userID int
	var expiresAt time.Time
	err = h.db.QueryRow(
		`SELECT user_id, expires_at FROM sessions WHERE session_id = $1`,
		cookie.Value,
	).Scan(&userID, &expiresAt)

	if err == sql.ErrNoRows {
		return nil, fmt.Errorf("invalid session")
	} else if err != nil {
		return nil, fmt.Errorf("database error: %w", err)
	}

	if time.Now().After(expiresAt) {
		h.db.Exec(`DELETE FROM sessions WHERE session_id = $1`, cookie.Value)
		return nil, fmt.Errorf("session expired")
	}

	var user models.User
	err = h.db.QueryRow(
		`SELECT id, email, name, role, created_at, updated_at, last_login 
		 FROM users WHERE id = $1`,
		userID,
	).Scan(
		&user.ID,
		&user.Email,
		&user.Name,
		&user.Role,
		&user.CreatedAt,
		&user.UpdatedAt,
		&user.LastLogin,
	)

	if err != nil {
		return nil, fmt.Errorf("user not found: %w", err)
	}

	return &user, nil
}

func (h *Handler) sendAuthResponse(w http.ResponseWriter, success bool, message string, redirect string) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")

	if redirect != "" {
		w.Header().Set("HX-Redirect", redirect)
	}

	if !success {
		// Return error HTML
		fmt.Fprintf(w, `<div class="bg-red-900/20 border border-red-500 text-red-300 px-4 py-3 rounded mb-4" role="alert">
			<div class="flex items-center gap-2">
				<svg class="w-5 h-5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
					<path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"></path>
				</svg>
				<span class="block sm:inline font-medium">%s</span>
			</div>
		</div>`, message)
	} else {
		// Return success HTML
		fmt.Fprintf(w, `<div class="bg-green-900/20 border border-green-500 text-green-300 px-4 py-3 rounded mb-4" role="alert">
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

	result, err := h.db.Exec(`DELETE FROM sessions WHERE expires_at < $1`, time.Now())
	if err != nil {
		return fmt.Errorf("failed to cleanup sessions: %w", err)
	}

	rowsAffected, _ := result.RowsAffected()
	if rowsAffected > 0 {
		log.Printf("Cleaned up %d expired sessions", rowsAffected)
	}

	return nil
}
