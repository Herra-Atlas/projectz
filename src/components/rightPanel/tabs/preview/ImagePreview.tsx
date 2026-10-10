import PreviewNote from "./PreviewNote";
import { useFileBytes } from "./useFileBytes";

/**
 * An image, drawn at the size the panel allows.
 *
 * The bytes come back as an object URL rather than through the asset protocol:
 * the workspace root moves at runtime, and a protocol scope is configured once at
 * startup, so the panel reads the file the same way every other preview does and
 * gets a URL it can revoke.
 *
 * `object-contain` inside a centred box: a 4000px photograph and a 16px icon both
 * land somewhere sensible, and nothing is cropped. Never upscaled past its own
 * size, because a blown-up screenshot is worse than a small one.
 */
export default function ImagePreview({ path }: { path: string }) {
  const { data, error } = useFileBytes(path);

  if (error) return <PreviewNote tone="error">{error}</PreviewNote>;
  if (!data) return <PreviewNote>Reading…</PreviewNote>;

  return (
    <div className="grid min-h-0 flex-1 place-items-center overflow-auto bg-[var(--rail)] p-4">
      <img
        src={data.url}
        alt={path}
        // A thumbnail strip's worth of pixels is not worth decoding a 12MB scan
        // for; the browser scales it down as it loads rather than after.
        decoding="async"
        className="max-h-full max-w-full object-contain"
      />
    </div>
  );
}
