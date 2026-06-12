<script lang="ts">
  import type { HomeDigestGroup } from './home-model';
  import './tokens.css';

  /**
   * V4 actor-grouped activity digest — "Today across your companies"
   * (home-healthy.png). Each group is a card: avatar initials, a narrative
   * headline ("Geoff added 2 files to hpo"), an 11px meta line, and a
   * chevron toggle that expands per-file rows. File verb lanes (ADD/UPD/DEL)
   * are gray text-2 — NOT colored (story AC; SPEC: status colors are dots).
   * The quiet "raw event log →" link opens the Recent Changes window.
   */
  interface Props {
    groups: HomeDigestGroup[];
    onopenlog?: () => void;
  }

  let { groups, onopenlog }: Props = $props();

  // Newest group starts expanded (matches home-healthy.png); the rest are
  // collapsed until clicked. Keyed by group id so toggles survive reorders.
  let expanded = $state<Record<string, boolean>>({});

  function isExpanded(id: string, index: number): boolean {
    return expanded[id] ?? index === 0;
  }

  function toggle(id: string, index: number) {
    expanded = { ...expanded, [id]: !isExpanded(id, index) };
  }
</script>

<section class="v4-digest" aria-label="Today across your companies">
  <div class="v4-digest-header">
    <h2 class="v4-digest-title">Today across your companies</h2>
    <p class="v4-digest-tools">
      grouped by person ·
      <button type="button" class="v4-digest-log" onclick={() => onopenlog?.()}>
        raw event log →
      </button>
    </p>
  </div>

  {#if groups.length === 0}
    <div class="v4-digest-empty">
      <p>Nothing yet today — activity appears here after files sync.</p>
    </div>
  {:else}
    <ol class="v4-digest-list">
      {#each groups as group, index (group.id)}
        <li class="v4-digest-group">
          <button
            type="button"
            class="v4-digest-head"
            aria-expanded={isExpanded(group.id, index)}
            onclick={() => toggle(group.id, index)}
          >
            <span class="v4-avatar" aria-hidden="true">{group.initials}</span>
            <span class="v4-digest-copy">
              <span class="v4-digest-headline">{group.headline}</span>
              <span class="v4-digest-meta">{group.meta}</span>
            </span>
            <span class="v4-chevron" class:open={isExpanded(group.id, index)} aria-hidden="true">
              ›
            </span>
          </button>
          {#if isExpanded(group.id, index)}
            <ol class="v4-file-list">
              {#each group.files as file (`${file.path}:${file.at}`)}
                <li class="v4-file-row">
                  <span class="v4-file-verb">{file.verb}</span>
                  <span class="v4-file-path">{file.path}</span>
                  <span class="v4-file-size">{file.sizeLabel}</span>
                </li>
              {/each}
            </ol>
          {/if}
        </li>
      {/each}
    </ol>
  {/if}
</section>

<style>
  .v4-digest {
    display: grid;
    gap: 10px;
  }

  .v4-digest-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }

  .v4-digest-title {
    margin: 0;
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .v4-digest-tools {
    margin: 0;
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 400;
  }

  .v4-digest-log {
    padding: 0;
    border: none;
    background: none;
    color: var(--v4-text-3);
    font: inherit;
    cursor: pointer;
  }

  .v4-digest-log:hover {
    color: var(--v4-text-2);
  }

  .v4-digest-empty {
    padding: 14px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-raised);
  }

  .v4-digest-empty p {
    margin: 0;
    color: var(--v4-text-3);
    font-size: 13px;
  }

  .v4-digest-list {
    display: grid;
    gap: 8px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .v4-digest-group {
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-raised);
    overflow: hidden;
  }

  .v4-digest-head {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 11px 14px;
    border: none;
    background: transparent;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .v4-digest-head:hover {
    background: var(--v4-control-faint);
  }

  .v4-avatar {
    display: grid;
    flex: 0 0 26px;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    background: var(--v4-control-faint);
    color: var(--v4-text-2);
    font-size: 11px;
    font-weight: 500;
    line-height: 1;
    place-items: center;
  }

  .v4-digest-copy {
    display: grid;
    gap: 3px;
    min-width: 0;
    flex: 1 1 auto;
  }

  .v4-digest-headline {
    overflow: hidden;
    color: var(--v4-text-1);
    font-size: 13px;
    font-weight: 400;
    line-height: 1.3;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .v4-digest-meta {
    overflow: hidden;
    color: var(--v4-text-3);
    font-size: 11px;
    line-height: 1.3;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .v4-chevron {
    flex: 0 0 auto;
    color: var(--v4-text-3);
    font-size: 13px;
    line-height: 1;
    transition: transform 120ms ease;
  }

  .v4-chevron.open {
    transform: rotate(90deg);
  }

  @media (prefers-reduced-motion: reduce) {
    .v4-chevron {
      transition: none;
    }
  }

  .v4-file-list {
    margin: 0;
    padding: 0 0 4px;
    list-style: none;
  }

  .v4-file-row {
    display: flex;
    align-items: baseline;
    gap: 12px;
    padding: 7px 14px 7px 52px;
    border-top: 1px solid var(--v4-rowline);
  }

  /* Verb lane — gray text, NOT colored (story AC). */
  .v4-file-verb {
    flex: 0 0 30px;
    color: var(--v4-text-2);
    font-size: 11px;
    font-weight: 400;
    letter-spacing: 0.04em;
  }

  .v4-file-path {
    overflow: hidden;
    min-width: 0;
    flex: 1 1 auto;
    color: var(--v4-text-1);
    font-size: 13px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .v4-file-size {
    flex: 0 0 auto;
    color: var(--v4-text-3);
    font-size: 11px;
  }
</style>
