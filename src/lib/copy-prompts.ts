/**
 * Copy-prompt registry — every notice surface in the app passes one of these
 * descriptors to <CopyPromptButton>. The button renders the result of
 * `buildPrompt(issue)` into the user's clipboard so they can paste it
 * straight into a Codex or Claude session running in HQ.
 *
 * Adding a new notice kind = one entry in this map. No new components,
 * no plumbing changes.
 *
 * Prompts speak to an HQ-aware agent — they reference HQ skills
 * (`/resolve-conflicts`, `/hq-login`, `/update-hq`, `/setup`, etc.) and the
 * hq CLI commands the agent has access to.
 */

export type IssueKind =
  | 'sync-conflict'
  | 'sync-failed'
  | 'auth-expired'
  | 'app-update-available'
  | 'hq-cli-update-available'
  | 'hq-cli-update-failed'
  | 'cloud-unreachable'
  | 'manifest-error'
  | 'workspace-needs-connect'
  | 'workspace-broken'
  | 'repair-company'
  | 'local-env-failure'
  | 'hq-version-undetectable'
  | 'hq-core-drift';

/**
 * Stable kind identifiers for `local-env-failure` payloads. These match the
 * `&'static str` constants returned by
 * `src-tauri/src/commands/run_cli_provision.rs::classify_local_env_failure`.
 * If you add a new kind on the Rust side, add it here and a builder branch
 * below.
 */
export type LocalEnvKind =
  | 'npm-cache-permission'
  | 'disk-full'
  | 'npm-registry-unreachable'
  | 'npm-registry-timeout';

export interface Issue {
  kind: IssueKind;
  payload?: Record<string, unknown>;
}

const HQ_CLI_UPGRADE_CMD = 'npm install -g @indigoai-us/hq-cli@latest';

function val(issue: Issue, key: string): string {
  const v = issue.payload?.[key];
  return v == null ? '' : String(v);
}

function num(issue: Issue, key: string): number {
  const v = issue.payload?.[key];
  return typeof v === 'number' ? v : 0;
}

const builders: Record<IssueKind, (i: Issue) => string> = {
  'sync-conflict': (i) => {
    const count = num(i, 'count');
    const company = val(i, 'company');
    const countLine = count > 0 ? `${count} file conflict${count === 1 ? '' : 's'}` : 'sync conflicts';
    return [
      `I'm seeing ${countLine} in my HQ Sync menubar app${company ? ` (company: ${company})` : ''}.`,
      '',
      'Please run `/resolve-conflicts` to walk me through each one. Use the local file as the source of truth unless the remote is clearly newer + intentional. After resolving, run `hq sync` once to confirm the menubar shows zero conflicts.',
    ].join('\n');
  },

  'sync-failed': (i) => {
    const msg = val(i, 'message');
    const company = val(i, 'company');
    return [
      `My HQ Sync just failed${company ? ` while syncing "${company}"` : ''}.`,
      '',
      msg ? `Error: ${msg}` : 'No error message was surfaced in the UI.',
      '',
      `Please investigate using \`/diagnose\` if the error is non-deterministic, or \`/investigate\` for a reproducible failure. Start by reading \`~/.hq/logs/hq-sync.log\` (last 200 lines) and \`~/.hq/sync-journal.${company || '<slug>'}.json\` to see what the runner attempted. Then propose a fix or a retry strategy before re-running \`hq sync\`.`,
    ].join('\n');
  },

  'auth-expired': (i) => {
    const msg = val(i, 'message');
    return [
      'My HQ Sync session expired and the menubar app is asking me to sign in again.',
      msg ? `\nError: ${msg}` : '',
      '',
      'Please run `/hq-login` to refresh my Cognito tokens. If a silent refresh fails, fall back to the browser sign-in flow. Confirm with `/hq-whoami` that the session is healthy before doing anything else.',
    ].filter(Boolean).join('\n');
  },

  'app-update-available': (i) => {
    const version = val(i, 'version');
    return [
      `My HQ Sync menubar app has an update available${version ? ` (v${version})` : ''}.`,
      '',
      "Please apply it. The in-app Install button calls Tauri's `install_update` and restarts the app — that's usually the right path. If it fails, fetch the latest DMG from the GitHub releases page of `indigoai-us/hq-sync` and install manually. After updating, open the popover and confirm the new version in the About / Settings.",
    ].join('\n');
  },

  'hq-cli-update-available': (i) => {
    const local = val(i, 'local');
    const latest = val(i, 'latest');
    return [
      `My globally-installed \`hq\` CLI is behind npm latest${local && latest ? ` (local v${local} → latest v${latest})` : ''}.`,
      '',
      `Please run \`${HQ_CLI_UPGRADE_CMD}\`. If the install errors with EACCES, my npm prefix is system-owned — switch to either \`sudo ${HQ_CLI_UPGRADE_CMD}\` (after confirming with me) or recommend reconfiguring npm to a user-owned prefix (\`~/.npm-global\`). After upgrade, confirm with \`hq --version\` matches the latest.`,
    ].join('\n');
  },

  'hq-cli-update-failed': (i) => {
    const error = val(i, 'error');
    return [
      'My HQ Sync menubar tried to upgrade the `hq` CLI for me and the install failed.',
      '',
      error ? `Error from npm: ${error}` : 'No error detail was surfaced.',
      '',
      `Please diagnose. Most common cause is EACCES against a system-prefix npm — in that case, run \`sudo ${HQ_CLI_UPGRADE_CMD}\` (with my confirmation) or walk me through moving npm's global prefix to \`~/.npm-global\`. After the upgrade succeeds, verify \`hq --version\` matches npm \`latest\`.`,
    ].join('\n');
  },

  'cloud-unreachable': (i) => {
    const error = val(i, 'error');
    return [
      'My HQ Sync menubar says the cloud is unreachable — it\'s showing local-only workspaces.',
      '',
      error ? `Last error: ${error}` : '',
      '',
      "Please check: (1) am I signed in? Run `/hq-whoami`. (2) Is hq-ops reachable? Hit the vault health endpoint with curl. (3) Are my Cognito tokens valid? If refresh is needed, run `/hq-login`. If the network is genuinely down, just note it and we'll retry later — don't fabricate a fix.",
    ].filter(Boolean).join('\n');
  },

  'manifest-error': (i) => {
    const error = val(i, 'error');
    return [
      "My HQ Sync menubar can't read `companies/manifest.yaml` — it fell back to folder enumeration.",
      '',
      error ? `Parser error: ${error}` : '',
      '',
      'Please open `~/HQ/companies/manifest.yaml` (or wherever my HQ folder is — check `~/.hq/menubar.json` → `hqPath`) and find the parse error. Likely a trailing tab, an unquoted value with a colon, or a stray BOM. After fixing, validate with `yamllint` if available. Do not regenerate the manifest from scratch — preserve the existing companies + their cloud_uid fields.',
    ].filter(Boolean).join('\n');
  },

  'workspace-needs-connect': (i) => {
    const slug = val(i, 'slug');
    return [
      `My HQ Sync menubar shows a local-only workspace${slug ? ` (\`${slug}\`)` : ''} that needs to be connected to a cloud vault.`,
      '',
      "The in-app Connect button calls `connect_workspace_to_cloud` — that's usually all I need. If it fails (cloud unreachable, name collision, permissions), tell me which and what to do next. Don't try to provision a brand-new bucket out of band — the backend handles bucket creation + manifest update atomically.",
    ].join('\n');
  },

  'workspace-broken': (i) => {
    const slug = val(i, 'slug');
    const reason = val(i, 'reason');
    return [
      `My HQ Sync menubar shows workspace \`${slug || '<unknown>'}\` as broken.`,
      '',
      reason ? `Reason: ${reason}` : 'The manifest cloud_uid does not match cloud reality.',
      '',
      'Please run `/repair-company` if it exists, otherwise: (1) compare `companies/manifest.yaml` cloud_uid vs. the actual cloud entity for this slug via the hq CLI, (2) update the manifest to match cloud truth, (3) commit the manifest change inside the HQ root, (4) re-open the menubar to verify the row flips back to synced.',
    ].join('\n');
  },

  'repair-company': (i) => {
    const slug = val(i, 'slug');
    return [
      `One of my HQ companies${slug ? ` (\`${slug}\`)` : ''} is in a bad state and I need help repairing it.`,
      '',
      "Please walk through: (1) read `companies/{slug}/manifest.yaml` (if any) + the row in `companies/manifest.yaml`, (2) verify the cloud_uid + bucket still exist server-side, (3) check the local folder structure matches the company template, (4) re-run `hq sync` for just that company and watch the journal. Propose each step as a decision queue — don't do destructive ops (delete folders, drop cloud entries) without my explicit go-ahead.",
    ].join('\n');
  },

  // Drift detail "Review with agent" — one entry's worth of context handed
  // off to Claude Code. Payload: `{ path, kind: 'modified'|'missing'|'added',
  // hqVersion }`. The prompt is path-scoped so the agent can read the local
  // file (if present) and the upstream version side-by-side before deciding.
  'hq-core-drift': (i) => {
    const path = val(i, 'path');
    const kind = val(i, 'kind'); // 'modified' | 'missing' | 'added'
    const hqVersion = val(i, 'hqVersion');
    const versionLine = hqVersion
      ? `My local hq-core is \`v${hqVersion}\`.`
      : "I'm not sure what hqVersion I'm on.";
    const kindLine = kind === 'missing'
      ? `It's listed in the upstream tree but missing from my local copy.`
      : kind === 'added'
      ? `It exists locally under a locked-path scope but doesn't exist in the upstream tree — I either added it or it's left over from a prior version.`
      : `It exists in both places but the contents differ from upstream.`;
    return [
      `My HQ Sync menubar detected a drift in a locked hq-core file: \`${path || '<path>'}\`.`,
      '',
      versionLine,
      kindLine,
      '',
      "Please walk me through fixing this:",
      `1. **Read both versions** — local at \`${path || '<path>'}\`, upstream at https://github.com/indigoai-us/hq-core/blob/v${hqVersion || '<tag>'}/${path || '<path>'}.`,
      "2. **Decide which to keep** — if my local edit is intentional, propose lifting it into a personal/ overlay so the locked file goes back to matching upstream. If the local edit is accidental or stale, recommend restoring from upstream.",
      "3. **Do not touch other drifted files** unless I ask — the menubar lists them separately so each gets its own decision.",
      "4. If the right call is a full hq-core re-sync, run `/update-hq` instead of editing individual files.",
    ].join('\n');
  },

  // Footer "HQ vX.Y.Z" row couldn't detect a version — the HQ folder is
  // missing, `core.yaml` is missing/unparseable, or its `hqVersion` field
  // is empty. All three mean the install is broken in a way the menubar
  // can't self-repair (we don't know which HQ to write to). Hand the agent
  // enough context to triage which case it is and walk the user back to a
  // working install. Optional `hqFolderPath` payload carries the path the
  // resolver thought it was using, so the agent can `ls -la` it directly
  // instead of guessing.
  'hq-version-undetectable': (i) => {
    const hqFolderPath = val(i, 'hqFolderPath');
    return [
      "My HQ Sync menubar can't detect the version of HQ I'm running.",
      '',
      hqFolderPath
        ? `The resolver picked: \`${hqFolderPath}\``
        : "The resolver couldn't even locate an HQ folder on this machine.",
      '',
      "Please diagnose which of these is true and walk me through fixing it:",
      "1. **HQ folder missing / wrong** — verify the path above exists (`ls -la`). If wrong, open my HQ Sync app → Settings → re-tether to the correct folder.",
      "2. **`core.yaml` missing** — at the HQ folder, check both canonical (`core/core.yaml`) and legacy (`core.yaml`) locations. If missing, the install is incomplete; run `/setup` to scaffold it, or `/update-hq` to repair from the latest hq-core release.",
      "3. **`core.yaml` unparseable or missing `hqVersion`** — read it and tell me what's there. If corrupted, restore from git or re-run `/update-hq`.",
      '',
      "Once `hqVersion` is readable again, the footer row will refresh next time I open the popover.",
    ].join('\n');
  },

  // Action-oriented prompts for user-environment failures the
  // `classify_local_env_failure` Rust helper recognises. The button is
  // wired to "attempt-the-fix" — these tell the agent to run the remediation
  // itself, with explicit user confirmation gates on any `sudo` step.
  'local-env-failure': (i) => {
    const kind = val(i, 'kind');
    const detail = val(i, 'detail');
    const slug = val(i, 'slug');
    const header = `My HQ Sync menubar just failed to provision \`${slug || '<workspace>'}\` because of a local-environment problem (\`${kind}\`).`;
    const detailLine = detail ? `\nError detail from the CLI: ${detail}` : '';

    switch (kind as LocalEnvKind) {
      case 'npm-cache-permission':
        return [
          header,
          detailLine,
          '',
          "My `~/.npm` cache has root-owned files (most likely from a previous `sudo npm` run), so `npx`'s npm couldn't open its index. The HQ Sync app routes `hq` provisioning through `npx -y --package=@indigoai-us/hq-cli@<range>`, so every Connect attempt will keep failing until this is fixed.",
          '',
          "Please attempt the fix:",
          "1. Confirm with me, then run `sudo chown -R $(id -u):$(id -g) ~/.npm` so the cache is user-owned again.",
          "2. Verify with `ls -ld ~/.npm` that the owner is no longer root.",
          "3. Re-trigger Connect from my menubar (or run `npx -y --package=@indigoai-us/hq-cli@latest hq --version` to confirm npx works again).",
          "Do NOT touch any other root-owned directories — only `~/.npm`. If `sudo` is unavailable in this session, walk me through the manual fix instead of skipping the verification.",
        ].filter(Boolean).join('\n');

      case 'disk-full':
        return [
          header,
          detailLine,
          '',
          "npm couldn't extract the `@indigoai-us/hq-cli` package because the disk is full. Please attempt the fix:",
          "1. Run `df -h /` and report the free space.",
          "2. Run `du -sh ~/.npm ~/Library/Caches/* ~/.Trash` (with my confirmation) so we can see the obvious candidates.",
          "3. Recommend (don't auto-delete) the biggest reclaimable targets — `~/.npm/_cacache` can be safely wiped with `npm cache clean --force`, Xcode DerivedData / iOS simulators are common offenders.",
          "4. After freeing space, re-trigger Connect from my menubar.",
        ].filter(Boolean).join('\n');

      case 'npm-registry-unreachable':
        return [
          header,
          detailLine,
          '',
          "npm couldn't resolve the npm registry DNS. Most likely: I'm offline, on a captive-portal Wi-Fi, or my registry config points at a private mirror that's down. Please attempt the fix:",
          "1. Run `npm config get registry` and report the value.",
          "2. Run `curl -sS -o /dev/null -w '%{http_code}\\n' https://registry.npmjs.org/` to see if the public registry is reachable.",
          "3. If a non-default registry is configured and unreachable, recommend resetting it with `npm config set registry https://registry.npmjs.org/` (after confirming with me).",
          "4. After connectivity recovers, re-trigger Connect from my menubar.",
        ].filter(Boolean).join('\n');

      case 'npm-registry-timeout':
        return [
          header,
          detailLine,
          '',
          "npm's TCP connection to the npm registry timed out. Likely a slow link, a corporate proxy, or an npmjs.org incident. Please attempt the fix:",
          "1. Check https://status.npmjs.org/ (note: don't fabricate — actually fetch).",
          "2. Run `npm config get proxy` and `npm config get https-proxy`; if either is set, ask me whether it should be unset.",
          "3. Retry `npx -y --package=@indigoai-us/hq-cli@latest hq --version` once and report the result.",
          "4. If transient, just retry Connect from my menubar. If persistent, walk me through `npm config set fetch-retry-maxtimeout` or a proxy unset.",
        ].filter(Boolean).join('\n');

      default:
        // Unknown kind — keep the prompt useful even if Rust shipped a new
        // kind ahead of the TS catalogue.
        return [
          header,
          detailLine,
          '',
          "I don't have a templated remediation for this kind yet. Please read `~/.hq/logs/hq-sync.log` (last 100 lines) and the `provision-cli` breadcrumbs there to figure out what npm or npx did, then propose a fix. Do not run any `sudo` commands without my explicit confirmation.",
        ].filter(Boolean).join('\n');
    }
  },
};

/**
 * Parse the `CliProvisionError::LocalEnv` Display string emitted by the Rust
 * backend. The format — `local environment failure (<kind>): <detail>` — is
 * locked by a Rust test (`local_env_display_contract_is_parseable`) so this
 * regex stays stable across releases.
 *
 * Returns `null` when the input doesn't match (e.g. a real vault error, an
 * unrelated frontend exception) so the caller can route the row to the
 * existing generic-error branch.
 */
export function parseLocalEnvFailure(
  message: string,
): { kind: LocalEnvKind; detail: string } | null {
  const m = /local environment failure \(([^)]+)\):\s*(.*)$/m.exec(message);
  if (!m) return null;
  const kind = m[1] as LocalEnvKind;
  const detail = m[2].trim();
  // Validate kind against the known catalogue — protects against a Rust
  // typo or a wire artifact ("(unknown)") leaking through to the button.
  const known: ReadonlySet<LocalEnvKind> = new Set([
    'npm-cache-permission',
    'disk-full',
    'npm-registry-unreachable',
    'npm-registry-timeout',
  ]);
  if (!known.has(kind)) return null;
  return { kind, detail };
}

export function buildPrompt(issue: Issue): string {
  const build = builders[issue.kind];
  if (!build) {
    return `My HQ Sync menubar surfaced an unknown issue kind (\`${issue.kind}\`). Please diagnose by reading the source at \`src/lib/copy-prompts.ts\` and the relevant component, then propose a fix.`;
  }
  return build(issue);
}
