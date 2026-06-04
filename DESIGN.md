# DESIGN.md — HQ Sync

> The visual system for HQ Sync. Tokens here are the source of truth; they are encoded as CSS custom properties in `src/styles/popover.css`, which the desktop window (`src/desktop-alt/styles/desktop-alt.css`) `@import`s so both surfaces share one token system (see "Unified desktop token set"). Components should consume tokens, not hardcode values. Light, dark, and reduced-transparency are first-class.

## Theme

The app follows the macOS system appearance. It is not "a dark app" or "a light app." The physical scene: a person glances at their menu bar mid-task, popover open over whatever is on their desktop, in whatever ambient light and OS appearance they run. That forces three things:

- Both light and dark are real, maintained themes (driven by `prefers-color-scheme`).
- Surfaces are translucent glass over the desktop, with a reduced-transparency fallback to solid surfaces.

## Color

Strategy: **Monochrome popover, restrained-color desktop board.** Two surfaces, two budgets.

- **The menu-bar popover stays monochrome.** No brand accent hue: neutral glass layering plus a single high-contrast "primary" (white in dark, near-black in light) for every active/selected/primary state. Warnings and errors render as neutral "notice" surfaces with meaning carried by copy, not stoplight color. The one historically-permitted color is a single restrained green for the positive confirmation (notification permission granted). The restraint is the identity: a calm native citizen, not a branded app demanding attention.

- **The desktop board/project/task surfaces (`src/desktop-alt/`) earn Restrained color.** These are the operator "Company OS" surfaces where density and at-a-glance status legibility matter, and they were ported from hq-desktop. They use a small, deliberate semantic palette for *state*, not decoration: priority (P1 `--red` · P2 `--amber` · P3 `--blue`), project/story state (live·in-progress `--emerald`/`--blue`, blocked `--amber`), acceptance-criteria progress (`--emerald`), and an 8-hue label palette (`labelColor()` in `projects-model.ts`) so labels are scannable. Color marks state and identity only; layout hierarchy still comes from surface layering, weight, and spacing. No full-saturation fills, no color on inactive states.

The split is intentional: the popover is read in motion and must disappear into the task; the desktop board is a dwell surface where color does real work.

### Primary (monochrome)

`--popover-primary` marks: the active toggle fill, the selected segmented-control segment, primary CTAs (Sync, Update), and the live-sync indicator.

- Dark mode: `#ffffff` on glass, with `--popover-primary-text: #111113` as the foreground on a filled primary.
- Light mode: `#111113`, with `--popover-primary-text: #ffffff`.

### Neutrals

Layered translucent surfaces over the glass background, plus text tiers, kept neutral (no hue tint):

- Background, surface, surface-strong, border, highlight (glass layering).
- Text tiers: heading, body, muted.
- Divider, action-hover, progress track/fill.
- Notice tokens (the unified non-alarming warning/error language).

## Typography

Two surfaces, two type treatments — the popover and the big window deliberately differ, mirroring the two-budget color split. The shared `--text-*` token names persist; the big window scope (`html[data-window='desktop-alt']`) redefines them so a single declaration site governs each surface.

### Popover (4-step ramp, system font)

System font only: `-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif`. One family carries everything. Monospace (`--font-mono`: `ui-monospace, SFMono-Regular, Menlo`) only for version strings and file paths.

Fixed rem scale (not fluid), tuned for a 320px popover. Ratio ~1.15 between steps:

| Token | Size | Use |
|---|---|---|
| `--text-xs` | 11px (0.6875rem) | descriptions, paths, pill labels, section headers |
| `--text-sm` | 12px (0.75rem) | secondary buttons, version values |
| `--text-base` | 13px (0.8125rem) | row labels, body |
| `--text-lg` | 15px (0.9375rem) | screen titles, the popover's primary status line |

Weight contrast does the hierarchy work: 600 for headings and labels-that-matter, 500 for standard labels, 400 for descriptions. Section headers are `--text-xs`, 600, uppercase, with positive letter-spacing and muted color.

### Big-window type & chrome (one size, Geist — mirrors hq-console)

The big window (`src/desktop-alt/`) is a wide, dwell-time "Company OS" surface. It follows the [hq-console](../../private/hq-console) language rather than the popover ramp:

- **Two sizes, and only two.** Body, titles, names, and descriptions are all **13px** (`--text-base`; all four `--text-*` ramp tokens are redefined to 13px in the desktop-alt scope so nothing drifts). The one permitted second size is **`--text-micro` (11px)**, reserved for monospace ALL-CAPS micro-labels and pills — status tags, section eyebrows, table headers, stat-tile labels, scope pills, and `kbd` shortcuts — so the uppercase tracking reads as a quiet label rather than a shout. Everything else is 13px; hierarchy is carried by **weight** (400 body / 600 headings + labels-that-matter) and the **grey (`--muted`) / white (`--fg`) split**, not size. A page title is 13px/650 white, not a bigger font.
- **Geist.** Bundled offline via `@fontsource-variable/geist` + `@fontsource-variable/geist-mono` (imported from `desktop-alt/main.ts`). `'Geist Variable'` carries body + headings; `--font-mono` (`'Geist Mono Variable'`) is reserved for IDs, paths, and version strings. A faint negative optical tracking (`-0.006em`) matches hq-console's finish.
- **Chrome.** Hairline borders over low-fill surfaces, square-ish corners, grey uppercase section headers + white body, generous horizontal padding. A restrained **Indigo accent** (`--accent: #6366f1`, `--accent-soft`) is used for <10% of the surface: the active-nav "you are here" dot and the focus ring only.
- **Card grids (the "Foundry tile").** Browsable collections — the Library (`LibraryList.svelte`) and the Board's Projects grid (`ProjectRow.svelte` in `ProjectListView`'s `auto-fill, minmax(296px, 1fr)` grid) — render as cards, not rows. The house card is a near-square (`4px`) hairline tile with a thin (3px) status/kind-colored left **accent bar** (a positioned indicator, not a `border-left` hack), a monospace ALL-CAPS micro-label, a scope pill, and a 2-line-clamped description. This thin in-card accent bar is the one sanctioned left-edge color cue; it is distinct from the banned decorative side-stripe on alerts/callouts/plain list items.
- **Why it diverges.** The popover is read in motion and must disappear into the menu bar; the big window is a dwell surface where a calm single-size, hairline system reads as a finished product, not a CLI. Semantic state color (priority, project/story state, AC progress, 8-hue labels) still applies on the board — it marks state, not brand, and never fills.

The single restrained green (`--popover-success`) positive-confirmation rule and the absolute design bans (no side-stripe borders, no gradient text, no decorative glassmorphism) hold on both surfaces.

## Spacing

A 4px base scale. Same padding everywhere is monotony; vary deliberately for rhythm.

| Token | Value |
|---|---|
| `--space-1` | 4px |
| `--space-2` | 8px |
| `--space-3` | 12px |
| `--space-4` | 16px |
| `--space-5` | 20px |
| `--space-6` | 24px |

Row vertical padding is `--space-2`/`--space-3`; surface insets are `--space-4`; section gaps are `--space-5`.

## Radius

The current values drift (7, 9, 10, 18). Consolidate to a scale:

| Token | Value | Use |
|---|---|---|
| `--radius-sm` | 8px | pills, small buttons, toggles, segments |
| `--radius-md` | 10px | grouped buttons, larger controls |
| `--radius-lg` | 14px | section group cards |
| `--radius-xl` | 18px | the popover window itself |

## Elevation

The window carries the only real shadow (native NSWindow shadow). Inside the popover, elevation is expressed by surface layering (translucent white/black alphas), never by drop shadows on inner elements. Section groups sit on a slightly stronger surface than the popover background.

## Motion

Transform and opacity only; never animate layout properties. 150–250ms, ease-out. Motion conveys state (toggle knob slide, section reveal, the sync progress fill), never decoration. No bounce, no orchestrated load sequence. Respect `prefers-reduced-motion` by collapsing durations to near-zero.

## Components

- **Toggle switch.** 36×20 pill, knob slides on a transform. Active fill is `--popover-primary`; knob flips to `--popover-primary-text` so it stays visible on the fill.
- **Segmented control** (release channel). Inset track; the active segment carries the `--popover-primary` fill.
- **Secondary button** ("Change…", "Check Now", "Enable"). Surface fill, muted text, full border, `--radius-md`.
- **Pill** (drift count, version, permission status). `--text-xs`, `--radius-sm`. The drift-count pill gets a visible label so "14" is never naked.
- **Grouped inset list** (the core Settings primitive). Rows live inside a section group: a `--surface` card at `--radius-lg`, hairline dividers *between rows within the group only*, a muted uppercase section header above it, and `--space-5` between groups. This is the macOS System Settings idiom: familiar, scannable, and it collapses a flat 13-row scroll into four labeled clusters.
- **Story card** (`StoryCard.svelte`, projects board). A focusable `<button>` surface card (`--bg`, `--radius-sm`, hairline border) carrying a monospace story ID, a 2-line-clamped title, up to two `LabelChip`s plus a `+N` overflow pill, a color-coded priority badge (`P1` `--red` · `P2` `--amber` · `P3` `--blue`) and optional model-hint badge, and an acceptance-criteria progress bar (`--emerald` fill, `scaleX` transform — matching hq-desktop's green AC progress). Completed stories (`passes`) render at 0.6 opacity; the focus ring is `2px solid var(--blue)`. AC progress carries no per-AC done flags, so it is derived from the story-level `passes` (full when complete, empty otherwise), mirroring hq-desktop.
- **Label chip** (`LabelChip.svelte`). A small pill whose deterministic color comes from the `labelColor()` palette — an 8-hue, low-saturation set (blue/purple/teal/pink/orange/cyan/lime/rose, mirroring hq-desktop) hashed stably from the label string — fed into inline `--chip-bg`/`--chip-border`/`--chip-fg` custom properties. Hues are translucent fills + matching borders + a brighter readable foreground, tuned for the dark desktop surface; no hardcoded hex in components.

## The settings architecture

Four labeled groups, ordered by how often a user touches them:

1. **Sync** — HQ Folder, Sync on Launch, Auto-sync, Instant sync, Sync personal vault.
2. **Notifications** — Notifications, System permission, Share notifications, Direct messages.
3. **Updates** — Use staging channel, Release channel, Check for Updates.
4. **General** — Start at Login, Version.

Operator-gated rows (staging channel, share/DM notifications, release channel) appear inside their group only when enabled, so a basic user sees three short groups, not four padded ones.

## The footer menu architecture

The popover footer keeps the HQ-core status row (version + labeled drift pill + update action), then a clear separation: primary navigation (Recent Changes, Settings) reads at full weight, while destructive actions (Sign out, Quit) are demoted to a compact, visually distinct row beneath a divider so they can't be hit by reflex.

## Unified desktop token set

HQ Sync ships two surfaces — the menubar **popover** and the Indigo **desktop window** (`src/desktop-alt/`). They are **one design system**, not two. There is a single source of truth for tokens:

- **Canonical tokens** live in `src/styles/popover.css` (`--popover-*`, `--text-*`, `--space-*`, `--radius-*`, `--popover-blur`). They define light, dark, and reduced-transparency variants via `@media (prefers-color-scheme: …)` and `@media (prefers-reduced-transparency: reduce)`.
- The desktop window's stylesheet (`src/desktop-alt/styles/desktop-alt.css`) **`@import`s `popover.css`**, so both windows resolve from the same definitions. There is no second, drifting copy.

### Desktop semantic alias layer

The desktop window predates the canonical naming and uses a shorter vocabulary. Those names are kept as a thin **alias layer** scoped under `html[data-window='desktop-alt']` so components don't need a mass rename. Every alias derives from a canonical primitive or holds a documented desktop-specific neutral — it is never an independent re-definition.

| Alias | Role | Source / value |
|---|---|---|
| `--bg` | shell / sidebar / main background | desktop neutral (`#0a0a0a` dark · `#f6f6f8` light) |
| `--bg-subtle` | status bar | desktop neutral (`#050505` dark · `#ececef` light) |
| `--bg-body` | document body behind the window surface | desktop neutral (`#131316` dark · `#e7e7ea` light) |
| `--desktop` | reserved desktop-canvas surface | desktop neutral (`#1a1a1c` dark · `#ffffff` light) |
| `--fg` | primary text / headings | desktop neutral (`#fafafa` dark · `#111113` light) |
| `--muted` / `--muted-2` / `--muted-3` | secondary → faintest text tiers | neutral alpha (same language as `--popover-text-muted`) |
| `--border` / `--border-strong` | hairlines, dividers | neutral alpha (same language as `--popover-border` / `--popover-divider`) |
| `--row-hover` / `--row-active` | row interaction surfaces | neutral alpha (same language as `--popover-action-hover`) |
| `--scrollbar-thumb` / `--scrollbar-thumb-hover` | scrollbar | neutral alpha |
| `--emerald` | positive-confirmation green | **aliases `--popover-success`** — the one permitted color |
| `--amber` / `--blue` / `--red` | legacy meetings status markers | retained semantic markers, not brand accent |

### Usage rules

1. **Consume tokens, never hardcode.** New desktop components use the aliases above (or canonical `--popover-*` / `--text-*` / `--space-*` / `--radius-*` directly). Do not introduce new hardcoded hex/rgba for anything a token already covers.
2. **Two color budgets (see Color above).** The popover stays monochrome — hierarchy from surface layering, weight, and spacing; the only color is the restrained green confirmation. The desktop board/project/task surfaces (`src/desktop-alt/`) earn Restrained *semantic* color via the existing markers — `--emerald` (live·complete·AC progress), `--blue` (in-progress·active·progress), `--amber` (blocked·P2), `--red` (P1) — plus the 8-hue `labelColor()` label palette. Color marks state/priority/identity only, never decoration and never on inactive states; layout hierarchy is still structural. Do not introduce new hardcoded hues beyond these markers and the label palette.
3. **Light, dark, and reduced-transparency are all first-class** for the desktop window, exactly as for the popover. The window follows system appearance (`color-scheme: light dark`); all three media blocks (`prefers-color-scheme: light`, `prefers-reduced-transparency: reduce`, and their combination) are covered, with dark values preserved byte-for-byte from the pre-unification stylesheet. The desktop window is fully opaque (no backdrop blur), so its reduced-transparency variant only promotes alpha surfaces to solid neutrals.
