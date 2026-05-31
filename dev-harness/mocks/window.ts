// Mock of @tauri-apps/api/window for the preview harness.
export function getCurrentWindow() {
  return {
    label: 'main',
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
