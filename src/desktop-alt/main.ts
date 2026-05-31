import { getCurrentWindow } from '@tauri-apps/api/window';
import { mount } from 'svelte';
import DesktopApp from './DesktopApp.svelte';

document.documentElement.dataset.window = getCurrentWindow().label;

const target = document.getElementById('desktop-alt');

if (!target) {
  throw new Error('Missing desktop-alt mount target');
}

const app = mount(DesktopApp, { target });

export default app;
