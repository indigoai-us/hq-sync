# DESIGN.md — HQ Sync

> The visual system for HQ Sync. Tokens here are the source of truth; they are encoded as CSS custom properties in `src/styles/popover.css`. Components should consume tokens, not hardcode values. Light, dark, and reduced-transparency are first-class.

## Theme

The app follows the macOS system appearance. It is not "a dark app" or "a light app." The physical scene: a person glances at their menu bar mid-task, popover open over whatever is on their desktop, in whatever ambient light and OS appearance they run. That forces three things:

- Both light and dark are real, maintained themes (driven by `prefers-color-scheme`).
- Surfaces are translucent glass over the desktop, with a reduced-transparency fallback to solid surfaces.

## Color

Strategy: **Monochrome.** This is a deliberate, owner-set constraint: no brand accent hue. The whole palette is neutral glass layering plus a single high-contrast "primary" (white in dark, near-black in light) that carries every active, selected, and primary state. There is no severity palette: warnings and errors render as neutral "notice" surfaces, with meaning carried by copy. The one permitted color is a single restrained green for the one genuinely positive confirmation (notification permission granted), nothing more.

The restraint is the identity. A monochrome menu-bar utility reads as a calm native citizen, not a branded app demanding attention. Hierarchy is carried entirely by surface layering, weight, and spacing, never by color.

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

System font only: `-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif`. One family carries everything. Monospace (`--font-mono`: `ui-monospace, SFMono-Regular, Menlo`) only for version strings and file paths.

Fixed rem scale (not fluid), tuned for a 320px popover. Ratio ~1.15 between steps:

| Token | Size | Use |
|---|---|---|
| `--text-xs` | 11px (0.6875rem) | descriptions, paths, pill labels, section headers |
| `--text-sm` | 12px (0.75rem) | secondary buttons, version values |
| `--text-base` | 13px (0.8125rem) | row labels, body |
| `--text-lg` | 15px (0.9375rem) | screen titles, the popover's primary status line |

Weight contrast does the hierarchy work: 600 for headings and labels-that-matter, 500 for standard labels, 400 for descriptions. Section headers are `--text-xs`, 600, uppercase, with positive letter-spacing and muted color.

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

## The settings architecture

Four labeled groups, ordered by how often a user touches them:

1. **Sync** — HQ Folder, Sync on Launch, Auto-sync, Instant sync, Sync personal vault.
2. **Notifications** — Notifications, System permission, Share notifications, Direct messages.
3. **Updates** — Use staging channel, Release channel, Check for Updates.
4. **General** — Start at Login, Version.

Operator-gated rows (staging channel, share/DM notifications, release channel) appear inside their group only when enabled, so a basic user sees three short groups, not four padded ones.

## The footer menu architecture

The popover footer keeps the HQ-core status row (version + labeled drift pill + update action), then a clear separation: primary navigation (Recent Changes, Settings) reads at full weight, while destructive actions (Sign out, Quit) are demoted to a compact, visually distinct row beneath a divider so they can't be hit by reflex.
