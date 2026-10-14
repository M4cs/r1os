// An agent uses your Android apps (ADR-0061, ADR-0063; T-113, T-137): mochi opens a food app on a screen of its own,
// shown small and live in its chat with its pointer gliding to every tap; the final step waits on a card for your yes,
// the wheel picks "allow" and a click answers. The app ("forkful") is made up and drawn as plain shapes, and the story
// is sped up: the real thing takes minutes.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion } from "motion/react";
import { type ReactNode, useEffect, useId, useRef } from "react";
import { colorOf } from "@companion/theme";
import { Character } from "@shell/ui/Character";
import { member } from "../crew";
import { Screen, useDevice } from "../R1";
import { Bubble, ChatHead, Dots, fill, INK, pop, T, useSteps } from "../ui";

/** When each step starts (ms), and what it is. The section on the home page lights its points by these. */
const AT = [250, 600, 950, 1350, 2000, 2550, 3000, 3450, 4350, 4750, 5000, 5350, 5800];
// 1 asked · 2 thinking · 3 the app opens · 4 types in the search · 5 taps a dish · 6 adds it · 7 opens the cart ·
// 8 the final step asks · 9 the wheel picks allow · 10 the button goes down · 11 allowed · 12 places it · 13 done

/** How long to show the story: its last step, and a beat to read the reply. */
export const APP_WINDOW_MS = 7600;

// The preview card in the chat (AppPreview.kt): the window's picture on black, as tall as the R1's own screen would be,
// narrowed to the 600×1200 window's shape, and under it the working dot and "mochi · forkful".
const BOX_W = 128;
const BOX_H = 176;
const APP_W = 88;
const APP_X = 4 + (BOX_W - APP_W) / 2;
const APP_Y = 4;
/** How far the chat's messages lift to clear the preview. */
const LIFT = 4 + BOX_H + 5 + 15 + 6 + 10;

// The made-up app's own look: light paper, a mustard accent, food as warm blocks.
const APP = { bg: "#f6f5f1", ink: "#1d1e22", soft: "#e7e5df", muted: "#a5a39c", brand: "#f2a33a" };
const FOOD = ["linear-gradient(135deg,#f0b46a,#c8643c)", "linear-gradient(135deg,#9cc77f,#4f8a4a)", "linear-gradient(135deg,#f3d27a,#d98f3a)", "linear-gradient(135deg,#e88b6b,#a8423a)"];

// The agent's arrow (PointerView.kt): a classic pointer in a 20-unit box, its tip at the top left.
const ARROW = "M1.5 1.5 L1.5 16.5 L5.6 12.9 L8.4 18.8 L11 17.6 L8.3 11.8 L13.6 11.8 Z";
const ARROW_DP = 14;
const GLIDE = 0.32;
const glide = { duration: GLIDE, ease: [0.42, 0, 0.58, 1] } as const;

/** Where the pointer goes at each step, in the app's own dp; it presses on arrival at the steps in PRESSES. */
const TARGETS: Record<number, { x: number; y: number }> = {
  4: { x: 40, y: 27 },
  5: { x: 46, y: 36 },
  6: { x: 44, y: 159 },
  7: { x: 77, y: 11 },
  12: { x: 44, y: 159 },
};
const PRESSES = [4, 5, 6, 7, 12];

/** What the tool pill says at each step (ChatScreen.kt's labels), and how many steps came before. */
function pill(step: number): { label: string; more: number } {
  if (step >= 12) return { label: "tapping", more: 9 };
  if (step >= 8) return { label: "waiting for you", more: 8 };
  if (step >= 6) return { label: "working in the app", more: step === 6 ? 6 : 7 };
  if (step === 5) return { label: "tapping", more: 4 };
  if (step === 4) return { label: "typing", more: 2 };
  return { label: "opening the app", more: 0 };
}

export function AppWindowScreen({ accent, onStep }: { accent: string; onStep?: (step: number) => void }) {
  const mochi = member("mochi");
  const agent = colorOf(mochi.color);
  const step = useSteps(AT);
  const { roll, hold } = useDevice();
  const told = useRef(onStep);
  useEffect(() => {
    told.current = onStep;
  });
  useEffect(() => {
    told.current?.(step);
  }, [step]);
  useEffect(() => {
    if (step === 9) roll(1);
    if (step === 10) hold(true);
    if (step === 11) hold(false);
  }, [step, roll, hold]);
  useEffect(() => () => hold(false), [hold]);

  const open = step >= 3;
  const asking = step >= 8 && step < 11;
  const p = pill(step);
  return (
    <Screen>
      <motion.div className="r1-layer" animate={{ x: asking ? -60 : 0, opacity: asking ? 0.35 : 1 }} transition={{ duration: 0.35, ease: [0.2, 0.8, 0.2, 1] }}>
        <ChatHead
          name="mochi"
          form={mochi.form}
          color={mochi.color}
          finish={mochi.finish}
          chip={
            <span style={{ display: "inline-flex", alignItems: "center", gap: 5, padding: "4px 10px", borderRadius: 999, background: T.card, fontSize: 13, fontWeight: 700, color: T.muted }}>
              <Icon name="fast-forward" weight="fill" size={13} /> sped up
            </span>
          }
        />
        <div style={{ position: "absolute", left: 12, right: 12, top: 62, bottom: 12, display: "flex", flexDirection: "column", justifyContent: "flex-end", overflow: "hidden" }}>
          <motion.div animate={{ y: open ? -LIFT : 0 }} transition={{ type: "spring", stiffness: 300, damping: 32 }} style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            {step >= 1 && (
              <Bubble mine color={accent}>
                get me sesame noodles on forkful
              </Bubble>
            )}
            {step === 2 && <Dots color={agent} />}
            {open && (
              <motion.div
                key={`${p.label}${p.more}`}
                initial={step === 3 ? { opacity: 0, y: 10, scale: 0.94 } : { scale: 1.06 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                transition={step === 3 ? pop : { duration: 0.16 }}
                style={{ alignSelf: "flex-start", padding: "6px 12px", borderRadius: 999, background: `${agent}24`, color: agent, fontSize: 14, fontWeight: 600, whiteSpace: "pre", transformOrigin: "0 50%" }}
              >
                {p.label}
                {p.more > 0 && <span style={{ opacity: 0.6 }}>{`  +${p.more}`}</span>}
              </motion.div>
            )}
            {step >= 13 && <Bubble color={agent}>ordered. sesame noodles, $14.80, about 20 min away.</Bubble>}
          </motion.div>
        </div>
        <AnimatePresence>{open && <Preview step={step} agent={agent} />}</AnimatePresence>
        <AnimatePresence>
          {(step === 11 || step === 12) && (
            <motion.div initial={{ opacity: 0, y: 12, scale: 0.94 }} animate={{ opacity: 1, y: 0, scale: 1 }} exit={{ opacity: 0, y: 6 }} transition={pop} style={{ position: "absolute", right: 14, bottom: 18, display: "flex", alignItems: "center", gap: 7, padding: "9px 14px", borderRadius: 999, background: T.card, fontSize: 15, fontWeight: 700, color: T.fg }}>
              <Icon name="check" weight="bold" size={15} style={{ color: agent }} /> allowed
            </motion.div>
          )}
        </AnimatePresence>
      </motion.div>
      <AnimatePresence>{asking && <Ask step={step} accent={accent} agent={agent} />}</AnimatePresence>
    </Screen>
  );
}

/** The app, small and live at the bottom left of the chat, with the agent's pointer over it. */
function Preview({ step, agent }: { step: number; agent: string }) {
  const working = step < 13;
  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.92 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.9 }}
      transition={pop}
      style={{ position: "absolute", left: 10, bottom: 10, width: BOX_W + 8, padding: "4px 4px 6px", borderRadius: 16, background: T.surface, boxShadow: "0 10px 24px rgba(0,0,0,.55)", transformOrigin: "0 100%" }}
    >
      <div style={{ position: "relative", width: BOX_W, height: BOX_H, borderRadius: 12, background: "#000", overflow: "hidden" }}>
        <div style={{ position: "absolute", left: APP_X - 4, top: 0, width: APP_W, height: BOX_H, background: APP.bg, overflow: "hidden" }}>
          <Picture step={step} />
        </div>
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 6, marginTop: 5, paddingLeft: 3 }}>
        <motion.span animate={{ background: working ? agent : T.muted }} transition={{ duration: 0.3 }} style={{ width: 7, height: 7, borderRadius: 4, flex: "none" }} />
        <span style={{ fontSize: 12, fontWeight: 700, color: T.fg, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>mochi · forkful</span>
      </div>
      <Pointer step={step} color={agent} />
    </motion.div>
  );
}

/** The agent's arrow: it glides to each target, presses with a ripple, and fades once the work is done. */
function Pointer({ step, color }: { step: number; color: string }) {
  const reached = Object.keys(TARGETS).map(Number).filter((s) => s <= step);
  const at = TARGETS[reached.length ? Math.max(...reached) : 4]!;
  const press = PRESSES.filter((s) => s <= step).pop() ?? 0;
  const shown = step >= 4 && step < 13;
  // Before its first tap it waits below and to the right, out of sight, and comes in from there.
  const x = APP_X + at.x + (step < 4 ? 46 : 0);
  const y = APP_Y + at.y + (step < 4 ? 60 : 0);
  const ring = ARROW_DP * 2.2;
  return (
    <motion.div
      initial={{ x, y, opacity: 0 }}
      animate={{ x, y, opacity: shown ? 1 : 0 }}
      transition={{ x: glide, y: glide, opacity: shown ? { duration: 0.2 } : { duration: 0.4, delay: 0.6 } }}
      style={{ position: "absolute", left: 0, top: 0, width: ARROW_DP, height: ARROW_DP, pointerEvents: "none" }}
    >
      {press > 0 && (
        <motion.div
          key={`ring${press}`}
          initial={{ opacity: 0, scale: 0.3 }}
          animate={{ opacity: [0.7, 0], scale: [0.3, 1] }}
          transition={{ delay: GLIDE + 0.08, duration: 0.38, ease: [0.2, 0.8, 0.2, 1] }}
          style={{ position: "absolute", left: 1 - ring / 2, top: 1 - ring / 2, width: ring, height: ring, borderRadius: "50%", border: `${ring * 0.08}px solid ${color}` }}
        />
      )}
      <motion.svg
        key={`arrow${press}`}
        viewBox="0 0 20 20"
        width={ARROW_DP}
        height={ARROW_DP}
        animate={{ scale: press ? [1, 1, 0.78, 1] : 1 }}
        transition={{ duration: GLIDE + 0.22, times: [0, GLIDE / (GLIDE + 0.22), (GLIDE + 0.08) / (GLIDE + 0.22), 1] }}
        style={{ position: "absolute", inset: 0, overflow: "visible", transformOrigin: "7% 7%" }}
      >
        <path d={ARROW} fill="#000" opacity={0.45} transform="translate(1.2 1.2)" />
        <path d={ARROW} fill={color} stroke={T.fg} strokeWidth={1.4} strokeLinejoin="round" />
      </motion.svg>
    </motion.div>
  );
}

// ---- the app's picture: a made-up food app, as plain shapes in its 88×176 dp ----------------------------------------

function Box({ x, y, w, h, bg, r = 2, children }: { x: number; y: number; w: number; h: number; bg?: string; r?: number; children?: ReactNode }) {
  return <div style={{ position: "absolute", left: x, top: y, width: w, height: h, borderRadius: r, background: bg }}>{children}</div>;
}

function Words({ x, y, size, color = APP.ink, weight = 700, children }: { x: number; y: number; size: number; color?: string; weight?: number; children: ReactNode }) {
  return <div style={{ position: "absolute", left: x, top: y, fontSize: size, fontWeight: weight, lineHeight: 1, color, whiteSpace: "nowrap", letterSpacing: "-0.01em" }}>{children}</div>;
}

/** The cart button at the top right, with a count once something's in it. */
function CartButton({ count }: { count: number }) {
  return (
    <Box x={71} y={4} w={13} h={13} r={7} bg={APP.soft}>
      <div style={{ position: "absolute", inset: 0, display: "grid", placeItems: "center", color: APP.ink }}>
        <Icon name="shopping-cart" weight="bold" size={8} />
      </div>
      {count > 0 && (
        <motion.div initial={{ scale: 0 }} animate={{ scale: 1 }} transition={pop} style={{ position: "absolute", right: -2, top: -2, width: 7, height: 7, borderRadius: 4, background: APP.brand, display: "grid", placeItems: "center", fontSize: 5, fontWeight: 800, color: APP.ink }}>
          {count}
        </motion.div>
      )}
    </Box>
  );
}

function Search({ typed, y }: { typed: boolean; y: number }) {
  return (
    <Box x={5} y={y} w={62} h={13} r={7} bg={APP.soft}>
      <div style={{ position: "absolute", left: 4, top: 3, color: APP.muted }}>
        <Icon name="magnifying-glass" weight="bold" size={7} />
      </div>
      {typed ? (
        <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.45, duration: 0.2 }}>
          <Words x={13} y={3.5} size={6.5} weight={600}>
            sesame noodles
          </Words>
        </motion.div>
      ) : (
        <Box x={13} y={5} w={26} h={3} bg="#cfccc4" />
      )}
    </Box>
  );
}

function Home({ typed }: { typed: boolean }) {
  return (
    <>
      <Words x={6} y={6} size={8} color={APP.brand} weight={800}>
        forkful
      </Words>
      <CartButton count={0} />
      <Search typed={typed} y={22} />
      {[0, 1, 2, 3].map((i) => (
        <Box key={i} x={6 + i * 20} y={42} w={16} h={16} r={5} bg={FOOD[i]} />
      ))}
      <Box x={6} y={66} w={30} h={4} bg={APP.ink} />
      <Box x={6} y={75} w={76} h={40} r={6} bg={FOOD[1]} />
      <Box x={6} y={119} w={44} h={4} bg={APP.ink} />
      <Box x={6} y={126} w={28} h={3} bg={APP.muted} />
      <Box x={6} y={136} w={76} h={40} r={6} bg={FOOD[3]} />
    </>
  );
}

function Results({ cart }: { cart: number }) {
  return (
    <>
      <Search typed y={4} />
      <CartButton count={cart} />
      {[0, 1, 2, 3, 4].map((i) => (
        <div key={i}>
          <Box x={6} y={26 + i * 30} w={22} h={22} r={5} bg={FOOD[i % 4]} />
          <Box x={33} y={30 + i * 30} w={i === 0 ? 44 : 36 - (i % 2) * 8} h={4} bg={APP.ink} />
          <Box x={33} y={37 + i * 30} w={30} h={3} bg={APP.muted} />
          <Box x={33} y={43 + i * 30} w={14} h={3} bg={APP.ink} />
        </div>
      ))}
    </>
  );
}

function Sheet() {
  return (
    <>
      <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} style={{ position: "absolute", inset: 0, background: "rgba(0,0,0,.32)" }} />
      <motion.div initial={{ y: 140 }} animate={{ y: 0 }} exit={{ y: 140 }} transition={{ type: "spring", stiffness: 420, damping: 36 }} style={{ position: "absolute", left: 0, right: 0, top: 40, bottom: 0, borderRadius: "10px 10px 0 0", background: APP.bg }}>
        <Box x={6} y={7} w={76} h={36} r={6} bg={FOOD[0]} />
        <Words x={6} y={49} size={7.5}>
          sesame noodles
        </Words>
        <Box x={6} y={60} w={18} h={4} bg={APP.ink} />
        {[0, 1].map((i) => (
          <div key={i}>
            <Box x={6} y={72 + i * 11} w={6} h={6} r={3} bg={i === 0 ? APP.brand : APP.soft} />
            <Box x={16} y={73.5 + i * 11} w={36 - i * 8} h={3} bg={APP.muted} />
          </div>
        ))}
        <Box x={6} y={112} w={76} h={16} r={8} bg={APP.brand}>
          <Words x={0} y={5} size={6.5} weight={800}>
            <span style={{ display: "block", width: 76, textAlign: "center" }}>add to cart</span>
          </Words>
        </Box>
      </motion.div>
    </>
  );
}

function Cart() {
  return (
    <>
      <div style={{ position: "absolute", left: 4, top: 5, color: APP.ink }}>
        <Icon name="caret-left" weight="bold" size={8} />
      </div>
      <Words x={14} y={6} size={8} weight={800}>
        your cart
      </Words>
      <Box x={6} y={22} w={18} h={18} r={5} bg={FOOD[0]} />
      <Words x={29} y={25} size={6.5} weight={600}>
        sesame noodles
      </Words>
      <Box x={29} y={34} w={12} h={3} bg={APP.muted} />
      <Box x={6} y={48} w={76} h={1} bg={APP.soft} />
      <Box x={6} y={55} w={26} h={3} bg={APP.muted} />
      <Box x={66} y={55} w={16} h={3} bg={APP.muted} />
      <Box x={6} y={63} w={20} h={3} bg={APP.muted} />
      <Box x={70} y={63} w={12} h={3} bg={APP.muted} />
      <Words x={6} y={73} size={7.5}>
        total
      </Words>
      <Words x={58} y={73} size={7.5}>
        $14.80
      </Words>
      <Box x={6} y={88} w={76} h={1} bg={APP.soft} />
      {["map-pin", "clock", "credit-card"].map((icon, i) => (
        <div key={icon}>
          <div style={{ position: "absolute", left: 6, top: 95 + i * 12, color: APP.muted }}>
            <Icon name={icon} weight="bold" size={7} />
          </div>
          <Box x={16} y={97 + i * 12} w={[34, 28, 40][i]!} h={3} bg={APP.muted} />
        </div>
      ))}
      <Box x={6} y={152} w={76} h={16} r={8} bg={APP.brand}>
        <Words x={0} y={5} size={6.5} weight={800}>
          <span style={{ display: "block", width: 76, textAlign: "center" }}>place order</span>
        </Words>
      </Box>
    </>
  );
}

function Placed() {
  return (
    <>
      <motion.div initial={{ scale: 0.4, opacity: 0 }} animate={{ scale: 1, opacity: 1 }} transition={pop} style={{ position: "absolute", left: 27, top: 44, width: 34, height: 34, borderRadius: 17, background: APP.brand, display: "grid", placeItems: "center", color: APP.ink }}>
        <Icon name="check" weight="bold" size={18} />
      </motion.div>
      <Words x={0} y={88} size={8} weight={800}>
        <span style={{ display: "block", width: APP_W, textAlign: "center" }}>order placed</span>
      </Words>
      <Words x={0} y={101} size={6.5} weight={600} color={APP.muted}>
        <span style={{ display: "block", width: APP_W, textAlign: "center" }}>about 20 min</span>
      </Words>
      <Box x={14} y={120} w={60} h={4} r={2} bg={APP.soft}>
        <motion.div initial={{ scaleX: 0 }} animate={{ scaleX: 0.2 }} transition={{ delay: 0.3, duration: 0.5 }} style={{ width: "100%", height: "100%", borderRadius: 2, background: APP.brand, transformOrigin: "0 50%" }} />
      </Box>
    </>
  );
}

/** The app's screen at this step: a page, and the item's sheet over the results while it's open. */
function Picture({ step }: { step: number }) {
  const page = step >= 13 ? "placed" : step >= 8 ? "cart" : step >= 5 ? "results" : "home";
  return (
    <>
      <AnimatePresence initial={false}>
        <motion.div key={page} initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.18 }} style={{ position: "absolute", inset: 0, background: APP.bg }}>
          {page === "home" && <Home typed={step >= 4} />}
          {page === "results" && <Results cart={step >= 7 ? 1 : 0} />}
          {page === "cart" && <Cart />}
          {page === "placed" && <Placed />}
        </motion.div>
      </AnimatePresence>
      <AnimatePresence>{step === 6 && <Sheet key="sheet" />}</AnimatePresence>
    </>
  );
}

// ---- the final step's card (ApprovalScreen.kt; r1d's commit_ask) ----------------------------------------------------

function AskRow({ icon, label, on, accent, agent, fillId }: { icon: string; label: string; on: boolean; accent: string; agent: string; fillId: string }) {
  return (
    <div style={{ position: "relative", height: 54, display: "flex", alignItems: "center", gap: 12, padding: "0 10px" }}>
      {on && <motion.div layoutId={fillId} initial={{ opacity: 0, scale: 0.94 }} animate={{ opacity: 1, scale: 1 }} transition={fill} style={{ position: "absolute", inset: 0, borderRadius: 12, background: accent }} />}
      <div style={{ position: "relative", width: 38, height: 38, borderRadius: 10, display: "grid", placeItems: "center", background: on ? "rgba(0,0,0,.12)" : `${agent}2e`, color: on ? INK : agent }}>
        <Icon name={icon} weight="bold" size={20} />
      </div>
      <div style={{ position: "relative", fontSize: 21, fontWeight: 700, color: on ? INK : T.fg }}>{label}</div>
    </div>
  );
}

function Ask({ step, accent, agent }: { step: number; accent: string; agent: string }) {
  const mochi = member("mochi");
  const fillId = `allow-fill-${useId()}`;
  return (
    <motion.div className="r1-layer" initial={{ x: 360 }} animate={{ x: 0 }} exit={{ x: 360 }} transition={{ duration: 0.35, ease: [0.2, 0.8, 0.2, 1] }} style={{ background: "#000", boxShadow: "-12px 0 30px rgba(0,0,0,.6)" }}>
      <div style={{ position: "absolute", left: 16, top: 10, fontSize: 28, fontWeight: 700, color: T.fg }}>allow this?</div>
      <div style={{ position: "absolute", left: 10, top: 48, display: "flex", alignItems: "center", gap: 8 }}>
        <Character form={mochi.form} color={mochi.color} finish={mochi.finish} state="idle" size={52} />
        <div style={{ fontSize: 18, fontWeight: 650, color: T.muted }}>mochi wants to</div>
      </div>
      <div style={{ position: "absolute", left: 16, right: 16, top: 106, fontSize: 24, fontWeight: 750, lineHeight: 1.1, color: T.fg }}>tap “place order” in forkful</div>
      <div style={{ position: "absolute", left: 14, right: 14, top: 168, padding: "12px 14px 12px", borderRadius: 16, background: T.card }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", fontSize: 17, fontWeight: 600, color: T.fg }}>
          sesame noodles <span style={{ fontSize: 15, color: T.muted }}>×1</span>
        </div>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "baseline", marginTop: 6, fontSize: 22, fontWeight: 800, color: T.fg }}>
          total <span>$14.80</span>
        </div>
        <div style={{ marginTop: 6, fontSize: 15, fontWeight: 500, lineHeight: 1.3, color: T.muted }}>to home</div>
        <div style={{ fontSize: 15, fontWeight: 500, lineHeight: 1.3, color: T.muted }}>about 20 min</div>
      </div>
      <div style={{ position: "absolute", left: 10, right: 10, top: 296, display: "flex", flexDirection: "column", gap: 2 }}>
        <AskRow icon="check" label="allow" on={step >= 9} accent={accent} agent={agent} fillId={fillId} />
        <AskRow icon="x" label="don't allow" on={false} accent={accent} agent={agent} fillId={fillId} />
      </div>
      <div style={{ position: "absolute", left: 16, right: 16, top: 414, fontSize: 14, fontWeight: 600, color: T.muted }}>or hold and say “allow” or “don't”</div>
    </motion.div>
  );
}
