# PRODUCT.md — HQ Sync

> Design context for the HQ Sync menu bar app. Consumed by the `impeccable` design skill and any contributor doing UI work. Keep this current when the product's purpose, audience, or voice shifts.

register: product

## Product purpose

HQ Sync is a macOS menu bar app that keeps a user's HQ folder synced with HQ Cloud. It wraps the `hq sync` engine for people who should never have to touch a terminal. It lives in the system tray, opens as a small translucent popover, and its entire job is to answer one question at a glance: *are my files safe and current?* Everything else (settings, recent changes, sign-in, conflict resolution, HQ-core updates) is secondary surface that the user visits rarely and leaves quickly.

This is a utility, not a destination. Success is the user opening the popover, getting their answer in under a second, and closing it. The interface should disappear into that task.

## Users

- **Primary: non-technical team members.** They were onboarded onto HQ by someone else. They do not know what a git branch is and never will. They need confidence that sync works and a clear, non-alarming path when it does not.
- **Secondary: the operator / power user (e.g. @getindigo.ai builders).** They get extra surface gated behind an identity check: release channels, the staging update channel, drift diagnostics, share and DM notifications. For them density is acceptable; for the primary user it must never leak.

The same binary serves both. Gated features must stay invisible to users who don't have them, so the default experience reads as simple even though the app is capable.

## Brand and tone

HQ's parent brand is Indigo. The product voice is calm, plain, and quietly competent: it tells the truth about state without drama. A failed sync is "couldn't reach the vault," not a red alarm. Copy is short and human. The app never blames the user and never shows a raw error where a plain sentence would do.

Visually the app is a native macOS citizen first and an Indigo product second. It uses system materials (translucent glass over the desktop), follows the OS light/dark appearance, and honors reduced-transparency. The Indigo identity shows up as a restrained accent on the things that matter (the live/selected/primary states), not as chrome.

## Anti-references

- **Not a dashboard.** No hero metrics, no big-number-small-label stat cards, no data-viz. It's a status line and a short list of toggles.
- **Not a settings labyrinth.** The whole point is that there isn't much to configure. The settings surface should feel shorter than the user fears, not longer.
- **Not alarmist.** No red banners, no yellow warning triangles, no severity colors as the primary signal. The codebase already retired its warning/error palette in favor of neutral "notice" surfaces; keep it that way. Meaning is carried by copy and a single calm accent, not by stoplight color.
- **Not a generic Electron tray app.** No flat gray chrome, no mismatched custom controls. It should feel like it shipped with macOS.

## Strategic principles

1. **Answer first, controls second.** The top of every surface states the current truth. Actions and configuration come after.
2. **Glanceability over completeness.** When in doubt, hide detail behind a click. The popover is read in motion.
3. **Familiar idioms win.** This is a product surface: use the patterns users already know from macOS System Settings and the menu bar. Don't reinvent a toggle or a list for flavor.
4. **Destructive actions are demoted and separated.** Sign out and Quit must never sit at the same weight, flush against, the things people click often.
5. **Gated complexity stays gated.** Operator features never raise the floor of the basic experience.

## Performance budgets (non-negotiable, from tests/PERF.md)

- Idle memory under 50 MB.
- Bundle under 15 MB.
- Popover open under 100 ms.

Design choices that would blow these (heavy fonts, large images, runtime animation libraries, layout-thrashing transitions) are out of bounds. Vanilla CSS, system fonts, transform/opacity-only motion.
