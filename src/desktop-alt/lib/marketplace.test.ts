import { describe, expect, it, vi } from 'vitest';

// `marketplace.ts` imports `invoke` at module load. We never exercise the real
// IPC here (these are pure-logic tests), so a no-op mock keeps the import happy.
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));

import {
  companyInstallTargets,
  filterListings,
  listingHaystack,
  type MarketplaceListing,
} from './marketplace';
import type { Workspace } from '../../lib/workspaces';

const listing = (overrides: Partial<MarketplaceListing> = {}): MarketplaceListing => ({
  id: 'lst_1',
  type: 'skill',
  name: 'Impeccable',
  slug: 'impeccable',
  version: '1.2.0',
  author: 'corey',
  summary: 'Improve a UI',
  contributes: '1 skill',
  createdAt: '2026-06-01T00:00:00Z',
  ...overrides,
});

const workspace = (overrides: Partial<Workspace> = {}): Workspace => ({
  slug: 'indigo',
  displayName: 'Indigo',
  kind: 'company',
  state: 'synced',
  cloudUid: 'cmp_1',
  bucketName: 'hq-vault-cmp-1',
  hasLocalFolder: true,
  localPath: '/Users/x/HQ/companies/indigo',
  membershipStatus: 'active',
  role: 'admin',
  lastSyncedAt: null,
  brokenReason: null,
  ...overrides,
});

describe('filterListings', () => {
  it('matches on name/slug/author/summary/contributes', () => {
    const items = [listing(), listing({ id: 'lst_2', name: 'Architect', slug: 'architect', author: 'jane' })];
    expect(filterListings(items, 'jane')).toHaveLength(1);
    expect(filterListings(items, 'impeccable')).toHaveLength(1);
    expect(filterListings(items, '')).toHaveLength(2);
  });

  it('builds a lowercased haystack', () => {
    expect(listingHaystack(listing({ name: 'LOUD' }))).toContain('loud');
  });
});

describe('companyInstallTargets — scope picker (tenant-isolation, default-deny)', () => {
  it('always includes an enabled Personal target first', () => {
    const targets = companyInstallTargets([]);
    expect(targets[0]).toEqual({ scope: { kind: 'personal' }, label: 'Personal', enabled: true });
  });

  it('enables a company the user is ADMIN of (active membership)', () => {
    const targets = companyInstallTargets([workspace({ role: 'admin', membershipStatus: 'active' })]);
    const co = targets.find((t) => t.scope.kind === 'company');
    expect(co).toBeDefined();
    expect(co!.enabled).toBe(true);
    expect(co!.scope).toEqual({ kind: 'company', slug: 'indigo' });
    expect(co!.label).toBe('Indigo');
  });

  it('enables a company the user OWNS', () => {
    const targets = companyInstallTargets([workspace({ role: 'owner' })]);
    expect(targets.find((t) => t.scope.kind === 'company')!.enabled).toBe(true);
  });

  it('DISABLES a company for a non-admin (member) with a clear reason', () => {
    const targets = companyInstallTargets([workspace({ role: 'member' })]);
    const co = targets.find((t) => t.scope.kind === 'company')!;
    expect(co.enabled).toBe(false);
    expect(co.reason).toMatch(/company-admin/i);
  });

  it('DISABLES a company with unknown/null role (default-deny)', () => {
    const targets = companyInstallTargets([workspace({ role: null })]);
    const co = targets.find((t) => t.scope.kind === 'company')!;
    expect(co.enabled).toBe(false);
    expect(co.reason).toMatch(/unknown/i);
  });

  it('DISABLES an admin whose membership is not active (e.g. pending)', () => {
    const targets = companyInstallTargets([
      workspace({ role: 'admin', membershipStatus: 'pending' }),
    ]);
    const co = targets.find((t) => t.scope.kind === 'company')!;
    expect(co.enabled).toBe(false);
    expect(co.reason).toMatch(/pending/i);
  });

  it('excludes the personal pseudo-company from the company list', () => {
    const targets = companyInstallTargets([
      workspace({ slug: 'personal', kind: 'personal', displayName: 'Personal' }),
    ]);
    // Only the synthesized Personal target — no duplicate company row.
    expect(targets).toHaveLength(1);
    expect(targets[0].scope).toEqual({ kind: 'personal' });
  });

  it('orders admin-enabled companies before disabled ones', () => {
    const targets = companyInstallTargets([
      workspace({ slug: 'acme', displayName: 'Acme', role: 'member' }),
      workspace({ slug: 'indigo', displayName: 'Indigo', role: 'admin' }),
    ]);
    const companies = targets.filter((t) => t.scope.kind === 'company');
    expect(companies[0].enabled).toBe(true);
    expect(companies[0].label).toBe('Indigo');
    expect(companies[1].enabled).toBe(false);
  });
});
