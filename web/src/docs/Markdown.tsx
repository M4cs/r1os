// Markdown, drawn as React from marked's tokens: headings with anchors, code in the site's panels, tables, callouts
// (notes, warnings and dangers, by GitHub's alert syntax), and links that stay in the site go through the router. Raw
// HTML in the source is left out.

import { Icon } from "@r1os/icons";
import { Lexer, type Token, type Tokens } from "marked";
import { createContext, type ReactNode, useContext } from "react";
import { Link } from "react-router";
import { Code } from "../site/Code";

/** Decides what a link does: return true when it's handled (the skill viewer's own files). */
const LinkCtx = createContext<((href: string) => boolean) | null>(null);

const CALLOUT_ICON: Record<string, string> = { note: "info", warning: "warning", danger: "warning-octagon" };

const ENTITIES: Record<string, string> ={ "&amp;": "&", "&lt;": "<", "&gt;": ">", "&quot;": '"', "&#39;": "'", "&#x27;": "'" };
const unescape = (s: string) => s.replace(/&(amp|lt|gt|quot|#39|#x27);/g, (m) => ENTITIES[m] ?? m);

export function Markdown({ tokens, onLink }: { tokens: Token[]; onLink?: (href: string) => boolean }) {
  return (
    <LinkCtx.Provider value={onLink ?? null}>
      <div className="md">{blocks(tokens)}</div>
    </LinkCtx.Provider>
  );
}

function blocks(tokens: Token[]): ReactNode[] {
  return tokens.map((t, i) => block(t, i));
}

function block(t: Token, key: number): ReactNode {
  switch (t.type) {
    case "heading": {
      const h = t as Tokens.Heading & { id?: string };
      const Tag = `h${Math.min(h.depth, 4)}` as "h1";
      return (
        <Tag key={key} id={h.id}>
          {inline(h.tokens)}
          {h.depth > 1 && h.id && (
            <a className="anchor" href={`#${h.id}`} aria-label="link to this section">
              #
            </a>
          )}
        </Tag>
      );
    }
    case "paragraph":
      return <p key={key}>{inline((t as Tokens.Paragraph).tokens)}</p>;
    case "code": {
      const c = t as Tokens.Code;
      return <Code key={key} code={c.text} lang={c.lang || undefined} />;
    }
    case "list": {
      const l = t as Tokens.List;
      const items = l.items.map((it, j) => (
        <li key={j}>
          {it.task && <input type="checkbox" checked={!!it.checked} readOnly />}
          {listItem(it)}
        </li>
      ));
      return l.ordered ? (
        <ol key={key} start={typeof l.start === "number" ? l.start : undefined}>
          {items}
        </ol>
      ) : (
        <ul key={key}>{items}</ul>
      );
    }
    case "table": {
      const tb = t as Tokens.Table;
      return (
        <div className="table" key={key}>
          <table>
            <thead>
              <tr>
                {tb.header.map((c, j) => (
                  <th key={j} style={{ textAlign: tb.align[j] ?? undefined }}>
                    {inline(c.tokens)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {tb.rows.map((row, r) => (
                <tr key={r}>
                  {row.map((c, j) => (
                    <td key={j} style={{ textAlign: tb.align[j] ?? undefined }}>
                      {inline(c.tokens)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    }
    case "blockquote": {
      // GitHub's alert syntax ranks a callout: `> [!WARNING]` is amber, `> [!CAUTION]` red (what can brick a device or
      // erase it), anything else a note in the accent. A screen reader hears the kind first.
      const bq = t as Tokens.Blockquote;
      const alert = bq.text.match(/^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\][ \t]*\n?/i);
      const kind = alert ? (({ warning: "warning", caution: "danger" }) as Record<string, string>)[alert[1]!.toLowerCase()] ?? "note" : "note";
      const body = alert ? Lexer.lex(bq.text.slice(alert[0].length)) : bq.tokens;
      return (
        <blockquote key={key} className={`callout ${kind}`}>
          <Icon name={CALLOUT_ICON[kind]!} weight={kind === "note" ? "bold" : "fill"} size={18} className="callout-icon" />
          <div>
            {kind !== "note" && <span className="sr">{kind}: </span>}
            {blocks(body)}
          </div>
        </blockquote>
      );
    }
    case "hr":
      return <hr key={key} />;
    case "text": {
      const tx = t as Tokens.Text;
      return <p key={key}>{tx.tokens ? inline(tx.tokens) : unescape(tx.text)}</p>;
    }
    default:
      return null;
  }
}

/** A list item: a loose one's blocks, or a tight one's words (and any nested list). */
function listItem(it: Tokens.ListItem): ReactNode {
  return it.tokens.map((t, i) => {
    if (t.type === "text") {
      const tx = t as Tokens.Text;
      return it.loose ? <p key={i}>{inline(tx.tokens ?? [])}</p> : <span key={i}>{tx.tokens ? inline(tx.tokens) : unescape(tx.text)}</span>;
    }
    return block(t, i);
  });
}

function inline(tokens: Token[] | undefined): ReactNode[] {
  if (!tokens) return [];
  return tokens.map((t, i) => <Inline key={i} t={t} />);
}

function Inline({ t }: { t: Token }) {
  const onLink = useContext(LinkCtx);
  switch (t.type) {
    case "text": {
      const tx = t as Tokens.Text;
      return <>{tx.tokens ? inline(tx.tokens) : unescape(tx.text)}</>;
    }
    case "escape":
      return <>{unescape((t as Tokens.Escape).text)}</>;
    case "strong":
      return <strong>{inline((t as Tokens.Strong).tokens)}</strong>;
    case "em":
      return <em>{inline((t as Tokens.Em).tokens)}</em>;
    case "del":
      return <del>{inline((t as Tokens.Del).tokens)}</del>;
    case "codespan":
      return <code className="inline">{unescape((t as Tokens.Codespan).text)}</code>;
    case "br":
      return <br />;
    case "link": {
      const l = t as Tokens.Link;
      const kids = inline(l.tokens);
      if (onLink && !/^(https?:|\/|#)/.test(l.href)) {
        return (
          <a
            href={l.href}
            onClick={(e) => {
              if (onLink(l.href)) e.preventDefault();
            }}
          >
            {kids}
          </a>
        );
      }
      if (l.href.startsWith("/") && !l.href.startsWith("/skills/")) return <Link to={l.href}>{kids}</Link>;
      if (l.href.startsWith("#") || l.href.startsWith("/skills/")) return <a href={l.href}>{kids}</a>;
      return (
        <a href={l.href} target="_blank" rel="noreferrer">
          {kids}
          <Icon name="arrow-up-right" weight="bold" size={12} className="ext" />
        </a>
      );
    }
    case "image": {
      const im = t as Tokens.Image;
      return <img src={im.href} alt={im.text} loading="lazy" />;
    }
    default:
      return null;
  }
}
