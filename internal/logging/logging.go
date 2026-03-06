package logging

import (
	"log"
	"log/slog"
	"os"
)

// Setup initializes the global slog logger based on the environment.
// In production, logs are JSON-formatted for structured log aggregation on Render.
// In development, logs use a human-readable text format.
func Setup(env string) *slog.Logger {
	var handler slog.Handler
	if env == "prod" {
		handler = slog.NewJSONHandler(os.Stdout, &slog.HandlerOptions{
			Level: slog.LevelInfo,
		})
	} else {
		handler = slog.NewTextHandler(os.Stdout, &slog.HandlerOptions{
			Level: slog.LevelDebug,
		})
	}

	logger := slog.New(handler)
	slog.SetDefault(logger)

	// Bridge standard log to slog so existing log.Printf calls also go through slog
	log.SetFlags(0)
	log.SetOutput(&slogWriter{logger: logger})

	return logger
}

// slogWriter adapts slog.Logger to io.Writer so standard log output is captured.
type slogWriter struct {
	logger *slog.Logger
}

func (w *slogWriter) Write(p []byte) (n int, err error) {
	// Trim trailing newline that log package adds
	msg := string(p)
	if len(msg) > 0 && msg[len(msg)-1] == '\n' {
		msg = msg[:len(msg)-1]
	}
	w.logger.Info(msg, "source", "stdlog")
	return len(p), nil
}
