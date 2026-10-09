import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { chatRepository } from "./chatRepository";
import type { ChatMessage, ChatSession, ChatSessionHeader } from "./types";
import type { WorkingMode } from "./chatMode";

/**
 * Where the conversation to reopen on launch is remembered.
 *
 * Namespaced and versioned like the other window-local keys, and deliberately
 * not a SQLite setting: it is where the window was left, not a preference about
 * how the app behaves, and it is worthless on another machine.
 */
const LAST_SESSION_KEY = "projectz.last-session.v1";

/**
 * Conversations, split into a list and the transcripts that have been fetched.
 *
 * The list holds headers only -- title, ordering, the dot, the token rollup --
 * which is everything a sidebar draws. Transcripts are fetched one conversation
 * at a time as it is opened. Loading all of them at startup meant paying for
 * every message in the database, across the IPC bridge and into React state, to
 * render a list of titles; the cost grew with history rather than with the number
 * of conversations.
 *
 * **Two rules keep this honest.**
 *
 * A header-only write goes through `chatRepository.saveHeader`, which omits the
 * `messages` key so the backend leaves every row untouched. A rename must never
 * rewrite a transcript, and it no longer has a way to.
 *
 * A session that is *selected* but not yet *loaded* is a real state, and is not
 * an empty conversation. `sessionsLoaded` reports it so the chat page can say
 * "loading" instead of drawing an empty transcript the user could then send into.
 */
export function useChatSessions(enabled: boolean, reloadKey = 0, modelId = "", providerId = "") {
  const [sessions, setSessions] = useState<ChatSessionHeader[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState("");
  /**
   * Transcripts by session id. Only conversations that have been opened appear
   * here, so a hit means "this is the transcript as last seen" rather than "this
   * conversation is empty".
   */
  const [transcripts, setTranscripts] = useState<Record<string, ChatMessage[]>>({});
  /**
   * Which conversations have been fetched, so an empty array can be told apart
   * from a fetch that has not happened yet.
   */
  const [sessionsLoaded, setSessionsLoaded] = useState<Record<string, boolean>>({});
  const [messagesError, setMessagesError] = useState("");

  /**
   * The conversation to reopen on launch.
   *
   * Held separately from `sessions` because it is a *preference*, not data: it
   * names a conversation that may since have been deleted, so it is validated
   * against the loaded list rather than trusted. Local storage rather than a
   * SQLite setting because it is a window-local UI position, in the same way the
   * selected model is.
   */
  const [lastActiveId, setLastActiveId] = useState<string | null>(
    () => localStorage.getItem(LAST_SESSION_KEY),
  );

  /**
   * The current header list, readable from a callback without making that
   * callback depend on it.
   *
   * `updateMessages` has to persist a transcript *together with* its header,
   * because a `ChatSession` carries both. Reading the list inside a state
   * setter would close over the value from the render that created the setter
   * and miss a header written in the same tick -- so a save could resurrect a
   * stale title. A ref always reads the latest committed value.
   */
  const sessionsRef = useRef<ChatSessionHeader[]>([]);
  sessionsRef.current = sessions;
  // Read inside the load effect, which must not re-run every time the user
  // switches conversation -- it would re-read the whole list on every click.
  const lastActiveIdRef = useRef(lastActiveId);
  /**
   * Whether the list has ever loaded, and which conversation is open.
   *
   * Both are read by the load effect, which has to be able to tell a first read from
   * a later one. A refresh -- a scheduled job has written a transcript of its own --
   * must not blank the list or re-read the conversation the reader is looking at;
   * doing either is what made the panel appear to restart under them.
   */
  const loadedRef = useRef(false);
  const activeIdRef = useRef<string | null>(activeId);
  activeIdRef.current = activeId;
  lastActiveIdRef.current = lastActiveId;
  // Read inside `selectSession`, which must not be rebuilt whenever a transcript
  // finishes loading -- otherwise the sidebar's click handler changes identity on
  // every fetch.
  const sessionsLoadedRef = useRef(sessionsLoaded);
  sessionsLoadedRef.current = sessionsLoaded;

  /**
   * Fetch one conversation's transcript, once.
   *
   * Shared by opening a conversation and by restoring one at launch, so there is
   * a single place that decides what "loaded" means and a single place a read
   * failure is reported from. A second copy would be a second answer to "what
   * happens if this fetch fails", and the two would drift.
   *
   * Never throws. A failed read leaves the conversation open and *not* loaded,
   * which the chat page shows as a retry rather than as an empty transcript: an
   * empty transcript is something the user can send into, and treating a failed
   * read as one would lose the conversation's history from the conversation.
   */
  const fetchTranscript = useCallback(async (id: string): Promise<ChatMessage[] | null> => {
    try {
      const messages = await chatRepository.listMessages(id);
      setTranscripts((current) => ({ ...current, [id]: messages }));
      setSessionsLoaded((current) => ({ ...current, [id]: true }));
      setMessagesError("");
      // Returned as well as stored. The stored copies only reach the refs on the
      // next render, so a caller that read them straight after awaiting this
      // would be reading the value from *before* the fetch -- which is what made
      // the overview report a read failure the first time and work the second,
      // once the transcript was already in the cache.
      return messages;
    } catch (reason: unknown) {
      setMessagesError(String(reason));
      setSessionsLoaded((current) => ({ ...current, [id]: false }));
      return null;
    }
  }, []);

  useEffect(() => {
    if (!enabled) return;
    let mounted = true;
    // Only a first read empties the list. A later one is a background refresh, and
    // taking the panel away to put it back is the flicker that made refreshing feel
    // like a restart.
    if (!loadedRef.current) setLoaded(false);
    chatRepository.listHeaders().then((saved) => {
      if (!mounted) return;
      setSessions(sortSessions(saved));
      // Reopen where the user left off, but only if that conversation is still
      // here. The stored id is a hint, not a fact: the conversation may have been
      // deleted since, and opening an id that no longer exists would leave the
      // view stuck loading a transcript that is never coming.
      const remembered = lastActiveIdRef.current;
      const restore = remembered && saved.some((session) => session.id === remembered) ? remembered : null;
      // The transcript is fetched only when the open conversation actually changes:
      // a refresh is about the list, and re-reading the messages under the reader
      // would redraw what they are in the middle of reading.
      if (restore && restore !== activeIdRef.current) void fetchTranscript(restore);
      setActiveId(restore);
      loadedRef.current = true;
      setLoaded(true);
      setLoadError("");
    }).catch((reason: unknown) => {
      if (mounted) setLoadError(String(reason));
    });
    return () => {
      mounted = false;
    };
  }, [enabled, reloadKey, fetchTranscript]);

  /**
   * Remembers the open conversation.
   *
   * Written on every change rather than on unload, because a close can be a crash
   * and an id that only ever reaches storage at shutdown is one the app does not
   * have. The write is cheap and idempotent.
   */
  useEffect(() => {
    // Removed as well as written. Leaving a deleted conversation's id behind
    // would mean every launch tries to reopen something that is gone, and the
    // check against the loaded list would have to run each time to discard it.
    if (activeId) localStorage.setItem(LAST_SESSION_KEY, activeId);
    else localStorage.removeItem(LAST_SESSION_KEY);
  }, [activeId]);

  // Transcripts survive a header reload on purpose: the list is re-read when a
  // session is deleted or cleared elsewhere, and dropping every fetched
  // transcript would mean re-fetching the conversation the user is looking at.
  // A session removed from the list is pruned below instead.

  const activeSession = useMemo<ChatSession | null>(() => {
    const header = sessions.find((session) => session.id === activeId);
    if (!header) return null;
    // A transcript that has not been fetched yet reads as `null` rather than as
    // `[]`, so the chat page can tell "loading" from "genuinely empty" instead of
    // showing an empty conversation the user might send into.
    const messages = transcripts[activeId ?? ""];
    return messages ? { ...header, messages } : null;
  }, [sessions, transcripts, activeId]);

  /**
   * Open a conversation, fetching its transcript the first time.
   *
   * Resolves with the messages when they arrive, so a caller that needs the
   * transcript before acting -- the statistics page opening a conversation's
   * overview, for instance -- can wait for it rather than reading a header.
   */
  const selectSession = useCallback(async (id: string | null) => {
    setActiveId(id);
    if (!id) return;
    // Already fetched: re-reading it would replace a transcript the user may have
    // just added to, and the array in memory is the newer one.
    if (sessionsLoadedRef.current[id]) return;
    await fetchTranscript(id);
  }, [fetchTranscript]);

  /**
   * Whether the active conversation's transcript has arrived.
   *
   * False means the chat page should not offer the composer yet.
   */
  const activeLoaded = activeId ? sessionsLoaded[activeId] === true : false;

  /**
   * One conversation with its transcript, for a read-only view.
   *
   * The session overview opens from the sidebar and from the statistics page,
   * and it needs the messages to compute its figures. It does not open the
   * conversation, so this fetches on demand without touching `activeId` --
   * clicking "Overview" must not also switch the chat view. Resolves with `null`
   * when the conversation is gone or its transcript cannot be read, so a caller
   * shows nothing rather than an empty overview full of zeroes.
   */
  const loadSessionForOverview = useCallback(async (id: string): Promise<ChatSession | null> => {
    // The header is read first: a conversation that is gone has nothing to show
    // even if a stale transcript is still in memory, and returning `null` here
    // lets the caller say so rather than drawing a record for a deleted session.
    const header = sessionsRef.current.find((session) => session.id === id);
    if (!header) return null;
    // A transcript already in memory is not re-read: the figures are derived
    // from the same rows either way, and this is the common case once a
    // conversation has been opened.
    const cached = transcripts[id];
    if (cached && sessionsLoaded[id]) return { ...header, messages: cached };
    // The messages come back from the fetch itself rather than from the ref: the
    // stored copy is not readable until the next render, so reading it here was
    // a race the first time a conversation was opened.
    const messages = await fetchTranscript(id);
    return messages ? { ...header, messages } : null;
  }, [transcripts, sessionsLoaded, fetchTranscript]);

  /**
   * A conversation created by a first send, carrying the mode and access level it
   * was about to be worked in.
   *
   * Both matter at creation rather than only afterwards. The picker works without
   * a conversation open, so the level chosen there has nowhere to be stored yet --
   * and a conversation created without them reads as Chat, which silently drops an
   * Agent first send back into a plain question. The choice reaches the stored
   * header from the send that creates it, so there is never a window in which a
   * conversation exists but the mode it was created in does not.
   *
   * Undefined when nothing was chosen, which leaves the header without the keys
   * and therefore reads as Chat with the default level.
   */
  const createSessionWithMessages = useCallback((messages: ChatMessage[], workingMode?: WorkingMode) => {
    const now = new Date().toISOString();
    const firstPrompt = messages.find((message) => message.role === "user")?.content;
    const session: ChatSession = {
      id: crypto.randomUUID(),
      title: firstPrompt ? firstPrompt.slice(0, 48).trim() : "New conversation",
      messages,
      createdAt: now,
      updatedAt: now,
      modelId: modelId || undefined,
      providerId: providerId || undefined,
      mode: workingMode?.mode,
      permission: workingMode?.permission,
    };
    setSessions((current) => sortSessions([session, ...current]));
    // Marked loaded without a fetch: these messages are the transcript, and
    // saving them wrote every one of them.
    setTranscripts((current) => ({ ...current, [session.id]: messages }));
    setSessionsLoaded((current) => ({ ...current, [session.id]: true }));
    setActiveId(session.id);
    void chatRepository.save(session);
    return session.id;
  }, [modelId, providerId]);

  const updateMessages = useCallback((id: string, messages: ChatMessage[]) => {
    // The transcript is written unconditionally and always carries the whole
    // conversation. A header-only write cannot persist a reply, so this is the
    // one path that must not be skipped for a session whose transcript is not in
    // memory -- and sending fewer messages than are stored is a deliberate
    // truncation (a retried reply), which the backend still allows.
    void chatRepository.save({ ...sessionFor(sessionsRef.current, id), messages });
    setTranscripts((current) => ({ ...current, [id]: messages }));
    setSessionsLoaded((current) => ({ ...current, [id]: true }));
    setSessions((current) => {
      const session = current.find((item) => item.id === id);
      if (!session) return current;
      const firstPrompt = messages.find((message) => message.role === "user")?.content;
      return sortSessions([
        {
          ...session,
          // `renamed` locks a manual rename; a model-generated title is applied
          // through `setAutoTitle` and must not be reverted to the raw prompt.
          title: firstPrompt && !session.renamed ? firstPrompt.slice(0, 48).trim() : session.title,
          updatedAt: new Date().toISOString(),
          modelId: modelId || session.modelId,
          providerId: providerId || session.providerId,
        },
        ...current.filter((item) => item.id !== id),
      ]);
    });
  }, [modelId, providerId]);

  /** Applies a header-only change and persists it without touching messages. */
  const updateHeader = useCallback((id: string, change: Partial<ChatSessionHeader>) => {
    setSessions((current) => {
      const session = current.find((item) => item.id === id);
      if (!session) return current;
      const updated: ChatSessionHeader = { ...session, ...change, updatedAt: new Date().toISOString() };
      void chatRepository.saveHeader(updated);
      return sortSessions([updated, ...current.filter((item) => item.id !== id)]);
    });
  }, []);

  /**
   * Apply a model-generated title. Manual renames win, and a later
   * `updateMessages` will not overwrite the generated title because
   * `renamed` is set here too.
   *
   * `renamedByUser` is deliberately left untouched: the title was chosen by a
   * model, not the user, so the overview must not report it as a manual rename.
   */
  const setAutoTitle = useCallback((id: string, title: string) => {
    const generated = title.trim();
    if (!generated) return;
    setSessions((current) => {
      const session = current.find((item) => item.id === id);
      if (!session || session.renamed) return current;
      const updated: ChatSessionHeader = { ...session, title: generated, renamed: true, updatedAt: new Date().toISOString() };
      void chatRepository.saveHeader(updated);
      return sortSessions([updated, ...current.filter((item) => item.id !== id)]);
    });
  }, []);

  const renameSession = useCallback((id: string, title: string) => {
    const renamed = title.trim();
    if (!renamed) return;
    // `renamedByUser` also flips when the user retypes a name on a chat that
    // already had a generated title, since from here on the user owns it.
    updateHeader(id, { title: renamed, renamed: true, renamedByUser: true });
  }, [updateHeader]);

  const setSessionDotColor = useCallback((id: string, dotColor?: string) => {
    updateHeader(id, { dotColor });
  }, [updateHeader]);

  /**
   * Records the mode and access level a conversation is being worked in.
   *
   * A header write like the pin and the dot, so it costs no transcript. Lives
   * here rather than in the chat page because this hook owns the session list
   * and is the only place that can write a header without also touching the
   * transcript.
   */
  const setSessionWorkingMode = useCallback((id: string, workingMode: WorkingMode) => {
    updateHeader(id, workingMode);
  }, [updateHeader]);

  const togglePinned = useCallback((id: string) => {
    setSessions((current) => {
      const session = current.find((item) => item.id === id);
      if (!session) return current;
      const updated: ChatSessionHeader = { ...session, pinned: !session.pinned };
      void chatRepository.saveHeader(updated);
      return sortSessions([updated, ...current.filter((item) => item.id !== id)]);
    });
  }, []);

  const deleteSession = useCallback((id: string) => {
    setSessions((current) => current.filter((session) => session.id !== id));
    setActiveId((current) => current === id ? null : current);
    // Forgetting the stored id stops the next launch trying to reopen a
    // conversation that no longer exists.
    setLastActiveId((current) => (current === id ? null : current));
    // The transcript and its loaded flag go together, so a later conversation
    // that somehow reused the id would fetch rather than show the deleted one.
    setTranscripts((current) => {
      const { [id]: _removed, ...rest } = current;
      return rest;
    });
    setSessionsLoaded((current) => {
      const { [id]: _removed, ...rest } = current;
      return rest;
    });
    void chatRepository.remove(id);
  }, []);

  /** Drops every conversation locally first so the sidebar empties straight away, then
      reports what the database actually removed so the caller can tell the user. A
      failure re-reads the list, because the optimistic clear may not have persisted. */
  const clearSessions = useCallback(async () => {
    setSessions([]);
    setTranscripts({});
    setSessionsLoaded({});
    setActiveId(null);
    setLastActiveId(null);
    try {
      return await chatRepository.clear();
    } catch (error) {
      // Re-read headers rather than full sessions: the question is only which
      // conversations survived, and their transcripts can be fetched when opened.
      const remaining = await chatRepository.listHeaders().catch(() => [] as ChatSessionHeader[]);
      setSessions(sortSessions(remaining));
      throw error;
    }
  }, []);

  return {
    sessions,
    activeSession,
    activeId,
    activeLoaded,
    loaded,
    loadError,
    messagesError,
    createSessionWithMessages,
    selectSession,
    loadSessionForOverview,
    updateMessages,
    setAutoTitle,
    renameSession,
    setSessionDotColor,
    setSessionWorkingMode,
    togglePinned,
    deleteSession,
    clearSessions,
  };
}

/** Pinned first, then most recently changed. */
function sortSessions(sessions: ChatSessionHeader[]): ChatSessionHeader[] {
  return [...sessions].sort((a, b) => Number(Boolean(b.pinned)) - Number(Boolean(a.pinned)) || b.updatedAt.localeCompare(a.updatedAt));
}

/**
 * The header for a session, read from the list.
 *
 * `updateMessages` writes a transcript alongside its header, so it needs the
 * header fields to persist with it. Reading them from state inside the setter is
 * not safe -- a header written in the same tick would not be in the value the
 * setter closed over -- so the caller passes the current list instead and this
 * is the one place that knows how to join the two.
 */
function sessionFor(sessions: ChatSessionHeader[], id: string): ChatSessionHeader {
  const session = sessions.find((item) => item.id === id);
  return session ?? { id, title: "New conversation", createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() };
}