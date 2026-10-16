import { memo } from "react";
import { splitWorkspacePaths } from "../../features/workspace/workspacePaths";

type PathTextProps = {
  /** The user's own message, as stored. */
  text: string;
  /** The open workspace root, or `null`/absent when nothing is open. */
  root: string | null | undefined;
};

/**
 * A user's message with any workspace path drawn in its short form.
 *
 * The full path is what the message stores and what the model was given, so the
 * tooltip hands back the real thing rather than a re-derivation: hovering answers
 * "which folder was that?" without the transcript spelling the whole path out on
 * every line.
 *
 * Memoised on its two props for the same reason `Markdown` is: the reply that is
 * streaming re-renders the chat once per frame, and a long transcript should not
 * re-scan every earlier message each time.
 */
export const PathText = memo(function PathText({ text, root }: PathTextProps) {
  const segments = splitWorkspacePaths(text, root);
  return (
    <>
      {segments.map((segment, index) =>
        segment.path ? (
          <span key={index} className="text-[var(--accent)]" title={segment.path}>
            {segment.text}
          </span>
        ) : (
          <span key={index}>{segment.text}</span>
        ),
      )}
    </>
  );
});
