import { getCurrentWindow } from '@tauri-apps/api/window';
import { mount } from 'svelte';
// Self-hosted Geist (variable weight axis) so the big window renders the real
// face offline, not the silent system fallback. Sans carries body + headings;
// Mono is reserved for IDs, paths, and version strings. See DESIGN.md.
import '@fontsource-variable/geist/wght.css';
import '@fontsource-variable/geist-mono/wght.css';
import DesktopApp from './DesktopApp.svelte';

document.documentElement.dataset.window = getCurrentWindow().label;

const target = document.getElementById('desktop-alt');

if (!target) {
  throw new Error('Missing desktop-alt mount target');
}

const app = mount(DesktopApp, { target });

export default app;
