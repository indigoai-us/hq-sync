<script lang="ts">
  /**
   * ProfilePanel — the desktop-alt **Profile** tab body (US-016).
   *
   * Lets a creator manage their public marketplace identity:
   *   • CLAIM step — when the user hasn't claimed a handle yet, a handle field
   *     with inline FORMAT feedback (mirrors the server rules) and authoritative
   *     claim feedback: a 409 shows "unavailable", a 400/403 shows the server's
   *     format/reserved reason inline (AC1, AC3).
   *   • EDIT step — once a handle is claimed (or the user enters their existing
   *     one), a bio textarea, native avatar upload, a social-links repeater
   *     (label + url), and a tip-URL field. URL fields carry a client-side
   *     http(s)-only hint; the server enforces it authoritatively (AC1).
   *   • Save → calls update_creator_profile (+ uploads a pending avatar), shows
   *     success, then fetches GET /v1/creators/{handle} to render the PUBLIC
   *     profile preview (AC2).
   *
   * The authoritative checks all live on the server (handle uniqueness/format/
   * reserved screening, URL scheme, avatar type/size, own-profile ownership);
   * this panel adds cheap client-side hints for fast feedback but never claims a
   * result it can't prove. Mirrors SubmitPanel/MarketplacePanel conventions:
   * Svelte 5 runes, the shared desktop-alt CSS variables, explicit
   * idle/busy/error/success states.
   */
  import {
    checkHandleFormat,
    checkHttpUrl,
    claimCreatorHandle,
    getCreatorProfile,
    pickAvatarFile,
    toClaimError,
    updateCreatorProfile,
    uploadCreatorAvatar,
    type CreatorProfile,
    type PublicCreatorPreview,
    type SocialLink,
  } from '../lib/marketplace';

  // ── Step state ───────────────────────────────────────────────────────────
  /** The claimed handle, once known (claim success OR a prior session). null = claim step. */
  let handle = $state<string | null>(null);

  // ── Claim step state ───────────────────────────────────────────────────────
  let handleInput = $state('');
  let claiming = $state(false);
  /** Inline claim error (format / reserved / generic). */
  let claimError = $state<string | null>(null);
  /** True when the entered handle is already taken (focused "unavailable"). */
  let handleTaken = $state(false);

  // Live FORMAT hint (client-side mirror of the server rules). Authoritative
  // check is the claim itself — this just gives instant feedback while typing.
  const formatHint = $derived.by(() => {
    if (handleInput.trim().length === 0) return null;
    const check = checkHandleFormat(handleInput);
    return check.ok ? null : check.reason;
  });
  const canClaim = $derived(
    handleInput.trim().length > 0 && formatHint === null && !claiming,
  );

  // ── Edit step state ─────────────────────────────────────────────────────────
  let bio = $state('');
  let tipUrl = $state('');
  let socialLinks = $state<SocialLink[]>([]);
  /** A picked-but-not-yet-uploaded avatar path (uploaded on Save). */
  let pendingAvatarPath = $state<string | null>(null);
  /** The currently-rendered avatar URL (from a prior upload / save echo). */
  let avatarUrl = $state<string | null>(null);

  let saving = $state(false);
  let saveError = $state<string | null>(null);
  let saved = $state(false);

  // ── Preview state ────────────────────────────────────────────────────────
  let preview = $state<PublicCreatorPreview | null>(null);
  let previewLoading = $state(false);
  let previewError = $state<string | null>(null);

  // Client-side URL hints (server is authoritative).
  const tipUrlHint = $derived.by(() => {
    const check = checkHttpUrl(tipUrl);
    return check.ok ? null : check.reason;
  });
  function socialUrlHint(url: string): string | null {
    const check = checkHttpUrl(url);
    return check.ok ? null : check.reason;
  }
  const hasInvalidSocialUrl = $derived(
    socialLinks.some((l) => l.url.trim().length > 0 && socialUrlHint(l.url) !== null),
  );
  const canSave = $derived(
    !saving && tipUrlHint === null && !hasInvalidSocialUrl,
  );

  const avatarPreviewName = $derived(
    pendingAvatarPath
      ? (pendingAvatarPath.split('/').filter(Boolean).pop() ?? pendingAvatarPath)
      : null,
  );

  // ── Claim flow ──────────────────────────────────────────────────────────────
  async function claim(): Promise<void> {
    if (!canClaim) return;
    claiming = true;
    claimError = null;
    handleTaken = false;
    try {
      const result = await claimCreatorHandle(handleInput);
      handle = result.handle;
      // Move into the edit step with a clean slate.
      bio = '';
      tipUrl = '';
      socialLinks = [];
      pendingAvatarPath = null;
      avatarUrl = null;
      saved = false;
    } catch (err) {
      const ce = toClaimError(err);
      handleTaken = ce.taken;
      claimError = ce.message;
    } finally {
      claiming = false;
    }
  }

  // ── Avatar ───────────────────────────────────────────────────────────────
  async function chooseAvatar(): Promise<void> {
    if (saving) return;
    try {
      const picked = await pickAvatarFile();
      if (picked) {
        pendingAvatarPath = picked;
        saved = false;
      }
    } catch (err) {
      saveError = err instanceof Error ? err.message : String(err);
    }
  }

  // ── Social links repeater ──────────────────────────────────────────────────
  function addSocial(): void {
    socialLinks = [...socialLinks, { label: '', url: '' }];
  }
  function removeSocial(index: number): void {
    socialLinks = socialLinks.filter((_, i) => i !== index);
  }

  // ── Save flow ──────────────────────────────────────────────────────────────
  async function save(): Promise<void> {
    if (!handle || !canSave) return;
    saving = true;
    saveError = null;
    saved = false;
    try {
      // 1. Upload a pending avatar first (server owns the avatarKey).
      if (pendingAvatarPath) {
        avatarUrl = await uploadCreatorAvatar(pendingAvatarPath);
        pendingAvatarPath = null;
      }
      // 2. Persist bio / tipUrl / socials. Only send non-empty social rows
      //    (a blank repeater row is dropped; an empty tipUrl clears it).
      const cleanedSocials = socialLinks
        .map((l) => ({ label: l.label.trim(), url: l.url.trim() }))
        .filter((l) => l.label.length > 0 || l.url.length > 0);
      const merged: CreatorProfile = await updateCreatorProfile({
        bio: bio.trim(),
        tipUrl: tipUrl.trim(),
        socialLinks: cleanedSocials,
      });
      // Reflect the server's merged state back into the form.
      bio = merged.bio ?? '';
      tipUrl = merged.tipUrl ?? '';
      socialLinks = merged.socialLinks ?? [];
      if (merged.avatarUrl) avatarUrl = merged.avatarUrl;
      saved = true;
      // 3. Refresh the public preview from the public route.
      await loadPreview();
    } catch (err) {
      saveError = err instanceof Error ? err.message : String(err);
    } finally {
      saving = false;
    }
  }

  async function loadPreview(): Promise<void> {
    if (!handle) return;
    previewLoading = true;
    previewError = null;
    try {
      preview = await getCreatorProfile(handle);
    } catch (err) {
      preview = null;
      previewError = err instanceof Error ? err.message : String(err);
    } finally {
      previewLoading = false;
    }
  }
</script>

<div class="profile" data-testid="profile-panel">
  {#if handle === null}
    <!-- ── Claim step (AC1) ──────────────────────────────────────────────── -->
    <header class="profile-head">
      <h2 class="profile-title">Claim your creator handle</h2>
      <p class="profile-sub">
        Your handle is your public identity on the marketplace — it attributes
        every pack you publish. Choose carefully; it can’t be changed later.
      </p>
    </header>

    <section class="claim" data-testid="profile-claim">
      <label class="field-label" for="profile-handle">Handle</label>
      <div class="handle-row">
        <span class="handle-at" aria-hidden="true">@</span>
        <input
          id="profile-handle"
          class="input handle-input"
          class:invalid={formatHint !== null || handleTaken}
          type="text"
          autocomplete="off"
          autocapitalize="off"
          spellcheck="false"
          placeholder="your-handle"
          data-testid="profile-handle-input"
          bind:value={handleInput}
          oninput={() => {
            claimError = null;
            handleTaken = false;
          }}
        />
      </div>

      {#if formatHint}
        <p class="field-hint warn" data-testid="profile-handle-format-hint">{formatHint}</p>
      {:else if handleInput.trim().length > 0}
        <p class="field-hint ok" data-testid="profile-handle-ok-hint">Looks valid — claim to confirm it’s available.</p>
      {:else}
        <p class="field-hint">3–30 characters · lowercase letters, numbers, hyphens, underscores.</p>
      {/if}

      <button
        type="button"
        class="btn btn-primary"
        data-testid="profile-claim-button"
        onclick={claim}
        disabled={!canClaim}
      >
        {claiming ? 'Claiming…' : 'Claim handle'}
      </button>

      {#if handleTaken}
        <p class="claim-status taken" role="alert" data-testid="profile-handle-taken">
          <strong>@{handleInput.trim()}</strong> is unavailable — try another.
        </p>
      {:else if claimError}
        <p class="claim-status error" role="alert" data-testid="profile-claim-error">{claimError}</p>
      {/if}
    </section>
  {:else}
    <!-- ── Edit step (AC1) ───────────────────────────────────────────────── -->
    <header class="profile-head">
      <h2 class="profile-title">
        Your profile <span class="handle-badge" data-testid="profile-claimed-handle">@{handle}</span>
      </h2>
      <p class="profile-sub">Edit your public bio, avatar, links, and tip URL.</p>
    </header>

    <section class="edit" data-testid="profile-edit">
      <!-- Avatar -->
      <div class="field">
        <span class="field-label">Avatar</span>
        <div class="avatar-row">
          {#if avatarUrl}
            <img class="avatar-img" src={avatarUrl} alt="Your avatar" data-testid="profile-avatar-img" />
          {:else}
            <div class="avatar-placeholder" aria-hidden="true">{handle.slice(0, 1).toUpperCase()}</div>
          {/if}
          <div class="avatar-actions">
            <button
              type="button"
              class="btn btn-secondary"
              data-testid="profile-avatar-choose"
              onclick={chooseAvatar}
              disabled={saving}
            >
              {pendingAvatarPath || avatarUrl ? 'Change image…' : 'Upload image…'}
            </button>
            {#if avatarPreviewName}
              <span class="avatar-chosen" title={pendingAvatarPath}>{avatarPreviewName}</span>
            {/if}
            <span class="field-hint">PNG, JPEG, WebP, or GIF · up to 2 MiB.</span>
          </div>
        </div>
      </div>

      <!-- Bio -->
      <div class="field">
        <label class="field-label" for="profile-bio">Bio</label>
        <textarea
          id="profile-bio"
          class="input textarea"
          rows="4"
          maxlength="2000"
          placeholder="Tell people what you build…"
          data-testid="profile-bio"
          bind:value={bio}
        ></textarea>
      </div>

      <!-- Social links repeater -->
      <div class="field">
        <span class="field-label">Social links</span>
        {#if socialLinks.length === 0}
          <p class="field-hint">No links yet.</p>
        {/if}
        {#each socialLinks as link, i (i)}
          <div class="social-row" data-testid="profile-social-row">
            <input
              class="input social-label"
              type="text"
              placeholder="Label (e.g. GitHub)"
              aria-label="Social link label"
              data-testid="profile-social-label"
              bind:value={link.label}
            />
            <input
              class="input social-url"
              class:invalid={link.url.trim().length > 0 && socialUrlHint(link.url) !== null}
              type="url"
              placeholder="https://…"
              aria-label="Social link URL"
              data-testid="profile-social-url"
              bind:value={link.url}
            />
            <button
              type="button"
              class="icon-btn"
              aria-label="Remove link"
              data-testid="profile-social-remove"
              onclick={() => removeSocial(i)}
            >×</button>
          </div>
          {#if link.url.trim().length > 0 && socialUrlHint(link.url)}
            <p class="field-hint warn" data-testid="profile-social-url-hint">{socialUrlHint(link.url)}</p>
          {/if}
        {/each}
        <button
          type="button"
          class="btn btn-secondary add-social"
          data-testid="profile-add-social"
          onclick={addSocial}
        >+ Add link</button>
      </div>

      <!-- Tip URL -->
      <div class="field">
        <label class="field-label" for="profile-tip">Tip / sponsor link</label>
        <input
          id="profile-tip"
          class="input"
          class:invalid={tipUrlHint !== null}
          type="url"
          placeholder="https://ko-fi.com/you"
          data-testid="profile-tip"
          bind:value={tipUrl}
        />
        {#if tipUrlHint}
          <p class="field-hint warn" data-testid="profile-tip-hint">{tipUrlHint}</p>
        {:else}
          <p class="field-hint">A plain link shown on your profile — only http(s) URLs.</p>
        {/if}
      </div>

      <div class="save-row">
        <button
          type="button"
          class="btn btn-primary"
          data-testid="profile-save"
          onclick={save}
          disabled={!canSave}
        >
          {saving ? 'Saving…' : 'Save profile'}
        </button>
        {#if saved}
          <span class="save-ok" role="status" data-testid="profile-save-ok">✓ Saved.</span>
        {/if}
      </div>

      {#if saveError}
        <p class="claim-status error" role="alert" data-testid="profile-save-error">{saveError}</p>
      {/if}
    </section>

    <!-- ── Public preview (AC2) ──────────────────────────────────────────── -->
    <section class="preview" data-testid="profile-preview">
      <div class="preview-head">
        <h3 class="preview-title">Public profile preview</h3>
        <button
          type="button"
          class="btn btn-secondary"
          data-testid="profile-preview-refresh"
          onclick={loadPreview}
          disabled={previewLoading}
        >
          {previewLoading ? 'Loading…' : preview ? 'Refresh' : 'Load preview'}
        </button>
      </div>

      {#if previewError}
        <p class="preview-empty" data-testid="profile-preview-empty">
          {previewError === 'no public profile yet'
            ? 'Save your profile to see how it looks publicly.'
            : previewError}
        </p>
      {:else if preview}
        <article class="preview-card">
          <div class="preview-id">
            {#if preview.creator.avatarUrl}
              <img class="preview-avatar" src={preview.creator.avatarUrl} alt="" />
            {:else}
              <div class="preview-avatar placeholder" aria-hidden="true">
                {(preview.creator.displayName || preview.creator.handle).slice(0, 1).toUpperCase()}
              </div>
            {/if}
            <div class="preview-names">
              <span class="preview-name" data-testid="profile-preview-name"
                >{preview.creator.displayName || preview.creator.handle}</span
              >
              <span class="preview-handle">@{preview.creator.handle}</span>
            </div>
          </div>

          {#if preview.creator.bio}
            <p class="preview-bio" data-testid="profile-preview-bio">{preview.creator.bio}</p>
          {/if}

          {#if preview.creator.socialLinks.length > 0}
            <div class="preview-links">
              {#each preview.creator.socialLinks as link (link.url)}
                <a class="preview-link" href={link.url} target="_blank" rel="noreferrer noopener"
                  >{link.label}</a
                >
              {/each}
            </div>
          {/if}

          {#if preview.creator.tipUrl}
            <a
              class="preview-tip"
              href={preview.creator.tipUrl}
              target="_blank"
              rel="noreferrer noopener"
              data-testid="profile-preview-tip">♥ Tip</a
            >
          {/if}

          <div class="preview-listings">
            <span class="preview-listings-title"
              >{preview.listings.length}
              {preview.listings.length === 1 ? 'published pack' : 'published packs'}</span
            >
            {#each preview.listings as listing (listing.id)}
              <span class="preview-listing" data-testid="profile-preview-listing">{listing.name}</span>
            {/each}
          </div>
        </article>
      {:else}
        <p class="preview-empty">Save your profile to preview it, or load the current public version.</p>
      {/if}
    </section>
  {/if}
</div>

<style>
  .profile {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    min-width: 0;
    max-width: 640px;
  }

  .profile-head {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .profile-title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    color: var(--fg);
    font-size: var(--text-lg, 15px);
    font-weight: 700;
  }

  .profile-sub {
    margin: 0;
    color: var(--muted);
    font-size: var(--text-base);
    max-width: 56ch;
  }

  .handle-badge {
    padding: 1px 8px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--blue) 16%, transparent);
    color: var(--blue);
    font-size: var(--text-base);
    font-weight: 650;
  }

  /* ---- shared field + input --------------------------------------------- */
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .field-label {
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .input {
    height: 32px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
  }

  .input::placeholder {
    color: var(--muted-3);
  }

  .input:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 1px;
  }

  .input.invalid {
    border-color: color-mix(in srgb, var(--red, #e5484d) 60%, var(--border));
  }

  .textarea {
    height: auto;
    padding: var(--space-2) var(--space-3);
    line-height: 1.5;
    resize: vertical;
  }

  .field-hint {
    margin: 0;
    color: var(--muted);
    font-size: var(--text-micro);
  }
  .field-hint.warn {
    color: var(--amber);
  }
  .field-hint.ok {
    color: var(--emerald);
  }

  /* ---- claim ------------------------------------------------------------ */
  .claim,
  .edit {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
  }

  .handle-row {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .handle-at {
    color: var(--muted-3);
    font-size: var(--text-base);
    font-weight: 700;
  }

  .handle-input {
    flex: 1 1 auto;
    min-width: 0;
  }

  .claim-status {
    margin: 0;
    font-size: var(--text-base);
  }
  .claim-status.taken {
    color: var(--amber);
  }
  .claim-status.error {
    color: var(--red, #e5484d);
    overflow-wrap: anywhere;
  }

  /* ---- avatar ----------------------------------------------------------- */
  .avatar-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .avatar-img,
  .avatar-placeholder {
    width: 56px;
    height: 56px;
    border-radius: 999px;
    object-fit: cover;
    flex: 0 0 auto;
  }

  .avatar-placeholder {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--border);
    background: var(--row-hover);
    color: var(--muted-2);
    font-size: var(--text-lg, 15px);
    font-weight: 700;
  }

  .avatar-actions {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .avatar-chosen {
    color: var(--fg);
    font-size: var(--text-micro);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 240px;
  }

  /* ---- social repeater -------------------------------------------------- */
  .social-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .social-label {
    flex: 0 0 140px;
    min-width: 0;
  }

  .social-url {
    flex: 1 1 auto;
    min-width: 0;
  }

  .icon-btn {
    display: inline-flex;
    flex: 0 0 auto;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 1;
    cursor: pointer;
  }
  .icon-btn:hover {
    border-color: var(--border-strong);
    color: var(--fg);
  }

  .add-social {
    align-self: flex-start;
  }

  /* ---- save ------------------------------------------------------------- */
  .save-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .save-ok {
    color: var(--emerald);
    font-size: var(--text-base);
    font-weight: 600;
  }

  /* ---- buttons ---------------------------------------------------------- */
  .btn {
    display: inline-flex;
    align-items: center;
    height: 32px;
    padding: 0 var(--space-3);
    border-radius: 4px;
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 600;
    cursor: pointer;
    align-self: flex-start;
    transition:
      background 140ms ease,
      border-color 140ms ease,
      opacity 140ms ease;
  }
  .btn:hover:not(:disabled) {
    border-color: var(--border-strong);
    background: var(--row-hover);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }
  .btn-primary {
    border-color: color-mix(in srgb, var(--blue) 55%, transparent);
    background: var(--blue);
    color: #fff;
  }
  .btn-primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--blue) 88%, #000);
  }
  .btn-secondary {
    background: var(--bg);
  }

  /* ---- preview ---------------------------------------------------------- */
  .preview {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .preview-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .preview-title {
    margin: 0;
    color: var(--muted-3);
    font-size: var(--text-micro);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .preview-empty {
    margin: 0;
    padding: var(--space-4);
    border: 1px dashed var(--border-strong);
    border-radius: 4px;
    background: var(--row-active);
    color: var(--muted);
    font-size: var(--text-base);
    text-align: center;
  }

  .preview-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
  }

  .preview-id {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .preview-avatar {
    width: 48px;
    height: 48px;
    border-radius: 999px;
    object-fit: cover;
    flex: 0 0 auto;
  }
  .preview-avatar.placeholder {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--border);
    background: var(--row-hover);
    color: var(--muted-2);
    font-weight: 700;
  }

  .preview-names {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .preview-name {
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 680;
  }

  .preview-handle {
    color: var(--blue);
    font-size: var(--text-base);
    font-weight: 600;
  }

  .preview-bio {
    margin: 0;
    color: var(--muted-2);
    font-size: var(--text-base);
    line-height: 19px;
    overflow-wrap: anywhere;
  }

  .preview-links {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .preview-link {
    color: var(--blue);
    font-size: var(--text-base);
    font-weight: 600;
    text-decoration: none;
  }
  .preview-link:hover {
    text-decoration: underline;
  }

  .preview-tip {
    align-self: flex-start;
    padding: 4px 12px;
    border: 1px solid color-mix(in srgb, var(--red, #e5484d) 40%, transparent);
    border-radius: 999px;
    background: color-mix(in srgb, var(--red, #e5484d) 10%, transparent);
    color: var(--red, #e5484d);
    font-size: var(--text-base);
    font-weight: 650;
    text-decoration: none;
  }

  .preview-listings {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding-top: var(--space-2);
    border-top: 1px solid var(--border);
  }

  .preview-listings-title {
    color: var(--muted);
    font-size: var(--text-micro);
    font-weight: 600;
  }

  .preview-listing {
    padding: 1px 8px;
    border: 1px solid var(--border);
    border-radius: 3px;
    background: var(--row-active);
    color: var(--muted-2);
    font-size: var(--text-micro);
  }

  @media (prefers-reduced-motion: reduce) {
    .btn {
      transition: none;
    }
  }
</style>
