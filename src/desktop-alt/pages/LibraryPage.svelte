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
  import LibraryBrowser from '../components/LibraryBrowser.svelte';

  let items = $state<LibraryItems>({ workers: [], skills: [] });
  let loading = $state(true);
  let error = $state<string | null>(null);

  const subtitle = $derived(
    `${items.workers.length} ${items.workers.length === 1 ? 'worker' : 'workers'} · ${items.skills.length} ${items.skills.length === 1 ? 'skill' : 'skills'} available to you`,
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
    <h1 id="library-page-title">Library</h1>
    <p>{subtitle}</p>
  </header>

  <LibraryBrowser {items} {loading} {error} />
</section>

<style>
  .library-page {
    display: grid;
    gap: 18px;
  }

  .page-header h1 {
    margin: 0;
    color: var(--fg);
    font-size: 22px;
    font-weight: 680;
    line-height: 29px;
  }

  .page-header p {
    margin: 5px 0 0;
    color: var(--muted);
    font-size: 13px;
    line-height: 18px;
  }
</style>
