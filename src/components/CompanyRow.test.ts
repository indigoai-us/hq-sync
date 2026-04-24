// Vitest + @testing-library/svelte — covers AC8:
//   - render state per badge (aws / local / both)
//   - click dispatch for each source
//   - disabled-during-promotion state
//   - error-state retry click
//
// `invoke` is mocked globally in `src/test-setup.ts`. Per-test we reset
// the mock and the store so tests can't leak state into each other.
import { render, screen, fireEvent } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import CompanyRow from './CompanyRow.svelte';
import { companiesState, type CompanyInfo } from '../lib/stores';

function mkCompany(overrides: Partial<CompanyInfo> = {}): CompanyInfo {
  return {
    slug: 'indigo',
    name: 'Indigo',
    uid: 'co-abc123',
    source: 'both',
    ...overrides,
  };
}

describe('CompanyRow', () => {
  beforeEach(() => {
    companiesState.reset();
  });

  afterEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockImplementation(async () => undefined);
  });

  describe('badge rendering', () => {
    it('shows AWS badge for source=aws', () => {
      render(CompanyRow, { company: mkCompany({ source: 'aws', uid: null }) });
      expect(screen.getByText('AWS')).toBeTruthy();
    });

    it('shows Local badge for source=local', () => {
      render(CompanyRow, { company: mkCompany({ source: 'local', uid: null }) });
      expect(screen.getByText('Local')).toBeTruthy();
    });

    it('shows Synced badge for source=both', () => {
      render(CompanyRow, { company: mkCompany({ source: 'both' }) });
      expect(screen.getByText('Synced')).toBeTruthy();
    });

    it('renders the company name in bold', () => {
      render(CompanyRow, { company: mkCompany({ name: 'Acme Corp' }) });
      expect(screen.getByText('Acme Corp')).toBeTruthy();
    });

    it('renders "never" when lastSyncedAt is missing', () => {
      render(CompanyRow, { company: mkCompany() });
      expect(screen.getByText('never')).toBeTruthy();
    });
  });

  describe('click dispatch', () => {
    it('invokes start_sync when source=aws', async () => {
      render(CompanyRow, { company: mkCompany({ source: 'aws', uid: null }) });
      await fireEvent.click(screen.getByRole('button', { name: /sync/i }));
      expect(invoke).toHaveBeenCalledWith('start_sync');
    });

    it('invokes start_sync when source=both', async () => {
      render(CompanyRow, { company: mkCompany({ source: 'both' }) });
      await fireEvent.click(screen.getByRole('button', { name: /sync/i }));
      expect(invoke).toHaveBeenCalledWith('start_sync');
    });

    it('invokes promote_company when source=local', async () => {
      render(CompanyRow, {
        company: mkCompany({ slug: 'acme', source: 'local', uid: null }),
      });
      await fireEvent.click(screen.getByRole('button', { name: /sync/i }));
      expect(invoke).toHaveBeenCalledWith('promote_company', { slug: 'acme' });
    });
  });

  describe('disabled-during-promotion state', () => {
    it('disables the button and shows the spinner while promoting', async () => {
      const company = mkCompany({ slug: 'acme', source: 'local', uid: null });
      render(CompanyRow, { company });

      // Simulate promote:start landing in the store.
      companiesState.startPromote('acme');

      // Give Svelte the microtask it needs to reflect the store change.
      await Promise.resolve();
      await Promise.resolve();

      const button = screen.getByRole('button', {
        name: /promoting/i,
      }) as HTMLButtonElement;
      expect(button.disabled).toBe(true);
      expect(screen.getByTestId('row-spinner')).toBeTruthy();
    });

    it('click is a no-op while already promoting', async () => {
      const company = mkCompany({ slug: 'acme', source: 'local', uid: null });
      render(CompanyRow, { company });

      companiesState.startPromote('acme');
      await Promise.resolve();
      await Promise.resolve();

      // Browser won't dispatch click on a disabled button, but we also
      // guard in `handleSync`; just confirm invoke wasn't called.
      const button = screen.getByRole('button', { name: /promoting/i });
      await fireEvent.click(button);
      expect(invoke).not.toHaveBeenCalled();
    });
  });

  describe('error state + retry', () => {
    it('renders the error message when setPromoteError fired', async () => {
      const company = mkCompany({ slug: 'acme', source: 'local', uid: null });
      render(CompanyRow, { company });

      companiesState.setPromoteError('acme', 'runner spawn failed');
      await Promise.resolve();
      await Promise.resolve();

      expect(screen.getByText('runner spawn failed')).toBeTruthy();
      expect(screen.getByRole('button', { name: /retry/i })).toBeTruthy();
    });

    it('clicking Retry clears the error and reinvokes promote_company', async () => {
      const company = mkCompany({ slug: 'acme', source: 'local', uid: null });
      render(CompanyRow, { company });

      companiesState.setPromoteError('acme', 'runner spawn failed');
      await Promise.resolve();
      await Promise.resolve();

      await fireEvent.click(screen.getByRole('button', { name: /retry/i }));
      expect(invoke).toHaveBeenCalledWith('promote_company', { slug: 'acme' });
    });
  });
});
