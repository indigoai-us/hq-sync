// Noop mock of @sentry/svelte for the preview harness.
export function init(): void {}
export function captureException(): void {}
export function captureMessage(): void {}
export function setUser(): void {}
export function setTag(): void {}
export function withScope(): void {}
export const browserTracingIntegration = () => ({});
export const replayIntegration = () => ({});
