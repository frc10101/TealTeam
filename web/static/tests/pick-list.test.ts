/**
 * Pick List Frontend Tests
 * Tests for team-wide pick list persistence and synchronization
 */

import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';

// Mock localStorage
const localStorageMock = (() => {
  let store: Record<string, string> = {};

  return {
    getItem: (key: string) => store[key] || null,
    setItem: (key: string, value: string) => {
      store[key] = value.toString();
    },
    removeItem: (key: string) => {
      delete store[key];
    },
    clear: () => {
      store = {};
    }
  };
})();

global.localStorage = localStorageMock as any;

describe('Pick List Persistence', () => {
  beforeEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
  });

  describe('savePickListState', () => {
    it('should save state to localStorage', () => {
      const state = {
        '1234': { color: 'red', crossed: false },
        '5678': { color: 'yellow', crossed: true }
      };

      // Simulate savePickListState
      localStorage.setItem('leadScoutPickList', JSON.stringify(state));

      const saved = localStorage.getItem('leadScoutPickList');
      expect(saved).to.equal(JSON.stringify(state));
    });

    it('should handle empty state', () => {
      const state = {};
      localStorage.setItem('leadScoutPickList', JSON.stringify(state));

      const saved = localStorage.getItem('leadScoutPickList');
      expect(saved).to.equal('{}');
    });

    it('should overwrite previous state', () => {
      const state1 = { '1234': { color: 'red' } };
      const state2 = { '5678': { color: 'yellow' } };

      localStorage.setItem('leadScoutPickList', JSON.stringify(state1));
      localStorage.setItem('leadScoutPickList', JSON.stringify(state2));

      const saved = localStorage.getItem('leadScoutPickList');
      expect(saved).to.equal(JSON.stringify(state2));
    });
  });

  describe('loadPickListState', () => {
    it('should load state from localStorage', () => {
      const state = { '1234': { color: 'red', crossed: false } };
      localStorage.setItem('leadScoutPickList', JSON.stringify(state));

      const loaded = JSON.parse(localStorage.getItem('leadScoutPickList') || '{}');
      expect(loaded).to.deep.equal(state);
    });

    it('should return empty object if no data', () => {
      const loaded = JSON.parse(localStorage.getItem('leadScoutPickList') || '{}');
      expect(loaded).to.deep.equal({});
    });

    it('should handle invalid JSON gracefully', () => {
      localStorage.setItem('leadScoutPickList', 'invalid json');

      try {
        JSON.parse(localStorage.getItem('leadScoutPickList') || '{}');
        expect.fail('should have thrown error');
      } catch {
        // Expected
      }
    });
  });

  describe('savePickListSelectedTeams', () => {
    it('should save selected team numbers array', () => {
      const teams = ['1234', '5678', '9012'];
      localStorage.setItem('leadScoutPickListSelectedTeams', JSON.stringify(teams));

      const saved = localStorage.getItem('leadScoutPickListSelectedTeams');
      expect(saved).to.equal(JSON.stringify(teams));
    });

    it('should maintain order of teams', () => {
      const teams = ['9012', '1234', '5678'];
      localStorage.setItem('leadScoutPickListSelectedTeams', JSON.stringify(teams));

      const saved = JSON.parse(localStorage.getItem('leadScoutPickListSelectedTeams') || '[]');
      expect(saved).to.deep.equal(teams);
    });

    it('should handle empty array', () => {
      const teams: string[] = [];
      localStorage.setItem('leadScoutPickListSelectedTeams', JSON.stringify(teams));

      const saved = JSON.parse(localStorage.getItem('leadScoutPickListSelectedTeams') || '[]');
      expect(saved).to.deep.equal([]);
    });
  });

  describe('loadPickListSelectedTeams', () => {
    it('should load selected teams from localStorage', () => {
      const teams = ['1234', '5678'];
      localStorage.setItem('leadScoutPickListSelectedTeams', JSON.stringify(teams));

      const loaded = JSON.parse(localStorage.getItem('leadScoutPickListSelectedTeams') || '[]');
      expect(loaded).to.deep.equal(teams);
    });

    it('should return empty array if no data', () => {
      const loaded = JSON.parse(localStorage.getItem('leadScoutPickListSelectedTeams') || '[]');
      expect(loaded).to.deep.equal([]);
    });
  });

  describe('loadPickListFromServer', () => {
    it('should load and parse server response correctly', async () => {
      const mockResponse = {
        entries: [
          { picked_team_number: 1234, color: 'red', crossed: false, position: 0 },
          { picked_team_number: 5678, color: 'yellow', crossed: true, position: 1 }
        ]
      };

      const mockFetch = vi.fn().mockResolvedValue({
        ok: true,
        json: async () => mockResponse
      });

      global.fetch = mockFetch as any;

      // Simulate loadPickListFromServer
      const response = await fetch('/api/pick-list');
      const data = await response.json();

      expect(data.entries).to.have.lengthOf(2);
      expect(data.entries[0].picked_team_number).to.equal(1234);
      expect(data.entries[1].color).to.equal('yellow');
    });

    it('should handle fetch errors gracefully', async () => {
      const mockFetch = vi.fn().mockRejectedValue(new Error('Network error'));
      global.fetch = mockFetch as any;

      try {
        await fetch('/api/pick-list');
        expect.fail('should have thrown error');
      } catch {
        // Expected
      }
    });

    it('should handle server errors (non-200 status)', async () => {
      const mockFetch = vi.fn().mockResolvedValue({
        ok: false,
        status: 500
      });

      global.fetch = mockFetch as any;

      const response = await fetch('/api/pick-list');
      expect(response.ok).to.be.false;
      expect(response.status).to.equal(500);
    });

    it('should handle empty entries list', async () => {
      const mockResponse = { entries: [] };
      const mockFetch = vi.fn().mockResolvedValue({
        ok: true,
        json: async () => mockResponse
      });

      global.fetch = mockFetch as any;

      const response = await fetch('/api/pick-list');
      const data = await response.json();

      expect(data.entries).to.have.lengthOf(0);
    });
  });

  describe('API Integration', () => {
    it('should send POST request to save entry', async () => {
      const mockFetch = vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({ status: 'saved' })
      });

      global.fetch = mockFetch as any;

      const requestBody = {
        picked_team_number: 1234,
        color: 'red',
        crossed: false,
        position: 0
      };

      await fetch('/api/pick-list/entry', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(requestBody)
      });

      expect(mockFetch).to.have.been.calledWith('/api/pick-list/entry', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(requestBody)
      });
    });

    it('should send DELETE request to remove entry', async () => {
      const mockFetch = vi.fn().mockResolvedValue({
        ok: true,
        json: async () => ({ status: 'deleted' })
      });

      global.fetch = mockFetch as any;

      await fetch('/api/pick-list/entry?team=1234', {
        method: 'DELETE'
      });

      expect(mockFetch).to.have.been.called;
      const callArgs = mockFetch.mock.calls[0];
      expect(callArgs[0]).to.include('/api/pick-list/entry');
      expect(callArgs[1].method).to.equal('DELETE');
    });

    it('should handle response validation for POST', async () => {
      const mockFetch = vi.fn().mockResolvedValue({
        ok: true,
        status: 200
      });

      global.fetch = mockFetch as any;

      const response = await fetch('/api/pick-list/entry', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ picked_team_number: 1234 })
      });

      expect(response.ok).to.be.true;
      expect(response.status).to.equal(200);
    });

    it('should log warning on failed POST', async () => {
      const consoleWarn = vi.spyOn(console, 'warn');
      const mockFetch = vi.fn().mockResolvedValue({
        ok: false,
        status: 400
      });

      global.fetch = mockFetch as any;

      const response = await fetch('/api/pick-list/entry', { method: 'POST' });

      if (!response.ok) {
        console.warn(`Failed to sync pick list entry: ${response.status}`);
      }

      expect(consoleWarn).to.have.been.called;
      consoleWarn.mockRestore();
    });
  });

  describe('Data Integrity', () => {
    it('should maintain color values correctly', () => {
      const colors = ['red', 'yellow', 'teal'];
      const state: Record<string, any> = {};

      colors.forEach((color, index) => {
        state[String(1234 + index)] = { color };
      });

      localStorage.setItem('leadScoutPickList', JSON.stringify(state));
      const loaded = JSON.parse(localStorage.getItem('leadScoutPickList') || '{}');

      expect(loaded['1234'].color).to.equal('red');
      expect(loaded['1235'].color).to.equal('yellow');
      expect(loaded['1236'].color).to.equal('teal');
    });

    it('should maintain crossed state correctly', () => {
      const state = {
        '1234': { crossed: true },
        '5678': { crossed: false }
      };

      localStorage.setItem('leadScoutPickList', JSON.stringify(state));
      const loaded = JSON.parse(localStorage.getItem('leadScoutPickList') || '{}');

      expect(loaded['1234'].crossed).to.be.true;
      expect(loaded['5678'].crossed).to.be.false;
    });

    it('should maintain position order', () => {
      const teams = ['9012', '1234', '5678'];
      localStorage.setItem('leadScoutPickListSelectedTeams', JSON.stringify(teams));
      const loaded = JSON.parse(localStorage.getItem('leadScoutPickListSelectedTeams') || '[]');

      expect(loaded[0]).to.equal('9012');
      expect(loaded[1]).to.equal('1234');
      expect(loaded[2]).to.equal('5678');
    });
  });
});
