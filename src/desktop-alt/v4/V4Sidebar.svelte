<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import type { Workspace, WorkspacesResult } from '../../lib/workspaces';
  import {
    getV4SidebarModel,
    V4_SIDEBAR_MAX_COMPANIES,
    type V4NavId,
    type V4Route,
  } from './model';
  import './tokens.css';

  /**
   * V4 primary sidebar (SPEC section 4 + chrome-master.png): 220px, raised
   * background, hairline right border. Nav (Home / Companies / Messages /
   * Meetings / Library) → COMPANIES section (6px status dot + name per
   * connected company, "N more…" overflow) → spacer → Settings footer
   * (13px "Settings" + 11px account email, hairline top border).
   *
   * Exactly one active row, driven by `route` (see getV4SidebarModel).
   * The companies list renders the `list_syncable_workspaces` result: pass it
   * via `companies` when the shell already holds it (DesktopApp does), or omit
   * the prop and the sidebar fetches the command itself on mount.
   */
  interface Props {
    route: V4Route;
    /** `list_syncable_workspaces` workspaces; omit to let the sidebar self-load. */
    companies?: Workspace[] | null;
    /** Signed-in account email for the Settings footer. */
    accountEmail?: string | null;
    maxCompanies?: number;
    onnavigate?: (route: V4Route) => void;
  }

  let {
    route,
    companies = null,
    accountEmail = null,
    maxCompanies = V4_SIDEBAR_MAX_COMPANIES,
    onnavigate,
  }: Props = $props();

  let fetched = $state<Workspace[]>([]);
  const model = $derived(getV4SidebarModel(route, companies ?? fetched, { maxCompanies }));

  onMount(() => {
    if (companies !== null) return;
    void invoke<WorkspacesResult>('list_syncable_workspaces')
      .then((result) => {
        fetched = result.workspaces;
      })
      .catch((err) => {
        console.error('list_syncable_workspaces failed:', err);
      });
  });

  function go(kind: V4NavId | 'settings', slug?: string) {
    onnavigate?.(slug ? { kind: 'company', slug } : { kind });
  }
</script>

<aside class="v4-sidebar" aria-label="Primary navigation">
  <nav class="v4-nav" aria-label="Primary">
    {#each model.nav as row (row.id)}
      <button
        type="button"
        class="v4-row"
        class:active={row.active}
        aria-current={row.active ? 'page' : undefined}
        onclick={() => go(row.id)}
      >
        {row.label}
      </button>
    {/each}
  </nav>

  <div class="v4-section-label" id="v4-companies-label">Companies</div>
  <nav class="v4-nav" aria-labelledby="v4-companies-label">
    {#each model.companies as row (row.slug)}
      <button
        type="button"
        class="v4-row v4-company-row"
        class:active={row.active}
        aria-current={row.active ? 'page' : undefined}
        onclick={() => go('companies', row.slug)}
      >
        <span class={`v4-dot ${row.tone}`} aria-hidden="true"></span>
        <span class="v4-company-name">{row.label}</span>
      </button>
    {/each}
    {#if model.overflowCount > 0}
      <button
        type="button"
        class="v4-row v4-more-row"
        data-testid="v4-more-companies"
        aria-label={`View ${model.overflowCount} more companies`}
        onclick={() => go('companies')}
      >
        View {model.overflowCount} more companies
      </button>
    {/if}
  </nav>

  <div class="v4-spacer"></div>

  <button
    type="button"
    class="v4-footer"
    class:active={model.settingsActive}
    aria-current={model.settingsActive ? 'page' : undefined}
    onclick={() => go('settings')}
  >
    <span class="v4-footer-label">Settings</span>
    {#if accountEmail}
      <span class="v4-footer-meta">{accountEmail}</span>
    {/if}
  </button>
</aside>

<style>
  .v4-sidebar {
    display: flex;
    flex-direction: column;
    flex: 0 0 220px;
    width: 220px;
    min-height: 0;
    height: 100%;
    padding: 14px 10px 0;
    border-right: 1px solid var(--v4-hairline);
    background: var(--v4-raised);
    font-family:
      'Inter Variable',
      Inter,
      -apple-system,
      'SF Pro Text',
      sans-serif;
  }

  .v4-nav {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .v4-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 28px;
    padding: 0 8px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--v4-text-2);
    font: inherit;
    font-size: 13px;
    font-weight: 400;
    line-height: 1;
    text-align: left;
    cursor: pointer;
  }

  .v4-row:hover {
    background: var(--v4-control-faint);
    color: var(--v4-text-1);
  }

  .v4-row.active {
    background: var(--v4-active-row);
    color: var(--v4-text-1);
    font-weight: 500;
  }

  .v4-section-label {
    margin: 18px 0 6px;
    padding: 0 8px;
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .v4-dot {
    flex: 0 0 6px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .v4-dot.ok {
    background: var(--v4-ok);
  }

  .v4-dot.warn {
    background: var(--v4-warn);
  }

  .v4-dot.error {
    background: var(--v4-error);
  }

  .v4-dot.idle {
    background: var(--v4-idle);
  }

  .v4-company-name {
    overflow: hidden;
    min-width: 0;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .v4-more-row {
    padding-left: 22px;
    color: var(--v4-text-3);
    font-size: 11px;
  }

  .v4-spacer {
    flex: 1 1 auto;
    min-height: 14px;
  }

  .v4-footer {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    margin: 0 -10px;
    padding: 12px 18px 14px;
    border: none;
    border-top: 1px solid var(--v4-hairline);
    background: transparent;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .v4-footer:hover .v4-footer-label,
  .v4-footer.active .v4-footer-label {
    color: var(--v4-text-1);
  }

  .v4-footer.active .v4-footer-label {
    font-weight: 500;
  }

  .v4-footer-label {
    color: var(--v4-text-2);
    font-size: 13px;
    font-weight: 400;
    line-height: 1.2;
  }

  .v4-footer-meta {
    overflow: hidden;
    max-width: 100%;
    color: var(--v4-text-3);
    font-size: 11px;
    line-height: 1.2;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
