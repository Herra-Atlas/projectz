import { Brain, ImageIcon, Type } from "lucide-react";
import type { ModelCapabilities } from "../../features/models/modelCapabilities";

/** Formats a context window for a badge: `1.05M`, `200K`, `32K`.
 *
 * Kept here so the closed picker button and the open list agree, rather than
 * repeating the suffix logic at each call site. */
export const compactContext = (tokens: number): string =>
  tokens >= 1_000_000
    ? `${(tokens / 1_000_000).toFixed(2).replace(/\.?0+$/, "")}M`
    : tokens >= 1_000
      ? `${Math.round(tokens / 1_000)}K`
      : `${tokens}`;

type ModelBadgesProps = {
  capabilities: ModelCapabilities | null | undefined;
  /** `compact` drops the icons and shows only the context figure, for dense rows. */
  compact?: boolean;
};

/** What a model is known to support, as small marks beside its name.
 *
 * Only what the provider actually reported is shown. A model that said nothing
 * renders no badges at all rather than a set of greyed-out guesses, because a
 * dimmed icon reads as "cannot" when it actually means "we have not asked".
 */
export default function ModelBadges({ capabilities, compact = false }: ModelBadgesProps) {
  const context = capabilities?.context_length;
  const inputs = capabilities?.input_modalities;
  const reasoning = capabilities?.supports_reasoning === true;
  // A provider that listed its modalities is trusted to mean them; one that did
  // not is not assumed either way, so no badge is drawn.
  const vision = Array.isArray(inputs) && inputs.includes("image");
  const audio = Array.isArray(inputs) && inputs.includes("audio");

  if (!capabilities) return null;

  const marks = (
    <>
      {!compact && reasoning && (
        <span className="inline-flex items-center gap-1" title="Supports a reasoning level">
          <Brain size={12} aria-hidden="true" className="text-[var(--quiet)]" />
          <span className="sr-only">Supports reasoning</span>
        </span>
      )}
      {!compact && vision && (
        <span className="inline-flex items-center" title="Can read images">
          <ImageIcon size={12} aria-hidden="true" className="text-[var(--quiet)]" />
          <span className="sr-only">Accepts images</span>
        </span>
      )}
      {!compact && audio && (
        <span className="inline-flex items-center" title="Can read audio">
          <Type size={12} aria-hidden="true" className="text-[var(--quiet)]" />
          <span className="sr-only">Accepts audio</span>
        </span>
      )}
      {typeof context === "number" && context > 0 && (
        <span className="shrink-0 text-[10px] tabular-nums text-[var(--quiet)]" title={`${context.toLocaleString()} token context window`}>
          {compactContext(context)}
        </span>
      )}
    </>
  );

  const shown = reasoning || vision || audio || typeof context === "number";
  if (!shown) return null;

  return <span className="flex shrink-0 items-center gap-1.5">{marks}</span>;
}