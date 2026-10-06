import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * An engine as the backend describes it.
 *
 * Only the fields a chooser reads. Declared here rather than imported from the
 * settings page because that page owns the *catalog* -- the thirty releases it
 * lists, the download buttons, the paging -- and none of that is any of this
 * hook's business. Importing the page's type would couple two unrelated views
 * and put a catalog field on a struct that never carries one.
 */
export type InstalledEngine = {
  id: string;
  version: string;
  installed: boolean;
  source: string;
};

/**
 * The engines actually on disk, for a chooser to offer.
 *
 * `refresh: false` throughout, which is what makes this cheap: it reads the
 * cached catalog file and only reaches the network when that cache is cold or
 * older than twelve hours. A chooser that opened on a fresh install would pay
 * one request, and every other open is served from disk -- which is what lets
 * this sit inside a page that renders a picker per model without becoming a
 * second catalog fetch each time a model is added.
 *
 * Never throws. A failure here is a chooser with no options, and the page above
 * it already reports a broken engine list; an exception would take the settings
 * dialog down over a control the user may not even have opened yet.
 */
export function useInstalledEngines(): InstalledEngine[] {
  const [engines, setEngines] = useState<InstalledEngine[]>([]);

  useEffect(() => {
    let mounted = true;
    void invoke<InstalledEngine[]>("local_engines_list", { refresh: false })
      .then((items) => {
        if (!mounted) return;
        // Only installed ones. A catalog row the user has not downloaded cannot
        // be chosen -- pinning a model to it would fail at load with a message
        // about a directory that does not exist, which is a worse way to learn
        // that than not being offered the option.
        setEngines(items.filter((engine) => engine.installed));
      })
      .catch(() => undefined);
    return () => { mounted = false; };
  }, []);

  return engines;
}
