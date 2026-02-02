"use strict";
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
// Log when HTMX makes requests (useful for debugging)
document.body.addEventListener('htmx:beforeRequest', function (event) {
    const htmxEvent = event;
    console.debug('HTMX Request:', htmxEvent.detail.pathInfo.requestPath);
});
// Update table selection indicator when a table is clicked
document.body.addEventListener('htmx:afterSettle', function (event) {
    const htmxEvent = event;
    // Get the URL to find which table was selected
    const url = new URL(window.location.href);
    const selectedTable = url.searchParams.get('table');
    if (selectedTable) {
        // Remove active state from all table links
        const allTableLinks = document.querySelectorAll('[hx-get*="/hx/development/db/table/"]');
        allTableLinks.forEach(link => {
            link.classList.remove('bg-gray-700', 'border-l-4', 'border-teal-500', 'pl-3');
        });
        // Add active state to the selected table
        const selectedLink = document.querySelector(`[hx-push-url*="table=${selectedTable}"]`);
        if (selectedLink) {
            selectedLink.classList.add('bg-gray-700', 'border-l-4', 'border-teal-500', 'pl-3');
        }
    }
});
// Handle HTMX errors
document.body.addEventListener('htmx:responseError', function (event) {
    const htmxEvent = event;
    console.error('HTMX Error:', htmxEvent.detail.xhr.status, htmxEvent.detail.xhr.statusText);
    // TODO: Show user-friendly error message
});
// Mobile menu toggle
console.log('site.ts loaded - starting mobile menu setup');
const mobileMenuBtn = document.getElementById('mobile-menu-btn');
const mobileMenu = document.getElementById('mobile-menu');
console.log('Mobile menu init - btn:', mobileMenuBtn, 'menu:', mobileMenu);
console.log('btn HTML:', mobileMenuBtn?.outerHTML);
console.log('menu HTML:', mobileMenu?.outerHTML);
if (mobileMenuBtn && mobileMenu) {
    console.log('Both elements found, adding event listener');
    mobileMenuBtn.addEventListener('click', function (e) {
        console.log('CLICK EVENT FIRED', e);
        e.preventDefault();
        console.log('Menu button clicked');
        console.log('Before toggle - hidden class:', mobileMenu.classList.contains('hidden'));
        mobileMenu.classList.toggle('hidden');
        console.log('After toggle - hidden class:', mobileMenu.classList.contains('hidden'));
        console.log('Menu classList after toggle:', mobileMenu.className);
    });
    console.log('Event listener added successfully');
    // Close menu when a link is clicked
    mobileMenu.querySelectorAll('a').forEach(link => {
        console.log('Adding click listener to link:', link.href);
        link.addEventListener('click', function () {
            console.log('Link clicked, hiding menu');
            mobileMenu.classList.add('hidden');
        });
    });
}
else {
    console.warn('Mobile menu elements not found - btn:', !!mobileMenuBtn, 'menu:', !!mobileMenu);
}
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
