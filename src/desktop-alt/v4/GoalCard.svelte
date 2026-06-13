<script lang="ts">
  import type { KeyResult, Objective } from '../lib/local-projects';
  import './tokens.css';

  interface Props {
    objective: Objective;
    progress: number;
    projectCount: number;
    storyCount: number;
  }

  let { objective, progress, projectCount, storyCount }: Props = $props();

  const status = $derived(goalStatus(objective.status));
  const krLine = $derived(keyResultLine(objective.keyResults));
  const title = $derived(objective.title || 'Untitled goal');

  function goalStatus(raw: string): { label: string; tone: 'ok' | 'warn' | 'error' | 'idle' } {
    const normalized = raw.toLowerCase().replace(/[_\s]+/g, '-').trim();
    if (normalized === 'on-track' || normalized === 'active' || normalized === 'running') {
      return { label: 'ON TRACK', tone: 'ok' };
    }
    if (normalized === 'at-risk' || normalized === 'review') {
      return { label: 'AT RISK', tone: 'warn' };
    }
    if (normalized === 'off-track' || normalized === 'blocked') {
      return { label: 'OFF TRACK', tone: 'error' };
    }
    return { label: normalized ? normalized.replace(/-/g, ' ').toUpperCase() : 'NO STATUS', tone: 'idle' };
  }

  function valueText(value: number | string | null | undefined, unit: string | undefined): string {
    if (value == null || value === '') return '';
    return `${value}${unit ?? ''}`;
  }

  function keyResultLabel(kr: KeyResult): string {
    const name = kr.title || kr.metric || 'Key result';
    const current = valueText(kr.current, kr.unit);
    const target = valueText(kr.target, kr.unit);
    if (current && target) return `${name} ${current}->${target}`;
    if (target) return `${name} ->${target}`;
    return name;
  }

  function keyResultLine(results: KeyResult[]): string {
    const visible = results.map(keyResultLabel).filter(Boolean);
    if (visible.length > 0) return visible.slice(0, 3).join(' · ');
    return objective.description || 'No key result defined yet';
  }
</script>

<article class="goal-card" data-testid="goal-card">
  <header class="goal-header">
    <h3>{title}</h3>
    <span class="goal-status">
      <span class={`status-dot ${status.tone}`} aria-hidden="true"></span>
      <span>{status.label}</span>
    </span>
  </header>

  <p class="goal-kr">{krLine}</p>

  <div class="goal-progress" aria-label={`${progress}% progress`}>
    <span class="progress-track" aria-hidden="true">
      <span class="progress-fill" style={`width: ${progress}%`}></span>
    </span>
    <span class="progress-number">{progress}%</span>
  </div>

  <p class="goal-counts">
    {projectCount} {projectCount === 1 ? 'project' : 'projects'} · {storyCount}
    {storyCount === 1 ? ' story' : ' stories'} in flight
  </p>
</article>

<style>
  .goal-card {
    display: flex;
    min-width: 0;
    min-height: 118px;
    flex-direction: column;
    gap: 9px;
    padding: 16px;
    border: 1px solid var(--v4-rowline);
    border-radius: 8px;
    background: var(--v4-raised);
    color: var(--v4-text-1);
    font-family:
      'Inter Variable',
      Inter,
      -apple-system,
      'SF Pro Text',
      sans-serif;
  }

  .goal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    min-width: 0;
  }

  h3 {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: var(--v4-text-1);
    font-size: var(--text-base);
    font-weight: 500;
    line-height: 1.25;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .goal-status {
    display: inline-flex;
    flex: 0 0 auto;
    align-items: center;
    gap: 7px;
    color: var(--v4-text-2);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.2;
  }

  .status-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .status-dot.ok {
    background: var(--v4-ok);
  }

  .status-dot.warn {
    background: var(--v4-warn);
  }

  .status-dot.error {
    background: var(--v4-error);
  }

  .status-dot.idle {
    background: var(--v4-idle);
  }

  .goal-kr {
    margin: 0;
    overflow: hidden;
    color: var(--v4-text-2);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.35;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .goal-progress {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 12px;
    min-width: 0;
    margin-top: auto;
  }

  .progress-track {
    height: 3px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--v4-control-faint);
  }

  .progress-fill {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--v4-text-2);
  }

  .progress-number,
  .goal-counts {
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1.25;
    font-variant-numeric: tabular-nums;
  }

  .goal-counts {
    margin: 0;
  }
</style>
