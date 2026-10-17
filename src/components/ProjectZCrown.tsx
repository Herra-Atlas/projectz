import type { CSSProperties } from "react";
import crown from "../assets/projectz-crown.png";

/**
 * The ProjectZ crown.
 *
 * **Drawn as a mask rather than as the image itself.** The file is black line
 * art, so used directly it would vanish on the dark rail -- and on any other dark
 * surface. As a mask it takes whatever the caller paints behind it
 * (`background-color`), which is how it becomes the title's own colour and stays
 * legible in every theme.
 *
 * `style` is merged last, so the caller sizes, places and tilts it; only the mask
 * plumbing is fixed here, and it is fixed in one place so the header's crown and
 * the collapsed sidebar's cannot drift apart.
 */
export default function ProjectZCrown({
  className,
  style,
}: {
  className?: string;
  style?: CSSProperties;
}) {
  return (
    <span
      aria-hidden="true"
      className={className}
      style={{
        maskImage: `url(${crown})`,
        WebkitMaskImage: `url(${crown})`,
        maskRepeat: "no-repeat",
        WebkitMaskRepeat: "no-repeat",
        maskSize: "contain",
        WebkitMaskSize: "contain",
        maskPosition: "center",
        WebkitMaskPosition: "center",
        ...style,
      }}
    />
  );
}
