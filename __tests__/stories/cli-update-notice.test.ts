import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// DEV-1705 / feedback_bf4dede2: the menubar app never told users their hq CLI
// was stale. The detection (registry check + semver compare + events) already
// existed; this change makes the surface NON-NAGGING — it shows the exact
// `npm install -g @indigoai-us/hq-cli@latest` one-liner the "please update"
// emails ask users to run, makes the banner dismissible per-version (sticky
// until a newer version publishes), and clears once the CLI is current.
//
// Source-contract assertions (mirroring the US-* story tests) so a dropped
// wire — the dismiss command, its registration, the copyable command, the
// per-version suppression rule — fails fast without a macOS Tauri build.

const read = (p: string) => readFileSync(resolve(process.cwd(), p), 'utf8');
const normalize = (s: string) => s.replace(/\s+/g, ' ');

const popover = read('src/components/Popover.svelte');
const app = read('src/App.svelte');
const hqCliUpdate = read('src-tauri/src/commands/hq_cli_update.rs');
const mainRs = read('src-tauri/src/main.rs');
const fixtures = read('dev-harness/fixtures.ts');
const harness = read('dev-harness/Harness.svelte');

describe('CLI-update notice: copyable one-liner', () => {
  it('shows the exact npm install command and a copy affordance', () => {
    const p = normalize(popover);
    // The literal command users are emailed — must match exactly.
    expect(popover).toContain(
      "const HQ_CLI_UPGRADE_CMD = 'npm install -g @indigoai-us/hq-cli@latest';",
    );
    // Rendered in the banner (copyable code element) + a Copy button.
    expect(p).toContain('<code class="cli-cmd">{HQ_CLI_UPGRADE_CMD}</code>');
    expect(p).toContain('onclick={copyHqCliCommand}');
    expect(p).toContain('navigator.clipboard.writeText(HQ_CLI_UPGRADE_CMD)');
    // Copied→reset feedback, not a transient toast.
    expect(p).toContain('hqCliCmdCopied');
  });
});

describe('CLI-update notice: dismissible per-version', () => {
  it('renders a dismiss control wired to the dismiss callback', () => {
    const p = normalize(popover);
    expect(p).toContain('ondismisshqcliupdate?: () => void;');
    expect(p).toContain('class="banner-dismiss"');
    expect(p).toContain('onclick={ondismisshqcliupdate}');
  });

  it('App.svelte persists the dismissal per-version then hides the banner', () => {
    const a = normalize(app);
    expect(a).toContain('async function handleDismissHqCliUpdate()');
    // Optimistic hide + persist keyed on the current latest version.
    expect(a).toContain('hqCliUpdateAvailable = null;');
    expect(a).toContain("invoke('set_hq_cli_update_dismissed', { version: latest })");
    // Passed down to the Popover.
    expect(a).toContain('ondismisshqcliupdate={handleDismissHqCliUpdate}');
  });
});

describe('CLI-update notice: backend dismissal + per-version reset', () => {
  it('persists the dismissal through the untyped-merge path (survives save_settings)', () => {
    const r = normalize(hqCliUpdate);
    expect(r).toContain('pub fn set_hq_cli_update_dismissed(version: String)');
    expect(r).toContain('const DISMISSED_VERSION_KEY: &str = "cliUpdateDismissedVersion";');
    expect(r).toContain('crate::commands::first_run::merge_menubar_flags(');
  });

  it('suppresses the banner for the dismissed (or older) version, re-shows on a newer one', () => {
    const r = normalize(hqCliUpdate);
    // Pure rule used by both the event emit and the on-focus check.
    expect(r).toContain(
      "pub(crate) fn suppress_for_dismissal(latest: &str, dismissed: Option<&str>) -> bool",
    );
    expect(r).toContain('cmp_semver(latest, d) != std::cmp::Ordering::Greater');
    // The live emit is gated; the on-focus check filters dismissed out.
    expect(r).toContain('if is_cli_update_dismissed(&info.latest)');
    expect(r).toContain('result.filter(|info| !is_cli_update_dismissed(&info.latest))');
  });

  it('registers the dismiss command in main.rs', () => {
    expect(normalize(mainRs)).toContain(
      'commands::hq_cli_update::set_hq_cli_update_dismissed',
    );
  });
});

describe('CLI-update notice: dev-harness preview', () => {
  it('exposes a stale-CLI fixture and a ?state=cli-update preview', () => {
    expect(normalize(fixtures)).toContain('export const hqCliUpdateAvailable = {');
    expect(normalize(fixtures)).toContain('ondismisshqcliupdate:');
    const h = normalize(harness);
    expect(h).toContain("stateOverride === 'cli-update'");
    expect(h).toContain('{ ...popoverProps, hqCliUpdateAvailable }');
  });
});
