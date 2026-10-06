type StartupScreenProps = { message: string; progress: number; error?: string; onRetry?: () => void };

export default function StartupScreen({ message, progress, error, onRetry }: StartupScreenProps) {
  return <main className="grid min-h-dvh place-items-center bg-[var(--page)] px-6 text-[var(--text)]" aria-live="polite">
    <section className="w-full max-w-sm">
      <h1 className="text-lg font-medium tracking-tight">ProjectZ</h1>
      <p className="mt-8 text-sm text-[var(--muted)]">{error || message}</p>
      {!error && <><div className="mt-3 h-1 overflow-hidden rounded-full bg-[var(--raised)]"><div className="h-full rounded-full bg-[var(--accent)] transition-[width] duration-300" style={{ width: `${Math.max(0, Math.min(progress, 100))}%` }} /></div><p className="mt-2 text-right text-xs tabular-nums text-[var(--quiet)]">{progress}%</p></>}
      {error && onRetry && <button type="button" onClick={onRetry} className="mt-4 min-h-10 rounded-md border border-[var(--line)] px-3 text-sm hover:bg-[var(--raised)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]">Retry startup</button>}
    </section>
  </main>;
}
