// The window's motion: springs, as on the R1 (retargetable, a little overshoot, settling fast).

export const spring = { type: "spring", stiffness: 420, damping: 34, mass: 0.9 } as const;
export const soft = { type: "spring", stiffness: 260, damping: 28 } as const;

/** A page coming in and going out. */
export const page = {
  initial: { opacity: 0, y: 14, filter: "blur(6px)" },
  animate: { opacity: 1, y: 0, filter: "blur(0px)", transition: soft },
  exit: { opacity: 0, y: -8, filter: "blur(4px)", transition: { duration: 0.14 } },
};
