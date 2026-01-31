package db

import (
	"context"
	"database/sql"
	"fmt"
	"os"
	"time"

	_ "github.com/lib/pq" // PostgreSQL driver
)

// DefaultTimeout is the default context timeout for database operations
const DefaultTimeout = 5 * time.Second

// Connect establishes a connection to the PostgreSQL database
func Connect() (*sql.DB, error) {
	dsn := os.Getenv("DATABASE_URL")
	if dsn == "" {
		return nil, fmt.Errorf("DATABASE_URL environment variable is not set")
	}

	db, err := sql.Open("postgres", dsn)
	if err != nil {
		return nil, fmt.Errorf("failed to open database: %w", err)
	}

	// Configure connection pool
	db.SetMaxOpenConns(25)
	db.SetMaxIdleConns(5)
	db.SetConnMaxLifetime(5 * time.Minute)

	// Verify connection
	ctx, cancel := context.WithTimeout(context.Background(), DefaultTimeout)
	defer cancel()

	if err := db.PingContext(ctx); err != nil {
		return nil, fmt.Errorf("failed to ping database: %w", err)
	}

	return db, nil
}

// WithTimeout creates a context with the default timeout
func WithTimeout(ctx context.Context) (context.Context, context.CancelFunc) {
	return context.WithTimeout(ctx, DefaultTimeout)
}

// TODO: Add database query helpers here
// Example:
//
// func GetItems(ctx context.Context, db *sql.DB) ([]models.Item, error) {
//     ctx, cancel := WithTimeout(ctx)
//     defer cancel()
//
//     rows, err := db.QueryContext(ctx, "SELECT id, name, description, created_at FROM items ORDER BY created_at DESC")
//     if err != nil {
//         return nil, err
//     }
//     defer rows.Close()
//
//     var items []models.Item
//     for rows.Next() {
//         var item models.Item
//         if err := rows.Scan(&item.ID, &item.Name, &item.Description, &item.CreatedAt); err != nil {
//             return nil, err
//         }
//         items = append(items, item)
//     }
//     return items, rows.Err()
// }
