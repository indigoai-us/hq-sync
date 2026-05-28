//! Feature gate for the alt desktop UX surface.
//!
//! Indigo-only gate for the alternate popover/desktop UX in development.
//! Delegates entirely to `feature_gate::is_indigo_user()` — there is no
//! parallel cache (PRD US-001 hard rule: reuse the existing OnceLock cache).
//!
//! On cold start (cache uninitialised) the underlying `is_indigo_user()`
//! call awaits `compute_gate()` and returns the canonical email-derived
//! answer instead of falling back to false. This matters because the
//! popover mounts and invokes the gate before any cloud round-trip has
//! had a chance to seed an unrelated cache — we owe the caller the real
//! answer, not a default.
//!
//! See `src-tauri/src/commands/meetings.rs::meetings_feature_enabled` for
//! the reference pattern this command mirrors.
//!
//! Result type is `Result<bool, String>` to match the established gate
//! command shape, but `is_indigo_user()` itself never errors — the Ok arm
//! is always taken.
use tauri::{AppHandle, Manager};

const WINDOW_LABEL: &str = "desktop-alt";

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanyActivitySummary {
    pub last7d: u32,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanySummary {
    pub board: u32,
    pub activity: CompanyActivitySummary,
    pub deployments: u32,
    pub secrets: u32,
}

#[tauri::command]
pub async fn desktop_alt_enabled() -> Result<bool, String> {
    Ok(crate::util::feature_gate::is_indigo_user().await)
}

#[tauri::command]
pub async fn get_company_summary(slug: String) -> Result<CompanySummary, String> {
    if slug.trim().is_empty() {
        return Err("company slug is required".to_string());
    }

    Ok(CompanySummary {
        board: 0,
        activity: CompanyActivitySummary { last7d: 0 },
        deployments: 0,
        secrets: 0,
    })
}

/// Open or focus the Indigo-only alternate desktop UX window.
///
/// The window is declared in `tauri.conf.json` as hidden, so normal app
/// startup does not surface it. This command is still defensive and can
/// rebuild the window if it was closed earlier in the session.
#[tauri::command]
pub async fn open_desktop_alt_window(app: AppHandle) -> Result<(), String> {
    if !desktop_alt_enabled().await? {
        return Err("desktop-alt is Indigo-only".to_string());
    }

    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        WINDOW_LABEL,
        tauri::WebviewUrl::App("desktop-alt.html".into()),
    )
    .title("HQ")
    .inner_size(1180.0, 760.0)
    .min_inner_size(960.0, 600.0)
    .resizable(true)
    .decorations(true)
    .title_bar_style(tauri::TitleBarStyle::Overlay)
    .transparent(false)
    .visible(true)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::util::feature_gate::is_allowed_email;

    // Note: `desktop_alt_enabled` itself depends on the on-disk Cognito
    // token cache so it isn't a pure unit-test target — the canonical
    // gate logic it delegates to is covered by the unit tests in
    // `util/feature_gate.rs` (test_positive_cases / test_negative_cases),
    // plus the command-specific assertions below that re-exercise the
    // allowlist contract this command is bound to.

    /// US-001 AC #4: command-path positive case for `@getindigo.ai`.
    #[test]
    fn desktop_alt_gate_admits_indigo_email() {
        assert!(is_allowed_email(Some("stefan@getindigo.ai")));
        assert!(is_allowed_email(Some("STEFAN@GetIndigo.AI")));
    }

    /// US-001 AC #4: command-path negative case for non-allowed emails.
    #[test]
    fn desktop_alt_gate_rejects_non_indigo_email() {
        assert!(!is_allowed_email(Some("someone@gmail.com")));
        assert!(!is_allowed_email(Some("admin@notindigo.ai")));
        // Look-alike — leading `@` in ALLOWED_DOMAIN blocks suffix match
        // on `forgetindigo.ai`.
        assert!(!is_allowed_email(Some("attacker@forgetindigo.ai")));
    }

    /// US-001 AC #4: missing/empty emails return false (never default-true).
    #[test]
    fn desktop_alt_gate_rejects_missing_email() {
        assert!(!is_allowed_email(None));
        assert!(!is_allowed_email(Some("")));
    }

    #[tokio::test]
    async fn company_summary_returns_placeholder_counts() {
        let summary = super::get_company_summary("acme".to_string())
            .await
            .expect("valid slug should return a summary");

        assert_eq!(summary.board, 0);
        assert_eq!(summary.activity.last7d, 0);
        assert_eq!(summary.deployments, 0);
        assert_eq!(summary.secrets, 0);
    }

    #[tokio::test]
    async fn company_summary_rejects_empty_slug() {
        assert_eq!(
            super::get_company_summary("".to_string()).await.unwrap_err(),
            "company slug is required"
        );
        assert_eq!(
            super::get_company_summary("   ".to_string())
                .await
                .unwrap_err(),
            "company slug is required"
        );
    }
}
