<script lang="ts">
  /**
   * Mission Control — the Manager ⇄ Liaison conversation for one team, so the
   * operator has the full context behind a pending question, plus a composer to
   * post a message straight into the team-manager inbox (no liaison needed).
   */
  import { agencyStore, sendAgencyMessage, selectAgencyTeam } from '../lib/agency-store.svelte';
  import { senderTone, relativeTime, type AgencyMessage } from '../lib/agency';

  const teams = $derived(agencyStore.teams);
  const selected = $derived(agencyStore.selected);
  const messages = $derived(agencyStore.messages);

  let draft = $state('');
  let busy = $state(false);
  let scroller = $state<HTMLDivElement | undefined>(undefined);

  // Stick to the bottom as the conversation grows.
  $effect(() => {
    void messages.length;
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });

  const KIND_BADGE: Record<string, string> = { ask: 'ASK', fyi: 'FYI', answer: 'ANSWER', learn: 'LEARN', close: 'CLOSED' };

  function teamKey(company: string, team: string) {
    return `${company}/${team}`;
  }

  async function send() {
    const text = draft.trim();
    if (!text || busy) return;
    busy = true;
    try {
      await sendAgencyMessage(text);
      draft = '';
    } catch (err) {
      console.error('send failed:', err);
    } finally {
      busy = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      void send();
    }
  }
</script>

<div class="acp">
  <header class="acp-head">
    <h2>Conversation</h2>
    {#if teams.length > 1}
      <div class="tabs" role="tablist">
        {#each teams as t (teamKey(t.company, t.team))}
          <button
            class="tab"
            class:active={selected && selected.company === t.company && selected.team === t.team}
            onclick={() => selectAgencyTeam(t.company, t.team)}
          >{t.team}</button>
        {/each}
      </div>
    {:else if selected}
      <span class="scope">{selected.company}/{selected.team}</span>
    {/if}
  </header>

  {#if !selected}
    <p class="empty">No team selected.</p>
  {:else}
    <div class="thread" bind:this={scroller}>
      {#if messages.length === 0}
        <p class="empty">No messages yet.</p>
      {/if}
      {#each messages as m, i (m.ts + '/' + m.inbox + '/' + i)}
        <div class="msg">
          <span class={`dot ${senderTone(m.from)}`} aria-hidden="true"></span>
          <div class="mbody">
            <div class="mhead">
              <span class={`mfrom ${senderTone(m.from)}`}>{m.from}</span>
              {#if KIND_BADGE[m.kind]}<span class={`kind ${m.kind}`}>{KIND_BADGE[m.kind]}</span>{/if}
              {#if m.ts}<span class="mage">{relativeTime(m.ts)}</span>{/if}
            </div>
            <p class="mtext">{m.text}</p>
          </div>
        </div>
      {/each}
    </div>

    <div class="composer">
      <textarea
        rows="2"
        placeholder="Message the team directly… (⌘↵ to send)"
        bind:value={draft}
        onkeydown={onKey}
      ></textarea>
      <button class="send" onclick={send} disabled={busy || !draft.trim()}>
        {busy ? 'Sending…' : 'Send'}
      </button>
    </div>
  {/if}
</div>

<style>
  .acp { display: flex; flex-direction: column; gap: 12px; min-height: 0; }
  .acp-head { display: flex; align-items: center; gap: 12px; }
  .acp-head h2 { margin: 0; font-size: var(--text-lg, 15px); color: var(--v4-text-1); }
  .scope { color: var(--v4-text-3); font-size: var(--text-base); text-transform: uppercase; letter-spacing: 0.04em; }
  .tabs { display: flex; gap: 6px; flex-wrap: wrap; }
  .tab {
    border: 1px solid var(--v4-hairline); border-radius: 999px; background: var(--v4-inset);
    color: var(--v4-text-2); font: inherit; font-size: var(--text-base); padding: 2px 12px; cursor: pointer;
  }
  .tab.active { color: var(--v4-text-1); border-color: var(--v4-ok); }
  .empty { color: var(--v4-text-3); font-size: var(--text-base); margin: 8px 0; }
  .thread {
    display: flex; flex-direction: column; gap: 10px;
    max-height: 320px; overflow-y: auto;
    border: 1px solid var(--v4-hairline); border-radius: 10px; background: var(--v4-inset); padding: 12px;
  }
  .msg { display: flex; gap: 8px; align-items: flex-start; }
  .mbody { min-width: 0; flex: 1 1 auto; }
  .mhead { display: flex; align-items: baseline; gap: 8px; }
  .mfrom { font-weight: 600; font-size: var(--text-base); color: var(--v4-text-2); }
  .mfrom.ok { color: var(--v4-ok); } .mfrom.warn { color: var(--v4-warn); } .mfrom.unread { color: var(--v4-unread); }
  .kind {
    font-size: 11px; letter-spacing: 0.04em; color: var(--v4-text-3);
    border: 1px solid var(--v4-hairline); border-radius: 4px; padding: 0 5px;
  }
  .kind.ask { color: var(--v4-warn); border-color: var(--v4-warn); }
  .kind.answer { color: var(--v4-ok); border-color: var(--v4-ok); }
  .mage { color: var(--v4-text-3); font-size: var(--text-base); margin-left: auto; }
  .mtext { margin: 2px 0 0; color: var(--v4-text-1); font-size: var(--text-base); line-height: 1.35; white-space: pre-wrap; word-break: break-word; }
  .dot { flex: 0 0 6px; width: 6px; height: 6px; border-radius: 999px; margin-top: 5px; background: var(--v4-idle); }
  .dot.ok { background: var(--v4-ok); } .dot.warn { background: var(--v4-warn); } .dot.unread { background: var(--v4-unread); } .dot.idle { background: var(--v4-idle); }
  .composer { display: flex; gap: 8px; align-items: flex-end; }
  textarea {
    flex: 1 1 auto; resize: vertical; border: 1px solid var(--v4-hairline); border-radius: 8px;
    background: var(--v4-raised); color: var(--v4-text-1); font: inherit; font-size: var(--text-base);
    padding: 8px 10px; box-sizing: border-box;
  }
  textarea:focus { outline: none; border-color: var(--v4-unread); }
  .send {
    border: 1px solid var(--v4-hairline); border-radius: 8px; background: var(--v4-unread); color: #04121f;
    font: inherit; font-weight: 600; font-size: var(--text-base); padding: 8px 16px; cursor: pointer;
  }
  .send:disabled { opacity: 0.45; cursor: default; }
</style>
