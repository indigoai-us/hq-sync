// src-tauri/src/sentry_scrub.rs — canonical fleet Rust scrubber.
// Only `Event`, `Context`, and `Value` are referenced inside this module.
// The `ClientOptions` and `Arc` imports that the wiring needs (see
// `lib.rs::run()` in this step and `main.rs::main()` in Step 17) live
// at the call site, not here — keeping this module's import list
// minimal so `cargo clippy -- -D warnings` stays clean.
use sentry::protocol::{Context, Event, Value};
use std::collections::BTreeMap;

const SENSITIVE_FIELD_NAMES: &[&str] = &[
    "authorization",
    "password",
    "secret",
    "apikey",
    "api_key",
    "token",
];

fn is_sensitive_key(k: &str) -> bool {
    SENSITIVE_FIELD_NAMES
        .iter()
        .any(|name| k.eq_ignore_ascii_case(name))
}

fn scrub_sensitive_in_value(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for (k, child) in map.iter_mut() {
                if is_sensitive_key(k) {
                    *child = Value::String("[Filtered]".into());
                } else {
                    scrub_sensitive_in_value(child);
                }
            }
        }
        Value::Array(items) => {
            for child in items.iter_mut() {
                scrub_sensitive_in_value(child);
            }
        }
        _ => {}
    }
}

/// Scrub a single `Context` value. Extracted as `pub(crate)` so the test
/// module can exercise the fail-closed branch directly without needing
/// to build a whole `Event`. `before_send` below calls this helper for
/// every entry in `event.contexts`, so production and tests go through
/// the SAME code path — if a future refactor tries to reintroduce
/// `if let Ok(scrubbed) = ... { *ctx = scrubbed }` (silently swallowing
/// round-trip failure), the test 6d below catches it.
///
/// FAIL CLOSED. Both `serde_json::to_value` and `serde_json::from_value`
/// are fallible. Silently swallowing a failure would leave the original
/// (unscrubbed) context on the event — exactly the silent-leak class
/// this scrubber exists to close. If the round-trip cannot complete we
/// return a marker `Context::Other` with the key `scrub_error` so the
/// failure is visible in Sentry; "Success Criterion: No auth tokens or
/// secrets appear in any Sentry event payload" takes precedence over
/// context fidelity.
pub(crate) fn scrub_context(ctx: Context) -> Context {
    match serde_json::to_value(&ctx) {
        Ok(mut val) => {
            scrub_sensitive_in_value(&mut val);
            match serde_json::from_value(val) {
                Ok(scrubbed) => scrubbed,
                Err(_) => scrub_error_marker(),
            }
        }
        Err(_) => scrub_error_marker(),
    }
}

fn scrub_error_marker() -> Context {
    let mut marker: BTreeMap<String, Value> = BTreeMap::new();
    marker.insert(
        "scrub_error".to_string(),
        Value::String("[Filtered — serde round-trip failed]".into()),
    );
    Context::Other(marker)
}

pub fn before_send(mut event: Event<'static>) -> Option<Event<'static>> {
    // protocol::Request.headers is a Map<String, String>; wipe sensitive
    // header values in-place. (Rust SDK's header map holds owned strings,
    // unlike JS where request.headers is a generic Record<string, unknown>.)
    if let Some(request) = event.request.as_mut() {
        let sensitive_keys: Vec<String> = request
            .headers
            .keys()
            .filter(|k| is_sensitive_key(k))
            .cloned()
            .collect();
        for k in sensitive_keys {
            request.headers.insert(k, "[Filtered]".into());
        }
    }

    // event.extra is BTreeMap<String, Value>; recurse into each value and
    // also redact top-level sensitive keys.
    for (k, v) in event.extra.iter_mut() {
        if is_sensitive_key(k) {
            *v = Value::String("[Filtered]".into());
        } else {
            scrub_sensitive_in_value(v);
        }
    }

    // event.contexts is BTreeMap<String, Context>; `Context` is a typed enum
    // (`Device`, `Os`, `Runtime`, `App`, `Browser`, `Gpu`, `Trace`, `Other`).
    // Delegate each variant to `scrub_context` above so production and the
    // tests in this file share one code path (see the doc-comment there).
    for (_name, ctx) in event.contexts.iter_mut() {
        let taken = std::mem::replace(ctx, Context::Other(BTreeMap::new()));
        *ctx = scrub_context(taken);
    }

    // event.breadcrumbs[].data is BTreeMap<String, Value> — same pattern
    // as event.extra. This is the surface that carries Authorization
    // headers from HTTP breadcrumbs, which is the single most common
    // auth-leak vector on the Rust side.
    for breadcrumb in event.breadcrumbs.values.iter_mut() {
        for (k, v) in breadcrumb.data.iter_mut() {
            if is_sensitive_key(k) {
                *v = Value::String("[Filtered]".into());
            } else {
                scrub_sensitive_in_value(v);
            }
        }
    }

    Some(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sentry::protocol::{AppContext, Breadcrumb, Request, RuntimeContext};

    // 1. Case-insensitive is_sensitive_key
    #[test]
    fn test_is_sensitive_key_case_insensitive() {
        assert!(is_sensitive_key("Authorization"));
        assert!(is_sensitive_key("AUTHORIZATION"));
        assert!(is_sensitive_key("authorization"));
        assert!(is_sensitive_key("Password"));
        assert!(is_sensitive_key("SECRET"));
        assert!(is_sensitive_key("token"));
        assert!(is_sensitive_key("apikey"));
        assert!(is_sensitive_key("api_key"));
        assert!(!is_sensitive_key("x-api-key"));
        assert!(!is_sensitive_key("url"));
        assert!(!is_sensitive_key("note"));
    }

    // 2. Header strip
    #[test]
    fn test_header_strip() {
        let mut event = Event::default();
        let mut request = Request::default();
        request
            .headers
            .insert("Authorization".to_string(), "Bearer xyz".to_string());
        request
            .headers
            .insert("X-Trace".to_string(), "keep".to_string());
        event.request = Some(request);

        let result = before_send(event).unwrap();
        let headers = &result.request.unwrap().headers;
        assert_eq!(headers["Authorization"], "[Filtered]");
        assert_eq!(headers["X-Trace"], "keep");
    }

    // 3. Extra top-level redact
    #[test]
    fn test_extra_top_level_redact() {
        let mut event = Event::default();
        event
            .extra
            .insert("token".to_string(), Value::String("abc".into()));
        event
            .extra
            .insert("note".to_string(), Value::String("ok".into()));

        let result = before_send(event).unwrap();
        assert_eq!(result.extra["token"], Value::String("[Filtered]".into()));
        assert_eq!(result.extra["note"], Value::String("ok".into()));
    }

    // 4. Extra nested redact
    #[test]
    fn test_extra_nested_redact() {
        let mut event = Event::default();
        let mut inner = serde_json::Map::new();
        inner.insert("password".to_string(), Value::String("x".into()));
        inner.insert("ok".to_string(), Value::String("y".into()));
        event
            .extra
            .insert("payload".to_string(), Value::Object(inner));

        let result = before_send(event).unwrap();
        if let Value::Object(inner) = &result.extra["payload"] {
            assert_eq!(inner["password"], Value::String("[Filtered]".into()));
            assert_eq!(inner["ok"], Value::String("y".into()));
        } else {
            panic!("expected object");
        }
    }

    // 5. Breadcrumb data strip
    #[test]
    fn test_breadcrumb_data_strip() {
        let mut event = Event::default();
        let mut breadcrumb = Breadcrumb::default();
        breadcrumb.data.insert(
            "authorization".to_string(),
            Value::String("Bearer leak".into()),
        );
        breadcrumb
            .data
            .insert("url".to_string(), Value::String("/api".into()));
        event.breadcrumbs.values.push(breadcrumb);

        let result = before_send(event).unwrap();
        let data = &result.breadcrumbs.values[0].data;
        assert_eq!(data["authorization"], Value::String("[Filtered]".into()));
        assert_eq!(data["url"], Value::String("/api".into()));
    }

    // 6a. Typed Context::App round-trip — non-sensitive typed fields preserved
    #[test]
    fn test_context_app_round_trip() {
        let mut event = Event::default();
        let mut other_fields = BTreeMap::new();
        other_fields.insert("apikey".to_string(), Value::String("leak".into()));
        other_fields.insert("build".to_string(), Value::String("keep".into()));
        let app_ctx = AppContext {
            app_name: Some("hq".into()),
            app_version: Some("1.0".into()),
            other: other_fields,
            ..Default::default()
        };
        event
            .contexts
            .insert("app".to_string(), Context::App(Box::new(app_ctx)));

        let result = before_send(event).unwrap();
        let ctx = &result.contexts["app"];

        // Round-trip succeeded → must NOT be the scrub_error marker
        if let Context::Other(map) = ctx {
            assert!(
                !map.contains_key("scrub_error"),
                "scrub_error marker must not appear on successful round-trip"
            );
        }

        // Deserialize back to inspect typed fields
        let val = serde_json::to_value(ctx).unwrap();
        // app_name preserved
        assert_eq!(val["app_name"], serde_json::json!("hq"));
        // sensitive extra field scrubbed (sentry AppContext uses `other` flattened)
        assert_eq!(val["apikey"], serde_json::json!("[Filtered]"));
        // non-sensitive extra field preserved
        assert_eq!(val["build"], serde_json::json!("keep"));
    }

    // 6b. Typed Context::Runtime round-trip
    #[test]
    fn test_context_runtime_round_trip() {
        let mut event = Event::default();
        let mut other_fields = BTreeMap::new();
        other_fields.insert("token".to_string(), Value::String("leak".into()));
        let runtime_ctx = RuntimeContext {
            name: Some("rust".into()),
            version: Some("1.80".into()),
            other: other_fields,
            ..Default::default()
        };
        event
            .contexts
            .insert("runtime".to_string(), Context::Runtime(Box::new(runtime_ctx)));

        let result = before_send(event).unwrap();
        let ctx = &result.contexts["runtime"];
        let val = serde_json::to_value(ctx).unwrap();
        assert_eq!(val["name"], serde_json::json!("rust"));
        assert_eq!(val["token"], serde_json::json!("[Filtered]"));
    }

    // 6c. Context::Other round-trip
    #[test]
    fn test_context_other_round_trip() {
        let mut event = Event::default();
        let mut map = BTreeMap::new();
        map.insert("apikey".to_string(), Value::String("leak".into()));
        map.insert("other".to_string(), Value::String("keep".into()));
        event
            .contexts
            .insert("custom".to_string(), Context::Other(map));

        let result = before_send(event).unwrap();
        let ctx = &result.contexts["custom"];
        if let Context::Other(map) = ctx {
            assert_eq!(map["apikey"], Value::String("[Filtered]".into()));
            assert_eq!(map["other"], Value::String("keep".into()));
        } else {
            panic!("expected Context::Other");
        }
    }

    // 6d. Fail-closed: serde round-trip failure produces scrub_error marker.
    // Context::Other serializes its BTreeMap as-is (no "type" tag added).
    // Inserting "type": null produces JSON {"type": null, ...}. serde's
    // internally-tagged Context enum requires the tag to be a string, so
    // from_value::<Context> returns Err, and scrub_context returns the
    // scrub_error marker instead of the original (potentially unscrubbed) data.
    #[test]
    fn test_fail_closed_scrub_error_marker() {
        let mut map = BTreeMap::new();
        // null type violates Context's internally-tagged serde shape
        map.insert("type".to_string(), Value::Null);
        map.insert("secret".to_string(), Value::String("original_leak".into()));
        let ctx = Context::Other(map);

        let result = scrub_context(ctx);

        match result {
            Context::Other(marker) => {
                assert!(
                    marker.contains_key("scrub_error"),
                    "expected scrub_error key in marker, got: {:?}",
                    marker
                );
                assert_eq!(
                    marker["scrub_error"],
                    Value::String("[Filtered — serde round-trip failed]".into())
                );
                assert!(
                    !marker.contains_key("secret"),
                    "original secret must not appear in error marker"
                );
            }
            other => panic!("expected Context::Other marker, got: {:?}", other),
        }
    }

    // 7. Empty event no-panic
    #[test]
    fn test_empty_event_no_panic() {
        let event = Event::default();
        let result = before_send(event);
        assert!(result.is_some());
    }
}
