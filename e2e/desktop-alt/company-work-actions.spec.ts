import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

describe('desktop-alt company work actions are functional', () => {
  const projects = readRepoFile('src/desktop-alt/pages/CompanyProjectsPage.svelte');
  const tasks = readRepoFile('src/desktop-alt/pages/CompanyTasksPage.svelte');
  const deployments = readRepoFile('src/desktop-alt/panels/DeploymentsPanel.svelte');
  const secrets = readRepoFile('src/desktop-alt/panels/SecretsPanel.svelte');

  it('wires Projects filtering and unlinked-project Link handoff', () => {
    expect(projects).toContain("type ProjectFilter = 'all' | 'active' | 'needs-link'");
    expect(projects).toContain('function cycleFilter()');
    expect(projects).toContain('matchesProjectFilter(project, projectFilter)');
    expect(projects).toContain('onclick={cycleFilter}');
    expect(projects).toContain('onclick={() => void requestLinkProject(project)}');
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
