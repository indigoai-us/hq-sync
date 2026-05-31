---
id: hq-sync-story-tests-read-source-at-module-level
title: Story-contract tests read component source at module level — update them when renaming/deleting source
scope: repo
trigger: Renaming, moving, or deleting any Svelte component or Rust source file in hq-sync that a story-contract test (`__tests__/stories/US-*.test.ts`) references
enforcement: soft
version: 1
created: 2026-05-29
updated: 2026-05-29
public: false
source: task-completion
learned_from: session/2026-05-29-desktop-alt-meetings-multiday-agenda
---

## Rule

Before renaming, moving, or deleting a Svelte component or Rust source file in this repo, grep `__tests__/stories/` for `readFileSync` references to that path and update them in the SAME change.

```bash
grep -rn "readFileSync" __tests__/stories/ | grep -i "<the-file-or-component-name>"
```

The story-contract tests call `readFileSync` on source files at **module level** (top of the file, outside any `it()` / `describe()` body) so they can run normalized-string `.toContain()` assertions against the source text. A dangling `readFileSync(resolve(process.cwd(), 'src/.../Deleted.svelte'), 'utf8')` throws `ENOENT` at **test-collection time** — the entire test file errors out before a single assertion runs, and the failure reads as a confusing module-load crash rather than a clean assertion diff.

## Rationale

These tests trade resilience for a cheap "the source still wires X to Y" guarantee. The cost is that they are coupled to exact file paths and exact source substrings, both of which drift under normal refactors. The module-level read is the sharpest edge: it converts a missing file into a collection-phase crash that can mask which assertion actually regressed.

Hit during the desktop-alt Meetings multi-day-agenda swap: `MeetingsToday.svelte` was deleted and replaced with `MeetingsAgenda.svelte`, but `US-006.test.ts` still `readFileSync`-ed the old path at module level, so the whole US-006 file threw at collection until the read was repointed. (A second, separate red in the same file was a stale negative assertion — see "How to comply" step 3.)

## How to comply

1. When renaming/deleting source, grep `__tests__/stories/` for the old path/name first.
2. Repoint the `readFileSync(...)` const (and any `normalize(...)` derived from it) to the new path, or delete the assertion block if the contract no longer applies.
3. While editing, sanity-check that the test's `.toContain` / `.not.toContain` substrings still match reality — a `.not.toContain` that contradicts the live source (e.g. asserting the page never calls a typed invoke it legitimately uses) is a stale assertion to correct, not a contract to preserve. Never loosen a still-valid assertion to make a test pass.
4. Run `npx vitest run __tests__/stories/<file>.test.ts` and confirm it collects + passes.

## Exceptions

None. This is test hygiene, not a behavior gate — but skipping it turns a one-line path update into a misleading collection crash.
