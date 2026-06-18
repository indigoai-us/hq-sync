/**
 * Pure helpers for the company file explorer (US-002).
 *
 * This module is the TypeScript half of the `get_company_file_tree` contract.
 * The Rust half lives in `src-tauri/src/commands/desktop_alt.rs` (struct
 * `FileNode` with `#[serde(rename_all = "camelCase")]`), so the wire payload
 * maps 1:1 onto the `FileNode` interface below.
 *
 * No Svelte runes and no Tauri here — just data and side-effect-free functions,
 * so the contract stays trivially unit-testable under vitest (the test itself is
 * US-006). The Rust side already sorts (dirs-before-files, case-insensitive
 * alphabetical), but `sortNodes` sorts again so the tree is independently
 * correct and robust to unsorted input.
 */

/**
 * One node in the company file tree.
 *
 * Mirrors the Rust `FileNode` struct exactly (camelCase on the wire):
 * - `name`     node display name; the root node's name is the company slug.
 * - `path`     HQ-folder-relative, forward-slash separated, e.g.
 *              `"companies/indigo/policies/foo.md"`.
 * - `isDir`    true for directories; files are leaves.
 * - `children` child nodes; files always have `[]`.
 */
export interface FileNode {
  name: string;
  path: string;
  isDir: boolean;
  children: FileNode[];
}

/**
 * One row in the flattened display order, paired with its indentation depth.
 * `depth` is 0 for the nodes passed in at the top level and increases by one
 * per level of nesting (used by the UI to scale padding-left).
 */
export interface FlatRow {
  node: FileNode;
  depth: number;
}

/**
 * Compare two nodes for display order: directories before files, then
 * case-insensitive alphabetical by name within each group. Pure comparator.
 */
function compareNodes(a: FileNode, b: FileNode): number {
  if (a.isDir !== b.isDir) {
    // Directories (isDir true) sort before files.
    return a.isDir ? -1 : 1;
  }
  return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
}

/**
 * Return a NEW array of nodes sorted for display (folders-before-files, each
 * group case-insensitive alphabetical), recursing into children so the entire
 * subtree is sorted. Does NOT mutate the input array or any input node — every
 * returned node is a fresh object with a freshly-sorted `children` array.
 */
export function sortNodes(nodes: FileNode[]): FileNode[] {
  return [...nodes]
    .map((node) => ({ ...node, children: sortNodes(node.children ?? []) }))
    .sort(compareNodes);
}

/**
 * Flatten a tree into display order for rendering/testing.
 *
 * Signature: `flattenTree(nodes, isExpanded, depth?)`
 * - `nodes`      the sibling nodes to flatten (e.g. a root's `children`, or a
 *                single-element array `[root]`). Sorted internally via
 *                `sortNodes`, so callers may pass unsorted input.
 * - `isExpanded` predicate keyed on the node's `path`; a directory's children
 *                are only emitted when `isExpanded(path)` returns true. Pass a
 *                `Set<string>` via `(p) => set.has(p)` for the common case.
 * - `depth`      starting depth for the passed-in nodes (default 0); recursion
 *                increments it per level.
 *
 * Returns a flat list of `{ node, depth }` in display order: each directory row
 * is followed immediately by its (recursively flattened) children when expanded.
 * Pure and side-effect-free.
 */
export function flattenTree(
  nodes: FileNode[],
  isExpanded: (path: string) => boolean,
  depth = 0,
): FlatRow[] {
  const rows: FlatRow[] = [];
  for (const node of sortNodes(nodes)) {
    rows.push({ node, depth });
    if (node.isDir && isExpanded(node.path)) {
      rows.push(...flattenTree(node.children ?? [], isExpanded, depth + 1));
    }
  }
  return rows;
}
