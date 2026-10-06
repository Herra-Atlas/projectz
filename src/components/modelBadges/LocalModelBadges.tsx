import { ImageIcon } from "lucide-react";
import type { LocalModel } from "../../features/models/types";
import { compactContext } from "./ModelBadges";

type LocalModelBadgesProps = {
  model: LocalModel;
  /** Whether this model has a projector configured, from `useLocalProjectors`. */
  hasProjector: boolean;
};

/**
 * What a local model is known to be, as small marks beside its name.
 *
 * Deliberately not `ModelBadges`: capabilities are what a remote provider
 * reports about a model, and a local model has none — its facts come from its own
 * GGUF file and its runtime settings. Forcing them through that shape would
 * fabricate a provider answer to render a file's own header, and the two would
 * drift the first time either changed.
 *
 * Same rule as the remote badges: only what is actually known is drawn. A model
 * whose file reported no quantisation or context gets no mark for it, because a
 * dimmed icon reads as "cannot" when it means "we have not asked".
 */
export default function LocalModelBadges({ model, hasProjector }: LocalModelBadgesProps) {
  const context = model.context_length;
  const quantization = model.quantization?.trim();

  const hasContext = typeof context === "number" && context > 0;
  if (!hasContext && !quantization && !hasProjector) return null;

  return (
    <span className="flex shrink-0 items-center gap-1.5">
      {hasProjector && (
        <span className="inline-flex items-center" title="A projector is configured, so this model can read images">
          <ImageIcon size={12} aria-hidden="true" className="text-[var(--quiet)]" />
          <span className="sr-only">Accepts images</span>
        </span>
      )}
      {quantization && (
        <span className="shrink-0 text-[10px] tabular-nums text-[var(--quiet)]" title="Weights quantisation">
          {quantization}
        </span>
      )}
      {hasContext && (
        <span
          className="shrink-0 text-[10px] tabular-nums text-[var(--quiet)]"
          title={`${context!.toLocaleString()} token context window declared by the model file`}
        >
          {compactContext(context!)}
        </span>
      )}
    </span>
  );
}