// Mock of @tauri-apps/api/event — a tiny in-browser pub/sub so the preview
// harness can drive event-driven components (e.g. BannerNotification, which
// renders only after a `banner:event`). Real Tauri delivers these from Rust;
// here the harness emits fixtures itself.
export type UnlistenFn = () => void;

type Handler = (e: { payload: unknown }) => void;
const handlers = new Map<string, Set<Handler>>();

export async function listen<T>(
  event: string,
  handler: (e: { payload: T }) => void
): Promise<UnlistenFn> {
  const set = handlers.get(event) ?? new Set<Handler>();
  set.add(handler as Handler);
  handlers.set(event, set);
  return () => {
    handlers.get(event)?.delete(handler as Handler);
  };
}

export async function emit(event: string, payload?: unknown): Promise<void> {
  handlers.get(event)?.forEach((h) => h({ payload }));
}
