# Desktop Alt UX

The desktop-alt UX is an Indigo-only dogfood surface for the macOS HQ Sync app. It adds a decorated secondary Tauri window while leaving the classic menubar popover as the default path.

## Access Model

- The popover asks `desktop_alt_enabled` on mount and passes the result into `Popover.svelte`.
- The toggle renders only inside `{#if desktopAltEnabled}` with `data-testid="desktop-alt-toggle"` and title `Open desktop view`.
- `open_desktop_alt_window` re-checks the same backend gate before showing or creating the window. Non-Indigo users get `desktop-alt is Indigo-only`.
- The gate delegates to `util::feature_gate::is_indigo_user()`; there is no separate desktop-alt allowlist, env var, or menubar preference.

## Window + Frontend Map

| Surface | Files |
| --- | --- |
| Tauri window declaration | `src-tauri/tauri.conf.json`, label `desktop-alt`, hidden at startup with `create: false` |
| Tauri capability | `src-tauri/capabilities/desktop-alt.json` |
| Rust command module | `src-tauri/src/commands/desktop_alt.rs` |
| Vite entry | `desktop-alt.html`, `src/desktop-alt/main.ts`, `vite.config.ts` `desktopAlt` input |
| Shell + route state | `src/desktop-alt/DesktopApp.svelte`, `src/desktop-alt/route.ts`, `src/desktop-alt/DesktopSidebar.svelte`, `src/desktop-alt/DesktopStatusBar.svelte` |
| Pages | `src/desktop-alt/pages/SyncPage.svelte`, `MeetingsPage.svelte`, `CompanyPage.svelte` |
| Company panels | `src/desktop-alt/panels/BoardPanel.svelte`, `ActivityPanel.svelte`, `DeploymentsPanel.svelte`, `SecretsPanel.svelte` |
| Global command surface | `src/desktop-alt/components/CommandPalette.svelte`, opened by command-K |

## Tauri Commands

All commands are registered in `src-tauri/src/main.rs`.

| Command | Purpose |
| --- | --- |
| `desktop_alt_enabled` | Returns the Indigo gate result. |
| `open_desktop_alt_window` | Shows/focuses an existing `desktop-alt` window or builds the 1180 x 760 decorated window. |
| `get_company_summary` | Returns placeholder counts for the company header. |
| `get_company_board` | Reads board data from the vault API at `/companies/{companyUid}/board`. |
| `get_company_activity` | Reads activity data from the vault API at `/companies/{companyUid}/activity`. |
| `get_company_deployments` | Reads hq-deploy apps from `https://api.indigo-hq.com/api/apps/me` with `x-org-slug`. |
| `get_company_secrets` | Reads hq-pro secrets metadata from `/secrets/{companyUid}` and returns grouped key metadata only. |

Company slugs are normalized in Rust, resolved through `list_syncable_workspaces`, and mapped to cloud company UIDs before vault API calls. A broken manifest UID can still resolve if the workspace row exposes the live cloud UID in its broken reason.

## Data + Security Notes

- Board and Activity are the surfaces that still need hq-console to hq-pro migration work. The inventory lives in `companies/indigo/projects/hq-sync-desktop-alt/references.md`.
- Deployments intentionally call hq-deploy directly; hq-deploy owns app rows, DNS state, deploy history, passwords, and share-token state.
- Secrets must never expose plaintext. `get_company_secrets` projects each row into `{ env, count, items: [{ key, upd, rot }] }`; parser and E2E coverage reject recursive `value` or `secret` fields.
- The desktop-alt capability grants only `core:default`, `core:event:default`, and `shell:allow-open`.

## Tests

Use the normal unit/story suite plus the desktop-alt E2E harness:

```bash
npm test
npm run test:e2e:desktop-alt
```

`npm run test:e2e:desktop-alt` runs a scripted source-contract harness by default. To exercise a live app through `tauri-driver`, set `HQ_SYNC_DESKTOP_ALT_LIVE=1` and `HQ_SYNC_DESKTOP_ALT_APP` or `HQ_SYNC_DESKTOP_ALT_APP_PATH`; `HQ_SYNC_DESKTOP_ALT_WEBDRIVER_URL` defaults to `http://127.0.0.1:4444`.
