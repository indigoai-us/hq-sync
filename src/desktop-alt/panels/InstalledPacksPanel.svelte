<script lang="ts">
  /**
   * InstalledPacksPanel — the desktop-alt **Installed** tab body (US-009).
   *
   * This is the unified home for *installed* HQ packs. It absorbs the function
   * of the old standalone Packages window (`src/packages/PackagesApp.svelte`,
   * removed in US-009) so packages are no longer split between a separate window
   * / Settings entry and the marketplace — installed packs and browsable
   * (Marketplace tab) packs now live in ONE coherent Library surface.
   *
   * It reuses the SAME Tauri commands the old window used — `list_packages`,
   * `check_package_updates`, `install_package`, `update_package`,
   * `uninstall_package` — and the same `packages:*` event stream, so the
   * install / update / uninstall flows are byte-for-byte the behaviour that
   * shipped, just re-housed. Nothing about the install pipeline changed; only
   * the surface that hosts it.
   *
   * Visual language matches LibraryList / MarketplacePanel: desktop-alt CSS
   * variables only (no hardcoded colors), Foundry-style tiles, monospace
   * micro-labels, hairline borders, and the shared light/dark/reduced-* contract.
   */
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import {
    shortSource,
    type PackagesView,
    type InstalledPack,
    type AvailablePack,
    type PackagesProgress,
    type PackagesDone,
  } from '../../lib/packages';

  let view = $state<PackagesView | null>(null);
  let loading = $state(true);
  let busy = $state<{ op: string; name: string } | null>(null);
  let logLines = $state<string[]>([]);
  let errorMsg = $state<string | null>(null);
  let confirmUninstall = $state<string | null>(null);

  const installed = $derived(view?.packs?.installed ?? []);
  const available = $derived(view?.packs?.available ?? []);
  const registryAvailable = $derived(view?.registry?.available ?? []);
  const updatesCount = $derived(installed.filter((p) => p.updateAvailable).length);

  async function refresh(): Promise<void> {
    try {
      view = await invoke<PackagesView>('list_packages');
      errorMsg = view?.error ?? null;
    } catch (e) {
      errorMsg = String(e);
    }
  }

  async function install(source: string, registry = false): Promise<void> {
    busy = { op: 'install', name: shortSource(source) };
    logLines = [];
    errorMsg = null;
    try {
      await invoke('install_package', { source, registry });
    } catch (e) {
      errorMsg = String(e);
      busy = null;
    }
  }

  async function update(name: string): Promise<void> {
    busy = { op: 'update', name };
    logLines = [];
    errorMsg = null;
    try {
      await invoke('update_package', { name });
    } catch (e) {
      errorMsg = String(e);
      busy = null;
    }
  }

  async function uninstall(name: string): Promise<void> {
    confirmUninstall = null;
    busy = { op: 'uninstall', name };
    logLines = [];
    errorMsg = null;
    try {
      await invoke('uninstall_package', { name });
      logLines = [`Uninstalled ${name}.`];
      await refresh();
    } catch (e) {
      errorMsg = String(e);
    } finally {
      busy = null;
    }
  }

  function checkUpdates(): void {
    invoke('check_package_updates').catch(() => {});
  }

  onMount(() => {
    const unlisteners: UnlistenFn[] = [];
    void (async () => {
      unlisteners.push(
        await listen<PackagesProgress>('packages:progress', (e) => {
          logLines = [...logLines.slice(-200), e.payload.line];
        }),
      );
      unlisteners.push(
        await listen<PackagesDone>('packages:complete', async () => {
          busy = null;
          await refresh();
        }),
      );
      unlisteners.push(
        await listen<PackagesDone>('packages:error', (e) => {
          errorMsg = e.payload.message ?? 'Operation failed';
          busy = null;
        }),
      );
      unlisteners.push(
        await listen<PackagesView>('packages:updates', (e) => {
          view = e.payload;
        }),
      );

      // No window-ready handshake here (this is an in-Library tab, not a
      // secondary window) — just cold-load on mount.
      await refresh();
      loading = false;
      // Kick off the slower update probe in the background.
      checkUpdates();
    })();

    return () => unlisteners.forEach((u) => u());
  });

  function contributeSummary(p: InstalledPack): string {
    const parts = Object.entries(p.contributes).map(([k, n]) => `${n} ${k}`);
    return parts.join(', ') || 'no contributions';
  }

  function isGatedOff(a: AvailablePack): boolean {
    return a.conditionalStatus === 'fail';
  }
</script>

<div class="installed-packs" data-testid="installed-packs-panel">
  <div class="toolbar">
    <p class="count" aria-live="polite">
      {#if loading}
        Loading…
      {:else}
        {installed.length}
        {installed.length === 1 ? 'pack' : 'packs'} installed
        {#if updatesCount > 0}
          <span class="badge" data-testid="installed-updates-badge"
            >{updatesCount} update{updatesCount === 1 ? '' : 's'}</span
          >
        {/if}
      {/if}
    </p>
    <button
      type="button"
      class="refresh"
      data-testid="installed-refresh"
      onclick={refresh}
      disabled={!!busy}>Refresh</button
    >
  </div>

  {#if errorMsg}
    <div class="state-error" role="alert" data-testid="installed-error">{errorMsg}</div>
  {/if}

  {#if busy}
    <section class="op" data-testid="installed-op">
      <div class="op-head">
        <span class="spinner" aria-hidden="true"></span>
        {busy.op === 'install' ? 'Installing' : busy.op === 'update' ? 'Updating' : 'Uninstalling'}
        <strong>{busy.name}</strong>…
      </div>
      {#if logLines.length}
        <pre class="log" data-testid="installed-log">{logLines.join('\n')}</pre>
      {/if}
    </section>
  {/if}

  {#if loading}
    <div class="grid-skeleton" aria-busy="true">
      {#each [0, 1, 2, 3] as cell (cell)}
        <div class="card-skeleton"></div>
      {/each}
    </div>
  {:else}
    <section class="group" data-testid="installed-group">
      <h2 class="group-title">Installed</h2>
      {#if installed.length === 0}
        <div class="state-empty">
          <p>No packs installed</p>
          <span>Browse the Marketplace tab to find and install packs.</span>
        </div>
      {/if}
      {#each installed as p (p.name)}
        <div class="row" data-testid="installed-row">
          <div class="row-main">
            <div class="row-title">
              <span class="row-name">{p.name}</span>
              {#if p.version}<span class="pill ver">v{p.version}</span>{/if}
              {#if p.updateAvailable}<span class="pill update">update</span>{/if}
              {#if p.hqCoreSatisfied === false}<span class="pill warn"
                  >needs HQ {p.requiresHqCore}</span
                >{/if}
              {#if p.links.broken > 0}<span class="pill warn"
                  >{p.links.broken} broken link{p.links.broken === 1 ? '' : 's'}</span
                >{/if}
            </div>
            <div class="row-sub">
              {#if p.error}{p.error}{:else}{contributeSummary(p)}{/if}
            </div>
          </div>
          <div class="row-actions">
            {#if p.updateAvailable}
              <button class="action primary" onclick={() => update(p.name)} disabled={!!busy}
                >Update</button
              >
            {/if}
            <button
              class="action danger"
              onclick={() => (confirmUninstall = p.name)}
              disabled={!!busy}>Uninstall</button
            >
          </div>
        </div>
        {#if confirmUninstall === p.name}
          <div class="confirm" data-testid="installed-confirm">
            Remove <strong>{p.name}</strong> and its host links?
            <button class="action danger" onclick={() => uninstall(p.name)}>Remove</button>
            <button class="action ghost" onclick={() => (confirmUninstall = null)}>Cancel</button>
          </div>
        {/if}
      {/each}
    </section>

    {#if available.length > 0 || registryAvailable.length > 0}
      <section class="group" data-testid="installed-available-group">
        <h2 class="group-title">Available from packs.yaml</h2>
        {#each available as a (a.source)}
          <div class="row">
            <div class="row-main">
              <div class="row-title">
                <span class="row-name">{shortSource(a.source)}</span>
                {#if isGatedOff(a)}<span class="pill warn">gated off</span>{/if}
              </div>
              {#if a.description}<div class="row-sub">{a.description}</div>{/if}
            </div>
            <div class="row-actions">
              <button class="action primary" onclick={() => install(a.source, false)} disabled={!!busy}
                >Install</button
              >
            </div>
          </div>
        {/each}
        {#each registryAvailable as r (r.slug)}
          <div class="row">
            <div class="row-main">
              <div class="row-title">
                <span class="row-name">{r.slug}</span>
                {#if r.tier}<span class="pill ver">{r.tier}</span>{/if}
              </div>
              <div class="row-sub">Registry package</div>
            </div>
            <div class="row-actions">
              <button class="action primary" onclick={() => install(r.slug, true)} disabled={!!busy}
                >Install</button
              >
            </div>
          </div>
        {/each}
      </section>
    {/if}

    {#if view?.registry?.offline}
      <p class="offline-note">Registry offline — showing local data only.</p>
    {/if}
  {/if}
</div>

<style>
  .installed-packs {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-width: 0;
  }

  .count {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    color: var(--muted);
    font-size: var(--text-base);
  }

  .badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--row-hover);
    color: var(--amber);
    font-size: var(--text-micro);
    font-weight: 600;
  }

  .refresh {
    height: 32px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
    cursor: pointer;
    transition:
      background 140ms ease,
      border-color 140ms ease;
  }

  .refresh:hover:not(:disabled) {
    border-color: var(--border-strong);
    background: var(--row-hover);
  }

  .refresh:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .refresh:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  /* ---- in-flight op + log ----------------------------------------------- */
  .op {
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
  }

  .op-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--fg);
    font-size: var(--text-base);
  }

  .log {
    margin: var(--space-2) 0 0;
    max-height: 160px;
    padding: var(--space-2) var(--space-3);
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--muted-2);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    line-height: 15px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .spinner {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid var(--border-strong);
    border-top-color: var(--blue);
    animation: installed-spin 0.7s linear infinite;
    display: inline-block;
  }

  @keyframes installed-spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* ---- groups + rows ---------------------------------------------------- */
  .group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .group-title {
    margin: 0;
    color: var(--muted-3);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
    padding: var(--space-3) var(--space-3) var(--space-3) calc(var(--space-3) + 4px);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
  }

  .row-main {
    flex: 1;
    min-width: 0;
  }

  .row-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
    min-width: 0;
  }

  .row-name {
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
  }

  .row-sub {
    margin-top: 2px;
    overflow: hidden;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
    text-overflow: ellipsis;
  }

  .row-actions {
    display: flex;
    flex-shrink: 0;
    gap: var(--space-2);
  }

  .pill {
    display: inline-flex;
    align-items: center;
    padding: 1px 7px;
    border: 1px solid var(--border);
    border-radius: 3px;
    background: var(--row-hover);
    color: var(--muted-2);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.05em;
    line-height: 15px;
    white-space: nowrap;
  }

  .pill.ver {
    color: var(--muted-2);
  }

  .pill.update {
    border-color: color-mix(in srgb, var(--amber) 40%, transparent);
    color: var(--amber);
  }

  .pill.warn {
    border-color: color-mix(in srgb, var(--red) 40%, transparent);
    color: var(--red);
  }

  .action {
    height: 28px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-hover);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 600;
    cursor: pointer;
    transition:
      background 140ms ease,
      border-color 140ms ease,
      filter 140ms ease;
  }

  .action:hover:not(:disabled) {
    border-color: var(--border-strong);
    background: var(--row-active);
  }

  .action:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .action:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .action.primary {
    border-color: var(--blue);
    background: var(--blue);
    color: #fff;
  }

  .action.primary:hover:not(:disabled) {
    filter: brightness(1.08);
    background: var(--blue);
  }

  .action.danger {
    border-color: color-mix(in srgb, var(--red) 45%, transparent);
    color: var(--red);
    background: transparent;
  }

  .action.danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--red) 10%, transparent);
  }

  .action.ghost {
    background: transparent;
  }

  .confirm {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
    color: var(--muted-2);
    font-size: var(--text-base);
  }

  /* ---- states ----------------------------------------------------------- */
  .state-error {
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
    color: var(--red);
    font-size: var(--text-base);
  }

  .state-empty {
    padding: var(--space-6);
    border: 1px dashed var(--border-strong);
    border-radius: 4px;
    background: var(--row-active);
    text-align: center;
  }

  .state-empty p {
    margin: 0 0 var(--space-1);
    color: var(--fg);
    font-weight: 650;
  }

  .state-empty span {
    color: var(--muted);
    font-size: var(--text-base);
  }

  .offline-note {
    margin: 0;
    color: var(--muted-3);
    font-size: var(--text-micro);
  }

  .grid-skeleton {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .card-skeleton {
    height: 56px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
    animation: installed-skeleton-pulse 1.3s ease-in-out infinite;
  }

  @keyframes installed-skeleton-pulse {
    0%,
    100% {
      opacity: 0.5;
    }
    50% {
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .refresh,
    .action {
      transition: none;
    }
    .spinner,
    .card-skeleton {
      animation: none;
    }
  }
</style>
