<script lang="ts">
  /**
   * AccountView — the HQ Sync Accounts (CRM) surface (hq-native-crm US-010).
   *
   * One read-only surface that answers "where is every client / where is this
   * client?" entirely from the vault-synced `crm-projection.json`:
   *
   *   • LIST view — all clients grouped by funnel stage (Lead → … → Active) with
   *     company, stage chip, owner, last-activity, and contract + billing status
   *     chips; plus a "needs attention" rail surfacing clients that carry open
   *     action-items from signals.
   *   • DETAIL view — one client end-to-end: demo origin, pipeline stage, the
   *     lifecycle rail, contract status, latest invoice + status, and the
   *     meetings/signals activity timeline, with a "source of truth: ontology"
   *     footer.
   *
   * Read-only by construction: every value comes from the projection (joined
   * server-side off the canonical ontology by hq-pro US-009). There are NO
   * network calls to Attio / Stripe / PandaDoc / Neon — the projection is read
   * LOCAL-FIRST with a vault-API fallback (the same pattern the Board surface
   * uses for `board.json`). Missing legs degrade to an em-dash chip. All logic
   * lives in the pure `account-view-model.ts`; this component is presentational.
   *
   * Design: the in-app V4 liquid-glass system + V4 status-dot palette (green
   * #30D158 = affirmative). Monochrome elsewhere — color is reserved for status
   * dots only; never purple (hard policy indigo-no-purple).
   */
  import { loadCrmProjection } from './crm-projection';
  import {
    buildAccountDetail,
    findAccount,
    groupAccountsByStage,
    needsAttention,
    orDash,
    relativeTime,
    statusTone,
    STAGE_LABELS,
    type AccountDetail,
    type CrmProjection,
    type DotTone,
    type ProjectedAccount,
  } from './account-view-model';
  import '../../desktop-alt/v4/tokens.css';

  interface Props {
    /** The company/workspace slug whose CRM projection this view renders. */
    slug: string;
    /** False for local folders that are not cloud-backed yet (calmer note). */
    cloudBacked?: boolean;
  }

  let { slug, cloudBacked = true }: Props = $props();

  let projection = $state<CrmProjection | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);

  // Detail drill-in: the selected account id (null === list view).
  let selectedId = $state<string | null>(null);

  const accounts = $derived<ProjectedAccount[]>(projection?.accounts ?? []);
  const groups = $derived(groupAccountsByStage(accounts));
  const attention = $derived(needsAttention(accounts));
  const selectedAccount = $derived<ProjectedAccount | null>(
    projection && selectedId ? findAccount(projection, selectedId) : null,
  );
  const detail = $derived<AccountDetail | null>(
    selectedAccount ? buildAccountDetail(selectedAccount) : null,
  );
  const syncedLabel = $derived(relativeTime(projection?.synced_at));

  // Load the projection whenever the company slug changes. Cancel-flag guards an
  // out-of-order completion when the user switches companies quickly.
  $effect(() => {
    const activeSlug = slug;
    projection = null;
    error = null;
    selectedId = null;

    if (!activeSlug) {
      loading = false;
      return;
    }

    loading = true;
    let cancelled = false;

    void (async () => {
      try {
        const result = await loadCrmProjection(activeSlug);
        if (cancelled) return;
        projection = result;
      } catch (err) {
        console.error('loadCrmProjection failed:', err);
        if (!cancelled) {
          error = 'Accounts unavailable. Try again after a sync.';
          projection = null;
        }
      } finally {
        if (!cancelled) loading = false;
      }
    })();

    return () => {
      cancelled = true;
    };
  });

  function openAccount(id: string): void {
    selectedId = id;
  }

  function backToList(): void {
    selectedId = null;
  }

  /** Stage-pill tone — affirmative once a client is signed/active. */
  function stageTone(stage: ProjectedAccount['stage']): DotTone {
    if (stage === 'active' || stage === 'signed') return 'ok';
    if (stage === 'lead') return 'idle';
    return 'warn';
  }
</script>

<section class="account-view" aria-label="Accounts" data-testid="account-view">
  {#if loading}
    <!-- Syncing / loading skeleton (mirrors the list column lanes). -->
    <div class="av-skeleton" aria-busy="true" data-testid="account-view-loading">
      {#each [0, 1, 2] as row (row)}
        <div class="av-skeleton-row"></div>
      {/each}
    </div>
  {:else if error}
    <div class="av-error" role="alert" data-testid="account-view-error">{error}</div>
  {:else if detail}
    <!-- ── DETAIL VIEW ──────────────────────────────────────────────── -->
    <article class="av-detail" data-testid="account-detail">
      <button type="button" class="av-back" onclick={backToList}>← Accounts</button>

      <header class="av-hero">
        <div class="av-hero-head">
          <h1 class="av-hero-name">{detail.account.name}</h1>
          <span class="av-readonly" title="This surface is read-only">READ ONLY</span>
        </div>
        <div class="av-hero-meta">
          <span class="av-domain">{orDash(detail.account.domain)}</span>
          <span class="av-pill av-pill-{stageTone(detail.account.stage)}">
            <span class="av-dot av-dot-{stageTone(detail.account.stage)}" aria-hidden="true"></span>
            {detail.stageLabel}
          </span>
          <span class="av-owner">{orDash(detail.account.owner)}</span>
        </div>
      </header>

      <!-- Lifecycle rail: Demo → Demo done → Proposal → Signed → Active -->
      <section class="av-lifecycle" aria-label="Lifecycle">
        {#each detail.lifecycle as step (step.stage)}
          <span class="av-step av-step-{step.state}" data-testid="lifecycle-step">
            <span class="av-step-mark" aria-hidden="true"></span>
            <span class="av-step-label">{step.label}</span>
          </span>
        {/each}
      </section>

      <!-- Four source cards: Inbound·Neon / Pipeline·Attio / Contract·PandaDoc / Billing·Stripe -->
      <section class="av-sources" aria-label="Sources of record">
        {#each detail.sourceCards as card (card.role)}
          <div class="av-source-card" data-testid="source-card" data-role={card.role}>
            <header class="av-source-head">
              <span class="av-source-label">{card.label}</span>
              <span class="av-dot av-dot-{card.tone}" aria-hidden="true"></span>
            </header>
            {#if card.source}
              <div class="av-source-value">{orDash(card.source.value)}</div>
              <div class="av-source-meta">{orDash(card.source.meta)}</div>
              <div class="av-source-status">{orDash(card.source.status)}</div>
              <div class="av-source-ref mono" title="external-id join key">
                {orDash(card.source.ref)}
              </div>
            {:else}
              <!-- Missing leg → em-dash chip (graceful degradation). -->
              <div class="av-source-empty" data-testid="source-missing">—</div>
            {/if}
          </div>
        {/each}
      </section>

      <!-- Activity timeline from meetings & signals -->
      <section class="av-timeline-section" aria-label="Activity">
        <h2 class="av-section-title">ACTIVITY · from meetings &amp; signals</h2>
        {#if detail.timeline.length === 0}
          <div class="av-empty" data-testid="timeline-empty">No activity yet</div>
        {:else}
          <ol class="av-timeline">
            {#each detail.timeline as entry, i (entry.sourceMeetingRef + i)}
              <li class="av-tl-row" data-testid="timeline-entry">
                <span class="av-tl-date mono">{orDash(entry.date)}</span>
                <span class="av-tl-type">{orDash(entry.type)}</span>
                <span class="av-tl-text">{orDash(entry.text)}</span>
              </li>
            {/each}
          </ol>
        {/if}
      </section>

      <footer class="av-footer" data-testid="account-detail-footer">
        source of truth: ontology · no network to Attio/Stripe/PandaDoc · read-only
      </footer>
    </article>
  {:else if accounts.length === 0}
    <!-- ── EMPTY (first run) ────────────────────────────────────────── -->
    <div class="av-firstrun" data-testid="account-view-empty">
      {#if !cloudBacked}
        <p class="av-note">
          This company is local only. Accounts appear here once its CRM projection syncs.
        </p>
      {/if}
      <div class="av-firstrun-card">
        <h2>No accounts yet</h2>
        <p>
          This view builds itself from the ontology — demo requests, pipeline,
          contracts, and billing — once a client is synced. No setup needed.
        </p>
      </div>
    </div>
  {:else}
    <!-- ── LIST VIEW ────────────────────────────────────────────────── -->
    <header class="av-list-head">
      <h1 class="av-title">Accounts</h1>
      <span class="av-synced" data-testid="account-view-synced">synced {syncedLabel}</span>
    </header>

    {#if attention.length > 0}
      <section class="av-attention" aria-label="Needs attention" data-testid="needs-attention">
        <h2 class="av-section-title">NEEDS ATTENTION</h2>
        <ul class="av-attention-list">
          {#each attention as item (item.account.id)}
            <li>
              <button
                type="button"
                class="av-attention-row"
                data-testid="needs-attention-row"
                onclick={() => openAccount(item.account.id)}
              >
                <span class="av-dot av-dot-warn" aria-hidden="true"></span>
                <span class="av-attention-name">{item.account.name}</span>
                <span class="av-attention-reason">{orDash(item.reason)}</span>
                <span class="av-attention-count mono">{item.openCount} open</span>
              </button>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#each groups as group (group.stage)}
      <section class="av-group" aria-label={group.label} data-testid="stage-group" data-stage={group.stage}>
        <header class="av-group-head">
          <span class="av-dot av-dot-{stageTone(group.stage)}" aria-hidden="true"></span>
          <h2 class="av-group-title">{group.label}</h2>
          <span class="av-group-count">{group.accounts.length}</span>
        </header>
        <table class="av-table" data-testid="accounts-table">
          <thead>
            <tr>
              <th class="av-th-client">CLIENT</th>
              <th class="av-th-stage">STAGE</th>
              <th class="av-th-owner">OWNER</th>
              <th class="av-th-activity">LAST ACTIVITY</th>
              <th class="av-th-contract">CONTRACT</th>
              <th class="av-th-billing">BILLING</th>
            </tr>
          </thead>
          <tbody>
            {#each group.accounts as account (account.id)}
              {@const contract = account.sources.contract ?? null}
              {@const billing = account.sources.billing ?? null}
              <tr class="av-row" data-testid="account-row">
                <td class="av-td-client">
                  <button type="button" class="av-client-button" onclick={() => openAccount(account.id)}>
                    <span class="av-client-name">{account.name}</span>
                    <small class="av-client-domain">{orDash(account.domain)}</small>
                  </button>
                </td>
                <td>
                  <span class="av-pill av-pill-{stageTone(account.stage)}">
                    {STAGE_LABELS[account.stage]}
                  </span>
                </td>
                <td class="av-td-owner">{orDash(account.owner)}</td>
                <td class="av-td-activity">{relativeTime(account.lastActivity)}</td>
                <td>
                  {#if contract}
                    <span class="av-chip" data-testid="contract-chip">
                      <span class="av-dot av-dot-{statusTone(contract.status)}" aria-hidden="true"></span>
                      {orDash(contract.status)}
                    </span>
                  {:else}
                    <span class="av-chip av-chip-empty" data-testid="contract-chip-empty">—</span>
                  {/if}
                </td>
                <td>
                  {#if billing}
                    <span class="av-chip" data-testid="billing-chip">
                      <span class="av-dot av-dot-{statusTone(billing.status)}" aria-hidden="true"></span>
                      {orDash(billing.status)}
                    </span>
                  {:else}
                    <span class="av-chip av-chip-empty" data-testid="billing-chip-empty">—</span>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </section>
    {/each}

    <footer class="av-footer" data-testid="account-list-footer">source of truth: ontology · read-only</footer>
  {/if}
</section>

<style>
  .account-view {
    display: flex;
    flex-direction: column;
    gap: 20px;
    min-width: 0;
    color: var(--v4-text-1);
    font-family: 'Inter Variable', Inter, -apple-system, 'SF Pro Text', sans-serif;
    font-size: var(--text-base, 13px);
  }

  .mono {
    font-family: 'Geist Mono', ui-monospace, 'SF Mono', Menlo, monospace;
    font-variant-numeric: tabular-nums;
  }

  /* ── status dots (the ONLY color; never purple) ──────────────────── */
  .av-dot {
    display: inline-block;
    width: 6px;
    height: 6px;
    flex: 0 0 auto;
    border-radius: 50%;
  }
  .av-dot-ok {
    background: var(--v4-ok);
  }
  .av-dot-warn {
    background: var(--v4-warn);
  }
  .av-dot-error {
    background: var(--v4-error);
  }
  .av-dot-idle {
    background: var(--v4-idle);
  }

  /* ── list head ───────────────────────────────────────────────────── */
  .av-list-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
  }
  .av-title {
    margin: 0;
    color: var(--v4-text-1);
    font-size: 18px;
    font-weight: 600;
  }
  .av-synced {
    color: var(--v4-text-3);
    font-size: var(--text-base, 13px);
  }

  .av-section-title {
    margin: 0 0 10px;
    color: var(--v4-text-3);
    font-size: var(--text-base, 13px);
    font-weight: 500;
    letter-spacing: 0.02em;
  }

  /* ── needs attention ─────────────────────────────────────────────── */
  .av-attention {
    padding: 14px 16px;
    border: 1px solid var(--v4-hairline);
    border-radius: 12px;
    background: var(--v4-inset);
  }
  .av-attention-list {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .av-attention-row {
    display: grid;
    grid-template-columns: 6px minmax(120px, 0.4fr) 1fr auto;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 9px 6px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .av-attention-row:hover {
    background: var(--v4-active-row);
  }
  .av-attention-row:focus-visible {
    outline: 1px solid var(--v4-control-border);
    outline-offset: 1px;
  }
  .av-attention-name {
    color: var(--v4-text-1);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .av-attention-reason {
    color: var(--v4-text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .av-attention-count {
    color: var(--v4-text-3);
    flex: 0 0 auto;
  }

  /* ── stage groups + table ────────────────────────────────────────── */
  .av-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .av-group-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .av-group-title {
    margin: 0;
    color: var(--v4-text-2);
    font-size: var(--text-base, 13px);
    font-weight: 500;
  }
  .av-group-count {
    color: var(--v4-text-3);
    font-size: var(--text-base, 13px);
  }

  .av-table {
    width: 100%;
    min-width: 720px;
    border-collapse: collapse;
    table-layout: fixed;
  }
  .av-table th {
    padding: 0 12px 8px 0;
    border-bottom: 1px solid var(--v4-rowline);
    color: var(--v4-text-3);
    font-size: var(--text-base, 13px);
    font-weight: 400;
    letter-spacing: 0.02em;
    text-align: left;
  }
  .av-th-stage {
    width: 132px;
  }
  .av-th-owner {
    width: 104px;
  }
  .av-th-activity {
    width: 120px;
  }
  .av-th-contract {
    width: 112px;
  }
  .av-th-billing {
    width: 96px;
  }
  .av-table td {
    height: 52px;
    padding: 8px 12px 8px 0;
    border-bottom: 1px solid var(--v4-rowline);
    vertical-align: middle;
    min-width: 0;
  }

  .av-client-button {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .av-client-name,
  .av-client-domain {
    overflow: hidden;
    max-width: 100%;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .av-client-name {
    color: var(--v4-text-1);
    font-weight: 500;
  }
  .av-client-button:hover .av-client-name {
    text-decoration: underline;
  }
  .av-client-button:focus-visible {
    outline: 1px solid var(--v4-control-border);
    outline-offset: 2px;
  }
  .av-client-domain {
    color: var(--v4-text-3);
  }
  .av-td-owner,
  .av-td-activity {
    color: var(--v4-text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .av-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 20px;
    padding: 0 8px;
    border-radius: 5px;
    background: var(--v4-control-faint);
    color: var(--v4-text-2);
    font-size: var(--text-base, 13px);
    line-height: 1;
    white-space: nowrap;
  }
  .av-pill-ok {
    color: var(--v4-text-1);
  }

  .av-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--v4-text-2);
    font-size: var(--text-base, 13px);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
  }
  .av-chip-empty {
    color: var(--v4-text-3);
  }

  /* ── detail view ─────────────────────────────────────────────────── */
  .av-detail {
    display: flex;
    flex-direction: column;
    gap: 20px;
    min-width: 0;
  }
  .av-back {
    align-self: flex-start;
    padding: 4px 0;
    border: 0;
    background: transparent;
    color: var(--v4-text-2);
    font: inherit;
    font-size: var(--text-base, 13px);
    cursor: pointer;
  }
  .av-back:hover {
    color: var(--v4-text-1);
  }
  .av-back:focus-visible {
    outline: 1px solid var(--v4-control-border);
    outline-offset: 2px;
  }

  .av-hero {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 18px 20px;
    border: 1px solid var(--v4-hairline);
    border-radius: 12px;
    background: var(--v4-raised);
  }
  .av-hero-head {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .av-hero-name {
    margin: 0;
    color: var(--v4-text-1);
    font-size: 18px;
    font-weight: 600;
  }
  .av-readonly {
    padding: 2px 7px;
    border: 1px solid var(--v4-hairline);
    border-radius: 4px;
    color: var(--v4-text-3);
    font-family: 'Geist Mono', ui-monospace, monospace;
    font-size: 11px;
    letter-spacing: 0.04em;
  }
  .av-hero-meta {
    display: flex;
    align-items: center;
    gap: 14px;
    flex-wrap: wrap;
    color: var(--v4-text-2);
  }
  .av-domain {
    color: var(--v4-text-2);
    font-family: 'Geist Mono', ui-monospace, monospace;
  }
  .av-owner {
    color: var(--v4-text-3);
  }

  /* lifecycle rail */
  .av-lifecycle {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .av-step {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 6px 12px;
    border: 1px solid var(--v4-hairline);
    border-radius: 999px;
    color: var(--v4-text-3);
    font-size: var(--text-base, 13px);
    white-space: nowrap;
  }
  .av-step-mark {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--v4-idle);
  }
  .av-step-past {
    color: var(--v4-text-2);
  }
  .av-step-past .av-step-mark {
    background: var(--v4-text-3);
  }
  .av-step-current {
    color: var(--v4-text-1);
    border-color: var(--v4-ok);
    box-shadow: 0 0 0 1px var(--v4-ok) inset;
  }
  .av-step-current .av-step-mark {
    background: var(--v4-ok);
  }

  /* source cards */
  .av-sources {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 12px;
  }
  .av-source-card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 14px;
    border: 1px solid var(--v4-hairline);
    border-radius: 12px;
    background: var(--v4-raised);
    min-width: 0;
  }
  .av-source-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .av-source-label {
    color: var(--v4-text-3);
    font-size: var(--text-base, 13px);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .av-source-value {
    color: var(--v4-text-1);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .av-source-meta,
  .av-source-status {
    color: var(--v4-text-2);
    font-size: var(--text-base, 13px);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .av-source-ref {
    color: var(--v4-text-3);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .av-source-empty {
    color: var(--v4-text-3);
    font-size: 20px;
    line-height: 1;
  }

  /* timeline */
  .av-timeline-section {
    display: flex;
    flex-direction: column;
  }
  .av-timeline {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
  }
  .av-tl-row {
    display: grid;
    grid-template-columns: 54px 96px 1fr;
    align-items: baseline;
    gap: 12px;
    padding: 9px 0;
    border-bottom: 1px solid var(--v4-rowline);
  }
  .av-tl-date {
    color: var(--v4-text-3);
    font-size: 11px;
  }
  .av-tl-type {
    color: var(--v4-text-2);
    font-size: var(--text-base, 13px);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .av-tl-text {
    color: var(--v4-text-1);
    font-size: var(--text-base, 13px);
  }

  /* footers + empty/loading/error */
  .av-footer {
    padding-top: 6px;
    color: var(--v4-text-3);
    font-family: 'Geist Mono', ui-monospace, monospace;
    font-size: 11px;
    letter-spacing: 0.02em;
  }
  .av-empty {
    padding: 16px;
    border: 1px dashed var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
    color: var(--v4-text-3);
    text-align: center;
  }
  .av-firstrun {
    display: flex;
    flex-direction: column;
    gap: 14px;
    align-items: center;
  }
  .av-firstrun-card {
    max-width: 420px;
    padding: 28px 24px;
    border: 1px solid var(--v4-hairline);
    border-radius: 12px;
    background: var(--v4-inset);
    text-align: center;
  }
  .av-firstrun-card h2 {
    margin: 0 0 8px;
    color: var(--v4-text-1);
    font-size: var(--text-base, 13px);
    font-weight: 500;
  }
  .av-firstrun-card p {
    margin: 0;
    color: var(--v4-text-3);
    line-height: 1.4;
  }
  .av-note {
    color: var(--v4-text-2);
    line-height: 1.35;
  }
  .av-error {
    padding: 12px 14px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
    color: var(--v4-text-2);
  }

  /* loading skeleton */
  .av-skeleton {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .av-skeleton-row {
    height: 52px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
    animation: av-pulse 1.3s ease-in-out infinite;
  }
  @keyframes av-pulse {
    0%,
    100% {
      opacity: 0.5;
    }
    50% {
      opacity: 1;
    }
  }

  @media (max-width: 980px) {
    .av-sources {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .av-group {
      overflow-x: auto;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .av-skeleton-row {
      animation: none;
    }
  }
</style>
