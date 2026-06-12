# V4 Implementation Notes

Last verified: 2026-06-12 on `feature/v4-redesign`.

## Visual QA Map

The implementation follows the approved references in `docs/design/v4/`:

- `chrome-master.png`, `system.png`: V4 shell, tokens, title bar, primary sidebar, secondary sidebar, and status bar.
- `home-healthy.png`, `home-syncing.png`, `home-error.png`: Home exception queue, sync progress, activity digest, and error card states.
- `companies.png`: Companies connection table, not-connected rows, invite actions, and provisioning/error rows.
- `company-overview.png`, `company-goals.png`, `company-projects.png`, `company-tasks.png`: company work-system overview, goals, projects, and tasks.
- `story-detail.png`, `project-detail.png`: hierarchy threading, checklist/status controls, project roll-up, PRD, and README rendering.
- `company-activity.png`, `company-deployments.png`, `company-secrets.png`: actor-grouped activity, deployment actions, and metadata-only secrets.
- `messages-*.png`: Messages shell, requests, channels, project channel context, work unfurls, catch-up, and Your Agent surfacing.
- `conflict-resolution.png`, `drift-detail.png`, `core-update.png`, `sync-halted.png`: safety flows and abort-only sync halted state.
- `settings.png`, `first-run.png`: V4 settings groups and updated first-run flow.
- `library.png`, `marketplace.png`, `creator-profile-moderation.png`: library, marketplace, profile, and moderation surfaces.
- `meetings.png`, `meeting-permissions.png`: gated Meetings page, meeting bot states, live recording controls, and TCC permission wizard.
- `banners-palette.png`, `system-states.png`: banner action surface, command palette grouping, status bar, skeletons, and empty states.

## Intentional Deviations

- The desktop-alt E2E suite uses the scripted harness unless `tauri-driver` is available. Live window screenshots were not captured in this verification run; the source-contract harness still checks the V4 structure, state wiring, and safety invariants.
- The Meetings page shows all synced upcoming meetings grouped by day instead of only today's meetings. This preserves the existing backend sync window and avoids an empty page when the next meeting is outside the local day.
- The command palette keeps keyboard navigation over one flat filtered result set while visually grouping rows under `ACTIONS` and `NAVIGATE`. This preserves Enter/arrow behavior while matching `banners-palette.png`.
- Banner notifications are implemented by one source-agnostic glass card with per-payload action labels rather than separate Svelte components per banner kind. DM, share, update, and meeting banners all route through the same action contract.

## Verification

- `npm run typecheck`
- `npm run lint`
- `npm test`
- `npm run test:e2e:desktop-alt`

Critical release guards:

- `e2e/desktop-alt/secrets-never-leak.spec.ts` keeps secrets metadata-only.
- `e2e/desktop-alt/safety-flows.spec.ts` verifies conflict, drift, core update, and sync-halted abort-only flows.
- Menubar/popover behavior remains outside the V4 desktop-alt route changes; changed V4 files are under `src/desktop-alt/` plus the shared `BannerNotification` source-contract coverage.
