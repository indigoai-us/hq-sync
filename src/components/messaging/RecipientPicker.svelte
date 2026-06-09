<script lang="ts">
  // Recipient autocomplete for the New Message compose flow (US-010).
  //
  // An email-style input with a dropdown that suggests, in priority order:
  //   (a) known contacts          — list_contacts
  //   (b) per-company members     — list_company_members for each of the
  //                                 caller's companies, grouped "From {name}"
  //   (c) a free-text "Send to {email}" row when the typed string is a valid
  //       email not already in (a) or (b)
  //
  // Matching/grouping/dedupe lives in `src/lib/recipientPicker.ts` (unit-tested);
  // this component owns the data fetches, keyboard handling, and rendering. It
  // emits the chosen recipient via the `onselect` callback and notifies the
  // parent of query changes via `onquerychange`.
  import { invoke } from '@tauri-apps/api/core';
  import {
    buildSuggestions,
    flattenRows,
    type ContactLike,
    type CompanyInfo,
    type SelectedRecipient,
    type SuggestionGroup,
    type SuggestionRow,
  } from '../../lib/recipientPicker';

  interface ContactsResponse {
    contacts: ContactLike[];
  }
  interface MembershipRow {
    companyUid: string;
    companyName: string | null;
    role: string | null;
    status: string;
  }

  interface Props {
    // The currently selected recipient (null until one is chosen). Owned by the
    // parent so it can clear the picker after a send.
    selected: SelectedRecipient | null;
    onselect: (recipient: SelectedRecipient | null) => void;
    placeholder?: string;
    disabled?: boolean;
  }

  let {
    selected = $bindable(),
    onselect,
    placeholder = 'Type a name or email…',
    disabled = false,
  }: Props = $props();

  let query = $state('');
  let open = $state(false);
  let activeIndex = $state(0);

  let contacts = $state<ContactLike[]>([]);
  let companies = $state<CompanyInfo[]>([]);
  let membersByCompany = $state<Record<string, ContactLike[]>>({});

  // Companies whose members we've already fetched (avoid re-fetching per keystroke).
  const fetchedCompanies = new Set<string>();

  const groups = $derived<SuggestionGroup[]>(
    query.trim().length === 0 && !selected
      ? []
      : buildSuggestions({ query, contacts, membersByCompany, companies }),
  );
  const flatRows = $derived<SuggestionRow[]>(flattenRows(groups));

  async function loadContacts(): Promise<void> {
    try {
      const resp = await invoke<ContactsResponse>('list_contacts');
      contacts = resp.contacts ?? [];
    } catch (err) {
      console.error('recipient-picker: list_contacts failed', err);
      contacts = [];
    }
  }

  async function loadCompanies(): Promise<void> {
    try {
      const list = await invoke<MembershipRow[]>('meetings_list_memberships');
      companies = (list ?? [])
        .filter((m) => m.status === 'active')
        .map((m) => ({ companyUid: m.companyUid, companyName: m.companyName }));
    } catch (err) {
      console.error('recipient-picker: meetings_list_memberships failed', err);
      companies = [];
    }
  }

  async function loadCompanyMembers(companyUid: string): Promise<void> {
    if (fetchedCompanies.has(companyUid)) return;
    fetchedCompanies.add(companyUid);
    try {
      const resp = await invoke<ContactsResponse>('list_company_members', { companyUid });
      membersByCompany = { ...membersByCompany, [companyUid]: resp.contacts ?? [] };
    } catch (err) {
      console.error('recipient-picker: list_company_members failed', companyUid, err);
      // Leave it unset; the group simply won't appear.
      fetchedCompanies.delete(companyUid);
    }
  }

  function onInput(value: string): void {
    query = value;
    activeIndex = 0;
    open = true;
    // A new keystroke means the prior selection no longer matches the text.
    if (selected) {
      selected = null;
      onselect(null);
    }
    // Lazily fetch every company's members once the user starts typing so
    // company groups can appear. Cheap: each company is fetched at most once.
    for (const co of companies) void loadCompanyMembers(co.companyUid);
  }

  function choose(row: SuggestionRow): void {
    selected = row.recipient;
    query = row.recipient.displayName || row.recipient.email;
    open = false;
    onselect(row.recipient);
  }

  function onKeydown(e: KeyboardEvent): void {
    if (!open || flatRows.length === 0) {
      if (e.key === 'ArrowDown') {
        open = true;
        e.preventDefault();
      }
      return;
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      activeIndex = (activeIndex + 1) % flatRows.length;
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      activeIndex = (activeIndex - 1 + flatRows.length) % flatRows.length;
    } else if (e.key === 'Enter') {
      // Only intercept Enter when a suggestion is highlighted — otherwise let it
      // bubble (the composer may treat ⌘↵ as send).
      if (!(e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        const row = flatRows[activeIndex];
        if (row) choose(row);
      }
    } else if (e.key === 'Escape') {
      open = false;
    }
  }

  // Map a flat index back to a (group, row) so highlight + click line up.
  function isActive(row: SuggestionRow): boolean {
    return flatRows[activeIndex] === row;
  }

  $effect(() => {
    void loadContacts();
    void loadCompanies();
  });
</script>

<div class="recipient-picker">
  <input
    class="recipient-input"
    type="text"
    role="combobox"
    aria-expanded={open}
    aria-controls="recipient-suggestions"
    aria-autocomplete="list"
    autocomplete="off"
    spellcheck="false"
    {placeholder}
    {disabled}
    value={query}
    oninput={(e) => onInput((e.currentTarget as HTMLInputElement).value)}
    onkeydown={onKeydown}
    onfocus={() => (open = query.trim().length > 0)}
  />

  {#if open && groups.length > 0}
    <ul class="suggestions" id="recipient-suggestions" role="listbox">
      {#each groups as group (group.key)}
        {#if group.label}
          <li class="group-heading" role="presentation">{group.label}</li>
        {/if}
        {#each group.rows as row (group.key + ':' + (row.recipient.personUid ?? row.recipient.email))}
          <li role="presentation">
            <button
              type="button"
              class="suggestion"
              class:active={isActive(row)}
              class:freetext={row.freeText}
              role="option"
              aria-selected={isActive(row)}
              onmousedown={(e) => {
                // mousedown (not click) so it fires before the input blur closes
                // the list.
                e.preventDefault();
                choose(row);
              }}
            >
              <span class="suggestion-primary">{row.primary}</span>
              {#if row.secondary}
                <span class="suggestion-secondary">{row.secondary}</span>
              {/if}
              {#if !row.freeText && row.recipient.connectionState !== 'active'}
                <span class="suggestion-tag">{row.recipient.connectionState === 'blocked' ? 'blocked' : 'not connected'}</span>
              {/if}
            </button>
          </li>
        {/each}
      {/each}
    </ul>
  {/if}
</div>

<style>
  /* Desktop "Company OS" language: hairline-bordered input + dropdown over a
     low-fill surface, one 13px size with 11px monospace caps for group headings
     and the not-connected tag, accent reserved for the active/hovered option +
     focus ring. Tokens come from the shared desktop alias layer
     (desktop-alt.css). */

  .recipient-picker {
    position: relative;
    width: 100%;
    font-family: var(--font-sans);
  }

  .recipient-input {
    width: 100%;
    box-sizing: border-box;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface-raise);
    color: var(--fg);
    font-family: var(--font-sans);
    font-size: var(--text-base);
    line-height: 1.4;
    letter-spacing: -0.006em;
  }

  .recipient-input:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .recipient-input:disabled {
    opacity: 0.6;
  }

  .suggestions {
    position: absolute;
    z-index: 20;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    margin: 0;
    padding: var(--space-1);
    list-style: none;
    max-height: 248px;
    overflow-y: auto;
    border-radius: var(--radius-md);
    border: 1px solid var(--border-strong);
    background: var(--bg);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
    scrollbar-width: thin;
    scrollbar-color: var(--scrollbar-thumb) transparent;
  }

  .group-heading {
    padding: var(--space-2) var(--space-2) var(--space-1);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--muted);
  }

  .suggestion {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    width: 100%;
    text-align: left;
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    font-family: var(--font-sans);
    cursor: pointer;
  }

  .suggestion:hover,
  .suggestion.active {
    background: var(--accent-soft);
  }

  .suggestion:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .suggestion.freetext {
    color: var(--muted);
  }

  .suggestion-primary {
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--fg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .suggestion.freetext .suggestion-primary {
    color: var(--muted-2);
    font-weight: 400;
  }

  .suggestion-secondary {
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    color: var(--muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .suggestion-tag {
    margin-left: auto;
    flex-shrink: 0;
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 2px var(--space-1);
    border-radius: var(--radius-sm);
    background: var(--surface-raise);
    color: var(--muted-2);
  }
</style>
