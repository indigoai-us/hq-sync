import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

describe('desktop-alt V4 library and marketplace family (US-014)', () => {
  const libraryPage = readRepoFile('src/desktop-alt/pages/LibraryPage.svelte');
  const libraryBrowser = readRepoFile('src/desktop-alt/components/LibraryBrowser.svelte');
  const marketplace = readRepoFile('src/desktop-alt/panels/MarketplacePanel.svelte');
  const profile = readRepoFile('src/desktop-alt/panels/ProfilePanel.svelte');
  const moderation = readRepoFile('src/desktop-alt/panels/ModerationPanel.svelte');

  it('library renders a card-grid browser with a detail panel', () => {
    expect(libraryPage).toContain('<LibraryBrowser {items} {loading} {error} forcedFilter={tab} />');
    expect(libraryBrowser).toContain('card grid');
    expect(libraryBrowser).toContain('detail slide-over');
    expect(libraryBrowser).toContain('{ id: \'installed\', label: \'Installed\' }');
    expect(libraryBrowser).toContain('{ id: \'marketplace\', label: \'Marketplace\' }');
  });

  it('marketplace has listings, install/installed states, README preview, and YOUR LISTINGS', () => {
    expect(marketplace).toContain('data-testid="marketplace-card"');
    expect(marketplace).toContain('data-testid="marketplace-install-button"');
    expect(marketplace).toContain('Installed.');
    expect(marketplace).toContain('data-testid="marketplace-readme-preview"');
    expect(marketplace).toContain('README preview');
    expect(marketplace).toContain('data-testid="marketplace-your-listings"');
    expect(marketplace).toContain('YOUR LISTINGS');
  });

  it('profile includes claim/edit public preview and creator request-access variant lives in moderation', () => {
    expect(profile).toContain('claimCreatorHandle');
    expect(profile).toContain('data-testid="profile-preview"');
    expect(profile).toContain('data-testid="profile-preview-listing"');
    expect(moderation).toContain('Creator-access requests');
    expect(moderation).toContain('data-testid="moderation-request-row"');
    expect(moderation).toContain('data-testid="moderation-request-approve"');
    expect(moderation).toContain('data-testid="moderation-request-deny"');
  });

  it('admin moderation queue remains gated and review actions are present', () => {
    expect(moderation).toContain("invoke<boolean>('desktop_alt_is_admin')");
    expect(moderation).toContain('data-testid="moderation-locked"');
    expect(moderation).toContain('data-testid="moderation-queue-section"');
    expect(moderation).toContain('data-testid="moderation-approve"');
    expect(moderation).toContain('data-testid="moderation-reject"');
  });
});
