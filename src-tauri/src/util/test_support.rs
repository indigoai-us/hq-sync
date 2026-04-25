// Shared test infrastructure for env-var-sensitive tests across util and commands modules.
//
// Both `util::journal::tests` and `commands::first_push::tests` mutate HQ_STATE_DIR.
// A single mutex here ensures they serialize even when cargo runs tests in parallel.
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

pub(crate) static ENV_MUTEX: Mutex<()> = Mutex::new(());

pub(crate) fn with_state_dir<F: FnOnce(&Path)>(f: F) {
    let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    std::env::set_var("HQ_STATE_DIR", tmp.path().to_str().unwrap());
    f(tmp.path());
    std::env::remove_var("HQ_STATE_DIR");
}
