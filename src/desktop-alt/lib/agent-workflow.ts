import { invoke } from '@tauri-apps/api/core';
import { buildClaudeCodeUrl } from '../../lib/claude-code-link';

export interface AgentWorkflowResult {
  /** True when the Claude Code deep-link was dispatched; false when we fell
   *  back to copying the prompt (or couldn't even do that). */
  ok: boolean;
  /** Plain-language status the caller can surface in a toast. */
  message: string;
}

/**
 * Hand a prepared prompt to the Claude Code agent — opens a new session cwd'd
 * into the user's HQ folder via the dedicated `open_claude_code_link` command
 * (NOT plugin-shell open(), which would have to be widened to the world).
 *
 * If the deep-link can't be dispatched (Claude Code not installed, link
 * rejected), the prompt is copied to the clipboard so the affordance is never
 * a dead end. The returned message is written for a toast, so every hq-*
 * desktop action ("Deploy a result", "Share a file", "Run a worker", the
 * Deployments Deploy button) routes the SAME get_config → buildClaudeCodeUrl →
 * open_claude_code_link → clipboard-fallback sequence through one place.
 *
 * `label` (e.g. "deploy workflow") tunes the success/fallback copy so the toast
 * reads naturally per action.
 */
export async function openAgentWorkflow(
  prompt: string,
  label = 'workflow',
): Promise<AgentWorkflowResult> {
  try {
    const config = await invoke<{ hqFolderPath?: string }>('get_config').catch(() => ({
      hqFolderPath: '',
    }));
    const url = buildClaudeCodeUrl({ folder: config.hqFolderPath ?? '', prompt });
    await invoke('open_claude_code_link', { url });
    return { ok: true, message: `Opened the ${label} in Claude Code.` };
  } catch (err) {
    console.error('openAgentWorkflow: open_claude_code_link failed:', err);
    try {
      await navigator.clipboard.writeText(prompt);
      return { ok: false, message: `Prompt copied — paste it into Claude Code to start the ${label}.` };
    } catch {
      return { ok: false, message: 'Could not open Claude Code or copy the prompt.' };
    }
  }
}
