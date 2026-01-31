package handlers

import (
	"net/http"
	"strconv"
	"time"

	"github.com/yourusername/yourproject/internal/models"
)

// Example in-memory data store (replace with DB queries in production)
var exampleItems = []models.Item{
	{ID: 1, Name: "Example Item 1", Description: "First item description", CreatedAt: time.Now()},
	{ID: 2, Name: "Example Item 2", Description: "Second item description", CreatedAt: time.Now()},
	{ID: 3, Name: "Example Item 3", Description: "Third item description", CreatedAt: time.Now()},
}
var nextID = 4

// HandleExampleTable returns an HTML table fragment
// Route: GET /hx/example/table
func (h *Handler) HandleExampleTable(w http.ResponseWriter, r *http.Request) {
	// TODO: Replace with database query when DB is connected
	// Example DB query:
	// items, err := h.db.QueryItems(r.Context())
	// if err != nil {
	//     http.Error(w, "Failed to load items", http.StatusInternalServerError)
	//     return
	// }

	data := map[string]any{
		"Items": exampleItems,
	}

	h.renderPartial(w, "example_table", data)
}

// HandleExampleItemCreate handles form submission to create an item
// Route: POST /hx/example/item
func (h *Handler) HandleExampleItemCreate(w http.ResponseWriter, r *http.Request) {
	if err := r.ParseForm(); err != nil {
		http.Error(w, "Invalid form data", http.StatusBadRequest)
		return
	}

	name := r.FormValue("name")
	description := r.FormValue("description")

	if name == "" {
		http.Error(w, "Name is required", http.StatusBadRequest)
		return
	}

	// TODO: Replace with database insert
	// Example DB insert:
	// item, err := h.db.CreateItem(r.Context(), name, description)
	// if err != nil {
	//     http.Error(w, "Failed to create item", http.StatusInternalServerError)
	//     return
	// }

	newItem := models.Item{
		ID:          nextID,
		Name:        name,
		Description: description,
		CreatedAt:   time.Now(),
	}
	nextID++
	exampleItems = append(exampleItems, newItem)

	// Return the updated table
	data := map[string]any{
		"Items": exampleItems,
	}

	h.renderPartial(w, "example_table", data)
}

// HandleExampleItemDelete handles item deletion
// Route: DELETE /hx/example/item/{id}
func (h *Handler) HandleExampleItemDelete(w http.ResponseWriter, r *http.Request) {
	idStr := r.PathValue("id")
	id, err := strconv.Atoi(idStr)
	if err != nil {
		http.Error(w, "Invalid ID", http.StatusBadRequest)
		return
	}

	// TODO: Replace with database delete
	// Example DB delete:
	// err := h.db.DeleteItem(r.Context(), id)
	// if err != nil {
	//     http.Error(w, "Failed to delete item", http.StatusInternalServerError)
	//     return
	// }

	// Remove item from slice
	for i, item := range exampleItems {
		if item.ID == id {
			exampleItems = append(exampleItems[:i], exampleItems[i+1:]...)
			break
		}
	}

	// Return the updated table
	data := map[string]any{
		"Items": exampleItems,
	}

	h.renderPartial(w, "example_table", data)
}

// TODO: Add more HTMX handlers here
// Remember: HTMX handlers return HTML FRAGMENTS only, not full pages
//
// func (h *Handler) HandleYourFragment(w http.ResponseWriter, r *http.Request) {
//     data := map[string]any{
//         "YourData": yourData,
//     }
//     h.renderPartial(w, "your_partial", data)
// }
