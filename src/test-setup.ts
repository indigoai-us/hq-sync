// Vitest setup — runs before every test file.
//
// Mocks `@tauri-apps/api/core`'s `invoke` so component tests can assert
// on backend calls without a live Tauri host. Tests that need custom
// per-call behaviour can override via `vi.mocked(invoke).mockImplementationOnce(...)`.
import { vi } from 'vitest';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => undefined),
}));
