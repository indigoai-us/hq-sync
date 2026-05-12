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
  | 'repair-company';

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
      'Please investigate using `/diagnose` if the error is non-deterministic, or `/investigate` for a reproducible failure. Start by reading `~/.hq/sync-debug.log` (last 200 lines) and `~/.hq/sync-journal.log` to see what the runner attempted. Then propose a fix or a retry strategy before re-running `hq sync`.',
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
};

export function buildPrompt(issue: Issue): string {
  const build = builders[issue.kind];
  if (!build) {
    return `My HQ Sync menubar surfaced an unknown issue kind (\`${issue.kind}\`). Please diagnose by reading the source at \`src/lib/copy-prompts.ts\` and the relevant component, then propose a fix.`;
  }
  return build(issue);
}
