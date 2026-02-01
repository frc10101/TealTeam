package handlers

import (
	"context"
	"database/sql"
	"fmt"
	"net/http"
	"strconv"
)

// TableInfo holds metadata about a database table
type TableInfo struct {
	Name     string
	RowCount int64
}

// ColumnInfo holds metadata about a table column
type ColumnInfo struct {
	Name     string
	Type     string
	Nullable bool
	Default  sql.NullString
}

// HandleDBViewer renders the database viewer page
// Route: GET /development/db
func (h *Handler) HandleDBViewer(w http.ResponseWriter, r *http.Request) {
	data := map[string]any{
		"Title":       "Database Viewer",
		"DBConnected": h.hasDB(),
	}

	if h.hasDB() {
		tables, err := h.getTableList(r.Context())
		if err != nil {
			data["Error"] = fmt.Sprintf("Failed to get tables: %v", err)
		} else {
			data["Tables"] = tables
		}

		// Check if a table is selected via query param
		selectedTable := r.URL.Query().Get("table")
		if selectedTable != "" {
			data["SelectedTable"] = selectedTable
		}
	}

	h.render(w, "db_viewer", data)
}

// HandleDBTableContent returns the table content as an HTMX fragment
// Route: GET /hx/development/db/table/{name}
func (h *Handler) HandleDBTableContent(w http.ResponseWriter, r *http.Request) {
	if !h.hasDB() {
		http.Error(w, "Database not connected", http.StatusServiceUnavailable)
		return
	}

	tableName := r.PathValue("name")
	if tableName == "" {
		http.Error(w, "Table name is required", http.StatusBadRequest)
		return
	}

	// Parse pagination params
	offset, _ := strconv.Atoi(r.URL.Query().Get("offset"))
	limit, _ := strconv.Atoi(r.URL.Query().Get("limit"))
	if limit <= 0 {
		limit = 50
	}
	if offset < 0 {
		offset = 0
	}

	data, err := h.getTableData(r.Context(), tableName, offset, limit)
	if err != nil {
		http.Error(w, fmt.Sprintf("Failed to get table data: %v", err), http.StatusInternalServerError)
		return
	}

	h.renderPartial(w, "db_table_content", data)
}

// getTableList retrieves all user tables from the database
func (h *Handler) getTableList(ctx context.Context) ([]TableInfo, error) {
	query := `
		SELECT 
			table_name,
			(SELECT COUNT(*) FROM information_schema.columns c WHERE c.table_name = t.table_name AND c.table_schema = 'public') as col_count
		FROM information_schema.tables t
		WHERE table_schema = 'public' 
		AND table_type = 'BASE TABLE'
		ORDER BY table_name
	`

	rows, err := h.db.QueryContext(ctx, query)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var tables []TableInfo
	for rows.Next() {
		var t TableInfo
		var colCount int64
		if err := rows.Scan(&t.Name, &colCount); err != nil {
			return nil, err
		}

		// Get row count for each table
		var count int64
		countQuery := fmt.Sprintf("SELECT COUNT(*) FROM %q", t.Name)
		if err := h.db.QueryRowContext(ctx, countQuery).Scan(&count); err != nil {
			count = 0
		}
		t.RowCount = count

		tables = append(tables, t)
	}

	return tables, rows.Err()
}

// getTableData retrieves data and schema for a specific table
func (h *Handler) getTableData(ctx context.Context, tableName string, offset, limit int) (map[string]any, error) {
	data := map[string]any{
		"SelectedTable": tableName,
		"Offset":        offset,
		"Limit":         limit,
	}

	// Get column info
	columns, err := h.getColumnInfo(ctx, tableName)
	if err != nil {
		return nil, fmt.Errorf("failed to get columns: %w", err)
	}
	data["Columns"] = columns

	// Get total row count
	var totalRows int
	countQuery := fmt.Sprintf("SELECT COUNT(*) FROM %q", tableName)
	if err := h.db.QueryRowContext(ctx, countQuery).Scan(&totalRows); err != nil {
		return nil, fmt.Errorf("failed to count rows: %w", err)
	}
	data["TotalRows"] = totalRows

	// Get rows
	dataQuery := fmt.Sprintf("SELECT * FROM %q ORDER BY 1 LIMIT %d OFFSET %d", tableName, limit, offset)
	rows, err := h.db.QueryContext(ctx, dataQuery)
	if err != nil {
		return nil, fmt.Errorf("failed to query rows: %w", err)
	}
	defer rows.Close()

	// Get column names from result
	colNames, err := rows.Columns()
	if err != nil {
		return nil, err
	}

	var rowData [][]any
	for rows.Next() {
		// Create a slice of interface{} to hold the values
		values := make([]any, len(colNames))
		valuePtrs := make([]any, len(colNames))
		for i := range values {
			valuePtrs[i] = &values[i]
		}

		if err := rows.Scan(valuePtrs...); err != nil {
			return nil, err
		}

		// Convert values to strings for display
		rowValues := make([]any, len(values))
		for i, v := range values {
			rowValues[i] = formatValue(v)
		}
		rowData = append(rowData, rowValues)
	}
	data["Rows"] = rowData

	return data, rows.Err()
}

// getColumnInfo retrieves column metadata for a table
func (h *Handler) getColumnInfo(ctx context.Context, tableName string) ([]ColumnInfo, error) {
	query := `
		SELECT 
			column_name,
			data_type,
			is_nullable,
			column_default
		FROM information_schema.columns
		WHERE table_schema = 'public' AND table_name = $1
		ORDER BY ordinal_position
	`

	rows, err := h.db.QueryContext(ctx, query, tableName)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var columns []ColumnInfo
	for rows.Next() {
		var c ColumnInfo
		var nullable string
		if err := rows.Scan(&c.Name, &c.Type, &nullable, &c.Default); err != nil {
			return nil, err
		}
		c.Nullable = nullable == "YES"
		columns = append(columns, c)
	}

	return columns, rows.Err()
}

// formatValue converts a database value to a display string
func formatValue(v any) string {
	if v == nil {
		return ""
	}

	switch val := v.(type) {
	case []byte:
		return string(val)
	case string:
		return val
	default:
		return fmt.Sprintf("%v", val)
	}
}
