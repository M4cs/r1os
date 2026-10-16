// A section's head, as every one on the home page has it: a few big words, one line under them, and maybe a link. As
// it scrolls in the words rise a short way together, then the line, then the link. With reduced motion it's simply
// there.

import { Icon } from "@r1os/icons";
import { motion, useReducedMotion } from "motion/react";
import type { ReactNode } from "react";
import { Link } from "react-router";
import { after, reveal } from "../site/motion";

export function Head({ title, sub, link }: { title: ReactNode; sub: ReactNode; link?: { to: string; label: string } }) {
  const reduce = useReducedMotion();
  return (
    <motion.header className="head" {...(reduce ? {} : reveal)}>
      <motion.h2 variants={after(0, 12)}>{title}</motion.h2>
      <motion.p variants={after(0.08)}>{sub}</motion.p>
      {link && (
        <motion.div variants={after(0.14, 8)}>
          <Link to={link.to} className="head-link">
            {link.label} <Icon name="arrow-right" weight="bold" size={16} />
          </Link>
        </motion.div>
      )}
    </motion.header>
  );
}
