package models

import "time"

// Item is a generic example model
// TODO: Replace or extend with your domain-specific models
type Item struct {
	ID          int       `json:"id"`
	Name        string    `json:"name"`
	Description string    `json:"description"`
	CreatedAt   time.Time `json:"created_at"`
	UpdatedAt   time.Time `json:"updated_at"`
}

// User represents an authenticated user
type User struct {
	ID           int        `json:"id"`
	Email        string     `json:"email"`
	Name         string     `json:"name"`
	PasswordHash string     `json:"-"` // Never send password hash in JSON
	Role         string     `json:"role"`
	LastLogin    *time.Time `json:"last_login,omitempty"`
	CreatedAt    time.Time  `json:"created_at"`
	UpdatedAt    time.Time  `json:"updated_at"`
}

// Session represents a user session
type Session struct {
	SessionID string
	UserID    int
	ExpiresAt time.Time
	CreatedAt time.Time
}

// TODO: Add more models as needed
// Keep models as plain structs - no ORM magic
// Database operations should be in the db package or handlers
