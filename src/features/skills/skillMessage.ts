import type { Skill } from "./types";

/**
 * How chosen skills are framed to the model.
 *
 * Built here rather than in Rust because the result is stored in the transcript
 * as an ordinary message. That is the point: once it is a message, every later
 * turn carries it inside the growing conversation, so the model re-reads the
 * instructions on each turn the way it re-reads anything else it was told -- and
 * because it sits inside the conversation, it is also inside the provider's
 * cached prefix, so it costs nothing to re-send.
 *
 * Injecting it per request instead would put it after the cache boundary, billed
 * again on every turn, and a turn that followed the selection would have no trace
 * of it at all.
 */

/** One skill's instructions, under a heading naming it. */
function block(skill: Skill): string {
  return `## ${skill.name}\n\n${skill.instructions}`;
}

/**
 * The instructions for a set of skills, as text.
 *
 * **One framing paragraph for all of them.** Two skills are not two authorities,
 * so what a skill *is* is stated once above the set rather than repeated above
 * each entry -- repeating it would read as each skill independently insisting on
 * its own importance.
 *
 * **The framing is what keeps a weak model honest.** Without it, instructions in
 * the same turn as a question get answered instead of obeyed. The paragraph names
 * them as the user's standing preferences and puts them ahead of the model's own
 * defaults, which is the whole reason a skill is delimited rather than pasted in
 * as if the user had typed it.
 */
export function buildSkillMessage(skills: Skill[]): string | null {
  if (skills.length === 0) return null;
  const blocks = skills.map(block).join("\n\n---\n\n");
  return (
    "The user has attached the following instructions for this reply. " +
    "They are the user's standing preferences, not part of the question below, and they take " +
    "priority over your own defaults. Where they conflict with what you would " +
    `otherwise do, follow them.\n\n${blocks}`
  );
}

/** The marker the message carries, so ids survive a save/load round trip. */
const SKILL_ID_PATTERN = /<!-- skill:([\w-]+) -->/g;

/**
 * The skills already in force in a conversation, by id.
 *
 * Read back out of the stored messages rather than kept in component state,
 * because the transcript is the record of what the model was told and a reopened
 * conversation has nothing else. This is what makes a skill survive for the rest
 * of the conversation, and it is why the composer can show it as already applied.
 */
export function activeSkillIds(messages: { content: string }[]): string[] {
  const ids = new Set<string>();
  for (const message of messages) {
    for (const match of message.content.matchAll(SKILL_ID_PATTERN)) ids.add(match[1]);
  }
  return [...ids];
}

/** The rule separating the instructions from what the user actually asked. */
const SEPARATOR = "\n\n---\n\n";

/**
 * Just what the user asked, without any skill instructions folded in.
 *
 * For drawing. The stored message is what the model was sent, so it has to keep
 * the instructions -- but a reader looking at the transcript should see the
 * question they asked, not the rules that were applied to it. The chips in the
 * composer already report which skills are in force, so drawing the bodies here
 * would repeat them in a form that reads as though the user had typed them.
 *
 * Falls back to the whole content when there is no separator, so an ordinary
 * message is never silently emptied by a marker this file failed to recognise.
 */
export function userTextOnly(content: string): string {
  const separator = content.lastIndexOf(SEPARATOR);
  return separator === -1 ? content : content.slice(separator + SEPARATOR.length);
}

/**
 * A user's message with the chosen skills' instructions folded into it.
 *
 * **Inlined rather than sent as its own `system` message.** llama.cpp's chat
 * templates reject a `system` message that is not the first one -- "System message
 * must be at the beginning" -- and other providers impose their own rules about
 * where a `system` role may appear. Putting the instructions inside the message
 * that uses them is accepted everywhere, and it also changes only the newest
 * message, so every earlier turn stays cached.
 *
 * **Delimited, not concatenated.** The instructions sit above a rule and the
 * question below it, with the framing paragraph naming them as preferences
 * rather than as part of the request. That is what stops a model reading the
 * rules as something the user asked about.
 *
 * **The marker rides along** so a later turn knows which skills are already in
 * force. It is an HTML comment, which the model sees as a comment and a reopened
 * conversation can read back as the only record of what the chat is running.
 */
export function withSkills(
  text: string,
  skills: Skill[],
): { content: string; appliedSkillIds: string[] } {
  if (skills.length === 0) return { content: text, appliedSkillIds: [] };
  const instructions = buildSkillMessage(skills);
  if (!instructions) return { content: text, appliedSkillIds: [] };
  const markers = skills.map((skill) => `<!-- skill:${skill.id} -->`).join("\n");
  return {
    content: `${markers}\n${instructions}${SEPARATOR}${text}`,
    appliedSkillIds: skills.map((skill) => skill.id),
  };
}
