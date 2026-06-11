# HQ Sync Desktop V4 — Design Specification

Source of truth for the V4 desktop-window redesign. Approved by Corey on 2026-06-11.
Visual reference: the PNG exports in this directory (one per view, exported from the Paper file "HQ Desktop").
Design project: `companies/indigo/projects/hq-sync-desktop-redesign/` (HQ-side, 12/12 stories passed) plus two approved follow-on revisions (double-sidebar chrome with companies in the sidebar; Linear-style goal/project/task model; agent-native messaging surfaces).

## 1. Product model

The desktop window is three products in one quiet macOS shell:

1. **A work system** (Linear translated for humans + agents): **Goal** (objective + measurable key results, status on-track/at-risk/off-track, owner can be a person or "Agent") → **Project** (PRD-backed, lead = You/Agent/teammate, progress = stories passing) → **Task/story** (US-xxx, AC checklist, assignee You/Agent/teammate, priority P1–P3). Agents pick up unassigned P1s automatically.
2. **A messaging system** ("Slack where the third participant is the workforce"): DMs, channels, requests — plus a pinned **"Your agent"** conversation (delegation as chat), live **work-object unfurls** (story/goal/share cards rendered from the same data the board uses), **project channels** that interleave system events (story passed, deploy succeeded, needs review) with human chat, and a **catch-up digest** instead of unread walls.
3. **The sync substrate**: exception-based Home (health is one sentence; decisions are inline cards; activity is a narrative digest grouped by actor), plus safety flows (conflict resolution, drift, core update, bulk-asymmetry halt).

## 2. Design tokens

```css
/* surfaces */
--v4-ground:   #161618;   /* window + main content background */
--v4-raised:   #1B1B1D;   /* cards, primary sidebar */
--v4-inset:    #19191B;   /* secondary sidebar, inset panels, table containers */
--v4-hairline: rgba(255,255,255,0.07);   /* section borders, dividers */
--v4-rowline:  rgba(255,255,255,0.05);   /* table row separators */

/* text — hierarchy is done entirely with these three grays */
--v4-text-1: #F2F2F3;   /* primary */
--v4-text-2: #9A9AA0;   /* secondary */
--v4-text-3: #5D5D63;   /* tertiary / meta */

/* status — the ONLY color in the app, almost always as 6px dots */
--v4-ok:     #30D158;   /* ok / connected / running / on-track / success */
--v4-warn:   #FEBC2E;   /* needs attention / review / at-risk */
--v4-error:  #FF453A;   /* error / blocked / off-track; also destructive text actions */
--v4-unread: #0A84FF;   /* unread dots + sent bubbles — Messages surfaces ONLY */
--v4-idle:   #5D5D63;   /* idle / paused / gated / backlog */

/* controls */
--v4-control-bg:     rgba(255,255,255,0.10);  /* primary button fill */
--v4-control-border: rgba(255,255,255,0.12);  /* secondary button border */
--v4-control-faint:  rgba(255,255,255,0.06);  /* chips, search field, segmented track */
--v4-active-row:     rgba(255,255,255,0.08);  /* sidebar/list selection */
```

Card borders at ~0.3 alpha of warn/error are allowed only on needs-attention / error cards (e.g. `rgba(254,188,46,0.3)`). Diff highlights in conflict resolution use 0.07-alpha green/amber fills. **No purple anywhere** (hard Indigo policy).

## 3. Typography

Inter (SF Pro when packaging allows — metrics-compatible). **Exactly three sizes, two weights:**

| Role | Size / weight | Color |
|---|---|---|
| View title (one per view) | 14px / 500 | text-1 |
| Body, rows, buttons | 13px / 400 | text-1 or text-2 |
| Emphasis (names, card titles, active nav) | 13px / 500 | text-1 |
| Meta, column headers, timestamps, footnotes, chips | 11px / 400 | text-2 or text-3 |

No bold (600+), no other sizes, no mono. Column headers are 11px UPPERCASE text-3.

## 4. Chrome anatomy (every full-window view)

See `chrome-master.png`.

- **Title bar (40px)**: traffic lights · live sync status (6px dot + "All synced" 13px + "· 12 watched · just now" text-3) · right: one primary text action ("Sync Now" / "Cancel" / "Retry" contextually).
- **Primary sidebar (220px, raised bg, hairline right border)**: nav — Home, Companies, Messages, Meetings, Library (Skills/Workers fold into Library) — then a **COMPANIES** section (11px label) listing each connected company as a row (6px status dot + 13px name; gray dot = paused; "N more…" overflow row), spacer, **Settings footer** (13px "Settings" + 11px account email, hairline top border).
- **Secondary sidebar (200px, inset bg, hairline right border)** — contextual menu, only on surfaces that need it:
  - Company pages: context header (14px/500 company name + status dot, 11px "Owner · 3 members · synced just now") · rows **Overview / Goals / Projects / Tasks / Activity / Deployments / Secrets / Library** · footer "Company settings · sync rules · members · roles".
  - Library: header "Library" + 11px "~/.hq · counts" · rows **Skills / Workers / Installed / Marketplace / Profile** · footer "Publish a pack".
  - Settings: header "Settings" + version line · rows **Sync / Notifications / Updates / General / Meetings** (Meetings row carries 11px "gated") · footer "Sign out".
  - Messages keeps its 300px conversation list instead. Home, Meetings, safety flows, First Run have no secondary column.
- **Selection**: exactly one active row per sidebar — `--v4-active-row` background, text-1 at 500. Active primary item maps to the view (company pages highlight the company row, not a nav item; Settings highlights the footer).
- Sidebars always reach the window bottom.

## 5. Per-view specs

Each view's PNG is normative. Key behaviors:

- **home-healthy/syncing/error.png** — Home = health sentence (title bar) + 11px meta line + NEEDS YOU queue (inline-action cards: conflict → Keep mine / Take theirs / Compare; drift → Restore / Keep edit / View diff) + "Today across your companies" digest grouped by actor with expandable file rows (verb lane ADD/UPD/DEL in text-2 gray, NOT colored) + quiet "raw event log →" link. Syncing state: progress card (file counts, thin bar, per-company fanout rows, current-file line); title-bar action becomes Cancel. Error state: error card with plain-language message, Sign in again / Retry, collapsible "Technical details" inset with request line/runner version/journal+log paths; auto-retry note in meta line.
- **companies.png** — CONNECTED table (role, members, last change, sync mode), provisioning row (amber dot + "provisioning cloud storage…"), error row (red dot + Retry), NOT CONNECTED section (local-dir → Connect; invite with expiry → Accept/Decline).
- **company-overview.png** — stat strip (4 quiet stats) → GOALS section (two goal cards: name + status dot/word, KR one-liner, progress bar + %, "N projects · M stories in flight") → IN FLIGHT table with goal-chip lane, priority (11px P1/P2 text), AC progress (3px bar + n/m), status dot+word (Running/Review/Gated).
- **company-goals.png** — goal cards with full KR tables (name | current → target | progress bar+%), owner ("You" / "Agent") + target quarter, LINKED PROJECTS chips, at-risk note row ("agent proposed 2 new projects" + Review proposal).
- **company-projects.png** — project list grouped by goal (+ NO GOAL group with "Link" nudge): name+meta | LEAD (Agent text-2 / You text-1 / teammate avatar) | progress | target | status.
- **company-tasks.png** — issue list grouped IN PROGRESS / IN REVIEW / TODO / DONE·RECENT (done rows 60% opacity): P-lane (24px) | id lane (52px, text-3) | title | project chip | assignee lane. Footnote: "Agents pick up unassigned P1s automatically."
- **story-detail.png** — right panel (420px): story id + title, 11px hierarchy line "Goal → Project → US-xxx", description, STATUS segmented control, AC checklist with pass toggles (writes back via `set_local_story_passes`), labels, depends-on, footer actions.
- **project-detail.png** — rendered PRD/README, "Goal: …" chip, KEY RESULTS card, tasks roll-up rail with You/Agent ownership.
- **company-activity/deployments/secrets.png** — activity: actor-grouped feed + direction toggle + date chip. Deployments: env chips, status dot+word, Deploy + **Rollback in red** (destructive convention), inline rollback confirm row. Secrets: metadata-only + explicit "values are never shown here" note; **no reveal affordance may ever exist** (locked by `e2e/desktop-alt/secrets-never-leak.spec.ts`).
- **messages-*.png** — conversation list (300px): pinned **Your agent** row (bolt avatar, status preview), recency-sorted rows with previews/timestamps/unread dots, channels inline (# avatar), All/People/Channels filters. Thread: bubbles ≤420px r16 (received 10%-white, sent `--v4-unread` blue), 11px sender names in channels, date separators, Delivered receipts, reaction pills. **Card-in-bubble pattern** (11px caps header + icon, 13px body, optional inset sub-card, action row) used for: agent prompts (Copy prompt / Run now / Schedule / Decline), agent results, story/goal unfurls (live data + Open / Assign to my agent), vault shares (ACL truth line + audit system-line when an agent reads it), and channel system events (story passed / story created / deploy succeeded with red Rollback / needs review). Catch-up state: "While you were away" ranked digest cards + "Mark all read". Composer hints: "⌘P to attach a prompt", "/run hands work to an agent".
- **meetings.png / meeting-permissions.png** — today list with bot states (invite / scheduled / join now / recording + Stop), scheduled bots, recordings; TCC permissions wizard (Granted / Needs permission + Grant / Not requested + Request). Entire surface indigo-gated.
- **library.png / marketplace.png / creator-profile-moderation.png** — card grid + detail panel; marketplace listings with install/installed states, README preview, YOUR LISTINGS; creator profile editor + request-access variant; admin moderation queue (gated).
- **settings.png / first-run.png** — grouped macOS-style sections matching every `menubar.json` knob (toggles: 26×16 pills, on = green fill — the one non-dot color exception, matching macOS); gated rows annotated 11px "indigo-gated". First-run welcome card + one-time auto-sync notice.
- **conflict-resolution.png / drift-detail.png / core-update.png / sync-halted.png** — conflict: side-by-side panes with who/when, changed-region highlight, SELECTED treatment, Keep yours / Use this one / Merge both / Decide later, "Nothing is lost — kept in history for 30 days". Drift: MODIFIED/MISSING/ADDED rows, restore-vs-keep, user-only files untouched. Core update: available / in-progress / failed-with-log-tail states. **Sync halted (bulk asymmetry): abort-only — no override/force/"sync anyway" affordance may exist** (hard policy `hq-sync-bulk-asymmetry-breaker-means-abort`).
- **system-states.png / banners-palette.png** — empty states, skeleton bars, status bar, four notification banners (DM/share/update/meeting), ⌘K palette (NAVIGATE + ACTIONS sections, footer hint row).

## 6. Conventions that auditors/reviewers must not "fix"

- Destructive text actions (Rollback, Confirm rollback) are intentionally `--v4-error` red.
- Settings toggles use green fill when on (macOS convention).
- Conflict-pane diff tints (0.07-alpha green/amber) are functional, not decorative.
- Recording indicator red dot is universal convention.
- Blue is allowed only on Messages surfaces (sent bubbles + unread dots).

## 7. Data sources (all existing Tauri commands — no new backend required for P1)

Goals: `get_local_company_goals` · Projects/stories: `get_local_projects`, `get_company_board`, `get_local_project_prd/readme` · writes: `set_local_project_status`, `set_local_story_passes` · sync: `start_sync`/`cancel_sync` + SyncEvent stream, `get_sync_status`, `list_syncable_workspaces`, `connect_workspace_to_cloud` · conflicts: conflict store + `resolve_conflict`, `open_in_editor` · drift/core: `check_core_state`, `install_hq_core_update`, `restore_from_upstream`, `open_drift_detail` · messages: existing DM/channel/reaction/request commands · activity: `get_company_activity` · deployments: `get_company_deployments` · secrets: `get_company_secrets` (metadata only) · settings: `get_settings`/`save_settings` · library/marketplace: existing command families. Agent-thread delegation and live unfurls beyond story/goal data may ship as UI with stub wiring behind a flag.
