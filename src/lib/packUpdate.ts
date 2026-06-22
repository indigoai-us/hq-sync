/** Banner title for the pack-update notice. Singular/plural aware. */
export function packUpdateTitle(count: number): string {
  return count === 1
    ? '1 pack has an update available'
    : `${count} packs have updates available`;
}
