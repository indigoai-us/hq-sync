<script lang="ts">
  import Settings from '../src/components/Settings.svelte';
  import Popover from '../src/components/Popover.svelte';
  import { popoverProps } from './fixtures';

  // View + theme driven by URL query so screenshots target a known state:
  //   ?view=settings|popover   ?theme=light|dark
  // For the popover view, size the browser viewport to ~320x440 (the real
  // window size) — the popover root fills 100vw/100vh. For settings, any
  // viewport works; it renders centered on a desktop-ish backdrop.
  const params = new URLSearchParams(window.location.search);
  const view = params.get('view') ?? 'settings';
  const theme = params.get('theme') ?? 'dark';

  document.documentElement.setAttribute('data-window', 'main');
  document.documentElement.dataset.forceTheme = theme;
</script>

{#if view === 'popover'}
  <Popover {...popoverProps} />
{:else}
  <div class="stage" class:light={theme === 'light'}>
    <div class="window">
      <Settings onback={() => (window.location.search = '?view=popover')} />
    </div>
  </div>
{/if}

<style>
  .stage {
    min-height: 100vh;
    display: grid;
    place-items: start center;
    padding: 32px;
    box-sizing: border-box;
    background: radial-gradient(120% 120% at 30% 10%, #3a3a52 0%, #1a1a24 55%, #0c0c12 100%);
  }
  .stage.light {
    background: radial-gradient(120% 120% at 30% 10%, #e9e9f2 0%, #d2d2e0 55%, #b9b9cc 100%);
  }
  .window {
    border-radius: 18px;
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.45), 0 2px 8px rgba(0, 0, 0, 0.3);
  }
</style>
