import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { chatRepository } from "../chat/chatRepository";
import type { ChatMessage, ChatSessionHeader } from "../chat/types";
import type { AiEvent } from "../../pages/chat/types";
import { createLiveRun, foldEvent, runningSubAgentFromEvent, type LiveRun, type RunningSubAgent } from "./liveRun";

/**
 * One sub-agent run, as the panel lists it.
 *
 * A session header, not a special shape: a finished run is stored in the same
 * table as a conversation because it *is* one — a transcript the same panel
 * draws. Only `kind` and `parentSessionId` tell them apart.
 */
export type SubAgentRun = ChatSessionHeader;

/**
 * One row of the panel's list.
 *
 * Both a finished run (from storage) and a running one (from memory) become this,
 * so the list has one shape to render. `running` is what decides the row's mark:
 * a spinner while the agent works, the agent icon once it is done.
 */
export type SubAgentListItem = {
  id: string;
  title: string;
  updatedAt: string;
  running: boolean;
};

/**
 * The sub-agent runs, and the transcript of whichever is open.
 *
 * **Two sources, one list.** Finished runs come from storage, read per-run like
 * the chat sidebar: the list is headers only, and a run's messages are fetched
 * when it is opened rather than for every run up front. Running runs come from
 * the event stream and the backend's registry, because a run is not written to
 * storage until it finishes -- so a panel that read only the database would be
 * blind to an agent already at work.
 *
 * **Three things make it live.** Opening the panel refreshes the list and seeds
 * the runs already going; a `subagent` event adds a run the moment it starts and
 * drops it when it ends; and every other event on the stream is folded into the
 * matching run while it is running, so its transcript streams.
 */
export function useSubAgents(enabled: boolean, parentSessionId: string | null) {
  const [headers, setHeaders] = useState<SubAgentRun[]>([]);
  const [live, setLive] = useState<LiveRun[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [transcript, setTranscript] = useState<ChatMessage[] | null>(null);
  const [error, setError] = useState("");
  const [loaded, setLoaded] = useState(false);

  // The id whose transcript is being shown. Kept in a ref so the event listener,
  // which is subscribed once, reads the current value rather than a stale one.
  const activeIdRef = useRef<string | null>(null);
  activeIdRef.current = activeId;
  // The live runs, for the same reason: the listener folds events into them
  // without re-subscribing on every change.
  const liveRef = useRef<LiveRun[]>([]);
  liveRef.current = live;
  const parentRef = useRef(parentSessionId);
  parentRef.current = parentSessionId;

  const refresh = useCallback(async () => {
    try {
      const list = await invoke<SubAgentRun[]>("database_list_subagents", {
        // Camel-case because that is what Tauri matches the Rust `parent_session_id`
        // argument against on the way in.
        parentSessionId: parentSessionId ?? null,
      });
      setHeaders(list);
      setError("");
      // A run that was open and has since been deleted (or is no longer under the
      // current conversation) is dropped rather than left showing a transcript the
      // list no longer contains. A run still going is kept: it is in the list too.
      setActiveId((current) =>
        current && (list.some((run) => run.id === current) || liveRef.current.some((run) => run.id === current))
          ? current
          : null,
      );
    } catch (reason: unknown) {
      setError(String(reason));
    } finally {
      setLoaded(true);
    }
  }, [parentSessionId]);

  /**
   * Shows the runs already at work when the panel opens.
   *
   * A run is only written to storage when it finishes, so without this a panel
   * opened mid-run would show nothing until the agent was done -- the exact case
   * this panel exists for. Runs already known from events are kept as they are,
   * so a seed that lands after a start event does not reset activity in progress.
   */
  const seed = useCallback(async () => {
    try {
      const running = await invoke<RunningSubAgent[]>("ai_running_subagents");
      const mine = running.filter((run) => parentSessionId === null || run.sessionId === parentSessionId);
      setLive((current) => {
        const known = new Map(current.map((run) => [run.id, run]));
        return mine.map((run) => known.get(run.id) ?? createLiveRun(run));
      });
    } catch {
      // A failed seed costs nothing: events still bring runs in as they start.
    }
  }, [parentSessionId]);

  useEffect(() => {
    if (!enabled) return;
    void refresh();
    void seed();
  }, [enabled, refresh, seed]);

  /**
   * Opens one run.
   *
   * A running run is drawn from memory, so there is nothing to fetch; only a
   * finished run's messages are read. Never throws: a failed read leaves the run
   * open and *not* loaded, so the tab shows a retry rather than an empty
   * transcript that reads as a run that did nothing.
   */
  const select = useCallback(async (id: string) => {
    setActiveId(id);
    setTranscript(null);
    if (liveRef.current.some((run) => run.id === id)) {
      setError("");
      return;
    }
    try {
      setTranscript(await chatRepository.listMessages(id));
      setError("");
    } catch (reason: unknown) {
      setError(String(reason));
    }
  }, []);

  /**
   * Returns to the list, dropping the open run's transcript with it.
   *
   * Both are cleared together: leaving a loaded transcript behind would make the
   * next `select` flash the previous run's messages for a frame.
   */
  const close = useCallback(() => {
    setActiveId(null);
    setTranscript(null);
  }, []);

  useEffect(() => {
    if (!enabled) return;
    const pending = listen<AiEvent>("ai-event", (event) => {
      const payload = event.payload;
      if (payload.kind === "subagent") {
        const metrics = payload.metrics;
        const id = metrics?.agent_id;
        if (!id) return;
        const sessionId = metrics?.session_id ?? null;
        // A run delegated from another conversation is not this panel's business.
        if (parentRef.current !== null && sessionId !== parentRef.current) return;
        if (metrics?.state === "running") {
          const seeded = runningSubAgentFromEvent(metrics);
          if (!seeded) return;
          setLive((current) => (current.some((run) => run.id === seeded.id) ? current : [...current, createLiveRun(seeded)]));
        } else {
          // The run is written to storage just before this arrives, so the list is
          // re-read and the run moves from memory to storage. The ref is cleared
          // first so a re-`select` below loads the row rather than skipping it.
          const next = liveRef.current.filter((run) => run.id !== id);
          liveRef.current = next;
          setLive(next);
          void refresh();
          // A run that finished while open has a stored transcript now; re-read it
          // so the open view stops being the in-memory one.
          if (activeIdRef.current === id) void select(id);
        }
        return;
      }
      // Any other event belongs to a run's stream only if we are tracking that run
      // id. The parent run's own events fall through here and are ignored.
      const run = liveRef.current.find((entry) => entry.id === payload.run_id);
      if (!run) return;
      foldEvent(run, payload);
      // A fresh array identity, so the panel re-renders from the mutated run.
      setLive((current) => [...current]);
    });
    return () => { pending.then((unlisten) => unlisten()); };
  }, [enabled, refresh, select]);

  // Running runs first, then finished ones, each group newest first from its own
  // source. A running run carries its start time as `updatedAt`, which is what the
  // list row shows until the stored row replaces it.
  const runs = useMemo<SubAgentListItem[]>(() => {
    const running = live.map((run) => ({ id: run.id, title: run.title, updatedAt: run.startedAt, running: true }));
    const finished = headers.map((run) => ({ id: run.id, title: run.title, updatedAt: run.updatedAt, running: false }));
    return [...running, ...finished];
  }, [live, headers]);

  const activeLive = live.find((run) => run.id === activeId) ?? null;

  return { runs, activeLive, activeId, transcript, select, close, refresh, error, loaded };
}
