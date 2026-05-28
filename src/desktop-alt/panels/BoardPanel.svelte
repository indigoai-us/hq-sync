<script lang="ts">
  import BoardCard from '../components/BoardCard.svelte';
  import { useCompanyBoard, type CompanyBoardCard } from '../lib/company-board.svelte';

  interface Props {
    slug: string;
  }

  type BoardMode = 'board' | 'list' | 'timeline';
  type BoardColumn = {
    id: 'inbox' | 'doing' | 'review' | 'done';
    label: string;
  };

  let { slug }: Props = $props();

  let searchOpen = $state(false);
  let searchTerm = $state('');
  let toast = $state('');
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  const boardState = useCompanyBoard({ slug: () => slug });
  const columns: BoardColumn[] = [
    { id: 'inbox', label: 'Inbox' },
    { id: 'doing', label: 'In progress' },
    { id: 'review', label: 'Review' },
    { id: 'done', label: 'Done' },
  ];
  const normalizedSearch = $derived(searchTerm.trim().toLowerCase());

  function cardsFor(column: BoardColumn): CompanyBoardCard[] {
    const cards = boardState.board[column.id] ?? [];
    if (!normalizedSearch) return cards;
    return cards.filter((card) => card.title.toLowerCase().includes(normalizedSearch));
  }

  function selectMode(mode: BoardMode) {
    if (mode === 'board') return;
    showToast('Coming soon');
  }

  function showToast(message: string) {
    toast = message;
    if (toastTimer) {
      clearTimeout(toastTimer);
    }
    toastTimer = setTimeout(() => {
      toast = '';
      toastTimer = null;
    }, 1800);
  }

  function toggleSearch() {
    searchOpen = !searchOpen;
    if (!searchOpen) {
      searchTerm = '';
    }
  }
</script>

<section class="board-panel" aria-labelledby="board-panel-title">
  <header class="board-toolbar">
    <div class="board-title">
      <h2 id="board-panel-title">Board</h2>
      <span>{boardState.loading ? 'Loading cards' : 'Vault board'}</span>
    </div>

    <div class="board-controls" aria-label="Board controls">
      <div class="segment-control" role="tablist" aria-label="Board views">
        <button
          type="button"
          role="tab"
          aria-selected="true"
          class="active"
          onclick={() => selectMode('board')}
        >
          Board
        </button>
        <button
          type="button"
          role="tab"
          aria-selected="false"
          onclick={() => selectMode('list')}
        >
          List
        </button>
        <button
          type="button"
          role="tab"
          aria-selected="false"
          onclick={() => selectMode('timeline')}
        >
          Timeline
        </button>
      </div>

      <button
        class="toolbar-button"
        type="button"
        aria-pressed={searchOpen}
        aria-controls="board-search"
        onclick={toggleSearch}
      >
        Find
      </button>
      <button
        class="toolbar-button"
        type="button"
        disabled
        title="Card creation in next release"
        aria-label="New card. Card creation in next release"
      >
        New
      </button>
    </div>
  </header>

  {#if searchOpen}
    <label class="search-row" for="board-search">
      <span>Find cards</span>
      <input
        id="board-search"
        type="search"
        placeholder="Search by title"
        bind:value={searchTerm}
        aria-label="Search board cards by title"
      />
    </label>
  {/if}

  {#if toast}
    <div class="toast" role="status" aria-live="polite">{toast}</div>
  {/if}

  {#if boardState.error}
    <div class="board-error" role="alert">
      <div>
        <strong>Board unavailable</strong>
        <span>{boardState.error}</span>
      </div>
      <button type="button" onclick={boardState.retry}>Retry</button>
    </div>
  {/if}

  <div class="board-grid" aria-busy={boardState.loading}>
    {#each columns as column (column.id)}
      {@const cards = cardsFor(column)}
      <section class="board-column" aria-labelledby={`board-column-${column.id}`}>
        <header>
          <h3 id={`board-column-${column.id}`}>{column.label}</h3>
          <span aria-label={`${cards.length} cards`}>{cards.length}</span>
        </header>

        <div class="column-cards">
          {#if cards.length > 0}
            {#each cards as card (card.id || card.title)}
              <BoardCard {card} />
            {/each}
          {:else}
            <div class="empty-column" aria-label={`${column.label} is empty`}>—</div>
          {/if}
        </div>
      </section>
    {/each}
  </div>
</section>

<style>
  .board-panel {
    display: grid;
    gap: 14px;
    min-width: 0;
  }

  .board-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    min-width: 0;
  }

  .board-title {
    min-width: 0;
  }

  .board-title h2 {
    margin: 0;
    color: #18181b;
    font-size: 16px;
    font-weight: 680;
    line-height: 22px;
  }

  .board-title span {
    display: block;
    margin-top: 2px;
    color: #71717a;
    font-size: 12px;
    line-height: 16px;
  }

  .board-controls {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .segment-control {
    display: flex;
    flex: 0 0 auto;
    gap: 2px;
    height: 30px;
    padding: 2px;
    border: 1px solid #d4d4d8;
    border-radius: 7px;
    background: #f4f4f5;
  }

  .segment-control button,
  .toolbar-button,
  .board-error button {
    height: 24px;
    min-width: 0;
    border: 0;
    border-radius: 5px;
    font: inherit;
    font-size: 12px;
    font-weight: 650;
    white-space: nowrap;
    cursor: default;
  }

  .segment-control button {
    padding: 0 9px;
    background: transparent;
    color: #71717a;
  }

  .segment-control button.active {
    background: #ffffff;
    color: #18181b;
    box-shadow: 0 1px 2px rgb(24 24 27 / 8%);
  }

  .toolbar-button,
  .board-error button {
    height: 30px;
    padding: 0 11px;
    border: 1px solid #d4d4d8;
    background: #ffffff;
    color: #27272a;
  }

  .toolbar-button:disabled {
    color: #a1a1aa;
    background: #f4f4f5;
  }

  .search-row {
    display: flex;
    align-items: center;
    gap: 9px;
    max-width: 360px;
    min-width: 0;
    color: #71717a;
    font-size: 12px;
    font-weight: 650;
    line-height: 16px;
  }

  .search-row input {
    width: 100%;
    min-width: 0;
    height: 30px;
    padding: 0 10px;
    border: 1px solid #d4d4d8;
    border-radius: 6px;
    background: #ffffff;
    color: #18181b;
    font: inherit;
  }

  .toast {
    justify-self: end;
    max-width: 240px;
    overflow: hidden;
    padding: 6px 10px;
    border: 1px solid #d4d4d8;
    border-radius: 6px;
    background: #27272a;
    color: #fafafa;
    font-size: 12px;
    font-weight: 650;
    line-height: 16px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .board-error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    min-width: 0;
    padding: 12px;
    border: 1px solid #fde68a;
    border-radius: 8px;
    background: #fffbeb;
    color: #854d0e;
  }

  .board-error div {
    display: grid;
    gap: 3px;
    min-width: 0;
  }

  .board-error strong,
  .board-error span {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .board-error strong {
    font-size: 13px;
    line-height: 18px;
  }

  .board-error span {
    font-size: 12px;
    line-height: 16px;
  }

  .board-grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(176px, 1fr));
    gap: 12px;
    min-width: 0;
  }

  .board-column {
    display: grid;
    grid-template-rows: auto minmax(220px, 1fr);
    gap: 10px;
    min-width: 0;
    padding: 10px;
    border: 1px solid #e4e4e7;
    border-radius: 8px;
    background: #f4f4f5;
  }

  .board-column > header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-width: 0;
  }

  .board-column h3 {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: #27272a;
    font-size: 12px;
    font-weight: 700;
    line-height: 16px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .board-column header span {
    flex: 0 0 auto;
    min-width: 22px;
    height: 18px;
    padding: 0 6px;
    border-radius: 999px;
    background: #e4e4e7;
    color: #52525b;
    font-size: 11px;
    font-weight: 650;
    line-height: 18px;
    text-align: center;
  }

  .column-cards {
    display: grid;
    align-content: start;
    gap: 8px;
    min-width: 0;
  }

  .empty-column {
    display: grid;
    min-height: 92px;
    place-items: center;
    border: 1px dashed #d4d4d8;
    border-radius: 8px;
    color: #a1a1aa;
    font-size: 18px;
    line-height: 24px;
  }

  @media (max-width: 1060px) {
    .board-grid {
      grid-template-columns: repeat(2, minmax(176px, 1fr));
    }
  }

  @media (max-width: 680px) {
    .board-toolbar,
    .board-controls {
      align-items: stretch;
      flex-direction: column;
    }

    .board-controls,
    .segment-control,
    .toolbar-button {
      width: 100%;
    }

    .segment-control button {
      flex: 1 1 0;
    }

    .board-grid {
      grid-template-columns: minmax(0, 1fr);
    }

    .board-error {
      align-items: stretch;
      flex-direction: column;
    }
  }
</style>
