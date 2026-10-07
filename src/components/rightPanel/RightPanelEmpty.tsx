import { Bot, FileText, Globe2, Terminal } from "lucide-react";
import type { PanelViewId } from "../../features/rightPanel/usePanelViews";

/**
 * The panel's empty state: the views it can hold, before any are open.
 *
 * **Rows rather than a picker.** These are not choices between destinations, the
 * way the model switcher is a choice between models -- each one opens a *view*
 * alongside the chat, and several can be open at once. So they are offered as a
 * list of things to open rather than as tabs to replace each other with.
 *
 * **No shortcuts yet.** A shortcut that is printed next to a row has to work, and
 * none of these are bound. They get added with the handlers, so the label is
 * never a promise the app breaks.
 */
const VIEWS: { id: PanelViewId; label: string; description: string; icon: React.ReactNode }[] = [
  {
    id: "files",
    label: "Files",
    description: "Browse and open project files",
    // Lucide directly rather than `ToolIcon`: that component maps the *model's*
    // tool names to 12px muted marks for the activity panel, and reusing it here
    // would make these rows wear the scale of a different feature.
    icon: <FileText size={16} className="shrink-0 text-[var(--muted)]" />,
  },
  {
    id: "terminal",
    label: "Terminal",
    description: "Shells in this project",
    icon: <Terminal size={16} className="shrink-0 text-[var(--muted)]" />,
  },
  {
    id: "browser",
    label: "Browser",
    description: "Preview your app or any page",
    icon: <Globe2 size={16} className="shrink-0 text-[var(--muted)]" />,
  },
  {
    id: "subagents",
    label: "Sub agents",
    description: "Watch the agents this chat spawned",
    icon: <Bot size={16} className="shrink-0 text-[var(--muted)]" />,
  },
];

type RightPanelEmptyProps = {
  onOpen: (view: PanelViewId) => void;
  /**
   * Whether a workspace folder is open.
   *
   * The Files view has nothing to show without one, so it is offered greyed out
   * rather than opening an empty pane that reads as broken. Terminal and Browser
   * do not depend on a folder, so they stay live.
   */
  hasWorkspace: boolean;
};

const AVAILABLE: PanelViewId[] = ["files", "browser", "terminal", "subagents"];

export default function RightPanelEmpty({ onOpen, hasWorkspace }: RightPanelEmptyProps) {
  return (
    <div className="flex h-full flex-col justify-center px-5 py-8">
      <h3 className="text-sm font-medium text-[var(--text)]">Nothing open</h3>
      <p className="mt-1 text-[13px] text-[var(--muted)]">Pick a view, or press its shortcut.</p>

      <ul className="mt-5 flex flex-col">
        {VIEWS.map((view) => {
          // Files needs a folder to browse; without one it is unavailable, like a
          // view that has not been built yet. `aria-disabled` keeps it in the tab
          // order and still hoverable, where `disabled` would read as broken.
          const unavailable = !AVAILABLE.includes(view.id) || (view.id === "files" && !hasWorkspace);
          return (
            <li key={view.id}>
              <button
                type="button"
                aria-disabled={unavailable}
                title={unavailable && view.id === "files" && !hasWorkspace ? "Open a folder first" : undefined}
                onClick={() => onOpen(view.id)}
                className="flex w-full items-center gap-3 rounded-md px-2 py-2.5 text-left transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--accent)] aria-disabled:pointer-events-none aria-disabled:opacity-50"
              >
                {/* `w-5` so every row's label starts at the same x whatever the
                    icon, rather than the text stepping in and out with the glyph. */}
                <span className="grid w-5 shrink-0 place-items-center">{view.icon}</span>
                <span className="w-[5.5rem] shrink-0 text-[13px] text-[var(--text)]">{view.label}</span>
                <span className="min-w-0 flex-1 truncate text-[12px] text-[var(--accent)]">{view.description}</span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}