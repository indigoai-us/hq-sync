<script lang="ts">
  /**
   * One-time auto-sync heads-up for users who UPDATED to an onboarding-aware
   * build. Auto-sync has always been the default; this just tells them it's on
   * and how to switch to manual. Notify-only — it never changes their setting.
   * Shown in-app the next time they open the popover (not forced open). Both
   * buttons dismiss; "Open Settings" also navigates to Settings.
   */
  interface Props {
    /** Dismiss the notice (marks it shown on the Rust side). */
    ondismiss?: () => void;
    /** Dismiss AND open the Settings view. */
    onopensettings?: () => void;
  }

  let { ondismiss, onopensettings }: Props = $props();
</script>

<div class="notice-overlay" role="dialog" aria-modal="true" aria-label="Auto-sync is on">
  <div class="notice-card">
    <h1>Auto-sync is on</h1>
    <p class="description">
      Your HQ now stays backed up automatically — no need to click anything.
      Prefer to sync only when you choose? Turn off <strong>Auto-sync</strong>
      in Settings and use the <strong>Sync now</strong> button instead.
    </p>

    <div class="actions">
      <button class="ghost-btn" onclick={() => onopensettings?.()}>Open Settings</button>
      <button class="primary-btn" onclick={() => ondismiss?.()}>Got it</button>
    </div>
  </div>
</div>

<style>
  .notice-overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    align-items: center;
    justify-content: center;
    box-sizing: border-box;
    padding: 1rem;
    background: var(--popover-bg, rgba(18, 18, 20, 0.82));
    backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    -webkit-backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    color: var(--popover-text, #e0e0e0);
    border-radius: 18px;
  }

  .notice-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    width: 100%;
    max-width: 280px;
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
    line-height: 1.45;
  }

  .description strong {
    color: var(--popover-text, #e0e0e0);
    font-weight: 600;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    width: 100%;
  }

  .primary-btn {
    flex: 1;
    padding: 0.625rem 1.25rem;
    font-size: 0.875rem;
    font-weight: 500;
    font-family: inherit;
    color: var(--popover-primary-text, #111113);
    background-color: var(--popover-primary, #ffffff);
    border: none;
    border-radius: 8px;
    cursor: pointer;
    transition: background-color 0.15s ease;
  }

  .primary-btn:hover {
    background-color: var(--popover-primary-hover, rgba(255, 255, 255, 0.9));
  }

  .primary-btn:active {
    background-color: var(--popover-primary-active, rgba(255, 255, 255, 0.78));
  }

  .ghost-btn {
    flex: 1;
    padding: 0.625rem 1rem;
    font-size: 0.875rem;
    font-weight: 500;
    font-family: inherit;
    color: var(--popover-text, #e0e0e0);
    background: transparent;
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.18));
    border-radius: 8px;
    cursor: pointer;
    transition: background-color 0.15s ease;
  }

  .ghost-btn:hover {
    background: rgba(255, 255, 255, 0.08);
  }

  @media (prefers-color-scheme: light) {
    h1 {
      color: #111113;
    }

    .description {
      color: #6b7280;
    }

    .description strong {
      color: #111113;
    }
  }
</style>
