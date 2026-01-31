# Build stage
FROM golang:1.22-alpine AS builder

WORKDIR /app

# Install build dependencies
RUN apk add --no-cache git nodejs npm

# Copy go mod files
COPY go.mod go.sum* ./
RUN go mod download

# Copy package files and install npm deps for Tailwind
COPY package.json package-lock.json* ./
RUN npm install

# Copy source code
COPY . .

# Build Tailwind CSS
RUN npm run css:build

# Build the Go binary
RUN CGO_ENABLED=0 GOOS=linux go build -o /server ./cmd/web

# Runtime stage
FROM alpine:latest

WORKDIR /app

# Install CA certificates for HTTPS
RUN apk --no-cache add ca-certificates

# Copy binary from builder
COPY --from=builder /server /server

# Copy static assets and templates
COPY --from=builder /app/web /app/web

# Expose port (Render will set PORT env var)
EXPOSE 8080

# Run the server
CMD ["/server"]
