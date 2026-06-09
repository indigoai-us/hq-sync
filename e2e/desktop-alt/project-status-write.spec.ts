import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

/**
 * US-010 — Project status writes (local persist + optimistic UI).
 *
 * Source-contract style (matching the desktop-alt harness): assert that the
 * status dropdown is now WRITABLE — selecting a status calls the projects-store
 * write with optimistic-paint + rollback, the store invokes the registered Rust
 * write command, and the new command is registered + capability-allowed.
 */

describe('desktop-alt project status write — store contract (US-010)', () => {
  const store = readRepoFile('src/desktop-alt/lib/projects-store.svelte.ts');
  const adapter = readRepoFile('src/desktop-alt/lib/local-projects.ts');

  it('the store invokes the registered Rust write commands', () => {
    // The adapter is the single place that calls the Tauri write commands, with
    // the camelCased args Tauri v2 exposes.
    expect(adapter).toContain("invoke('set_local_project_status', { boardPath, projectId, status })");
    expect(adapter).toContain("invoke('set_local_story_passes', { prdPath, storyId, passes })");
    // The store routes status writes through that adapter.
    expect(store).toContain('saveLocalProjectStatus');
  });

  it('applies the change optimistically and rolls back on failure', () => {
    // Optimistic: the overlay is set to `next` BEFORE awaiting the write.
    expect(store).toContain('statusOverride.set(key, next)');
    // The write is awaited after the optimistic set.
    const optimisticIdx = store.indexOf('statusOverride.set(key, next)');
    const awaitIdx = store.indexOf('await saveLocalProjectStatus');
    expect(optimisticIdx).toBeGreaterThan(-1);
    expect(awaitIdx).toBeGreaterThan(optimisticIdx);
    // Rollback: on catch, the overlay is restored to `previous` and a clear,
    // user-facing error is returned (not a raw thrown error).
    expect(store).toContain('statusOverride.set(key, previous)');
    expect(store).toContain('Could not save the status change');
    // Board path is derived from companies/<company>/board.json.
    expect(store).toContain('companies/${company}/board.json');
  });

  it('exposes the story-passes optimistic toggle too', () => {
    expect(store).toContain('export async function setStoryPasses');
    expect(store).toContain('passesOverride.set(key, next)');
    expect(store).toContain('passesOverride.set(key, previous)');
  });
});

describe('desktop-alt status dropdown wires onStatusChange → write (US-010)', () => {
  const detail = readRepoFile('src/desktop-alt/pages/ProjectDetailView.svelte');
  // The detail view's status writes are hosted by the per-company board panel
  // (US-011) now that the top-level BoardPage is gone.
  const board = readRepoFile('src/desktop-alt/panels/CompanyBoardPanel.svelte');

  it('selecting a status calls the store write through an onclick handler', () => {
    // The dropdown options now call selectStatus (was a no-op menu-close in 009).
    expect(detail).toContain('onclick={() => selectStatus(status)}');
    expect(detail).toContain('async function selectStatus');
    expect(detail).toContain("import { setProjectStatus } from '../lib/projects-store.svelte'");
    expect(detail).toContain('await setProjectStatus(');
  });

  it('paints optimistically and rolls back the rendered status on failure', () => {
    // Local override drives the rendered status (optimistic), defaulting to the
    // raw project status.
    expect(detail).toContain('statusOverride ?? toEditableStatus(project.status)');
    // Optimistic set before await; rollback + error surface on a failed result.
    expect(detail).toContain('statusOverride = next');
    expect(detail).toContain('statusOverride = previous');
    expect(detail).toContain('statusError = result.error');
    expect(detail).toContain('data-testid="status-error"');
  });

  it('notifies the board via onStatusChange so the list row refreshes', () => {
    expect(detail).toContain('onStatusChange?.(project.id, next)');
    expect(board).toContain('onStatusChange={onProjectStatusChange}');
    expect(board).toContain('function onProjectStatusChange');
  });
});

describe('desktop-alt status write — registration + capability (US-010)', () => {
  it('registers the write commands in main.rs', () => {
    const main = readRepoFile('src-tauri/src/main.rs');
    expect(main).toContain('commands::projects_local::set_local_project_status');
    expect(main).toContain('commands::projects_local::set_local_story_passes');
  });

  it('documents the write commands in the desktop-alt capability', () => {
    const cap = readRepoFile('src-tauri/capabilities/desktop-alt.json');
    expect(cap).toContain('set_local_project_status');
    expect(cap).toContain('set_local_story_passes');
  });

  it('the Rust write command guards path + GA gate + atomic write', () => {
    const rust = readRepoFile('src-tauri/src/commands/projects_local.rs');
    expect(rust).toContain('pub async fn set_local_project_status');
    expect(rust).toContain('desktop_features_enabled().await');
    // Path-traversal guard + board.json-only target.
    expect(rust).toContain('is_within(hq_root, &abs)');
    expect(rust).toContain('Some("board.json")');
    // Atomic write: serialize → temp → rename.
    expect(rust).toContain('fn atomic_write_json');
    expect(rust).toContain('std::fs::rename(&tmp_path, target)');
  });
});
