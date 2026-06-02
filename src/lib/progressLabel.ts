/**
 * Calm, friendly labels for the file currently being transferred during a sync.
 *
 * The first sync of a fresh HQ uploads the entire `core/` scaffold — the
 * release-shipped HQ files (docs, hooks, knowledge, policies, scripts, skills,
 * workers). They are identical for every user and are NOT the user's own
 * content. Letting thousands of unfamiliar `core/…` paths stream past (or the
 * live label churn through them) reads as "all my stuff is being uploaded" and
 * alarms first-time users.
 *
 * `displayLabel` collapses any `core/` path into one steady, reassuring line so
 * the run reads as one-time *setup* rather than a flood. The honest file
 * counter elsewhere is unaffected — this only changes the human-facing label.
 */

export const CORE_SETUP_LABEL = 'Setting up HQ core files…';

/**
 * True when `path` lives under the release-shipped `core/` tree (root-level
 * `core/…` or a nested `…/core/…`). Leading slashes are tolerated.
 */
export function isCorePath(path: string | null | undefined): boolean {
  if (!path) return false;
  const p = path.replace(/^\/+/, '');
  return p === 'core' || p.startsWith('core/') || p.includes('/core/');
}

/**
 * Map a transferred file path to its human-facing label: core files collapse to
 * `CORE_SETUP_LABEL`; everything else is returned unchanged.
 */
export function displayLabel(path: string | null | undefined): string {
  if (isCorePath(path)) return CORE_SETUP_LABEL;
  return path ?? '';
}
