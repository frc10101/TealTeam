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

// Update table selection indicator when a table is clicked
document.body.addEventListener('htmx:afterSettle', function(event: Event) {
  const htmxEvent = event as HtmxResponseErrorEvent;
  
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
document.body.addEventListener('htmx:responseError', function(event: Event) {
  const htmxEvent = event as HtmxResponseErrorEvent;
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
  mobileMenuBtn.addEventListener('click', function(e: Event) {
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
    link.addEventListener('click', function() {
      console.log('Link clicked, hiding menu');
      mobileMenu.classList.add('hidden');
    });
  });
} else {
  console.warn('Mobile menu elements not found - btn:', !!mobileMenuBtn, 'menu:', !!mobileMenu);
}

type SubmissionHistoryEntry = {
  event: string;
  team: string;
  alliance: string;
  time: string;
};

const submissionForm = document.getElementById('scouting-form') as HTMLFormElement | null;
const submissionHistoryList = document.getElementById('submission-history');
const submissionHistoryEmpty = document.getElementById('submission-history-empty');
const submissionHistoryKey = 'scoutingSubmissionHistory';

function getSelectedText(id: string): string {
  const select = document.getElementById(id) as HTMLSelectElement | null;
  if (!select) {
    return '';
  }
  const option = select.options[select.selectedIndex];
  return option ? option.textContent?.trim() ?? '' : '';
}

function loadSubmissionHistory(): SubmissionHistoryEntry[] {
  const raw = localStorage.getItem(submissionHistoryKey);
  if (!raw) {
    return [];
  }
  try {
    const parsed = JSON.parse(raw) as SubmissionHistoryEntry[];
    return Array.isArray(parsed) ? parsed : [];
  } catch (error) {
    console.warn('Invalid submission history payload', error);
    return [];
  }
}

function renderSubmissionHistory(): void {
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
    title.textContent = `${entry.team}`;

    const meta = document.createElement('div');
    meta.className = 'text-xs text-gray-400';
    meta.textContent = `${entry.event} | ${entry.alliance} | ${entry.time}`;

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
    const alliance = getSelectedText('alliance-color') || 'Alliance';
    const time = new Date().toLocaleString();

    const entry: SubmissionHistoryEntry = {
      event: eventName,
      team: teamName,
      alliance: alliance,
      time: time
    };

    const entries = loadSubmissionHistory();
    entries.unshift(entry);
    const trimmed = entries.slice(0, 5);
    localStorage.setItem(submissionHistoryKey, JSON.stringify(trimmed));
  });
}

type PickListEntry = {
  color?: 'red' | 'yellow' | 'teal';
  crossed?: boolean;
};

type PickListTeamMeta = {
  teamNumber: string;
  teamName: string;
  rank?: number;
};

const pickList = document.getElementById('pick-list');
const pickListKey = 'leadScoutPickList';
const pickListSelectedKey = 'leadScoutPickListSelectedTeams';
const pickListColorClasses: Record<string, string[]> = {
  red: ['bg-red-900', 'border-red-500', 'text-red-200'],
  yellow: ['bg-yellow-900', 'border-yellow-600', 'text-yellow-200'],
  teal: ['bg-teal-900', 'border-teal-500', 'text-teal-200']
};

// Global state for pick list (scoped to module level)
let state: Record<string, PickListEntry> = {};
let selectedTeamNumbers: string[] = [];
let allTeams: Map<string, PickListTeamMeta> = new Map();

// Placeholder for renderPickList - will be defined in pickList handler
let renderPickList: () => void = () => {};

function loadPickListState(): Record<string, PickListEntry> {
  const raw = localStorage.getItem(pickListKey);
  if (!raw) {
    return {};
  }
  try {
    const parsed = JSON.parse(raw) as Record<string, PickListEntry>;
    return parsed && typeof parsed === 'object' ? parsed : {};
  } catch (error) {
    console.warn('Invalid pick list payload', error);
    return {};
  }
}

function savePickListState(newState: Record<string, PickListEntry>): void {
  state = newState;
  // Save to localStorage immediately for UI responsiveness
  localStorage.setItem(pickListKey, JSON.stringify(state));
  
  // Sync individual entries to backend
  Object.entries(state).forEach(([teamNumber, entry]) => {
    const position = selectedTeamNumbers.indexOf(teamNumber);
    fetch('/api/pick-list/entry', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json'
      },
      body: JSON.stringify({
        picked_team_number: parseInt(teamNumber),
        color: entry.color || null,
        crossed: entry.crossed || false,
        position: position >= 0 ? position : 0
      })
    }).then((response) => {
      if (!response.ok) {
        console.warn(`Failed to sync pick list entry for team ${teamNumber}: ${response.status}`);
      }
    }).catch((error) => console.warn('Failed to sync pick list entry', error));
  });
}

function loadPickListSelectedTeams(): string[] {
  const raw = localStorage.getItem(pickListSelectedKey);
  if (!raw) {
    return [];
  }
  try {
    const parsed = JSON.parse(raw) as string[];
    return Array.isArray(parsed) ? parsed : [];
  } catch (error) {
    console.warn('Invalid pick list team selection payload', error);
    return [];
  }
}

function savePickListSelectedTeams(teamNumbers: string[]): void {
  selectedTeamNumbers = teamNumbers;
  // Save to localStorage immediately
  localStorage.setItem(pickListSelectedKey, JSON.stringify(selectedTeamNumbers));
  
  // Sync each team as an entry to backend
  selectedTeamNumbers.forEach((teamNumber, position) => {
    const entry = state[teamNumber] || {};
    fetch('/api/pick-list/entry', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json'
      },
      body: JSON.stringify({
        picked_team_number: parseInt(teamNumber),
        color: entry.color || null,
        crossed: entry.crossed || false,
        position: position
      })
    }).then((response) => {
      if (!response.ok) {
        console.warn(`Failed to sync pick list for team ${teamNumber}: ${response.status}`);
      }
    }).catch((error) => console.warn('Failed to sync pick list entry', error));
  });
}

async function loadPickListFromServer(): Promise<void> {
  try {
    const response = await fetch('/api/pick-list');
    if (!response.ok) {
      console.warn('Failed to load pick list from server, using cached data');
      // Fall back to rendering from localStorage
      state = loadPickListState();
      selectedTeamNumbers = loadPickListSelectedTeams().filter((teamNumber) => allTeams.has(teamNumber));
      renderPickList();
      return;
    }
    
    const data = await response.json() as { entries: Array<{ picked_team_number: number; color?: string; crossed?: boolean; position: number }> };
    if (!data.entries || data.entries.length === 0) {
      // Empty list from server is fine
      state = {};
      selectedTeamNumbers = [];
      renderPickList();
      return;
    }

    // Rebuild state and selected teams from server data
    const newState: Record<string, PickListEntry> = {};
    const newSelectedTeamNumbers: string[] = [];
    
    data.entries.forEach((entry) => {
      const teamNumberStr = String(entry.picked_team_number);
      newSelectedTeamNumbers[entry.position] = teamNumberStr;
      newState[teamNumberStr] = {
        color: entry.color as 'red' | 'yellow' | 'teal' | undefined,
        crossed: entry.crossed || false
      };
    });
    
    selectedTeamNumbers = newSelectedTeamNumbers.filter((t) => t !== undefined);
    state = newState;
    localStorage.setItem(pickListKey, JSON.stringify(state));
    localStorage.setItem(pickListSelectedKey, JSON.stringify(selectedTeamNumbers));
    renderPickList();
  } catch (error) {
    console.warn('Failed to load pick list from server:', error);
    // Fall back to localStorage
    state = loadPickListState();
    selectedTeamNumbers = loadPickListSelectedTeams().filter((teamNumber) => allTeams.has(teamNumber));
    renderPickList();
  }
}

function clearPickListColors(element: HTMLElement): void {
  Object.values(pickListColorClasses).forEach((classes) => {
    element.classList.remove(...classes);
  });
}

function applyPickListEntry(element: HTMLElement, entry: PickListEntry): void {
  clearPickListColors(element);
  element.classList.add('bg-gray-900', 'border-gray-700');

  if (entry.color && pickListColorClasses[entry.color]) {
    element.classList.add(...pickListColorClasses[entry.color]);
  }

  const label = element.querySelector('.pick-list-label') as HTMLElement | null;
  const status = element.querySelector('.pick-list-status') as HTMLElement | null;
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
  const pickListElement = pickList;
  const addToggle = document.getElementById('pick-list-add-toggle') as HTMLButtonElement | null;
  const addPanel = document.getElementById('pick-list-add-panel') as HTMLElement | null;
  const teamSelect = document.getElementById('pick-list-team-select') as HTMLSelectElement | null;
  const addTeamButton = document.getElementById('pick-list-add-team') as HTMLButtonElement | null;

  state = loadPickListState();

  if (teamSelect) {
    Array.from(teamSelect.options).forEach((option) => {
      if (!option.value) {
        return;
      }
      const rankRaw = option.dataset.rank;
      const rank = rankRaw ? Number.parseInt(rankRaw, 10) : undefined;
      allTeams.set(option.value, {
        teamNumber: option.value,
        teamName: option.dataset.teamName || '',
        rank: Number.isFinite(rank) ? rank : undefined
      });
    });
  }

  selectedTeamNumbers = loadPickListSelectedTeams().filter((teamNumber) => allTeams.has(teamNumber));

  function createPickListItem(team: PickListTeamMeta): HTMLElement {
    const item = document.createElement('li');
    item.className = 'pick-list-item rounded-lg border border-gray-700 bg-gray-900 px-3 py-2 cursor-move transition-opacity duration-200';
    item.dataset.teamNumber = team.teamNumber;
    item.draggable = true;

    const rankLabel = typeof team.rank === 'number' ? `#${team.rank}` : '-';
    const teamName = team.teamName ? ` - ${team.teamName}` : '';
    item.innerHTML = `
      <div class="space-y-3">
        <div class="flex items-start justify-between gap-2">
          <div>
            <div class="pick-list-label text-sm font-semibold text-gray-200">
              <a href="/teams?team=${team.teamNumber}" class="text-inherit hover:text-teal-200">Team ${team.teamNumber}${teamName}</a>
            </div>
            <div class="text-xs text-gray-500">Rank: ${rankLabel}</div>
          </div>
          <div class="flex items-center gap-2">
            <span class="pick-list-position rounded-full border border-teal-700 bg-teal-900/40 px-2 py-0.5 text-xs font-semibold text-teal-200"></span>
            <span class="pick-list-status text-xs text-gray-500"></span>
          </div>
        </div>
        <div class="border-t border-gray-700"></div>
        <div class="flex flex-wrap gap-1">
          <button type="button" class="rounded-full bg-red-900 px-2 py-1 text-xs text-red-200" data-action="pick" data-color="red">Red</button>
          <button type="button" class="rounded-full bg-yellow-900 px-2 py-1 text-xs text-yellow-200" data-action="pick" data-color="yellow">Yellow</button>
          <button type="button" class="rounded-full bg-teal-900 px-2 py-1 text-xs text-teal-200" data-action="pick" data-color="teal">Teal</button>
          <button type="button" class="rounded-full border border-gray-700 px-2 py-1 text-xs text-gray-400" data-action="cross">Cross</button>
          <button type="button" class="rounded-full border border-gray-700 px-2 py-1 text-xs text-gray-400" data-action="remove">Remove</button>
        </div>
      </div>
    `;

    return item;
  }

  function updatePickListPositions(): void {
    Array.from(pickListElement.querySelectorAll<HTMLElement>('.pick-list-item')).forEach((item, index) => {
      const position = item.querySelector('.pick-list-position') as HTMLElement | null;
      if (position) {
        position.textContent = String(index + 1);
      }
    });
  }

  // Assign to module-level variable so other functions can use it
  renderPickList = (): void => {
    pickListElement.innerHTML = '';
    selectedTeamNumbers.forEach((teamNumber) => {
      const team = allTeams.get(teamNumber);
      if (!team) {
        return;
      }
      const item = createPickListItem(team);
      pickListElement.appendChild(item);
      applyPickListEntry(item, state[teamNumber] || {});
    });
    updatePickListPositions();
  };

  function setPickerVisibility(visible: boolean): void {
    if (!addPanel || !addToggle) {
      return;
    }
    addPanel.classList.toggle('hidden', !visible);
    addPanel.classList.toggle('flex', visible);
    addToggle.textContent = visible ? 'Cancel' : '+ Add Team';
  }

  // Load pick list from server first, falling back to localStorage
  loadPickListFromServer();

  if (addToggle && addPanel) {
    addToggle.addEventListener('click', () => {
      const isHidden = addPanel.classList.contains('hidden');
      setPickerVisibility(isHidden);
    });
  }

  if (addTeamButton && teamSelect) {
    addTeamButton.addEventListener('click', () => {
      const teamNumber = teamSelect.value;
      if (!teamNumber || !allTeams.has(teamNumber)) {
        return;
      }
      if (selectedTeamNumbers.includes(teamNumber)) {
        return;
      }
      selectedTeamNumbers.push(teamNumber);
      savePickListSelectedTeams(selectedTeamNumbers);
      renderPickList();
      teamSelect.value = '';
      setPickerVisibility(false);
    });
  }

  pickListElement.addEventListener('click', (event) => {
    const target = event.target as HTMLElement;
    if (!target || !target.dataset.action) {
      return;
    }

    const item = target.closest('.pick-list-item') as HTMLElement | null;
    if (!item) {
      return;
    }

    const teamNumber = item.dataset.teamNumber;
    if (!teamNumber) {
      return;
    }

    if (target.dataset.action === 'remove') {
      selectedTeamNumbers = selectedTeamNumbers.filter((value) => value !== teamNumber);
      
      // Delete from server
      fetch(`/api/pick-list/entry?team=${teamNumber}`, {
        method: 'DELETE'
      }).catch((error) => console.warn('Failed to delete pick list entry', error));
      
      // Delete from local state
      delete state[teamNumber];
      
      savePickListSelectedTeams(selectedTeamNumbers);
      renderPickList();
      return;
    }

    const entry = state[teamNumber] || {};
    if (target.dataset.action === 'pick') {
      const color = target.dataset.color as PickListEntry['color'];
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

  // Drag and drop functionality
  let draggedItem: HTMLElement | null = null;

  pickListElement.addEventListener('dragstart', (event) => {
    const target = event.target as HTMLElement;
    if (target.classList.contains('pick-list-item')) {
      draggedItem = target;
      target.style.opacity = '0.4';
    }
  });

  pickListElement.addEventListener('dragend', (event) => {
    const target = event.target as HTMLElement;
    if (target.classList.contains('pick-list-item')) {
      target.style.opacity = '1';
    }
  });

  pickListElement.addEventListener('dragover', (event) => {
    event.preventDefault();
  });

  pickListElement.addEventListener('drop', (event) => {
    event.preventDefault();
    const target = event.target as HTMLElement;
    const dropTarget = target.classList.contains('pick-list-item')
      ? target
      : target.closest('.pick-list-item') as HTMLElement | null;

    if (draggedItem && dropTarget && draggedItem !== dropTarget) {
      const allItems = Array.from(pickListElement.querySelectorAll('.pick-list-item'));
      const draggedIndex = allItems.indexOf(draggedItem);
      const dropIndex = allItems.indexOf(dropTarget);

      if (draggedIndex < dropIndex) {
        dropTarget.parentNode?.insertBefore(draggedItem, dropTarget.nextSibling);
      } else {
        dropTarget.parentNode?.insertBefore(draggedItem, dropTarget);
      }

      selectedTeamNumbers = Array.from(pickListElement.querySelectorAll<HTMLElement>('.pick-list-item'))
        .map((item) => item.dataset.teamNumber || '')
        .filter((teamNumber) => teamNumber !== '');
      savePickListSelectedTeams(selectedTeamNumbers);
      updatePickListPositions();
    }
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
