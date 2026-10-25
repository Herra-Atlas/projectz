/**
 * A skill as the folder-backed model defines it.
 *
 * The content lives on disk in `<data>/skills/<slug>/`, and the folder *is* the
 * record: its name is the label, the `description` in `SKILL.md` is the trigger,
 * and the folders beside it are what the row's glyphs report. Nothing here is
 * stored anywhere else -- there is no state table to fall out of step with the
 * disk, and no enabled flag, because a skill that exists is offered.
 */
export type Skill = {
  /** The folder name and the spec's `name`. Stable across a rename. */
  slug: string;
  /** The label as drawn, made from the folder name. */
  name: string;
  /** The trigger. `null` when the header carries none. */
  description: string | null;
  /** Supporting files, excluding SKILL.md. */
  file_count: number;
  has_scripts: boolean;
  has_references: boolean;
  has_assets: boolean;
};

/** One supporting file beside `SKILL.md`. */
export type SkillFile = {
  /** Relative to the skill folder, always `/` separated. */
  path: string;
  size: number;
  is_script: boolean;
};

/** What `skills_get` returns: the skill, its body, and what sits beside it. */
export type SkillDetail = [Skill, string, SkillFile[]];

/**
 * The same shape the Rust `SkillDraft` takes.
 *
 * `description` is omitted as `null` when blank: the backend treats a
 * whitespace-only trigger as none, and sending nothing keeps the two from having
 * to agree on it.
 */
export type SkillDraft = {
  name: string;
  description: string | null;
  instructions: string;
};

/** The header template a new skill starts from. */
export const blankSkillMarkdown = (name: string): string =>
  `---\nname: ${name}\ndescription: ""\n---\n\n`;
