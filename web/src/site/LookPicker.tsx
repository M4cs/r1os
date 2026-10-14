// The look: dark, light or the system's, and an accent from the R1's palette. The whole site takes it, as the companion
// takes the R1's look.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { ACCENTS, type Mode, useLook } from "./look";
import { spring } from "./motion";

export function LookPicker() {
  const { look, setLook } = useLook();
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => !box.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("pointerdown", away);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", away);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);
  return (
    <div ref={box} style={{ position: "relative" }}>
      <button className="look-btn" aria-label="change the look" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <Icon name="palette" weight="bold" size={19} />
      </button>
      <AnimatePresence>
        {open && (
          <motion.div
            className="look-pop"
            initial={{ opacity: 0, y: -6, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1, transition: spring }}
            exit={{ opacity: 0, y: -4, scale: 0.98, transition: { duration: 0.12 } }}
            style={{ transformOrigin: "top right" }}
          >
            <h4>mode</h4>
            <Seg<Mode> value={look.mode} options={["system", "dark", "light"]} onChange={(mode) => setLook({ mode })} id="mode" />
            <h4>accent</h4>
            <div className="swatches">
              {Object.entries(ACCENTS).map(([name, c]) => (
                <button key={name} className="swatch" style={{ background: c }} aria-label={name} title={name} onClick={() => setLook({ accent: c })}>
                  {look.accent === c && <motion.span className="swatch-ring" layoutId="accent-ring" style={{ borderColor: c }} transition={spring} />}
                </button>
              ))}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

function Seg<T extends string>({ value, options, onChange, id, wide }: { value: T; options: readonly T[]; onChange: (v: T) => void; id: string; wide?: boolean }) {
  return (
    <div className={`seg ${wide ? "wide" : ""}`} role="radiogroup">
      {options.map((o) => (
        <button key={o} role="radio" aria-checked={o === value} className={`seg-item ${o === value ? "on" : ""}`} onClick={() => onChange(o)}>
          {o === value && <motion.span className="seg-fill" layoutId={`seg-${id}`} transition={spring} />}
          <span>{o}</span>
        </button>
      ))}
    </div>
  );
}
