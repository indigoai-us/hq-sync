import { describe, expect, it } from 'vitest';
import {
  dirEntryToLazyNode,
  flattenLazy,
  flattenTree,
  sortNodes,
  type DirEntry,
  type FileNode,
  type LazyNode,
} from './file-tree';

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

describe('file-tree lazy helpers (US-010)', () => {
  function entry(overrides: Partial<DirEntry>): DirEntry {
    return {
      name: 'x',
      path: 'x',
      isDir: false,
      hasChildren: false,
      ...overrides,
    };
  }

  /** Convenience: a loaded lazy directory node with children. */
  function lazyDir(
    name: string,
    path: string,
    children: LazyNode[] | undefined = undefined,
  ): LazyNode {
    return {
      name,
      path,
      isDir: true,
      hasChildren: true,
      loaded: children !== undefined,
      children,
    };
  }

  function lazyFile(name: string, path: string): LazyNode {
    return { name, path, isDir: false, hasChildren: false, loaded: false };
  }

  it('dirEntryToLazyNode maps a DirEntry to an unloaded node', () => {
    const node = dirEntryToLazyNode(
      entry({ name: 'repos', path: 'repos', isDir: true, hasChildren: true }),
    );
    expect(node).toEqual({
      name: 'repos',
      path: 'repos',
      isDir: true,
      hasChildren: true,
      loaded: false,
      children: undefined,
    });
  });

  it('clears hasChildren for files (only dirs can be expandable)', () => {
    const node = dirEntryToLazyNode(
      // A backend that erroneously set hasChildren on a file must not produce
      // an expandable file row.
      entry({ name: 'README.md', path: 'README.md', isDir: false, hasChildren: true }),
    );
    expect(node.hasChildren).toBe(false);
  });

  it('flattenLazy emits only loaded+expanded subtrees (lazy: unloaded dirs show no children)', () => {
    const tree: LazyNode[] = [
      // Loaded + has a child.
      lazyDir('companies', 'companies', [lazyFile('manifest.yaml', 'companies/manifest.yaml')]),
      // hasChildren but NOT loaded yet → no children even if "expanded".
      lazyDir('repos', 'repos', undefined),
    ];
    const expanded = new Set(['companies', 'repos']);
    const rows = flattenLazy(tree, (p) => expanded.has(p));

    expect(rows.map((r) => ({ path: r.node.path, depth: r.depth }))).toEqual([
      { path: 'companies', depth: 0 },
      { path: 'companies/manifest.yaml', depth: 1 },
      // repos is expanded but unloaded → no children rows yet (lazy).
      { path: 'repos', depth: 0 },
    ]);
  });

  it('flattenLazy hides a collapsed (loaded) directory subtree', () => {
    const tree: LazyNode[] = [
      lazyDir('core', 'core', [lazyFile('core.yaml', 'core/core.yaml')]),
    ];
    // Not expanded → child hidden even though it is loaded.
    const rows = flattenLazy(tree, () => false);
    expect(rows.map((r) => r.node.path)).toEqual(['core']);
  });

  it('flattenLazy sorts folders-before-files, case-insensitive alphabetical', () => {
    const tree: LazyNode[] = [
      lazyFile('zeta.txt', 'zeta.txt'),
      lazyDir('Beta', 'Beta'),
      lazyDir('alpha', 'alpha'),
      lazyFile('Apple.txt', 'Apple.txt'),
    ];
    const rows = flattenLazy(tree, () => false);
    expect(rows.map((r) => r.node.name)).toEqual([
      'alpha',
      'Beta',
      'Apple.txt',
      'zeta.txt',
    ]);
  });

  it('flattenLazy tracks depth across nested loaded dirs', () => {
    const tree: LazyNode[] = [
      lazyDir('repos', 'repos', [
        lazyDir('public', 'repos/public', [
          lazyFile('hq-sync', 'repos/public/hq-sync'),
        ]),
      ]),
    ];
    const expanded = new Set(['repos', 'repos/public']);
    const rows = flattenLazy(tree, (p) => expanded.has(p));
    expect(rows.map((r) => ({ path: r.node.path, depth: r.depth }))).toEqual([
      { path: 'repos', depth: 0 },
      { path: 'repos/public', depth: 1 },
      { path: 'repos/public/hq-sync', depth: 2 },
    ]);
  });
});
