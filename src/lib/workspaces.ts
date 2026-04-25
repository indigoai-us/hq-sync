// Mirrors src-tauri/src/commands/workspaces.rs::Workspace.
// Returned by the Rust `list_syncable_workspaces` Tauri command and rendered
// by the menubar's WorkspaceList component.
export interface Workspace {
  slug: string;
  displayName: string;
  kind: 'personal' | 'company';
  state: 'personal' | 'synced' | 'cloud-only' | 'local-only';
  cloudUid: string | null;
  bucketName: string | null;
  hasLocalFolder: boolean;
  localPath: string | null;
  membershipStatus: string | null;
  lastSyncedAt: string | null;
}

// Mirrors src-tauri/src/commands/workspaces.rs::WorkspacesResult.
export interface WorkspacesResult {
  workspaces: Workspace[];
  cloudReachable: boolean;
  error: string | null;
  hqFolderPath: string;
}
