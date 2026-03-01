package db

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"gorm.io/gorm"
)

// ApplyMigrations applies SQL migrations from the given directory.
// It keeps track of applied migrations in the schema_migrations table.
func ApplyMigrations(database *gorm.DB, dir string) error {
	if database == nil {
		return nil
	}

	entries, err := os.ReadDir(dir)
	if err != nil {
		return fmt.Errorf("failed to read migrations dir: %w", err)
	}

	if err := ensureMigrationsTable(database); err != nil {
		return err
	}

	files := make([]string, 0, len(entries))
	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		name := entry.Name()
		if strings.HasSuffix(name, ".sql") {
			files = append(files, name)
		}
	}

	sort.Strings(files)

	for _, file := range files {
		fullPath := filepath.Join(dir, file)

		var count int64
		if err := database.Raw("SELECT COUNT(*) FROM schema_migrations WHERE filename = ?", file).Scan(&count).Error; err != nil {
			return fmt.Errorf("failed to check migration %s: %w", file, err)
		}

		if count > 0 {
			continue
		}

		content, err := os.ReadFile(fullPath)
		if err != nil {
			return fmt.Errorf("failed to read migration %s: %w", file, err)
		}
		if len(strings.TrimSpace(string(content))) == 0 {
			continue
		}

		tx := database.Begin()
		if tx.Error != nil {
			return fmt.Errorf("failed to start transaction for %s: %w", file, tx.Error)
		}

		if err := tx.Exec(string(content)).Error; err != nil {
			_ = tx.Rollback()
			return fmt.Errorf("failed to apply migration %s: %w", file, err)
		}

		if err := tx.Exec("INSERT INTO schema_migrations (filename) VALUES (?)", file).Error; err != nil {
			_ = tx.Rollback()
			return fmt.Errorf("failed to record migration %s: %w", file, err)
		}

		if err := tx.Commit().Error; err != nil {
			return fmt.Errorf("failed to commit migration %s: %w", file, err)
		}
	}

	return nil
}

// ResetMigrations clears migration history (test databases only).
func ResetMigrations(database *gorm.DB) error {
	if database == nil {
		return nil
	}

	return database.Exec("DROP TABLE IF EXISTS schema_migrations").Error
}

func ensureMigrationsTable(database *gorm.DB) error {
	return database.Exec(`
		CREATE TABLE IF NOT EXISTS schema_migrations (
			id SERIAL PRIMARY KEY,
			filename VARCHAR(255) UNIQUE NOT NULL,
			applied_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
		)
	`).Error
}
