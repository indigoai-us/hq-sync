/**
 * Type mirrors for the Packages window. These match the JSON shapes produced
 * by the `hq` CLI (`hq packs list --json`, `hq packages list --json`) and the
 * `packages.rs` Tauri command merge. Kept loose where the CLI payload is large
 * — only the fields the UI renders are typed.
 */

export type LinkCounts = {
  live: number;
  broken: number;
  missing: number;
  foreign: number;
};

export interface InstalledPack {
  name: string;
  version?: string;
  publisher?: string;
  source?: string;
  transport: string | null;
  requiresHqCore?: string;
  hqCoreSatisfied: boolean | null;
  contributes: Record<string, number>;
  links: LinkCounts;
  brokenLinks: Array<{ key: string; item: string; dst: string }>;
  inCatalog: boolean;
  updateAvailable: boolean | null;
  error?: string;
}

export interface AvailablePack {
  source: string;
  description?: string;
  installed: false;
  conditional?: string;
  conditionalStatus: 'pass' | 'fail' | 'unevaluated' | 'none';
}

export interface PacksList {
  hqRoot: string;
  hqVersion: string | null;
  installed: InstalledPack[];
  available: AvailablePack[];
  warnings: string[];
}

export interface RegistryEntry {
  name?: string;
  slug: string;
  version?: string;
  scope?: string;
}

export interface RegistryAvailable {
  slug: string;
  tier?: string;
}

export interface RegistryList {
  installed: RegistryEntry[];
  available: RegistryAvailable[];
  offline: boolean;
}

/** The merged payload `list_packages` / `packages_window_ready` return. */
export interface PackagesView {
  packs: PacksList | null;
  registry: RegistryList | null;
  error: string | null;
}

/** Progress line emitted during install/update. */
export interface PackagesProgress {
  op: 'install' | 'update';
  name: string;
  line: string;
}

export interface PackagesDone {
  op: string;
  name: string;
  message?: string;
}

/** A short, human label for a pack source string (drops the long git prefix). */
export function shortSource(source: string | undefined): string {
  if (!source) return 'unknown source';
  // github:owner/repo#packages/hq-pack-x  ->  hq-pack-x
  const hashIdx = source.lastIndexOf('/');
  if (hashIdx >= 0 && source.includes('hq-pack-')) {
    const tail = source.slice(hashIdx + 1);
    return tail.split('@')[0];
  }
  return source;
}
