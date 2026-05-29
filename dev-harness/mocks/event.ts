// Mock of @tauri-apps/api/event — no backend events in the preview harness.
export type UnlistenFn = () => void;

export async function listen<T>(
  _event: string,
  _handler: (e: { payload: T }) => void
): Promise<UnlistenFn> {
  return () => {};
}

export async function emit(_event: string, _payload?: unknown): Promise<void> {}
