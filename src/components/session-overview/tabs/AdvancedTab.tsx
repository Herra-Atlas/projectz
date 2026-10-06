import { Brain, Globe2, Timer } from "lucide-react";
import type { SessionStats } from "../../../features/chat/sessionStats";
import type { ChatSession } from "../../../features/chat/types";
import { Group, Stat, StatGrid } from "../StatGroup";

/** The record's own metadata: what answered, what it looked up, and the flags
    that describe the transcript. Kept off the opening view because none of it
    is a performance figure. */
export default function AdvancedTab({ session, stats }: { session: ChatSession; stats: SessionStats }) {
  return (
    <>
      <Group icon={<Brain size={13} />} title="Models">
        {stats.models.length === 0 ? (
          <p className="text-sm text-[var(--muted)]">No model recorded for this conversation.</p>
        ) : (
          <ul className="space-y-2">
            {stats.models.map((model) => (
              <li key={model} className="flex min-w-0 items-center justify-between gap-3">
                <span className="truncate text-sm text-[var(--text)]" title={model}>{model}</span>
                {stats.providers.length > 0 && (
                  <span className="shrink-0 text-[11px] text-[var(--quiet)]">{stats.providers.join(", ")}</span>
                )}
              </li>
            ))}
          </ul>
        )}
      </Group>

      {stats.searches > 0 && (
        <Group icon={<Globe2 size={13} />} title="Web search">
          <StatGrid>
            <Stat label="Searches run" value={String(stats.searches)} hint="tool results in this chat" />
          </StatGrid>
        </Group>
      )}

      <Group icon={<Timer size={13} />} title="Flags">
        <div className="flex flex-wrap gap-2">
          {session.pinned && <Flag>Pinned</Flag>}
          {/* A model-generated title also sets `renamed`, so the two cases are
              reported separately: only `renamedByUser` means the user typed it,
              and a title nobody set explicitly is the opening prompt. */}
          {session.renamedByUser
            ? <Flag>Title set by you</Flag>
            : session.renamed
              ? <Flag>Title generated</Flag>
              : <Flag>Title from first message</Flag>}
          {stats.hasEstimatedRates && <Flag>Some rates estimated</Flag>}
          {stats.models.length === 0 && <Flag>No model recorded</Flag>}
        </div>
      </Group>
    </>
  );
}

/** A flag. Square-cornered and flat, so a row of them reads as labelled
    entries in a record rather than as a set of decorative pills. */
function Flag({ children }: { children: React.ReactNode }) {
  return (
    <span className="border-l-2 border-[var(--line)] bg-[var(--raised)] px-2.5 py-1 text-[11px] text-[var(--muted)]">
      {children}
    </span>
  );
}
