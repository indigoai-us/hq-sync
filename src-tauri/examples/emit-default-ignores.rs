// Prints DEFAULT_IGNORES verbatim, one per line, for the parity script.
// MUST match src/util/ignore.rs::DEFAULT_IGNORES byte-for-byte — the parity
// script (check-default-ignores-parity.sh) diff-checks this against the TS
// constant AND against util/ignore.rs via the ignore crate's behavior.

const DEFAULT_IGNORES: &[&str] = &[
    // VCS + OS
    ".git/", ".git", ".DS_Store", "Thumbs.db",
    // Node / JS
    "node_modules/", "dist/", "build/", ".next/", ".nuxt/",
    ".svelte-kit/", ".turbo/", ".parcel-cache/", ".vite/", "coverage/",
    // Rust / Tauri
    "target/",
    // Python
    "__pycache__/", "*.pyc", ".pytest_cache/", ".mypy_cache/",
    ".ruff_cache/", ".venv/", "venv/",
    // Go / JVM / other
    "vendor/", "out/", "*.class",
    // Generic caches / temp
    ".cache/", "tmp/", ".tmp/",
    // HQ sync internal state (never round-trip these)
    "*.pid", ".hq-sync.pid",
    ".hq-sync-journal.json",
    ".hq-sync-state.json",
    "modules.lock",
    // HQ repos directory (managed separately, not synced)
    "repos/",
    // Secrets / env
    ".env", ".env.*",
];

fn main() {
    for pat in DEFAULT_IGNORES {
        println!("{}", pat);
    }
}
