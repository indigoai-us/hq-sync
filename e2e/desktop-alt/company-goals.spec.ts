import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

describe('desktop-alt company Goals view source contract (US-006)', () => {
  const page = readRepoFile('src/desktop-alt/pages/CompanyGoalsPage.svelte');
  const companyPage = readRepoFile('src/desktop-alt/pages/CompanyPage.svelte');
  const adapter = readRepoFile('src/desktop-alt/lib/local-projects.ts');

  it('wires the dedicated Goals section to get_local_company_goals', () => {
    expect(companyPage).toContain("import CompanyGoalsPage from './CompanyGoalsPage.svelte'");
    expect(companyPage).toContain("{:else if tab === 'goals'}");
    expect(companyPage).toContain('<CompanyGoalsPage slug={company.slug} />');
    expect(page).toContain('loadCompanyGoals(activeSlug)');
    expect(adapter).toContain("invoke<CompanyGoalsWire>('get_local_company_goals'");
  });

  it('renders KR rows with current-to-target values and progress bars', () => {
    expect(page).toContain('data-testid="kr-table"');
    expect(page).toContain('data-testid="kr-row"');
    expect(page).toContain('Current → target');
    expect(page).toContain('{formatValue(kr.current, kr.unit)} → {formatValue(kr.target, kr.unit)}');
    expect(page).toContain('class="progress-track"');
    expect(page).toContain('style={`width: ${progress}%`}');
    expect(page).toContain('aria-label={`${progress}% progress`}');
  });

  it('renders owner, target quarter, linked projects, and at-risk surfacing', () => {
    expect(page).toContain('owner: {ownerLabel(objective.owner)}');
    expect(page).toContain('target {quarterLabel(objective.timeframe)');
    expect(page).toContain('data-testid="linked-projects"');
    expect(page).toContain('data-testid="linked-project-chip"');
    expect(page).toContain('data-testid="at-risk-note"');
    expect(page).toContain('Review proposal');
    expect(page).toContain('rgba(254, 188, 46, 0.3)');
  });

  it('linked project chips drill into the existing project detail view', () => {
    expect(page).toContain("import ProjectDetailView from './ProjectDetailView.svelte'");
    expect(page).toContain("import StoryPanel from '../v4/StoryPanel.svelte'");
    expect(page).toContain('onclick={() => openProject(project)}');
    expect(page).toContain('loadLocalProjectStories(project.prdPath)');
    expect(page).toContain('<ProjectDetailView');
    expect(page).toContain('<StoryPanel');
  });

  it('has the empty state for companies with no local goals file', () => {
    expect(page).toContain('data-testid="empty-goals-state"');
    expect(page).toContain('No goals yet');
  });
});
