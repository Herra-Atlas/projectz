import { useEffect, useRef, useState } from "react";
import { CircleHelp } from "lucide-react";

/** A question the agent is waiting on. */
export type PendingQuestion = {
  /** Ties the prompt to the run, so two sessions can each have one open. */
  runId: string;
  questionId: string;
  question: string;
  /** Suggested answers, offered as buttons. May be empty. */
  options: string[];
};

type ToolQuestionProps = {
  request: PendingQuestion;
  onAnswer: (questionId: string, answer: string) => void;
};

/**
 * Asks the user a question the agent cannot answer itself.
 *
 * **A bar above the composer, like the approval prompt.** The two are the same
 * kind of thing -- the run is parked until the user acts -- so they take the same
 * place and the same surface, and the transcript stays in view behind them.
 *
 * **The question wraps rather than truncates.** An approval shows a command, and a
 * command is one line; a question is a sentence, sometimes two, and cutting it off
 * would hide the very thing being asked.
 *
 * **Options are buttons, but the field is always there.** A closed set is offered
 * as choices for speed, and the free-text field means a user is never forced into
 * an option that does not fit -- the model asked a question, not a multiple-choice
 * exam.
 */
export default function ToolQuestion({ request, onAnswer }: ToolQuestionProps) {
  const [text, setText] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  // Focus the field so the answer can be typed straight away.
  useEffect(() => { inputRef.current?.focus(); }, []);

  const submit = () => {
    const answer = text.trim();
    if (answer) onAnswer(request.questionId, answer);
  };

  return (
    <div
      role="alertdialog"
      aria-modal="false"
      aria-labelledby="tool-question-title"
      className="flex flex-col gap-2 rounded-lg border border-[color-mix(in_srgb,var(--accent)_30%,var(--line))] bg-[var(--panel)] px-3 py-2.5 shadow-[0_10px_28px_rgba(0,0,0,0.22)]"
    >
      <div className="flex items-start gap-2">
        <CircleHelp size={13} className="mt-0.5 shrink-0 text-[var(--accent)]" />
        <span id="tool-question-title" className="text-[13px] leading-5 text-[var(--text)]">
          {request.question}
        </span>
      </div>

      {request.options.length > 0 && (
        <div className="flex flex-wrap gap-1.5 pl-[21px]">
          {request.options.map((option) => (
            <button
              key={option}
              type="button"
              onClick={() => onAnswer(request.questionId, option)}
              className="inline-flex h-7 items-center rounded-md border border-[var(--line)] px-2.5 text-[11px] text-[var(--text)] transition-colors hover:border-[var(--line-strong)] hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
            >
              {option}
            </button>
          ))}
        </div>
      )}

      {/* The field sits inside a form so Enter submits, matching the composer's
          own send behaviour rather than inventing a second one. */}
      <form
        onSubmit={(event) => { event.preventDefault(); submit(); }}
        className="flex items-center gap-1.5 pl-[21px]"
      >
        <input
          ref={inputRef}
          value={text}
          onChange={(event) => setText(event.target.value)}
          placeholder="Answer…"
          aria-label="Your answer"
          className="h-8 min-w-0 flex-1 rounded-md border border-[var(--line)] bg-[var(--rail)] px-2.5 text-[12px] text-[var(--text)] outline-none placeholder:text-[var(--quiet)] focus:border-[var(--accent)]"
        />
        <button
          type="submit"
          disabled={!text.trim()}
          className="inline-flex h-8 shrink-0 items-center rounded-md bg-[var(--accent)] px-3 text-[11px] font-medium text-[var(--accent-ink)] transition-opacity hover:opacity-90 disabled:opacity-40 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          Answer
        </button>
      </form>
    </div>
  );
}
