<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { open } from '@tauri-apps/plugin-shell';

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
    <p class="description">Connect your HQ account to enable sync</p>

    <button
      class="sign-in-btn"
      onclick={handleSignIn}
      disabled={loading}
    >
      {#if loading}
        <span class="spinner"></span>
        Signing in...
      {:else}
        Sign in
      {/if}
    </button>

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

    .footer {
      color: #9ca3af;
    }

    .error {
      color: #dc2626;
    }
  }
</style>
