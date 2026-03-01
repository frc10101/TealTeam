# Build stage
FROM golang:1.24-alpine AS builder

WORKDIR /app

# Install Node.js and npm for Tailwind CSS and TypeScript
RUN apk add --no-cache nodejs npm

# Copy package files and install npm dependencies first (for better caching)
COPY package.json package-lock.json* ./
RUN npm install

# Copy all source files needed for CSS and TS compilation
COPY web/tailwind ./web/tailwind
COPY web/static/js ./web/static/js
COPY web/templates ./web/templates
COPY tailwind.config.js ./
COPY tsconfig.json ./

# Create output directories and clean old build artifacts
RUN mkdir -p ./web/static/css && rm -f ./web/static/js/site.js ./web/static/css/site.css

# Build Tailwind CSS and TypeScript (npm run build = both)
RUN npm run build

# Verify CSS was built
RUN ls -la ./web/static/css/

# Copy go mod files for Go dependency caching
COPY go.mod go.sum ./
RUN go mod download

# Copy remaining source code
COPY cmd ./cmd
COPY internal ./internal
COPY migrations ./migrations

# Copy environment file for tests if present, otherwise use example
COPY .env* ./

# Test stage - Run all tests to ensure code quality
# Tests use environment variables from .env file
RUN go test -v ./...

# Build the Go binary
RUN CGO_ENABLED=0 GOOS=linux go build -o /server ./cmd/web

# Runtime stage (production-ready, minimal image)
FROM alpine:latest

WORKDIR /app

# Install CA certificates for HTTPS
RUN apk --no-cache add ca-certificates

# Copy binary from builder
COPY --from=builder /server /server

# Copy static assets and templates (includes compiled CSS and JS)
COPY --from=builder /app/web ./web

# Copy migrations for runtime auto-apply
COPY --from=builder /app/migrations ./migrations

# Expose port (Render will bind to PORT env var)
EXPOSE 8080

# Run the server
CMD ["/server"]
