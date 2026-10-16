import { Component, type ErrorInfo, type ReactNode } from "react";

/**
 * Catches a render crash so it reads as a message instead of a blank window.
 *
 * An error thrown anywhere in the tree unmounts the whole React root, and with no
 * boundary above it there is nothing on screen to say why -- which is the state a
 * user describes as "the app restarted". This keeps the failure legible and offers
 * the one action that gets back to a working app without touching the database.
 *
 * Deliberately the last line of defence, not a substitute for fixing the throw: it
 * catches render errors only, which is what a bad payload or a missing field
 * becomes, and reports them to the console with the component stack so the cause
 * is findable.
 */
type Props = { children: ReactNode };
type State = { message: string };

export default class ErrorBoundary extends Component<Props, State> {
  state: State = { message: "" };

  static getDerivedStateFromError(error: unknown): State {
    return { message: error instanceof Error ? error.message : String(error) };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error("ProjectZ crashed while rendering:", error, info.componentStack);
  }

  render() {
    if (!this.state.message) return this.props.children;
    return (
      <div className="flex h-dvh flex-col items-center justify-center gap-3 bg-[var(--page)] px-6 text-center text-[var(--text)]">
        <p className="text-sm font-medium">Something went wrong.</p>
        <p className="max-w-md break-words text-xs text-[var(--muted)]">{this.state.message}</p>
        <button
          type="button"
          onClick={() => window.location.reload()}
          className="mt-1 inline-flex h-8 items-center rounded-md bg-[var(--accent)] px-3 text-[12px] font-medium text-[var(--accent-ink)] transition-opacity hover:opacity-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]"
        >
          Reload
        </button>
      </div>
    );
  }
}
