package db

import (
	"context"
	"errors"
	"fmt"
	"os"
	"time"

	"gorm.io/driver/postgres"
	"gorm.io/gorm"
	"gorm.io/gorm/logger"
)

// DefaultTimeout is the default context timeout for database operations
const DefaultTimeout = 5 * time.Second

// customLogger wraps the default logger to skip ErrRecordNotFound logs
type customLogger struct {
	logger.Interface
}

func (cl *customLogger) Trace(ctx context.Context, begin time.Time, fc func() (string, int64), err error) {
	// Skip logging ErrRecordNotFound errors
	if errors.Is(err, gorm.ErrRecordNotFound) {
		return
	}
	cl.Interface.Trace(ctx, begin, fc, err)
}

// Connect establishes a connection to the PostgreSQL database using GORM
func Connect() (*gorm.DB, error) {
	dsn := os.Getenv("DATABASE_URL")
	if dsn == "" {
		return nil, fmt.Errorf("DATABASE_URL environment variable is not set")
	}

	gormDB, err := gorm.Open(postgres.Open(dsn), &gorm.Config{
		Logger: &customLogger{Interface: logger.Default},
	})
	if err != nil {
		return nil, fmt.Errorf("failed to open database: %w", err)
	}

	sqlDB, err := gormDB.DB()
	if err != nil {
		return nil, fmt.Errorf("failed to get sql DB: %w", err)
	}

	// Configure connection pool
	sqlDB.SetMaxOpenConns(25)
	sqlDB.SetMaxIdleConns(5)
	sqlDB.SetConnMaxLifetime(5 * time.Minute)

	// Verify connection
	ctx, cancel := context.WithTimeout(context.Background(), DefaultTimeout)
	defer cancel()

	if err := sqlDB.PingContext(ctx); err != nil {
		return nil, fmt.Errorf("failed to ping database: %w", err)
	}

	return gormDB, nil
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
