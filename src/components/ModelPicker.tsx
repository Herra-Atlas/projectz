import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronDown, Server } from "lucide-react";

type Endpoint = { id: string; name: string; models: string[]; enabled: boolean };

export default function ModelPicker({
  selectedEndpoint,
  selectedModel,
  onEndpointChange,
  onModelChange,
  refreshKey,
}: {
  selectedEndpoint: string;
  selectedModel: string;
  onEndpointChange: (id: string) => void;
  onModelChange: (model: string) => void;
  refreshKey: number;
}) {
  const [endpoints, setEndpoints] = useState<Endpoint[]>([]);
  const [modelInput, setModelInput] = useState(selectedModel || "");

  useEffect(() => {
    invoke<Endpoint[]>("ai_list_endpoints").then(setEndpoints).catch(() => setEndpoints([]));
  }, [refreshKey]);

  useEffect(() => setModelInput(selectedModel || ""), [selectedModel]);

  const endpoint = endpoints.find((item) => item.id === selectedEndpoint);

  return (
    <div className="flex min-w-0 items-center gap-2">
      <label className="relative flex h-10 min-w-0 items-center gap-2 rounded-lg border border-[var(--line)] bg-[var(--rail)] px-3 text-[var(--muted)] focus-within:border-[var(--accent)]">
        <Server size={15} className="shrink-0" />
        <span className="sr-only">AI provider</span>
        <select
          aria-label="AI provider"
          value={selectedEndpoint}
          onChange={(event) => {
            onEndpointChange(event.target.value);
            setModelInput("");
            onModelChange("");
          }}
          className="w-full min-w-[110px] max-w-[160px] appearance-none bg-transparent pr-5 text-[13px] text-[var(--text)] outline-none"
        >
          <option value="">{endpoints.length ? "Choose provider" : "Add provider"}</option>
          {endpoints.filter((item) => item.enabled).map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
        </select>
        <ChevronDown size={14} className="pointer-events-none absolute right-3" />
      </label>
      {endpoint && <input
        aria-label="Model name"
        className="h-10 w-36 rounded-lg border border-[var(--line)] bg-[var(--rail)] px-3 text-[13px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)] sm:w-44"
        placeholder="Model name"
        value={modelInput}
        onChange={(event) => {
          setModelInput(event.target.value);
          onModelChange(event.target.value);
        }}
        list="known-models"
      />}
      <datalist id="known-models">
        {endpoint?.models.map((item) => <option key={item} value={item} />)}
      </datalist>
    </div>
  );
}
