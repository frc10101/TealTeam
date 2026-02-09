package handlers

import (
	"context"
	"database/sql"
	"fmt"
	"net/http"
	"strconv"

	"github.com/gin-gonic/gin"
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
func (h *Handler) HandleDBViewer(c *gin.Context) {
	// Check if user is authenticated
	user, err := h.GetSessionUser(c)
	if err != nil || user == nil {
		// Redirect to sign-in page
		http.Redirect(c.Writer, c.Request, "/sign-in", http.StatusSeeOther)
		return
	}

	data := map[string]any{
		"Title":       "Database Viewer",
		"DBConnected": h.hasDB(),
		"User":        user,
	}

	if h.hasDB() {
		tables, err := h.getTableList(c.Request.Context())
		if err != nil {
			data["Error"] = fmt.Sprintf("Failed to get tables: %v", err)
		} else {
			data["Tables"] = tables
		}

		// Check if a table is selected via query param
		selectedTable := c.Query("table")
		if selectedTable != "" {
			data["SelectedTable"] = selectedTable
		}
	}

	h.render(c, "db_viewer", data)
}

// HandleDBTableContent returns the table content as an HTMX fragment
// Route: GET /hx/development/db/table/{name}
func (h *Handler) HandleDBTableContent(c *gin.Context) {
	if !h.hasDB() {
		http.Error(c.Writer, "Database not connected", http.StatusServiceUnavailable)
		return
	}

	tableName := c.Param("name")
	if tableName == "" {
		http.Error(c.Writer, "Table name is required", http.StatusBadRequest)
		return
	}

	// Parse pagination params
	offset, _ := strconv.Atoi(c.Query("offset"))
	limit, _ := strconv.Atoi(c.Query("limit"))
	if limit <= 0 {
		limit = 50
	}
	if offset < 0 {
		offset = 0
	}

	data, err := h.getTableData(c.Request.Context(), tableName, offset, limit)
	if err != nil {
		http.Error(c.Writer, fmt.Sprintf("Failed to get table data: %v", err), http.StatusInternalServerError)
		return
	}

	h.renderPartial(c, "db_table_content", data)
}

// getTableList retrieves all user tables from the database
func (h *Handler) getTableList(ctx context.Context) ([]TableInfo, error) {
	var tableNames []string
	if err := h.db.WithContext(ctx).
		Table("information_schema.tables").
		Select("table_name").
		Where("table_schema = ? AND table_type = ?", "public", "BASE TABLE").
		Order("table_name").
		Scan(&tableNames).Error; err != nil {
		return nil, err
	}

	var tables []TableInfo
	for _, name := range tableNames {
		var colCount int64
		if err := h.db.WithContext(ctx).
			Table("information_schema.columns").
			Where("table_schema = ? AND table_name = ?", "public", name).
			Count(&colCount).Error; err != nil {
			return nil, err
		}

		var rowCount int64
		if err := h.db.WithContext(ctx).Table(name).Count(&rowCount).Error; err != nil {
			rowCount = 0
		}

		tables = append(tables, TableInfo{Name: name, RowCount: rowCount})
	}

	return tables, nil
}

// getTableData retrieves data and schema for a specific table
func (h *Handler) getTableData(ctx context.Context, tableName string, offset, limit int) (map[string]any, error) {
	data := map[string]any{
		"SelectedTable": tableName,
		"Offset":        offset,
		"Limit":         limit,
	}

	allowed, err := h.isTableAllowed(ctx, tableName)
	if err != nil {
		return nil, fmt.Errorf("failed to validate table: %w", err)
	}
	if !allowed {
		return nil, fmt.Errorf("invalid table name")
	}

	// Get column info
	columns, err := h.getColumnInfo(ctx, tableName)
	if err != nil {
		return nil, fmt.Errorf("failed to get columns: %w", err)
	}
	data["Columns"] = columns

	// Get total row count
	var totalRows int64
	if err := h.db.WithContext(ctx).Table(tableName).Count(&totalRows).Error; err != nil {
		return nil, fmt.Errorf("failed to count rows: %w", err)
	}
	data["TotalRows"] = totalRows

	// Get rows
	var rowMaps []map[string]any
	query := h.db.WithContext(ctx).Table(tableName).Limit(limit).Offset(offset)
	if len(columns) > 0 {
		query = query.Order(columns[0].Name)
	}
	if err := query.Find(&rowMaps).Error; err != nil {
		return nil, fmt.Errorf("failed to query rows: %w", err)
	}

	var rowData [][]any
	for _, row := range rowMaps {
		rowValues := make([]any, len(columns))
		for i, col := range columns {
			rowValues[i] = formatValue(row[col.Name])
		}
		rowData = append(rowData, rowValues)
	}
	data["Rows"] = rowData

	return data, nil
}

// getColumnInfo retrieves column metadata for a table
func (h *Handler) getColumnInfo(ctx context.Context, tableName string) ([]ColumnInfo, error) {
	var rawColumns []struct {
		Name     string
		Type     string
		Nullable string
		Default  sql.NullString
	}

	if err := h.db.WithContext(ctx).
		Table("information_schema.columns").
		Select("column_name as name, data_type as type, is_nullable as nullable, column_default as default").
		Where("table_schema = ? AND table_name = ?", "public", tableName).
		Order("ordinal_position").
		Scan(&rawColumns).Error; err != nil {
		return nil, err
	}

	columns := make([]ColumnInfo, 0, len(rawColumns))
	for _, col := range rawColumns {
		columns = append(columns, ColumnInfo{
			Name:     col.Name,
			Type:     col.Type,
			Nullable: col.Nullable == "YES",
			Default:  col.Default,
		})
	}

	return columns, nil
}

func (h *Handler) isTableAllowed(ctx context.Context, tableName string) (bool, error) {
	var count int64
	if err := h.db.WithContext(ctx).
		Table("information_schema.tables").
		Where("table_schema = ? AND table_type = ? AND table_name = ?", "public", "BASE TABLE", tableName).
		Count(&count).Error; err != nil {
		return false, err
	}
	return count > 0, nil
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
