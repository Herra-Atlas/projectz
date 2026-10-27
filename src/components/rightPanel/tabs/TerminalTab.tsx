import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Plus, Send, Trash2 } from "lucide-react";
import type { TerminalSession } from "../../../features/rightPanel/usePanelViews";

const MAX_LINES = 2_000;

type LocalLogEvent = { modelId?: string; line?: string };
type TerminalTabProps = {
  sessions: TerminalSession[];
  activeId: string | null;
  workspace: string | null;
  loadedModelId: string | null;
  loadingModelId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
  onClose: (id: string) => void;
};

export default function TerminalTab({ sessions, activeId, workspace, loadedModelId, loadingModelId, onSelect, onNew, onClose }: TerminalTabProps) {
  const [logs, setLogs] = useState<Record<string, string[]>>({});
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const scrollRef = useRef<HTMLDivElement>(null);
  const serverId = loadedModelId ?? loadingModelId;
  const active = sessions.find((session) => session.id === activeId);

  useEffect(() => {
    let activeListener = true;
    const unlisten = listen<LocalLogEvent>("local-model-log", ({ payload }) => {
      if (!activeListener || !payload.modelId || !payload.line) return;
      setLogs((current) => {
        const lines = [...(current[payload.modelId!] ?? []), payload.line!].slice(-MAX_LINES);
        return { ...current, [payload.modelId!]: lines };
      });
    });
    return () => {
      activeListener = false;
      void unlisten.then((stop) => stop());
    };
  }, []);

  useEffect(() => {
    if (!serverId) return;
    let mounted = true;
    // Each engine answers its own command; llama keeps its own. The listener
    // above is shared, so lines land on one map keyed by the id the session
    // carries -- and the two audio engines are separate processes, so the
    // session's id is what says which one's buffer to read.
    const audio = serverId === "audiocpp-wake" ? "wake" : serverId === "audiocpp" ? "speech" : null;
    // The ONNX wake engine has no process, so its lines come from the app's own
    // buffer rather than from a child's stdout.
    const request: Promise<string[]> =
      serverId === "openwakeword"
        ? invoke<string[]>("wake_logs")
        : audio
          ? invoke<string[]>("audio_logs", { purpose: audio })
          : invoke<string[]>("local_model_logs");
    void request.then((lines) => {
      if (mounted) setLogs((current) => ({ ...current, [serverId]: lines.slice(-MAX_LINES) }));
    }).catch(() => undefined);
    return () => { mounted = false; };
  }, [serverId]);

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [activeId, logs]);

  const runCommand = async () => {
    if (!active || active.kind === "server" || !workspace) return;
    const command = drafts[active.id]?.trim();
    if (!command) return;
    setDrafts((current) => ({ ...current, [active.id]: "" }));
    setLogs((current) => ({
      ...current,
      [active.id]: [...(current[active.id] ?? []), `PS> ${command}`].slice(-MAX_LINES),
    }));
    try {
      const output = await invoke<string>("panel_terminal_run", { command, workspace });
      setLogs((current) => ({ ...current, [active.id]: [...(current[active.id] ?? []), output].slice(-MAX_LINES) }));
    } catch (error) {
      setLogs((current) => ({ ...current, [active.id]: [...(current[active.id] ?? []), String(error)].slice(-MAX_LINES) }));
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex min-h-10 shrink-0 items-center gap-1 overflow-x-auto border-b border-[var(--line)] px-2">
        {sessions.map((session) => (
          <div key={session.id} className={`group flex shrink-0 items-center gap-1 rounded px-2 py-1 text-xs ${session.id === activeId ? "bg-[var(--raised)] text-[var(--text)]" : "text-[var(--muted)]"}`}>
            <button type="button" onClick={() => onSelect(session.id)} className="truncate" title={session.title}>{session.title}</button>
            {/* A server row closes for real: its close stops the process first
                (in the panel, which owns the stop), so the × is never disabled.
                Closing a dead row is just removing its last output. */}
            <button
              type="button"
              onClick={() => onClose(session.id)}
              aria-label={`Close ${session.title}`}
              title={session.kind === "server" ? `Stop ${session.title} and close` : `Close ${session.title}`}
              className="rounded p-0.5 text-[var(--quiet)] opacity-0 hover:bg-[var(--line)] hover:text-[var(--text)] group-hover:opacity-100"
            >×</button>
          </div>
        ))}
        <button type="button" onClick={onNew} aria-label="New terminal" title="New terminal" className="grid size-6 shrink-0 place-items-center rounded text-[var(--muted)] hover:bg-[var(--raised)] hover:text-[var(--text)]"><Plus size={14} /></button>
      </div>
      {active?.kind === "server" ? (
        <div ref={scrollRef} className="min-h-0 flex-1 overflow-auto bg-[var(--page)] p-3 font-mono text-[11px] leading-5 text-[var(--text)]">
          {(logs[active.id] ?? []).map((line, index) => <div key={`${index}-${line}`} className="whitespace-pre-wrap break-all">{line}</div>)}
          {!(logs[active.id]?.length) && <p className="text-[var(--quiet)]">{active.id === "audiocpp" ? "Waiting for the audio engine…" : "Waiting for llama-server output…"}</p>}
        </div>
      ) : active ? (
        <>
          <div ref={scrollRef} className="min-h-0 flex-1 overflow-auto bg-[var(--page)] p-3 font-mono text-[11px] leading-5 text-[var(--text)]">
            {(logs[active.id] ?? []).map((line, index) => <div key={`${index}-${line}`} className="whitespace-pre-wrap break-all">{line}</div>)}
          </div>
          <form onSubmit={(event) => { event.preventDefault(); void runCommand(); }} className="flex shrink-0 items-center gap-2 border-t border-[var(--line)] p-2">
            <span className="font-mono text-[11px] text-[var(--accent)]">PS&gt;</span>
            <input
              value={drafts[active.id] ?? ""}
              onChange={(event) => setDrafts((current) => ({ ...current, [active.id]: event.target.value }))}
              disabled={!workspace}
              placeholder={workspace ? "Enter PowerShell command" : "Open a workspace to use the terminal"}
              className="min-w-0 flex-1 bg-transparent font-mono text-xs text-[var(--text)] outline-none placeholder:text-[var(--quiet)]"
            />
            <button type="submit" disabled={!workspace || !drafts[active.id]?.trim()} aria-label="Run command" className="grid size-7 place-items-center rounded text-[var(--muted)] hover:bg-[var(--raised)] disabled:opacity-40"><Send size={14} /></button>
            <button type="button" onClick={() => setLogs((current) => ({ ...current, [active.id]: [] }))} aria-label="Clear terminal" title="Clear terminal" className="grid size-7 place-items-center rounded text-[var(--quiet)] hover:bg-[var(--raised)] hover:text-[var(--text)]"><Trash2 size={13} /></button>
          </form>
        </>
      ) : (
        <div className="flex flex-1 flex-col items-center justify-center gap-2 text-sm text-[var(--muted)]">
          <p>No terminal open</p>
          <button type="button" onClick={onNew} className="rounded px-2 py-1 text-xs text-[var(--accent)] hover:bg-[var(--raised)]">Open a terminal</button>
        </div>
      )}
    </div>
  );
}
