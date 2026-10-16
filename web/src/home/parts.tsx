// A section's head: a few big words, one line under them, and maybe a link. Each section sets it the way its content
// asks (T-191), so the page doesn't open every part the same way:
// - `stage`: centred over a show that's centred too (the agents, everything you'd expect);
// - `beside`: left-aligned, in a column beside the R1 or a list (android, dock mode, private);
// - `split`: across the page, the words on the left and the line and link on the right (need an app);
// - `quiet`: a heading's size, over a picture that leads, its link at the far end (the companion).
// As it scrolls in the words rise a short way together, then the line, then the link. With reduced motion it's simply
// there.

import { Icon } from "@r1os/icons";
import { motion, useReducedMotion } from "motion/react";
import type { ReactNode } from "react";
import { Link } from "react-router";
import { after, reveal } from "../site/motion";

export type HeadLayout = "stage" | "beside" | "split" | "quiet";

export function Head({
  title,
  sub,
  link,
  layout,
  className,
}: {
  title: ReactNode;
  sub: ReactNode;
  link?: { to: string; label: string };
  layout: HeadLayout;
  className?: string;
}) {
  const reduce = useReducedMotion();
  return (
    <motion.header className={`head ${layout}${className ? ` ${className}` : ""}`} {...(reduce ? {} : reveal)}>
      <motion.h2 variants={after(0, 12)}>{title}</motion.h2>
      <motion.p variants={after(0.08)}>{sub}</motion.p>
      {link && (
        <motion.div className="head-go" variants={after(0.14, 8)}>
          <Link to={link.to} className="head-link">
            {link.label} <Icon name="arrow-right" weight="bold" size={16} />
          </Link>
        </motion.div>
      )}
    </motion.header>
  );
}
