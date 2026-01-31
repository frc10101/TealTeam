/*
 * Site TypeScript
 * 
 * Keep this file MINIMAL. Business logic belongs on the server.
 * Use this only for:
 * - UI polish (animations, transitions)
 * - Modal handling
 * - Small utility functions
 * - Progressive enhancement
 */

// Type declarations for HTMX events
interface HtmxRequestEvent extends Event {
  detail: {
    pathInfo: {
      requestPath: string;
    };
  };
}

interface HtmxResponseErrorEvent extends Event {
  detail: {
    xhr: XMLHttpRequest;
  };
}

// Log when HTMX makes requests (useful for debugging)
document.body.addEventListener('htmx:beforeRequest', function(event: Event) {
  const htmxEvent = event as HtmxRequestEvent;
  console.debug('HTMX Request:', htmxEvent.detail.pathInfo.requestPath);
});

// Handle HTMX errors
document.body.addEventListener('htmx:responseError', function(event: Event) {
  const htmxEvent = event as HtmxResponseErrorEvent;
  console.error('HTMX Error:', htmxEvent.detail.xhr.status, htmxEvent.detail.xhr.statusText);
  // TODO: Show user-friendly error message
});

// TODO: Add UI utility functions here
// Example: Modal handling, toast notifications, etc.

/*
 * Example: Simple toast notification helper
 * 
 * function showToast(message: string, type: string = 'info'): void {
 *   const toast = document.createElement('div');
 *   toast.className = `alert alert-${type} fixed bottom-4 right-4 z-50`;
 *   toast.textContent = message;
 *   document.body.appendChild(toast);
 *   setTimeout(() => toast.remove(), 3000);
 * }
 */
