/**
 * Reading a stored message back for display.
 *
 * Skills used to be folded into the user's message by the composer: a marker,
 * the instructions, a rule, then what the user actually typed. That path is
 * gone, and so is the marker -- but this function used to strip on *any*
 * `\n\n---\n\n`, which is also an ordinary Markdown horizontal rule. A long
 * message that happened to contain one (a pasted report, a changelog, a spec)
 * lost everything above the last rule, and a message ending in one was drawn
 * empty.
 *
 * There is no marker left to test for, so the honest answer is to show the
 * stored text. The cost of being wrong the other way is a pre-migration
 * transcript showing its old instructions above the question -- visible
 * clutter, never missing words.
 */
export function userTextOnly(content: string): string {
  return content;
}
