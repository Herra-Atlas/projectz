import type { ToolUsage } from "../statistics/types";
import type { ChatMessage, ChatSession } from "./types";

export type SessionStats = {
  messageCount: number;
  userMessages: number;
  assistantMessages: number;
  /** Replies that reported timing, i.e. the sample the rates are based on. */
  measuredReplies: number;
  reasoningReplies: number;
  searches: number;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  /** Every request's input, added up. Counts a reply's tool rounds, so it is the
      conversation's real spend and is never smaller than [`promptTokens`]. */
  usedPromptTokens: number;
  usedCompletionTokens: number;
  usedTotalTokens: number;
  /**
   * Calls made to a model across the conversation, tool rounds included.
   *
   * One reply is at least one request, and a reply that ran three tools is four.
   * A reply saved before the counter existed contributes one, which is exact
   * for a single-request reply and an undercount for an old tool-heavy one.
   */
  requests: number;
  /** Tool calls by name, busiest first, from the steps behind each reply. */
  tools: ToolUsage[];
  /** Summed time spent generating, across measured replies. */
  generationSeconds: number;
  promptSeconds: number;
  /** Completion tokens per second across all measured replies. */
  aggregateTokensPerSecond: number;
  /** Mean of the per-reply prompt-processing rates, when reported. */
  averagePromptTokensPerSecond: number | null;
  /** Largest prompt size seen, i.e. how full the context got. */
  peakContextTokens: number;
  /** True when any rate was inferred from text length rather than reported. */
  hasEstimatedRates: boolean;
  /** Prompt tokens the provider served from its cache, over replies that said. */
  cachedTokens: number;
  /** Those same replies' prompt, the denominator of `cacheHitRate`. */
  cachePromptTokens: number;
  /** Share of the prompt read from cache, or null when no reply reported one. */
  cacheHitRate: number | null;
  models: string[];
  providers: string[];
  /** Per-reply figures for the charts, oldest first. */
  replies: ReplySample[];
  /** Context size per reply, for the context chart. */
  context: ContextPoint[];
  createdAt: string;
  updatedAt: string;
};

/** One reply's own figures, oldest first, for the per-reply charts. */
export type ReplySample = {
  /** 1-based position among the conversation's replies. */
  index: number;
  /** The conversation it started from -- `context_tokens`, not the last round. */
  contextTokens: number;
  completionTokens: number;
  generationSeconds: number;
  /** Completion tokens per second for this reply, or null when unmeasured. */
  tokensPerSecond: number | null;
  /** True when the rate was inferred from text length rather than reported. */
  rateEstimated: boolean;
};

/** One point on the context chart: how large the conversation was when the
    reply's first request was sent, plotted in the order the replies happened. */
export type ContextPoint = {
  index: number;
  /** The conversation at the start of this reply. It grows as turns are added,
      so a drop is a trim rather than an ordinary turn. */
  contextTokens: number;
  completionTokens: number;
  /** The provider's cache figure for that same first request, so the two are
      comparable. Absent on replies saved before it was recorded. */
  cachedTokens: number | null;
};

const emptyStats = (session: ChatSession): SessionStats => ({
  messageCount: 0,
  userMessages: 0,
  assistantMessages: 0,
  measuredReplies: 0,
  reasoningReplies: 0,
  searches: 0,
  promptTokens: 0,
  completionTokens: 0,
  totalTokens: 0,
  usedPromptTokens: 0,
  usedCompletionTokens: 0,
  usedTotalTokens: 0,
  requests: 0,
  tools: [],
  generationSeconds: 0,
  promptSeconds: 0,
  aggregateTokensPerSecond: 0,
  averagePromptTokensPerSecond: null,
  peakContextTokens: 0,
  hasEstimatedRates: false,
  cachedTokens: 0,
  cachePromptTokens: 0,
  cacheHitRate: null,
  models: [],
  providers: [],
  replies: [],
  context: [],
  createdAt: session.createdAt,
  updatedAt: session.updatedAt,
});

const add = (values: number[]) => values.reduce((total, value) => total + value, 0);

/**
 * Derives display figures for the session overview.
 *
 * Rates are aggregated from totals rather than averaged per reply, so one slow
 * long answer is not counted as heavily as one fast short answer. Only replies
 * that actually reported timing contribute, and a reply's estimated rate is
 * excluded from the prompt-processing average because it is not measured.
 */
export function computeSessionStats(session: ChatSession): SessionStats {
  const stats = emptyStats(session);
  const promptRates: number[] = [];
  const models = new Set<string>();
  const providers = new Set<string>();
  const tools = new Map<string, ToolUsage>();

  stats.messageCount = session.messages.length;

  for (const message of session.messages) {
    if (message.modelId) models.add(message.modelId);
    if (message.providerId) providers.add(message.providerId);

    if (message.role === "user") {
      stats.userMessages += 1;
      continue;
    }
    if (message.role === "tool") {
      stats.searches += 1;
      continue;
    }
    if (message.role !== "assistant") continue;

    stats.assistantMessages += 1;
    if (message.reasoning) stats.reasoningReplies += 1;

    // Every reply made at least one call; a reply that ran tools made more, and
    // the backend counted them. A reply saved before the counter existed falls
    // back to one.
    if (message.metrics) stats.requests += message.metrics.requests ?? 1;

    // The steps are the only record of a tool call: no message row is written
    // for one. Counted here rather than fetched again so the overview needs no
    // second query.
    for (const step of message.activity ?? []) {
      // A synthetic row (the retry wait) is drawn like a call but is not one.
      if (step.kind !== "tool" || step.synthetic) continue;
      const entry = tools.get(step.tool) ?? { tool: step.tool, calls: 0, cached: 0, failed: 0 };
      entry.calls += 1;
      if (step.cached) entry.cached += 1;
      if (step.failed) entry.failed += 1;
      tools.set(step.tool, entry);
    }

    const metrics = message.metrics;
    if (!metrics) continue;

    // A reply counts as measured only once real throughput is known.
    const measured = metrics.generation_rate_estimated !== true;
    // How large the conversation was when this reply *started*. The last round is
    // larger whenever a tool ran, but that round is never stored and never
    // re-sent, so it is not the conversation's size. `context_tokens` is written
    // by the backend; older replies only have the final-round figure.
    const contextTokens = metrics.context_tokens ?? metrics.prompt_tokens;
    if (contextTokens != null) {
      stats.promptTokens += contextTokens;
      stats.peakContextTokens = Math.max(stats.peakContextTokens, contextTokens);
    }
    if (metrics.completion_tokens != null) stats.completionTokens += metrics.completion_tokens;
    // What the reply spent across every round it made, which is larger than the
    // figures above whenever a tool ran. Falls back to the last-round figures for
    // a reply saved before the rounds were totalled -- exact for a reply that made
    // a single request, an undercount only for an old one.
    stats.usedPromptTokens += metrics.used_prompt_tokens ?? metrics.prompt_tokens ?? 0;
    stats.usedCompletionTokens += metrics.used_completion_tokens ?? metrics.completion_tokens ?? 0;
    // The cache figure belongs to the round that reported it -- the provider
    // answers "how much of *this* request did I already have" -- so its
    // denominator is that same round's prompt, not the reply's starting context.
    if (metrics.cached_tokens != null && metrics.prompt_tokens != null) {
      stats.cachedTokens += metrics.cached_tokens;
      stats.cachePromptTokens += metrics.prompt_tokens;
    }
    stats.generationSeconds += metrics.generation_seconds;
    stats.promptSeconds += metrics.prompt_seconds;
    if (metrics.generation_rate_estimated) stats.hasEstimatedRates = true;

    // A reply with no usage reporting contributes nothing to the charts, so it
    // is left out rather than plotted as a zero that looks like a stall.
    const completionTokens = metrics.completion_tokens ?? 0;
    if (metrics.prompt_tokens != null || metrics.completion_tokens != null) {
      stats.replies.push({
        index: stats.assistantMessages,
        contextTokens: contextTokens ?? 0,
        completionTokens,
        generationSeconds: metrics.generation_seconds,
        // A rate is only shown when the model reported the timing behind it.
        tokensPerSecond: measured && metrics.generation_seconds > 0
          ? completionTokens / metrics.generation_seconds
          : null,
        rateEstimated: metrics.generation_rate_estimated === true,
      });
      // The context chart follows the same measure. It is tracked separately from
      // `replies` because a context figure is worth showing even when the reply's
      // own timing was inferred.
      if (contextTokens != null) {
        stats.context.push({
          index: stats.assistantMessages,
          contextTokens,
          completionTokens,
          // The first round's cache figure, not the last round's: a tool round
          // inflates its prompt with results that are never stored, so the two
          // series must describe the same request or the chart shows a peak the
          // next turn cannot repeat. An old reply has only the last-round figure,
          // and mixing the two is what made a working cache look like a
          // collapsing context.
          cachedTokens: metrics.context_cached_tokens ?? null,
        });
      }
    }

    if (!measured) continue;
    stats.measuredReplies += 1;
    if (metrics.prompt_eval_tokens_per_second != null) promptRates.push(metrics.prompt_eval_tokens_per_second);
  }

  stats.totalTokens = stats.promptTokens + stats.completionTokens;
  stats.usedTotalTokens = stats.usedPromptTokens + stats.usedCompletionTokens;
  stats.aggregateTokensPerSecond = stats.generationSeconds > 0
    ? stats.completionTokens / stats.generationSeconds
    : 0;
  stats.averagePromptTokensPerSecond = promptRates.length > 0 ? add(promptRates) / promptRates.length : null;
  stats.cacheHitRate = stats.cachePromptTokens > 0 ? stats.cachedTokens / stats.cachePromptTokens : null;
  stats.models = [...models];
  stats.providers = [...providers];
  stats.tools = [...tools.values()].sort((a, b) => b.calls - a.calls);
  return stats;
}

/** Formats a token count compactly: 1.2M, 18.4K, 942. */
export const formatTokens = (count: number) =>
  count >= 1_000_000 ? `${(count / 1_000_000).toFixed(1)}M`
    : count >= 10_000 ? `${(count / 1_000).toFixed(1)}K`
      : count.toLocaleString();

export const formatTimestamp = (iso: string) => {
  const parsed = new Date(iso);
  if (Number.isNaN(parsed.getTime())) return "—";
  return parsed.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
};

/** Re-exported so the overview does not need to know the message shape. */
export type { ChatMessage };
