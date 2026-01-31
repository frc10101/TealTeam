/*
 * Site JavaScript
 * 
 * Keep this file MINIMAL. Business logic belongs on the server.
 * Use this only for:
 * - UI polish (animations, transitions)
 * - Modal handling
 * - Small utility functions
 * - Progressive enhancement
 */

// Log when HTMX makes requests (useful for debugging)
document.body.addEventListener('htmx:beforeRequest', function(event) {
  console.debug('HTMX Request:', event.detail.pathInfo.requestPath);
});

// Handle HTMX errors
document.body.addEventListener('htmx:responseError', function(event) {
  console.error('HTMX Error:', event.detail.xhr.status, event.detail.xhr.statusText);
  // TODO: Show user-friendly error message
});

// TODO: Add UI utility functions here
// Example: Modal handling, toast notifications, etc.

/*
 * Example: Simple toast notification helper
 * 
 * function showToast(message, type = 'info') {
 *   const toast = document.createElement('div');
 *   toast.className = `alert alert-${type} fixed bottom-4 right-4 z-50`;
 *   toast.textContent = message;
 *   document.body.appendChild(toast);
 *   setTimeout(() => toast.remove(), 3000);
 * }
 */
