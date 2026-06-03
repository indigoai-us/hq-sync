/**
 * Human-facing titles for shared paths in the "Shared with Me" notification.
 *
 * When a teammate shares a whole directory, the path arrives as a wildcard like
 * `projects/client-stats-redesign/*` (or `/**` for a recursive share). Naively
 * taking the last path segment (`split('/').pop()`) surfaces the literal `*` as
 * the card title — meaningless to the recipient. `shareTitle` strips the
 * wildcard suffix and shows the directory name with a trailing slash
 * (`client-stats-redesign/`) so a folder share reads as a folder, while plain
 * file shares keep their filename unchanged.
 */

/** Matches a trailing `/*`, `/**`, or a bare `*`/`**` (optionally slash-led). */
const WILDCARD_SUFFIX = /\/?\*\*?$/;

/**
 * Title to show for a single shared path.
 *
 * - `projects/foo/*`  → `foo/`   (directory share — trailing slash signals folder)
 * - `projects/foo/**` → `foo/`   (recursive directory share)
 * - `docs/a.md`       → `a.md`   (file share — unchanged)
 * - `*` / `**`        → `All files` (whole-vault share — no segment to name)
 */
export function shareTitle(path: string): string {
  const isWildcardDir = WILDCARD_SUFFIX.test(path);
  const cleaned = path.replace(WILDCARD_SUFFIX, '');
  const last = cleaned.split('/').filter(Boolean).pop();
  if (!last) return 'All files';
  return isWildcardDir ? `${last}/` : last;
}
