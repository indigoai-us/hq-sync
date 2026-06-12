import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

describe('desktop-alt company work actions are functional', () => {
  const projects = readRepoFile('src/desktop-alt/pages/CompanyProjectsPage.svelte');
  const tasks = readRepoFile('src/desktop-alt/pages/CompanyTasksPage.svelte');
  const companyPage = readRepoFile('src/desktop-alt/pages/CompanyPage.svelte');
  const storyPanel = readRepoFile('src/desktop-alt/v4/StoryPanel.svelte');
  const companyBoardStore = readRepoFile('src/desktop-alt/lib/company-board.svelte.ts');
  const messages = readRepoFile('src/components/messaging/MessagesShell.svelte');
  const deployments = readRepoFile('src/desktop-alt/panels/DeploymentsPanel.svelte');
  const secrets = readRepoFile('src/desktop-alt/panels/SecretsPanel.svelte');

  it('wires Projects filtering and unlinked-project Link handoff', () => {
    expect(projects).toContain("type ProjectFilter = 'all' | 'active' | 'needs-link'");
    expect(projects).toContain('function cycleFilter()');
    expect(projects).toContain('matchesProjectFilter(project, projectFilter)');
    expect(projects).toContain('onclick={cycleFilter}');
    expect(projects).toContain('void requestLinkProject(project);');
    expect(projects).toContain("invoke('open_claude_code_link', { url })");
    expect(projects).toContain('data-testid="filtered-projects-empty-state"');
  });

  it('wires Tasks filtering instead of leaving the Filter button inert', () => {
    expect(tasks).toContain("type TaskFilter = 'all' | 'open' | 'mine' | 'p1'");
    expect(tasks).toContain('const filteredRows = $derived');
    expect(tasks).toContain('matchesTaskFilter(row, taskFilter)');
    expect(tasks).toContain('function cycleFilter()');
    expect(tasks).toContain('onclick={cycleFilter}');
    expect(tasks).toContain('data-testid="filtered-tasks-empty-state"');
  });

  it('wires Tasks rows into the V4 story detail panel', () => {
    expect(tasks).toContain("import StoryPanel from '../v4/StoryPanel.svelte'");
    expect(tasks).toContain('const selectedTask = $derived');
    expect(tasks).toContain('function openTask(row: TaskRow): void');
    expect(tasks).toContain('function openTaskFromKey(event: KeyboardEvent, row: TaskRow): void');
    expect(tasks).toContain('class="task-row"');
    expect(tasks).toContain('type="button"');
    expect(tasks).toContain('onclick={() => openTask(row)}');
    expect(tasks).toContain('<StoryPanel');
    expect(tasks).toContain('story={selectedTask?.story ?? null}');
    expect(tasks).toContain('onselectDependency={selectStoryById}');
    expect(tasks).toContain('{onStoryPassesChange}');
  });

  it('keeps Projects and Tasks responsive inside the narrow V4 shell panes', () => {
    expect(projects).toContain('container: company-projects / inline-size');
    expect(projects).toContain('@container company-projects');
    expect(projects).toContain('grid-template-columns: minmax(0, 1fr)');
    expect(tasks).toContain('container: company-tasks / inline-size');
    expect(tasks).toContain('@container company-tasks');
    expect(tasks).toContain('grid-template-columns: 38px minmax(0, 1fr)');
  });

  it('keeps story details legible over the desktop shell', () => {
    expect(storyPanel).toContain('background: var(--v4-raised)');
    expect(storyPanel).toContain('z-index: 100');
    expect(storyPanel).toContain('@media (max-width: 520px)');
    expect(storyPanel).toContain('width: 100vw');
    expect(companyPage).not.toContain('will-change: opacity, transform');
  });

  it('treats null local bridge payloads as empty data', () => {
    expect(companyBoardStore).toContain('function shapeBoard(raw: CompanyBoard | null | undefined)');
    expect(companyBoardStore).toContain('raw?.inbox ?? []');
    expect(messages).toContain("invoke<ChannelsResponse | null>('list_channels')");
    expect(messages).toContain('channels = resp?.channels ?? []');
  });

  it('wires Deployments find and deploy workflow controls', () => {
    expect(deployments).toContain('bind:value={deploymentQuery}');
    expect(deployments).toContain('matchesDeploymentQuery(deployment, deploymentQuery)');
    expect(deployments).toContain("onclick={() => void openDeployWorkflow()}");
    expect(deployments).toContain('buildClaudeCodeUrl({ folder: config.hqFolderPath ?? \'\', prompt })');
    expect(deployments).toContain("invoke('open_claude_code_link', { url })");
    expect(deployments).toContain('data-testid="filtered-deployments-empty-state"');
    expect(deployments).not.toContain('title="Deploy from terminal: /deploy"');
  });

  it('wires Secrets export and new-key controls to the HQ secrets workflow', () => {
    expect(secrets).toContain("onclick={() => void openSecretsPrompt('export')}");
    expect(secrets).toContain("onclick={() => void openSecretsPrompt('new')}");
    expect(secrets).toContain('/hq-secrets ${slug}');
    expect(secrets).toContain('buildClaudeCodeUrl({ folder: config.hqFolderPath ?? \'\', prompt })');
    expect(secrets).toContain("invoke('open_claude_code_link', { url })");
    expect(secrets).not.toContain('Export not available');
    expect(secrets).not.toContain('Create from CLI');
  });
});
