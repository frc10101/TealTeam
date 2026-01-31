package main

import (
	"log"
	"net/http"
	"os"

	"github.com/yourusername/yourproject/internal/db"
	"github.com/yourusername/yourproject/internal/handlers"
	"github.com/yourusername/yourproject/internal/middleware"
)

func main() {
	// Load configuration
	port := os.Getenv("PORT")
	if port == "" {
		port = "8080"
	}

	// Initialize database (optional - comment out if not using)
	database, err := db.Connect()
	if err != nil {
		log.Printf("Warning: Database connection failed: %v", err)
		log.Println("Running without database support")
		database = nil
	}
	defer func() {
		if database != nil {
			database.Close()
		}
	}()

	// Initialize handlers
	h := handlers.New(database)

	// Create router
	mux := http.NewServeMux()

	// Static files
	fs := http.FileServer(http.Dir("web/static"))
	mux.Handle("GET /static/", http.StripPrefix("/static/", fs))

	// Full page routes (render with layout)
	mux.HandleFunc("GET /", h.HandleIndex)
	mux.HandleFunc("GET /example", h.HandleExamplePage)

	// HTMX fragment routes (return HTML fragments only)
	mux.HandleFunc("GET /hx/example/table", h.HandleExampleTable)
	mux.HandleFunc("POST /hx/example/item", h.HandleExampleItemCreate)
	mux.HandleFunc("DELETE /hx/example/item/{id}", h.HandleExampleItemDelete)

	// TODO: Add more routes here
	// Full pages: mux.HandleFunc("GET /yourpage", h.HandleYourPage)
	// HTMX fragments: mux.HandleFunc("GET /hx/yourfeature/fragment", h.HandleYourFragment)

	// Apply middleware
	handler := middleware.Chain(
		mux,
		middleware.Logger,
		middleware.Recover,
	)

	// Start server
	log.Printf("Server starting on http://localhost:%s", port)
	if err := http.ListenAndServe(":"+port, handler); err != nil {
		log.Fatal(err)
	}
}
