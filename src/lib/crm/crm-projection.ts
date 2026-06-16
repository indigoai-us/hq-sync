/**
 * Loader adapter for the vault-synced CRM projection (US-010, hq-native-crm).
 *
 * Reads `companies/{slug}/crm-projection.json` the SAME way the desktop app
 * reads `board.json`: LOCAL-FIRST (a filesystem scan of the resolved HQ folder)
 * with a VAULT-API FALLBACK when the local copy is missing or not synced yet.
 * Both paths are served by the single Rust command
 * `get_company_crm_projection` (mirrors `get_local_company_goals` for the local
 * scan and `get_company_board` for the vault fallback).
 *
 * No network calls to Attio / Stripe / PandaDoc / Neon are made anywhere here —
 * the projection was already joined off the canonical ontology entities by
 * hq-pro (US-009); this is a pure read of derived JSON.
 */

import { invoke } from '@tauri-apps/api/core';
import { normalizeProjection, type CrmProjection } from './account-view-model';

/** True for a payload that carries no usable projection (null / empty object). */
function isEmptyPayload(raw: unknown): boolean {
  return raw == null || (typeof raw === 'object' && Object.keys(raw).length === 0);
}

/**
 * Load + normalize a single company's CRM projection, LOCAL-FIRST with a
 * VAULT-API FALLBACK — the same pattern the Board surface uses for `board.json`.
 *
 * 1. Read the on-disk `companies/{slug}/crm-projection.json` via
 *    `get_company_crm_projection`. The Rust command param is `company_slug`;
 *    Tauri v2 exposes it camelCased.
 * 2. When the local copy is absent (never synced to this Mac, CRM not enabled,
 *    or a sync in flight — the command returns JSON `null`), fall back to the
 *    vault API via `get_company_crm_projection_vault` (`slug`).
 *
 * A company with no projection on EITHER leg resolves to an EMPTY projection
 * rather than throwing — the caller renders the empty state. Only a hard backend
 * error (path escape, signed-out caller, `AUTH_REQUIRED:`) rejects. NO network
 * is ever made to Attio / Stripe / PandaDoc / Neon.
 */
export async function loadCrmProjection(slug: string): Promise<CrmProjection> {
  const local = await invoke<unknown>('get_company_crm_projection', {
    companySlug: slug,
  });
  if (!isEmptyPayload(local)) {
    return normalizeProjection(local);
  }

  // Local miss → vault-API fallback (same as the board.json fallback path).
  const remote = await invoke<unknown>('get_company_crm_projection_vault', {
    slug,
  });
  return normalizeProjection(remote);
}

export type { CrmProjection } from './account-view-model';
