import { describe, expect, it } from 'vitest';
import { canEnableRealtimeSync } from './realtime-sync';

// Auto-sync (Beta) is gated to @getindigo.ai accounts during the internal
// rollout. The gate is a UI affordance only — anyone with devtools can flip
// it — but this test pins the contract so the Settings component can rely on
// a single source of truth.

describe('canEnableRealtimeSync', () => {
  it('allows lowercase @getindigo.ai email', () => {
    expect(canEnableRealtimeSync('alice@getindigo.ai')).toBe(true);
  });

  it('allows mixed-case @GetIndigo.AI (Cognito stores casing verbatim)', () => {
    expect(canEnableRealtimeSync('Alice.Smith@GetIndigo.AI')).toBe(true);
  });

  it('rejects unrelated domains', () => {
    expect(canEnableRealtimeSync('alice@example.com')).toBe(false);
    expect(canEnableRealtimeSync('alice@gmail.com')).toBe(false);
  });

  it('rejects look-alike domains that merely contain getindigo.ai', () => {
    // Suffix check, not substring — these must NOT pass.
    expect(canEnableRealtimeSync('alice@getindigo.ai.evil.example')).toBe(false);
    expect(canEnableRealtimeSync('alice@notgetindigo.ai')).toBe(false);
  });

  it('rejects null email (no signed-in user / no id_token)', () => {
    expect(canEnableRealtimeSync(null)).toBe(false);
  });

  it('rejects empty string', () => {
    expect(canEnableRealtimeSync('')).toBe(false);
  });

  it('rejects strings with no @ separator', () => {
    expect(canEnableRealtimeSync('getindigo.ai')).toBe(false);
  });

  it('rejects a leading/trailing-whitespace email rather than silently trimming', () => {
    // The Cognito id_token claim is the source of truth — if it had whitespace
    // we'd want to know, not paper over it.
    expect(canEnableRealtimeSync(' alice@getindigo.ai ')).toBe(false);
  });
});
