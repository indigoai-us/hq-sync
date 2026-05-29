/// Browser preview harness config — renders Settings/Popover with mocked
/// Tauri APIs so design work can iterate in a normal browser. NOT used by the
/// Tauri build (that uses vite.config.ts). Run: npm run dev:preview
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { resolve } from 'node:path';

const mock = (f: string) => resolve(__dirname, 'dev-harness/mocks', f);

export default defineConfig({
  plugins: [svelte()],
  define: {
    __APP_VERSION__: JSON.stringify('0.0.0-preview'),
  },
  resolve: {
    alias: {
      '@tauri-apps/api/core': mock('core.ts'),
      '@tauri-apps/api/event': mock('event.ts'),
      '@tauri-apps/api/window': mock('window.ts'),
      '@tauri-apps/api/app': mock('app.ts'),
      '@tauri-apps/plugin-shell': mock('plugin-shell.ts'),
      '@sentry/svelte': mock('sentry.ts'),
    },
  },
  server: {
    port: 1422,
    strictPort: true,
  },
});
