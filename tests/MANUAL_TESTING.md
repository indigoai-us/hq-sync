# HQ Sync Menubar — Manual Testing Checklist

> **Policy deviation:** This project uses manual testing + Loom video for V1 instead of automated e2e tests. See [e2e-backpressure-required.md policy deviation](#policy-deviation) at the bottom of this document.

## How to Use This Checklist

1. Run through each section on a **fresh macOS VM** (macOS 13 Ventura or later).
2. Mark each step with `[x]` when it passes, or note the failure.
3. Record a **Loom video** walking through the entire checklist before each release.
4. Publish the Loom video link in the GitHub Release notes.

---

## Environment Setup

### Prerequisites

- macOS 13.0+ (Ventura, Sonoma, or Sequoia)
- Fresh user account (or clean `~/.hq/` state)
- `hq` CLI installed and on PATH (`which hq` returns a path)
- Valid Indigo Cognito account for OAuth testing
- Network access to AWS Cognito (us-east-1) and HQ sync backend
- A second machine or simulated remote for conflict testing (UJ-003)

### Reset Procedure (between test runs)

```bash
# Back up existing state if needed
cp -r ~/.hq ~/.hq.backup.$(date +%s)

# Remove menubar preferences
rm -f ~/.hq/menubar.json

# Remove app from /Applications (if testing fresh install)
rm -rf "/Applications/HQ Sync.app"

# Kill any running menubar instances
pkill -f "HQ Sync" || true
```

---

## User Journey Tests

### UJ-001: First Install to First Sync in <5 Minutes, Zero Terminal

**Goal:** A new user reaches first successful sync without ever opening Terminal.app.

**Stories involved:** US-001, US-003, US-005, US-008, US-009, US-010, US-013

**Prerequisites:**
- Fresh macOS machine (or reset state per above)
- hq-installer completed (auth, company, HQ folder chosen)
- `~/.hq/config.json` and `~/.hq/cognito-tokens.json` exist

**Steps:**

- [ ] 1. Install HQ Sync.app into /Applications (via installer bundle or DMG)
- [ ] 2. Launch HQ Sync.app — verify tray icon appears in menu bar within 5 seconds
- [ ] 3. Click tray icon — verify popover opens in <100ms
- [ ] 4. Verify popover shows: company name, HQ folder path, "Sync Now" button
- [ ] 5. Verify authentication state shows "authenticated" (inherited from `~/.hq/cognito-tokens.json`)
- [ ] 6. Click "Sync Now" — verify progress indicator appears
- [ ] 7. Wait for sync completion — verify completion timestamp appears (e.g., "Just now")
- [ ] 8. Verify total elapsed time from step 1 to step 7 is **under 5 minutes**
- [ ] 9. Verify Terminal.app was **never opened** during the entire flow

**Expected outcome:** User completes first sync in <5 min with zero terminal interaction. Tray icon transitions: idle -> syncing -> idle.

---

### UJ-002: Returning User — Expired Token Silent Refresh

**Goal:** Menubar silently refreshes an expired Cognito access token without user interruption.

**Stories involved:** US-003, US-008

**Prerequisites:**
- HQ Sync.app installed and previously authenticated
- Valid refresh token (within 30-day TTL)

**Steps:**

- [ ] 1. Quit HQ Sync.app (`Cmd+Q` or right-click tray -> Quit)
- [ ] 2. Manually expire the access token:
  ```bash
  # Edit ~/.hq/cognito-tokens.json
  # Set "expiresAt" to a past timestamp, e.g.:
  # "expiresAt": "2020-01-01T00:00:00.000Z"
  ```
- [ ] 3. Launch HQ Sync.app
- [ ] 4. Click tray icon — verify popover opens without any error or "Sign in" prompt
- [ ] 5. Verify auth state shows "authenticated" with a **new** `expiresAt` in the future
- [ ] 6. Click "Sync Now" — verify sync completes successfully
- [ ] 7. Verify `~/.hq/cognito-tokens.json` has been updated with new access token and future `expiresAt`
- [ ] 8. Verify **no error dialogs**, **no browser windows**, and **no "Sign in" prompts** appeared

**Expected outcome:** Token refresh happens transparently. User sees no interruption. Sync works immediately.

---

### UJ-003: Sync Conflict — Resolve in Popover Modal, No Terminal

**Goal:** User resolves file conflicts entirely through the popover GUI.

**Stories involved:** US-006, US-007, US-011

**Prerequisites:**
- HQ Sync.app installed and authenticated
- Two machines (or simulated remote) pointing to the same HQ folder
- Files to create conflicts with (text file, binary/image file, directory)

#### Scenario A: Text File Conflict

- [ ] 1. On Machine A, edit `~/HQ/notes/test.md` — add "local edit" to line 1
- [ ] 2. On Machine B (or simulate remote), edit the same file — add "remote edit" to line 1
- [ ] 3. Sync Machine B first (so remote has the "remote edit" version)
- [ ] 4. On Machine A, click "Sync Now"
- [ ] 5. Verify conflict modal appears in popover listing `notes/test.md`
- [ ] 6. Verify modal shows local vs remote indicators (timestamp, file size)
- [ ] 7. Select "Keep Local" for the file
- [ ] 8. Verify sync resumes and completes
- [ ] 9. Verify file contains the local version ("local edit")

#### Scenario B: Binary File Conflict

- [ ] 1. Create a test image `~/HQ/assets/logo.png` on both machines with different content
- [ ] 2. Sync one machine first to establish remote version
- [ ] 3. On the other machine, click "Sync Now"
- [ ] 4. Verify conflict modal appears listing `assets/logo.png`
- [ ] 5. Select "Keep Remote" for the file
- [ ] 6. Verify sync completes and local file matches remote version

#### Scenario C: Directory Conflict

- [ ] 1. On Machine A, create a new directory `~/HQ/projects/new-project/` with files
- [ ] 2. On Machine B, create the same directory path with different files
- [ ] 3. Sync Machine B first
- [ ] 4. On Machine A, click "Sync Now"
- [ ] 5. Verify conflict modal appears listing directory conflicts
- [ ] 6. Resolve each conflict via the modal
- [ ] 7. Verify sync completes cleanly

#### Post-Conflict Verification

- [ ] 8. Run a second sync on Machine A — verify it completes with **no conflicts** (clean sync)
- [ ] 9. Verify **no terminal interaction** was required for any resolution

**Expected outcome:** All 3 conflict types resolved entirely via GUI. Second sync is clean.

---

### UJ-004: Retether — User Changes HQ Path via Settings

**Goal:** User changes the HQ folder path through Settings and sync operates against the new path.

**Stories involved:** US-005, US-012

**Prerequisites:**
- HQ Sync.app installed and authenticated
- Current HQ path is `~/HQ` (or whatever default)
- A second valid HQ folder exists (e.g., `~/HQ-alt`)

**Steps:**

- [ ] 1. Note current HQ path displayed in popover header
- [ ] 2. Right-click tray icon -> select "Settings" (or click Settings button in popover)
- [ ] 3. Verify Settings window opens showing current HQ path
- [ ] 4. Click "Change HQ path" / "Change..." button
- [ ] 5. Verify native macOS folder picker (NSOpenPanel) appears
- [ ] 6. Select a new folder (e.g., `~/HQ-alt`)
- [ ] 7. Verify Settings window updates to show new path
- [ ] 8. Close Settings, open popover — verify popover header shows new HQ path
- [ ] 9. Click "Sync Now" — verify sync operates against the **new** path
- [ ] 10. Verify `~/.hq/menubar.json` contains the updated `hqPath` value
- [ ] 11. Verify old HQ folder (`~/HQ`) is **untouched** (no data loss, no journal deletion)
- [ ] 12. Quit and relaunch app — verify new path persists

**Expected outcome:** Path change via Settings is immediate, persists across restart, and old data is preserved.

---

### UJ-005: Auto-Update — New Version Installed Silently

**Goal:** Tauri updater detects, downloads, and installs a new version without data loss.

**Stories involved:** US-015, US-016

**Prerequisites:**
- HQ Sync.app v1.0.0 (or test version) installed
- Access to publish a new version to GitHub Releases
- `latest.json` endpoint configured in `tauri.conf.json`

**Steps:**

- [ ] 1. Install v1.0.0 of HQ Sync.app
- [ ] 2. Record current state: note contents of `~/.hq/menubar.json`, `~/.hq/cognito-tokens.json`, and sync journal
- [ ] 3. Publish v1.0.1 to GitHub Releases with updated `latest.json`
- [ ] 4. Relaunch HQ Sync.app (or wait for periodic check — up to 6 hours)
- [ ] 5. Verify update prompt appears: "Restart to install update" (or similar)
- [ ] 6. Accept the update — verify app restarts
- [ ] 7. Verify app is now running v1.0.1 (check About / version display)
- [ ] 8. Verify `~/.hq/menubar.json` is **unchanged** (settings preserved)
- [ ] 9. Verify `~/.hq/cognito-tokens.json` is **unchanged** (auth preserved)
- [ ] 10. Verify sync journal is **unchanged** (sync history preserved)
- [ ] 11. Click "Sync Now" — verify sync works without re-authentication

#### Update Refusal Path

- [ ] 12. Repeat steps 1-4 with a new version
- [ ] 13. **Decline** the update when prompted
- [ ] 14. Verify app continues running on current version
- [ ] 15. Verify prompt re-appears on next app launch

**Expected outcome:** Update installs cleanly with zero data loss. No re-auth required. Refusal is respected and re-prompted.

---

### UJ-006: Auto-Provisioning + Personal HQ

**Goal:** Verify that unprovisioned companies are auto-created server-side and that personal (non-company) content is mirrored to the user's personal S3 bucket.

**Stories involved:** Steps 5, 6, 7, 8 (provision_missing_companies, first_push, personal provision + first-push)

**Prerequisites:**

- Fresh `~/.hq/` state: no `companies/*/` directories yet created locally; vault has exactly 1 personal `prs_*` entity
- `~/.hq/config.json` exists (written by hq-installer)
- At least one `companies/<slug>/company.yaml` with `cloud: true` exists under `${HQ_FOLDER}/companies/<slug>/` that has no matching `cmp_*` entity yet
- Personal content (non-`companies/*` files) exists under `${HQ_FOLDER}/` (e.g., `notes/intro.md`)
- Staging-binding block (plan.md) exported: `$STAGE`, `$API_HOST`, `$PERSON_UID`

**Steps:**

- [ ] 1. Confirm pre-state: `ls "${HQ_FOLDER}/companies/"` shows at least one slug without a `.hq/config.json` inside
- [ ] 2. Click "Sync Now" in the HQ Sync menubar popover
- [ ] 3. Wait for sync completion (popover shows "Sync complete")

**Expected outcome (a) — Company auto-provisioning:**

- The company folder `${HQ_FOLDER}/companies/<slug>/` gets a `.hq/config.json` written with `companyUid`, `companySlug`, `bucketName`, and `vaultApiUrl` keys
- A new `cmp_*` entity appears in the vault for that slug (find_by_slug → create path)
- An S3 bucket `hq-vault-cmp-<new-slug>` is reachable

**Expected outcome (b) — Personal first-push via /sts/vend-self:**

- Personal content (anything NOT under `companies/*`) is uploaded to `s3://hq-vault-prs-<personal-uid>/`
- `~/.hq/sync-journal.personal.json` exists with `version == "1"` and `files` keys > 0
- The personal-mode sync runner authenticates by calling **`POST /sts/vend-self`** with `body.personUid` matching the caller's resolved person entity (NOT `/sts/vend-child`). Verify via:
  ```bash
  START_TIME_MS=$(($(date +%s) * 1000 - 300000))
  aws logs filter-log-events \
    --log-group-name "/aws/lambda/$VEND_SELF_LAMBDA_NAME" \
    --filter-pattern '"vend-self"' \
    --start-time "$START_TIME_MS" \
    | jq '.events | length'
  # => ≥1 (vend-self was called)
  aws logs filter-log-events \
    --log-group-name "/aws/lambda/$VEND_CHILD_LAMBDA_NAME" \
    --filter-pattern '"vend-child"' \
    --start-time "$START_TIME_MS" \
    | jq '.events | length'
  # => 0 (vend-child was NOT used for personal sync)
  ```

**Verification:**

```bash
# Resolve UIDs from staging-binding block
source <(cat plan.md | grep -A50 'staging-binding block')  # or set manually

# Company bucket reachable with at least top-level listing
aws s3 ls "s3://hq-vault-cmp-<new-slug>/"

# Personal bucket contains non-companies/* content
aws s3 ls "s3://hq-vault-prs-${PERSON_UID}/"

# Journal written
jq -r 'keys | length' ~/.hq/sync-journal.personal.json
# => returns integer > 0

# company.yaml MUST be byte-for-byte unchanged
sha256sum "${HQ_FOLDER}/companies/<slug>/company.yaml"
# compare against pre-test hash recorded in step 1
```

---

### UJ-007: Telemetry Opt-In Round-Trip

**Goal:** Verify the full telemetry pipeline — opt-in flag propagation, JSONL scanning, strip-list enforcement, DynamoDB storage, and cursor advancement.

**Stories involved:** Steps 1, 3, 11, 12 (usage routes, installer opt-in, machineId, telemetry collector)

**Prerequisites:**

- Dev Cognito user opted-in via the installer wizard (Step 3); `~/.hq/menubar.json` must contain `"telemetryEnabled": true`
- At least one `~/.claude/projects/**/*.jsonl` file exists with ≥1 JSON line containing sensitive fields (`content`, `thinking`, or `text`)
- Staging-binding block exported: `$STAGE`, `$API_HOST`, `$PERSON_UID`, `$JWT_SUB`

**Steps:**

- [ ] 1. Confirm opt-in: `jq -r '.telemetryEnabled' ~/.hq/menubar.json` → `true`
- [ ] 2. Confirm at least one JSONL exists: `ls ~/.claude/projects/**/*.jsonl | head -3`
- [ ] 3. Click "Sync Now" in the HQ Sync menubar popover
- [ ] 4. Wait for sync completion (popover shows "Sync complete")

**Expected outcome:**

On sync completion, `send_telemetry_if_opted_in` fires via `tauri::async_runtime::spawn`. It:

1. Calls `GET /v1/usage/opt-in` (returns `{ "enabled": true }`)
2. Scans `~/.claude/projects/**/*.jsonl` starting from stored cursor offsets in `~/.hq/telemetry-cursor.json`
3. Applies the KEEP/REMOVE allowlist (unknown fields dropped by default; `content`, `thinking`, `text` never survive)
4. Batches rows up to ~1 MB
5. POSTs to `/v1/usage` (no top-level `personUid` — server resolves from JWT)
6. Advances the cursor **only** on HTTP 200

**Verification:**

```bash
# Strip-list enforcement: no prompt body field survives
aws dynamodb scan \
  --table-name "hq-vault-usage-events-${STAGE}" \
  --limit 5 \
  | jq '.Items[] | keys | map(select(. == "content" or . == "thinking" or . == "text"))'
# => must return [] for every row

# Spoof rejection guard
curl -s -o /dev/null -w "%{http_code}" \
  -X POST "${API_HOST}/v1/usage" \
  -H "Authorization: Bearer ${ACCESS_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"events":[{"eventKey":"spoof-test"}],"personUid":"prs_forged"}'
# => must return 400

# Confirm forged UID wrote nothing
aws dynamodb query \
  --table-name "hq-vault-usage-events-${STAGE}" \
  --key-condition-expression "personUid = :p" \
  --expression-attribute-values '{":p":{"S":"prs_forged"}}' \
  | jq '.Count'
# => must return 0

# Cursor file written and non-empty
jq -r 'keys | length' ~/.hq/telemetry-cursor.json
# => returns integer > 0 after first sync

# Cursor stores non-negative byte offset per file
jq -r 'to_entries[0].value.offset' ~/.hq/telemetry-cursor.json
# => returns a non-negative integer
```

---

## Per-Story Acceptance Tests

### US-001: Repo Scaffold + Tauri Dev

- [ ] Clone repo: `git clone git@github.com:indigoai-us/hq-sync.git`
- [ ] Run `npm install` — completes without errors
- [ ] Run `npm run tauri dev` — Tauri window opens on macOS
- [ ] Verify `tauri.conf.json` has bundle ID `ai.indigo.hq-sync-menubar`
- [ ] Verify `tauri.conf.json` targets macOS 13.0 minimum
- [ ] Verify universal binary build target is configured

### US-002: Rust Reusables + Tauri Permissions

- [ ] Run `cargo check --manifest-path=src-tauri/Cargo.toml` — passes
- [ ] Run `cargo test --manifest-path=src-tauri/Cargo.toml` — all tests pass
- [ ] Verify `process.rs` exists and matches hq-installer source (subprocess runner with SIGTERM/SIGKILL)
- [ ] Verify `oauth.rs` exists and is adapted for HQ Sync context
- [ ] Verify Tauri permissions allow: shell execute, fs read (`~/.hq/`), HTTPS network, tray icon

### US-003: Cognito Token Inherit + Native Refresh

- [ ] **No tokens:** Remove `~/.hq/cognito-tokens.json` -> `get_auth_state()` returns `{authenticated: false}`
- [ ] **Valid tokens:** Restore valid tokens -> `get_auth_state()` returns `{authenticated: true}` with future `expiresAt`
- [ ] **Expired access + valid refresh:** Set `expiresAt` to past -> silent refresh occurs -> returns `{authenticated: true}` with new `expiresAt`
- [ ] **No Keychain ACL prompts** observed during or after refresh

### US-004: Native OAuth Login Flow

- [ ] Delete `~/.hq/cognito-tokens.json`
- [ ] Open menubar — verify "Sign in to HQ" button appears
- [ ] Click "Sign in" — system browser opens to Cognito hosted UI
- [ ] Complete sign-in in browser
- [ ] Browser shows "You may close this tab"
- [ ] Menubar popover updates to authenticated state
- [ ] Verify `~/.hq/cognito-tokens.json` is written with valid tokens

### US-005: Config Reading + HQ Path Detection

- [ ] **Config present:** Valid `~/.hq/config.json` -> popover shows company name + HQ path
- [ ] **Config absent:** Remove `~/.hq/config.json` -> error state with download/installer link
- [ ] **Menubar override:** Create `~/.hq/menubar.json` with custom `hqPath` -> popover shows overridden path

### US-007: Sync Command + Event Streaming

- [ ] Click "Sync Now" -> progress events stream to UI in real time
- [ ] Completion event updates last-synced timestamp
- [ ] Kill app mid-sync (`kill -9 $(pgrep "HQ Sync")`) -> verify no zombie `hq` process (`ps aux | grep hq`)
- [ ] Trigger error (disable network) -> error event reaches UI with readable message
- [ ] Cancel sync mid-run (if cancel button exists) -> subprocess terminates cleanly

### US-009: Tray Icon State Swap

- [ ] App launch -> tray icon appears (idle state, monochrome)
- [ ] Start sync -> icon changes to syncing state
- [ ] Sync completes -> icon returns to idle state
- [ ] Trigger error -> icon shows error state (red badge)
- [ ] Trigger conflict -> icon shows conflict state (amber badge)
- [ ] Verify all states in **light mode**
- [ ] Verify all states in **dark mode**
- [ ] Click tray icon -> popover opens
- [ ] Right-click tray icon -> context menu shows: Sync Now, Settings, Quit

### Meeting detect-notify (US-001, US-004, US-005, US-006, US-007)

> These steps require the Recall Desktop SDK to be installed and the user to be signed into Recall.
> Run after building with `npm run tauri dev` or from a packaged `.app`.

#### MDN-001: Recall SDK sidecar starts on launch

- [ ] Launch HQ Sync.app
- [ ] Verify `~/.hq/sync-debug.log` does NOT contain `RECALL_SDK_UNAVAILABLE`
- [ ] With the SDK binary absent: rename it, relaunch → verify `RECALL_SDK_UNAVAILABLE` is written to the log and the app continues running (no crash, tray icon appears)
- [ ] Restore the binary and relaunch → verify the error disappears

#### MDN-002: Synthetic detected event surfaces a macOS notification

- [ ] Inject a synthetic `meeting:detected` Tauri event for a Zoom URL (via `__TAURI_INVOKE__` in devtools or a test harness):
  ```js
  window.__TAURI__.event.emit('meeting:detected', {
    detectionId: 'test-1',
    meetingUrl: 'https://zoom.us/j/999000111',
    platform: 'zoom',
    detectedAt: new Date().toISOString(),
    source: 'sdk-active-app'
  })
  ```
- [ ] Verify a macOS notification appears with title `Meeting detected` and body containing `zoom`
- [ ] Verify the tray icon gains a badge dot (Prompt state)
- [ ] Click the tray icon → open MeetingsWindow → verify the badge dot clears

#### MDN-003: Dedup against an existing bot suppresses the notification

- [ ] Start a bot for the test Zoom URL (via MeetingsWindow or curl `POST /v1/bot/invite`)
- [ ] Inject the same `meeting:detected` event for that URL
- [ ] Verify **no** new macOS notification fires
- [ ] Verify the tray badge does **not** appear (or does not increment if already showing)
- [ ] Verify `sync-debug.log` contains a `dedup-hit` or suppression log line

#### MDN-004: Record action wires through bot invite with ontology participants

- [ ] Inject a `meeting:detected` event for a URL with no existing bot
- [ ] From the resulting notification (or MeetingsWindow row), click **Record**
- [ ] Verify a bot invite is sent (`POST /v1/bot/invite` in network tab or DDB record created)
- [ ] If the company has ontology person entities, verify `metadata.participants` is non-empty on the bot record in DynamoDB

#### MDN-005: Ledger deduplication persists across restarts

- [ ] Inject a `meeting:detected` event → notification fires
- [ ] Quit HQ Sync.app
- [ ] Verify `~/.hq/meeting-notify-ledger.json` contains an entry for the test URL with `action: "notified"`
- [ ] Relaunch HQ Sync.app
- [ ] Inject the same `meeting:detected` event within 6 hours of the first
- [ ] Verify **no** duplicate notification fires (ledger suppressed it)
- [ ] Manually edit the `notifiedAt` timestamp in the ledger to be >6 hours ago
- [ ] Inject the event again → verify the notification fires (suppression window expired)

#### MDN-006: Per-platform disable suppresses events for that platform

- [ ] Open Settings → Meeting detection section
- [ ] Uncheck `Zoom` while leaving others checked
- [ ] Inject a `meeting:detected` event with `platform: 'zoom'`
- [ ] Verify **no** notification fires
- [ ] Inject a `meeting:detected` event with `platform: 'meet'`
- [ ] Verify a notification **does** fire (Google Meet is still enabled)
- [ ] Re-enable Zoom in Settings — verify subsequent Zoom events surface again (no restart required)

#### MDN-007: Global detection toggle off suppresses all events

- [ ] Open Settings → Meeting detection → toggle `Detect upcoming meetings` to OFF
- [ ] Inject `meeting:detected` events for zoom, meet, teams
- [ ] Verify **no** notifications fire and tray badge does not appear
- [ ] Toggle back ON → verify next event surfaces immediately (no restart)

#### MDN-008: notifications:false in menubar.json suppresses everything

- [ ] Quit HQ Sync.app
- [ ] Edit `~/.hq/menubar.json` to set `"notifications": false`
- [ ] Relaunch HQ Sync.app
- [ ] Inject a `meeting:detected` event with no existing bot
- [ ] Verify **no** macOS notification fires and **no** tray badge appears

---

### US-015: Code Signing + Notarization CI

- [ ] Push a git tag `v0.x.x` -> GitHub Actions workflow triggers
- [ ] Workflow completes successfully
- [ ] Signed + notarized DMG appears in GitHub Releases
- [ ] Verify signature: `spctl -a -vv "HQ Sync.app"` -> accepted
- [ ] Verify universal binary: `file "HQ Sync.app/Contents/MacOS/HQ Sync"` -> shows x86_64 + arm64
- [ ] Launch on clean macOS 13+ machine -> **no Gatekeeper warnings**

---

## Release Checklist

Before each release (v1.0.0 and every subsequent minor/patch):

- [ ] All UJ tests above pass on fresh macOS VM
- [ ] All per-story acceptance tests pass for completed stories
- [ ] Loom video recorded covering full checklist walkthrough
- [ ] Loom video link added to GitHub Release notes
- [ ] Performance budget verified (see `tests/PERF.md`)
- [ ] `spctl` verification passes on built DMG
- [ ] No zombie processes observed during any test

---

## Policy Deviation

### Reference

**Policy:** `e2e-backpressure-required.md`
**Enforcement:** Hard (normally)
**Status:** Documented exception for V1

### Justification

This project deviates from the `e2e-backpressure-required.md` policy which requires automated e2e tests for all deployable projects. The deviation was approved during PRD interview question QUALITY-2.

**Reasons for V1 exception:**
- Dogfood-only cohort of 10 internal Indigo users with a direct feedback channel (Slack, in-person)
- macOS native app testing requires platform-specific tooling (AppleScript for tray, Playwright for WebView) that adds significant setup cost
- Manual testing + Loom video provides sufficient coverage for a 10-person internal rollout
- Fast iteration via auto-updater means bugs can be patched and shipped within hours

### V2 Commitment

Before any external customer rollout, the following automated e2e tests **must** be added:
- **Playwright** for popover WebView interactions (sync button, conflict modal, settings)
- **AppleScript** for tray icon state verification and context menu testing
- Automated test suite integrated into CI (GitHub Actions)
- Tracked as a follow-up story in V2 scope

### Compensating Controls (V1)

- Manual testing checklist (this document) run before every release
- Loom video proof published in every GitHub Release
- Direct user feedback channel (10 internal users)
- Performance budget hard gate (`tests/PERF.md`) blocks release on budget miss
