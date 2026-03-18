package frc

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/url"
	"strings"
	"sync"
	"time"
)

var ErrInternetUnavailable = errors.New("internet unavailable")

const (
	connectivityProbeAddress = "1.1.1.1:443"
	connectivityProbeNetwork = "tcp"
	connectivityProbeTimeout = 1500 * time.Millisecond
	connectivityCacheTTL     = 3 * time.Second
	apiRetryMaxAttempts      = 3
)

type connectivityCacheState struct {
	CheckedAt time.Time
	OK        bool
	ErrText   string
}

type networkRuntimeState struct {
	mu                 sync.RWMutex
	connectivity       connectivityCacheState
	LastAPISuccessAt   time.Time
	LastAPIErrorAt     time.Time
	LastAPIError       string
	LastSuccessfulSync time.Time
}

// NetworkStatusSnapshot provides network and API health details for UI and logs.
type NetworkStatusSnapshot struct {
	CheckedAt          time.Time `json:"checkedAt,omitempty"`
	InternetReachable  bool      `json:"internetReachable"`
	InternetError      string    `json:"internetError,omitempty"`
	LastAPISuccessAt   time.Time `json:"lastApiSuccessAt,omitempty"`
	LastAPIErrorAt     time.Time `json:"lastApiErrorAt,omitempty"`
	LastAPIError       string    `json:"lastApiError,omitempty"`
	LastSuccessfulSync time.Time `json:"lastSuccessfulSync,omitempty"`
}

var runtimeState networkRuntimeState

// IsInternetUnavailable reports whether an error was caused by a failed connectivity probe.
func IsInternetUnavailable(err error) bool {
	return errors.Is(err, ErrInternetUnavailable)
}

// RefreshConnectivity performs a connectivity probe (using cached result within TTL).
func RefreshConnectivity(ctx context.Context) error {
	return ensureInternetConnectivity(ctx)
}

func ensureInternetConnectivityForBaseURL(ctx context.Context, baseURL string) error {
	u, err := url.Parse(baseURL)
	if err != nil {
		return ensureInternetConnectivity(ctx)
	}

	host := strings.TrimSpace(u.Hostname())
	if host == "" {
		return ensureInternetConnectivity(ctx)
	}

	port := u.Port()
	if port == "" {
		if strings.EqualFold(u.Scheme, "http") {
			port = "80"
		} else {
			port = "443"
		}
	}

	probeAddress := net.JoinHostPort(host, port)

	dialCtx, cancel := context.WithTimeout(ctx, connectivityProbeTimeout)
	defer cancel()

	var d net.Dialer
	conn, err := d.DialContext(dialCtx, connectivityProbeNetwork, probeAddress)
	if err != nil {
		reason := fmt.Sprintf("failed to reach %s (%v)", probeAddress, err)
		recordConnectivityResult(false, reason)
		return fmt.Errorf("%w: failed to reach %s (%v)", ErrInternetUnavailable, probeAddress, err)
	}
	_ = conn.Close()
	recordConnectivityResult(true, "")

	return nil
}

// GetNetworkStatusSnapshot returns the current health snapshot.
func GetNetworkStatusSnapshot() NetworkStatusSnapshot {
	runtimeState.mu.RLock()
	defer runtimeState.mu.RUnlock()

	return NetworkStatusSnapshot{
		CheckedAt:          runtimeState.connectivity.CheckedAt,
		InternetReachable:  runtimeState.connectivity.OK,
		InternetError:      runtimeState.connectivity.ErrText,
		LastAPISuccessAt:   runtimeState.LastAPISuccessAt,
		LastAPIErrorAt:     runtimeState.LastAPIErrorAt,
		LastAPIError:       runtimeState.LastAPIError,
		LastSuccessfulSync: runtimeState.LastSuccessfulSync,
	}
}

func recordConnectivityResult(ok bool, errText string) {
	runtimeState.mu.Lock()
	defer runtimeState.mu.Unlock()

	runtimeState.connectivity.CheckedAt = time.Now().UTC()
	runtimeState.connectivity.OK = ok
	runtimeState.connectivity.ErrText = errText
}

func recordAPISuccess() {
	runtimeState.mu.Lock()
	defer runtimeState.mu.Unlock()

	now := time.Now().UTC()
	runtimeState.LastAPISuccessAt = now
	// Treat successful API calls as authoritative proof that internet/API path is reachable.
	runtimeState.connectivity.CheckedAt = now
	runtimeState.connectivity.OK = true
	runtimeState.connectivity.ErrText = ""
	// Keep last error timestamp/message for diagnostics.
}

func recordAPIError(err error) {
	if err == nil {
		return
	}
	runtimeState.mu.Lock()
	defer runtimeState.mu.Unlock()

	runtimeState.LastAPIErrorAt = time.Now().UTC()
	runtimeState.LastAPIError = err.Error()
}

func recordSuccessfulSync() {
	runtimeState.mu.Lock()
	defer runtimeState.mu.Unlock()
	runtimeState.LastSuccessfulSync = time.Now().UTC()
}

func shouldRetryRequestError(err error) bool {
	if err == nil {
		return false
	}
	if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
		return false
	}
	var netErr net.Error
	if errors.As(err, &netErr) {
		return true
	}
	return false
}

func shouldRetryStatusCode(statusCode int) bool {
	if statusCode == 429 {
		return true
	}
	return statusCode >= 500 && statusCode <= 599
}

func backoffDelay(attempt int) time.Duration {
	if attempt <= 0 {
		return 250 * time.Millisecond
	}
	if attempt == 1 {
		return 500 * time.Millisecond
	}
	return time.Second
}

func sleepWithContext(ctx context.Context, d time.Duration) error {
	if d <= 0 {
		return nil
	}
	t := time.NewTimer(d)
	defer t.Stop()

	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-t.C:
		return nil
	}
}

func shouldSkipConnectivityCheck(baseURL string) bool {
	u, err := url.Parse(baseURL)
	if err != nil {
		return false
	}

	host := strings.TrimSpace(u.Hostname())
	if host == "" {
		return false
	}

	if strings.EqualFold(host, "localhost") {
		return true
	}

	ip := net.ParseIP(host)
	if ip == nil {
		return false
	}

	return ip.IsLoopback() || ip.IsPrivate() || ip.IsLinkLocalUnicast()
}

func ensureInternetConnectivity(ctx context.Context) error {
	runtimeState.mu.RLock()
	cached := runtimeState.connectivity
	runtimeState.mu.RUnlock()

	if !cached.CheckedAt.IsZero() && time.Since(cached.CheckedAt) <= connectivityCacheTTL {
		if cached.OK {
			return nil
		}
		return fmt.Errorf("%w: %s", ErrInternetUnavailable, cached.ErrText)
	}

	dialCtx, cancel := context.WithTimeout(ctx, connectivityProbeTimeout)
	defer cancel()

	var d net.Dialer
	conn, err := d.DialContext(dialCtx, connectivityProbeNetwork, connectivityProbeAddress)
	if err != nil {
		reason := fmt.Sprintf("failed to reach %s (%v)", connectivityProbeAddress, err)
		recordConnectivityResult(false, reason)
		return fmt.Errorf("%w: failed to reach %s (%v)", ErrInternetUnavailable, connectivityProbeAddress, err)
	}
	_ = conn.Close()
	recordConnectivityResult(true, "")

	return nil
}
