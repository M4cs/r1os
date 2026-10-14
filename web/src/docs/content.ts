// The docs: one markdown file per page in ./content, with a little frontmatter (title, description, group, order).
// Pages are grouped in the sidebar in GROUPS' order.

import { marked, type Token, type Tokens } from "marked";

export interface Heading {
  depth: number;
  text: string;
  id: string;
}

export interface Doc {
  slug: string;
  title: string;
  description: string;
  group: string;
  order: number;
  body: string;
  tokens: Token[];
  headings: Heading[];
  /** Words to search: the title, headings and text. */
  haystack: string;
}

export const GROUPS = ["start", "the os", "build", "extend", "contribute"];

const files = import.meta.glob("./content/*.md", { query: "?raw", import: "default", eager: true }) as Record<string, string>;

/** `---\nkey: value\n---` at the top: its fields, and what follows. */
export function frontmatter(src: string): { meta: Record<string, string>; body: string } {
  const m = src.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?/);
  if (!m) return { meta: {}, body: src };
  const meta: Record<string, string> = {};
  for (const line of m[1]!.split(/\r?\n/)) {
    const kv = line.match(/^([\w-]+):\s*(.*)$/);
    if (kv) meta[kv[1]!] = kv[2]!.replace(/^["']|["']$/g, "");
  }
  return { meta, body: src.slice(m[0].length) };
}

export const slugify = (s: string) =>
  s
    .toLowerCase()
    .replace(/[`*_]/g, "")
    .replace(/[^\p{L}\p{N}\s-]/gu, "")
    .trim()
    .replace(/\s+/g, "-");

/** A token's words, without markup. */
export function plain(tokens: Token[] | undefined): string {
  if (!tokens) return "";
  return tokens.map((t) => ("tokens" in t && t.tokens ? plain(t.tokens) : "text" in t ? String(t.text) : "")).join("");
}

export function parse(body: string): { tokens: Token[]; headings: Heading[] } {
  const tokens = marked.lexer(body);
  const seen = new Map<string, number>();
  const headings: Heading[] = [];
  for (const t of tokens) {
    if (t.type !== "heading") continue;
    const h = t as Tokens.Heading;
    const text = plain(h.tokens);
    let id = slugify(text);
    const n = seen.get(id) ?? 0;
    seen.set(id, n + 1);
    if (n) id = `${id}-${n}`;
    (h as Tokens.Heading & { id?: string }).id = id;
    headings.push({ depth: h.depth, text, id });
  }
  return { tokens, headings };
}

export const DOCS: Doc[] = Object.entries(files)
  .map(([path, src]) => {
    const { meta, body } = frontmatter(src);
    const { tokens, headings } = parse(body);
    const slug = path.replace(/^.*\/|\.md$/g, "");
    return {
      slug,
      title: meta.title ?? slug,
      description: meta.description ?? "",
      group: meta.group ?? "start",
      order: Number(meta.order ?? 99),
      body,
      tokens,
      headings,
      haystack: `${meta.title ?? ""} ${meta.description ?? ""} ${body}`.toLowerCase(),
    };
  })
  .sort((a, b) => GROUPS.indexOf(a.group) - GROUPS.indexOf(b.group) || a.order - b.order);

export const docOf = (slug: string | undefined) => DOCS.find((d) => d.slug === (slug ?? "introduction"));
