/**
 * Which engine the Local page is showing.
 *
 * The page used to be one list, because there was one kind of engine. Speech
 * adds two more kinds with nothing in common with a GGUF model -- a Whisper model
 * and a Piper voice are not things llama.cpp can load -- so the page is split by
 * engine and each half keeps its own vocabulary. `local` alone in
 * `settingsPages.ts` still names the page; this only names the half.
 */
export type LocalCategory = "llama" | "stt" | "tts";

const CATEGORIES: ReadonlyArray<{ id: LocalCategory; label: string }> = [
  { id: "llama", label: "llama.cpp" },
  { id: "stt", label: "Speech recognition" },
  { id: "tts", label: "Speech synthesis" },
];

/** The heading the page carries while a category is open. */
export const LOCAL_TITLES: Record<LocalCategory, string> = {
  llama: "Local models",
  stt: "Speech recognition",
  tts: "Speech synthesis",
};

export function LocalCategoryTabs({
  value,
  onChange,
}: {
  value: LocalCategory;
  onChange: (next: LocalCategory) => void;
}) {
  return (
    <div
      role="group"
      aria-label="Local engine"
      className="inline-flex shrink-0 gap-0.5 rounded-lg bg-[var(--raised)] p-0.5"
    >
      {CATEGORIES.map((category) => (
        <button
          key={category.id}
          type="button"
          aria-pressed={value === category.id}
          onClick={() => onChange(category.id)}
          className={`min-h-7 rounded-[6px] px-2.5 text-[12px] transition-colors focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-[var(--accent)] ${value === category.id ? "bg-[var(--panel)] text-[var(--text)] shadow-sm" : "text-[var(--muted)] hover:text-[var(--text)]"}`}
        >
          {category.label}
        </button>
      ))}
    </div>
  );
}
