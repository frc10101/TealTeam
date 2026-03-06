package main

import (
	"flag"
	"fmt"
	"log"
	"os"

	appdb "github.com/frc10101/TealTeam/internal/db"
	"github.com/frc10101/TealTeam/internal/models"
	"golang.org/x/crypto/bcrypt"
)

const defaultDBURL = "postgres://user:password@localhost:5432/yourdb?sslmode=disable"

func createUser() {
	// Parse command line flags
	email := flag.String("email", "", "User email")
	password := flag.String("password", "", "User password")
	name := flag.String("name", "", "User name")
	flag.Parse()

	if *email == "" || *password == "" || *name == "" {
		log.Fatal("Usage: go run ./cmd/scripts/create_user -email=user@example.com -password=yourpassword -name='Your Name'")
	}

	// Connect to database
	databaseURL := os.Getenv("DATABASE_URL")
	if databaseURL == "" {
		databaseURL = defaultDBURL
		if err := os.Setenv("DATABASE_URL", databaseURL); err != nil {
			log.Fatalf("Failed to set DATABASE_URL: %v", err)
		}
	}

	db, err := appdb.Connect()
	if err != nil {
		log.Fatalf("Failed to connect to database: %v", err)
	}
	sqlDB, err := db.DB()
	if err != nil {
		log.Fatalf("Failed to access sql DB: %v", err)
	}
	defer sqlDB.Close()

	// Hash password
	hashedPassword, err := bcrypt.GenerateFromPassword([]byte(*password), 12)
	if err != nil {
		log.Fatalf("Failed to hash password: %v", err)
	}

	// Insert user
	user := models.User{
		Email:        *email,
		Name:         *name,
		PasswordHash: string(hashedPassword),
		Role:         "user",
	}
	if err := db.Create(&user).Error; err != nil {
		log.Fatalf("Failed to create user: %v", err)
	}

	fmt.Printf("✅ User created successfully!\n")
	fmt.Printf("   ID: %d\n", user.ID)
	fmt.Printf("   Email: %s\n", *email)
	fmt.Printf("   Name: %s\n", *name)
	fmt.Printf("\nYou can now sign in at http://localhost:8080/sign-in\n")
}
