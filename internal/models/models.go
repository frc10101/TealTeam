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

// User is an example user model
// TODO: Customize based on your authentication needs
type User struct {
	ID        int       `json:"id"`
	Email     string    `json:"email"`
	Name      string    `json:"name"`
	CreatedAt time.Time `json:"created_at"`
	UpdatedAt time.Time `json:"updated_at"`
}

// TODO: Add more models as needed
// Keep models as plain structs - no ORM magic
// Database operations should be in the db package or handlers
