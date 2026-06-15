<script lang="ts">
  import Settings from '../src/components/Settings.svelte';
  import Popover from '../src/components/Popover.svelte';
  import BannerNotification from '../src/components/BannerNotification.svelte';
  import CompanyPage from '../src/desktop-alt/pages/CompanyPage.svelte';
  import CompaniesPage from '../src/desktop-alt/pages/CompaniesPage.svelte';
  import DesktopApp from '../src/desktop-alt/DesktopApp.svelte';
  import MeetingPermissionsWindow from '../src/components/MeetingPermissionsWindow.svelte';
  import Conversation, {
    type ConversationMessage,
  } from '../src/components/messaging/Conversation.svelte';
  import CreateChannel from '../src/components/messaging/CreateChannel.svelte';
  import '../src/desktop-alt/styles/desktop-alt.css';
  import { popoverProps, bannerFixtures, workspaces, hqCliUpdateAvailable } from './fixtures';
  import { emit } from '@tauri-apps/api/event';

  // Fixture thread for ?view=conversation — exercises the copy-message toolbar
  // and the copy-prompt button (the last inbound message carries an agent
  // prompt). Times are passed in via ISO strings so the harness stays
  // deterministic without Date.now().
  const conversationMessages: ConversationMessage[] = [
    {
      eventId: 'm1',
      fromPersonUid: 'prs_maya',
      fromDisplayName: 'Maya Chen',
      body: 'Morning! Did the conflict-versioning branch land?',
      createdAt: '2026-06-10T16:02:00Z',
      direction: 'in',
    },
    {
      eventId: 'm2',
      fromPersonUid: 'prs_me',
      fromDisplayName: 'Corey Epstein',
      body: 'Just merged it — running the e2e suite now.',
      createdAt: '2026-06-10T16:04:00Z',
      direction: 'out',
    },
    {
      eventId: 'm3',
      fromPersonUid: 'prs_maya',
      fromDisplayName: 'Maya Chen',
      body: 'Nice. Can you kick off the audit on the indigo repo?',
      details: 'Repo: repos/private/indigo-app · branch: main',
      prompt: '/run-project indigo-app --story audit-pass --headless',
      createdAt: '2026-06-10T16:06:00Z',
      direction: 'in',
    },
  ];

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
  // ?state=error   renders the "Sync initialized" notice banner.
  // ?state=cli-update renders the "hq CLI update available" banner (copyable
  //                one-liner + dismiss ×) off a deliberately-stale CLI fixture.
  // Otherwise the popover mounts in its idle fixture state.
  const stateOverride = params.get('state');
  const previewPopoverProps =
    stateOverride === 'error'
      ? { ...popoverProps, syncState: 'error' as const, errorMessage: 'failed to push indigo: exit 1', errorCompany: 'indigo' }
      : stateOverride === 'cli-update'
        ? { ...popoverProps, hqCliUpdateAvailable }
        : popoverProps;

  // The banner reads its transparent-window CSS off html[data-window=dm-banner]
  // and renders only after a `banner:event`. Set the attr + emit the fixture
  // once the component's listener has mounted (next tick).
  document.documentElement.setAttribute(
    'data-window',
    view === 'banner'
      ? 'dm-banner'
      : view === 'company' || view === 'desktop' || view === 'companies'
        ? 'desktop-alt'
        : view === 'permissions'
          ? 'meeting-permissions'
          : view === 'conversation' || view === 'createchannel'
            ? 'messages'
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
  <Popover {...previewPopoverProps} />
{:else if view === 'conversation'}
  <!-- The shared messaging Conversation (desktop Messages styling via
       data-window='messages'). Hover a bubble to reveal the copy-message
       button; the last message carries an agent prompt → Copy prompt. -->
  <div class="conversation-stage">
    <Conversation
      messages={conversationMessages}
      showAuthors={true}
      onsend={() => {}}
      ontogglereaction={() => {}}
    />
  </div>
{:else if view === 'createchannel'}
  <!-- The New-channel modal (font-size pass). data-window='messages' so the
       desktop tokens resolve. Companies/contacts come from Tauri commands that
       the harness doesn't fully mock, so the dropdown + picker may be empty —
       the type scale is what this view is for. -->
  <div class="conversation-stage" style="justify-content: center; background: var(--bg, #161618);">
    <CreateChannel onclose={() => {}} oncreated={() => {}} />
  </div>
{:else if view === 'company'}
  <!-- The desktop window's company page (default Board tab). Sized to the
       real desktop content area; data-window='desktop-alt' activates the
       desktop token aliases. -->
  <div class="desktop-stage">
    <CompanyPage company={indigoWorkspace} />
  </div>
{:else if view === 'companies'}
  <!-- The desktop Companies page in isolation (DesktopApp is auth-gated). Drives
       the per-company Shared/All sync-mode toggle, the All→Shared confirm, and
       the cloud-unreachable gating. Append ?cloud=off to preview the offline
       notice + disabled writes. Resize the viewport to ~1180x720. -->
  <div class="desktop-stage">
    <CompaniesPage
      {workspaces}
      cloudReachable={params.get('cloud') !== 'off'}
      onopencompany={() => {}}
      onrefresh={() => {}}
    />
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

  /* Conversation preview: a fixed-width column with the messages-window
     surface, so the thread + composer render at realistic proportions. The
     component is column-flex and fills height, so the stage pins it. */
  .conversation-stage {
    box-sizing: border-box;
    width: 460px;
    height: 100vh;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    background: var(--bg, #161618);
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
