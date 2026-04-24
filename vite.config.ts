/// <reference types="vitest/config" />
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { sentryVitePlugin } from "@sentry/vite-plugin";
import pkg from "./package.json" with { type: "json" };

export default defineConfig({
  plugins: [
    svelte(),
    sentryVitePlugin({
      org: process.env.SENTRY_ORG ?? "indigo-d0",
      project: process.env.SENTRY_PROJECT ?? "hq-sync-web",
      authToken: process.env.SENTRY_AUTH_TOKEN,
      release: { name: `hq-sync-web@${pkg.version}` },
    }),
  ],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  clearScreen: false,
  server: {
    port: 1421,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "safari13",
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: process.env.TAURI_ENV_DEBUG ? true : "hidden",
  },
  test: {
    environment: "node",
    globals: true,
    include: ["src/**/*.test.ts"],
  },
});
