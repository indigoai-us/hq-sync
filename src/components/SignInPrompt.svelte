<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { open } from '@tauri-apps/plugin-shell';
  import { getCurrentWindow } from '@tauri-apps/api/window';

  interface Props {
    onsuccess?: (auth: { authenticated: boolean; expiresAt: string }) => void;
  }

  let { onsuccess }: Props = $props();

  let loading = $state(false);
  let error = $state('');

  async function handleSignIn() {
    loading = true;
    error = '';

    try {
      // Step 1: Start OAuth login to get authorize URL
      const { authorizeUrl, state } = await invoke<{
        authorizeUrl: string;
        state: string;
      }>('start_oauth_login');

      // Step 2: Open browser for user to authenticate
      await open(authorizeUrl);

      // Step 3: Listen for the OAuth callback code
      const { code } = await invoke<{ code: string }>(
        'oauth_listen_for_code',
        { state }
      );

      // Step 4: Exchange code for tokens
      const result = await invoke<{
        authenticated: boolean;
        expiresAt: string;
      }>('oauth_exchange_code', { code });

      // Step 5: Notify parent of success
      if (result.authenticated) {
        // Pull focus back from the browser to the menubar popover so the
        // user sees the post-sign-in UI transition immediately. `.show()`
        // is defensive — the popover should still be open from the tray
        // click that started this flow, but the OAuth redirect can take a
        // while and users occasionally dismiss the window in the meantime.
        try {
          const win = getCurrentWindow();
          await win.show();
          await win.setFocus();
        } catch (focusErr) {
          // Focus-stealing isn't critical; log but don't block success.
          console.warn('[signin] failed to refocus window:', focusErr);
        }
        onsuccess?.(result);
      } else {
        error = 'Authentication failed. Please try again.';
      }
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    } finally {
      loading = false;
    }
  }
</script>

<div class="sign-in-container">
  <div class="sign-in-card">
    <div class="icon">
      <svg
        width="48"
        height="48"
        viewBox="0 0 48 48"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
      >
        <path
          d="M24 4L8 12v12c0 11.1 6.8 21.4 16 24 9.2-2.6 16-12.9 16-24V12L24 4z"
          fill="#6366f1"
          opacity="0.15"
        />
        <path
          d="M24 4L8 12v12c0 11.1 6.8 21.4 16 24 9.2-2.6 16-12.9 16-24V12L24 4z"
          stroke="#6366f1"
          stroke-width="2.5"
          stroke-linejoin="round"
          fill="none"
        />
        <path
          d="M18 24l4 4 8-8"
          stroke="#6366f1"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
    </div>

    <h1>Sign in to HQ</h1>
    <p class="description">Use your Google account to sync your HQ files.</p>

    <button
      class="sign-in-btn"
      onclick={handleSignIn}
      disabled={loading}
    >
      {#if loading}
        <span class="spinner"></span>
        Waiting for browser…
      {:else}
        <svg
          class="google-glyph"
          width="18"
          height="18"
          viewBox="0 0 18 18"
          aria-hidden="true"
        >
          <path
            d="M17.64 9.2c0-.637-.057-1.251-.164-1.84H9v3.481h4.844a4.14 4.14 0 0 1-1.796 2.716v2.259h2.908c1.702-1.567 2.684-3.875 2.684-6.615z"
            fill="#4285F4"
          />
          <path
            d="M9 18c2.43 0 4.467-.806 5.956-2.184l-2.908-2.259c-.806.54-1.837.86-3.048.86-2.344 0-4.328-1.584-5.036-3.711H.957v2.332A8.997 8.997 0 0 0 9 18z"
            fill="#34A853"
          />
          <path
            d="M3.964 10.706A5.41 5.41 0 0 1 3.682 9c0-.593.102-1.17.282-1.706V4.962H.957A8.997 8.997 0 0 0 0 9c0 1.452.348 2.827.957 4.038l3.007-2.332z"
            fill="#FBBC05"
          />
          <path
            d="M9 3.579c1.321 0 2.508.454 3.44 1.345l2.582-2.58C13.463.892 11.426 0 9 0A8.997 8.997 0 0 0 .957 4.962L3.964 7.294C4.672 5.167 6.656 3.58 9 3.58z"
            fill="#EA4335"
          />
        </svg>
        Continue with Google
      {/if}
    </button>

    {#if loading}
      <p class="loading-hint">
        A browser window opened for Google sign-in. Complete it there and
        you'll return here automatically.
      </p>
    {/if}

    {#if error}
      <p class="error">{error}</p>
    {/if}

    <p class="footer">Powered by Indigo</p>
  </div>
</div>

<style>
  .sign-in-container {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100vh;
    padding: 1rem;
  }

  .sign-in-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    width: 100%;
    max-width: 280px;
  }

  .icon {
    margin-bottom: 1rem;
  }

  h1 {
    font-size: 1.25rem;
    font-weight: 600;
    color: #ffffff;
    margin: 0 0 0.5rem 0;
  }

  .description {
    font-size: 0.8125rem;
    color: #a0a0b0;
    margin: 0 0 1.5rem 0;
    line-height: 1.4;
  }

  .sign-in-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.625rem 1.25rem;
    font-size: 0.875rem;
    font-weight: 500;
    font-family: inherit;
    color: #ffffff;
    background-color: #6366f1;
    border: none;
    border-radius: 8px;
    cursor: pointer;
    transition: background-color 0.15s ease, opacity 0.15s ease;
  }

  .sign-in-btn:hover:not(:disabled) {
    background-color: #4f46e5;
  }

  .sign-in-btn:active:not(:disabled) {
    background-color: #4338ca;
  }

  .sign-in-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }

  .spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid rgba(255, 255, 255, 0.3);
    border-top-color: #ffffff;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .error {
    font-size: 0.75rem;
    color: #ef4444;
    margin: 0.75rem 0 0 0;
    line-height: 1.4;
  }

  .loading-hint {
    font-size: 0.6875rem;
    color: #a0a0b0;
    margin: 0.75rem 0 0 0;
    line-height: 1.4;
  }

  .google-glyph {
    flex-shrink: 0;
  }

  .footer {
    font-size: 0.6875rem;
    color: #555568;
    margin: 1.5rem 0 0 0;
    letter-spacing: 0.02em;
  }

  @media (prefers-color-scheme: light) {
    h1 {
      color: #1a1a2e;
    }

    .description {
      color: #6b7280;
    }

    .loading-hint {
      color: #6b7280;
    }

    .footer {
      color: #9ca3af;
    }

    .error {
      color: #dc2626;
    }
  }
</style>
