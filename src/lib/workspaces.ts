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

/**
 * Companies the signed-in user has an ACTIVE membership in that are not on this
 * machine yet — i.e. they accepted an invite (the cloud membership is `active`,
 * which is what `claimByEmail` produces on first sign-in) but the workspace has
 * never been pulled down (`state === 'cloud-only'`, no local folder).
 *
 * These are the rows the menubar surfaces a "You've been added to {company} —
 * Sync to pull it" prompt for. The whole point: a teammate who accepts an
 * invite (email link or HQ Console) shouldn't have to know any command — the
 * app already knows about the membership, so it offers the one-click pull.
 *
 * Excluded on purpose:
 *  - `membershipStatus === 'pending'` — an unaccepted/ungranted invite, nothing
 *    to pull yet (the V4 Companies surface renders those as invite rows).
 *  - `synced` / `personal` / `local-only` / `broken` — already local, or not a
 *    fresh cloud membership to pull.
 *
 * Deduped first so a company present in both the manifest and cloud memberships
 * is only offered once.
 */
export function joinableMemberships(workspaces: Workspace[]): Workspace[] {
  return dedupeWorkspaces(workspaces).filter(
    (w) =>
      w.state === 'cloud-only' &&
      w.membershipStatus === 'active' &&
      // The personal vault is NEVER a "you've been added — pull it" target: it
      // auto-provisions for every user and is surfaced separately as the
      // canonical Personal row (state === 'personal'). A phantom
      // `company:personal` cloud-only row — emitted by workspaces.rs §2 when the
      // personal cloud membership has no local match, and not collapsed by
      // dedupe (different `kind`) — must not drive the prompt. Guard by slug so
      // a row mis-typed as kind=company can't leak through either.
      w.slug !== 'personal',
  );
}
