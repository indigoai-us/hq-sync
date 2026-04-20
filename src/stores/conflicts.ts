import { invoke } from '@tauri-apps/api/core';

export interface ConflictFile {
  path: string;
  localHash: string;
  remoteHash: string;
  canAutoResolve: boolean;
  status: 'pending' | 'resolving' | 'resolved' | 'error';
  resolution?: 'keep-local' | 'keep-remote';
  error?: string;
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

  addConflict(conflict: {
    path: string;
    localHash: string;
    remoteHash: string;
    canAutoResolve: boolean;
  }) {
    // Deduplicate by path
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

  async openInEditor(path: string) {
    try {
      await invoke('open_in_editor', { path });
    } catch (e) {
      console.error('Failed to open in editor:', e);
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
