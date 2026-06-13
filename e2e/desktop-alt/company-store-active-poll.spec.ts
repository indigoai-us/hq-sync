import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

/**
 * Regression — the company warm-cache must re-poll ONLY the company currently on
 * screen, not every known company.
 *
 * Context: the store warmed all N companies' 5 datasets at launch and then
 * re-fetched ALL of them on a 30s timer AND on every window focus — ~5 fetches ×
 * N companies of repeated waste (get_company_summary alone re-fetches the other
 * four), regardless of which company (if any) was open. Now a launch warm primes
 * everything once, but the background refresh follows the active slug
 * (setActiveCompany, driven by DesktopApp's activeCompany), so an idle Home/
 * Messages view polls nothing and an open company refreshes just itself.
 */
describe('company-store scopes the background poll to the active company', () => {
  const store = readRepoFile('src/desktop-alt/lib/company-store.svelte.ts');
  const app = readRepoFile('src/desktop-alt/DesktopApp.svelte');

  it('polls + focus-refreshes only the active slug, not all warmed slugs', () => {
    expect(store).toContain('export function setActiveCompany');
    expect(store).toContain('function refreshActive');
    expect(store).toContain('if (activeSlug)');
    expect(store).toContain('setInterval(refreshActive');
    expect(store).toContain("addEventListener('focus', refreshActive)");
    // the old "refresh every warmed company" loop must be gone from the poll path
    expect(store).not.toContain('setInterval(refreshAll');
    expect(store).not.toContain("addEventListener('focus', refreshAll)");
  });

  it('DesktopApp tells the store which company is on screen', () => {
    expect(app).toContain('setActiveCompany');
    expect(app).toContain('setActiveCompany(activeCompany?.slug ?? null)');
  });
});
