import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Skill } from "./types";

/**
 * Every skill folder, disabled ones included.
 *
 * One read for the whole page rather than one per card: the backend already
 * merges both roots and the state table in a single pass, so asking twice would
 * scan twice for the same answer. `reload` is what a switch or a file write calls
 * to bring the list back in step.
 */
export function useSkillList() {
  const [skills, setSkills] = useState<Skill[]>([]);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState("");

  const reload = useCallback(async () => {
    try {
      setSkills(await invoke<Skill[]>("skills_list"));
      setFailed("");
    } catch (reason) {
      setFailed(String(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  return { skills, loading, failed, reload };
}
