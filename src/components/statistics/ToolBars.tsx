import type { ToolUsage } from "../../features/statistics/types";

/** How many tools are drawn before the rest are left out. A panel is a glance,
    not the full catalogue: the long tail is one call each and would push the
    tools that matter off the bottom. */
const SHOWN = 7;

/**
 * Tool calls as a ranked list of bars.
 *
 * A bar list rather than a chart: the names are long, there are only a handful,
 * and every row's own count is the interesting number — an axis and gridlines
 * would take space to say less. The width is relative to the busiest tool, so
 * the shape of a session reads at a glance.
 *
 * `cached` and `failed` are shown under a row only when they are nonzero. A
 * zero is not worth a line, and a tool that was never cached is not a problem
 * to report.
 *
 * Shared by the Statistics page and the session overview's Statistics tab, so
 * "what the agent did" is drawn the same way in both places.
 */
export function ToolBars({ tools, empty }: { tools: ToolUsage[]; empty: string }) {
  const shown = tools.slice(0, SHOWN);
  if (shown.length === 0) {
    return <p className="px-4 py-10 text-center text-sm text-[var(--muted)]">{empty}</p>;
  }

  const most = shown[0]?.calls ?? 0;

  return (
    <ul className="flex flex-col gap-2.5 px-4 py-3.5">
      {shown.map((tool) => (
        <li key={tool.tool} className="min-w-0">
          <div className="flex items-baseline justify-between gap-3">
            <span className="truncate text-xs text-[var(--text)]" title={tool.tool}>{tool.tool}</span>
            <span className="shrink-0 text-xs tabular-nums text-[var(--muted)]">{tool.calls.toLocaleString()}</span>
          </div>
          <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-[var(--raised)]">
            <span
              className="block h-full rounded-full bg-[var(--accent)]"
              // A tool with one call beside one with four hundred would otherwise
              // draw as nothing; the floor keeps every listed row visible.
              style={{ width: `${most > 0 ? Math.max(3, (tool.calls / most) * 100) : 0}%` }}
            />
          </div>
          {(tool.cached > 0 || tool.failed > 0) && (
            <p className="mt-1 text-[10px] text-[var(--quiet)]">
              {tool.cached > 0 && `${tool.cached.toLocaleString()} cached`}
              {tool.cached > 0 && tool.failed > 0 && " · "}
              {tool.failed > 0 && `${tool.failed.toLocaleString()} failed`}
            </p>
          )}
        </li>
      ))}
    </ul>
  );
}
