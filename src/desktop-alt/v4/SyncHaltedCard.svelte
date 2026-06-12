<script lang="ts">
  import './tokens.css';

  interface Props {
    reason: string;
    onabort: () => void;
  }

  let { reason, onabort }: Props = $props();
</script>

<article class="sync-halted-card" aria-label="Sync halted">
  <header>
    <span>SYNC HALTED</span>
    <strong>Sync stopped before changing more files.</strong>
  </header>
  <p>{reason}</p>
  <p class="guidance">Review the issue, then start a fresh sync when you are ready.</p>
  <footer>
    <button type="button" onclick={onabort}>Abort sync</button>
  </footer>
</article>

<style>
  .sync-halted-card {
    display: grid;
    gap: 10px;
    min-width: 0;
    padding: 14px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
    color: var(--v4-text-1);
  }

  header {
    display: grid;
    gap: 4px;
    min-width: 0;
  }

  header span {
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 500;
  }

  header strong {
    font-size: 14px;
    font-weight: 500;
    line-height: 1.3;
  }

  p {
    margin: 0;
    color: var(--v4-text-2);
    font-size: 12px;
    line-height: 1.4;
  }

  .guidance {
    color: var(--v4-text-3);
  }

  footer {
    display: flex;
    justify-content: flex-end;
  }

  button {
    height: 30px;
    padding: 0 11px;
    border: 1px solid var(--v4-hairline);
    border-radius: 6px;
    background: transparent;
    color: var(--v4-text-2);
    font: inherit;
    font-size: 12px;
  }
</style>
