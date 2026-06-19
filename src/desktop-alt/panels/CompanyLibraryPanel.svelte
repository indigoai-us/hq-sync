<script lang="ts">
  /**
   * CompanyLibraryPanel — the per-company Library tab. Lists a single company's
   * private workers + company-scoped skills via get_library_company, reusing the
   * shared LibraryBrowser (Workers|Skills toggle + text filter + detail).
   *
   * Load convention mirrors the other company panels: slug-keyed $effect with a
   * cancel flag so switching companies fast can't paint stale data. A second
   * $effect subscribes to window focus / sync:complete and bumps `refreshNonce`,
   * so a worker created in another tool surfaces without remounting the panel.
   */
  import { loadLibraryCompany, type LibraryItems } from '../lib/library';
  import { subscribeLibraryRefresh } from '../lib/library-refresh';
  import type { UnlistenFn } from '@tauri-apps/api/event';
  import LibraryBrowser from '../components/LibraryBrowser.svelte';

  interface Props {
    /** The company/workspace slug this library is scoped to. */
    slug: string;
  }

  let { slug }: Props = $props();

  let items = $state<LibraryItems>({ workers: [], skills: [] });
  let loading = $state(true);
  let error = $state<string | null>(null);
  /** Bumped by the focus / sync:complete refresh subscription to re-fetch. */
  let refreshNonce = $state(0);

  $effect(() => {
    const activeSlug = slug;
    // Re-run whenever the refresh subscription bumps the nonce.
    refreshNonce;
    items = { workers: [], skills: [] };
    error = null;

    if (!activeSlug) {
      loading = false;
      return;
    }

    loading = true;
    let cancelled = false;

    void (async () => {
      try {
        const result = await loadLibraryCompany(activeSlug);
        if (!cancelled) items = result;
      } catch (err) {
        console.error('loadLibraryCompany failed:', err);
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

  // Re-fetch on window focus / sync:complete so a worker created elsewhere
  // appears without remounting the panel. Wired once.
  $effect(() => {
    let unlisten: UnlistenFn | undefined;
    let disposed = false;

    void subscribeLibraryRefresh(() => {
      refreshNonce += 1;
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  });
</script>

<section class="company-library" aria-label="Library" data-testid="company-library-panel">
  {#if !loading && !error && items.workers.length === 0 && items.skills.length === 0}
    <div class="empty-state">
      No company-specific workers or skills yet. Shared ones live in the top-level Library (⌘3).
    </div>
  {:else}
    <LibraryBrowser {items} {loading} {error} />
  {/if}
</section>

<style>
  .company-library {
    min-width: 0;
  }

  .empty-state {
    padding: var(--space-4);
    border: 1px dashed var(--border);
    border-radius: var(--radius-md);
    color: var(--muted-3);
    font-size: var(--text-base);
    text-align: center;
  }
</style>
