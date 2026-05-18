import * as Sentry from "@sentry/svelte";
import App from './App.svelte';
import NewFilesDetail from './components/NewFilesDetail.svelte';
import MeetingsWindow from './components/MeetingsWindow.svelte';
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
// Tag the document so per-window `:global(html, body)` rules can scope
// themselves with `[data-window="…"]` and stop bleeding across windows.
// Without this, whichever component's CSS gets bundled last wins the
// body background — most visibly turning the transparent popover into
// a solid black rectangle when MeetingsWindow's #18181b body bg
// overrode App.svelte's `background: transparent`.
document.documentElement.dataset.window = windowLabel;

let Component: typeof App;
if (windowLabel === 'new-files-detail') {
  Component = NewFilesDetail as unknown as typeof App;
} else if (windowLabel === 'meetings-window') {
  Component = MeetingsWindow as unknown as typeof App;
} else {
  Component = App;
}

const app = mount(Component, { target: document.getElementById('app')! });

export default app;
