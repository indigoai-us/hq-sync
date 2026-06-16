import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  // The real component-mount E2E (mission-control.test.ts) imports
  // `*.svelte` files and renders them, so the desktop-alt suite needs the
  // Svelte compiler in its pipeline. The source-contract specs don't import
  // components, so this is a no-op for them. `hot: false` keeps the compiler in
  // a test-friendly (no HMR) mode under vitest.
  plugins: [svelte({ hot: false })],
  // Resolve Svelte's *client* build (not the server/SSR build) so `mount()` is
  // available — without the `browser` condition vitest picks svelte's
  // index-server export and component mounting throws. Pairs with the
  // per-spec happy-dom environment.
  resolve: {
    conditions: ['browser'],
  },
  test: {
    // Per-spec `// @vitest-environment` pragmas override this default — the
    // source-contract specs stay in node, while the real component-mount E2E
    // (mission-control.test.ts) opts into happy-dom to render the page.
    environment: 'node',
    globals: true,
    include: ['e2e/desktop-alt/**/*.spec.ts', 'e2e/desktop-alt/**/*.test.ts'],
    passWithNoTests: false,
    reporters: ['default'],
  },
});
