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
  models: string[];
  providers: string[];
  /** Per-reply figures for the charts, oldest first. */
  replies: ReplySample[];
  /** Prompt size per reply, for the context chart. */
  context: ContextPoint[];
  createdAt: string;
  updatedAt: string;
};

/** One reply's own figures, oldest first, for the per-reply charts. */
export type ReplySample = {
  /** 1-based position among the conversation's replies. */
  index: number;
  promptTokens: number;
  completionTokens: number;
  generationSeconds: number;
  /** Completion tokens per second for this reply, or null when unmeasured. */
  tokensPerSecond: number | null;
  /** True when the rate was inferred from text length rather than reported. */
  rateEstimated: boolean;
};

/** One point on the context chart: the prompt size sent for a single reply,
    plotted in the order the replies happened. */
export type ContextPoint = {
  index: number;
  /** Tokens sent to the model, which grows as the conversation accumulates. */
  promptTokens: number;
  completionTokens: number;
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
  generationSeconds: 0,
  promptSeconds: 0,
  aggregateTokensPerSecond: 0,
  averagePromptTokensPerSecond: null,
  peakContextTokens: 0,
  hasEstimatedRates: false,
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

    const metrics = message.metrics;
    if (!metrics) continue;

    // A reply counts as measured only once real throughput is known.
    const measured = metrics.generation_rate_estimated !== true;
    if (metrics.prompt_tokens != null) {
      stats.promptTokens += metrics.prompt_tokens;
      stats.peakContextTokens = Math.max(stats.peakContextTokens, metrics.prompt_tokens);
    }
    if (metrics.completion_tokens != null) stats.completionTokens += metrics.completion_tokens;
    stats.generationSeconds += metrics.generation_seconds;
    stats.promptSeconds += metrics.prompt_seconds;
    if (metrics.generation_rate_estimated) stats.hasEstimatedRates = true;

    // A reply with no usage reporting contributes nothing to the charts, so it
    // is left out rather than plotted as a zero that looks like a stall.
    const promptTokens = metrics.prompt_tokens ?? 0;
    const completionTokens = metrics.completion_tokens ?? 0;
    if (metrics.prompt_tokens != null || metrics.completion_tokens != null) {
      stats.replies.push({
        index: stats.assistantMessages,
        promptTokens,
        completionTokens,
        generationSeconds: metrics.generation_seconds,
        // A rate is only shown when the model reported the timing behind it.
        tokensPerSecond: measured && metrics.generation_seconds > 0
          ? completionTokens / metrics.generation_seconds
          : null,
        rateEstimated: metrics.generation_rate_estimated === true,
      });
      // The context chart plots the prompt size at each turn. It is tracked
      // separately from `replies` because a context figure is worth showing even
      // when the reply's own timing was inferred.
      if (metrics.prompt_tokens != null) {
        stats.context.push({
          index: stats.assistantMessages,
          promptTokens: metrics.prompt_tokens,
          completionTokens,
        });
      }
    }

    if (!measured) continue;
    stats.measuredReplies += 1;
    if (metrics.prompt_eval_tokens_per_second != null) promptRates.push(metrics.prompt_eval_tokens_per_second);
  }

  stats.totalTokens = stats.promptTokens + stats.completionTokens;
  stats.aggregateTokensPerSecond = stats.generationSeconds > 0
    ? stats.completionTokens / stats.generationSeconds
    : 0;
  stats.averagePromptTokensPerSecond = promptRates.length > 0 ? add(promptRates) / promptRates.length : null;
  stats.models = [...models];
  stats.providers = [...providers];
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
