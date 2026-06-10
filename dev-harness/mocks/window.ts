// Mock of @tauri-apps/api/window for the preview harness.
// The window label is selectable via `?window=<label>` so design work can
// preview any window (e.g. `?window=messages`) — defaults to the popover.
const previewLabel =
  (typeof location !== 'undefined' &&
    new URLSearchParams(location.search).get('window')) ||
  'main';

export function getCurrentWindow() {
  return {
    label: previewLabel,
    async listen() {
      return () => {};
    },
    async onFocusChanged() {
      return () => {};
    },
    async show() {},
    async hide() {},
    async setFocus() {},
    async close() {},
    async isVisible() {
      return true;
    },
  };
}
