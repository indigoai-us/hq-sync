import { describe, expect, it } from 'vitest';
import { copyableText } from './conversation-copy';

describe('copyableText', () => {
  it('returns the trimmed message body for a body copy', () => {
    expect(copyableText({ body: '  hello team  ' }, 'body')).toBe('hello team');
  });

  it('returns the trimmed agent prompt for a prompt copy', () => {
    expect(
      copyableText({ body: 'see prompt', prompt: '  /run audit  ' }, 'prompt'),
    ).toBe('/run audit');
  });

  it('returns null for a prompt copy when the message has no prompt', () => {
    expect(copyableText({ body: 'just a message' }, 'prompt')).toBeNull();
    expect(copyableText({ body: 'just a message', prompt: null }, 'prompt')).toBeNull();
  });

  it('returns null when the requested text is empty or whitespace', () => {
    expect(copyableText({ body: '   ' }, 'body')).toBeNull();
    expect(copyableText({ body: 'x', prompt: '   ' }, 'prompt')).toBeNull();
  });

  it('copies body and prompt independently from the same message', () => {
    const msg = { body: 'kick off the run', prompt: '/execute-task US-001' };
    expect(copyableText(msg, 'body')).toBe('kick off the run');
    expect(copyableText(msg, 'prompt')).toBe('/execute-task US-001');
  });
});
