import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Skill } from "./types";

/**
 * Every skill folder, newest read wins.
 *
 * One read for the whole page rather than one per row: the backend scans a single
 * directory, so asking twice would scan twice for the same answer. `reload` is
 * what a create, a delete and an install call to bring the list back in step --
 * and also what the page calls when the panel closes, because a skill's files are
 * edited outside this hook by design.
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

/**
 * Creates a skill folder from a name, and returns the slug it was given.
 *
 * The backend derives the slug (lowercase, hyphens, de-duplicated), so the folder
 * the caller should open is the one it answers with rather than one guessed on
 * this side. An empty body is deliberate: a new skill is a folder with a header
 * and nothing else, and the Files panel is what fills it in.
 */
export async function createSkill(name: string): Promise<Skill> {
  return invoke<Skill>("skills_create", {
    skill: { name, description: null, instructions: "" },
  });
}
