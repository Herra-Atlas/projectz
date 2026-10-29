import { useEffect, useRef } from "react";
import type { ChatMode } from "../../features/chat/chatMode";
import { useWeather } from "../../features/weather/useWeather";
import type { WeatherCondition } from "../../features/weather/weather";

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
 * under the reader, and returning to it is a new arrival anyway. A configured
 * connection's weather only takes the line when it is worth taking (see
 * `greetingFor`); the clock form paints immediately and remains the fallback
 * when weather is unreachable.
 *
 * Who it is addressed to comes from `app.general.userName`, set in Settings →
 * General → Your name. With no name the greeting stands alone ("Good evening");
 * naming somebody the greeting did not choose for them is the one thing it must
 * not do.
 */
/**
 * The greeting, ranked by how much it actually says.
 *
 * Three tiers, most interesting first. Severe weather (a storm, rain) beats the
 * clock because it is the thing that will shape the day. The clock comes next --
 * late night and morning are worth naming. Everything else falls through to the
 * plain time of day. A grey sky does not outrank the hour: "Cloudy day" was
 * spending the one line this screen has on the least interesting fact available,
 * and it was doing it most of the time.
 */
function greetingFor(hour: number, name: string, weather: WeatherCondition | null): string {
  // A name typed in a hurry (" john") still reads as itself, capitalised once.
  const trimmed = name.trim();
  const whom = trimmed ? trimmed.charAt(0).toUpperCase() + trimmed.slice(1) : "";
  const notable = weather ? weatherGreeting(weather) : null;
  if (notable) return addressee(notable, whom);
  if (hour < 5) return addressee("Working late", whom);
  if (hour < 12) return addressee("Good morning", whom);
  if (hour < 18) return addressee("Good afternoon", whom);
  if (hour < 22) return addressee("Good evening", whom);
  return addressee("Working late", whom);
}

/** "Good evening" alone, or "Good evening, Atlas" once a name is known. */
function addressee(words: string, whom: string): string {
  return whom ? `${words}, ${whom}` : words;
}

/**
 * The headline for a condition, or `null` when it is not worth saying.
 *
 * Only weather that changes what the day is like gets to interrupt the clock.
 */
function weatherGreeting(weather: WeatherCondition): string | null {
  switch (weather) {
    case "storm":
      return "Stormy out there";
    case "rain":
      return "Rainy day";
    // These describe the sky rather than the day. Left to the clock.
    case "drizzle":
    case "snow":
    case "fog":
    case "cloudy":
    case "clear":
      return null;
  }
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
  const weather = useWeather();
  const greeting = greetingFor(new Date().getHours(), userName, weather);

  // One line, and it is the truth about the mode in force. Saying "switch to
  // Agent" to someone already in Agent would be telling them to do what they
  // have done; with no model there is nothing to describe yet, so it only
  // points at the picker.
  const hint = !hasModel
    ? "Pick a model below to begin."
    : mode === "agent"
      ? "Ask for anything. The agent can create groundbreaking things that you won't."
      : "Ask a question. Hopefully I can answer whatever you need.";

  // The hint changes while the screen is up -- switching mode, or a model being
  // picked -- so the line re-enters with the same lift instead of swapping its
  // words in place. The stagger belongs to the screen's own arrival: on a change
  // this is already the second line the eye is on, and 110ms of nothing would
  // read as a stutter.
  //
  // A ref rather than state, because flipping it must not re-render: a changed
  // `animation-delay` retimes the animation that is already running.
  const arrived = useRef(false);
  useEffect(() => {
    arrived.current = true;
  }, []);

  return (
    <section className="mx-auto flex w-full max-w-3xl flex-1 flex-col items-center justify-center px-4 pb-20 text-center" aria-labelledby="welcome-title">
      <h2 id="welcome-title" className="pz-welcome-in text-[34px] font-medium leading-[1.15] tracking-[-0.03em] text-[var(--text)]">
        {greeting}
      </h2>
      <p
        key={hint}
        className="pz-welcome-in mt-3.5 text-[14px] leading-6 text-[var(--muted)]"
        style={arrived.current ? undefined : { animationDelay: "110ms" }}
      >
        {hint}
      </p>
    </section>
  );
}
