export const DEFAULT_SYNC_POLL_SECONDS = 600;

export const SYNC_POLL_INTERVAL_OPTIONS = [
  { value: 60, label: '1 min' },
  { value: 120, label: '2 min' },
  { value: 300, label: '5 min' },
  { value: 600, label: '10 min' },
] as const;

export function normalizeSyncPollSeconds(seconds: number | null | undefined): number {
  return typeof seconds === 'number' && Number.isFinite(seconds) && seconds > 0
    ? seconds
    : DEFAULT_SYNC_POLL_SECONDS;
}

export function humanizeSyncPollInterval(seconds: number | null | undefined): string {
  const normalized = normalizeSyncPollSeconds(seconds);
  if (normalized % 60 === 0) {
    const minutes = normalized / 60;
    return `every ${minutes} ${minutes === 1 ? 'minute' : 'minutes'}`;
  }
  return `every ${normalized} ${normalized === 1 ? 'second' : 'seconds'}`;
}
