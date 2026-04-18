<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import SignInPrompt from './components/SignInPrompt.svelte';

  let authenticated = $state(false);
  let expiresAt = $state('');
  let checking = $state(true);
  let syncState = $state<'idle' | 'syncing' | 'error' | 'conflict'>('idle');

  // Collected unlisten handles for cleanup
  let unlisteners: UnlistenFn[] = [];

  async function setupTrayListeners() {
    // Tray menu events
    unlisteners.push(
      await listen('tray:sync-now', async () => {
        syncState = 'syncing';
        await invoke('set_tray_state', { state: 'syncing' });
        try {
          await invoke('start_sync');
        } catch (err) {
          console.error('start_sync failed:', err);
          syncState = 'error';
          await invoke('set_tray_state', { state: 'error' });
        }
      })
    );

    unlisteners.push(
      await listen('tray:open-settings', () => {
        console.log('Settings requested (not yet implemented — US-012)');
      })
    );

    // Sync state events → update local state + tray icon (defensive double-bind)
    unlisteners.push(
      await listen('sync:progress', async () => {
        syncState = 'syncing';
        await invoke('set_tray_state', { state: 'syncing' });
      })
    );

    unlisteners.push(
      await listen('sync:complete', async () => {
        syncState = 'idle';
        await invoke('set_tray_state', { state: 'idle' });
      })
    );

    unlisteners.push(
      await listen('sync:error', async () => {
        syncState = 'error';
        await invoke('set_tray_state', { state: 'error' });
      })
    );

    unlisteners.push(
      await listen('sync:conflict', async () => {
        syncState = 'conflict';
        await invoke('set_tray_state', { state: 'conflict' });
      })
    );
  }

  $effect(() => {
    checkAuth();
    setupTrayListeners();

    return () => {
      unlisteners.forEach((unlisten) => unlisten());
      unlisteners = [];
    };
  });

  async function checkAuth() {
    try {
      const state = await invoke<{
        authenticated: boolean;
        expiresAt: string | null;
      }>('get_auth_state');

      authenticated = state.authenticated;
      expiresAt = state.expiresAt ?? '';
    } catch {
      authenticated = false;
    } finally {
      checking = false;
    }
  }

  function handleAuthSuccess(auth: { authenticated: boolean; expiresAt: string }) {
    authenticated = auth.authenticated;
    expiresAt = auth.expiresAt;
  }
</script>

<main>
  {#if checking}
    <div class="loading">
      <span class="dot-spinner"></span>
    </div>
  {:else if authenticated}
    <div class="authenticated">
      <svg
        width="32"
        height="32"
        viewBox="0 0 48 48"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
      >
        <circle cx="24" cy="24" r="20" fill="#6366f1" opacity="0.15" />
        <circle cx="24" cy="24" r="20" stroke="#6366f1" stroke-width="2.5" fill="none" />
        <path
          d="M16 24l6 6 10-10"
          stroke="#6366f1"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
      <h1>Signed in</h1>
      {#if expiresAt}
        <p class="expires">Session expires: {new Date(expiresAt).toLocaleString()}</p>
      {/if}
    </div>
  {:else}
    <SignInPrompt onsuccess={handleAuthSuccess} />
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    padding: 0;
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto,
      Oxygen, Ubuntu, Cantarell, sans-serif;
    background-color: #1a1a2e;
    color: #e0e0e0;
  }

  main {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100vh;
    text-align: center;
    padding: 0;
  }

  .loading {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
  }

  .dot-spinner {
    display: inline-block;
    width: 20px;
    height: 20px;
    border: 2.5px solid rgba(99, 102, 241, 0.2);
    border-top-color: #6366f1;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .authenticated {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
  }

  .authenticated h1 {
    font-size: 1.25rem;
    font-weight: 600;
    color: #ffffff;
    margin: 0;
  }

  .expires {
    font-size: 0.75rem;
    color: #a0a0b0;
    margin: 0;
  }

  @media (prefers-color-scheme: light) {
    :global(body) {
      background-color: #f8f9fa;
      color: #1a1a2e;
    }

    .authenticated h1 {
      color: #1a1a2e;
    }

    .expires {
      color: #6b7280;
    }
  }
</style>
