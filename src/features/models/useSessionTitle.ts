import { useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ModelSelection } from "./types";

/** Cap on characters of the user's message sent to the title model.
 *
 * A title is at most six words, so the opening message only needs enough of it
 * to know what the conversation is about. A long pasted log or document told
 * the model everything it needed in the first few hundred characters; sending
 * the rest was latency for input that could not change the answer.
 */
const PROMPT_LIMIT = 500;

type UseSessionTitleOptions = {
  /** Title models in priority order; empty switches the feature off. */
  selections: ModelSelection[];
  /** Applies the generated title; should no-op on a manually renamed chat. */
  onTitle: (sessionId: string, title: string) => void;
  /** Reports a failure to the user without interrupting the chat. */
  onError: (message: string) => void;
};

/**
 * Names a conversation from the user's opening message only.
 *
 * Fired at send time rather than after the reply, so the title request runs
 * alongside the chat instead of waiting for it, and the prompt stays small. The
 * backend walks the configured models in order, so a local model whose engine
 * is not running falls through to the next entry rather than failing.
 *
 * Session ids are remembered so a conversation is only ever titled once, even
 * if the user keeps chatting in it.
 */
export function useSessionTitle({ selections, onTitle, onError }: UseSessionTitleOptions) {
  const titledSessionIdsRef = useRef(new Set<string>());
  const onTitleRef = useRef(onTitle);
  onTitleRef.current = onTitle;
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  useEffect(() => {
    // A newly configured model should be able to name conversations started
    // after the change, so previously handled sessions are forgotten.
    titledSessionIdsRef.current.clear();
  }, [selections]);

  return useCallback((sessionId: string, userMessage: string) => {
    if (selections.length === 0 || titledSessionIdsRef.current.has(sessionId)) return;
    const prompt = userMessage.trim().slice(0, PROMPT_LIMIT);
    if (!prompt) return;
    // Mark before awaiting so a fast second send cannot fire twice.
    titledSessionIdsRef.current.add(sessionId);
    void invoke<string>("ai_generate_title", { selections, prompt })
      .then((title) => {
        if (title.trim()) onTitleRef.current(sessionId, title);
      })
      .catch((reason: unknown) => onErrorRef.current(String(reason)));
  }, [selections]);
}
