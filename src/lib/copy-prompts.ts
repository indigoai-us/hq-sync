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
  | 'hq-cli-update-available'
  | 'hq-cli-update-failed'
  | 'cloud-unreachable'
  | 'manifest-error'
  | 'workspace-needs-connect'
  | 'workspace-broken'
  | 'repair-company'
  | 'local-env-failure'
  | 'hq-version-undetectable'
  | 'hq-core-drift'
  | 'hq-core-drift-all'
  | 'hq-core-update-failed';

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

  // Drift detail "Copy prompt for all" — one prompt that covers the WHOLE
  // drift report so the user can resolve every file in a single agent
  // session. Payload: `{ hqVersion, isBuilder, files: [{path, kind, staging}] }`.
  //
  // Audience-aware: most HQ users are NOT hq-core builders — they can't open
  // PRs against (or even read) the private hq-core-staging repo, so for them
  // the only correct resolution is "lift intentional edits into a personal/
  // overlay, restore the rest from upstream" — never leave a locked core file
  // edited in place. Builders (staging classification ran, so at least one
  // file carries a staging tag) get the staging-grouped variant that
  // distinguishes already-promoted noise from genuinely-unaccounted edits.
  'hq-core-drift-all': (i) => {
    const ver = val(i, 'hqVersion') || '<tag>';
    const files = Array.isArray(i.payload?.files)
      ? (i.payload!.files as Array<{ path: string; kind: string; staging: string | null }>)
      : [];
    const isBuilder = i.payload?.isBuilder === true;
    const total = files.length;
    const plural = total === 1 ? '' : 's';
    const bullets = (paths: string[]) =>
      paths.length ? paths.map((p) => `- \`${p}\``).join('\n') : '- (none)';

    const modified = files.filter((f) => f.kind === 'modified').map((f) => f.path);
    const missing = files.filter((f) => f.kind === 'missing').map((f) => f.path);
    const added = files.filter((f) => f.kind === 'added').map((f) => f.path);

    if (!isBuilder) {
      return [
        `My HQ Sync menubar's "Core Drift" panel shows ${total} locked hq-core file${plural} that differ from what v${ver} shipped.`,
        '',
        "I'm a regular HQ user — I don't promote changes to hq-core and can't open PRs against `hq-core-staging`. So the right resolution for each file is either to **lift my intentional edit into a `personal/` overlay** (so it survives `/update-hq`) or to **restore the file from upstream**. I should never leave a locked core file edited in place — that just keeps drifting against every release.",
        '',
        '**Modified** (local content differs from upstream):',
        bullets(modified),
        '',
        '**Missing** (upstream ships these, absent locally):',
        bullets(missing),
        '',
        '**Added** (local files under a locked scope, not in upstream):',
        bullets(added),
        '',
        'Please resolve ALL of them as one batch:',
        `1. For each **Modified** file, read my local copy and the upstream version at https://github.com/indigoai-us/hq-core/blob/v${ver}/<path>. If my edit is intentional, move it into a \`personal/\` overlay (e.g. \`personal/<type>/<entry>\`, which master-sync symlinks back into \`core/\`) and restore the locked core file to upstream. If it's accidental or stale, just restore from upstream.`,
        "2. For each **Missing** file, restore it from upstream — it's a release-shipped file I shouldn't be without.",
        '3. For each **Added** file, decide whether it belongs in `personal/` (move it there) or should be deleted.',
        '4. Give me a decision queue — one file at a time, each with a keep-as-personal / restore / delete recommendation and a one-line reason. Show me the plan before changing anything.',
        "5. After we finish, the Core Drift panel should read zero. If I'd rather discard all local core edits and re-pull the release wholesale, use `/update-hq` instead.",
      ].join('\n');
    }

    // Builder variant — group by staging classification.
    const unaccounted = files.filter((f) => f.staging === 'unaccounted').map((f) => f.path);
    const stagingMain = files.filter((f) => f.staging === 'staging-main').map((f) => f.path);
    const prGroups = new Map<string, string[]>();
    for (const f of files) {
      if (f.staging && f.staging.startsWith('pr:')) {
        const n = f.staging.slice(3);
        if (!prGroups.has(n)) prGroups.set(n, []);
        prGroups.get(n)!.push(f.path);
      }
    }
    const prLines = [...prGroups.entries()]
      .sort((a, b) => Number(a[0]) - Number(b[0]))
      .map(([n, paths]) => `- **PR #${n}**: ${paths.map((p) => `\`${p}\``).join(', ')}`);

    return [
      `My HQ Sync menubar's "Core Drift" panel shows ${total} locked hq-core file${plural} that differ from v${ver} (my last released version). I'm an HQ builder — I promote core changes through \`indigoai-us/hq-core-staging\`. The menubar already classified each file against staging:`,
      '',
      '**Unaccounted** (NOT in staging main or any open PR — the real action items):',
      bullets(unaccounted),
      '',
      '**Missing locally**:',
      bullets(missing),
      '',
      '**Already in staging main** (settled — clears at the next release, no action):',
      bullets(stagingMain),
      '',
      '**In an open staging PR** (in-flight — no action unless the PR is stale):',
      prLines.length ? prLines.join('\n') : '- (none)',
      '',
      'Please help me clear the **Unaccounted** + **Missing** set as one batch:',
      `1. For each **Unaccounted** file, read local vs upstream (https://github.com/indigoai-us/hq-core/blob/v${ver}/<path>) and decide: **promote** it into hq-core-staging (via \`/promote-hq-core\`) if it's a real core improvement, **lift** it into a \`personal/\` overlay if it's machine-specific, or **restore** from upstream if it's stale.`,
      '2. For each **Missing** file, restore from upstream.',
      '3. Leave the already-in-staging files alone — they clear when the next hq-core release is cut and I run `/update-hq`.',
      "4. Give me a decision queue — one file at a time, with a promote / personal / restore recommendation and a one-line reason. Don't edit locked core in place.",
    ].join('\n');
  },

  // Footer "Update to vX.Y.Z" pill ran the hq-core rescue (release
  // `install_hq_core_update` or staging `run_replace_from_staging`) and it
  // didn't complete. We NEVER surface the raw "exit N" chip to the user — an
  // exit code isn't actionable. Instead the menubar hands the agent the full
  // context to finish the update by hand. `exitCode === -1` is the App.svelte
  // catch-branch sentinel — the Tauri invoke itself threw before the rescue
  // script could report its own code, which in practice means the install is
  // too far out of date for the in-app rescue to bridge and needs a guided
  // `/update-hq`. Any other non-zero is the rescue script's own exit
  // (clone/scan/overlay failure). Payload: `{ exitCode, logTail, logPath,
  // channel, targetVersion, targetRepo, hqVersion }`.
  'hq-core-update-failed': (i) => {
    const exitCode = num(i, 'exitCode');
    const logTail = val(i, 'logTail');
    const logPath = val(i, 'logPath');
    const channel = val(i, 'channel');
    const target = val(i, 'targetVersion');
    const repo = val(i, 'targetRepo');
    const hqVersion = val(i, 'hqVersion');
    const targetLine =
      channel === 'staging'
        ? 'It was overlaying the latest **staging** hq-core.'
        : target
          ? `It was trying to update me to **v${target}**${repo ? ` (from \`${repo}\`)` : ''}.`
          : 'It was pulling the latest hq-core release.';
    const causeLine =
      exitCode === -1
        ? "The in-app rescue exited before it could run (exit -1) — usually this means my install is too far out of date for the menubar to bridge on its own, so it needs a guided update from you."
        : `The rescue script exited with code ${exitCode}.`;
    return [
      "My HQ Sync menubar tried to update HQ core for me and it didn't finish.",
      '',
      hqVersion
        ? `I'm currently on hq-core \`v${hqVersion}\`.`
        : "I'm not sure which hq-core version I'm on.",
      targetLine,
      causeLine,
      logTail ? `\nLast log lines:\n${logTail}` : '',
      logPath ? `Full log: \`${logPath}\`` : '',
      '',
      "Please walk me through a guided update with `/update-hq`. First read the log above (and `~/.hq/logs/hq-sync.log`, last 100 lines) to see why the in-app rescue failed, then check `git status` in my HQ root for uncommitted local changes the overlay would clobber — if there are any, help me stash or commit them before updating. Then run `/update-hq` to pull the latest hq-core release and confirm the footer version row matches the target once it completes. Don't force-overwrite locked-core drift without showing me what changes first.",
    ].filter(Boolean).join('\n');
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
