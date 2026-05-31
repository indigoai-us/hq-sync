<script lang="ts">
  // Custom in-app notification banner — one surface for every source (DM,
  // share, update). A transparent, always-on-top, non-activating Tauri webview
  // (see commands/banner.rs) rendering a glassy card that matches macOS's
  // notification look. On action it invokes `banner_action`, which re-emits the
  // unified `notification:banner-action` event — App.svelte routes it by `kind`
  // (open DM/share detail, copy prompt, install update). Auto-dismisses; hover
  // pauses.
  import '../styles/popover.css';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';

  interface BannerPayload {
    kind: string;
    title: string;
    body: string;
    iconText?: string | null;
    actionLabel?: string | null;
    actionId?: string | null;
    clickActionId: string;
    data: unknown;
  }

  const AUTO_DISMISS_MS = 6000;

  let payload = $state<BannerPayload | null>(null);
  let leaving = $state(false);
  let paused = $state(false);
  let dismissTimer: ReturnType<typeof setTimeout> | undefined;

  function armDismiss(): void {
    clearTimeout(dismissTimer);
    if (paused) return;
    dismissTimer = setTimeout(() => void dismiss(), AUTO_DISMISS_MS);
  }

  async function dismiss(): Promise<void> {
    if (leaving) return;
    leaving = true; // play slide-out, then close the window in Rust.
    setTimeout(() => void invoke('dismiss_banner').catch(() => {}), 180);
  }

  async function action(actionId: string | null | undefined): Promise<void> {
    if (!payload || !actionId) return;
    clearTimeout(dismissTimer);
    leaving = true;
    try {
      // banner_action re-emits notification:banner-action AND closes the window.
      await invoke('banner_action', { action: actionId, payload });
    } catch (err) {
      console.error('banner: action failed', err);
      void invoke('dismiss_banner').catch(() => {});
    }
  }

  function onPointerEnter(): void {
    paused = true;
    clearTimeout(dismissTimer);
  }
  function onPointerLeave(): void {
    paused = false;
    armDismiss();
  }

  $effect(() => {
    let unlisten: (() => void) | undefined;
    listen<BannerPayload>('banner:event', (e) => {
      payload = e.payload;
      armDismiss(); // (re)start the countdown each time a notification arrives.
    }).then((fn) => {
      unlisten = fn;
      // Ready-handshake: tell Rust the listener is mounted so it emits the
      // pending payload + shows the window (mirrors DmDetail / ShareDetail).
      invoke('banner_window_ready');
    });
    return () => {
      unlisten?.();
      clearTimeout(dismissTimer);
    };
  });
</script>

<svelte:options runes={true} />

{#if payload}
  <!-- Whole card is the click affordance (body click → clickActionId). Buttons
       stopPropagation so the chip / close don't also trigger it. -->
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div
    class="banner"
    class:leaving
    data-kind={payload.kind}
    role="button"
    tabindex="0"
    onpointerenter={onPointerEnter}
    onpointerleave={onPointerLeave}
    onclick={() => action(payload?.clickActionId)}
  >
    <div class="avatar" aria-hidden="true">{payload.iconText ?? '•'}</div>

    <div class="content">
      <div class="top">
        <span class="app">HQ Sync</span>
        <span class="sep">·</span>
        <span class="from">{payload.title}</span>
        <button
          class="close"
          aria-label="Dismiss"
          onclick={(e) => { e.stopPropagation(); void dismiss(); }}
        >×</button>
      </div>
      <p class="body">{payload.body}</p>
      {#if payload.actionLabel}
        <div class="actions">
          <button
            class="chip"
            onclick={(e) => { e.stopPropagation(); void action(payload?.actionId); }}
          >{payload.actionLabel}</button>
          <span class="hint">click to open</span>
        </div>
      {/if}
    </div>

    {#if !paused && !leaving}
      <span class="lifebar" style="animation-duration: {AUTO_DISMISS_MS}ms"></span>
    {/if}
  </div>
{/if}

<style>
  /* The data-window attr is set ON <html> (main.ts), so target html directly —
     the old `[data-window] html` descendant selector matched nothing, leaving
     popover.css's opaque html/#app background painting the square window behind
     the rounded card. !important beats popover.css's global rules. */
  :global(html[data-window="dm-banner"]),
  :global(html[data-window="dm-banner"] body),
  :global(html[data-window="dm-banner"] #app) {
    margin: 0;
    padding: 0;
    background: transparent !important;
    background-color: transparent !important;
    color-scheme: dark;
    overflow: hidden;
  }

  .banner {
    position: relative;
    box-sizing: border-box;
    width: 100vw;
    /* Fill the window exactly — the window is sized tight from Rust (BANNER_H)
       so the vibrancy backdrop and card coincide. No JS resize: resizing the
       NSWindow leaves the NSVisualEffectView's rounded-corner mask at the old
       geometry, which exposed square corners behind the card. */
    height: 100vh;
    display: flex;
    gap: 0.75rem;
    align-items: center;
    padding: 0.875rem;
    /* Pure liquid glass — let the native Popover-material frost (banner.rs)
       dominate. NO CSS backdrop-filter (native vibrancy does the blur; a CSS
       one renders a square ignoring border-radius). A faint dark tint keeps
       text legible; a top sheen + bright inner edge give the glass its sparkle. */
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.08), rgba(255, 255, 255, 0) 40%),
      linear-gradient(180deg, rgba(28, 28, 36, 0.10) 0%, rgba(16, 16, 22, 0.16) 100%);
    border: 0.5px solid rgba(255, 255, 255, 0.22);
    border-radius: 18px;
    box-shadow:
      inset 0 0.5px 0 rgba(255, 255, 255, 0.18),
      0 14px 44px rgba(0, 0, 0, 0.5);
    color: rgba(255, 255, 255, 0.9);
    font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    cursor: pointer;
    overflow: hidden;
    animation: slide-in 0.28s cubic-bezier(0.16, 1, 0.3, 1);
    transition: transform 0.18s ease, opacity 0.18s ease;
  }

  .banner.leaving {
    transform: translateY(-12px);
    opacity: 0;
  }

  @keyframes slide-in {
    from { transform: translateY(-120%); opacity: 0; }
    to   { transform: translateY(0);     opacity: 1; }
  }

  .avatar {
    flex-shrink: 0;
    width: 38px;
    height: 38px;
    border-radius: 11px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 0.8125rem;
    font-weight: 600;
    color: #dce8ff;
    background: linear-gradient(135deg, rgba(120, 170, 255, 0.45), rgba(150, 120, 255, 0.4));
    box-shadow: inset 0 0 0 0.5px rgba(255, 255, 255, 0.18);
  }

  /* Source-tinted avatars so the kind reads at a glance. */
  .banner[data-kind="share"] .avatar {
    background: linear-gradient(135deg, rgba(126, 226, 168, 0.42), rgba(120, 170, 255, 0.38));
  }
  .banner[data-kind="update"] .avatar {
    background: linear-gradient(135deg, rgba(255, 196, 120, 0.45), rgba(255, 140, 120, 0.4));
    font-size: 1.05rem;
  }

  .content { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 0.25rem; }
  .top { display: flex; align-items: center; gap: 0.375rem; font-size: 0.75rem; }
  /* "HQ Sync" + separator never shrink/wrap — the sender is the only part
     that truncates with an ellipsis when the title is long. */
  .app { font-weight: 600; color: #fff; white-space: nowrap; flex-shrink: 0; }
  .sep { color: rgba(255, 255, 255, 0.4); flex-shrink: 0; }
  .from {
    flex: 1;
    min-width: 0;
    color: rgba(255, 255, 255, 0.7);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .close {
    margin-left: auto;
    flex-shrink: 0;
    width: 18px;
    height: 18px;
    line-height: 1;
    border: none;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.08);
    color: rgba(255, 255, 255, 0.7);
    font-size: 0.875rem;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background-color 0.12s ease;
  }
  .close:hover { background: rgba(255, 255, 255, 0.18); }

  .body {
    margin: 0;
    font-size: 0.8125rem;
    line-height: 1.4;
    color: rgba(255, 255, 255, 0.88);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    word-break: break-word;
  }

  .actions { display: flex; align-items: center; gap: 0.625rem; margin-top: 0.125rem; }

  .chip {
    padding: 0.25rem 0.625rem;
    border-radius: 7px;
    border: none;
    font-size: 0.6875rem;
    font-weight: 500;
    font-family: inherit;
    cursor: pointer;
    background: rgba(120, 170, 255, 0.22);
    color: #dce8ff;
    transition: background-color 0.12s ease;
  }
  .chip:hover { background: rgba(120, 170, 255, 0.34); }

  .hint { font-size: 0.625rem; color: rgba(255, 255, 255, 0.4); }

  .lifebar {
    position: absolute;
    /* Inset from the edges so its square ends can never reach the card's
       rounded corners. */
    left: 12px;
    right: 12px;
    bottom: 6px;
    height: 2px;
    border-radius: 2px;
    transform-origin: left;
    background: linear-gradient(90deg, rgba(120, 170, 255, 0.7), rgba(150, 120, 255, 0.6));
    animation-name: drain;
    animation-timing-function: linear;
    animation-fill-mode: forwards;
  }

  @keyframes drain {
    from { transform: scaleX(1); }
    to   { transform: scaleX(0); }
  }
</style>
