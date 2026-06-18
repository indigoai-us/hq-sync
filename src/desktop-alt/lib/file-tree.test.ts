import { describe, expect, it } from 'vitest';
import { flattenTree, sortNodes, type FileNode } from './file-tree';

/**
 * US-006 — Frontend unit tests for the pure company-file-tree helpers.
 *
 * Covers the contract deferred from US-002:
 *   - sortNodes: folders-before-files, case-insensitive alphabetical within each
 *     group, recursive into children, and purity (no input mutation).
 *   - flattenTree: depth tracking + path flattening in display order, honoring
 *     the isExpanded predicate (collapsed dirs hide their subtree).
 *
 * These are dependency-free pure functions (no Svelte runes, no Tauri), so the
 * tests import the real module and assert on data only.
 */

/** Convenience constructor for a leaf file node. */
function file(name: string, path: string): FileNode {
  return { name, path, isDir: false, children: [] };
}

/** Convenience constructor for a directory node. */
function dir(name: string, path: string, children: FileNode[] = []): FileNode {
  return { name, path, isDir: true, children };
}

describe('file-tree sortNodes (US-006)', () => {
  it('sorts folders before files', () => {
    const input: FileNode[] = [
      file('readme.md', 'companies/test/readme.md'),
      dir('policies', 'companies/test/policies'),
      file('config.json', 'companies/test/config.json'),
      dir('data', 'companies/test/data'),
    ];

    const sorted = sortNodes(input);

    // All directories must come before any file, regardless of name.
    const firstFileIndex = sorted.findIndex((n) => !n.isDir);
    const lastDirIndex = sorted.map((n) => n.isDir).lastIndexOf(true);
    expect(lastDirIndex).toBeLessThan(firstFileIndex);

    expect(sorted.map((n) => n.name)).toEqual([
      'data',
      'policies',
      'config.json',
      'readme.md',
    ]);
  });

  it('sorts alphabetically (case-insensitive) within each group', () => {
    const input: FileNode[] = [
      dir('Zebra', 'companies/test/Zebra'),
      dir('alpha', 'companies/test/alpha'),
      file('Banana.txt', 'companies/test/Banana.txt'),
      file('apple.txt', 'companies/test/apple.txt'),
    ];

    const sorted = sortNodes(input);

    // Folders: alpha before Zebra (case-insensitive). Files: apple before Banana.
    expect(sorted.map((n) => n.name)).toEqual([
      'alpha',
      'Zebra',
      'apple.txt',
      'Banana.txt',
    ]);
  });

  it('sorts recursively into children', () => {
    const input: FileNode[] = [
      dir('root', 'companies/test/root', [
        file('b.txt', 'companies/test/root/b.txt'),
        dir('sub', 'companies/test/root/sub'),
        file('a.txt', 'companies/test/root/a.txt'),
      ]),
    ];

    const sorted = sortNodes(input);
    const childNames = sorted[0].children.map((n) => n.name);

    // Directory (sub) first, then files alphabetically (a.txt, b.txt).
    expect(childNames).toEqual(['sub', 'a.txt', 'b.txt']);
  });

  it('does not mutate the input array or input nodes (pure)', () => {
    const child = file('a.txt', 'companies/test/root/a.txt');
    const root = dir('root', 'companies/test/root', [
      file('b.txt', 'companies/test/root/b.txt'),
      child,
    ]);
    const input: FileNode[] = [
      file('z.txt', 'companies/test/z.txt'),
      root,
    ];
    const snapshotOrder = input.map((n) => n.name);
    const snapshotChildOrder = root.children.map((n) => n.name);

    const sorted = sortNodes(input);

    // Original arrays are untouched.
    expect(input.map((n) => n.name)).toEqual(snapshotOrder);
    expect(root.children.map((n) => n.name)).toEqual(snapshotChildOrder);
    // Returned nodes are fresh objects, not the same references.
    expect(sorted).not.toBe(input);
    const sortedRoot = sorted.find((n) => n.name === 'root')!;
    expect(sortedRoot).not.toBe(root);
    expect(sortedRoot.children).not.toBe(root.children);
  });

  it('handles missing children arrays without throwing', () => {
    // A node whose children is undefined (defensive — Rust always sends []).
    const input = [
      { name: 'orphan', path: 'companies/test/orphan', isDir: true } as FileNode,
    ];
    const sorted = sortNodes(input);
    expect(sorted[0].children).toEqual([]);
  });
});

describe('file-tree flattenTree (US-006)', () => {
  const tree: FileNode[] = [
    dir('companies/test', 'companies/test', [
      dir('policies', 'companies/test/policies', [
        file('foo.md', 'companies/test/policies/foo.md'),
      ]),
      file('readme.md', 'companies/test/readme.md'),
    ]),
  ];

  it('flattens to display order with depth and path tracking', () => {
    // Everything expanded.
    const rows = flattenTree(tree, () => true);

    expect(rows.map((r) => ({ path: r.node.path, depth: r.depth }))).toEqual([
      { path: 'companies/test', depth: 0 },
      { path: 'companies/test/policies', depth: 1 },
      { path: 'companies/test/policies/foo.md', depth: 2 },
      { path: 'companies/test/readme.md', depth: 1 },
    ]);
  });

  it('hides a collapsed directory subtree', () => {
    // Only the root is expanded; companies/test/policies is collapsed, so its
    // child foo.md must not appear.
    const expanded = new Set(['companies/test']);
    const rows = flattenTree(tree, (p) => expanded.has(p));

    expect(rows.map((r) => r.node.path)).toEqual([
      'companies/test',
      'companies/test/policies',
      'companies/test/readme.md',
    ]);
    expect(rows.map((r) => r.node.path)).not.toContain(
      'companies/test/policies/foo.md',
    );
  });

  it('flattens folders-before-files at every level', () => {
    const unsorted: FileNode[] = [
      file('z.txt', 'companies/test/z.txt'),
      dir('a-dir', 'companies/test/a-dir', [
        file('inner.txt', 'companies/test/a-dir/inner.txt'),
      ]),
    ];
    const rows = flattenTree(unsorted, () => true);

    // Directory row emitted before the sibling file, with its child nested.
    expect(rows.map((r) => r.node.path)).toEqual([
      'companies/test/a-dir',
      'companies/test/a-dir/inner.txt',
      'companies/test/z.txt',
    ]);
  });

  it('respects a non-zero starting depth', () => {
    const rows = flattenTree(
      [file('a.txt', 'companies/test/a.txt')],
      () => true,
      3,
    );
    expect(rows[0].depth).toBe(3);
  });
});
