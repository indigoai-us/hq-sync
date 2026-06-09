import { getCurrentWindow } from '@tauri-apps/api/window';
import { mount } from 'svelte';
// Self-hosted variable faces so the big window renders the real type offline,
// not a silent system fallback. The redesigned monochrome liquid-glass surface
// uses Inter for UI/body, Inter Tight for display headings, and Geist Mono for
// data — IDs, paths, counts, versions. See DESIGN.md → "Big-window type".
import '@fontsource-variable/inter/wght.css';
import '@fontsource-variable/inter-tight/wght.css';
import '@fontsource-variable/geist-mono/wght.css';
import DesktopApp from './DesktopApp.svelte';

document.documentElement.dataset.window = getCurrentWindow().label;

const target = document.getElementById('desktop-alt');

if (!target) {
  throw new Error('Missing desktop-alt mount target');
}

const app = mount(DesktopApp, { target });

export default app;
