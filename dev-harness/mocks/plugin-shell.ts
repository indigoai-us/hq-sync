// Mock of @tauri-apps/plugin-shell for the preview harness.
export async function open(target: string): Promise<void> {
  console.debug('[harness] shell.open:', target);
}
