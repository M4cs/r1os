// Linking an R1 (ADR-0048, docs/specs/companion.md §3): this computer's QR, big; the R1 reads it with its camera; then
// "link R1?" with the four digits both screens show, and a yes here pins it.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import { Character } from "@shell/ui/Character";
import { type LinkState, link } from "../link";
import { page, soft, spring } from "../motion";

export function Pair({ state, first, onDone, onInstall }: { state: LinkState; first: boolean; onDone: () => void; onInstall: () => void }) {
  const [err, setErr] = useState<string | null>(null);
  const showing = state.pairing;
  const asking = state.asking;

  // A code as soon as the page opens (and a new one when it runs out).
  useEffect(() => {
    if (!showing && !asking) link.pair().catch((e) => setErr(String(e)));
  }, [showing === null, asking === null]); // eslint-disable-line react-hooks/exhaustive-deps

  const left = useCountdown(showing?.left_s ?? 0, showing?.uri);
  // Ran out: a new one.
  useEffect(() => {
    if (showing && left === 0) link.pair().catch((e) => setErr(String(e)));
  }, [left, showing]);

  return (
    <motion.section className="pair" {...page}>
      <AnimatePresence mode="wait">
        {asking ? (
          <motion.div key="ask" className="ask" initial={{ opacity: 0, scale: 0.94 }} animate={{ opacity: 1, scale: 1, transition: spring }} exit={{ opacity: 0, scale: 0.96 }}>
            <Character form="orb" color="orange" state="listening" size={120} />
            <h1>link {asking.name}?</h1>
            <p className="muted">check it shows the same code</p>
            <div className="code mono">
              {asking.code.split("").map((d, i) => (
                <motion.span key={i} initial={{ y: 18, opacity: 0 }} animate={{ y: 0, opacity: 1 }} transition={{ ...spring, delay: 0.06 * i }}>
                  {d}
                </motion.span>
              ))}
            </div>
            <div className="row">
              <motion.button className="btn primary" whileTap={{ scale: 0.96 }} onClick={() => link.answer(true)}>
                link
              </motion.button>
              <motion.button className="btn" whileTap={{ scale: 0.96 }} onClick={() => link.answer(false)}>
                not now
              </motion.button>
            </div>
          </motion.div>
        ) : (
          <motion.div key="qr" className="pair-grid" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0, scale: 0.97 }}>
            <div className="qr-side">
              <motion.div className="qr-card" initial={{ rotate: -2, scale: 0.92, opacity: 0 }} animate={{ rotate: 0, scale: 1, opacity: 1, transition: soft }}>
                <div className="qr-glow" />
                {showing ? <div className="qr" dangerouslySetInnerHTML={{ __html: showing.svg }} /> : <div className="qr empty" />}
              </motion.div>
              <div className="qr-left muted">{showing ? `good for ${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")}` : "making a code…"}</div>
            </div>
            <div className="steps">
              <h1>{first ? "your r1, on your computer" : "link another r1"}</h1>
              <p className="lede muted">manage it on a big screen, sign it in, and later lend its agents this computer.</p>
              <ol>
                <Step n={1} icon="gear-six">on your r1, open <b>settings › computers</b></Step>
                <Step n={2} icon="camera">choose <b>link a computer</b></Step>
                <Step n={3} icon="scan">point it at this code</Step>
              </ol>
              {err && <p className="err">{err}</p>}
              {!first && (
                <button className="btn ghost" onClick={() => { void link.stop(); onDone(); }}>
                  cancel
                </button>
              )}
              {first && (
                <button className="btn ghost" onClick={onInstall}>
                  <Icon name="usb" weight="bold" size={16} /> r1os isn't on your r1 yet? install it over usb
                </button>
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.section>
  );
}

function Step({ n, icon, children }: { n: number; icon: string; children: React.ReactNode }) {
  return (
    <motion.li initial={{ opacity: 0, x: 12 }} animate={{ opacity: 1, x: 0 }} transition={{ ...soft, delay: 0.08 * n }}>
      <span className="step-icon">
        <Icon name={icon} weight="bold" size={18} />
      </span>
      <span>{children}</span>
    </motion.li>
  );
}

/** Seconds left on the code, counting down between the core's updates. */
function useCountdown(from: number, key?: string): number {
  const [left, setLeft] = useState(from);
  useEffect(() => {
    setLeft(from);
    if (!key) return;
    const t = setInterval(() => setLeft((s) => Math.max(0, s - 1)), 1000);
    return () => clearInterval(t);
  }, [from, key]);
  return left;
}
