// The pieces of a chat in the companion (ADR-0051): replies in markdown, the cards agents show (ADR-0039's catalog,
// drawn here as the R1 draws them natively), builds, agents' messages to each other, and tool steps.

import { Icon } from "@r1os/icons";
import { Fragment, type ReactNode, useEffect, useState } from "react";
import { type R1Info, mediaUrl, openUrl } from "../link";

// ---- markdown ----------------------------------------------------------------------------------------------
// Enough of it for agents' replies: paragraphs, headings, lists, quotes, code (inline and fenced), bold, italics,
// links (opened in the browser) and bare URLs. Built as elements, never as HTML.

export function Markdown({ text }: { text: string }) {
  const blocks: ReactNode[] = [];
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const fence = line.match(/^\s*```(\w*)/);
    if (fence) {
      const code: string[] = [];
      i++;
      while (i < lines.length && !/^\s*```/.test(lines[i])) code.push(lines[i++]);
      i++;
      blocks.push(
        <pre key={blocks.length} className="md-code">
          <code>{code.join("\n")}</code>
        </pre>,
      );
      continue;
    }
    if (!line.trim()) {
      i++;
      continue;
    }
    const heading = line.match(/^(#{1,4})\s+(.*)$/);
    if (heading) {
      blocks.push(
        <div key={blocks.length} className={`md-h md-h${heading[1].length}`}>
          {inline(heading[2])}
        </div>,
      );
      i++;
      continue;
    }
    if (/^\s*([-*+]|\d+[.)])\s+/.test(line)) {
      const ordered = /^\s*\d+[.)]\s+/.test(line);
      const items: string[] = [];
      while (i < lines.length && /^\s*([-*+]|\d+[.)])\s+/.test(lines[i])) {
        let item = lines[i].replace(/^\s*([-*+]|\d+[.)])\s+/, "");
        i++;
        // A wrapped line carries on the item.
        while (i < lines.length && lines[i].trim() && /^\s{2,}\S/.test(lines[i]) && !/^\s*([-*+]|\d+[.)])\s+/.test(lines[i])) item += ` ${lines[i++].trim()}`;
        items.push(item);
      }
      const List = ordered ? "ol" : "ul";
      blocks.push(
        <List key={blocks.length} className="md-list">
          {items.map((it, n) => (
            <li key={n}>{inline(it)}</li>
          ))}
        </List>,
      );
      continue;
    }
    if (/^\s*>/.test(line)) {
      const quote: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i])) quote.push(lines[i++].replace(/^\s*>\s?/, ""));
      blocks.push(
        <blockquote key={blocks.length} className="md-quote">
          {inline(quote.join(" "))}
        </blockquote>,
      );
      continue;
    }
    const para: string[] = [];
    while (i < lines.length && lines[i].trim() && !/^\s*(```|#{1,4}\s|[-*+]\s|\d+[.)]\s|>)/.test(lines[i])) para.push(lines[i++]);
    if (para.length === 0) para.push(lines[i++]);
    blocks.push(
      <p key={blocks.length} className="md-p">
        {para.map((l, n) => (
          <Fragment key={n}>
            {n > 0 && <br />}
            {inline(l)}
          </Fragment>
        ))}
      </p>,
    );
  }
  return <div className="md">{blocks}</div>;
}

/** Inline markdown: `code`, **bold**, *italic*, [links](url) and bare URLs. */
function inline(s: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(`[^`]+`)|(\*\*[^*]+\*\*)|(__[^_]+__)|(\*[^*\s][^*]*\*)|(_[^_\s][^_]*_)|(\[[^\]]+\]\([^)\s]+\))|(https?:\/\/[^\s)]+)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(s))) {
    if (m.index > last) out.push(s.slice(last, m.index));
    const t = m[0];
    const k = out.length;
    if (m[1]) out.push(<code key={k} className="md-inline">{t.slice(1, -1)}</code>);
    else if (m[2] || m[3]) out.push(<strong key={k}>{inline(t.slice(2, -2))}</strong>);
    else if (m[4] || m[5]) out.push(<em key={k}>{inline(t.slice(1, -1))}</em>);
    else if (m[6]) {
      const [, label, url] = t.match(/^\[([^\]]+)\]\(([^)\s]+)\)$/) ?? [];
      out.push(<Link key={k} url={url}>{label}</Link>);
    } else out.push(<Link key={k} url={t}>{t}</Link>);
    last = m.index + t.length;
  }
  if (last < s.length) out.push(s.slice(last));
  return out;
}

function Link({ url, children }: { url?: string; children: ReactNode }) {
  return (
    <a className="md-link" href={url} onClick={(e) => { e.preventDefault(); if (url) void openUrl(url); }}>
      {children}
    </a>
  );
}

// ---- cards (crate::cards) ------------------------------------------------------------------------------------

type Card = { type: string; [k: string]: unknown };
type Obj = Record<string, unknown>;
const str = (v: unknown) => (typeof v === "string" ? v : typeof v === "number" ? String(v) : "");
const num = (v: unknown) => (typeof v === "number" ? v : Number(v));
const list = (v: unknown) => (Array.isArray(v) ? (v as Obj[]) : []);

export function ChatCard({ card }: { card: Card }) {
  const title = str(card.title);
  switch (card.type) {
    case "headlines":
      return (
        <div className="ccard">
          {title && <div className="ccard-title">{title}</div>}
          <div className="ccard-headlines">
            {list(card.items).map((h, n) => (
              <button key={n} className="ccard-headline" onClick={() => h.url && void openUrl(str(h.url))}>
                {h.image ? <img src={str(h.image)} alt="" /> : null}
                <span>
                  <span className="ccard-strong">{str(h.title)}</span>
                  {h.tagline ? <span className="ccard-sub">{str(h.tagline)}</span> : null}
                  <span className="ccard-meta">{[str(h.source), str(h.time)].filter(Boolean).join(" · ")}</span>
                </span>
              </button>
            ))}
          </div>
        </div>
      );
    case "link":
      return (
        <button className="ccard ccard-linkcard" onClick={() => void openUrl(str(card.url))}>
          {card.image ? <img src={str(card.image)} alt="" /> : null}
          <span>
            <span className="ccard-strong">{title}</span>
            {card.summary ? <span className="ccard-sub">{str(card.summary)}</span> : null}
            <span className="ccard-meta">{str(card.source) || hostOf(str(card.url))}</span>
          </span>
        </button>
      );
    case "weather": {
      const unit = `°${str(card.unit) || "C"}`;
      return (
        <div className="ccard">
          <div className="ccard-title">{str(card.place)}</div>
          <div className="ccard-weather">
            <Icon name={str(card.icon) || "cloud"} size={46} />
            <div>
              <div className="ccard-temp">{Math.round(num(card.temp))}{unit}</div>
              <div className="ccard-sub">{str(card.condition)}</div>
            </div>
            <div className="ccard-meta ccard-hilo">
              {card.high != null && <span>high {Math.round(num(card.high))}°</span>}
              {card.low != null && <span>low {Math.round(num(card.low))}°</span>}
              {card.feels != null && <span>feels {Math.round(num(card.feels))}°</span>}
            </div>
          </div>
          {card.note ? <div className="ccard-sub">{str(card.note)}</div> : null}
          {list(card.hours).length > 0 && (
            <div className="ccard-hours">
              {list(card.hours).map((h, n) => (
                <span key={n}>
                  <span className="ccard-meta">{str(h.time)}</span>
                  <Icon name={str(h.icon) || "cloud"} size={18} />
                  <span>{Math.round(num(h.temp))}°</span>
                </span>
              ))}
            </div>
          )}
          {list(card.days).map((d, n) => (
            <div key={n} className="ccard-day">
              <span>{str(d.day)}</span>
              <Icon name={str(d.icon) || "cloud"} size={18} />
              <span className="ccard-meta">{Math.round(num(d.low))}°</span>
              <span className="ccard-strong">{Math.round(num(d.high))}°</span>
            </div>
          ))}
        </div>
      );
    }
    case "image":
      return (
        <div className="ccard ccard-image">
          <img src={str(card.url)} alt={str(card.caption)} onClick={() => card.link && void openUrl(str(card.link))} />
          {(str(card.caption) || str(card.source)) && <div className="ccard-meta">{[str(card.caption), str(card.source)].filter(Boolean).join(" · ")}</div>}
        </div>
      );
    case "stats":
      return (
        <div className="ccard">
          {title && <div className="ccard-title">{title}</div>}
          <div className="ccard-stats">
            {list(card.items).map((s, n) => (
              <div key={n}>
                <div className="ccard-meta">{str(s.label)}</div>
                <div className="ccard-value">{str(s.value)}</div>
                {s.delta ? (
                  <div className={`ccard-delta ${str(s.trend)}`}>
                    {s.trend === "up" ? "▲ " : s.trend === "down" ? "▼ " : ""}
                    {str(s.delta)}
                  </div>
                ) : null}
              </div>
            ))}
          </div>
        </div>
      );
    case "bars": {
      const items = list(card.items);
      const max = Math.max(1e-9, ...items.map((b) => num(b.value)));
      return (
        <div className="ccard">
          {title && <div className="ccard-title">{title}</div>}
          {items.map((b, n) => (
            <div key={n} className="ccard-bar">
              <span className="ccard-meta">{str(b.label)}</span>
              <span className="ccard-bar-track">
                <span className="ccard-bar-fill" style={{ width: `${(num(b.value) / max) * 100}%` }} />
              </span>
              <span className="ccard-strong">{str(b.value)}{str(card.unit)}</span>
            </div>
          ))}
        </div>
      );
    }
    case "line": {
      const pts = list(card.points);
      const vals = pts.map((p) => num(p.value));
      const lo = Math.min(...vals);
      const hi = Math.max(...vals);
      const w = 320;
      const h = 90;
      const xy = vals.map((v, n) => `${(n / Math.max(1, vals.length - 1)) * w},${h - ((v - lo) / Math.max(1e-9, hi - lo)) * (h - 8) - 4}`).join(" ");
      return (
        <div className="ccard">
          {title && <div className="ccard-title">{title}</div>}
          <svg className="ccard-line" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none">
            <polyline points={xy} fill="none" stroke="var(--accent)" strokeWidth="2.5" strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
          </svg>
          <div className="ccard-line-ends ccard-meta">
            <span>{str(pts[0]?.label)} · {str(pts[0]?.value)}{str(card.unit)}</span>
            <span>{str(pts[pts.length - 1]?.label)} · {str(pts[pts.length - 1]?.value)}{str(card.unit)}</span>
          </div>
        </div>
      );
    }
    case "steps":
      return (
        <div className="ccard">
          {title && <div className="ccard-title">{title}</div>}
          <ol className="ccard-steps">
            {list(card.items).map((s, n) => (
              <li key={n}>
                <span className="ccard-strong">{str(s.title)}</span>
                {s.detail ? <span className="ccard-sub">{str(s.detail)}</span> : null}
              </li>
            ))}
          </ol>
        </div>
      );
    case "table": {
      const cols = Array.isArray(card.columns) ? (card.columns as string[]) : [];
      const rows = Array.isArray(card.rows) ? (card.rows as string[][]) : [];
      return (
        <div className="ccard">
          {title && <div className="ccard-title">{title}</div>}
          <table className="ccard-table">
            <thead>
              <tr>{cols.map((c, n) => <th key={n}>{c}</th>)}</tr>
            </thead>
            <tbody>
              {rows.map((r, n) => (
                <tr key={n}>{r.map((c, k) => <td key={k}>{c}</td>)}</tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }
    default:
      return <div className="ccard ccard-meta">a card the companion can't draw yet ({card.type})</div>;
  }
}

function hostOf(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, "");
  } catch {
    return url;
  }
}

// ---- photos sent with a message ------------------------------------------------------------------------------

export function Photo({ r1, id }: { r1: R1Info; id: string }) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let gone = false;
    mediaUrl(r1.id, id).then((u) => !gone && setUrl(u)).catch(() => undefined);
    return () => {
      gone = true;
    };
  }, [r1.id, id]);
  return url ? <img className="chat-photo" src={url} alt="" /> : <span className="chat-photo loading" />;
}

// ---- builds and agents' mail -----------------------------------------------------------------------------------

const BUILD_WORDS: Record<string, string> = {
  writing: "writing it",
  checking: "checking it",
  repairing: "fixing what the checks found",
  installing: "installing it",
  done: "done",
  failed: "it didn't work",
};

export function BuildLine({ stage, app, edit, error, label }: { stage: string; app?: string; edit?: boolean; error?: string; label?: string }) {
  const done = stage === "done";
  const failed = stage === "failed";
  return (
    <div className={`chat-build ${done ? "done" : failed ? "failed" : ""}`}>
      <Icon name={done ? "check-circle" : failed ? "warning-circle" : "hammer"} weight={done ? "fill" : "bold"} size={18} />
      <span>
        <span className="ccard-strong">{label || (edit ? "changing an app" : "building an app")}</span>
        <span className="ccard-sub">{failed && error ? error : `${BUILD_WORDS[stage] ?? stage}${app && done ? ` · ${app}` : ""}`}</span>
      </span>
      {!done && !failed && <span className="chat-dots"><i /><i /><i /></span>}
    </div>
  );
}

export function MailLine({ text, out, who, kind }: { text: string; out: boolean; who: string; kind?: string }) {
  const verb = out ? (kind === "tell" ? `left ${who} a note` : `asked ${who}`) : kind === "reply" ? `${who} answered` : `${who} wrote`;
  return (
    <div className={`chat-mail ${out ? "out" : ""}`}>
      <Icon name={out ? "paper-plane-tilt" : "envelope-simple-open"} size={16} />
      <span>
        <span className="ccard-meta">{verb}</span>
        <span className="chat-mail-text">{text}</span>
      </span>
    </div>
  );
}
