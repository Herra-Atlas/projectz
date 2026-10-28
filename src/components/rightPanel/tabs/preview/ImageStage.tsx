/**
 * The box a picture is drawn in.
 *
 * Shared by the two image previews -- the one that reads bytes and the one that
 * draws an `.svg`'s text -- because both want the same thing: a rail-coloured
 * stage, centred, scrolling when the picture is larger than the panel, and never
 * cropped. Keeping it here means those rules cannot drift between the two.
 */
export default function ImageStage({ children }: { children: React.ReactNode }) {
  return (
    <div className="grid min-h-0 flex-1 place-items-center overflow-auto bg-[var(--rail)] p-4">
      {children}
    </div>
  );
}
