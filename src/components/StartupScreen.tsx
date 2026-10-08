type StartupScreenProps = { message: string; progress: number; error?: string; onRetry?: () => void };

/**
 * What the window shows while the database opens and the first query runs.
 *
 * Centred and quiet: the app name, the step in the backend's own words, and a
 * hairline of progress. It deliberately does not dress a two-second wait up with
 * a splash screen, and it names the actual work ("Importing existing data")
 * rather than saying "Loading", which names nothing.
 */
export default function StartupScreen({ message, progress, error, onRetry }: StartupScreenProps) {
  const clamped = Math.max(0, Math.min(progress, 100));
  return <main className="grid min-h-dvh place-items-center bg-[var(--page)] px-6 text-[var(--text)]" aria-live="polite">
    <section className="w-full max-w-xs text-center">
      <h1 className="text-[22px] font-semibold tracking-tight">ProjectZ</h1>
      <p className="mt-2 text-[13px] leading-5 text-[var(--quiet)]">{error || message}</p>
      {!error && <div className="mt-6 h-[3px] overflow-hidden rounded-full bg-[var(--raised)]"><div className="h-full rounded-full bg-[var(--accent)] transition-[width] duration-300" style={{ width: `${clamped}%` }} /></div>}
      {error && onRetry && <button type="button" onClick={onRetry} className="mt-6 inline-flex min-h-9 items-center rounded-lg border border-[var(--line)] px-3.5 text-[13px] font-medium transition-colors hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]">Retry startup</button>}
    </section>
  </main>;
}
