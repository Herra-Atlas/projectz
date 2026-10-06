import { Bar, BarChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import type { UsageReport } from "../../features/statistics/types";

/** Where the activity came from: what the user sent, what the model replied,
    and how many searches the agent ran. Three bars rather than a stacked one so
    the totals stay comparable against each other. */
export default function ActivityChart({ report }: { report: UsageReport }) {
  const data = [
    { name: "Sent", value: report.user_messages, fill: "var(--accent)" },
    { name: "Replies", value: report.assistant_messages, fill: "var(--muted)" },
    { name: "Searches", value: report.searches, fill: "var(--quiet)" },
  ];
  const total = data.reduce((sum, entry) => sum + entry.value, 0);

  if (total === 0) {
    return <p className="px-4 py-12 text-center text-sm text-[var(--muted)]">No messages in this period.</p>;
  }

  return (
    <div className="h-[220px] w-full px-1 py-3">
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={data} margin={{ top: 4, right: 12, bottom: 0, left: 0 }}>
          <CartesianGrid stroke="var(--line)" strokeDasharray="2 4" vertical={false} />
          <XAxis
            dataKey="name"
            stroke="var(--quiet)"
            tick={{ fontSize: 11 }}
            tickLine={false}
            axisLine={{ stroke: "var(--line)" }}
          />
          <YAxis stroke="var(--quiet)" tick={{ fontSize: 11 }} tickLine={false} axisLine={false} width={32} allowDecimals={false} />
          <Tooltip
            cursor={{ fill: "color-mix(in srgb, var(--raised) 60%, transparent)" }}
            contentStyle={{
              background: "var(--rail)",
              border: "1px solid var(--line)",
              borderRadius: 8,
              fontSize: 12,
            }}
            labelStyle={{ color: "var(--muted)" }}
            formatter={(value) => [Number(value).toLocaleString(), "Count"]}
          />
          <Bar dataKey="value" radius={[4, 4, 0, 0]} maxBarSize={56} fill="var(--accent)" />
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}
