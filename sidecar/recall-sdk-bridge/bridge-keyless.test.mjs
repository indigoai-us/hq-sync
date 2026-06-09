import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * Source-contract regression guard: the Recall Desktop SDK bridge is KEYLESS.
 *
 * Recall's Desktop SDK authorizes recording solely via a per-recording,
 * company-scoped upload token (POST /v1/recall/upload-token, passed on the
 * start-recording command). It never needs the account-wide Recall API key —
 * init() takes only the region apiUrl. An account-wide key is a security
 * exposure: it controls every bot + every recording/transcript across the whole
 * Recall account, and Recall has no scoped keys. hq-pro PR #300 stopped
 * GET /v1/recall/credentials from returning the real key, and this bridge
 * stopped reading/requiring it.
 *
 * bridge.mjs boots the real SDK at import time (top-level await + process.exit),
 * so it can't be imported here — we assert against its source text instead, the
 * same source-contract approach as e2e/desktop-alt/recall-sidecar-bundle.spec.ts.
 */

const bridgeSrc = readFileSync(
  fileURLToPath(new URL('./bridge.mjs', import.meta.url)),
  'utf8',
);

describe('recall-sdk-bridge is keyless', () => {
  it('never reads or requires a Recall API key', () => {
    expect(bridgeSrc).not.toMatch(/RECALL_API_KEY/);
    // The old boot guard exited 3 when the key was absent; keeping it would kill
    // every (now keyless) launch and break meeting detection. It must be gone.
    expect(bridgeSrc).not.toContain('process.exit(3)');
  });

  it('still initializes the SDK with only the region apiUrl (keyless init)', () => {
    expect(bridgeSrc).toMatch(/RecallAiSdk\.init\(\{/);
    expect(bridgeSrc).toContain('apiUrl');
  });

  it('authorizes recording via the per-recording upload token, not a key', () => {
    expect(bridgeSrc).toContain(
      'RecallAiSdk.startRecording({ windowId, uploadToken })',
    );
  });
});
