/// <reference types="vitest" />
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte({ hot: !process.env.VITEST })],
  clearScreen: false,
  server: {
    port: 1421,
    strictPort: true,
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test-setup.ts'],
    // Svelte 5 runes need the browser build to run in tests. Inlining
    // svelte + testing-library tells Vitest to transform them rather
    // than resolving the SSR entry.
    server: {
      deps: {
        inline: [/^svelte/, '@testing-library/svelte'],
      },
    },
  },
  resolve: {
    // `browser` condition ensures testing-library's svelte5 rendering
    // path is picked when running under jsdom.
    conditions: process.env.VITEST ? ['browser'] : undefined,
  },
});
