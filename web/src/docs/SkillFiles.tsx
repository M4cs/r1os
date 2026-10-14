// The r1os skill's files (public/skills/r1os), as served by the site: a one-line install that fetches them all from
// here, then each file in a tab, drawn as the docs are. Links between the files switch tabs.

import { Icon } from "@r1os/icons";
import { LayoutGroup, motion } from "motion/react";
import { useEffect, useMemo, useState } from "react";
import { Code } from "../site/Code";
import { spring } from "../site/motion";
import { frontmatter, parse } from "./content";
import { Markdown } from "./Markdown";

const BASE = "/skills/r1os";
const FILES = ["SKILL.md", "references/rules.md", "references/codebase.md", "references/how-it-works.md", "references/gotchas.md", "references/workflow.md", "references/api.md", "references/enee-spec.md"];

export function SkillFiles() {
  const [file, setFile] = useState(FILES[0]!);
  const [texts, setTexts] = useState<Record<string, string>>({});
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let live = true;
    Promise.all(FILES.map((f) => fetch(`${BASE}/${f}`).then((r) => (r.ok ? r.text() : Promise.reject(new Error(f))))))
      .then((all) => live && setTexts(Object.fromEntries(FILES.map((f, i) => [f, all[i]!]))))
      .catch(() => live && setFailed(true));
    return () => {
      live = false;
    };
  }, []);
  const origin = typeof window !== "undefined" ? window.location.origin : "";
  const install = useMemo(
    () =>
      [
        "# from your project's root (or use ~/.claude/skills for every project)",
        "mkdir -p .claude/skills/r1os/references && cd .claude/skills/r1os",
        `for f in ${FILES.join(" ")}; do`,
        `  curl -fsSLo "$f" "${origin}${BASE}/$f"`,
        "done",
      ].join("\n"),
    [origin],
  );
  const text = texts[file];
  const doc = useMemo(() => {
    if (!text) return null;
    const { meta, body } = frontmatter(text);
    return { meta, ...parse(body) };
  }, [text]);
  // A link to another of the skill's files opens its tab.
  const onLink = (href: string) => {
    const from = file.includes("/") ? file.slice(0, file.lastIndexOf("/") + 1) : "";
    const target = new URL(href, `https://x/${from}`).pathname.slice(1);
    if (FILES.includes(target)) {
      setFile(target);
      const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
      document.getElementById("skill-files")?.scrollIntoView({ behavior: reduce ? "auto" : "smooth", block: "start" });
      return true;
    }
    return false;
  };
  return (
    <section className="skill md">
      <h2 id="install-in-one-go">
        install in one go
        <a className="anchor" href="#install-in-one-go">
          #
        </a>
      </h2>
      <p>This fetches every file from this site into a Claude Code skills folder.</p>
      <Code code={install} lang="bash" title="install the r1os skill" />
      <h2 id="skill-files">
        the files
        <a className="anchor" href="#skill-files">
          #
        </a>
      </h2>
      <div className="skill-box">
        <LayoutGroup id="skill-tabs">
          <div className="skill-tabs" role="tablist">
            {FILES.map((f) => (
              <button key={f} role="tab" aria-selected={f === file} className={`skill-tab ${f === file ? "on" : ""}`} onClick={() => setFile(f)}>
                {f === file && <motion.span className="skill-tab-fill" layoutId="skill-tab" transition={spring} />}
                <span>{f.replace("references/", "")}</span>
              </button>
            ))}
          </div>
        </LayoutGroup>
        <div className="skill-bar">
          <span className="mono muted">
            {BASE}/{file}
          </span>
          <span className="grow" />
          <a className="btn small ghost" href={`${BASE}/${file}`} download={file.replace("references/", "")}>
            <Icon name="download-simple" weight="bold" size={15} /> download
          </a>
          <a className="btn small ghost" href={`${BASE}/${file}`} target="_blank" rel="noreferrer">
            <Icon name="file-text" weight="bold" size={15} /> raw
          </a>
        </div>
        <div className="skill-body">
          {failed && <p className="muted">the skill's files didn't load. they're at {BASE}/.</p>}
          {!failed && !doc && <p className="muted">loading…</p>}
          {doc && (
              <motion.div key={file} initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} transition={spring}>
                {doc.meta.name && (
                  <div className="skill-meta">
                    <span className="readout">
                      name <b>{doc.meta.name}</b>
                    </span>
                    <p>{doc.meta.description}</p>
                  </div>
                )}
                <Markdown tokens={doc.tokens} onLink={onLink} />
              </motion.div>
            )}
        </div>
      </div>
    </section>
  );
}
