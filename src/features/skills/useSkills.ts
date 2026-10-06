import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Skill } from "./types";

/**
 * The enabled skills, loaded once the composer picker first opens.
 *
 * `skills_enabled`, not `skills_list`: the management screen needs the disabled
 * ones too, because "where did that skill go" is a question the disabled answer
 * is meant to settle. The composer cannot apply one, so offering it there would
 * be a row that silently does nothing.
 *
 * **Loaded lazily rather than at mount.** A picker that has never been opened has
 * no reason to cost a database read, and skills change rarely -- reading on open
 * also picks up anything written in Settings since the last launch, which is the
 * one moment staleness would actually be noticed.
 */
export function useSkills(active: boolean) {
  const [skills, setSkills] = useState<Skill[]>([]);
  const [loading, setLoading] = useState(false);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      setSkills(await invoke<Skill[]>("skills_enabled"));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (active) void reload();
  }, [active, reload]);

  return { skills, loading, reload };
}
