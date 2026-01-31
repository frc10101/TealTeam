#!/bin/bash
# Development script - run Go server with hot reload using air (if installed)
# Or run directly with go run

set -e

# Check if air is installed for hot reload
if command -v air &> /dev/null; then
    echo "Starting with hot reload (air)..."
    air
else
    echo "Starting Go server..."
    echo "TIP: Install air for hot reload: go install github.com/cosmtrek/air@latest"
    go run ./cmd/web
fi
