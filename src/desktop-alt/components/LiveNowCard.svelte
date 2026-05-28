<script lang="ts">
  import type { ActiveMeeting } from '../../lib/activeMeetings';

  interface Props {
    meeting: ActiveMeeting | null;
    onstart: (windowId: string) => void;
    onstop: (windowId: string) => void;
  }

  let { meeting, onstart, onstop }: Props = $props();

  const isRecording = $derived(meeting?.state === 'recording' || meeting?.state === 'stopping');
  const isBusy = $derived(meeting?.state === 'starting' || meeting?.state === 'stopping');
  const title = $derived(meeting?.summary || platformLabel(meeting?.platform) || 'Detected meeting');
  const stateLabel = $derived(meeting ? labelForState(meeting.state) : 'Standing by');

  function platformLabel(platform?: string): string {
    if (!platform) return '';
    if (platform === 'meet') return 'Google Meet';
    return platform.charAt(0).toUpperCase() + platform.slice(1);
  }

  function labelForState(state: ActiveMeeting['state']): string {
    switch (state) {
      case 'starting':
        return 'Starting';
      case 'recording':
        return 'Recording';
      case 'stopping':
        return 'Stopping';
      case 'error':
        return 'Needs attention';
      case 'detected':
      default:
        return 'Detected';
    }
  }
</script>

<section class="live-card" aria-labelledby="live-now-title">
  <div class="live-heading">
    <div>
      <p>Live now</p>
      <h2 id="live-now-title">{title}</h2>
    </div>
    <span class="live-pill" class:recording={meeting?.state === 'recording'}>
      {stateLabel}
    </span>
  </div>

  {#if meeting}
    <dl class="live-details">
      <div>
        <dt>Platform</dt>
        <dd>{platformLabel(meeting.platform) || 'Meeting app'}</dd>
      </div>
      <div>
        <dt>Detected</dt>
        <dd>{new Date(meeting.detectedAt).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })}</dd>
      </div>
    </dl>

    {#if meeting.error}
      <p class="live-error">{meeting.error}</p>
    {/if}

    <div class="live-actions">
      {#if isRecording}
        <button type="button" onclick={() => onstop(meeting.windowId)} disabled={isBusy}>
          {meeting.state === 'stopping' ? 'Stopping' : 'Stop recording'}
        </button>
      {:else}
        <button type="button" class="primary" onclick={() => onstart(meeting.windowId)} disabled={isBusy}>
          {meeting.state === 'starting' ? 'Starting' : 'Start recording'}
        </button>
      {/if}
    </div>
  {:else}
    <p class="empty-copy">No active meeting window has been detected.</p>
  {/if}
</section>

<style>
  .live-card {
    min-width: 0;
    padding: 14px;
    border: 1px solid #e4e4e7;
    border-radius: 8px;
    background: #ffffff;
  }

  .live-heading {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 14px;
  }

  .live-heading > div {
    min-width: 0;
  }

  .live-heading p,
  .empty-copy,
  .live-error {
    margin: 0;
    color: #71717a;
    font-size: 12px;
    line-height: 18px;
  }

  .live-heading h2 {
    margin: 2px 0 0;
    overflow: hidden;
    color: #18181b;
    font-size: 18px;
    font-weight: 680;
    line-height: 24px;
    overflow-wrap: anywhere;
  }

  .live-pill {
    flex: 0 0 auto;
    padding: 4px 8px;
    border-radius: 999px;
    background: #f4f4f5;
    color: #52525b;
    font-size: 11px;
    font-weight: 650;
    line-height: 14px;
  }

  .live-pill.recording {
    background: #fee2e2;
    color: #991b1b;
  }

  .live-details {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
    margin: 14px 0 0;
  }

  .live-details div {
    min-width: 0;
    padding: 9px 10px;
    border-radius: 6px;
    background: #fafafa;
  }

  .live-details dt {
    color: #71717a;
    font-size: 11px;
    font-weight: 650;
    line-height: 15px;
    text-transform: uppercase;
  }

  .live-details dd {
    margin: 1px 0 0;
    overflow: hidden;
    color: #18181b;
    font-size: 13px;
    font-weight: 650;
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .live-error {
    margin-top: 10px;
    color: #9f1239;
  }

  .live-actions {
    display: flex;
    justify-content: flex-end;
    margin-top: 14px;
  }

  .live-actions button {
    height: 30px;
    padding: 0 12px;
    border: 1px solid #d4d4d8;
    border-radius: 6px;
    background: #ffffff;
    color: #3f3f46;
    font: inherit;
    font-size: 12px;
    font-weight: 650;
    cursor: default;
    transition:
      background 140ms cubic-bezier(.2, .7, .2, 1),
      border-color 140ms cubic-bezier(.2, .7, .2, 1),
      opacity 140ms cubic-bezier(.2, .7, .2, 1),
      transform 140ms cubic-bezier(.2, .7, .2, 1);
  }

  .live-actions button:hover:not(:disabled) {
    border-color: #a1a1aa;
    background: #f4f4f5;
    color: #18181b;
    transform: translateY(-1px);
  }

  .live-actions button:focus-visible {
    outline: 2px solid #2563eb;
    outline-offset: 2px;
  }

  .live-actions button.primary {
    border-color: #27272a;
    background: #27272a;
    color: #fafafa;
  }

  .live-actions button.primary:hover:not(:disabled) {
    border-color: #18181b;
    background: #18181b;
    color: #ffffff;
  }

  .live-actions button:disabled {
    opacity: 0.56;
  }

  @media (prefers-reduced-motion: reduce) {
    .live-actions button {
      transition: none;
    }

    .live-actions button:hover:not(:disabled) {
      transform: none;
    }
  }
</style>
