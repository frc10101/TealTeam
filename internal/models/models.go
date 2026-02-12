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
	ID           int        `json:"id" gorm:"column:id;primaryKey"`
	Email        string     `json:"email" gorm:"column:email"`
	Name         string     `json:"name" gorm:"column:name"`
	PasswordHash string     `json:"-" gorm:"column:password_hash"` // Never send password hash in JSON
	TeamNumber   *int       `json:"team_number,omitempty" gorm:"column:team_number"`
	Role         string     `json:"role" gorm:"column:role"`
	IsAdmin      bool       `json:"is_admin" gorm:"column:is_admin"`
	LastLogin    *time.Time `json:"last_login,omitempty" gorm:"column:last_login"`
	CreatedAt    time.Time  `json:"created_at" gorm:"column:created_at"`
	UpdatedAt    time.Time  `json:"updated_at" gorm:"column:updated_at"`
}

// Session represents a user session
type Session struct {
	SessionID string    `gorm:"column:session_id;primaryKey"`
	UserID    int       `gorm:"column:user_id"`
	SelectedEventID *int `gorm:"column:selected_event_id"`
	ExpiresAt time.Time `gorm:"column:expires_at"`
	CreatedAt time.Time `gorm:"column:created_at"`
}

// TODO: Add more models as needed
// Keep models as plain structs - no ORM magic
// Database operations should be in the db package or handlers
