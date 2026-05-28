<script lang="ts" module>
  export type DeploymentState = 'active' | 'deploying' | 'paused';

  export interface DeploymentEntry {
    sub: string;
    url: string;
    state: DeploymentState;
    lastDeploy: string;
    size: string;
    ver: string;
    pwd: boolean;
  }
</script>

<script lang="ts">
  import { open } from '@tauri-apps/plugin-shell';

  interface Props {
    deployment: DeploymentEntry;
  }

  let { deployment }: Props = $props();

  const stateLabel = $derived(deployment.state.charAt(0).toUpperCase() + deployment.state.slice(1));

  async function openDeployment() {
    await open(`https://${deployment.url}`);
  }
</script>

<div class="deployment-row" aria-label={`${deployment.sub} deployment`}>
  <span class={`status-dot ${deployment.state}`} title={stateLabel} aria-label={stateLabel}></span>

  <div class="subdomain-cell">
    <span class="subdomain" title={deployment.sub}>{deployment.sub}</span>
    {#if deployment.pwd}
      <span class="lock-icon" title="Password locked" aria-label="Password locked"></span>
    {/if}
    <span class="url" title={deployment.url}>{deployment.url}</span>
  </div>

  <time class="last-deploy" title={deployment.lastDeploy}>{deployment.lastDeploy}</time>
  <span class="size" title={deployment.size}>{deployment.size}</span>
  <span class="version" title={deployment.ver}>{deployment.ver}</span>

  <div class="row-actions">
    <button
      class="icon-button"
      type="button"
      title="Open in browser"
      aria-label={`Open ${deployment.sub} in browser`}
      onclick={openDeployment}
    >
      <span class="open-icon" aria-hidden="true"></span>
    </button>
    <button
      class="icon-button more-button"
      type="button"
      title="More"
      aria-label={`More actions for ${deployment.sub}`}
      disabled
    >
      <span class="more-icon" aria-hidden="true"></span>
    </button>
  </div>
</div>

<style>
  .deployment-row {
    display: grid;
    grid-template-columns: 14px 1.4fr 1fr auto auto auto;
    align-items: center;
    gap: 12px;
    min-width: 0;
    padding: 10px 13px;
    border-top: 1px solid var(--border);
  }

  .deployment-row:first-child {
    border-top: 0;
  }

  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--muted-2);
    justify-self: center;
  }

  .status-dot.active {
    background: var(--emerald);
  }

  .status-dot.deploying {
    background: var(--blue);
    animation: pulse 1.4s ease-in-out infinite;
  }

  .status-dot.paused {
    background: var(--amber);
  }

  .subdomain-cell {
    display: grid;
    grid-template-columns: minmax(0, auto) auto;
    align-items: center;
    justify-content: start;
    gap: 5px 7px;
    min-width: 0;
  }

  .subdomain,
  .url,
  .last-deploy,
  .size,
  .version {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .subdomain {
    color: var(--fg);
    font-size: 13px;
    font-weight: 680;
    line-height: 18px;
  }

  .url {
    grid-column: 1 / -1;
    color: var(--muted);
    font-size: 11px;
    line-height: 15px;
  }

  .lock-icon {
    position: relative;
    width: 11px;
    height: 10px;
    border: 1.5px solid var(--muted);
    border-radius: 2px;
  }

  .lock-icon::before {
    position: absolute;
    left: 1px;
    top: -7px;
    width: 7px;
    height: 7px;
    border: 1.5px solid var(--muted);
    border-bottom: 0;
    border-radius: 7px 7px 0 0;
    content: '';
  }

  .last-deploy,
  .size,
  .version {
    color: var(--muted-3);
    font-size: 12px;
    font-weight: 600;
    line-height: 16px;
  }

  .size,
  .version {
    font-family: 'Geist Mono', ui-monospace, SFMono-Regular, monospace;
  }

  .row-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .icon-button {
    display: grid;
    place-items: center;
    width: 34px;
    height: 28px;
    overflow: hidden;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: 10px;
    font-weight: 700;
    white-space: nowrap;
    cursor: default;
  }

  .icon-button:hover:not(:disabled) {
    border-color: var(--border-strong);
    background: var(--row-hover);
  }

  .icon-button:disabled {
    color: var(--muted-3);
    background: var(--row-hover);
  }

  .open-icon {
    position: relative;
    width: 12px;
    height: 12px;
    border: 1.5px solid currentcolor;
    border-radius: 2px;
  }

  .open-icon::before {
    position: absolute;
    top: -3px;
    right: -4px;
    width: 7px;
    height: 7px;
    border-top: 1.5px solid currentcolor;
    border-right: 1.5px solid currentcolor;
    content: '';
  }

  .open-icon::after {
    position: absolute;
    top: -2px;
    right: -3px;
    width: 10px;
    height: 1.5px;
    background: currentcolor;
    content: '';
    transform: rotate(-45deg);
    transform-origin: right center;
  }

  .more-icon,
  .more-icon::before,
  .more-icon::after {
    width: 3px;
    height: 3px;
    border-radius: 999px;
    background: currentcolor;
  }

  .more-icon {
    position: relative;
  }

  .more-icon::before,
  .more-icon::after {
    position: absolute;
    top: 0;
    content: '';
  }

  .more-icon::before {
    left: -6px;
  }

  .more-icon::after {
    right: -6px;
  }

  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
      transform: scale(1);
    }

    50% {
      opacity: 0.45;
      transform: scale(1.28);
    }
  }

  @media (max-width: 760px) {
    .deployment-row {
      grid-template-columns: 14px minmax(0, 1fr) auto;
      gap: 8px;
    }

    .last-deploy,
    .size,
    .version {
      display: none;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .status-dot.deploying {
      animation: none;
    }
  }
</style>
