// The companion's window (docs/specs/companion.md §5): its own title bar, your R1s down the side, and the R1 you
// picked (or pairing a new one). It looks like the R1: its accent, mode and background, its agents, springs. The switch
// at the top left turns it into chats with your agents (ADR-0051).

import { Icon } from "@r1os/icons";
import { AnimatePresence, LayoutGroup, motion } from "motion/react";
import { useEffect, useState } from "react";
import { Character } from "@shell/ui/Character";
import { type R1Info, link, onPairRequest, useLink, windowDo } from "./link";
import { Backdrop } from "./views/Backdrop";
import { ChatsPanel } from "./views/Chats";
import { Computer } from "./views/Computer";
import { Pair } from "./views/Pair";
import { PAGES, type Page, R1View, useR1 } from "./views/R1View";
import { spring } from "./motion";

export function App() {
  const state = useLink();
  const [pairing, setPairing] = useState(false);
  const [mine, setMine] = useState(false);
  const [picked, setPicked] = useState<string | null>(null);
  const [page, setPage] = useState<Page>("overview");
  // Managing the R1, or chatting with its agents: the switch at the top left, remembered.
  const [mode, setModeState] = useState<"manage" | "chats">(() => (localStorage.getItem("mode") === "chats" ? "chats" : "manage"));
  const setMode = (m: "manage" | "chats") => {
    setModeState(m);
    try {
      localStorage.setItem("mode", m);
    } catch {
      // No storage: it opens on manage.
    }
  };
  const r1s = state?.r1s ?? [];
  const selected = r1s.find((r) => r.id === picked) ?? r1s[0] ?? null;
  const r1 = useR1(selected);

  // The tray's "link an R1".
  useEffect(() => {
    let off: (() => void) | undefined;
    onPairRequest(() => setPairing(true)).then((f) => (off = f));
    return () => off?.();
  }, []);

  // A new R1 just linked: show it.
  const [known, setKnown] = useState<string[]>([]);
  useEffect(() => {
    const ids = r1s.map((r) => r.id);
    const fresh = ids.find((id) => known.length > 0 && !known.includes(id));
    if (fresh) {
      setPicked(fresh);
      setPairing(false);
    }
    if (ids.join() !== known.join()) setKnown(ids);
  }, [r1s, known]);

  const showPair = pairing || (state !== null && r1s.length === 0) || !!state?.asking;
  const unread = r1.agents.reduce((n, a) => n + (a.unread ?? 0), 0);
  const chats = mode === "chats" && !showPair;

  return (
    <div className="app">
      <Backdrop look={r1.look} />
      <header className="titlebar" data-tauri-drag-region>
        <div className="brand" data-tauri-drag-region>
          <span className="brand-dot" />
          r1os
        </div>
        <div className="mode-tabs">
          {(["manage", "chats"] as const).map((m) => (
            <button key={m} className={`mode-tab ${mode === m ? "on" : ""}`} onClick={() => setMode(m)}>
              {mode === m && <motion.span className="mode-tab-fill" layoutId="mode-fill" transition={spring} />}
              <Icon name={m === "manage" ? "sliders-horizontal" : "chats-circle"} weight={mode === m ? "fill" : "bold"} size={15} />
              <span>{m}</span>
              {m === "chats" && unread > 0 && <span className="badge">{unread}</span>}
            </button>
          ))}
        </div>
        {chats && r1s.length > 1 && (
          <div className="mode-r1s">
            {r1s.map((r) => (
              <button key={r.id} className={`part ${r.id === selected?.id ? "on" : ""}`} onClick={() => setPicked(r.id)}>
                {r.name}
              </button>
            ))}
          </div>
        )}
        <div className="grow" data-tauri-drag-region />
        <button className="winbtn" onClick={() => windowDo("minimize")} title="minimise">
          <Icon name="minus" size={16} />
        </button>
        <button className="winbtn" onClick={() => windowDo("toggleMaximize")} title="maximise">
          <Icon name="square" size={14} />
        </button>
        <button className="winbtn close" onClick={() => windowDo("close")} title="close (r1os keeps running in the tray)">
          <Icon name="x" size={16} />
        </button>
      </header>
      {chats ? (
        <div className="body chats-body">
          {state === null ? (
            <div className="center muted">starting…</div>
          ) : selected?.online ? (
            <ChatsPanel key={selected.id} r1={selected} data={r1} />
          ) : (
            <div className="center muted">{selected ? `${selected.name.toLowerCase()} is asleep: wake it to chat from here` : "link an r1 to chat with its agents"}</div>
          )}
        </div>
      ) : (
      <div className="body">
        <aside className="side">
          <div className="side-label">your r1s</div>
          <LayoutGroup id="side">
            {r1s.map((r, i) => (
              <Device
                key={r.id}
                r1={r}
                agent={r.id === selected?.id ? r1.focus : null}
                on={!showPair && !mine && r.id === selected?.id}
                index={i}
                onClick={() => {
                  setPicked(r.id);
                  setPairing(false);
                  setMine(false);
                  if (state?.pairing) void link.stop();
                }}
              />
            ))}
            <AnimatePresence initial={false}>
              {selected?.online && !showPair && !mine && (
                <motion.nav
                  key={`pages-${selected.id}`}
                  className="side-pages"
                  initial={{ opacity: 0, height: 0 }}
                  animate={{ opacity: 1, height: "auto", transition: spring }}
                  exit={{ opacity: 0, height: 0, transition: { duration: 0.18 } }}
                  layout
                >
                  {PAGES.map((p) => (
                    <button key={p.id} className={`side-page ${p.id === page ? "on" : ""}`} onClick={() => setPage(p.id)}>
                      {p.id === page && <motion.span className="side-page-fill" layoutId="page-fill" transition={spring} />}
                      <Icon name={p.icon} weight={p.id === page ? "fill" : "bold"} size={17} />
                      <span>{p.id}</span>
                    </button>
                  ))}
                </motion.nav>
              )}
            </AnimatePresence>
            <motion.button
              className={`device add ${showPair ? "on" : ""}`}
              onClick={() => {
                setPairing(true);
                setMine(false);
              }}
              whileTap={{ scale: 0.97 }}
              layout
              transition={spring}
            >
              {showPair && <motion.div className="fill" layoutId="side-fill" transition={spring} />}
              <span className="device-icon">
                <Icon name="plus" weight="bold" size={20} />
              </span>
              <span className="device-text">
                <span className="device-name">link an r1</span>
                <span className="device-sub">scan a code with its camera</span>
              </span>
            </motion.button>
          </LayoutGroup>
          <div className="grow" />
          {state && (
            <motion.button className={`device me ${mine ? "on" : ""}`} onClick={() => { setMine(true); setPairing(false); }} whileTap={{ scale: 0.97 }} layout transition={spring}>
              {mine && <motion.div className="fill" layoutId="side-fill" transition={spring} />}
              <span className="device-icon small"><Icon name="desktop" weight="bold" size={17} /></span>
              <span className="device-text">
                <span className="device-name">this computer</span>
                <span className="device-sub">{state.me.name.toLowerCase()}</span>
              </span>
            </motion.button>
          )}
        </aside>
        <motion.main className="main" layoutScroll>
          <AnimatePresence mode="wait">
            {state === null ? (
              <motion.div key="wait" className="center muted" initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
                starting…
              </motion.div>
            ) : mine ? (
              <Computer key="me" state={state} />
            ) : showPair ? (
              <Pair
                key="pair"
                state={state}
                first={r1s.length === 0}
                onDone={() => setPairing(false)}
              />
            ) : selected ? (
              <R1View key={`r1-${selected.id}`} r1={selected} data={r1} page={page} onPage={setPage} />
            ) : null}
          </AnimatePresence>
        </motion.main>
      </div>
      )}
    </div>
  );
}

function Device({ r1, agent, on, index, onClick }: {
  r1: R1Info;
  agent: { form: string; color: string; finish: string; eyes?: string; wear?: string[] } | null;
  on: boolean;
  index: number;
  onClick: () => void;
}) {
  return (
    <motion.button
      className={`device ${on ? "on" : ""}`}
      onClick={onClick}
      initial={{ opacity: 0, x: -12 }}
      animate={{ opacity: 1, x: 0 }}
      transition={{ ...spring, delay: index * 0.04 }}
      whileTap={{ scale: 0.97 }}
      layout
    >
      {on && <motion.div className="fill" layoutId="side-fill" transition={spring} />}
      <span className="device-char">
        <Character form={agent?.form ?? "orb"} color={agent?.color ?? "orange"} finish={agent?.finish ?? "soft"} eyes={agent?.eyes} wear={agent?.wear} state={r1.online ? "idle" : "sleeping"} size={40} />
      </span>
      <span className="device-text">
        <span className="device-name">{r1.name}</span>
        <span className="device-sub">
          <span className={`dot ${r1.online ? "awake" : ""}`} />
          {r1.online ? `awake · ${r1.path === "relay" ? "through a relay" : r1.path === "direct" ? "direct" : "connecting"}` : "asleep"}
        </span>
      </span>
    </motion.button>
  );
}
