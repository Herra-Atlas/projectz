import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Server } from "lucide-react";

type ProviderIconProps = { baseUrl: string; name: string; expanded: boolean; onToggle: () => void };

export default function ProviderIcon({ baseUrl, name, expanded, onToggle }: ProviderIconProps) {
  const [icon, setIcon] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let active = true;
    setIcon(null);
    setFailed(false);
    void invoke<string | null>("provider_favicon", { baseUrl }).then((dataUrl) => {
      if (active) setIcon(dataUrl);
    }).catch(() => {
      if (active) setFailed(true);
    });
    return () => { active = false; };
  }, [baseUrl]);

  return (
    <button type="button" onClick={onToggle} aria-expanded={expanded} aria-label={`${expanded ? "Collapse" : "Expand"} ${name}`} title={`${expanded ? "Collapse" : "Expand"} ${name}`} className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]">
      {icon && !failed ? <img src={icon} alt="" className="size-5 object-contain" onError={() => setFailed(true)} /> : <Server size={16} />}
    </button>
  );
}
