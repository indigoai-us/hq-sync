import * as Sentry from "@sentry/svelte";
import App from './App.svelte';
import { mount } from 'svelte';
import { beforeSend } from "./sentry-before-send";

Sentry.init({
  dsn: import.meta.env.VITE_SENTRY_DSN,
  initialScope: { tags: { repo: "hq-sync-web" } },
  release: `hq-sync-web@${__APP_VERSION__}`,
  beforeSend,
});

const app = mount(App, { target: document.getElementById('app')! });

export default app;
