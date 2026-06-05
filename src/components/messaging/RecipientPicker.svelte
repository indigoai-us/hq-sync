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
  .recipient-picker {
    position: relative;
    width: 100%;
  }

  .recipient-input {
    width: 100%;
    box-sizing: border-box;
    padding: 0.5rem 0.625rem;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    background: rgba(255, 255, 255, 0.04);
    color: var(--popover-text, #e0e0e0);
    font-family: inherit;
    font-size: 0.8125rem;
    line-height: 1.4;
  }

  .recipient-input:focus {
    outline: none;
    border-color: rgba(255, 255, 255, 0.28);
    background: rgba(255, 255, 255, 0.06);
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
    padding: 0.25rem;
    list-style: none;
    max-height: 248px;
    overflow-y: auto;
    border-radius: 10px;
    border: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.1));
    background: var(--popover-bg, #1a1a22);
    backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    -webkit-backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 255, 255, 0.15) transparent;
  }

  .group-heading {
    padding: 0.4375rem 0.5rem 0.1875rem;
    font-size: 0.625rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--popover-text-muted, #8a8a98);
  }

  .suggestion {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    width: 100%;
    text-align: left;
    padding: 0.4375rem 0.5rem;
    border: none;
    border-radius: 7px;
    background: transparent;
    color: inherit;
    font-family: inherit;
    cursor: pointer;
  }

  .suggestion:hover,
  .suggestion.active {
    background: rgba(120, 170, 255, 0.16);
  }

  .suggestion.freetext {
    color: var(--popover-text-muted, #a0a0b0);
  }

  .suggestion-primary {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--popover-text, #e8e8ee);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .suggestion.freetext .suggestion-primary {
    color: var(--popover-text-muted, #b0b0bc);
    font-weight: 400;
  }

  .suggestion-secondary {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #8a8a98);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .suggestion-tag {
    margin-left: auto;
    flex-shrink: 0;
    font-size: 0.5625rem;
    font-weight: 600;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    padding: 0.0625rem 0.3125rem;
    border-radius: 999px;
    background: rgba(255, 176, 102, 0.22);
    color: #ffd9b0;
  }
</style>
