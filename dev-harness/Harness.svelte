<script lang="ts">
  import Settings from '../src/components/Settings.svelte';
  import Popover from '../src/components/Popover.svelte';
  import BannerNotification from '../src/components/BannerNotification.svelte';
  import CompanyPage from '../src/desktop-alt/pages/CompanyPage.svelte';
  import DesktopApp from '../src/desktop-alt/DesktopApp.svelte';
  import MeetingPermissionsWindow from '../src/components/MeetingPermissionsWindow.svelte';
  import '../src/desktop-alt/styles/desktop-alt.css';
  import { popoverProps, bannerFixtures, workspaces } from './fixtures';
  import { emit } from '@tauri-apps/api/event';

  // The Indigo workspace fixture drives the ?view=company desktop board preview.
  const indigoWorkspace = workspaces.find((w) => w.slug === 'indigo') ?? workspaces[0];

  // View + theme driven by URL query so screenshots target a known state:
  //   ?view=settings|popover|banner   ?theme=light|dark
  //   banner view also takes ?kind=share|meeting|dm|update (default share)
  // For the popover view, size the browser viewport to ~320x440 (the real
  // window size) — the popover root fills 100vw/100vh. For settings, any
  // viewport works; it renders centered on a desktop-ish backdrop.
  const params = new URLSearchParams(window.location.search);
  const view = params.get('view') ?? 'settings';
  const theme = params.get('theme') ?? 'dark';
  const bannerKind = params.get('kind') ?? 'share';

  // The banner reads its transparent-window CSS off html[data-window=dm-banner]
  // and renders only after a `banner:event`. Set the attr + emit the fixture
  // once the component's listener has mounted (next tick).
  document.documentElement.setAttribute(
    'data-window',
    view === 'banner'
      ? 'dm-banner'
      : view === 'company' || view === 'desktop'
        ? 'desktop-alt'
        : view === 'permissions'
          ? 'meeting-permissions'
          : 'main'
  );
  document.documentElement.dataset.forceTheme = theme;

  if (view === 'banner') {
    const payload = bannerFixtures[bannerKind] ?? bannerFixtures.share;
    setTimeout(() => void emit('banner:event', payload), 50);
  }
</script>

{#if view === 'permissions'}
  <!-- The Meeting Permissions wizard. Resize the preview viewport to ~620x720. -->
  <MeetingPermissionsWindow />
{:else if view === 'desktop'}
  <!-- The full desktop-alt window shell (title bar verdict, sidebar, pages,
       live strip). Resize the preview viewport to ~1180x720. -->
  <DesktopApp />
{:else if view === 'banner'}
  <!-- The banner fills 100vw/100vh (tight native window). Resize the preview
       viewport to ~366x104 to see it at real proportions. -->
  <BannerNotification />
{:else if view === 'popover'}
  <Popover {...popoverProps} />
{:else if view === 'company'}
  <!-- The desktop window's company page (default Board tab). Sized to the
       real desktop content area; data-window='desktop-alt' activates the
       desktop token aliases. -->
  <div class="desktop-stage">
    <CompanyPage company={indigoWorkspace} />
  </div>
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

  /* Desktop window content area (company page). desktop-alt.css paints the
     body background under html[data-window='desktop-alt']; this just insets
     the page like the real window's main pane. */
  .desktop-stage {
    box-sizing: border-box;
    min-height: 100vh;
    padding: 28px 32px;
  }

  /* Banner preview: the real window is 366x104, pinned top-right over the
     desktop. The browser harness can't show native NSVisualEffectView vibrancy,
     so a busy wallpaper-ish backdrop stands in to judge tint + the HQ mark.
     (True liquid glass must be confirmed in the Tauri runtime.) */
  :global(html[data-window='dm-banner']),
  :global(html[data-window='dm-banner'] body) {
    background: radial-gradient(120% 120% at 75% 10%, #4a5a7a 0%, #232838 55%, #0c0c12 100%) !important;
  }
  .banner-stage {
    width: 366px;
    height: 104px;
    margin: 40px auto;
  }
</style>
