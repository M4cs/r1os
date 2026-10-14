// A section's head, as every one on the home page has it: a few big words, one line under them, and maybe a link. The
// words rise one after another as it scrolls in, then the line, then the link; a screen reader hears the head once,
// whole. With reduced motion it's simply there.

import { Icon } from "@r1os/icons";
import { motion, useReducedMotion } from "motion/react";
import { Fragment, type ReactNode } from "react";
import { Link } from "react-router";
import { after, reveal, STAGGER, word } from "../site/motion";

/** A line's words, each rising in turn after `from` words' worth of delay. Under a parent with `reveal`. */
export function Words({ text, from = 0 }: { text: string; from?: number }) {
  const words = text.split(/\s+/).filter(Boolean);
  return (
    <>
      <span className="sr">{text}</span>
      <span aria-hidden>
        {words.map((w, i) => (
          <Fragment key={i}>
            {i > 0 && " "}
            <motion.span className="head-word" variants={word} custom={from + i}>
              {w}
            </motion.span>
          </Fragment>
        ))}
      </span>
    </>
  );
}

/** How many words a head's title has, for timing what follows it. */
const count = (title: ReactNode) => (typeof title === "string" ? title.split(/\s+/).filter(Boolean).length : 1);

export function Head({ title, sub, link }: { title: ReactNode; sub: ReactNode; link?: { to: string; label: string } }) {
  const reduce = useReducedMotion();
  const n = count(title);
  return (
    <motion.header className="head" {...(reduce ? {} : reveal)}>
      <h2>{typeof title === "string" ? <Words text={title} /> : <motion.span variants={after(0)} style={{ display: "inline-block" }}>{title}</motion.span>}</h2>
      <motion.p variants={after(n * STAGGER + 0.1)}>{sub}</motion.p>
      {link && (
        <motion.div variants={after(n * STAGGER + 0.18, 10)}>
          <Link to={link.to} className="head-link">
            {link.label} <Icon name="arrow-right" weight="bold" size={16} />
          </Link>
        </motion.div>
      )}
    </motion.header>
  );
}
