/**
 * The recognition half of the Local page.
 *
 * Empty on purpose rather than hidden: the category exists so the page does not
 * grow a fourth tab later without a thought, and a reader who wonders whether
 * the app transcribes audio gets an answer instead of a guess. Nothing here
 * pretends to work.
 */
export function SpeechRecognitionCategory() {
  return (
    <div className="px-4 py-7">
      <p className="text-sm text-[var(--muted)]">Speech recognition is in development.</p>
      <p className="mt-1 text-xs text-[var(--quiet)]">
        A Whisper model will install here once transcription is ready.
      </p>
    </div>
  );
}
