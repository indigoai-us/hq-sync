import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { emptyCompanyBoard } from '../../src/desktop-alt/lib/company-board.svelte';

const companyPage = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/pages/CompanyPage.svelte'),
  'utf8',
);
const boardPanel = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/panels/BoardPanel.svelte'),
  'utf8',
);
const boardCard = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/BoardCard.svelte'),
  'utf8',
);
const companyBoard = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/lib/company-board.svelte.ts'),
  'utf8',
);
const tauriMain = readFileSync(resolve(process.cwd(), 'src-tauri/src/main.rs'), 'utf8');

function normalize(source: string): string {
  return source.replace(/\s+/g, ' ');
}

describe('US-009: Board panel reads vault board.json via Tauri command', () => {
  it('wires the board tab to get_company_board with the selected company slug', () => {
    const page = normalize(companyPage);
    const store = normalize(companyBoard);

    expect(page).toContain("import BoardPanel from '../panels/BoardPanel.svelte'");
    expect(page).toContain('<BoardPanel slug={company.slug} />');
    expect(store).toContain("void invoke<CompanyBoard>('get_company_board', { slug })");
    expect(store).toContain('return () => { cancelled = true; };');
    expect(tauriMain).toContain('commands::desktop_alt::get_company_board');
  });

  it('renders four prototype columns with empty placeholders and card metadata', () => {
    const panel = normalize(boardPanel);
    const card = normalize(boardCard);

    expect(emptyCompanyBoard()).toEqual({ inbox: [], doing: [], review: [], done: [] });
    expect(panel).toContain("{ id: 'inbox', label: 'Inbox' }");
    expect(panel).toContain("{ id: 'doing', label: 'In progress' }");
    expect(panel).toContain("{ id: 'review', label: 'Review' }");
    expect(panel).toContain("{ id: 'done', label: 'Done' }");
    expect(panel).toContain('{#each columns as column (column.id)}');
    expect(panel).toContain('function cardKey(column: BoardColumn, card: CompanyBoardCard, index: number): string');
    expect(panel).toContain('{#each cards as card, index (cardKey(column, card, index))}');
    expect(panel).not.toContain('{#each cards as card (card.id || card.title)}');
    expect(panel).toContain('<div class="empty-column" aria-label={`${column.label} is empty`}>—</div>');
    expect(card).toContain('card.assigneeInitials?.trim()');
    expect(card).toContain("card.tag?.trim() || card.labels?.find((label) => label.trim()) || 'Untagged'");
    expect(card).toContain("card.age?.trim() || card.subtitle?.trim() || 'New'");
  });

  it('keeps v1 toolbar behavior scoped to Board with Find filtering and disabled New', () => {
    const panel = normalize(boardPanel);

    expect(panel).toContain("type BoardMode = 'board' | 'list' | 'timeline'");
    expect(panel).toContain("if (mode === 'board') return; showToast('Coming soon');");
    expect(panel).toContain('aria-selected="true" class="active"');
    expect(panel).toContain('aria-controls="board-search"');
    expect(panel).toContain('bind:value={searchTerm}');
    expect(panel).toContain('card.title.toLowerCase().includes(normalizedSearch)');
    expect(panel).toContain('title="Card creation in next release"');
    expect(panel).toContain('disabled');
  });

  it('renders inline command errors with retry instead of crashing the page', () => {
    const panel = normalize(boardPanel);
    const store = normalize(companyBoard);

    expect(store).toContain("console.error('get_company_board failed:', err)");
    expect(store).toContain('retry() { reloadToken += 1; }');
    expect(panel).toContain('{#if boardState.error}');
    expect(panel).toContain('<div class="board-error" role="alert">');
    expect(panel).toContain('<button type="button" onclick={boardState.retry}>Retry</button>');
  });
});
