import { useEffect, useState } from "react";
import PreviewNote from "./PreviewNote";
import ImageStage from "./ImageStage";

/**
 * An `.svg` drawn from its own text.
 *
 * **The text, not the bytes.** Everything else that is a picture is read through
 * `panel_read_bytes`, but an `.svg` is XML the editor already holds in
 * `useFileEditor` -- so it is handed here rather than read a second time. That
 * also means the preview follows the editor: typing in the source redraws the
 * picture on the next blob, with no save and no re-read.
 *
 * **Loaded as an image, never injected as markup.** An `<img>` gives the file a
 * browser decoder with scripting and external loads switched off, so an `.svg`
 * someone else wrote cannot run anything in the app's own context. Inserting the
 * markup -- `innerHTML` -- would be the shorter route and the wrong one: a
 * `<script>` inside would still be inert, but an `onload` attribute would not be.
 *
 * Drawn through the same stage as [`ImagePreview`], so a vector and a bitmap sit
 * in the panel identically: fitted, never cropped, never stretched past its own
 * size.
 */
export default function SvgPreview({ text }: { text: string }) {
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    // Created in the effect rather than a `useMemo`: the object URL has a
    // lifetime, and an effect can end it. A memo would hand the same URL back
    // after its cleanup had already revoked it.
    const next = URL.createObjectURL(new Blob([text], { type: "image/svg+xml" }));
    setUrl(next);
    setFailed(false);
    return () => URL.revokeObjectURL(next);
  }, [text]);

  if (failed) return <PreviewNote tone="error">This SVG could not be drawn.</PreviewNote>;
  if (!url) return <PreviewNote>Drawing…</PreviewNote>;

  return (
    <ImageStage>
      <img
        src={url}
        alt="SVG preview"
        // A broken document is a decode failure, not an exception, so the only
        // way to say so is the element's own error event.
        onError={() => setFailed(true)}
        // SVG scales without loss, but a picture is still never enlarged past
        // the panel: an icon blown up to fill the width reads as a mistake.
        decoding="async"
        className="max-h-full max-w-full object-contain"
      />
    </ImageStage>
  );
}
