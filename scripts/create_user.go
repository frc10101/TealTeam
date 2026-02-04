package main

import (
	"database/sql"
	"flag"
	"fmt"
	"log"
	"os"

	_ "github.com/lib/pq"
	"golang.org/x/crypto/bcrypt"
)

func createUser() {
	// Parse command line flags
	email := flag.String("email", "", "User email")
	password := flag.String("password", "", "User password")
	name := flag.String("name", "", "User name")
	flag.Parse()

	if *email == "" || *password == "" || *name == "" {
		log.Fatal("Usage: go run scripts/create_user.go -email=user@example.com -password=yourpassword -name='Your Name'")
	}

	// Connect to database
	databaseURL := os.Getenv("DATABASE_URL")
	if databaseURL == "" {
		databaseURL = "postgres://user:password@localhost:5432/yourdb?sslmode=disable"
	}

	db, err := sql.Open("postgres", databaseURL)
	if err != nil {
		log.Fatalf("Failed to connect to database: %v", err)
	}
	defer db.Close()

	// Test connection
	if err := db.Ping(); err != nil {
		log.Fatalf("Failed to ping database: %v", err)
	}

	// Hash password
	hashedPassword, err := bcrypt.GenerateFromPassword([]byte(*password), 12)
	if err != nil {
		log.Fatalf("Failed to hash password: %v", err)
	}

	// Insert user
	var userID int
	err = db.QueryRow(
		`INSERT INTO users (email, name, password_hash, role) 
		 VALUES ($1, $2, $3, 'user') 
		 RETURNING id`,
		*email, *name, string(hashedPassword),
	).Scan(&userID)

	if err != nil {
		log.Fatalf("Failed to create user: %v", err)
	}

	fmt.Printf("✅ User created successfully!\n")
	fmt.Printf("   ID: %d\n", userID)
	fmt.Printf("   Email: %s\n", *email)
	fmt.Printf("   Name: %s\n", *name)
	fmt.Printf("\nYou can now sign in at http://localhost:8080/sign-in\n")
}
