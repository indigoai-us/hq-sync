import { invoke } from '@tauri-apps/api/core';

/**
 * Shape of a single entry in `<hqRoot>/.hq-conflicts/index.json`, as recorded
 * by the sync engine (@indigoai-us/hq-cloud) on a conflict.
 *
 * As of the conflict-versioning change (hq-cloud US-001), the engine NO LONGER
 * writes a sibling `<file>.conflict-<ts>-<machine>.<ext>` mirror file — S3
 * versioning is the safety net instead. Consequently `conflictPath` is gone
 * from this entry; consumers must not read it.
 */
export interface ConflictIndexEntry {
  /** Stable identifier for the conflict record. */
  id: string;
  /** Repo-relative path of the file that conflicted (the real file on disk). */
  originalPath: string;
  /** ISO timestamp the conflict was detected. */
  detectedAt: string;
  /** Which leg of the sync surfaced the conflict. */
  side: 'push' | 'pull';
  /** Machine that recorded the conflict. */
  machineId: string;
  /** Content hash of the local copy at detection time. */
  localHash: string;
  /** Content hash of the remote copy at detection time. */
  remoteHash: string;
  /** S3 version id of the remote object, when the engine captured one. */
  remoteVersionId?: string;
}

/**
 * UI-facing conflict model. Derived from {@link ConflictIndexEntry}; carries
 * the resolution lifecycle state the components render. Keyed by `path`
 * (the original on-disk path) — there is no separate mirror path anymore.
 */
export interface ConflictFile {
  /** Repo-relative original path (was `originalPath` in the index entry). */
  path: string;
  /** ISO timestamp the conflict was detected, surfaced in the UI. */
  detectedAt?: string;
  /** Which sync leg surfaced it (`push`/`pull`), surfaced in the UI. */
  side?: 'push' | 'pull';
  /** Machine that recorded the conflict. */
  machineId?: string;
  localHash: string;
  remoteHash: string;
  /** S3 version id of the remote copy, when available. */
  remoteVersionId?: string;
  status: 'pending' | 'resolving' | 'resolved' | 'error';
  resolution?: 'keep-local' | 'keep-remote';
  error?: string;
}

/** Map a raw index entry (new engine shape) into the UI-facing model. */
export function fromIndexEntry(entry: ConflictIndexEntry): Omit<ConflictFile, 'status'> {
  return {
    path: entry.originalPath,
    detectedAt: entry.detectedAt,
    side: entry.side,
    machineId: entry.machineId,
    localHash: entry.localHash,
    remoteHash: entry.remoteHash,
    remoteVersionId: entry.remoteVersionId,
  };
}

class ConflictStore {
  private _conflicts: ConflictFile[] = [];
  private _listeners: Set<() => void> = new Set();

  get conflicts(): ConflictFile[] {
    return this._conflicts;
  }

  get pending(): ConflictFile[] {
    return this._conflicts.filter((c) => c.status === 'pending');
  }

  get allResolved(): boolean {
    return (
      this._conflicts.length > 0 &&
      this._conflicts.every((c) => c.status === 'resolved')
    );
  }

  get hasConflicts(): boolean {
    return this._conflicts.some((c) => c.status !== 'resolved');
  }

  get count(): number {
    return this._conflicts.length;
  }

  subscribe(fn: () => void) {
    this._listeners.add(fn);
    return () => this._listeners.delete(fn);
  }

  private notify() {
    this._listeners.forEach((fn) => fn());
  }

  /**
   * Add a conflict from a raw index entry (new engine shape). Deduplicated by
   * original path so the same file surfacing twice in a run is shown once.
   */
  addConflict(entry: ConflictIndexEntry) {
    const conflict = fromIndexEntry(entry);
    if (this._conflicts.some((c) => c.path === conflict.path)) return;
    this._conflicts = [
      ...this._conflicts,
      { ...conflict, status: 'pending' as const },
    ];
    this.notify();
  }

  async resolveConflict(
    path: string,
    strategy: 'keep-local' | 'keep-remote'
  ) {
    const conflict = this._conflicts.find((c) => c.path === path);
    if (!conflict || conflict.status !== 'pending') return;
    this.updateStatus(path, 'resolving');
    try {
      await invoke('resolve_conflict', { path, strategy });
      this.updateStatus(path, 'resolved', strategy);
    } catch (e) {
      this.updateStatus(path, 'error', undefined, String(e));
    }
  }

  async resolveAll(strategy: 'keep-local' | 'keep-remote') {
    const pendingPaths = this.pending.map((c) => c.path);
    for (const path of pendingPaths) {
      await this.resolveConflict(path, strategy);
    }
  }

  clear() {
    this._conflicts = [];
    this.notify();
  }

  private updateStatus(
    path: string,
    status: ConflictFile['status'],
    resolution?: ConflictFile['resolution'],
    error?: string
  ) {
    this._conflicts = this._conflicts.map((c) =>
      c.path === path ? { ...c, status, resolution, error } : c
    );
    this.notify();
  }
}

export const conflictStore = new ConflictStore();
