/** Shapes returned by the `database_usage_report` Tauri command. */

export type TokenBucket = {
  /** Bucket start as an ISO date, month, or year depending on the timeframe. */
  bucket: string;
  prompt_tokens: number;
  completion_tokens: number;
};

export type SessionUsage = {
  id: string;
  title: string;
  created_at: string;
  message_count: number;
  prompt_tokens: number;
  completion_tokens: number;
};

export type ModelUsage = {
  model_id: string;
  provider_id: string;
  replies: number;
  prompt_tokens: number;
  completion_tokens: number;
};

/** One model's tokens in one bucket, for the per-model series. */
export type ModelBucket = {
  bucket: string;
  model_id: string;
  provider_id: string;
  prompt_tokens: number;
  completion_tokens: number;
};

export type UsageReport = {
  range: string;
  since: string;
  sessions: number;
  user_messages: number;
  assistant_messages: number;
  searches: number;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  /**
   * Prompt tokens the provider reported as served from its cache.
   *
   * `cache_reported_replies` is what makes these three readable together. Zero
   * replies means the provider reported no cache figure at all, which is an
   * unknown rather than a rate of zero, so the page shows a dash instead of
   * `0%`.
   */
  cached_tokens: number;
  /** Prompt tokens those figures were measured against, from the same replies. */
  cache_prompt_tokens: number;
  cache_reported_replies: number;
  timeline: TokenBucket[];
  session_usage: SessionUsage[];
  models: ModelUsage[];
  model_timeline: ModelBucket[];
  /**
   * What taste spent, in its own figures.
   *
   * A learning run is not a conversation, so it is reported beside the totals
   * rather than folded into them. `taste_runs` counts attempts, including a
   * skipped or failed one that reported no tokens.
   */
  taste_prompt_tokens: number;
  taste_completion_tokens: number;
  taste_total_tokens: number;
  taste_runs: number;
  taste_models: ModelUsage[];
  /** Taste's own per-model series, drawn by the same chart as the other views. */
  taste_timeline: ModelBucket[];
};
