package main

import (
	"flag"
	"fmt"
	"log/slog"
	"os"

	"github.com/frc10101/TealTeam/internal/db"
	"github.com/frc10101/TealTeam/internal/frc"
	"github.com/frc10101/TealTeam/internal/handlers"
	"github.com/frc10101/TealTeam/internal/logging"
	"github.com/gin-gonic/gin"
	"github.com/joho/godotenv"
)

// Database configuration for different environments
var dbConfigs = map[string]string{
	"test": "postgres://user:password@127.0.0.1:5432/yourdb?sslmode=disable",
	"prod": "", // Set via RENDER_DATABASE_URL or DATABASE_URL environment variable
}

func main() {
	// Load .env file for local development (ignored if file doesn't exist)
	_ = godotenv.Load()

	// Parse command line flags
	env := flag.String("env", "", "Environment to use: 'test' (local Docker) or 'prod' (Render)")
	flag.Parse()

	// If env flag not provided, check ENVIRONMENT variable (set by Render)
	if *env == "" {
		if envVar := os.Getenv("ENVIRONMENT"); envVar != "" {
			if envVar == "production" {
				*env = "prod"
			} else {
				*env = "test"
			}
		} else {
			*env = "test"
		}
	}

	// Validate environment
	if *env != "test" && *env != "prod" {
		slog.Error("invalid environment", "env", *env)
		os.Exit(1)
	}

	// Initialize structured logging (JSON in prod, text in dev)
	logger := logging.Setup(*env)

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
		slog.Info("running in TEST mode", "database", "local-docker")
	case "prod":
		// Check for Render's database URL first, then fall back to DATABASE_URL
		databaseURL = os.Getenv("RENDER_DATABASE_URL")
		if databaseURL == "" {
			databaseURL = os.Getenv("DATABASE_URL")
		}
		if databaseURL == "" {
			slog.Error("production mode requires RENDER_DATABASE_URL or DATABASE_URL environment variable")
			os.Exit(1)
		}
		slog.Info("running in PRODUCTION mode", "database", "render")
	}

	// Set DATABASE_URL for db.Connect() to use
	os.Setenv("DATABASE_URL", databaseURL)

	// Initialize database
	database, err := db.Connect()
	if err != nil {
		slog.Warn("database connection failed, running without database", "error", err)
		database = nil
	} else {
		slog.Info("database connected successfully")
		if *env == "test" {
			if err := db.ResetMigrations(database); err != nil {
				slog.Error("migration reset failed", "error", err)
				os.Exit(1)
			}
		}
		if err := db.ApplyMigrations(database, "migrations"); err != nil {
			slog.Error("migration failed", "error", err)
			os.Exit(1)
		}

		frc.SyncOnBoot(database)

		// Start background team stats syncer
		syncConfig := frc.LoadSyncConfig()
		if syncConfig.TBAAuthKey == "" {
			slog.Warn("TBA_AUTH_KEY not configured, team stats sync disabled")
		} else {
			syncer := frc.NewTeamStatsSyncer(database, syncConfig)
			syncer.Start()
			defer syncer.Stop()
		}
	}
	defer func() {
		if database != nil {
			if sqlDB, err := database.DB(); err == nil {
				_ = sqlDB.Close()
			}
		}
	}()

	slog.Info("starting server", "env", *env, "port", port)

	// Set Gin mode based on environment
	if *env == "prod" {
		gin.SetMode(gin.ReleaseMode)
	}

	// Initialize handlers
	h := handlers.New(database)

	// Start background cleanup for expired sessions and submissions (20-min TTL)
	cleanupStop := make(chan struct{})
	go h.StartBackgroundCleanup(cleanupStop)
	defer close(cleanupStop)

	// Create Gin router
	router := gin.New()
	// Trust Render's proxy headers for proper client IP detection
	router.TrustedPlatform = "X-Forwarded-For"
	router.Use(logging.RequestLogger(logger), logging.Recovery(logger))

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
	router.GET("/teams", h.HandleTeamPage)
	router.GET("/account", h.HandleAccountPage)
	router.POST("/submission", h.HandleSubmission)

	// Authentication API routes
	router.POST("/api/auth/login", h.HandleLogin)
	router.POST("/api/auth/signup", h.HandleSignup)
	router.POST("/api/auth/logout", h.HandleLogout)
	router.POST("/api/account/change-password", h.HandleChangePassword)
	router.POST("/api/events/select", h.HandleSelectEvent)
	router.POST("/api/frc/sync", h.HandleFRCSync)

	// HTMX fragment routes (return HTML fragments only)
	router.GET("/hx/development/db/table/:name", h.HandleDBTableContent)
	router.GET("/hx/events/summary", h.HandleEventSummary)
	router.GET("/submission/event-teams", h.HandleGetEventTeams)
	router.GET("/hx/teams/search", h.HandleTeamSearch)
	router.GET("/hx/teams/data", h.HandleTeamEventData)
	router.POST("/hx/teams/fetch-past-events", h.HandleFetchPastEvents)
	router.GET("/hx/matches/schedule", h.HandleMatchSchedule)
	router.GET("/hx/drive-coach/matches", h.HandleDriveCoachMatches)
	router.POST("/hx/lead-scout/submissions/:id/approve", h.HandleApproveSubmission)
	router.POST("/hx/lead-scout/submissions/:id/decline", h.HandleDeclineSubmission)
	router.POST("/submission/resubmit/:id", h.HandleResubmit)
	router.GET("/submission/edit/:id", h.HandleEditRejectedSubmission)

	// TODO: Add more routes here
	// Full pages: router.GET("/yourpage", h.HandleYourPage)
	// HTMX fragments: router.GET("/hx/yourfeature/fragment", h.HandleYourFragment)

	// Start server
	slog.Info("server listening", "addr", fmt.Sprintf("http://localhost:%s", port))
	if err := router.Run(":" + port); err != nil {
		slog.Error("server failed", "error", err)
		os.Exit(1)
	}
}
