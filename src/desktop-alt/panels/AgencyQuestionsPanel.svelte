<script lang="ts">
  /**
   * Mission Control — pending agency QUESTIONS with an inline answer composer.
   * Answering writes back to the team-manager inbox (same as liaison.sh answer),
   * so you can respond directly here instead of through the liaison's
   * AskUserQuestion. Mirrors alongside the liaison — the [ans:<id>] dedup keeps
   * either side from double-answering.
   */
  import { agencyStore, submitAnswer } from '../lib/agency-store.svelte';
  import type { AgencyQuestion } from '../lib/agency';

  let drafts = $state<Record<string, string>>({});
  let busy = $state<Record<string, boolean>>({});
  let note = $state<Record<string, string>>({});

  const questions = $derived(agencyStore.questions);

  async function send(q: AgencyQuestion) {
    const answer = (drafts[q.id] ?? '').trim();
    if (!answer || busy[q.id]) return;
    busy = { ...busy, [q.id]: true };
    try {
      const res = await submitAnswer(q, answer);
      note = { ...note, [q.id]: res === 'already-answered' ? 'Already answered' : 'Sent ✓' };
      drafts = { ...drafts, [q.id]: '' };
    } catch (err) {
      console.error('answer failed:', err);
      note = { ...note, [q.id]: 'Failed to send' };
    } finally {
      busy = { ...busy, [q.id]: false };
    }
  }

  function onKey(e: KeyboardEvent, q: AgencyQuestion) {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      void send(q);
    }
  }
</script>

<div class="aqp">
  <header class="aqp-head">
    <h2>Questions <span class="count">{questions.length}</span></h2>
    <p class="sub">Answer the team directly — no liaison needed</p>
  </header>

  {#if questions.length === 0}
    <p class="empty">No questions — you're all caught up.</p>
  {:else}
    <ul class="qlist">
      {#each questions as q (q.company + '/' + q.team + '/' + q.id)}
        <li class="qcard">
          <div class="qmeta"><span class="team">{q.company}/{q.team}</span></div>
          <p class="qtext">{q.question}</p>
          <div class="qanswer">
            <textarea
              rows="2"
              placeholder="Your answer… (⌘↵ to send)"
              bind:value={drafts[q.id]}
              onkeydown={(e) => onKey(e, q)}
            ></textarea>
            <div class="qrow">
              <button
                class="send"
                onclick={() => send(q)}
                disabled={busy[q.id] || !(drafts[q.id] ?? '').trim()}
              >
                {busy[q.id] ? 'Sending…' : 'Send'}
              </button>
              {#if note[q.id]}<span class="note">{note[q.id]}</span>{/if}
            </div>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .aqp { display: flex; flex-direction: column; gap: 12px; min-height: 0; }
  .aqp-head h2 {
    margin: 0;
    font-size: var(--text-lg, 15px);
    color: var(--v4-text-1);
    display: flex; align-items: center; gap: 8px;
  }
  .count {
    font-family: var(--font-display);
    font-size: var(--text-base);
    color: var(--v4-text-3);
    background: var(--v4-inset);
    border: 1px solid var(--v4-hairline);
    border-radius: 999px;
    padding: 0 8px;
  }
  .sub { margin: 4px 0 0; color: var(--v4-text-3); font-size: var(--text-base); }
  .empty { color: var(--v4-text-3); font-size: var(--text-base); margin: 8px 0; }
  .qlist { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 10px; overflow-y: auto; }
  .qcard {
    border: 1px solid var(--v4-hairline);
    border-radius: 10px;
    background: var(--v4-inset);
    padding: 12px;
    display: flex; flex-direction: column; gap: 8px;
  }
  .qmeta .team {
    color: var(--v4-text-3);
    font-size: var(--text-base);
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .qtext { margin: 0; color: var(--v4-text-1); font-size: var(--text-base); line-height: 1.35; }
  .qanswer { display: flex; flex-direction: column; gap: 8px; }
  textarea {
    width: 100%;
    resize: vertical;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-raised);
    color: var(--v4-text-1);
    font: inherit;
    font-size: var(--text-base);
    padding: 8px 10px;
    box-sizing: border-box;
  }
  textarea:focus { outline: none; border-color: var(--v4-ok); }
  .qrow { display: flex; align-items: center; gap: 10px; }
  .send {
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-ok);
    color: #08110a;
    font: inherit;
    font-weight: 600;
    padding: 6px 14px;
    cursor: pointer;
  }
  .send:disabled { opacity: 0.45; cursor: default; }
  .note { color: var(--v4-text-3); font-size: var(--text-base); }
</style>
