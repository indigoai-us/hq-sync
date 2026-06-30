//! Shared TLS transport builder for the app's MQTT-over-WSS connections to AWS
//! IoT Core (`commands/dm_mqtt.rs` and `commands/sessions/outpost.rs`).
//!
//! ## Why this exists
//!
//! `rumqttc::Transport::wss_with_default_config()` builds its TLS via
//! `TlsConfiguration::default()`, which does
//! `rustls_native_certs::load_native_certs().expect("could not load platform certs")`.
//! On macOS that reads the keychain trust store through the Security framework, and
//! it PANICS (FATAL, process-killing) when the framework returns a transient I/O
//! error — `errSecIO`, code -36 — observed in production hq-sync@0.8.30 as Sentry
//! issues **HQ-SYNC-D** and the sibling **HQ-SYNC-WEB-Q** (one fatal panic, two
//! projects, same code path). The panic fires synchronously while building the
//! transport, so the eventloop's send-after-close JoinError guard does not catch it.
//!
//! ## The fix
//!
//! AWS IoT Core's Data-ATS endpoint is served by public Amazon Trust Services CAs,
//! which are present in the bundled Mozilla root set — the same `webpki-roots` that
//! reqwest already uses for all of this app's HTTPS. So we never need the OS keychain
//! for these connections. Building the rustls config from the bundled roots removes
//! the keychain dependency entirely, which removes the panic at its source: a keychain
//! I/O hiccup can no longer crash the process. On a genuine TLS/connection failure,
//! rumqttc surfaces a recoverable eventloop error and the caller reconnects with capped
//! backoff (and, for DMs, the 60s poll backstops delivery throughout).
//!
//! The `ClientConfig` is built via rumqttc's own re-exported rustls
//! (`rumqttc::tokio_rustls::rustls`), so its type matches rumqttc exactly and there is
//! no separate rustls version to keep pinned in lock-step.

/// Build a `rumqttc::Transport::Wss` whose TLS trust anchors come from the bundled
/// webpki/Mozilla root set instead of the macOS platform keychain. See the module
/// docs for the full rationale (Sentry HQ-SYNC-D / HQ-SYNC-WEB-Q).
pub fn wss_transport_with_bundled_roots() -> rumqttc::Transport {
    use rumqttc::tokio_rustls::rustls::{ClientConfig, RootCertStore};
    use rumqttc::TlsConfiguration;
    use std::sync::Arc;

    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    rumqttc::Transport::Wss(TlsConfiguration::Rustls(Arc::new(config)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // HQ-SYNC-D / HQ-SYNC-WEB-Q: the WSS transport must build its TLS config from the
    // bundled webpki roots, never from the macOS keychain. The default
    // `wss_with_default_config()` path panics (FATAL) on a transient keychain I/O error
    // (errSecIO, -36); building from bundled roots removes that dependency entirely.
    #[test]
    fn wss_transport_builds_from_bundled_roots_without_panicking() {
        // Constructing the transport must not panic (the bug was a panic on
        // `load_native_certs().expect(...)`) and must yield a rustls-backed WSS
        // transport built from the bundled roots — no platform keychain touched.
        let transport = wss_transport_with_bundled_roots();
        assert!(
            matches!(
                transport,
                rumqttc::Transport::Wss(rumqttc::TlsConfiguration::Rustls(_))
            ),
            "expected a Wss(Rustls) transport built from bundled webpki roots"
        );
    }

    #[test]
    fn bundled_root_store_is_non_empty() {
        // Guard against an empty/misimported root set silently shipping (which would
        // fail every TLS handshake). The bundled Mozilla set must carry trust anchors,
        // and building it must not panic or reach for the OS keychain.
        use rumqttc::tokio_rustls::rustls::RootCertStore;
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        assert!(
            !roots.is_empty(),
            "bundled webpki root store must contain trust anchors"
        );
    }
}
