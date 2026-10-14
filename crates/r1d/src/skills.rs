//! Skills (T-086): Agent Skills (agentskills.io: a folder with a SKILL.md, its instructions, and sometimes scripts) the
//! R1's agents use.
//!
//! - **Where:** the box's shared home, so one install serves every agent. `npx skills add` (vercel-labs/skills, Node in
//!   the box since 2026-10-11) keeps each in `~/.agents/skills/<name>`, which Codex reads, and links it into
//!   `~/.claude/skills` for Claude Code. The skills on the user's claude.ai account are there too: Claude Code keeps
//!   them in `~/.claude/skills/synced/<account>/<name>` (claude.ai's to change; they can be turned off here).
//! - **Adding:** [`add`], from `POST /api/skills {source, skills?}` (the companion) or an agent's `add_skill` (which asks
//!   on the R1's card first: a skill can run scripts). `source` is anything `skills add` takes: `owner/repo`, a GitHub
//!   URL or folder, a git URL, a URL to a SKILL.md or an archive. One with several skills and none named lists them.
//! - **On for every agent** unless turned off, for all of them or one (`connection_rules`: `skill:<name>` → `off`; the
//!   agent `*` is everyone). A Claude chat gets `skillOverrides: {<name>: "off"}`, a Codex turn
//!   `skills.config = [{name, enabled = false}]`. A change restarts the agents' warm processes.
//! - Claude chats run with the `Skill` tool and without `--disable-slash-commands` (which turns skills off too); a
//!   message that starts with "/" is prefixed, so it isn't taken as a command ([`not_a_command`]).

use crate::hub::{Event, Hub, Source};
use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

/// The skills CLI, pinned.
const CLI: &str = "skills@1.7.2";
const NPX: &str = "/usr/bin/npx";
/// Cloning a repo over the R1's Wi-Fi, and npm fetching the CLI the first time.
const LIMIT: Duration = Duration::from_secs(180);
/// "Everyone" in `connection_rules`.
pub const ALL: &str = "*";

/// Claude Code's own skills (2.1.29x), for working on code in a repo: always off here (`update-config` would change the
/// CLI's settings every agent shares). They aren't listed.
const BUNDLED: &[&str] = &[
    "update-config", "keybindings-help", "fewer-permission-prompts", "loop", "schedule", "init", "simplify", "code-review",
    "security-review", "run", "plugin-authoring", "workflow-authoring", "claude-api", "dataviz",
];

/// The claude.ai account's skills made for claude.ai's own app (its browser, computer use, Docs, artifacts, memory),
/// whose tools the R1's agents don't have: off until turned on.
pub const OFF_AT_FIRST: &[&str] = &["built-in-browser", "chrome-browser", "computer-use", "docs", "morning", "import-memory"];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// "added" (on the R1, removable) or "claude.ai" (the account's).
    pub from: &'static str,
}

/// The box's home, from outside it.
fn home() -> PathBuf {
    Path::new(crate::jail::BOX).join("home")
}

/// The skills the agents have: added ones, then the claude.ai account's, by name.
pub fn installed() -> Vec<Skill> {
    let home = home();
    let mut out: Vec<Skill> = Vec::new();
    let mut take = |dir: &Path, from: &'static str| {
        if let Some(s) = read(dir, from)
            && !out.iter().any(|x| x.name == s.name)
        {
            out.push(s);
        }
    };
    // Added: the skills CLI's copies, and any folder put in Claude's by hand. Its links into ~/.agents (paths inside the
    // box) are the copies already read.
    for root in [home.join(".agents/skills"), home.join(".claude/skills")] {
        for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name == "synced" || name.starts_with('.') || e.file_type().is_ok_and(|t| t.is_symlink()) {
                continue;
            }
            take(&e.path(), "added");
        }
    }
    for account in std::fs::read_dir(home.join(".claude/skills/synced")).into_iter().flatten().flatten() {
        for e in std::fs::read_dir(account.path()).into_iter().flatten().flatten() {
            take(&e.path(), "claude.ai");
        }
    }
    out.sort_by(|a, b| (a.from != "added", &a.name).cmp(&(b.from != "added", &b.name)));
    out
}

fn read(dir: &Path, from: &'static str) -> Option<Skill> {
    let md = std::fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let (name, description) = front(&md);
    let folder = dir.file_name()?.to_string_lossy().into_owned();
    Some(Skill { name: name.unwrap_or(folder), description: description.unwrap_or_default(), from })
}

/// `name` and `description` from a SKILL.md's YAML frontmatter: plain, quoted, or folded over the lines after.
pub fn front(md: &str) -> (Option<String>, Option<String>) {
    let mut lines = md.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }
    let block: Vec<&str> = lines.take_while(|l| l.trim() != "---").collect();
    let field = |key: &str| -> Option<String> {
        let i = block.iter().position(|l| l.strip_prefix(key).is_some_and(|r| r.starts_with(':')))?;
        let first = block[i][key.len() + 1..].trim();
        let value = if first.is_empty() || matches!(first, ">" | "|" | ">-" | "|-") {
            // Folded or literal: the indented lines that follow, as one line.
            block[i + 1..].iter().take_while(|l| l.starts_with(' ') || l.starts_with('\t')).map(|l| l.trim()).collect::<Vec<_>>().join(" ")
        } else {
            first.to_owned()
        };
        let v = value.trim();
        let v = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).or_else(|| v.strip_prefix('\'').and_then(|v| v.strip_suffix('\''))).unwrap_or(v);
        (!v.is_empty()).then(|| v.replace("\\\"", "\""))
    };
    (field("name"), field("description"))
}

// ---- on and off ---------------------------------------------------------------------------------------------------

fn rules(db: &Connection, agent: &str, level: &str) -> anyhow::Result<Vec<String>> {
    let mut st = db.prepare("SELECT substr(key, 7) FROM connection_rules WHERE agent = ?1 AND level = ?2 AND key LIKE 'skill:%' ORDER BY 1")?;
    let names = st.query_map(params![agent, level], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(names)
}

/// Off for everyone: turned off, or off at first ([`OFF_AT_FIRST`]) and not turned on.
pub fn off_for_all(db: &Connection) -> anyhow::Result<Vec<String>> {
    let on = rules(db, ALL, "on")?;
    let mut off = rules(db, ALL, "off")?;
    off.extend(OFF_AT_FIRST.iter().filter(|n| !on.iter().any(|o| o == *n)).map(|n| (*n).to_owned()));
    off.sort();
    off.dedup();
    Ok(off)
}

/// The skills off for `agent`: for everyone, or for it.
pub fn off(db: &Connection, agent: &str) -> anyhow::Result<Vec<String>> {
    let mut off = off_for_all(db)?;
    off.extend(rules(db, agent, "off")?);
    off.sort();
    off.dedup();
    Ok(off)
}

/// The skills turned off for one agent only.
pub fn off_exactly(db: &Connection, agent: &str) -> anyhow::Result<Vec<String>> {
    rules(db, agent, "off")
}

/// On or off, for everyone (`*`) or one agent. One that's off at first is turned on for everyone with an `on` rule.
pub fn set_on(db: &Connection, agent: &str, skill: &str, on: bool) -> anyhow::Result<()> {
    let key = format!("skill:{skill}");
    db.execute("DELETE FROM connection_rules WHERE agent = ?1 AND key = ?2", params![agent, key])?;
    let level = match (on, agent == ALL && OFF_AT_FIRST.contains(&skill)) {
        (true, true) => Some("on"),
        (true, false) => None,
        (false, _) => Some("off"),
    };
    if let Some(level) = level {
        db.execute("INSERT INTO connection_rules (agent, key, level) VALUES (?1, ?2, ?3)", params![agent, key, level])?;
    }
    Ok(())
}

/// A Claude chat's `--settings`: Claude Code's own skills and the ones off for its agent, all `off`.
pub fn claude_settings(hub: &Hub, agent: &str) -> String {
    let mut off = hub.core.with(|db| off(db, agent)).unwrap_or_default();
    off.extend(BUNDLED.iter().map(|b| (*b).to_owned()));
    let overrides: serde_json::Map<String, Value> = off.into_iter().map(|n| (n, json!("off"))).collect();
    json!({ "skillOverrides": overrides }).to_string()
}

/// A Codex turn's `-c` value for the skills off for its agent. None when none are.
pub fn codex_config(hub: &Hub, agent: &str) -> Option<String> {
    let off = hub.core.with(|db| off(db, agent)).unwrap_or_default();
    if off.is_empty() {
        return None;
    }
    let entries: Vec<String> = off.iter().map(|n| format!("{{name = {}, enabled = false}}", json!(n))).collect();
    Some(format!("skills.config=[{}]", entries.join(", ")))
}

/// What a user's message would be if it starts with "/": Claude Code takes that as a command (or a skill to run),
/// and skills are on, so it's told it's the user's words.
pub fn not_a_command(text: &str) -> std::borrow::Cow<'_, str> {
    if text.trim_start().starts_with('/') { format!("[The user's message, as they wrote it:]\n{text}").into() } else { text.into() }
}

// ---- adding and removing -------------------------------------------------------------------------------------------

/// A source as `skills add` takes it: one word, no option.
fn check_source(source: &str) -> anyhow::Result<&str> {
    let s = source.trim();
    anyhow::ensure!(!s.is_empty() && s.len() <= 400, "say where the skill is: owner/repo, or a link");
    anyhow::ensure!(!s.starts_with('-') && !s.chars().any(char::is_whitespace), "that isn't a skill's address: owner/repo, or a link");
    Ok(s)
}

fn check_name(name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) && !name.starts_with(['-', '.']),
        "`{name}` isn't a skill's name"
    );
    Ok(())
}

/// Runs the skills CLI in the box, as the agent user, without its telemetry. Its output, colours gone.
async fn cli(args: &[&str]) -> anyhow::Result<String> {
    let mut full = vec!["-y", CLI];
    full.extend_from_slice(args);
    let jail = crate::jail::Jail::default();
    let env = [("NO_COLOR", "1"), ("DO_NOT_TRACK", "1"), ("DISABLE_TELEMETRY", "1"), ("CI", "1")];
    let mut cmd = jail.command(NPX, &full, "/home/agent", &env)?;
    let out = tokio::time::timeout(LIMIT, cmd.stdin(Stdio::null()).kill_on_drop(true).output())
        .await
        .map_err(|_| anyhow::anyhow!("it took longer than {} s", LIMIT.as_secs()))??;
    let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    Ok(plain(&text))
}

/// Without terminal colours.
fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// The skills in a source (`skills add --list`): name and description, from its listing.
pub fn listed(out: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for line in out.lines() {
        let Some(rest) = line.strip_prefix('│') else { continue };
        if let Some(name) = rest.strip_prefix("    ").filter(|r| !r.starts_with(' ') && !r.trim().is_empty()) {
            let name = name.trim();
            if name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
                found.push((name.to_owned(), String::new()));
            }
        } else if let Some(d) = rest.strip_prefix("      ")
            && let Some(last) = found.last_mut()
        {
            let d = d.trim();
            if !d.is_empty() {
                if !last.1.is_empty() {
                    last.1.push(' ');
                }
                last.1.push_str(d);
            }
        }
    }
    found
}

/// The skills a source holds (at least one), without adding them.
pub async fn in_source(source: &str) -> anyhow::Result<Vec<(String, String)>> {
    let source = check_source(source)?;
    let out = cli(&["add", source, "--list"]).await?;
    let found = listed(&out);
    anyhow::ensure!(!found.is_empty(), "no skills there: {}", last_line(&out));
    Ok(found)
}

/// What adding came to: the skills added, or the ones to choose from (several and none named).
#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Added {
    Added(Vec<String>),
    Choose(Vec<Value>),
}

/// Adds `pick` from `source` (all of it if it holds one skill; its list if it holds several and none were named) for
/// every agent.
pub async fn add(hub: &Arc<Hub>, source: &str, pick: &[String]) -> anyhow::Result<Added> {
    let source = check_source(source)?;
    for p in pick {
        check_name(p)?;
    }
    let names: Vec<String> = if pick.is_empty() {
        let found = in_source(source).await?;
        if found.len() > 1 {
            return Ok(Added::Choose(found.into_iter().map(|(name, description)| json!({ "name": name, "description": description })).collect()));
        }
        vec![found[0].0.clone()]
    } else {
        pick.to_vec()
    };
    let before: Vec<String> = installed().into_iter().map(|s| s.name).collect();
    let mut args = vec!["add", source, "-g", "-a", "claude-code", "codex", "-s"];
    args.extend(names.iter().map(String::as_str));
    args.push("-y");
    let out = cli(&args).await?;
    let now = installed();
    let added: Vec<String> = now.iter().filter(|s| s.from == "added" && (names.contains(&s.name) || !before.contains(&s.name))).map(|s| s.name.clone()).collect();
    anyhow::ensure!(!added.is_empty(), "it didn't add anything: {}", last_line(&out));
    tracing::info!(source, ?added, "added skills");
    // Running chats see new skills as they come (Claude Code watches its skills folder); new ones from now on.
    hub.emit(Event::ConnectionsChanged, Source::Api);
    Ok(Added::Added(added))
}

/// Removes an added skill (claude.ai's stay: turn them off instead).
pub async fn remove(hub: &Arc<Hub>, name: &str) -> anyhow::Result<()> {
    check_name(name)?;
    let skill = installed().into_iter().find(|s| s.name == name).ok_or_else(|| anyhow::anyhow!("no skill `{name}`"))?;
    anyhow::ensure!(skill.from == "added", "`{name}` comes from your claude.ai account: turn it off instead");
    let out = cli(&["remove", name, "-g", "-a", "claude-code", "codex", "-y"]).await?;
    anyhow::ensure!(!installed().iter().any(|s| s.name == name && s.from == "added"), "it's still there: {}", last_line(&out));
    hub.core.with(|db| {
        db.execute("DELETE FROM connection_rules WHERE key = ?1", params![format!("skill:{name}")])?;
        anyhow::Ok(())
    })?;
    tracing::info!(name, "removed a skill");
    hub.emit(Event::ConnectionsChanged, Source::Api);
    Ok(())
}

fn last_line(out: &str) -> String {
    out.lines().map(|l| l.trim_matches(|c: char| c.is_whitespace() || "│└┌◇◆■●".contains(c))).rfind(|l| !l.is_empty()).unwrap_or("no output").chars().take(200).collect()
}

/// For Settings › Connections and the companion: the skills (each off for everyone or not), and each agent's.
pub fn state(hub: &Hub) -> anyhow::Result<Value> {
    let all_off = hub.core.with(off_for_all)?;
    let skills: Vec<Value> = installed()
        .into_iter()
        .map(|s| json!({ "name": s.name, "description": s.description, "from": s.from, "on": !all_off.contains(&s.name) }))
        .collect();
    let agents: Vec<Value> = hub
        .core
        .agents()?
        .into_iter()
        // An agent that works on a computer uses that computer's CLIs, not the box's skills.
        .filter(|a| a.computer().is_none())
        .map(|a| {
            let off = hub.core.with(|db| off_exactly(db, &a.id)).unwrap_or_default();
            json!({ "id": a.id, "name": a.name, "brain": a.brain, "off": off })
        })
        .collect();
    Ok(json!({ "skills": skills, "agents": agents }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter() {
        assert_eq!(front("---\nname: pdf\ndescription: Read and fill PDFs.\n---\n# PDF"), (Some("pdf".into()), Some("Read and fill PDFs.".into())));
        assert_eq!(front("---\nname: \"imagegen\"\ndescription: \"Make \\\"images\\\"\"\n---"), (Some("imagegen".into()), Some("Make \"images\"".into())));
        assert_eq!(front("---\nname: x\ndescription: >\n  One line,\n  and two.\nlicense: MIT\n---"), (Some("x".into()), Some("One line, and two.".into())));
        assert_eq!(front("# no frontmatter"), (None, None));
    }

    #[test]
    fn a_sources_list() {
        let out = "◇  Found 2 skills\n│\n│    pdf\n│\n│      Read, fill and\n│      merge PDFs.\n│\n│    docx\n│\n│      Word files.\n│\n└  Use --skill <name>";
        assert_eq!(listed(out), vec![("pdf".into(), "Read, fill and merge PDFs.".into()), ("docx".into(), "Word files.".into())]);
        assert_eq!(plain("\u{1b}[38;5;102m$\u{1b}[0m skills"), "$ skills");
    }

    #[test]
    fn on_and_off() {
        let db = Connection::open_in_memory().unwrap();
        crate::connections::migrate(&db).unwrap();
        // Off at first until turned on; others on until turned off.
        assert_eq!(off(&db, "a1").unwrap(), OFF_AT_FIRST.iter().map(|s| s.to_string()).collect::<Vec<_>>().tap_sorted());
        set_on(&db, ALL, "docs", true).unwrap();
        assert!(!off(&db, "a1").unwrap().contains(&"docs".to_owned()));
        set_on(&db, ALL, "pdf", false).unwrap();
        set_on(&db, "a1", "xlsx", false).unwrap();
        assert!(off(&db, "a1").unwrap().contains(&"pdf".to_owned()) && off(&db, "a1").unwrap().contains(&"xlsx".to_owned()));
        assert!(!off(&db, "a2").unwrap().contains(&"xlsx".to_owned()));
        set_on(&db, ALL, "docs", false).unwrap();
        assert!(off(&db, "a2").unwrap().contains(&"docs".to_owned()));
        set_on(&db, ALL, "pdf", true).unwrap();
        assert!(!off(&db, "a2").unwrap().contains(&"pdf".to_owned()));
    }

    trait Sorted {
        fn tap_sorted(self) -> Self;
    }

    impl Sorted for Vec<String> {
        fn tap_sorted(mut self) -> Self {
            self.sort();
            self
        }
    }

    #[test]
    fn sources_and_names() {
        assert!(check_source("anthropics/skills").is_ok());
        assert!(check_source("https://github.com/anthropics/skills/tree/main/skills/pdf").is_ok());
        assert!(check_source("--global").is_err());
        assert!(check_source("a b").is_err());
        assert!(check_name("frontend-design").is_ok());
        assert!(check_name("../x").is_err());
        assert_eq!(not_a_command("/cost"), "[The user's message, as they wrote it:]\n/cost");
        assert_eq!(not_a_command("what's up"), "what's up");
    }
}
