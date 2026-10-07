/** A skill as `database/skills.rs` defines it, in the shape the wire delivers. */
export type Skill = {
  id: string;
  name: string;
  /** One line for the picker. Optional, because a skill whose name says enough
   * should not need one written. */
  description: string | null;
  instructions: string;
  /** Free-text label. `null` is unlabelled and is distinct from `""`. */
  skill_type: string | null;
  /** `user` for one written deliberately, `generated` for one the agent drafted
   * from a past conversation, `bundled` for one the app ships. They are trusted
   * differently. */
  origin: "user" | "generated" | "bundled";
  enabled: boolean;
  use_count: number;
  created_at: string;
  updated_at: string;
};

/** The editable half of a skill. The rest is the application's to maintain. */
export type SkillDraft = {
  name: string;
  description: string;
  instructions: string;
  skillType: string;
};

/**
 * The same shape `NewSkill` takes.
 *
 * `skill_type` is omitted rather than sent as `""` when blank: the backend treats
 * a whitespace-only label as unlabelled, and sending nothing keeps the two from
 * having to agree on it.
 */
export const toSkillPayload = (draft: SkillDraft) => ({
  name: draft.name,
  description: draft.description || null,
  instructions: draft.instructions,
  skill_type: draft.skillType || null,
});

/** The empty draft a new skill starts from. */
export const blankSkillDraft = (): SkillDraft => ({
  name: "",
  description: "",
  instructions: "",
  skillType: "",
});
