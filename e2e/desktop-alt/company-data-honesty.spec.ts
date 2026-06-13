import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

/**
 * Regression — the company work-system surfaces must show real PRD data or an
 * honest placeholder, never fabricated values.
 *
 * Context: Tasks/Projects/Goals invented assignees, completion times, start
 * weekdays, target dates, and "agent proposed N projects" from row index / a
 * string hash of the story or project id. prd.json carries no assignee/owner
 * field and no per-story/target dates — only Project.createdAt/updatedAt are
 * real — so any of those is a made-up value presented as fact. This locks the
 * fabrications out: derive from real fields (createdAt) or say "Unassigned"/"—".
 */
describe('company surfaces show real data, not fabrications', () => {
  const tasks = readRepoFile('src/desktop-alt/pages/CompanyTasksPage.svelte');
  const projects = readRepoFile('src/desktop-alt/pages/CompanyProjectsPage.svelte');
  const goals = readRepoFile('src/desktop-alt/pages/CompanyGoalsPage.svelte');

  it('Tasks does not invent assignees or completion times', () => {
    expect(tasks).not.toContain('projectIndex'); // index-mod assignee threading
    expect(tasks).not.toContain('h ago'); // fabricated "passed Nh ago"
    expect(tasks).not.toContain("return 'Agent'");
    expect(tasks).toContain("'Unassigned'");
    expect(tasks).toContain("row.story.passes ? 'Done'");
  });

  it('Projects derives "started" from the real createdAt, not a hashed weekday', () => {
    expect(projects).not.toContain('startedDay');
    expect(projects).not.toContain("'Wed'"); // weekday-hash table
    expect(projects).toContain('project.createdAt');
    expect(projects).toContain('formatProjectDate');
    expect(projects).not.toMatch(/Jun \$\{day\}/); // hashed target date
  });

  it('Goals does not claim a fabricated agent proposal count', () => {
    expect(goals).not.toContain('agent proposed');
  });
});
