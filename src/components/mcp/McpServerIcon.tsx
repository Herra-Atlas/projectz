import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Plug } from "lucide-react";

type McpServerIconProps = {
  /** The server's command line, which the backend scans for a URL. */
  command: string;
  name: string;
  expanded: boolean;
  onToggle: () => void;
};

/**
 * A server's icon, fetched and cached exactly the way a provider's is.
 *
 * The backend owns the whole of it -- one cache directory, one fetch, one set of
 * rules for what counts as an image -- so this component only asks and draws. The
 * button is also the disclosure, as it is on the provider list: one control that
 * says what the row is and opens what is inside it, rather than two.
 *
 * A server that is a program has nothing to fetch, and that is most of them, so
 * the glyph is not a failure state -- it is what a command-line server looks
 * like. It has to be asked once per command line, because that is what the
 * address is derived from.
 */
export default function McpServerIcon({
  command,
  name,
  expanded,
  onToggle,
}: McpServerIconProps) {
  const [icon, setIcon] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let active = true;
    setIcon(null);
    setFailed(false);
    void invoke<string | null>("mcp_favicon", { command })
      .then((dataUrl) => {
        if (active) setIcon(dataUrl);
      })
      .catch(() => {
        if (active) setFailed(true);
      });
    return () => {
      active = false;
    };
  }, [command]);

  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={expanded}
      aria-label={`${expanded ? "Collapse" : "Expand"} ${name}`}
      title={`${expanded ? "Collapse" : "Expand"} ${name}`}
      className="grid size-9 shrink-0 place-items-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
    >
      {icon && !failed ? (
        <img src={icon} alt="" className="size-5 object-contain" onError={() => setFailed(true)} />
      ) : (
        <Plug size={16} />
      )}
    </button>
  );
}
