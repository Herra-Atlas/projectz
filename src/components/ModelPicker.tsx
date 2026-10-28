import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Server } from "lucide-react";
import Select from "./Select";

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
      <Select
        label="AI provider"
        icon={<Server size={15} />}
        value={selectedEndpoint}
        placeholder={endpoints.length ? "Choose provider" : "Add provider"}
        options={endpoints.filter((item) => item.enabled).map((item) => ({ value: item.id, label: item.name }))}
        onChange={(value) => {
          onEndpointChange(value);
          setModelInput("");
          onModelChange("");
        }}
        className="h-10 min-w-[130px] max-w-[175px]"
      />
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
