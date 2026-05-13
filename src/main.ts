import * as Sentry from "@sentry/svelte";
import App from './App.svelte';
import NewFilesDetail from './components/NewFilesDetail.svelte';
import { mount } from 'svelte';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { beforeSend } from "./sentry-before-send";

Sentry.init({
  dsn: import.meta.env.VITE_SENTRY_DSN,
  initialScope: { tags: { repo: "hq-sync-web" } },
  release: `hq-sync-web@${__APP_VERSION__}`,
  beforeSend,
});

const windowLabel = getCurrentWindow().label;
const Component = windowLabel === 'new-files-detail' ? NewFilesDetail : App;

const app = mount(Component, { target: document.getElementById('app')! });

export default app;
