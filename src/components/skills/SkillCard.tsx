import {
  BookOpen,
  ChevronRight,
  Code2,
  FileText,
  Package,
  PenLine,
  ShieldAlert,
  Sparkles,
} from "lucide-react";
import { Toggle } from "../SettingsSection";
import type { Skill } from "../../features/skills/types";

/**
 * One skill drawn as a card.
 *
 * **The body opens the editor; the switch and the card are separate targets.** A
 * button cannot contain a button, so the click-to-edit area and the enable switch
 * are siblings. The switch is in the footer, where a reader already expects
 * per-row state, and the whole rest of the card is the way in.
 *
 * **The icon is the family.** A skill's `type` is the only grouping it has, and a
 * small glyph that follows it lets the eye sort a wall of cards before reading a
 * single name. It is decorative; the type is also printed in the footer.
 *
 * **Enabled is carried by the surface, not by opacity.** An off skill sits on the
 * rail colour and its words step back; an on skill sits on the panel colour. An
 * opacity veil would dim the controls along with the text.
 */
export default function SkillCard({
  skill,
  onOpen,
  onToggle,
}: {
  skill: Skill;
  onOpen: (skill: Skill) => void;
  onToggle: (skill: Skill) => void;
}) {
  const on = skill.enabled;
  const Icon = iconFor(skill.skill_type);

  return (
    <article
      className={`group flex flex-col overflow-hidden rounded-xl border transition-colors duration-150 ${
        on
          ? "border-[var(--line)] bg-[var(--panel)] hover:border-[color-mix(in_srgb,var(--accent)_38%,var(--line))]"
          : "border-[var(--line)] bg-[var(--rail)] hover:border-[var(--line-strong)]"
      }`}
    >
      <button
        type="button"
        onClick={() => onOpen(skill)}
        aria-label={`Edit ${skill.name}`}
        className="flex flex-1 items-start gap-3 p-4 text-left focus-visible:outline-2 focus-visible:outline-offset-[-3px] focus-visible:outline-[var(--accent)]"
      >
        <span
          aria-hidden
          className={`mt-0.5 grid size-8 shrink-0 place-items-center rounded-lg border ${
            on
              ? "border-[color-mix(in_srgb,var(--accent)_22%,var(--line))] bg-[color-mix(in_srgb,var(--accent)_9%,var(--panel))] text-[var(--accent)]"
              : "border-[var(--line)] bg-[var(--raised)] text-[var(--quiet)]"
          }`}
        >
          <Icon size={15} />
        </span>

        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-2">
            <span
              className={`truncate text-[13.5px] font-medium tracking-tight ${
                on ? "text-[var(--text)]" : "text-[var(--muted)]"
              }`}
            >
              {skill.name}
            </span>
            {skill.origin === "generated" && (
              <span
                title="Drafted by the agent from a conversation"
                className="inline-flex shrink-0 items-center gap-1 rounded bg-[var(--raised)] px-1.5 py-0.5 text-[10px] text-[var(--quiet)]"
              >
                <Sparkles size={10} aria-hidden /> Agent
              </span>
            )}
            {skill.origin === "bundled" && (
              <span
                title="Shipped with the app"
                className="inline-flex shrink-0 items-center gap-1 rounded bg-[var(--raised)] px-1.5 py-0.5 text-[10px] text-[var(--quiet)]"
              >
                <Package size={10} aria-hidden /> Included
              </span>
            )}
          </span>

          {/* Two lines of description, clamped, so a wall of cards has a rhythm
              regardless of how much each skill chose to say. */}
          <span
            className={`mt-1 line-clamp-2 block min-h-9 text-xs leading-[18px] ${
              on ? "text-[var(--muted)]" : "text-[var(--quiet)]"
            }`}
          >
            {skill.description || "No description — write one so the agent knows when to read this."}
          </span>

          <span className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10.5px] text-[var(--quiet)]">
            {skill.file_count > 0 && (
              <span className="inline-flex items-center gap-1" title="Files beside SKILL.md">
                <FileText size={11} aria-hidden />
                {skill.file_count} file{skill.file_count === 1 ? "" : "s"}
              </span>
            )}
            {skill.use_count > 0 && <span>used {skill.use_count}×</span>}
            {skill.has_scripts && !skill.trusted && (
              <span
                className="inline-flex items-center gap-1 text-[color-mix(in_srgb,var(--danger)_78%,var(--quiet))]"
                title="This skill has scripts, which are not allowed to run yet"
              >
                <ShieldAlert size={11} aria-hidden /> Scripts blocked
              </span>
            )}
          </span>
        </span>

        <ChevronRight
          size={15}
          aria-hidden
          className="mt-1 shrink-0 text-[var(--quiet)] transition-transform duration-150 group-hover:translate-x-0.5"
        />
      </button>

      <div className="flex items-center justify-between gap-2 border-t border-[var(--line)] px-4 py-2">
        <span className="truncate text-[10.5px] uppercase tracking-wide text-[var(--quiet)]">
          {skill.skill_type?.trim() || "Unlabelled"}
        </span>
        <span className="flex shrink-0 items-center gap-2">
          <span className={`text-[11px] ${on ? "text-[var(--muted)]" : "text-[var(--quiet)]"}`}>
            {on ? "On" : "Off"}
          </span>
          <Toggle
            checked={on}
            onChange={() => onToggle(skill)}
            label={`${on ? "Disable" : "Enable"} ${skill.name}`}
          />
        </span>
      </div>
    </article>
  );
}

/**
 * The glyph a family gets. Unknown labels fall back to a spark, not to nothing:
 * an empty tile reads as a missing icon rather than as an unlabelled skill.
 */
function iconFor(type: string | null) {
  switch ((type ?? "").trim().toLowerCase()) {
    case "code":
    case "frontend":
    case "backend":
    case "rust":
    case "typescript":
      return Code2;
    case "writing":
    case "documentation":
    case "articles":
    case "messages":
      return PenLine;
    case "research":
    case "browser":
      return BookOpen;
    default:
      return Sparkles;
  }
}
