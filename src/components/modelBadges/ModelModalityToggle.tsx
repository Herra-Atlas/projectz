import { useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Brain, Ear, ImageIcon, Wrench } from "lucide-react";

import { compactContext } from "./ModelBadges";
import type { ModelCapabilities } from "../../features/models/modelCapabilities";

/**
 * What a model can do, with the inputs the user can correct.
 *
 * The provider's answer is not always right or present: a gateway that lists no
 * modalities leaves the composer refusing an image the model takes. The eye and
 * the ear are therefore buttons -- click one and the app treats that model as
 * able to read that kind of input from then on. The rest are read-only marks,
 * because "this model reasons" is not something a click can make true.
 *
 * Written through `ai_model_modality_set`, which stores the assertion and
 * returns the merged capabilities, so the row and the composer cannot disagree
 * about what was just claimed.
 */
export function ModelModalityToggle({
  endpointId,
  model,
  capabilities,
  onChange,
  disabled,
}: {
  endpointId: string;
  model: string;
  capabilities?: ModelCapabilities;
  onChange: (capabilities: ModelCapabilities) => void;
  disabled?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const inputs = capabilities?.input_modalities ?? [];
  const images = inputs.includes("image");
  const audio = inputs.includes("audio");

  const flip = (modality: "image" | "audio", on: boolean) => {
    if (busy) return;
    setBusy(true);
    void invoke<ModelCapabilities>("ai_model_modality_set", {
      endpointId,
      model,
      modality,
      on,
    })
      .then(onChange)
      .catch(() => undefined)
      .finally(() => setBusy(false));
  };

  return (
    <span className="flex shrink-0 items-center gap-1">
      <Mark
        on={images}
        label={images ? "Treat as able to read images" : "Treat as able to read images"}
        title={
          images
            ? "Reads images — click to stop treating it that way"
            : "Click if this model can read images"
        }
        disabled={disabled || busy}
        onClick={() => flip("image", !images)}
      >
        <ImageIcon size={12} />
      </Mark>
      <Mark
        on={audio}
        label="Treat as able to read audio"
        title={
          audio
            ? "Reads audio — click to stop treating it that way"
            : "Click if this model can read audio"
        }
        disabled={disabled || busy}
        onClick={() => flip("audio", !audio)}
      >
        <Ear size={12} />
      </Mark>
      {capabilities?.supports_reasoning === true && (
        <span className="inline-flex items-center text-[var(--quiet)]" title="Reports reasoning support">
          <Brain size={12} aria-hidden="true" />
        </span>
      )}
      {capabilities?.supports_tools === true && (
        <span className="inline-flex items-center text-[var(--quiet)]" title="Supports tool calls">
          <Wrench size={12} aria-hidden="true" />
        </span>
      )}
      {typeof capabilities?.context_length === "number" && capabilities.context_length > 0 && (
        <span className="shrink-0 text-[10px] tabular-nums text-[var(--quiet)]" title="Declared context window">
          {compactContext(capabilities.context_length)}
        </span>
      )}
    </span>
  );
}

/** One clickable modality mark. Filled when asserted, hollow when not. */
function Mark({
  on,
  label,
  title,
  disabled,
  onClick,
  children,
}: {
  on: boolean;
  label: string;
  title: string;
  disabled?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-pressed={on}
      aria-label={label}
      title={title}
      disabled={disabled}
      onClick={onClick}
      className={`grid size-6 place-items-center rounded transition-colors focus-visible:outline-2 focus-visible:outline-[var(--accent)] disabled:opacity-40 ${
        on
          ? "text-[var(--accent)] hover:bg-[var(--raised)]"
          : "text-[var(--quiet)] opacity-60 hover:bg-[var(--raised)] hover:text-[var(--text)] hover:opacity-100"
      }`}
    >
      {children}
    </button>
  );
}
