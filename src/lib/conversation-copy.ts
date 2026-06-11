// Pure clipboard-text resolver for conversation bubbles. A message can offer
// two copy actions: the whole message body, and (when present) the attached
// agent prompt delivered via `hq dm --prompt`. Keeping the selection logic here
// — trimmed, with empty → null so the caller no-ops — lets it be unit-tested
// without a DOM (the Svelte component just calls navigator.clipboard with the
// result).

export type CopyKind = 'body' | 'prompt';

export interface CopyableMessage {
  body: string;
  prompt?: string | null;
}

/**
 * Resolve the text to put on the clipboard for a given copy action.
 * Returns null when there's nothing meaningful to copy (empty/whitespace, or a
 * prompt copy on a message that carries no prompt) so the caller can skip the
 * clipboard write and the "Copied!" feedback.
 */
export function copyableText(msg: CopyableMessage, kind: CopyKind): string | null {
  const raw = kind === 'prompt' ? msg.prompt : msg.body;
  const trimmed = raw?.trim();
  return trimmed ? trimmed : null;
}
