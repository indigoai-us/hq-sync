<script lang="ts">
  /**
   * LibraryPage — the root/shared Library surface (a top-level desktop-alt
   * destination, ⌘3). Lists every shared/public worker plus root + personal
   * skills, with a Workers|Skills toggle, a text filter, and a detail slide-over.
   *
   * Data is loaded once from the local FS via get_library_root; the shared
   * LibraryBrowser owns the filter/toggle/list/detail UI.
   */
  import { loadLibraryRoot, type LibraryItems } from '../lib/library';
  import type { LibraryTab } from '../route';
  import LibraryBrowser from '../components/LibraryBrowser.svelte';

  interface Props {
    /** Which library surface to show — driven by the sidebar route. */
    tab?: LibraryTab;
  }

  let { tab = 'skills' }: Props = $props();

  let items = $state<LibraryItems>({ workers: [], skills: [] });
  let loading = $state(true);
  let error = $state<string | null>(null);

  const HEADINGS: Record<LibraryTab, string> = {
    skills: 'Skills',
    workers: 'Workers',
    installed: 'Installed',
    marketplace: 'Marketplace',
    profile: 'Profile',
  };
  const heading = $derived(HEADINGS[tab]);

  const subtitle = $derived(
    tab === 'skills'
      ? `${items.skills.length} ${items.skills.length === 1 ? 'skill' : 'skills'} available to you`
      : tab === 'workers'
        ? `${items.workers.length} ${items.workers.length === 1 ? 'worker' : 'workers'} available to you`
        : tab === 'installed'
          ? 'Marketplace packs installed in your HQ'
          : tab === 'marketplace'
            ? 'Discover and install skills and workers'
            : 'Your HQ profile and published work',
  );

  $effect(() => {
    loading = true;
    error = null;
    let cancelled = false;

    void (async () => {
      try {
        const result = await loadLibraryRoot();
        if (!cancelled) items = result;
      } catch (err) {
        console.error('loadLibraryRoot failed:', err);
        if (!cancelled) {
          error = 'Library unavailable. Try again after a sync.';
          items = { workers: [], skills: [] };
        }
      } finally {
        if (!cancelled) loading = false;
      }
    })();

    return () => {
      cancelled = true;
    };
  });
</script>

<section class="library-page" aria-labelledby="library-page-title">
  <header class="page-header">
    <h1 id="library-page-title">{heading}</h1>
    <p>{subtitle}</p>
  </header>

  <LibraryBrowser {items} {loading} {error} forcedFilter={tab} />
</section>

<style>
  .library-page {
    display: grid;
    gap: 18px;
  }

  .page-header h1 {
    margin: 0;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 29px;
  }

  .page-header p {
    margin: 5px 0 0;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 18px;
  }
</style>
