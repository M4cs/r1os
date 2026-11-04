// Every page's frame: the bar at the top (r1os, the pages, the look, the source), the page, and the footer. The page
// you're on takes the accent's soft fill, and the fill springs between pages, as the device's focus does between rows.
// On the home page the fill follows the part you're reading, and a thin accent line under the bar shows how far down
// you are. A new page rises in softly. With reduced motion nothing moves; it's simply there.

import { Icon } from "@r1os/icons";
import { AnimatePresence, LayoutGroup, motion, MotionConfig, useReducedMotion, useScroll, useSpring } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { Link, Outlet, ScrollRestoration, useLocation, useNavigationType } from "react-router";
import { arrive, spring } from "./motion";
import { LookPicker } from "./LookPicker";
import { DISCORD, ISSUES, REPO } from "./repo";

const PAGES = [
  { to: "/#android", label: "android", hash: "#android" },
  { to: "/#agents", label: "agents", hash: "#agents" },
  { to: "/#everything", label: "features", hash: "#everything" },
  { to: "/#dock", label: "dock", hash: "#dock" },
  { to: "/#speed", label: "speed", hash: "#speed" },
  { to: "/docs", label: "docs" },
] as const;

const SPIED = new Set<string>(PAGES.flatMap((p) => ("hash" in p ? [p.hash.slice(1)] : [])));

/** The home page's section in the middle of the screen, if the nav has it: an observer on a thin band across the
 *  middle, so nothing runs while the page is still. It never touches the address or the history. */
function useSpy(on: boolean): string | null {
  const [spy, setSpy] = useState<string | null>(null);
  useEffect(() => {
    if (!on) {
      setSpy(null);
      return;
    }
    let io: IntersectionObserver | undefined;
    const raf = requestAnimationFrame(() => {
      const sections = [...document.querySelectorAll("main section[id]")];
      const inBand = new Set<Element>();
      io = new IntersectionObserver(
        (entries) => {
          for (const e of entries) {
            if (e.isIntersecting) inBand.add(e.target);
            else inBand.delete(e.target);
          }
          const at = sections.find((s) => inBand.has(s));
          setSpy(at && SPIED.has(at.id) ? at.id : null);
        },
        { rootMargin: "-42% 0px -52% 0px" },
      );
      for (const s of sections) io.observe(s);
    });
    return () => {
      cancelAnimationFrame(raf);
      io?.disconnect();
    };
  }, [on]);
  return spy;
}

/** How far down the home page you are: a thin accent line under the bar, moved by transform alone. */
function Progress() {
  const reduce = useReducedMotion();
  const { scrollYProgress } = useScroll();
  const smooth = useSpring(scrollYProgress, { stiffness: 320, damping: 40, restDelta: 0.0005 });
  return (
    <motion.div
      aria-hidden
      style={{ position: "absolute", left: 0, right: 0, bottom: -1, height: 2, background: "var(--accent)", transformOrigin: "0 50%", scaleX: reduce ? scrollYProgress : smooth, pointerEvents: "none" }}
    />
  );
}

export function Root() {
  const loc = useLocation();
  const reduce = useReducedMotion();
  const [scrolled, setScrolled] = useState(false);
  const [menu, setMenu] = useState(false);
  const [hover, setHover] = useState<string | null>(null);
  const bar = useRef<HTMLElement>(null);
  const sheet = useRef<HTMLElement>(null);
  // The first page is there at once; later ones rise in.
  const [settled, setSettled] = useState(false);
  useEffect(() => setSettled(true), []);
  useEffect(() => {
    const on = () => setScrolled(window.scrollY > 8);
    on();
    window.addEventListener("scroll", on, { passive: true });
    return () => window.removeEventListener("scroll", on);
  }, []);
  // The phone's menu closes as the look's picker does: on Escape, or a press anywhere outside it and the bar.
  useEffect(() => {
    if (!menu) return;
    const away = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!sheet.current?.contains(t) && !bar.current?.contains(t)) setMenu(false);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setMenu(false);
    document.addEventListener("pointerdown", away);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", away);
      document.removeEventListener("keydown", esc);
    };
  }, [menu]);
  // Every navigation closes the menu, and a link to a part of a page scrolls to it once it's drawn, even a link to the
  // address you're already on. Back or Forward to another page leaves the place to ScrollRestoration. With reduced
  // motion the scroll is a jump.
  const nav = useNavigationType();
  const lastPath = useRef(loc.pathname);
  useEffect(() => {
    setMenu(false);
    const samePage = lastPath.current === loc.pathname;
    lastPath.current = loc.pathname;
    if (!loc.hash || (nav === "POP" && !samePage)) return;
    const raf = requestAnimationFrame(() =>
      document.getElementById(decodeURIComponent(loc.hash.slice(1)))?.scrollIntoView({ behavior: reduce ? "auto" : "smooth", block: "start" }),
    );
    return () => cancelAnimationFrame(raf);
  }, [loc.key]); // eslint-disable-line react-hooks/exhaustive-deps

  const home = loc.pathname === "/";
  const spy = useSpy(home);
  const on = (p: (typeof PAGES)[number]) => ("hash" in p ? home && spy === p.hash.slice(1) : loc.pathname.startsWith(p.to));
  const top = loc.pathname.split("/")[1] ?? "";
  return (
    <MotionConfig reducedMotion="user">
      <div className="site">
        <a className="skip" href="#main">
          skip to the page
        </a>
        <header className={`top ${scrolled ? "scrolled" : ""}`} ref={bar}>
          <div className="wrap top-bar">
            <Link to="/" className="brand" aria-label="r1os home">
              <span className="brand-dot" />
              r1os
            </Link>
            <LayoutGroup id="nav">
              <nav className="nav wide" onMouseLeave={() => setHover(null)}>
                {PAGES.map((p) => (
                  <Link
                    key={p.label}
                    to={p.to}
                    className={`nav-item ${on(p) ? "on" : ""}`}
                    aria-current={on(p) ? ("hash" in p ? "location" : "page") : undefined}
                    onMouseEnter={() => setHover(p.label)}
                  >
                    {hover === p.label && !on(p) && <motion.span className="nav-hover" layoutId="nav-hover" transition={spring} />}
                    {on(p) && <motion.span className="nav-fill" layoutId="nav-fill" transition={spring} />}
                    <span>{p.label}</span>
                  </Link>
                ))}
              </nav>
            </LayoutGroup>
            <div className="grow" />
            <LookPicker />
            <a href={REPO} className="btn ghost small source-link">
              <Icon name="github-logo" weight="bold" size={16} />
              source
            </a>
            <a href={DISCORD} className="btn ghost small source-link icon-link" aria-label="discord" title="discord" style={{ marginLeft: 6 }}>
              <Icon name="discord-logo" weight="bold" size={16} />
            </a>
            <Link to="/docs/start" className="btn primary small" style={{ marginLeft: 6 }}>
              install
            </Link>
            <button className="menu-btn" aria-label="menu" aria-expanded={menu} onClick={() => setMenu((m) => !m)} style={{ marginLeft: 6 }}>
              <Icon name={menu ? "x" : "list"} weight="bold" size={20} />
            </button>
          </div>
          {home && <Progress />}
        </header>
        <AnimatePresence>
          {menu && (
            <motion.nav ref={sheet} className="sheet-nav" initial={{ opacity: 0, y: -6 }} animate={{ opacity: 1, y: 0, transition: spring }} exit={{ opacity: 0, y: -4, transition: { duration: 0.14 } }}>
              {PAGES.map((p) => (
                <Link key={p.label} to={p.to} className={on(p) ? "on" : ""} aria-current={on(p) ? ("hash" in p ? "location" : "page") : undefined}>
                  {p.label}
                </Link>
              ))}
              <a href={REPO}>
                source <Icon name="arrow-up-right" weight="bold" size={16} />
              </a>
              <a href={DISCORD}>
                discord <Icon name="arrow-up-right" weight="bold" size={16} />
              </a>
            </motion.nav>
          )}
        </AnimatePresence>
        <main id="main" tabIndex={-1}>
          <motion.div key={top} {...arrive} initial={settled && !reduce ? arrive.initial : false}>
            <Outlet />
          </motion.div>
        </main>
        <Footer />
        <ScrollRestoration getKey={(l) => l.pathname} />
      </div>
    </MotionConfig>
  );
}

function Footer() {
  return (
    <footer className="foot">
      <div className="wrap">
        <div className="foot-grid">
          <div className="foot-col first">
            <Link to="/" className="brand">
              <span className="brand-dot" />
              r1os
            </Link>
            <p style={{ maxWidth: 320, color: "var(--sub)" }}>an agent-first os for the rabbit r1, with a full android device inside.</p>
          </div>
          <div className="foot-col">
            <h4>the os</h4>
            <Link to="/#android">agents in your apps</Link>
            <Link to="/#agents">agents</Link>
            <Link to="/#apps">apps</Link>
            <Link to="/#everything">everything it does</Link>
            <Link to="/#dock">dock mode</Link>
            <Link to="/#speed">speed</Link>
            <Link to="/#companion">the companion</Link>
          </div>
          <div className="foot-col">
            <h4>build</h4>
            <Link to="/docs/start">install r1os</Link>
            <Link to="/docs/develop#the-simulator">try the simulator</Link>
            <Link to="/docs/apps">build an app</Link>
            <Link to="/docs/extend">bring your own agent</Link>
            <Link to="/docs/api">the api</Link>
          </div>
          <div className="foot-col">
            <h4>open source</h4>
            <a href={REPO}>the source</a>
            <a href={ISSUES}>report an issue</a>
            <a href={DISCORD}>join the discord</a>
            <Link to="/docs/develop">develop r1os</Link>
            <Link to="/docs/skill">the r1os skill</Link>
            <a href="/skills/r1os/SKILL.md" style={{ textTransform: "none" }}>
              SKILL.md
            </a>
          </div>
        </div>
        <p className="foot-note">
          r1os is open source under the mit licence. it's independent: it isn't made by or affiliated with rabbit inc.,
          anthropic, openai or google. it runs on the rabbit r1. rabbit, r1, rabbitos and os3 are rabbit inc.'s; claude
          is anthropic's, codex is openai's, and android and google play are google's. your agents run under your own
          accounts.
        </p>
      </div>
    </footer>
  );
}
