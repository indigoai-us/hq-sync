// Mirrors src-tauri/src/commands/workspaces.rs::Workspace.
// Returned by the Rust `list_syncable_workspaces` Tauri command and rendered
// by the menubar's WorkspaceList component.
export interface Workspace {
  slug: string;
  displayName: string;
  kind: 'personal' | 'company';
  // 'broken' = manifest declares cloud_uid that doesn't match cloud reality
  // (different UID, or no membership for slug). User can hit Connect to
  // reconcile — only surfaced when cloudReachable is true.
  state: 'personal' | 'synced' | 'cloud-only' | 'local-only' | 'broken';
  cloudUid: string | null;
  bucketName: string | null;
  hasLocalFolder: boolean;
  localPath: string | null;
  membershipStatus: string | null;
  lastSyncedAt: string | null;
  // Diagnostic when state is 'broken'. Surfaced in the row tooltip + Connect
  // button hint. Always null for non-broken states.
  brokenReason: string | null;
}

// Mirrors src-tauri/src/commands/workspaces.rs::WorkspacesResult.
export interface WorkspacesResult {
  workspaces: Workspace[];
  cloudReachable: boolean;
  error: string | null;
  hqFolderPath: string;
  // Top-level manifest parse/IO error. Non-null means the user has a
  // companies/manifest.yaml we couldn't read; UI shows a soft notice and
  // workspaces fall back to folder enumeration.
  manifestError: string | null;
}
