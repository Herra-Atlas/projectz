import type { ChatMode } from "../../features/chat/chatMode";

/**
 * The screen an empty conversation shows.
 *
 * Two lines, arrived at by cutting every second sentence: the greeting is the
 * whole headline, because it is the one thing that is different every time the
 * screen appears and the one thing worth reading; the line under it is the
 * only instruction the composer needs. A question like "What are we working
 * on?" was the greeting's job already, and asking it again just delayed the
 * field the user is about to type in.
 *
 * The greeting is read from the clock on render rather than held in state --
 * an empty conversation left open past midnight should not relabel itself
 * under the reader, and returning to it is a new arrival anyway.
 *
 * Who it is addressed to comes from `app.general.userName`, set in Settings →
 * General → Your name. "Sir" is the standing before a name was given -- the
 * greeting is never left speaking to nobody.
 */
function greetingFor(hour: number, name: string): string {
  // A name typed in a hurry (" john") still reads as itself, capitalised once.
  const whom = name ? name.charAt(0).toUpperCase() + name.slice(1) : "Sir";
  if (hour < 5) return `Working late, ${whom}.`;
  if (hour < 12) return `Good morning, ${whom}.`;
  if (hour < 18) return `Good afternoon, ${whom}.`;
  if (hour < 22) return `Good evening, ${whom}.`;
  return `Working late, ${whom}.`;
}

type WelcomeProps = {
  /** Whether a model is selected, which decides what the line under the greeting does. */
  hasModel: boolean;
  /** Chat runs no tools; Agent can touch the workspace. The line says which is in force. */
  mode: ChatMode;
  /** How the user asked to be addressed; empty keeps the greeting's own wording. */
  userName: string;
};

export default function Welcome({ hasModel, mode, userName }: WelcomeProps) {
  const greeting = greetingFor(new Date().getHours(), userName);

  // One line, and it is the truth about the mode in force. Saying "switch to
  // Agent" to someone already in Agent would be telling them to do what they
  // have done; with no model there is nothing to describe yet, so it only
  // points at the picker.
  const hint = !hasModel
    ? "Pick a model below to begin."
    : mode === "agent"
      ? "Ask for anything. The agent can create groundbreaking things that you won't."
      : "Ask a question. Hopefully I can answer whatever you need.";

  return (
    <section className="mx-auto flex w-full max-w-3xl flex-1 flex-col items-center justify-center px-4 pb-20 text-center" aria-labelledby="welcome-title">
      <h2 id="welcome-title" className="pz-welcome-in text-[34px] font-medium leading-[1.15] tracking-[-0.03em] text-[var(--text)]">
        {greeting}
      </h2>
      <p className="pz-welcome-in mt-3.5 text-[14px] leading-6 text-[var(--muted)]" style={{ animationDelay: "110ms" }}>
        {hint}
      </p>
    </section>
  );
}
