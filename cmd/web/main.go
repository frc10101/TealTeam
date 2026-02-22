package main

import (
	"flag"
	"fmt"
	"log"
	"os"

	"github.com/frc10101/TealTeam/internal/db"
	"github.com/frc10101/TealTeam/internal/frc"
	"github.com/frc10101/TealTeam/internal/handlers"
	"github.com/gin-gonic/gin"
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
		if *env == "test" {
			if err := db.ResetMigrations(database); err != nil {
				log.Fatalf("Migration reset failed: %v", err)
			}
		}
		if err := db.ApplyMigrations(database, "migrations"); err != nil {
			log.Fatalf("Migration failed: %v", err)
		}

		frc.SyncOnBoot(database)
	}
	defer func() {
		if database != nil {
			if sqlDB, err := database.DB(); err == nil {
				_ = sqlDB.Close()
			}
		}
	}()

	fmt.Printf("\n📋 Environment: %s\n", *env)

	// Initialize handlers
	h := handlers.New(database)

	// Create Gin router
	router := gin.New()
	router.Use(gin.Logger(), gin.Recovery())

	// Static files
	router.Static("/static", "./web/static")

	// Full page routes (render with layout)
	router.GET("/", h.HandleIndex)
	router.GET("/submission", h.HandleSubmissionPage)
	router.GET("/lead-scout", h.HandleAdminViewer)
	router.GET("/lead-scout/submissions/:id", h.HandleViewSubmission)
	router.GET("/drive-coach", h.HandleCoachViewer)
	router.GET("/development/db", h.HandleDBViewer)
	router.GET("/sign-in", h.HandleSignIn)
	router.GET("/sign-up", h.HandleSignUp)
	router.POST("/submission", h.HandleSubmission)

	// Authentication API routes
	router.POST("/api/auth/login", h.HandleLogin)
	router.POST("/api/auth/signup", h.HandleSignup)
	router.POST("/api/auth/logout", h.HandleLogout)
	router.POST("/api/events/select", h.HandleSelectEvent)
	router.POST("/api/frc/sync", h.HandleFRCSync)

	// HTMX fragment routes (return HTML fragments only)
	router.GET("/hx/development/db/table/:name", h.HandleDBTableContent)
	router.GET("/hx/events/summary", h.HandleEventSummary)
	router.GET("/submission/event-teams", h.HandleGetEventTeams)
	router.POST("/hx/lead-scout/submissions/:id/approve", h.HandleApproveSubmission)
	router.POST("/hx/lead-scout/submissions/:id/decline", h.HandleDeclineSubmission)

	// TODO: Add more routes here
	// Full pages: router.GET("/yourpage", h.HandleYourPage)
	// HTMX fragments: router.GET("/hx/yourfeature/fragment", h.HandleYourFragment)

	// Start server
	log.Printf("Server starting on http://localhost:%s", port)
	if err := router.Run(":" + port); err != nil {
		log.Fatal(err)
	}
}
