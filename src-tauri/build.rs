fn main() {
    println!("cargo:rerun-if-env-changed=HQ_SYNC_SENTRY_DSN");
    println!(
        "cargo:rustc-env=SENTRY_DSN={}",
        std::env::var("HQ_SYNC_SENTRY_DSN").unwrap_or_default()
    );

    // Emit the shipped npm/tauri.conf.json version as `APP_VERSION` so the
    // client-attribution headers report the user-facing release version
    // rather than the Cargo crate version. The two version numbers drift
    // deliberately — the Rust crate is internal, the npm package.json is
    // what users see in About-dialogs and DMG names. Reads ../package.json
    // at compile time so there's no runtime manifest lookup.
    println!("cargo:rerun-if-changed=../package.json");
    let pkg_json = std::fs::read_to_string("../package.json")
        .expect("build.rs: failed to read ../package.json");
    let version = extract_json_string_field(&pkg_json, "version")
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=APP_VERSION={}", version);

    tauri_build::build()
}

// Tiny ad-hoc parse for top-level string fields in package.json. Avoids
// pulling serde_json into the build-script dep graph just to read one value.
fn extract_json_string_field(json: &str, field: &str) -> Option<String> {
    let needle = format!("\"{}\"", field);
    let start = json.find(&needle)?;
    let after_key = &json[start + needle.len()..];
    let colon = after_key.find(':')?;
    let after_colon = after_key[colon + 1..].trim_start();
    let stripped = after_colon.strip_prefix('"')?;
    let end = stripped.find('"')?;
    Some(stripped[..end].to_string())
}
