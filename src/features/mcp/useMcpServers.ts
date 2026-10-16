import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { McpServer, McpServerView } from "./types";

/**
 * The configured MCP servers, and the three things that can be done to them.
 *
 * **Reading is cheap; refreshing is not.** The mount read is the config plus
 * whatever each server said last time, so the page paints immediately. Only
 * `refresh` -- and saving, which refreshes the server it saved -- starts a
 * process, because a settings window should open at the speed of a settings
 * window rather than at the speed of the slowest program the user configured.
 *
 * Every command answers with the whole list rather than a delta, so one setter
 * here keeps the rows, the tool lists and the errors from drifting apart.
 */
export function useMcpServers() {
  const [servers, setServers] = useState<McpServerView[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    invoke<McpServerView[]>("mcp_servers")
      .then((list) => {
        if (alive) setServers(list);
      })
      // A failed read leaves the list empty, which the page draws as "none yet".
      // Reaching the servers is what surfaces a real failure, on the row it
      // belongs to rather than as one message about all of them.
      .catch(() => undefined)
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, []);

  /** Runs one command, which always answers with the whole list. */
  const run = useCallback(async (command: string, args: Record<string, unknown>) => {
    setBusy(true);
    try {
      setServers(await invoke<McpServerView[]>(command, args));
    } finally {
      setBusy(false);
    }
  }, []);

  const refresh = useCallback(() => run("mcp_refresh", {}), [run]);
  const save = useCallback(
    (server: McpServer) => run("mcp_save", { server }),
    [run],
  );
  const remove = useCallback((id: string) => run("mcp_delete", { id }), [run]);

  return { servers, loading, busy, refresh, save, remove };
}
