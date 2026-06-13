export function hqSkillMarkdownLink(skill: string, hqFolderPath?: string | null): string {
  const name = skill.trim();
  const root = (hqFolderPath ?? '').trim().replace(/\/+$/, '');
  const path = root ? `${root}/.claude/skills/${name}/SKILL.md` : `.claude/skills/${name}/SKILL.md`;
  return `[$${name}](${path})`;
}
