# Desktop Alt UX

The desktop-alt UX is the GA desktop surface for signed-in HQ Sync users. It adds the V4 decorated Tauri window while leaving the classic menubar popover as the default path.

## Access Model

- The popover asks `desktop_alt_enabled` on mount and passes the result into `Popover.svelte`.
- The toggle renders only inside `{#if desktopAltEnabled}` with `data-testid="desktop-alt-toggle"` and title `Open desktop view`.
- `open_desktop_alt_window` re-checks the same backend gate before showing or creating the window. Signed-out users get `desktop-alt requires a signed-in user`.
- The gate delegates to `util::feature_gate::desktop_features_enabled()`, which admits signed-in users. Indigo-only checks still protect admin/pre-release surfaces such as Moderation and non-stable update channels.

## Window + Frontend Map

| Surface | Files |
| --- | --- |
| Tauri window declaration | `src-tauri/tauri.conf.json`, label `desktop-alt`, hidden at startup with `create: false` |
| Tauri capability | `src-tauri/capabilities/desktop-alt.json` |
| Rust command module | `src-tauri/src/commands/desktop_alt.rs` |
| Vite entry | `desktop-alt.html`, `src/desktop-alt/main.ts`, `vite.config.ts` `desktopAlt` input |
| Shell + route state | `src/desktop-alt/DesktopApp.svelte`, `src/desktop-alt/route.ts`, `src/desktop-alt/v4/V4Sidebar.svelte`, `src/desktop-alt/v4/V4SecondarySidebar.svelte`, `src/desktop-alt/v4/V4TitleBar.svelte`, `src/desktop-alt/DesktopStatusBar.svelte` |
| Pages | `src/desktop-alt/pages/HomePage.svelte`, `CompaniesPage.svelte`, `CompanyPage.svelte`, `CompanyGoalsPage.svelte`, `CompanyProjectsPage.svelte`, `CompanyTasksPage.svelte`, `MessagesPage.svelte`, `MeetingsPage.svelte`, `LibraryPage.svelte`, `SettingsPage.svelte`, `ConflictResolutionPage.svelte`, `DriftDetailPage.svelte`, `ProjectDetailView.svelte` |
| Company panels | `src/desktop-alt/panels/CompanyBoardPanel.svelte`, `ActivityPanel.svelte`, `DeploymentsPanel.svelte`, `SecretsPanel.svelte`, `CompanyLibraryPanel.svelte` |
| Global command surface | `src/desktop-alt/components/CommandPalette.svelte`, opened by command-K and grouped into actions/navigation rows |

## Tauri Commands

All commands are registered in `src-tauri/src/main.rs`.

| Command | Purpose |
| --- | --- |
| `desktop_alt_enabled` | Returns the Indigo gate result. |
| `open_desktop_alt_window` | Shows/focuses an existing `desktop-alt` window or builds the decorated window. The window is built `transparent(true)` and gets its native macOS glass backing applied via `glass::apply_liquid_glass_window` immediately after build (0.8.1-beta.1). |
| `get_company_project_creators` | Fetches the cloud board for a company and returns only projects with a non-empty creator (derived from the prd's S3 `created-by` metadata) for the Projects-list **Lead** column (0.8.2-beta.1). Projects with no stamped creator are omitted and stay "Unassigned" in the UI. Pure parse is `parse_project_creators`, unit-tested in `desktop_alt.rs`. |
| `get_company_summary` | Returns counts for the company header and overview stats. |
| `get_company_board` | Reads board data from the vault API at `/companies/{companyUid}/board`. |
| `get_company_activity` | Reads activity data from the vault API at `/companies/{companyUid}/activity`. |
| `get_company_deployments` | Reads hq-deploy apps from `https://api.indigo-hq.com/api/apps/me` with `x-org-slug`. |
| `get_company_secrets` | Reads hq-pro secrets metadata from `/secrets/{companyUid}` and returns grouped key metadata only. |
| `get_local_company_goals`, `get_local_projects`, `get_local_project_prd`, `get_local_project_readme` | Read local HQ work-system data for V4 goals, projects, tasks, and detail views. |
| `set_local_project_status`, `set_local_story_passes` | Write V4 project and story status changes back to local project files. |

Company slugs are normalized in Rust, resolved through `list_syncable_workspaces`, and mapped to cloud company UIDs before vault API calls. A broken manifest UID can still resolve if the workspace row exposes the live cloud UID in its broken reason.

## Data + Security Notes

- V4 reads work-system data from local HQ goals/projects where possible, while Activity and Deployments still use their existing service-backed command paths.
- Deployments intentionally call hq-deploy directly; hq-deploy owns app rows, DNS state, deploy history, passwords, and share-token state.
- Secrets must never expose plaintext. `get_company_secrets` projects each row into `{ env, count, items: [{ key, upd, rot }] }`; parser and E2E coverage reject recursive `value` or `secret` fields.
- The desktop-alt capability grants only `core:default`, `core:event:default`, and `shell:allow-open`.

## Window Appearance

- **Liquid Glass window (0.8.1-beta.1).** The desktop window is built `transparent(true)`; `src-tauri/src/glass.rs::apply_liquid_glass_window` inserts a native `NSGlassEffectView` at the very back of the content view on macOS 26 (Tahoe) so the window reads as real Liquid Glass over the desktop. On older macOS the class is resolved at runtime as absent and the code falls back to the same `NSVisualEffectView` `UnderWindowBackground` vibrancy the menubar popover uses, so every supported OS still gets a translucent window. AppKit is main-thread-only, so this runs via `run_on_main_thread`. The native material backs the *window*; in-window panels get matched translucent styling in CSS (the material cannot refract the webview's own DOM).
- **Light-mode adaptivity (0.8.2-beta.1).** The V4 surface follows the OS appearance. `src/desktop-alt/v4/tokens.css` carries a light token set under `prefers-color-scheme: light`, and `src/desktop-alt/styles/desktop-alt.css` carries matching light glass/surface overrides. Previously the desktop window was dark-only.
- **HQ console links.** Every external link the desktop window opens into the HQ web console is centralized in `src/desktop-alt/lib/hq-console.ts` (base `https://hq.getindigo.ai`) — Company Settings (the console company page), invite, integrations, and creator profiles. Links open in the system browser via `@tauri-apps/plugin-shell`.

## Tests

Use the normal unit/story suite plus the desktop-alt E2E harness:

```bash
npm test
npm run test:e2e:desktop-alt
```

`npm run test:e2e:desktop-alt` runs a scripted source-contract harness by default. To exercise a live app through `tauri-driver`, set `HQ_SYNC_DESKTOP_ALT_LIVE=1` and `HQ_SYNC_DESKTOP_ALT_APP` or `HQ_SYNC_DESKTOP_ALT_APP_PATH`; `HQ_SYNC_DESKTOP_ALT_WEBDRIVER_URL` defaults to `http://127.0.0.1:4444`.
