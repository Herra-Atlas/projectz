/**
 * A skill as the folder-backed model defines it.
 *
 * The content lives on disk in `<data>/skills/<slug>/`, so what travels here is
 * the header plus the app's own state: whether it is offered, whether its
 * scripts may run, and how often it has been read.
 */
export type Skill = {
  /** The folder name and the spec's `name`. Stable across a rename. */
  slug: string;
  /** The display label, from `metadata.title` or the slug. */
  name: string;
  description: string | null;
  /** Free-text family. `null` is unlabelled and is distinct from `""`. */
  skill_type: string | null;
  /** `user` for one written deliberately, `generated` for one the agent
   * drafted, `bundled` for one the app ships. They are trusted differently. */
  origin: "user" | "generated" | "bundled";
  enabled: boolean;
  /** Whether `scripts/` may be executed. Off until the user says otherwise. */
  trusted: boolean;
  use_count: number;
  /** Supporting files, excluding SKILL.md. */
  file_count: number;
  has_scripts: boolean;
  created_at: string;
  updated_at: string;
};

/** One supporting file beside `SKILL.md`. */
export type SkillFile = {
  /** Relative to the skill folder, always `/` separated. */
  path: string;
  size: number;
  is_script: boolean;
};

/** A skill with the parts a listing does not need. */
export type SkillDetail = {
  skill: Skill;
  instructions: string;
  files: SkillFile[];
};

/** The editable half of a skill. */
export type SkillDraft = {
  name: string;
  description: string;
  instructions: string;
  skillType: string;
};

/**
 * The same shape the Rust `SkillDraft` takes.
 *
 * `skill_type` and `description` are omitted as `null` when blank: the backend
 * treats a whitespace-only label as unlabelled, and sending nothing keeps the two
 * from having to agree on it.
 */
export const toSkillPayload = (draft: SkillDraft) => ({
  name: draft.name,
  description: draft.description.trim() || null,
  instructions: draft.instructions,
  skill_type: draft.skillType.trim() || null,
});

/** The empty draft a new skill starts from. */
export const blankSkillDraft = (): SkillDraft => ({
  name: "",
  description: "",
  instructions: "",
  skillType: "",
});

/** A byte count for a file row, in the unit a reader expects. */
export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
