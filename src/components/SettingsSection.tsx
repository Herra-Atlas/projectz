import type { ReactNode } from "react";

/** Shared primitives for the settings pages.
    A section is a titled group of rows on its own surface: the heading names the
    category, and the bordered panel under it holds the settings. Grouping the
    rows into one surface rather than ruling them directly on the page is what
    lets a reader see where one category ends and the next begins, which a run of
    hairlines down an undifferentiated column cannot show. */

type SettingsSectionProps = { title: string; children: ReactNode };

export function SettingsSection({ title, children }: SettingsSectionProps) {
  return (
    <section className="mt-7 first:mt-0">
      <h3 className="pb-2 pl-0.5 text-[12px] font-semibold tracking-tight text-[var(--muted)]">{title}</h3>
      <div className="divide-y divide-[var(--line)] overflow-hidden rounded-lg border border-[var(--line)] bg-[var(--panel)]">{children}</div>
    </section>
  );
}

type SettingRowProps = { label: string; description?: string; /** Full-width controls, such as a textarea, sit under the label instead of beside it. */ stacked?: boolean; control?: ReactNode };

export function SettingRow({ label, description, stacked, control }: SettingRowProps) {
  return (
    <div className={`flex gap-6 px-4 py-3.5 ${stacked ? "flex-col items-stretch" : "min-h-[58px] items-center justify-between"}`}>
      <div className="min-w-0">
        <p className="text-[13px]">{label}</p>
        {description && <p className="mt-0.5 max-w-md text-[12px] leading-5 text-[var(--muted)]">{description}</p>}
      </div>
      {control && <div className={stacked ? "mt-3" : "shrink-0"}>{control}</div>}
    </div>
  );
}

export function Toggle({ checked, onChange, label }: { checked: boolean; onChange: (next: boolean) => void; label: string }) {
  return (
    <button type="button" role="switch" aria-label={label} aria-checked={checked} onClick={() => onChange(!checked)} className={`relative inline-flex h-[22px] w-[38px] items-center rounded-full transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] ${checked ? "bg-[var(--accent)]" : "bg-[var(--raised)]"}`}>
      <span className={`inline-block size-4 rounded-full bg-[var(--page)] transition-transform ${checked ? "translate-x-[20px]" : "translate-x-0.5"}`} />
    </button>
  );
}

type SegmentedProps<T extends string> = { value: T; options: readonly T[]; onChange: (next: T) => void; label: string };

/** Mutually exclusive options shown as one control, so the current choice is visible
    without opening anything. */
export function Segmented<T extends string>({ value, options, onChange, label }: SegmentedProps<T>) {
  return (
    <div role="group" aria-label={label} className="inline-flex shrink-0 gap-0.5 rounded-lg bg-[var(--raised)] p-0.5">
      {options.map((option) => <button key={option} type="button" aria-pressed={value === option} onClick={() => onChange(option)} className={`min-h-7 rounded-[6px] px-2.5 text-[12px] transition-colors focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${value === option ? "bg-[var(--panel)] text-[var(--text)] shadow-sm" : "text-[var(--muted)] hover:text-[var(--text)]"}`}>{option}</button>)}
    </div>
  );
}
