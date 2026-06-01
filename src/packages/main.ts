import { getCurrentWindow } from '@tauri-apps/api/window';
import { mount } from 'svelte';
import PackagesApp from './PackagesApp.svelte';

document.documentElement.dataset.window = getCurrentWindow().label;

const target = document.getElementById('packages');

if (!target) {
  throw new Error('Missing packages mount target');
}

const app = mount(PackagesApp, { target });

export default app;
