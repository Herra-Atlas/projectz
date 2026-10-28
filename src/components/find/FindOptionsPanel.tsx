import { CaseSensitive, Type, WholeWord } from "lucide-react";
import type { ReactNode } from "react";
import type { FindOptions } from "../../features/find/options";

type FindOptionsPanelProps = {
  open: boolean;
  options: FindOptions;
  onChange: (next: FindOptions) => void;
};

/**
 * The three switches under the field, behind the sliders button.
 *
 * **Closed by default, and animated open.** The common search is a word and
 * Enter; three rows of options under every search would be a form where the
 * field should be the whole thing. The panel expands by animating
 * `grid-template-rows` from `0fr` to `1fr` (`App.css`), which is the one way to
 * transition to a content height without measuring it.
 *
 * Each row is one `role="switch"` button rather than a label with a control
 * beside it: the whole row is the target, pressing Space toggles it, and there is
 * no nested button to fight over the click.
 */
export default function FindOptionsPanel({ open, options, onChange }: FindOptionsPanelProps) {
  return (
    <div className="pz-find-options" data-open={open} inert={!open} aria-hidden={!open}>
      <div>
        <div className="flex flex-col gap-0.5 border-t border-[var(--line)] px-2 py-2">
          <OptionRow
            icon={<CaseSensitive size={14} />}
            label="Match case"
            description="Only the exact capitalisation."
            checked={options.caseSensitive}
            onChange={(caseSensitive) => onChange({ ...options, caseSensitive })}
          />
          <OptionRow
            icon={<WholeWord size={14} />}
            label="Whole word"
            description="Skips matches inside a longer word."
            checked={options.wholeWord}
            onChange={(wholeWord) => onChange({ ...options, wholeWord })}
          />
          <OptionRow
            icon={<Type size={14} />}
            label="Ignore accents"
            description="a also finds à, á and ä."
            checked={options.ignoreAccents}
            onChange={(ignoreAccents) => onChange({ ...options, ignoreAccents })}
          />
          <p className="px-1 pt-1 text-[10.5px] leading-4 text-[var(--quiet)]">
            Enter next · Shift+Enter previous · Esc close
          </p>
        </div>
      </div>
    </div>
  );
}

type OptionRowProps = {
  icon: ReactNode;
  label: string;
  description: string;
  checked: boolean;
  onChange: (next: boolean) => void;
};

function OptionRow({ icon, label, description, checked, onChange }: OptionRowProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className="flex min-h-9 items-center gap-2.5 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)]"
    >
      <span className={`shrink-0 transition-colors ${checked ? "text-[var(--accent)]" : "text-[var(--quiet)]"}`}>{icon}</span>
      <span className="min-w-0 flex-1">
        <span className="block text-[12px] leading-4 text-[var(--text)]">{label}</span>
        <span className="mt-0.5 block text-[11px] leading-4 text-[var(--quiet)]">{description}</span>
      </span>
      {/* The switch is drawn here rather than reusing `Toggle`: a switch inside a
          switch is two controls in one row, and the row is already the control. */}
      <span
        aria-hidden="true"
        className={`relative inline-flex h-[20px] w-[34px] shrink-0 items-center rounded-full transition-colors duration-200 ${checked ? "bg-[var(--accent)]" : "bg-[var(--raised)]"}`}
      >
        <span
          className={`inline-block size-3.5 rounded-full bg-[var(--page)] shadow-sm transition-transform duration-200 ${checked ? "translate-x-[16.5px]" : "translate-x-[3px]"}`}
        />
      </span>
    </button>
  );
}
