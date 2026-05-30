// Mock of @tauri-apps/api/app for the preview harness.
export async function getVersion(): Promise<string> {
  return '0.2.0-beta.4';
}

export async function getName(): Promise<string> {
  return 'HQ Sync';
}
