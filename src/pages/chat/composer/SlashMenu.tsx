import { ChevronRight } from "lucide-react";
import type { SlashCommand } from "./slashCommands";

/**
 * The menu that opens above the composer while a `/` command is being typed.
 *
 * Presentational: it draws the matches and reports the one chosen. Which match is
 * highlighted, and what choosing does, belong to the composer, which owns both
 * the draft and the state a command changes.
 *
 * **Mouse-down, not click.** The field keeps focus while the menu is up, and a
 * click that moved focus first would blur the field and close the menu before the
 * choice landed -- so the press is taken before focus can move.
 */
export default function SlashMenu({
  commands,
  activeIndex,
  onChoose,
}: {
  commands: SlashCommand[];
  /** Index drawn as highlighted; kept within range by the caller. */
  activeIndex: number;
  onChoose: (command: SlashCommand) => void;
}) {
  return (
    <div
      role="listbox"
      aria-label="Commands"
      className="absolute bottom-full left-0 z-50 mb-2 w-[min(340px,calc(100vw-48px))] rounded-xl border border-[var(--line)] bg-[var(--rail)] p-1.5 shadow-2xl"
    >
      {commands.map((command, index) => (
        <button
          key={command.name}
          type="button"
          role="option"
          aria-selected={index === activeIndex}
          onMouseDown={(event) => {
            event.preventDefault();
            onChoose(command);
          }}
          className={`flex w-full items-center gap-3 rounded-lg px-2.5 py-2 text-left focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${
            index === activeIndex ? "bg-[var(--raised)]" : "hover:bg-[var(--raised)]"
          }`}
        >
          <ChevronRight size={15} className="shrink-0 text-[var(--muted)]" />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-[13px] text-[var(--text)]">{command.label}</span>
            <span className="block truncate text-[11px] leading-4 text-[var(--quiet)]">
              /{command.name} — {command.hint}
            </span>
          </span>
        </button>
      ))}
    </div>
  );
}
