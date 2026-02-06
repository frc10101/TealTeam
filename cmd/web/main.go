package main

import (
	"flag"
	"fmt"
	"log"
	"net/http"
	"os"

	"github.com/frc10101/TealTeam/internal/db"
	"github.com/frc10101/TealTeam/internal/handlers"
	"github.com/frc10101/TealTeam/internal/middleware"
)

// Database configuration for different environments
var dbConfigs = map[string]string{
	"test": "postgres://user:password@127.0.0.1:5432/yourdb?sslmode=disable",
	"prod": "", // Set via RENDER_DATABASE_URL or DATABASE_URL environment variable
}

func main() {
	// Parse command line flags
	env := flag.String("env", "test", "Environment to use: 'test' (local Docker) or 'prod' (Render)")
	flag.Parse()

	// Validate environment
	if *env != "test" && *env != "prod" {
		log.Fatalf("Invalid environment '%s'. Use 'test' or 'prod'", *env)
	}

	// Load configuration
	port := os.Getenv("PORT")
	if port == "" {
		port = "8080"
	}

	// Set database URL based on environment
	var databaseURL string
	switch *env {
	case "test":
		// Prefer DATABASE_URL from environment (e.g., Docker) if set
		databaseURL = os.Getenv("DATABASE_URL")
		if databaseURL == "" {
			databaseURL = dbConfigs["test"]
		}
		log.Println("🧪 Running in TEST mode (local Docker database)")
	case "prod":
		// Check for Render's database URL first, then fall back to DATABASE_URL
		databaseURL = os.Getenv("RENDER_DATABASE_URL")
		if databaseURL == "" {
			databaseURL = os.Getenv("DATABASE_URL")
		}
		if databaseURL == "" {
			log.Fatal("❌ Production mode requires RENDER_DATABASE_URL or DATABASE_URL environment variable")
		}
		log.Println("🚀 Running in PRODUCTION mode (Render database)")
	}

	// Set DATABASE_URL for db.Connect() to use
	os.Setenv("DATABASE_URL", databaseURL)

	// Initialize database
	database, err := db.Connect()
	if err != nil {
		log.Printf("Warning: Database connection failed: %v", err)
		log.Println("Running without database support")
		database = nil
	} else {
		log.Println("✅ Database connected successfully")
	}
	defer func() {
		if database != nil {
			database.Close()
		}
	}()

	fmt.Printf("\n📋 Environment: %s\n", *env)

	// Initialize handlers
	h := handlers.New(database)

	// Create router
	mux := http.NewServeMux()

	// Static files
	fs := http.FileServer(http.Dir("web/static"))
	mux.Handle("GET /static/", http.StripPrefix("/static/", fs))

	// Full page routes (render with layout)
	mux.HandleFunc("GET /", h.HandleIndex)
	mux.HandleFunc("GET /submission", h.HandleSubmissionPage)
	mux.HandleFunc("GET /development/db", h.HandleDBViewer)
	mux.HandleFunc("GET /sign-in", h.HandleSignIn)
	mux.HandleFunc("GET /sign-up", h.HandleSignUp)

	// Authentication API routes
	mux.HandleFunc("POST /api/auth/login", h.HandleLogin)
	mux.HandleFunc("POST /api/auth/signup", h.HandleSignup)
	mux.HandleFunc("POST /api/auth/logout", h.HandleLogout)

	// HTMX fragment routes (return HTML fragments only)
	mux.HandleFunc("GET /hx/development/db/table/{name}", h.HandleDBTableContent)

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
