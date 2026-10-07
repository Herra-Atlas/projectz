/**
 * The mark for a run that is writing right now.
 *
 * A ring turned by a gradient tail, so it reads as motion rather than as a
 * static dotted circle. Shared between the sidebar's session block and the Sub
 * agents panel, so a run that is streaming looks the same wherever it is
 * watched -- the sidebar marks a conversation's background reply with it, and
 * the panel marks a sub-agent with it.
 *
 * `gradientId` names the SVG gradient, and has to be unique per instance: two
 * spinners with the same id share one definition, and a gradient is referenced
 * by id rather than by element.
 */
export default function LiveSpinner({ gradientId, size = 18 }: { gradientId: string; size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" className="animate-spin" aria-hidden="true">
      <defs>
        <linearGradient id={gradientId} x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" stopColor="currentColor" stopOpacity="1" />
          <stop offset="60%" stopColor="currentColor" stopOpacity="0.35" />
          <stop offset="100%" stopColor="currentColor" stopOpacity="0.05" />
        </linearGradient>
      </defs>
      <circle cx="12" cy="12" r="9" fill="none" stroke={`url(#${gradientId})`} strokeWidth="3" strokeLinecap="round" strokeDasharray="42 15" />
    </svg>
  );
}
