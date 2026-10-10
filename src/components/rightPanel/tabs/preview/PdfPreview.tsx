import { useEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import PreviewNote from "./PreviewNote";
import { useFileBytes } from "./useFileBytes";

/**
 * The part of a PDF.js document this view uses.
 *
 * Declared here rather than imported: PDF.js types a large surface, and a preview
 * that draws one page needs three of its members. Naming them keeps the library
 * at arm's length, so a version bump that moves a type does not reach this file.
 */
type PdfPage = {
  getViewport: (options: { scale: number }) => { width: number; height: number };
  render: (options: {
    canvasContext: CanvasRenderingContext2D;
    viewport: unknown;
  }) => { promise: Promise<void> };
};
type PdfDocument = { numPages: number; getPage: (page: number) => Promise<PdfPage> };

/**
 * A load in progress.
 *
 * The document is torn down through the *task* that opened it rather than through
 * the document: PDF.js keeps the worker's outstanding work on the task, and
 * destroying the document alone leaves it running.
 */
type PdfLoadingTask = { promise: Promise<PdfDocument>; destroy: () => Promise<void> };

/** The widest a page is drawn, so a maximised window does not render a poster. */
const MAX_WIDTH = 1100;
const MIN_SCALE = 0.4;
const MAX_SCALE = 3;

/**
 * The library, with its worker started once for the session.
 *
 * A module-level promise rather than a load per file: PDF.js pulls in a worker and
 * several hundred kilobytes, and doing that again for every PDF opened would make
 * the second one as slow as the first. The worker is resolved through the bundler
 * with `new URL(..., import.meta.url)`, which is what makes it work in the built
 * app as well as in dev.
 */
let pdfjs: Promise<typeof import("pdfjs-dist")> | null = null;

function loadPdfjs() {
  pdfjs ??= import("pdfjs-dist").then((library) => {
    library.GlobalWorkerOptions.workerPort = new Worker(
      new URL("pdfjs-dist/build/pdf.worker.min.mjs", import.meta.url),
      { type: "module" },
    );
    return library;
  });
  return pdfjs;
}

/**
 * A PDF, one page at a time.
 *
 * **One page at a time, not a continuous scroll.** PDF.js renders a page to a
 * canvas on demand; drawing a forty-page document as one scrolling surface means
 * forty canvases alive at once, which is exactly the sort of thing that makes a
 * panel feel heavy. Paging also gives the reader a real position -- "page 3 of
 * 12" -- which a scroll bar over a rasterised page cannot.
 *
 * The page is scaled to the panel's width and re-rendered when the panel is
 * resized, so the text is never a fixed size that is too small to read or too
 * large to fit.
 */
export default function PdfPreview({ path }: { path: string }) {
  const { data, error } = useFileBytes(path);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const documentRef = useRef<PdfDocument | null>(null);
  const taskRef = useRef<PdfLoadingTask | null>(null);
  const [pages, setPages] = useState(0);
  const [page, setPage] = useState(1);
  const [failure, setFailure] = useState<string | null>(null);
  /** Bumped on resize, so the current page is drawn again at the new width. */
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    if (!data) return;
    let active = true;
    setFailure(null);
    setPages(0);

    void (async () => {
      try {
        const library = await loadPdfjs();
        // A copy: PDF.js may hand the buffer to its worker, which would detach
        // the one `useFileBytes` is still holding an object URL for.
        const task = library.getDocument({
          data: new Uint8Array(data.bytes.slice(0)),
        }) as unknown as PdfLoadingTask;
        taskRef.current = task;
        const loaded = await task.promise;
        if (!active) return;
        documentRef.current = loaded;
        setPages(loaded.numPages);
        setPage(1);
      } catch (reason) {
        if (active) setFailure(reason instanceof Error ? reason.message : String(reason));
      }
    })();

    return () => {
      active = false;
      documentRef.current = null;
      const open = taskRef.current;
      taskRef.current = null;
      // Cancels the worker's outstanding work for a document nobody is looking
      // at, which matters when the reader clicks through files quickly.
      if (open) void open.destroy();
    };
  }, [data]);

  useEffect(() => {
    let frame = 0;
    const onResize = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => setRevision((value) => value + 1));
    };
    window.addEventListener("resize", onResize);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("resize", onResize);
    };
  }, []);

  useEffect(() => {
    const document_ = documentRef.current;
    const canvas = canvasRef.current;
    if (!document_ || !canvas || page < 1 || page > document_.numPages) return;
    let active = true;

    void (async () => {
      try {
        const rendered = await document_.getPage(page);
        if (!active) return;
        const base = rendered.getViewport({ scale: 1 });
        const available = Math.min(canvas.parentElement?.clientWidth ?? MAX_WIDTH, MAX_WIDTH) - 32;
        const scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, available / base.width));
        const viewport = rendered.getViewport({ scale });
        canvas.width = Math.floor(viewport.width);
        canvas.height = Math.floor(viewport.height);
        const context = canvas.getContext("2d");
        if (!context) return;
        await rendered.render({ canvasContext: context, viewport }).promise;
      } catch (reason) {
        if (active) setFailure(reason instanceof Error ? reason.message : String(reason));
      }
    })();

    return () => {
      active = false;
    };
  }, [page, pages, data, revision]);

  if (error) return <PreviewNote tone="error">{error}</PreviewNote>;
  if (failure) return <PreviewNote tone="error">Could not open this PDF: {failure}</PreviewNote>;
  if (!data || pages === 0) return <PreviewNote>Reading…</PreviewNote>;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex min-h-8 shrink-0 items-center gap-1 border-b border-[var(--line)] px-3">
        <button
          type="button"
          onClick={() => setPage((current) => Math.max(1, current - 1))}
          disabled={page <= 1}
          aria-label="Previous page"
          className="grid size-6 place-items-center rounded text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] disabled:opacity-40"
        >
          <ChevronLeft size={14} />
        </button>
        <span className="min-w-0 flex-1 text-center text-[11px] tabular-nums text-[var(--quiet)]">
          Page {page} of {pages}
        </span>
        <button
          type="button"
          onClick={() => setPage((current) => Math.min(pages, current + 1))}
          disabled={page >= pages}
          aria-label="Next page"
          className="grid size-6 place-items-center rounded text-[var(--muted)] transition-colors hover:bg-[var(--raised)] hover:text-[var(--text)] disabled:opacity-40"
        >
          <ChevronRight size={14} />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto bg-[var(--rail)] p-4">
        <canvas ref={canvasRef} className="mx-auto block bg-white shadow-sm" aria-label={`Page ${page} of ${path}`} />
      </div>
    </div>
  );
}
