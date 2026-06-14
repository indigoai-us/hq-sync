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
  role: string | null;
  lastSyncedAt: string | null;
  // Diagnostic when state is 'broken'. Surfaced in the row tooltip + Connect
  // button hint. Always null for non-broken states.
  brokenReason: string | null;
  // Invite metadata from the vault membership row (`invitedBy` is a prs_*
  // person uid, `invitedAt` an ISO timestamp). Only meaningful while
  // membershipStatus === 'pending' — the V4 Companies overview renders the
  // NOT CONNECTED invite row from them.
  invitedBy: string | null;
  invitedAt: string | null;
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

/**
 * Collapse duplicate workspaces to one entry per `kind:slug`, first occurrence
 * wins (preserving the backend ordering of the survivor).
 *
 * `list_syncable_workspaces` is the UNION of manifest companies and cloud
 * memberships, so a company present in both arrives twice under the same
 * kind+slug. Every list that renders a keyed `{#each}` over workspaces (the
 * classic popover's WorkspaceList keys by `kind:slug`) throws Svelte's
 * `each_key_duplicate` on a repeat — which aborts the whole render and freezes
 * the surface. Dedupe through here before keying.
 */
export function dedupeWorkspaces(workspaces: Workspace[]): Workspace[] {
  const seen = new Set<string>();
  return workspaces.filter((workspace) => {
    const key = `${workspace.kind}:${workspace.slug}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}
