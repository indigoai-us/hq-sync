<script lang="ts">
  // Custom in-app notification banner — one surface for every source (DM,
  // share, update). A transparent, always-on-top, non-activating Tauri webview
  // (see commands/banner.rs) rendering a glassy card that matches macOS's
  // notification look. On action it invokes `banner_action`, which re-emits the
  // unified `notification:banner-action` event — App.svelte routes it by `kind`
  // (open DM/share detail, copy prompt, install update). Auto-dismisses; hover
  // pauses.
  import '../styles/popover.css';
  import { tick } from 'svelte';
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

  // Card content (title + body + optional action row). Measured after each
  // payload so the window can hug the content height — see fitHeight().
  let contentEl = $state<HTMLDivElement | undefined>(undefined);
  const AVATAR_H = 38; // px — the HQ chip; the window never shrinks below it
  const CARD_PAD = 28; // px — vertical padding (0.875rem top + bottom)

  /** Measure the rendered card and resize the native window to fit it, so a
   *  one-line banner isn't padded out to the tallest (share/meeting) layout. */
  async function fitHeight(): Promise<void> {
    await tick();
    requestAnimationFrame(() => {
      const ch = contentEl?.getBoundingClientRect().height ?? 0;
      const target = Math.round(Math.max(ch, AVATAR_H) + CARD_PAD);
      void invoke('resize_banner', { height: target }).catch(() => {});
    });
  }

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
      leaving = false; // a fresh banner reusing the window must not stay faded.
      armDismiss(); // (re)start the countdown each time a notification arrives.
      void fitHeight(); // size the window to the new content.
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
    <div class="avatar" aria-hidden="true">
      <!-- Flat monochrome HQ wordmark (src/assets/hq-mark.svg, inlined so it
           inherits `currentColor` and needs no bundler asset wiring). One mark
           for every source — no per-kind gradient. -->
      <svg class="hq-mark" viewBox="0 0 280 161" fill="currentColor" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="HQ">
        <path d="M85.7251 3.66162H118.034V154.434H85.7251V89.8176H32.3085V154.434H0V3.66162H32.3085V57.5091H85.7251V3.66162Z"/>
        <path d="M257.169 160.035L241.014 144.096C235.343 147.973 229.096 150.988 222.276 153.142C215.527 155.296 208.419 156.373 200.952 156.373C190.757 156.373 181.172 154.363 172.197 150.342C163.223 146.25 155.325 140.65 148.505 133.542C141.684 126.362 136.335 118.07 132.458 108.664C128.581 99.187 126.642 89.0278 126.642 78.1865C126.642 67.417 128.581 57.3296 132.458 47.9242C136.335 38.4471 141.684 30.1187 148.505 22.939C155.325 15.7593 163.223 10.1592 172.197 6.1386C181.172 2.0462 190.757 0 200.952 0C211.219 0 220.84 2.0462 229.814 6.1386C238.789 10.1592 246.686 15.7593 253.507 22.939C260.328 30.1187 265.641 38.4471 269.446 47.9242C273.323 57.3296 275.261 67.417 275.261 78.1865C275.261 86.0123 274.184 93.5151 272.031 100.695C269.948 107.803 267.077 114.444 263.415 120.618L280 137.203L257.169 160.035ZM200.952 124.065C203.896 124.065 206.732 123.741 209.46 123.095C212.26 122.449 214.952 121.552 217.537 120.403L208.491 111.357L231.322 88.5252L239.291 96.4946C240.512 93.6946 241.409 90.7509 241.984 87.6637C242.63 84.5764 242.953 81.4173 242.953 78.1865C242.953 71.8684 241.84 65.9452 239.614 60.4168C237.461 54.8885 234.445 50.0422 230.568 45.878C226.691 41.642 222.204 38.3394 217.106 35.9701C212.08 33.529 206.696 32.3085 200.952 32.3085C195.208 32.3085 189.788 33.529 184.69 35.9701C179.664 38.3394 175.213 41.642 171.336 45.878C167.459 50.0422 164.407 54.8885 162.182 60.4168C160.028 65.9452 158.951 71.8684 158.951 78.1865C158.951 84.5046 160.028 90.4637 162.182 96.0639C164.407 101.592 167.459 106.474 171.336 110.71C175.213 114.875 179.664 118.141 184.69 120.511C189.788 122.88 195.208 124.065 200.952 124.065Z"/>
      </svg>
    </div>

    <div class="content" bind:this={contentEl}>
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
    /* Adapt to the OS appearance so the light-mode tokens in popover.css fire —
       the native Popover-material frost (banner.rs) already follows the system
       light/dark, so locking the webview to dark left white text on a light
       frost (unreadable) in Light Mode. `light dark` keeps the webview in step
       with the frost; text colours below resolve through the adaptive tokens. */
    color-scheme: light dark;
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
       one renders a square ignoring border-radius). Tint + border + sheen reuse
       the SAME popover.css tokens as the main menubar window so the frost reads
       identically (the window applies the identical apply_vibrancy call). */
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.1), rgba(255, 255, 255, 0) 42%),
      var(--popover-bg);
    border: 0.5px solid var(--popover-border);
    border-radius: 18px;
    box-shadow:
      inset 0 0.5px 0 var(--popover-highlight),
      0 14px 44px rgba(0, 0, 0, 0.5);
    color: var(--popover-text);
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

  /* Flat, monochrome HQ chip — one neutral glass square for every source. No
     gradient, no per-kind colour (deliberately removed): the HQ mark is the
     brand cue, the kind reads from the title + body. Chip surface + edge reuse
     popover.css tokens so it sits in the same glass as the card. */
  .avatar {
    flex-shrink: 0;
    width: 38px;
    height: 38px;
    border-radius: 11px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--popover-text-heading);
    background: var(--popover-surface);
    box-shadow: inset 0 0 0 0.5px var(--popover-border);
  }

  .hq-mark {
    width: 20px;
    height: auto;
    display: block;
  }

  .content { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 0.25rem; }
  .top { display: flex; align-items: center; gap: 0.375rem; font-size: 0.75rem; }
  /* "HQ Sync" + separator never shrink/wrap — the sender is the only part
     that truncates with an ellipsis when the title is long. */
  .app { font-weight: 600; color: var(--popover-text-heading); white-space: nowrap; flex-shrink: 0; }
  .sep { color: var(--popover-text-muted); flex-shrink: 0; }
  .from {
    flex: 1;
    min-width: 0;
    color: var(--popover-text);
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
    background: var(--popover-surface);
    color: var(--popover-text-muted);
    font-size: 0.875rem;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background-color 0.12s ease;
  }
  .close:hover { background: var(--popover-surface-strong); }

  .body {
    margin: 0;
    font-size: 0.8125rem;
    line-height: 1.4;
    color: var(--popover-text);
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

  .hint { font-size: 0.625rem; color: var(--popover-text-muted); }

  /* Light Mode: the near-white chip text is illegible on a light frost — give
     the action chip a darker blue text + a touch more fill so it stays the
     accent affordance while remaining readable. */
  @media (prefers-color-scheme: light) {
    .chip {
      background: rgba(40, 90, 200, 0.16);
      color: #1c3d80;
    }
    .chip:hover { background: rgba(40, 90, 200, 0.26); }
  }
</style>
