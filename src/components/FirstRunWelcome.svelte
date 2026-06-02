<script lang="ts">
  /**
   * First-run welcome carousel. Shown once, on a brand-new install's first
   * launch, layered over the Popover while the first cloud sync runs
   * underneath. A short 2–3 slide intro: what HQ Sync does, the fact it's
   * already syncing (with the one-time built-in-files reassurance), and where
   * to find it. Dismissing (Done or ×) fires `ondone`, which marks first-run
   * complete on the Rust side.
   */
  interface Props {
    /** Fired when the user finishes or dismisses the carousel. */
    ondone?: () => void;
  }

  let { ondone }: Props = $props();

  interface Slide {
    title: string;
    body: string;
  }

  const slides: Slide[] = [
    {
      title: 'Welcome to HQ Sync',
      body: 'HQ Sync keeps your HQ backed up to the cloud and in sync across your devices and team — quietly, in the background.',
    },
    {
      title: "We're syncing you now",
      body: "Your first sync is already running. It includes HQ's built-in files, so the count can look big — that's normal, and it only happens once.",
    },
    {
      title: 'Find it anytime',
      body: 'HQ Sync lives in your menu bar at the top of the screen. Click its icon to check status or sync on demand. It keeps running in the background.',
    },
  ];

  let index = $state(0);
  const isLast = $derived(index === slides.length - 1);

  function next() {
    if (isLast) {
      ondone?.();
    } else {
      index += 1;
    }
  }

  function back() {
    if (index > 0) index -= 1;
  }

  function dismiss() {
    ondone?.();
  }
</script>

<div class="welcome-overlay" role="dialog" aria-modal="true" aria-label="Welcome to HQ Sync">
  <div class="welcome-card">
    <button class="close-btn" onclick={dismiss} aria-label="Dismiss welcome">×</button>

    <div class="welcome-body">
      <h1>{slides[index].title}</h1>
      <p class="description">{slides[index].body}</p>
    </div>

    <div class="dots" aria-hidden="true">
      {#each slides as _, i}
        <span class="dot" class:active={i === index}></span>
      {/each}
    </div>

    <div class="actions">
      {#if index > 0}
        <button class="ghost-btn" onclick={back}>Back</button>
      {:else}
        <span class="spacer"></span>
      {/if}
      <button class="primary-btn" onclick={next}>
        {isLast ? 'Done' : 'Next'}
      </button>
    </div>
  </div>
</div>

<style>
  .welcome-overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    align-items: center;
    justify-content: center;
    box-sizing: border-box;
    padding: 1rem;
    /* Scrim over the live popover so the running sync stays faintly visible
       behind the card. */
    background: var(--popover-bg, rgba(18, 18, 20, 0.82));
    backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    -webkit-backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    color: var(--popover-text, #e0e0e0);
    border-radius: 18px;
  }

  .welcome-card {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    width: 100%;
    max-width: 280px;
  }

  .close-btn {
    position: absolute;
    top: -0.25rem;
    right: -0.25rem;
    width: 24px;
    height: 24px;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 1.1rem;
    line-height: 1;
    color: var(--popover-text-muted, #a0a0b0);
    background: transparent;
    border: none;
    border-radius: 6px;
    cursor: pointer;
  }

  .close-btn:hover {
    color: var(--popover-text, #e0e0e0);
    background: rgba(255, 255, 255, 0.08);
  }

  .welcome-body {
    min-height: 132px;
    display: flex;
    flex-direction: column;
    justify-content: center;
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
    margin: 0;
    line-height: 1.45;
  }

  .dots {
    display: flex;
    gap: 0.375rem;
    margin: 1.25rem 0;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.22);
    transition: background-color 0.15s ease;
  }

  .dot.active {
    background: var(--popover-primary, #ffffff);
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    width: 100%;
  }

  .spacer {
    flex: 1;
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
  }
</style>
