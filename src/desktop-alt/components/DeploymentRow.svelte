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

  let expanded = $state(false);

  const stateLabel = $derived(deployment.state.charAt(0).toUpperCase() + deployment.state.slice(1));
  const detailId = $derived(`deploy-detail-${deployment.sub}`);

  async function openDeployment() {
    await open(`https://${deployment.url}`);
  }

  function toggleDetail() {
    expanded = !expanded;
  }
</script>

<div class="deployment-row" class:is-open={expanded} aria-label={`${deployment.sub} deployment`}>
  <span class={`status-dot ${deployment.state}`} title={stateLabel} aria-label={stateLabel}></span>

  <button
    class="subdomain-cell"
    type="button"
    aria-expanded={expanded}
    aria-controls={detailId}
    title={`Show ${deployment.sub} deployment detail`}
    onclick={toggleDetail}
  >
    <span class="subdomain" title={deployment.sub}>{deployment.sub}</span>
    {#if deployment.pwd}
      <span class="lock-icon" title="Password locked" aria-label="Password locked"></span>
    {/if}
    <span class={`disclosure ${expanded ? 'open' : ''}`} aria-hidden="true"></span>
    <span class="url" title={deployment.url}>{deployment.url}</span>
  </button>

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
      class:is-active={expanded}
      type="button"
      title="Show detail"
      aria-label={`Show detail for ${deployment.sub}`}
      aria-expanded={expanded}
      aria-controls={detailId}
      onclick={toggleDetail}
    >
      <span class="more-icon" aria-hidden="true"></span>
    </button>
  </div>
</div>

{#if expanded}
  <div class="deployment-detail" id={detailId} role="region" aria-label={`${deployment.sub} deployment detail`}>
    <dl class="detail-grid">
      <div class="detail-field">
        <dt>Status</dt>
        <dd>
          <span class={`status-dot ${deployment.state}`} aria-hidden="true"></span>
          {stateLabel}
        </dd>
      </div>
      <div class="detail-field">
        <dt>Last deploy</dt>
        <dd>{deployment.lastDeploy}</dd>
      </div>
      <div class="detail-field">
        <dt>Size</dt>
        <dd class="mono">{deployment.size}</dd>
      </div>
      <div class="detail-field">
        <dt>Version</dt>
        <dd class="mono">{deployment.ver}</dd>
      </div>
      <div class="detail-field">
        <dt>URL</dt>
        <dd>
          <button class="detail-link" type="button" onclick={openDeployment}>
            {deployment.url}
          </button>
        </dd>
      </div>
      <div class="detail-field">
        <dt>Access</dt>
        <dd>{deployment.pwd ? 'Password protected' : 'Public'}</dd>
      </div>
    </dl>

    <p class="detail-note">
      Managed via <code>hq-deploy</code> — run <code>/deploy</code> from your terminal to redeploy.
    </p>
  </div>
{/if}

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

  .deployment-row.is-open {
    background: var(--row-hover);
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
    grid-template-columns: minmax(0, auto) auto auto;
    align-items: center;
    justify-content: start;
    gap: 5px 7px;
    min-width: 0;
    margin: -4px -6px;
    padding: 4px 6px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .subdomain-cell:hover {
    background: var(--row-active);
  }

  .subdomain-cell:focus-visible,
  .detail-link:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .disclosure {
    width: 6px;
    height: 6px;
    border-right: 1.5px solid var(--muted);
    border-bottom: 1.5px solid var(--muted);
    transform: rotate(-45deg);
    transition: transform 0.12s ease;
  }

  .disclosure.open {
    transform: rotate(45deg);
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
    font-size: var(--text-base);
    font-weight: 680;
    line-height: 18px;
  }

  .url {
    grid-column: 1 / -1;
    color: var(--muted);
    font-size: var(--text-base);
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
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 16px;
  }

  .size,
  .version {
    font-family: var(--font-mono);
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
    font-size: var(--text-base);
    font-weight: 700;
    white-space: nowrap;
    cursor: pointer;
  }

  .icon-button:hover:not(:disabled) {
    border-color: var(--border-strong);
    background: var(--row-hover);
  }

  .icon-button:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .icon-button.is-active {
    border-color: var(--border-strong);
    background: var(--row-active);
  }

  .icon-button:disabled {
    color: var(--muted-3);
    background: var(--row-hover);
    cursor: default;
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

  .deployment-detail {
    display: grid;
    gap: 12px;
    padding: 12px 13px 14px;
    border-top: 1px solid var(--border);
    background: var(--bg-subtle);
  }

  .detail-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    gap: 10px 18px;
    margin: 0;
  }

  .detail-field {
    display: grid;
    gap: 3px;
    min-width: 0;
  }

  .detail-field dt {
    color: var(--muted);
    font-size: var(--text-base);
    font-weight: 700;
    line-height: 14px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
  }

  .detail-field dd {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    margin: 0;
    overflow-wrap: anywhere;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 16px;
  }

  .detail-field dd.mono {
    font-family: var(--font-mono);
  }

  .detail-field .status-dot {
    justify-self: start;
  }

  .detail-link {
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--blue);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 600;
    text-align: left;
    overflow-wrap: anywhere;
    cursor: pointer;
  }

  .detail-link:hover {
    text-decoration: underline;
  }

  .detail-note {
    margin: 0;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
  }

  .detail-note code {
    padding: 1px 4px;
    border-radius: 4px;
    background: var(--row-hover);
    color: var(--muted-2);
    font-family: var(--font-mono);
    font-size: var(--text-base);
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
