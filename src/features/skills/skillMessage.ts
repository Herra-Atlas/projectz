/**
 * Reading a stored message back for display.
 *
 * Skills used to be folded into the user's message by the composer: a marker,
 * the instructions, a rule, then what the user actually typed. That path is gone
 * -- a skill is now a folder the model reads with `skill_read`, and the body
 * arrives as a tool result rather than as part of the question -- but the
 * messages it produced are still in every existing transcript.
 *
 * This file is therefore history rather than machinery: one function that finds
 * the typed question inside a message that may still carry the old prefix. It
 * stays because the alternative is a reopened conversation drawing a wall of
 * instructions above every question, which is exactly what the old separator
 * existed to prevent.
 */

/** The rule the old inline form put between the instructions and the question. */
const SEPARATOR = "\n\n---\n\n";

/**
 * Just what the user asked, without any inline skill instructions.
 *
 * Falls back to the whole content when there is no separator, so an ordinary
 * message is never silently emptied by a marker this file failed to recognise.
 */
export function userTextOnly(content: string): string {
  const separator = content.lastIndexOf(SEPARATOR);
  return separator === -1 ? content : content.slice(separator + SEPARATOR.length);
}
