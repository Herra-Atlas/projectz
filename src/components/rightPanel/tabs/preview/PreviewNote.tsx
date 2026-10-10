import { LoaderCircle } from "lucide-react";

/**
 * A preview that is not ready, or that could not be read.
 *
 * One component for both states because they occupy the same place and read as
 * the same kind of interruption: the panel is showing this file and here is why
 * there is nothing in it yet. The loading state carries a spinner for the same
 * reason the panel uses one elsewhere -- a `.docx` is a whole document being
 * unpacked, and a blank panel while that happens reads as a broken file.
 */
export default function PreviewNote({ tone = "quiet", children }: { tone?: "quiet" | "error"; children: React.ReactNode }) {
  return (
    <div className="flex min-h-0 flex-1 items-start gap-2 overflow-auto px-4 py-3 text-xs">
      {tone === "quiet" ? <LoaderCircle size={13} className="mt-0.5 shrink-0 animate-spin text-[var(--quiet)]" /> : null}
      <p className={tone === "error" ? "text-[var(--danger)]" : "text-[var(--quiet)]"}>{children}</p>
    </div>
  );
}
