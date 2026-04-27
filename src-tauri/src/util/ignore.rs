// companies/indigo/repos/hq-sync/src-tauri/src/util/ignore.rs
// Items here are unused in this step but consumed by later sync/upload steps.
#![allow(dead_code)]
use std::path::{Path, PathBuf};
use ignore::gitignore::{Gitignore, GitignoreBuilder};

pub const DEFAULT_IGNORES: &[&str] = &[
    // VCS + OS
    ".git/", ".git", ".DS_Store", "Thumbs.db",
    // Node / JS
    "node_modules/", "dist/", "build/", ".next/", ".nuxt/",
    ".svelte-kit/", ".turbo/", ".parcel-cache/", ".vite/", "coverage/",
    // Company-local: secrets/config, datasets, prompt libraries — all stay
    // on-disk and never round-trip through the company vault bucket.
    "settings/",
    "data/",
    "workers/",
    // Rust / Tauri
    "target/",
    // Python
    "__pycache__/", "*.pyc", ".pytest_cache/", ".mypy_cache/",
    ".ruff_cache/", ".venv/", "venv/",
    // Go / JVM / other
    "vendor/", "out/", "*.class",
    // Generic caches / temp
    ".cache/", "tmp/", ".tmp/",
    // HQ sync internal state (never round-trip these). The `.hq-*` wildcard
    // covers `.hq-sync.pid`, `.hq-sync-journal.json`, `.hq-sync-state.json`,
    // `.hq-embeddings-pending.json`, and any future internal-state file. The
    // `.hqignore` / `.hqsyncignore` / `.hqinclude` config files don't match
    // (no hyphen) and the `.hq/` directory is unaffected.
    "*.pid", ".hq-*",
    "modules.lock",
    // hq-root identity marker — discovered locally per-machine, never synced.
    "core.yaml",
    // hq modules manifest — local module-resolution state, never synced.
    "modules/modules.yaml",
    // per-company identity file — written locally on first sync, never round-tripped.
    "company.yaml",
    // HQ repos directory (managed separately, not synced)
    "repos/",
    // Secrets / env
    ".env", ".env.*",
];

pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

pub struct IgnoreFilter {
    matcher: Gitignore,
    hq_root: PathBuf,
}

impl IgnoreFilter {
    pub fn for_hq_root(hq_root: &Path) -> Result<Self, String> {
        let mut builder = GitignoreBuilder::new(hq_root);
        for pat in DEFAULT_IGNORES {
            builder
                .add_line(None, pat)
                .map_err(|e| format!("default pattern `{pat}`: {e}"))?;
        }
        for name in [".gitignore", ".hqignore", ".hqsyncignore"] {
            let p = hq_root.join(name);
            if p.exists() {
                if let Some(e) = builder.add(&p) {
                    return Err(format!("{}: {e}", p.display()));
                }
            }
        }
        Ok(Self {
            matcher: builder.build().map_err(|e| e.to_string())?,
            hq_root: hq_root.to_path_buf(),
        })
    }

    /// Matches hq-cloud's behavior: true = should sync, false = ignore.
    /// Outside-root branch intentionally returns `true` — the TS
    /// `createIgnoreFilter(hqRoot)(filePath)` at
    /// `packages/hq-cloud/src/ignore.ts:105-109` returns `true` when
    /// `path.relative(hqRoot, filePath)` is empty OR starts with `..`.
    pub fn should_sync(&self, abs_path: &Path) -> bool {
        let rel = match abs_path.strip_prefix(&self.hq_root) {
            Ok(r) => r,
            Err(_) => return true, // outside root — matches TS behavior
        };
        !self.matcher.matched_path_or_any_parents(rel, /*is_dir*/ false).is_ignore()
    }

    pub fn within_size_limit(abs_path: &Path) -> bool {
        std::fs::metadata(abs_path).map_or(false, |m| m.len() <= MAX_FILE_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn git_dir_is_ignored() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let filter = IgnoreFilter::for_hq_root(root).unwrap();
        assert!(!filter.should_sync(&root.join(".git")));
    }

    #[test]
    fn regular_file_is_synced() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let filter = IgnoreFilter::for_hq_root(root).unwrap();
        assert!(filter.should_sync(&root.join("companies/indigo/docs/foo.md")));
    }

    #[test]
    fn nested_node_modules_is_ignored() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let filter = IgnoreFilter::for_hq_root(root).unwrap();
        assert!(!filter.should_sync(&root.join("companies/indigo/node_modules/x")));
    }

    #[test]
    fn hqignore_pattern_is_ignored() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join(".hqignore"), "knowledge/*.secret\n").unwrap();
        let filter = IgnoreFilter::for_hq_root(root).unwrap();
        assert!(!filter.should_sync(&root.join("knowledge/api.secret")));
    }

    #[test]
    fn re_include_works() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join(".hqignore"), "knowledge/\n!knowledge/keep.md\n").unwrap();
        let filter = IgnoreFilter::for_hq_root(root).unwrap();
        assert!(!filter.should_sync(&root.join("knowledge/other.md")));
        assert!(filter.should_sync(&root.join("knowledge/keep.md")));
    }

    #[test]
    fn company_local_dirs_are_ignored() {
        // settings/, data/, and workers/ contain credentials, datasets, and
        // company-private prompt libraries respectively — none of these belong
        // in the company vault bucket. Regressing this would silently leak
        // sensitive content to S3 on the next sync.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let filter = IgnoreFilter::for_hq_root(root).unwrap();
        assert!(!filter.should_sync(&root.join("companies/indigo/settings/aws.json")));
        assert!(!filter.should_sync(&root.join("companies/indigo/data/exports/leads.csv")));
        assert!(!filter.should_sync(&root.join("companies/indigo/workers/cmo/worker.yaml")));
    }

    #[test]
    fn outside_of_root_returns_true() {
        let filter = IgnoreFilter::for_hq_root(Path::new("/some/other/path")).unwrap();
        assert!(filter.should_sync(Path::new("/tmp/not-hq/foo.md")));
    }

    #[test]
    fn hq_root_core_yaml_marker_is_ignored() {
        // core.yaml is the local hq-root identity marker. It must never
        // round-trip through the bucket — pulling another machine's marker
        // would corrupt root discovery.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let filter = IgnoreFilter::for_hq_root(root).unwrap();
        assert!(!filter.should_sync(&root.join("core.yaml")));
    }

    #[test]
    fn modules_manifest_is_ignored() {
        // modules.yaml is the local modules-resolution manifest. Per-machine
        // state, never synced.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let filter = IgnoreFilter::for_hq_root(root).unwrap();
        assert!(!filter.should_sync(&root.join("modules/modules.yaml")));
    }

    #[test]
    fn company_yaml_is_ignored() {
        // company.yaml is written locally on first sync from the entity
        // context. Round-tripping would let one machine's identity overwrite
        // another's.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let filter = IgnoreFilter::for_hq_root(root).unwrap();
        assert!(!filter.should_sync(&root.join("companies/indigo/company.yaml")));
        assert!(!filter.should_sync(&root.join("company.yaml")));
    }

    #[test]
    fn hq_dash_prefix_is_ignored_but_hqignore_family_and_hq_dir_sync() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let filter = IgnoreFilter::for_hq_root(root).unwrap();
        // .hq-* internal state files must never round-trip through the bucket.
        assert!(!filter.should_sync(&root.join(".hq-sync.pid")));
        assert!(!filter.should_sync(&root.join(".hq-sync-journal.json")));
        assert!(!filter.should_sync(&root.join(".hq-sync-state.json")));
        assert!(!filter.should_sync(&root.join(".hq-embeddings-pending.json")));
        assert!(!filter.should_sync(&root.join("companies/indigo/.hq-foo.json")));
        assert!(!filter.should_sync(&root.join(".hq-cache/blob.bin")));
        // Sync-config files and the .hq/ directory still sync.
        assert!(filter.should_sync(&root.join(".hqignore")));
        assert!(filter.should_sync(&root.join(".hqsyncignore")));
        assert!(filter.should_sync(&root.join(".hqinclude")));
        assert!(filter.should_sync(&root.join("companies/indigo/.hq/config.json")));
    }

    #[test]
    fn re_include_overrides_default_ignores() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // .env is in DEFAULT_IGNORES; a .hqignore negation must override it.
        fs::write(root.join(".hqignore"), "!.env\n").unwrap();
        fs::write(root.join(".env"), "SECRET=1\n").unwrap();
        let filter = IgnoreFilter::for_hq_root(root).unwrap();
        assert!(filter.should_sync(&root.join(".env")));
    }
}
