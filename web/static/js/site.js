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

const submissionForm = document.getElementById('scouting-form');
const submissionHistoryList = document.getElementById('submission-history');
const submissionHistoryEmpty = document.getElementById('submission-history-empty');
const submissionHistoryKey = 'scoutingSubmissionHistory';

function getSelectedText(id) {
    const select = document.getElementById(id);
    if (!select) {
        return '';
    }
    const option = select.options[select.selectedIndex];
    return option ? option.textContent?.trim() ?? '' : '';
}

function loadSubmissionHistory() {
    const raw = localStorage.getItem(submissionHistoryKey);
    if (!raw) {
        return [];
    }
    try {
        const parsed = JSON.parse(raw);
        return Array.isArray(parsed) ? parsed : [];
    }
    catch (error) {
        console.warn('Invalid submission history payload', error);
        return [];
    }
}

function renderSubmissionHistory() {
    if (!submissionHistoryList || !submissionHistoryEmpty) {
        return;
    }
    const entries = loadSubmissionHistory();
    submissionHistoryList.innerHTML = '';

    if (!entries.length) {
        submissionHistoryEmpty.classList.remove('hidden');
        return;
    }

    submissionHistoryEmpty.classList.add('hidden');
    entries.forEach((entry) => {
        const item = document.createElement('li');
        item.className = 'rounded-lg border border-gray-700 bg-gray-900 px-3 py-2';

        const title = document.createElement('div');
        title.className = 'text-sm font-semibold text-gray-200';
        title.textContent = `${entry.matchType} - ${entry.team}`;

        const meta = document.createElement('div');
        meta.className = 'text-xs text-gray-400';
        meta.textContent = `${entry.event} | ${entry.alliance} ${entry.position} | ${entry.time}`;

        item.appendChild(title);
        item.appendChild(meta);
        submissionHistoryList.appendChild(item);
    });
}

if (submissionForm) {
    renderSubmissionHistory();

    submissionForm.addEventListener('submit', () => {
        const eventName = getSelectedText('event-id') || 'Event';
        const teamName = getSelectedText('team-id') || 'Team';
        const matchType = getSelectedText('match-type') || 'Match';
        const alliance = getSelectedText('alliance-color') || 'Alliance';
        const position = getSelectedText('alliance-position') || 'Pos';
        const time = new Date().toLocaleString();

        const entry = {
            event: eventName,
            team: teamName,
            matchType: matchType,
            alliance: alliance,
            position: position,
            time: time
        };

        const entries = loadSubmissionHistory();
        entries.unshift(entry);
        const trimmed = entries.slice(0, 5);
        localStorage.setItem(submissionHistoryKey, JSON.stringify(trimmed));
    });
}

const pickList = document.getElementById('pick-list');
const pickListKey = 'leadScoutPickList';
const pickListColorClasses = {
    red: ['bg-red-900', 'border-red-500', 'text-red-200'],
    yellow: ['bg-yellow-900', 'border-yellow-600', 'text-yellow-200'],
    teal: ['bg-teal-900', 'border-teal-500', 'text-teal-200']
};

function loadPickListState() {
    const raw = localStorage.getItem(pickListKey);
    if (!raw) {
        return {};
    }
    try {
        const parsed = JSON.parse(raw);
        return parsed && typeof parsed === 'object' ? parsed : {};
    }
    catch (error) {
        console.warn('Invalid pick list payload', error);
        return {};
    }
}

function savePickListState(state) {
    localStorage.setItem(pickListKey, JSON.stringify(state));
}

function clearPickListColors(element) {
    Object.values(pickListColorClasses).forEach((classes) => {
        element.classList.remove(...classes);
    });
}

function applyPickListEntry(element, entry) {
    clearPickListColors(element);
    element.classList.add('bg-gray-900', 'border-gray-700');

    if (entry.color && pickListColorClasses[entry.color]) {
        element.classList.add(...pickListColorClasses[entry.color]);
    }

    const label = element.querySelector('.pick-list-label');
    const status = element.querySelector('.pick-list-status');
    if (label) {
        label.classList.toggle('text-gray-200', !entry.crossed);
        label.classList.toggle('text-gray-500', !!entry.crossed);
        label.classList.toggle('italic', !!entry.crossed);
    }
    if (status) {
        status.textContent = entry.crossed ? '[X]' : '';
    }
}

if (pickList) {
    const state = loadPickListState();
    pickList.querySelectorAll('.pick-list-item').forEach((item) => {
        const teamNumber = item.dataset.teamNumber;
        if (!teamNumber) {
            return;
        }
        applyPickListEntry(item, state[teamNumber] || {});
    });

    pickList.addEventListener('click', (event) => {
        const target = event.target;
        if (!target || !target.dataset.action) {
            return;
        }

        const item = target.closest('.pick-list-item');
        if (!item) {
            return;
        }

        const teamNumber = item.dataset.teamNumber;
        if (!teamNumber) {
            return;
        }

        const entry = state[teamNumber] || {};
        if (target.dataset.action === 'pick') {
            const color = target.dataset.color;
            if (color) {
                entry.color = color;
            }
            entry.crossed = false;
        }

        if (target.dataset.action === 'cross') {
            entry.crossed = !entry.crossed;
            if (entry.crossed) {
                entry.color = undefined;
            }
        }

        state[teamNumber] = entry;
        savePickListState(state);
        applyPickListEntry(item, entry);
    });
}
            try {
                entries = JSON.parse(raw);
            }
            catch (error) {
                console.warn('Invalid submission history payload', error);
            }
        }
        entries.unshift(entry);
        const trimmed = entries.slice(0, 5);
        localStorage.setItem(submissionHistoryKey, JSON.stringify(trimmed));
    });
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
