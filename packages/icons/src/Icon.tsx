import { type CSSProperties, useEffect, useState } from "react";

import { type IconWeight, iconSvgSync, loadIcon } from "./icons";

export interface IconProps {
  /** A Phosphor icon name, e.g. `barbell`. Unknown names render an empty box of the same size. */
  name: string;
  weight?: IconWeight;
  /** Box size in px (or any CSS length). The glyph uses `currentColor`. */
  size?: number | string;
  className?: string;
  style?: CSSProperties;
  /** Accessible name; without it the icon is decorative (`aria-hidden`). */
  title?: string;
}

/** Inline SVG, sized to its box. The markup comes from Phosphor's static assets (trusted), never from app data. */
function sized(svg: string): string {
  return svg.replace("<svg ", '<svg width="100%" height="100%" style="display:block" ');
}

export function Icon({ name, weight = "regular", size = 20, className, style, title }: IconProps) {
  const [svg, setSvg] = useState(() => iconSvgSync(name, weight));
  useEffect(() => {
    let live = true;
    const ready = iconSvgSync(name, weight);
    setSvg(ready);
    if (ready === undefined) void loadIcon(name, weight).then((s) => live && setSvg(s));
    return () => {
      live = false;
    };
  }, [name, weight]);

  return (
    <span
      className={className ? `ph-icon ${className}` : "ph-icon"}
      role={title ? "img" : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
      style={{ display: "inline-block", flex: "none", lineHeight: 0, width: size, height: size, ...style }}
      // biome-ignore lint/security/noDangerouslySetInnerHtml: static Phosphor SVG assets, looked up by validated name
      dangerouslySetInnerHTML={svg ? { __html: sized(svg) } : undefined}
    />
  );
}
