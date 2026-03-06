package logging

import (
	"log/slog"
	"time"

	"github.com/gin-gonic/gin"
)

// RequestLogger returns a Gin middleware that logs each HTTP request using slog.
// It captures method, path, status, latency, client IP, and user agent.
func RequestLogger(logger *slog.Logger) gin.HandlerFunc {
	return func(c *gin.Context) {
		start := time.Now()
		path := c.Request.URL.Path
		query := c.Request.URL.RawQuery

		c.Next()

		latency := time.Since(start)
		status := c.Writer.Status()

		attrs := []slog.Attr{
			slog.Int("status", status),
			slog.String("method", c.Request.Method),
			slog.String("path", path),
			slog.Duration("latency", latency),
			slog.String("ip", c.ClientIP()),
			slog.Int("bytes", c.Writer.Size()),
		}
		if query != "" {
			attrs = append(attrs, slog.String("query", query))
		}
		if c.Errors.String() != "" {
			attrs = append(attrs, slog.String("errors", c.Errors.String()))
		}

		msg := "HTTP request"
		if status >= 500 {
			logger.LogAttrs(c.Request.Context(), slog.LevelError, msg, attrs...)
		} else if status >= 400 {
			logger.LogAttrs(c.Request.Context(), slog.LevelWarn, msg, attrs...)
		} else {
			logger.LogAttrs(c.Request.Context(), slog.LevelInfo, msg, attrs...)
		}
	}
}

// Recovery returns a Gin middleware that recovers from panics and logs them via slog.
func Recovery(logger *slog.Logger) gin.HandlerFunc {
	return func(c *gin.Context) {
		defer func() {
			if err := recover(); err != nil {
				logger.Error("panic recovered",
					"error", err,
					"method", c.Request.Method,
					"path", c.Request.URL.Path,
					"ip", c.ClientIP(),
				)
				c.AbortWithStatus(500)
			}
		}()
		c.Next()
	}
}
