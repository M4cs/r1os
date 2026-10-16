// The docs: the pages down the side in their groups (the one you're on filled with the accent, springing between
// pages), a search over every page, the page itself, and what's on it down the other side, following your place.

import { Icon } from "@r1os/icons";
import { LayoutGroup, motion } from "motion/react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Link, NavLink, Outlet, useLocation, useNavigate, useNavigationType, useParams } from "react-router";
import { spring } from "../site/motion";
import { DOCS, GROUPS, docOf, type Doc } from "./content";
import { Markdown } from "./Markdown";
import { SkillFiles } from "./SkillFiles";
import "../styles/docs.css";

export function DocsLayout() {
  const [open, setOpen] = useState(false);
  const loc = useLocation();
  useEffect(() => setOpen(false), [loc.pathname]);
  return (
    <div className="wrap docs">
      <button className="docs-menu" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
        <Icon name="list" weight="bold" size={18} />
        {docOf(loc.pathname.split("/")[2])?.title.toLowerCase() ?? "docs"}
        <Icon name={open ? "caret-up" : "caret-down"} weight="bold" size={14} />
      </button>
      <aside className={`docs-side ${open ? "open" : ""}`}>
        <Sidebar />
      </aside>
      <Outlet />
    </div>
  );
}

function Sidebar() {
  const [q, setQ] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const navigate = useNavigate();
  useEffect(() => {
    const on = (e: KeyboardEvent) => {
      if ((e.key === "k" && (e.metaKey || e.ctrlKey)) || (e.key === "/" && document.activeElement === document.body)) {
        e.preventDefault();
        input.current?.focus();
      }
    };
    window.addEventListener("keydown", on);
    return () => window.removeEventListener("keydown", on);
  }, []);
  const results = useMemo(() => search(q), [q]);
  return (
    <>
      <label className="docs-search">
        <Icon name="magnifying-glass" weight="bold" size={16} />
        <input
          ref={input}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && results[0]) {
              navigate(results[0].to);
              setQ("");
            }
            if (e.key === "Escape") setQ("");
          }}
          placeholder="search the docs"
          aria-label="search the docs"
        />
        <kbd className="kbd">/</kbd>
      </label>
      {q.trim() ? (
        <div className="docs-results">
          {results.length === 0 && (
            <p className="muted small docs-empty">
              nothing matches “{q}”. try a single word, or{" "}
              <button className="text-link" onClick={() => setQ("")}>
                see every page
              </button>
              .
            </p>
          )}
          {results.map((r) => (
            <Link key={r.to} to={r.to} className="docs-result" onClick={() => setQ("")}>
              <b>{r.title}</b>
              <span>{r.where}</span>
            </Link>
          ))}
        </div>
      ) : (
        <LayoutGroup id="docs-nav">
          <nav className="docs-nav">
            {GROUPS.map((g) => (
              <div key={g} className="docs-group">
                <div className="docs-group-label">{g}</div>
                {DOCS.filter((d) => d.group === g).map((d) => (
                  <NavLink key={d.slug} to={d.slug === "introduction" ? "/docs" : `/docs/${d.slug}`} end className={({ isActive }) => `docs-link ${isActive ? "on" : ""}`}>
                    {({ isActive }) => (
                      <>
                        {isActive && <motion.span className="docs-link-fill" layoutId="docs-fill" transition={spring} />}
                        <span>{d.title.toLowerCase()}</span>
                      </>
                    )}
                  </NavLink>
                ))}
              </div>
            ))}
          </nav>
        </LayoutGroup>
      )}
    </>
  );
}

/** Pages and sections whose words match, best first: titles, then headings, then text. */
function search(q: string): { to: string; title: string; where: string }[] {
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return [];
  const out: { to: string; title: string; where: string; score: number }[] = [];
  for (const d of DOCS) {
    if (!words.every((w) => d.haystack.includes(w))) continue;
    const base = d.slug === "introduction" ? "/docs" : `/docs/${d.slug}`;
    const inTitle = words.every((w) => d.title.toLowerCase().includes(w));
    out.push({ to: base, title: d.title.toLowerCase(), where: d.description, score: inTitle ? 3 : 1 });
    for (const h of d.headings) {
      if (h.depth > 1 && words.some((w) => h.text.toLowerCase().includes(w))) out.push({ to: `${base}#${h.id}`, title: h.text.toLowerCase(), where: d.title.toLowerCase(), score: 2 });
    }
  }
  return out.sort((a, b) => b.score - a.score).slice(0, 12);
}

export function DocPage() {
  const { slug } = useParams();
  const doc = docOf(slug);
  const loc = useLocation();
  const nav = useNavigationType();
  // To the section a link named, once the page is drawn; else the top. Back and Forward keep the place you left.
  useEffect(() => {
    if (loc.hash) requestAnimationFrame(() => document.getElementById(decodeURIComponent(loc.hash.slice(1)))?.scrollIntoView({ block: "start" }));
    else if (nav !== "POP") window.scrollTo({ top: 0 });
  }, [loc.pathname, loc.hash, nav]);
  useEffect(() => {
    document.title = doc ? `${doc.title} · r1os docs` : "r1os docs";
    return () => {
      document.title = "r1os";
    };
  }, [doc]);
  if (!doc)
    return (
      <article className="docs-main">
        <h1>nothing here</h1>
        <p className="muted">
          this page isn't in the docs. <Link to="/docs" className="text-link">start from the introduction</Link>, or search
          for it on the left.
        </p>
      </article>
    );
  const i = DOCS.indexOf(doc);
  const prev = DOCS[i - 1];
  const next = DOCS[i + 1];
  return (
    <>
      {/* A new page comes in at once (no waiting for the last to leave), keyed so it rises each time. */}
      <motion.article key={doc.slug} className="docs-main" initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} transition={{ type: "spring", stiffness: 300, damping: 30 }}>
          <div className="docs-crumb mono">docs / {doc.group}</div>
          <Markdown tokens={doc.tokens} />
          {doc.slug === "skill" && <SkillFiles />}
          <nav className="docs-pager">
            {prev ? <Pager doc={prev} dir="prev" /> : <span />}
            {next ? <Pager doc={next} dir="next" /> : <span />}
          </nav>
      </motion.article>
      <OnThisPage doc={doc} extra={doc.slug === "skill" ? SKILL_SECTIONS : []} />
    </>
  );
}

function Pager({ doc, dir }: { doc: Doc; dir: "prev" | "next" }) {
  return (
    <Link to={doc.slug === "introduction" ? "/docs" : `/docs/${doc.slug}`} className={`docs-pager-link ${dir}`}>
      <span className="muted small">{dir === "prev" ? "previous" : "next"}</span>
      <b>
        {dir === "prev" && <Icon name="arrow-left" weight="bold" size={15} />}
        {doc.title.toLowerCase()}
        {dir === "next" && <Icon name="arrow-right" weight="bold" size={15} />}
      </b>
    </Link>
  );
}

/** The skill page's own sections, drawn by SkillFiles after the markdown. */
const SKILL_SECTIONS = [
  { depth: 2, text: "install in one go", id: "install-in-one-go" },
  { depth: 2, text: "the files", id: "skill-files" },
];

/** The page's sections, the one you're reading lit. */
function OnThisPage({ doc, extra }: { doc: Doc; extra: { depth: number; text: string; id: string }[] }) {
  const items = [...doc.headings.filter((h) => h.depth === 2 || h.depth === 3), ...extra];
  const [at, setAt] = useState<string | null>(items[0]?.id ?? null);
  useEffect(() => {
    // Looked up on each scroll: a page drawn after this runs (it waits for the last one to leave) still counts.
    const on = () => {
      let cur = items[0]?.id ?? null;
      for (const h of items) {
        const el = document.getElementById(h.id);
        if (el && el.getBoundingClientRect().top < 140) cur = h.id;
      }
      setAt(cur);
    };
    on();
    const late = setTimeout(on, 400);
    window.addEventListener("scroll", on, { passive: true });
    return () => {
      clearTimeout(late);
      window.removeEventListener("scroll", on);
    };
  }, [doc.slug]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!items.length) return <aside className="docs-toc" />;
  return (
    <aside className="docs-toc">
      <div className="docs-group-label">on this page</div>
      <LayoutGroup id="toc">
        {items.map((h) => (
          <a key={h.id} href={`#${h.id}`} className={`docs-toc-link d${h.depth} ${at === h.id ? "on" : ""}`}>
            {at === h.id && <motion.span className="docs-toc-mark" layoutId="toc-mark" transition={spring} />}
            {h.text.toLowerCase()}
          </a>
        ))}
      </LayoutGroup>
    </aside>
  );
}
