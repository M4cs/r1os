// Code, as the site shows it: a dark panel in the R1's mono, a light touch of colour (keys in the accent, strings,
// comments faint, {{bindings}}), and a copy button. Small on purpose: yaml, shell, json and the like.

import { Icon } from "@r1os/icons";
import { type ReactNode, useEffect, useRef, useState } from "react";
import "../styles/code.css";

type Lang = string | undefined;

const STRING = /("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')/;
const BIND = /(\{\{[^}]*\}\})/;

function words(text: string, lang: Lang): ReactNode[] {
  const out: ReactNode[] = [];
  const parts = text.split(new RegExp(`${STRING.source}|${BIND.source}`, "g")).filter((p) => p !== undefined && p !== "");
  parts.forEach((p, i) => {
    if (BIND.test(p) && p.startsWith("{{")) out.push(<span key={i} className="tk-bind">{p}</span>);
    else if (STRING.test(p) && /^["']/.test(p)) out.push(<span key={i} className="tk-str">{p}</span>);
    else if (lang === "bash" || lang === "sh") {
      // The command's first word, and flags.
      out.push(
        ...p.split(/(\s--?[\w-]+)/g).map((q, j) => (/^\s--?[\w-]+$/.test(q) ? <span key={`${i}-${j}`} className="tk-flag">{q}</span> : q)),
      );
    } else out.push(p);
  });
  return out;
}

function line(text: string, lang: Lang, n: number): ReactNode {
  const comment = lang === "yaml" || lang === "bash" || lang === "sh" || lang === "toml" ? text.match(/^(\s*)(#.*)$/) : text.match(/^(\s*)(\/\/.*)$/);
  if (comment) return <span key={n} className="tk-com">{text}</span>;
  if (lang === "yaml") {
    const m = text.match(/^(\s*-?\s*)([\w.$-]+)(:)(.*)$/);
    if (m) return <span key={n}>{m[1]}<span className="tk-key">{m[2]}</span>{m[3]}{words(m[4]!, lang)}</span>;
  }
  if (lang === "json") {
    const m = text.match(/^(\s*)("[^"]+")(\s*:)(.*)$/);
    if (m) return <span key={n}>{m[1]}<span className="tk-key">{m[2]}</span>{m[3]}{words(m[4]!, lang)}</span>;
  }
  if ((lang === "bash" || lang === "sh") && /^\s*\w/.test(text)) {
    const m = text.match(/^(\s*)([\w./-]+)(.*)$/);
    if (m) return <span key={n}>{m[1]}<span className="tk-cmd">{m[2]}</span>{words(m[3]!, lang)}</span>;
  }
  return <span key={n}>{words(text, lang)}</span>;
}

export function highlight(code: string, lang: Lang): ReactNode[] {
  return code.split("\n").flatMap((l, i, all) => (i < all.length - 1 ? [line(l, lang, i), "\n"] : [line(l, lang, i)]));
}

export function Code({ code, lang, title, className }: { code: string; lang?: string; title?: string; className?: string }) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const copy = () => {
    navigator.clipboard?.writeText(code).then(() => {
      setCopied(true);
      clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setCopied(false), 1400);
    });
  };
  return (
    <div className={`code ${className ?? ""}`}>
      <div className="code-head">
        <span className="code-title">{title ?? lang ?? "text"}</span>
        <button className="code-copy" onClick={copy} aria-label="copy">
          <Icon name={copied ? "check" : "copy"} weight="bold" size={14} />
          {copied ? "copied" : "copy"}
        </button>
      </div>
      <pre>
        <code>{highlight(code.replace(/\n$/, ""), lang)}</code>
      </pre>
    </div>
  );
}
