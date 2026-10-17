import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
// @ts-expect-error type error without @types/node package
import path from "node:path";
const host = process.env.TAURI_DEV_HOST;

/** Folders that are never part of the frontend and are never worth watching. */
const NOISY = new Set(["node_modules", ".git", "src-tauri", "target", "dist", ".vite"]);

/** Top-level files whose change should still reach the dev server. */
const ROOT_FILES = ["index.html", "package.json"];

/**
 * Watches only the files the frontend is actually built from.
 *
 * **Why this exists.** Vite watches the whole project, so *any* file written
 * under it reaches the dev server -- by the agent, by an MCP tool, or by the
 * workspace the agent is working in when that folder is the project itself. A
 * change Vite cannot hot-swap makes it reload the page, and a reload remounts the
 * app: the loading screen appears and every running reply is lost, including one
 * parked on a question that therefore has nobody left to answer it. That is the
 * "the app restarts" being seen, and it is not a crash.
 *
 * **The rule.** `src/` and `public/` and the root config files are watched, so an
 * edit to the app still hot-reloads exactly as before; a file anywhere else under
 * the project is not, so nothing the agent writes can pull the window out from
 * under a running reply. Directories are never ignored -- the walk has to pass
 * through them to reach `src` -- only non-frontend *files* are dropped.
 */
function watchOnlyFrontend() {
  const root = process.cwd();
  return (target: string, stats?: { isDirectory: () => boolean }) => {
    const relative = path.relative(root, target);
    // The project root itself, or something outside it: leave Vite's own rules be.
    if (!relative || relative.startsWith("..")) return false;
    const [top] = relative.split(/[\\/]/);
    if (NOISY.has(top)) return true;
    if (top === "src" || top === "public") return false;
    if (ROOT_FILES.includes(relative) || relative.startsWith("vite.config.") || relative.startsWith("tsconfig")) return false;
    // Never drop a folder, or the walk could not reach `src`. Any other file is
    // dropped, which is the whole point -- it cannot reload the app.
    return stats ? !stats.isDirectory() : true;
  };
}

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react(), tailwindcss()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. `src-tauri`, and anything the agent or its tools write, is not watched.
      ignored: watchOnlyFrontend(),
    },
  },
}));
