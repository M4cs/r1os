//! r1d's own database, `<data>/core.db` (docs/specs/agents.md §2, docs/specs/experience.md §4):
//! the agents, onboarding progress, each agent's chats with their messages, and memory (facts every
//! agent shares, and each agent's own).
//!
//! A database from before multiple agents (ADR-0011) is migrated when it's opened: its one agent
//! (`settings.agent`) becomes the first row in `agents`, and its chats belong to that agent. Its
//! memories were about the user, so they become shared facts.
//!
//! Chats are of two kinds (ADR-0012): `user`, a conversation with the user, and `thread`, an agent's
//! exchanges with one other agent (`peer`), where other agents' messages wake it (crate::mail).

use anyhow::Context;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;

pub struct Core {
    db: Mutex<Connection>,
}

/// The most agents on one R1 (the home wheel, and a prompt that will list the others).
pub const MAX_AGENTS: usize = 8;

/// One of the user's agents (docs/specs/agents.md §1).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Agent {
    /// Stable id: 8 hex digits (also its directory under the agent box's `agents/`).
    pub id: String,
    pub name: String,
    /// The character's shape (FORMS).
    pub form: String,
    /// A named accent from the app palette.
    pub color: String,
    /// How the colour is rendered (FINISHES): soft light, matte, grain, noise or a gradient.
    pub finish: String,
    /// Its eyes (the wardrobe's, crate::wardrobe).
    pub eyes: String,
    /// What it wears (crate::wardrobe, T-062): at most one thing in each of the wardrobe's slots (its head, face,
    /// cheeks, neck and back).
    pub wear: Vec<String>,
    /// friendly · concise · playful · calm
    pub personality: String,
    /// One optional line the user adds, ≤ 200 chars.
    pub custom: String,
    /// What it's for, in a few words (≤ 120 chars; empty = a bit of everything).
    pub role: String,
    /// [`BRAINS`]: the brain new chats use. A chat keeps the brain it started with.
    pub brain: String,
    /// What it may change on the R1 (ADR-0045): area (crate::approvals AREAS) → ask, allow or never. Unset: its
    /// area's default (crate::approvals::default_level).
    pub can: std::collections::BTreeMap<String, String>,
    /// Where it works (docs/specs/companion.md §7.2): `r1`, in its box here, or `computer:<key>`, a linked computer
    /// whose companion runs its turns.
    pub works_on: String,
    /// On a computer: the folders it may work in besides its own workspace (paths on that computer).
    pub folders: Vec<String>,
    /// On a computer: the permission mode its new chats start in (MODES; the CLI's `--permission-mode`). Each chat
    /// keeps the one it started with.
    pub mode: String,
}

/// What an agent can talk with: the CLIs in its box or on a computer, or the owner's own Hermes gateway (ADR-0070).
pub const BRAINS: &[&str] = &["claude", "codex", "hermes"];

/// How an agent working on a computer asks (Max, 2026-10-10): `manual` (what isn't allowed comes to its levels and
/// the R1's card), `acceptEdits` (changes files in its folders without asking), `auto` (Claude decides what's safe),
/// `plan` (plans only, changes nothing). Not `bypassPermissions`: its folders are the only bound on a PC (Q41).
pub const MODES: &[&str] = &["manual", "acceptEdits", "auto", "plan"];

impl Default for Agent {
    fn default() -> Self {
        Agent {
            id: String::new(),
            name: "Pip".into(),
            form: "orb".into(),
            color: "orange".into(),
            finish: "soft".into(),
            eyes: "classic".into(),
            wear: Vec::new(),
            personality: "friendly".into(),
            custom: String::new(),
            role: String::new(),
            brain: "claude".into(),
            can: std::collections::BTreeMap::new(),
            works_on: "r1".into(),
            folders: Vec::new(),
            mode: "manual".into(),
        }
    }
}

impl Agent {
    pub fn validate(&mut self) -> anyhow::Result<()> {
        self.name = self.name.trim().chars().take(20).collect();
        if self.name.is_empty() {
            anyhow::bail!("the agent needs a name");
        }
        self.custom = self.custom.trim().chars().take(200).collect();
        self.role = self.role.trim().chars().take(120).collect();
        anyhow::ensure!(FORMS.contains(&self.form.as_str()), "unknown form `{}`", self.form);
        anyhow::ensure!(FINISHES.contains(&self.finish.as_str()), "unknown finish `{}`", self.finish);
        if self.eyes.is_empty() {
            self.eyes = "classic".into();
        }
        let wardrobe = crate::wardrobe::get();
        anyhow::ensure!(wardrobe.eyes.contains(&self.eyes), "unknown eyes `{}` (eyes: {})", self.eyes, wardrobe.eyes.join(", "));
        let mut seen = std::collections::HashSet::new();
        self.wear.retain(|w| seen.insert(w.clone()));
        for w in &self.wear {
            anyhow::ensure!(wardrobe.slot_of(w).is_some(), "unknown wear `{w}` ({})", wardrobe.describe());
        }
        for slot in &wardrobe.slots {
            anyhow::ensure!(
                self.wear.iter().filter(|w| slot.items.contains(w)).count() <= 1,
                "one thing {} at most: {}",
                slot.label.to_lowercase(),
                slot.items.join(", ")
            );
        }
        anyhow::ensure!(["friendly", "concise", "playful", "calm"].contains(&self.personality.as_str()), "unknown personality");
        anyhow::ensure!(BRAINS.contains(&self.brain.as_str()), "unknown brain `{}`", self.brain);
        for (area, level) in &self.can {
            anyhow::ensure!(crate::approvals::AREAS.contains(&area.as_str()), "unknown area `{area}`");
            anyhow::ensure!(crate::approvals::LEVELS.contains(&level.as_str()), "`{level}` isn't ask, allow or never");
        }
        // A palette name, or any colour of your own as `#rrggbb` (T-060), kept lowercase.
        let hex = self.color.len() == 7 && self.color.starts_with('#') && self.color[1..].chars().all(|c| c.is_ascii_hexdigit());
        if hex {
            self.color = self.color.to_ascii_lowercase();
        }
        anyhow::ensure!(hex || (self.color.chars().all(|c| c.is_ascii_lowercase()) && !self.color.is_empty()), "bad color: a palette name or #rrggbb");
        if self.works_on.is_empty() {
            self.works_on = "r1".into();
        }
        let computer = self.works_on.strip_prefix("computer:");
        anyhow::ensure!(
            self.works_on == "r1" || computer.is_some_and(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric())),
            "works_on is r1 or computer:<key>"
        );
        anyhow::ensure!(self.brain != "hermes" || computer.is_none(), "a Hermes agent works on its own gateway, not on a linked computer");
        self.folders.retain(|f| !f.trim().is_empty());
        self.folders.dedup();
        anyhow::ensure!(self.folders.len() <= 8, "eight folders at most");
        anyhow::ensure!(self.folders.iter().all(|f| f.len() <= 300 && !f.chars().any(char::is_control)), "a folder is a path");
        if self.mode.is_empty() {
            self.mode = "manual".into();
        }
        anyhow::ensure!(MODES.contains(&self.mode.as_str()), "mode is one of {}", MODES.join(", "));
        Ok(())
    }

    /// The linked computer it works on, if it's not the R1.
    pub fn computer(&self) -> Option<&str> {
        self.works_on.strip_prefix("computer:")
    }
}

/// The character shapes the shell draws (shell/src/ui/Character.tsx).
pub const FORMS: &[&str] = &["orb", "blob", "bot", "sprite", "cat", "ghost", "bunny", "bean", "cloud"];
/// How a character's colour is rendered (the same file).
pub const FINISHES: &[&str] = &["soft", "matte", "grain", "noise", "gradient", "grainy", "aurora"];

/// Agent ids name directories on the R1, so they're only ever 8 lowercase hex digits.
pub fn valid_agent_id(id: &str) -> bool {
    id.len() == 8 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Debug, Clone, Serialize)]
pub struct Chat {
    pub id: String,
    /// The agent the chat is with.
    pub agent: String,
    pub title: String,
    pub brain: String,
    /// The CLI's session (Claude) or thread (Codex) id; none for a new chat, or after the
    /// conversation was summarized into a fresh session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub created: i64,
    pub updated: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<String>,
    /// The context the CLI session holds, in tokens, after its last turn (0 = not known).
    pub context: i64,
    /// Replies (assistant, build and error messages) the user hasn't seen yet.
    pub unread: i64,
    /// What came before the current session, written by the agent when r1d tidied the context
    /// (crate::chat::tidy). It goes into the system prompt of every process for this chat.
    #[serde(skip)]
    pub summary: Option<String>,
    /// `user` (a conversation with the user) or `thread` (with another agent, `peer`; ADR-0012).
    pub kind: String,
    /// A thread's other agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    /// The permission mode it started in (an agent on a computer; crate::core::MODES). None: its agent's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub id: i64,
    pub role: String,
    pub text: String,
    pub at: i64,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub meta: Value,
}

/// Memory: short facts, all of them in the agent's system prompt.
const MEMORY_MAX: usize = 300;
/// Shared facts (about the user; every agent sees them).
const SHARED_MAX: i64 = 50;
/// Each agent's own facts.
const OWN_MAX: i64 = 50;

#[derive(Debug, Clone, Serialize)]
pub struct Memory {
    pub id: i64,
    pub text: String,
    pub at: i64,
    /// Every agent sees it (a fact about the user), rather than only `agent`.
    pub shared: bool,
    /// Whose own fact it is (None when shared).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// The agent that wrote it (None for facts from before multiple agents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The messages that count as something new for the user (`m` in chat `c`): in a conversation with
/// them, the agent's replies, builds, errors and other agents' messages (`mail`); in a thread between
/// agents, only the other agent's messages, not the agent's own answers to it (ADR-0012). CHAT_COLUMNS
/// spells it out too.
const UNREAD: &str = "(CASE WHEN c.kind = 'thread' THEN m.role = 'mail' ELSE m.role IN ('assistant', 'build', 'error', 'mail') END)";

const CHAT_COLUMNS: &str = "c.id, c.agent, c.title, c.brain, c.session, c.created, c.updated, c.context, c.summary,
    (SELECT text FROM messages m WHERE m.chat = c.id AND m.role != 'note'
       AND NOT (m.role = 'user' AND json_extract(m.meta, '$.routine') IS NOT NULL) ORDER BY m.id DESC LIMIT 1),
    (SELECT count(*) FROM messages m WHERE m.chat = c.id AND (CASE WHEN c.kind = 'thread' THEN m.role = 'mail'
       ELSE m.role IN ('assistant', 'build', 'error', 'mail') END) AND m.at > c.seen),
    c.kind, c.peer, c.mode";

fn chat_row(r: &rusqlite::Row) -> rusqlite::Result<Chat> {
    Ok(Chat {
        id: r.get(0)?,
        agent: r.get(1)?,
        title: r.get(2)?,
        brain: r.get(3)?,
        session: r.get(4)?,
        created: r.get(5)?,
        updated: r.get(6)?,
        context: r.get(7)?,
        summary: r.get(8)?,
        last: r.get(9)?,
        unread: r.get(10)?,
        kind: r.get(11)?,
        peer: r.get(12)?,
        mode: r.get(13)?,
    })
}

fn memory_row(r: &rusqlite::Row) -> rusqlite::Result<Memory> {
    let agent: Option<String> = r.get(3)?;
    Ok(Memory { id: r.get(0)?, text: r.get(1)?, at: r.get(2)?, shared: agent.is_none(), agent, by: r.get(4)? })
}

fn has_column(db: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut st = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = st.query_map([], |r| r.get::<_, String>(1))?.collect::<Result<Vec<_>, _>>()?;
    Ok(names.iter().any(|n| n == column))
}

pub(crate) fn add_column(db: &Connection, table: &str, column: &str, decl: &str) -> rusqlite::Result<()> {
    if !has_column(db, table, column)? {
        db.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))?;
    }
    Ok(())
}

/// Brings a database from any earlier version up to multiple agents. Safe to run every start.
fn migrate(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS agents (
           id TEXT PRIMARY KEY, position INTEGER NOT NULL, created INTEGER NOT NULL, profile TEXT NOT NULL);",
    )?;
    add_column(db, "chats", "agent", "TEXT")?;
    add_column(db, "chats", "summary", "TEXT")?;
    add_column(db, "chats", "context", "INTEGER NOT NULL DEFAULT 0")?;
    if !has_column(db, "chats", "seen")? {
        // When the user last saw the chat's replies: everything from before counts as seen.
        db.execute_batch("ALTER TABLE chats ADD COLUMN seen INTEGER NOT NULL DEFAULT 0; UPDATE chats SET seen = updated;")?;
    }
    add_column(db, "memories", "agent", "TEXT")?;
    add_column(db, "memories", "author", "TEXT")?;
    // ADR-0012: an agent's threads with other agents.
    add_column(db, "chats", "kind", "TEXT NOT NULL DEFAULT 'user'")?;
    add_column(db, "chats", "peer", "TEXT")?;
    add_column(db, "chats", "mode", "TEXT")?;
    // Messages for an agent whose computer wasn't there (crate::remote): sent when it's back.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS waiting (chat TEXT NOT NULL, said TEXT NOT NULL, images TEXT NOT NULL DEFAULT '[]', at INTEGER NOT NULL);",
    )?;
    let n: i64 = db.query_row("SELECT count(*) FROM agents", [], |r| r.get(0))?;
    if n == 0 {
        // The single agent from before (or the default one on a new device).
        let legacy: Option<String> = db.query_row("SELECT value FROM settings WHERE key = 'agent'", [], |r| r.get(0)).optional()?;
        let mut agent: Agent = legacy.and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        agent.id = random_hex(4);
        if agent.validate().is_err() {
            agent = Agent { id: agent.id, ..Agent::default() };
        }
        db.execute(
            "INSERT INTO agents (id, position, created, profile) VALUES (?1, 0, ?2, ?3)",
            params![agent.id, now(), serde_json::to_string(&agent)?],
        )?;
    }
    let first: String = db.query_row("SELECT id FROM agents ORDER BY position, created LIMIT 1", [], |r| r.get(0))?;
    db.execute("UPDATE chats SET agent = ?1 WHERE agent IS NULL", [&first])?;
    db.execute_batch("CREATE INDEX IF NOT EXISTS chats_agent ON chats(agent, updated);")?;
    // What the user decided about each app's permissions (crate::broker). `decl` is the permission as
    // the app declared it when the user decided: a changed declaration asks again.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS grants (
           app TEXT NOT NULL, perm TEXT NOT NULL, decl TEXT NOT NULL, granted INTEGER NOT NULL,
           via TEXT NOT NULL, at INTEGER NOT NULL, used_at INTEGER, uses INTEGER NOT NULL DEFAULT 0,
           PRIMARY KEY (app, perm));
         CREATE TABLE IF NOT EXISTS trigger_runs (
           app TEXT NOT NULL, trigger TEXT NOT NULL, last INTEGER NOT NULL, PRIMARY KEY (app, trigger));",
    )?;
    crate::alarms::migrate(db)?;
    crate::timers::migrate(db)?;
    crate::routines::migrate(db)?;
    crate::routines::mark_prompts(db)?;
    crate::connections::migrate(db)?;
    crate::missed::migrate(db)?;
    crate::notices::migrate(db)?;
    crate::media::migrate(db)?;
    crate::journal::migrate(db)?;
    crate::mail::migrate(db)?;
    Ok(())
}

impl Core {
    /// Runs `f` on the database, for modules that keep their own tables here (crate::alarms).
    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        f(&self.db.lock().unwrap())
    }

    pub fn open(dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let db = Connection::open(dir.join("core.db")).context("open core.db")?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS chats (
               id TEXT PRIMARY KEY, title TEXT NOT NULL, brain TEXT NOT NULL, session TEXT,
               created INTEGER NOT NULL, updated INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS messages (
               id INTEGER PRIMARY KEY, chat TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
               role TEXT NOT NULL, text TEXT NOT NULL, at INTEGER NOT NULL, meta TEXT);
             CREATE INDEX IF NOT EXISTS messages_chat ON messages(chat, id);
             CREATE TABLE IF NOT EXISTS memories (id INTEGER PRIMARY KEY, text TEXT NOT NULL, at INTEGER NOT NULL);",
        )?;
        migrate(&db).context("migrate core.db")?;
        Ok(Core { db: Mutex::new(db) })
    }

    fn get<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Option<T> {
        let db = self.db.lock().unwrap();
        let v: Option<String> = db.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional().ok()?;
        serde_json::from_str(&v?).ok()
    }

    fn put<T: Serialize>(&self, key: &str, value: &T) -> anyhow::Result<()> {
        let json = serde_json::to_string(value)?;
        self.db.lock().unwrap().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, json],
        )?;
        Ok(())
    }

    // ---- agents ---------------------------------------------------------------------------------

    /// The agents in the user's order (the home wheel's).
    pub fn agents(&self) -> anyhow::Result<Vec<Agent>> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare("SELECT id, profile FROM agents ORDER BY position, created")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, profile) = row?;
            let mut a: Agent = serde_json::from_str(&profile).unwrap_or_default();
            a.id = id;
            out.push(a);
        }
        Ok(out)
    }

    pub fn agent(&self, id: &str) -> Option<Agent> {
        self.agents().ok()?.into_iter().find(|a| a.id == id)
    }

    /// The agent at the front of the wheel. There's always at least one (migrate, delete_agent).
    pub fn first_agent(&self) -> Agent {
        self.agents().ok().and_then(|a| a.into_iter().next()).unwrap_or_default()
    }

    /// The agent a chat is with, or the first one if it's gone.
    pub fn agent_or_first(&self, id: &str) -> Agent {
        self.agent(id).unwrap_or_else(|| self.first_agent())
    }

    pub fn create_agent(&self, mut agent: Agent) -> anyhow::Result<Agent> {
        agent.validate()?;
        agent.id = random_hex(4);
        let db = self.db.lock().unwrap();
        let n: i64 = db.query_row("SELECT count(*) FROM agents", [], |r| r.get(0))?;
        anyhow::ensure!(usize::try_from(n).unwrap_or(usize::MAX) < MAX_AGENTS, "there's room for {MAX_AGENTS} agents");
        let position: i64 = db.query_row("SELECT coalesce(max(position), -1) + 1 FROM agents", [], |r| r.get(0))?;
        db.execute(
            "INSERT INTO agents (id, position, created, profile) VALUES (?1, ?2, ?3, ?4)",
            params![agent.id, position, now(), serde_json::to_string(&agent)?],
        )?;
        Ok(agent)
    }

    pub fn update_agent(&self, id: &str, mut agent: Agent) -> anyhow::Result<Agent> {
        agent.validate()?;
        agent.id = id.to_owned();
        let n = self.db.lock().unwrap().execute(
            "UPDATE agents SET profile = ?2 WHERE id = ?1",
            params![id, serde_json::to_string(&agent)?],
        )?;
        anyhow::ensure!(n == 1, "no agent `{id}`");
        Ok(agent)
    }

    /// Deletes an agent with its chats and its own memories. The last agent can't be deleted.
    pub fn delete_agent(&self, id: &str) -> anyhow::Result<()> {
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        let n: i64 = tx.query_row("SELECT count(*) FROM agents", [], |r| r.get(0))?;
        anyhow::ensure!(n > 1, "an R1 needs at least one agent");
        anyhow::ensure!(tx.execute("DELETE FROM agents WHERE id = ?1", [id])? == 1, "no agent `{id}`");
        tx.execute("DELETE FROM chats WHERE agent = ?1 OR peer = ?1", [id])?;
        tx.execute("DELETE FROM memories WHERE agent = ?1", [id])?;
        tx.execute("DELETE FROM mail WHERE sender = ?1 OR recipient = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// Puts the agents in this order. Ids not listed keep their place after the listed ones.
    pub fn order_agents(&self, ids: &[String]) -> anyhow::Result<()> {
        let current: Vec<String> = self.agents()?.into_iter().map(|a| a.id).collect();
        let mut order: Vec<&String> = Vec::new();
        for id in ids {
            if current.contains(id) && !order.contains(&id) {
                order.push(id);
            }
        }
        for id in &current {
            if !order.contains(&id) {
                order.push(id);
            }
        }
        let mut db = self.db.lock().unwrap();
        let tx = db.transaction()?;
        for (i, id) in order.iter().enumerate() {
            tx.execute("UPDATE agents SET position = ?2 WHERE id = ?1", params![id, i64::try_from(i)?])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The agent the user last talked to (home opens on it; words spoken elsewhere go to it).
    pub fn focus(&self) -> Agent {
        self.get::<String>("focus").and_then(|id| self.agent(&id)).unwrap_or_else(|| self.first_agent())
    }

    pub fn set_focus(&self, id: &str) -> anyhow::Result<()> {
        self.put("focus", &id)
    }

    // ---- agents talking to each other (ADR-0012) -----------------------------------------------------

    /// The user paused agents waking each other: messages are still delivered, as badges, but no agent
    /// runs a turn the user didn't start.
    pub fn agents_paused(&self) -> bool {
        self.get::<bool>("agents_paused").unwrap_or(false)
    }

    pub fn set_agents_paused(&self, paused: bool) -> anyhow::Result<()> {
        self.put("agents_paused", &paused)
    }

    // ---- the wheel and the button (Settings › Controls) -------------------------------------------------

    /// The wheel turned the other way round.
    pub fn invert_wheel(&self) -> bool {
        self.get("invert_wheel").unwrap_or(false)
    }

    pub fn set_invert_wheel(&self, on: bool) -> anyhow::Result<()> {
        self.put("invert_wheel", &on)
    }

    /// What a triple press opens (crate::device_settings::TRIPLE), `off` by default.
    pub fn triple_press(&self) -> String {
        self.get("triple_press").unwrap_or_else(|| "off".into())
    }

    pub fn set_triple_press(&self, s: &str) -> anyhow::Result<()> {
        self.put("triple_press", &s)
    }

    // ---- the PIN lock (crate::pin) ----------------------------------------------------------------------

    /// The PIN's salted hash, if one is set.
    pub fn pin(&self) -> Option<crate::pin::Hashed> {
        self.get("pin")
    }

    pub fn set_pin(&self, h: Option<&crate::pin::Hashed>) -> anyhow::Result<()> {
        match h {
            Some(h) => self.put("pin", h),
            None => {
                self.db.lock().unwrap().execute("DELETE FROM settings WHERE key IN ('pin', 'pin_tries')", [])?;
                Ok(())
            }
        }
    }

    /// "Lock after": how long the display may be off before the R1 locks (s; 0 by default).
    pub fn pin_grace_s(&self) -> i64 {
        self.get("pin_grace_s").unwrap_or(0)
    }

    pub fn set_pin_grace_s(&self, s: i64) -> anyhow::Result<()> {
        self.put("pin_grace_s", &s)
    }

    /// Wrong codes so far, and when the next try may be (kept over restarts).
    pub fn pin_tries(&self) -> crate::pin::Tries {
        self.get("pin_tries").unwrap_or_default()
    }

    pub fn set_pin_tries(&self, t: crate::pin::Tries) -> anyhow::Result<()> {
        self.put("pin_tries", &t)
    }

    // ---- the time zone -----------------------------------------------------------------------------

    /// The R1's time zone follows its network (ADR-0030), unless the user picked one.
    pub fn zone_auto(&self) -> bool {
        self.get::<bool>("zone_auto").unwrap_or(true)
    }

    pub fn set_zone_auto(&self, on: bool) -> anyhow::Result<()> {
        self.put("zone_auto", &on)
    }

    // ---- app builds ---------------------------------------------------------------------------------

    /// The Claude model app builds use (Settings › App builds), unless a build asks for another.
    pub fn build_model(&self) -> String {
        self.get::<String>("build_model").filter(|m| BUILD_MODELS.contains(&m.as_str())).unwrap_or_else(|| "sonnet".into())
    }

    pub fn set_build_model(&self, model: &str) -> anyhow::Result<()> {
        anyhow::ensure!(BUILD_MODELS.contains(&model), "`{model}` isn't one of {}", BUILD_MODELS.join(", "));
        self.put("build_model", &model)
    }

    /// The Recorder titles and sums up each tape when it stops (T-068): off unless you turn it on.
    pub fn recorder_summaries(&self) -> bool {
        self.get::<bool>("recorder_summaries").unwrap_or(false)
    }

    pub fn set_recorder_summaries(&self, on: bool) -> anyhow::Result<()> {
        self.put("recorder_summaries", &on)
    }

    // ---- the look (ADR-0043) ------------------------------------------------------------------------

    /// The device's look: today's when none was ever set (or it no longer reads).
    pub fn look(&self) -> crate::look::Look {
        self.get("look").unwrap_or_default()
    }

    pub fn set_look(&self, look: &crate::look::Look) -> anyhow::Result<()> {
        self.put("look", look)
    }

    // ---- the power policy's defaults (T-176) --------------------------------------------------------------

    /// Which of the power policy's defaults are in place once, and yours from then on.
    pub fn power_defaults(&self) -> crate::power::Done {
        self.get("power_defaults").unwrap_or_default()
    }

    pub fn set_power_defaults(&self, done: &crate::power::Done) -> anyhow::Result<()> {
        self.put("power_defaults", done)
    }

    // ---- linked computers (ADR-0048) ------------------------------------------------------------------

    /// The computers this R1 trusts (their keys and names).
    pub fn computers(&self) -> Vec<crate::link::Computer> {
        self.get("computers").unwrap_or_default()
    }

    pub fn set_computers(&self, all: &[crate::link::Computer]) -> anyhow::Result<()> {
        self.put("computers", &all)
    }

    // ---- what agents may change (ADR-0045) --------------------------------------------------------

    /// An agent's level for an area: ask (the default), allow or never.
    pub fn permission(&self, agent: &str, area: &str) -> String {
        self.agent(agent).and_then(|a| a.can.get(area).cloned()).unwrap_or_else(|| crate::approvals::default_level(area).into())
    }

    pub fn set_permission(&self, agent: &str, area: &str, level: &str) -> anyhow::Result<()> {
        let mut a = self.agent(agent).ok_or_else(|| anyhow::anyhow!("no agent `{agent}`"))?;
        a.can.insert(area.to_owned(), level.to_owned());
        self.update_agent(agent, a).map(|_| ())
    }

    // ---- apps pinned to home (ADR-0044) -------------------------------------------------------------

    /// The Android apps you've let agents use (crate::appwin, Q55): their packages.
    pub fn agent_apps(&self) -> Vec<String> {
        self.get("android_agent_apps").unwrap_or_default()
    }

    pub fn set_agent_apps(&self, packages: &[String]) -> anyhow::Result<()> {
        self.put("android_agent_apps", &packages)
    }

    /// When you last caught up on what you missed (crate::missed): ms, 0 if never.
    pub fn missed_seen(&self) -> i64 {
        self.get("missed_seen").unwrap_or(0)
    }

    pub fn set_missed_seen(&self, at: i64) -> anyhow::Result<()> {
        self.put("missed_seen", &at)
    }

    /// What the agents' CLIs last said they reach (crate::connections), so a restart doesn't forget it.
    pub fn connections_seen(&self) -> Vec<crate::connections::Server> {
        self.get("connections_seen").unwrap_or_default()
    }

    pub fn set_connections_seen(&self, servers: &[crate::connections::Server]) -> anyhow::Result<()> {
        self.put("connections_seen", &servers)
    }

    /** The apps pinned to home, in order: a mini-app's id, or `android:<package>`. */
    pub fn home_pins(&self) -> Vec<String> {
        self.get("home_pins").unwrap_or_default()
    }

    pub fn set_home_pins(&self, pins: &[String]) -> anyhow::Result<()> {
        self.put("home_pins", &pins)
    }

    // ---- the glance screen (ADR-0042) --------------------------------------------------------------

    /// The apps whose cards are on the glance screen, in order.
    pub fn glance_apps(&self) -> Vec<String> {
        self.get("glance_apps").unwrap_or_default()
    }

    pub fn set_glance_apps(&self, apps: &[String]) -> anyhow::Result<()> {
        self.put("glance_apps", &apps)
    }

    /// The apps with a glance card seen so far: one joins the glance screen only the first time.
    pub fn glance_known(&self) -> Vec<String> {
        self.get("glance_known").unwrap_or_default()
    }

    pub fn set_glance_known(&self, apps: &[String]) -> anyhow::Result<()> {
        self.put("glance_known", &apps)
    }

    // ---- onboarding -------------------------------------------------------------------------------

    /// Onboarding progress: the step the user is on, or "done".
    pub fn onboarding(&self) -> String {
        self.get("onboarding").unwrap_or_else(|| "hello".into())
    }

    pub fn set_onboarding(&self, step: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            ["hello", "wifi", "agent", "brain", "ready", "done"].contains(&step),
            "unknown onboarding step `{step}`"
        );
        self.put("onboarding", &step)
    }

    // ---- chats ------------------------------------------------------------------------------------

    /// Chats of `kind` (`user`, `thread`, or `all`), newest first: one agent's, or everyone's.
    pub fn chats(&self, agent: Option<&str>, kind: &str) -> anyhow::Result<Vec<Chat>> {
        let db = self.db.lock().unwrap();
        let rows = db
            .prepare(&format!(
                "SELECT {CHAT_COLUMNS} FROM chats c WHERE (?1 IS NULL OR c.agent = ?1) AND (?2 = 'all' OR c.kind = ?2)
                 ORDER BY c.updated DESC LIMIT 200"
            ))?
            .query_map(params![agent, kind], chat_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// `agent`'s thread with `peer` (ADR-0012), made the first time they exchange a message. It talks with
    /// `brain`, the agent's when it was made, and keeps it, like any chat.
    pub fn thread(&self, agent: &str, peer: &str, brain: &str, title: &str) -> anyhow::Result<Chat> {
        let found: Option<String> = self
            .db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM chats WHERE agent = ?1 AND kind = 'thread' AND peer = ?2", [agent, peer], |r| r.get(0))
            .optional()?;
        if let Some(id) = found {
            return self.chat(&id)?.context("the thread went away");
        }
        let (id, t) = (random_hex(8), now());
        self.db.lock().unwrap().execute(
            "INSERT INTO chats (id, agent, title, brain, session, created, updated, seen, kind, peer)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?5, ?5 - 1, 'thread', ?6)",
            params![id, agent, title, brain, t, peer],
        )?;
        self.chat(&id)?.context("the thread went away")
    }

    /// The newest message from another agent the user hasn't seen, in `agent`'s threads: (from, kind, text).
    pub fn latest_mail(&self, agent: &str) -> Option<(String, String, String)> {
        let db = self.db.lock().unwrap();
        let row: Option<(String, Option<String>)> = db
            .query_row(
                "SELECT m.text, m.meta FROM messages m JOIN chats c ON m.chat = c.id
                 WHERE c.agent = ?1 AND c.kind = 'thread' AND m.role = 'mail' AND m.at > c.seen ORDER BY m.id DESC LIMIT 1",
                [agent],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()
            .flatten();
        let (text, meta) = row?;
        let meta: Value = meta.and_then(|m| serde_json::from_str(&m).ok()).unwrap_or(Value::Null);
        Some((meta["from"].as_str()?.to_owned(), meta["kind"].as_str().unwrap_or("ask").to_owned(), text))
    }

    pub fn chat(&self, id: &str) -> anyhow::Result<Option<Chat>> {
        let db = self.db.lock().unwrap();
        Ok(db.query_row(&format!("SELECT {CHAT_COLUMNS} FROM chats c WHERE c.id = ?1"), [id], chat_row).optional()?)
    }

    pub fn new_chat(&self, id: &str, agent: &str, brain: &str) -> anyhow::Result<Chat> {
        let t = now();
        self.db.lock().unwrap().execute(
            // seen just before it was made: its first reply is unread even within the same ms.
            "INSERT INTO chats (id, agent, title, brain, session, created, updated, seen) VALUES (?1, ?2, 'New chat', ?3, NULL, ?4, ?4, ?4 - 1)",
            params![id, agent, brain, t],
        )?;
        Ok(Chat {
            id: id.into(),
            agent: agent.into(),
            title: "New chat".into(),
            brain: brain.into(),
            session: None,
            created: t,
            updated: t,
            last: None,
            context: 0,
            unread: 0,
            summary: None,
            kind: "user".into(),
            peer: None,
            mode: None,
        })
    }

    /// The permission mode a chat keeps (an agent on a computer).
    pub fn set_chat_mode(&self, chat: &str, mode: &str) -> anyhow::Result<()> {
        anyhow::ensure!(MODES.contains(&mode), "mode is one of {}", MODES.join(", "));
        self.db.lock().unwrap().execute("UPDATE chats SET mode = ?2 WHERE id = ?1", params![chat, mode])?;
        Ok(())
    }

    /// The user has seen the chat's replies (it's open on screen).
    pub fn mark_seen(&self, chat: &str) -> anyhow::Result<()> {
        self.db.lock().unwrap().execute("UPDATE chats SET seen = max(seen, ?2) WHERE id = ?1", params![chat, now()])?;
        Ok(())
    }

    /// The user has seen `chat`, and with it the messages between agents it shows (ADR-0012): questions and
    /// notes sent from it, and answers that landed in it. Their copies in the other agents' threads count as
    /// seen too, up to the newest of them, so reading Nova's answer in Echo's chat clears Nova's badge (Max,
    /// 2026-10-10). Returns the threads that changed: (agent, thread).
    pub fn seen_mail_from(&self, chat: &str) -> anyhow::Result<Vec<(String, String)>> {
        let db = self.db.lock().unwrap();
        let tied: Vec<(String, i64)> = db
            .prepare("SELECT thread, max(at) FROM mail WHERE origin_chat = ?1 AND thread IS NOT NULL GROUP BY thread")?
            .query_map([chat], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut changed = Vec::new();
        for (thread, at) in tied {
            if db.execute("UPDATE chats SET seen = ?2 WHERE id = ?1 AND seen < ?2", params![thread, at])? > 0 {
                let agent: Option<String> = db.query_row("SELECT agent FROM chats WHERE id = ?1", [&thread], |r| r.get(0)).optional()?;
                if let Some(a) = agent {
                    changed.push((a, thread));
                }
            }
        }
        Ok(changed)
    }

    /// Replies and other agents' messages in `agent`'s chats the user hasn't seen.
    pub fn unread(&self, agent: &str) -> anyhow::Result<i64> {
        let db = self.db.lock().unwrap();
        Ok(db.query_row(
            &format!(
                "SELECT count(*) FROM messages m JOIN chats c ON m.chat = c.id
                 WHERE c.agent = ?1 AND {UNREAD} AND m.at > c.seen"
            ),
            [agent],
            |r| r.get(0),
        )?)
    }

    pub fn set_session(&self, chat: &str, session: &str) -> anyhow::Result<()> {
        self.db.lock().unwrap().execute("UPDATE chats SET session = ?2 WHERE id = ?1", params![chat, session])?;
        Ok(())
    }

    pub fn set_title(&self, chat: &str, title: &str) -> anyhow::Result<()> {
        self.db.lock().unwrap().execute("UPDATE chats SET title = ?2 WHERE id = ?1", params![chat, title])?;
        Ok(())
    }

    /// Records how much context the chat's CLI session holds after a turn.
    pub fn set_context(&self, chat: &str, tokens: i64) -> anyhow::Result<()> {
        self.db.lock().unwrap().execute("UPDATE chats SET context = ?2 WHERE id = ?1", params![chat, tokens])?;
        Ok(())
    }

    /// The conversation continues in a fresh CLI session that starts from `summary`.
    pub fn set_summary(&self, chat: &str, summary: &str) -> anyhow::Result<()> {
        self.db.lock().unwrap().execute(
            "UPDATE chats SET summary = ?2, session = NULL, context = 0 WHERE id = ?1",
            params![chat, summary],
        )?;
        Ok(())
    }

    /// Deletes a chat with its messages, and the messages between agents that were sent from it or kept in
    /// it (a thread's).
    pub fn delete_chat(&self, chat: &str) -> anyhow::Result<()> {
        let db = self.db.lock().unwrap();
        db.execute("DELETE FROM chats WHERE id = ?1", [chat])?;
        db.execute("DELETE FROM mail WHERE origin_chat = ?1 OR thread = ?1", [chat])?;
        Ok(())
    }

    /// A message waiting for its agent's computer (crate::remote): what the agent is to be told, and the photos with it.
    pub fn add_waiting(&self, chat: &str, said: &str, images: &[String]) -> anyhow::Result<()> {
        self.db.lock().unwrap().execute(
            "INSERT INTO waiting (chat, said, images, at) VALUES (?1, ?2, ?3, ?4)",
            params![chat, said, serde_json::to_string(images)?, now()],
        )?;
        Ok(())
    }

    /// Takes the waiting messages whose chats' agents `wanted` (oldest first): (chat, said, images).
    pub fn take_waiting(&self, wanted: impl Fn(&str) -> bool) -> anyhow::Result<Vec<(String, String, Vec<String>)>> {
        let rows: Vec<(i64, String, String, String, String)> = self
            .db
            .lock()
            .unwrap()
            .prepare("SELECT w.rowid, w.chat, w.said, w.images, c.agent FROM waiting w JOIN chats c ON c.id = w.chat ORDER BY w.at")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, Option<String>>(4)?.unwrap_or_default())))?
            .collect::<Result<_, _>>()?;
        // `wanted` may look at the agents (the database again): not under its lock.
        let taken: Vec<_> = rows.into_iter().filter(|r| wanted(&r.4)).collect();
        let db = self.db.lock().unwrap();
        let mut out = Vec::new();
        for (rowid, chat, said, images, _) in taken {
            if db.execute("DELETE FROM waiting WHERE rowid = ?1", [rowid])? > 0 {
                out.push((chat, said, serde_json::from_str(&images).unwrap_or_default()));
            }
        }
        Ok(out)
    }

    pub fn add_message(&self, chat: &str, role: &str, text: &str, meta: &Value) -> anyhow::Result<Message> {
        self.add_message_at(chat, role, text, meta, now())
    }

    /// A message at a given time (ms): a message between agents takes its `mail` row's, so seeing it in one
    /// chat can count for its copy in another ([`Core::seen_mail_from`]).
    pub fn add_message_at(&self, chat: &str, role: &str, text: &str, meta: &Value, t: i64) -> anyhow::Result<Message> {
        let db = self.db.lock().unwrap();
        let meta_s = if meta.is_null() { None } else { Some(meta.to_string()) };
        db.execute(
            "INSERT INTO messages (chat, role, text, at, meta) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![chat, role, text, t, meta_s],
        )?;
        let id = db.last_insert_rowid();
        db.execute("UPDATE chats SET updated = ?2 WHERE id = ?1", params![chat, t])?;
        Ok(Message { id, role: role.into(), text: text.into(), at: t, meta: meta.clone() })
    }

    /// The routine whose run is the chat's latest turn (its prompt's `routine`, T-092), if that turn was one.
    pub fn last_turn_routine(&self, chat: &str) -> anyhow::Result<Option<String>> {
        let db = self.db.lock().unwrap();
        let meta: Option<Option<String>> = db
            .query_row("SELECT meta FROM messages WHERE chat = ?1 AND role = 'user' ORDER BY id DESC LIMIT 1", [chat], |r| r.get(0))
            .optional()?;
        Ok(meta.flatten().and_then(|m| serde_json::from_str::<Value>(&m).ok()).and_then(|v| v["routine"].as_str().map(str::to_owned)))
    }

    /// A routine's run that had nothing to tell (crate::chat::says_nothing): its prompt goes, if it's still the chat's
    /// latest message (the run kept nothing else, such as a card), and the chat's `updated` goes back to its latest
    /// message's. Whether it went.
    pub fn drop_routine_prompt(&self, chat: &str) -> anyhow::Result<bool> {
        let db = self.db.lock().unwrap();
        let last: Option<(i64, String, Option<String>)> = db
            .query_row("SELECT id, role, meta FROM messages WHERE chat = ?1 ORDER BY id DESC LIMIT 1", [chat], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let Some((id, role, meta)) = last else { return Ok(false) };
        let routine = meta.and_then(|m| serde_json::from_str::<Value>(&m).ok()).is_some_and(|v| v["routine"].is_string());
        if role != "user" || !routine {
            return Ok(false);
        }
        db.execute("DELETE FROM messages WHERE id = ?1", [id])?;
        db.execute("UPDATE chats SET updated = COALESCE((SELECT max(at) FROM messages WHERE chat = ?1), updated) WHERE id = ?1", [chat])?;
        Ok(true)
    }

    pub fn messages(&self, chat: &str) -> anyhow::Result<Vec<Message>> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare("SELECT id, role, text, at, meta FROM messages WHERE chat = ?1 ORDER BY id")?;
        let rows = st
            .query_map([chat], |r| {
                let meta: Option<String> = r.get(4)?;
                Ok(Message {
                    id: r.get(0)?,
                    role: r.get(1)?,
                    text: r.get(2)?,
                    at: r.get(3)?,
                    meta: meta.and_then(|m| serde_json::from_str(&m).ok()).unwrap_or(Value::Null),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ---- memory -----------------------------------------------------------------------------------

    /// The shared facts, then `agent`'s own (oldest first). Without an agent: shared facts only.
    pub fn memories(&self, agent: Option<&str>) -> anyhow::Result<Vec<Memory>> {
        let db = self.db.lock().unwrap();
        let mut st = db.prepare(
            "SELECT id, text, at, agent, author FROM memories WHERE agent IS NULL OR agent = ?1
             ORDER BY agent IS NOT NULL, id",
        )?;
        let rows = st.query_map([agent.unwrap_or("")], memory_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// `agent` remembers a fact: its own, or `shared` with every agent (a fact about the user).
    pub fn remember(&self, agent: &str, text: &str, shared: bool) -> anyhow::Result<Memory> {
        let text = text.trim();
        anyhow::ensure!(!text.is_empty() && text.len() <= MEMORY_MAX, "a memory is 1–{MEMORY_MAX} characters");
        let db = self.db.lock().unwrap();
        let (n, max): (i64, i64) = if shared {
            (db.query_row("SELECT count(*) FROM memories WHERE agent IS NULL", [], |r| r.get(0))?, SHARED_MAX)
        } else {
            (db.query_row("SELECT count(*) FROM memories WHERE agent = ?1", [agent], |r| r.get(0))?, OWN_MAX)
        };
        anyhow::ensure!(n < max, "memory is full ({max} {}); forget something first", if shared { "shared facts" } else { "facts" });
        let at = now();
        let owner = (!shared).then_some(agent);
        db.execute("INSERT INTO memories (text, at, agent, author) VALUES (?1, ?2, ?3, ?4)", params![text, at, owner, agent])?;
        Ok(Memory { id: db.last_insert_rowid(), text: text.into(), at, shared, agent: owner.map(str::to_owned), by: Some(agent.to_owned()) })
    }

    /// Forgets one fact. An agent (Some) can forget shared facts and its own; the user (None), any.
    pub fn forget(&self, agent: Option<&str>, id: i64) -> anyhow::Result<bool> {
        let db = self.db.lock().unwrap();
        let n = match agent {
            Some(a) => db.execute("DELETE FROM memories WHERE id = ?1 AND (agent IS NULL OR agent = ?2)", params![id, a])?,
            None => db.execute("DELETE FROM memories WHERE id = ?1", [id])?,
        };
        Ok(n > 0)
    }
}

/// The user's decision about one of an app's permissions (crate::broker).
#[derive(Debug, Clone)]
pub struct Grant {
    /// The permission as declared when the user decided (crate::broker::decl).
    pub decl: String,
    pub granted: bool,
    pub used_at: Option<i64>,
    pub uses: i64,
}

impl Core {
    // ---- app permissions (crate::broker) ----------------------------------------------------------

    /// The user's decisions about `app`'s permissions, by permission id.
    pub fn grants(&self, app: &str) -> std::collections::HashMap<String, Grant> {
        let db = self.db.lock().unwrap();
        let rows = db
            .prepare("SELECT perm, decl, granted, used_at, uses FROM grants WHERE app = ?1")
            .and_then(|mut st| {
                st.query_map([app], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        Grant { decl: r.get(1)?, granted: r.get::<_, i64>(2)? != 0, used_at: r.get(3)?, uses: r.get(4)? },
                    ))
                })?
                .collect::<Result<std::collections::HashMap<_, _>, _>>()
            });
        rows.unwrap_or_else(|e| {
            tracing::warn!(app, error = %e, "couldn't read the app's grants");
            Default::default()
        })
    }

    /// Records the user's decision about one permission, as `decl` declares it.
    pub fn set_grant(&self, app: &str, perm: &str, decl: &str, granted: bool, via: &str) -> anyhow::Result<()> {
        self.db.lock().unwrap().execute(
            "INSERT INTO grants (app, perm, decl, granted, via, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(app, perm) DO UPDATE SET decl = excluded.decl, granted = excluded.granted,
               via = excluded.via, at = excluded.at",
            params![app, perm, decl, granted as i64, via, now()],
        )?;
        Ok(())
    }

    /// The app used a permission (shown on its page: "used 2 min ago").
    pub fn grant_used(&self, app: &str, perm: &str) {
        let r = self.db.lock().unwrap().execute(
            "UPDATE grants SET used_at = ?3, uses = uses + 1 WHERE app = ?1 AND perm = ?2",
            params![app, perm, now()],
        );
        if let Err(e) = r {
            tracing::warn!(app, perm, error = %e, "couldn't record a permission's use");
        }
    }

    /// Forgets every decision about `app`, and its triggers' runs (it was purged from the trash).
    pub fn forget_grants(&self, app: &str) {
        let db = self.db.lock().unwrap();
        let _ = db.execute("DELETE FROM grants WHERE app = ?1", [app]);
        let _ = db.execute("DELETE FROM trigger_runs WHERE app = ?1", [app]);
    }

    // ---- triggers (crate::scheduler) --------------------------------------------------------------

    /// When `app`'s timed trigger last ran (ms), if it has been seen.
    pub fn last_run(&self, app: &str, trigger: &str) -> Option<i64> {
        self.db
            .lock()
            .unwrap()
            .query_row("SELECT last FROM trigger_runs WHERE app = ?1 AND trigger = ?2", params![app, trigger], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_last_run(&self, app: &str, trigger: &str, ms: i64) {
        let r = self.db.lock().unwrap().execute(
            "INSERT INTO trigger_runs (app, trigger, last) VALUES (?1, ?2, ?3)
             ON CONFLICT(app, trigger) DO UPDATE SET last = excluded.last",
            params![app, trigger, ms],
        );
        if let Err(e) = r {
            tracing::warn!(app, trigger, error = %e, "couldn't record a trigger's run");
        }
    }
}

/// The models an app build can use: Claude's aliases, as the `claude` CLI takes them, and `codex` (Codex's own, T-045).
pub const BUILD_MODELS: &[&str] = &["haiku", "sonnet", "opus", "codex"];

/// Random hex, from the OS RNG.
pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS RNG");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// A random (v4) UUID, for CLI session ids.
pub fn uuid() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("OS RNG");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("r1d-core-{}", uuid()))
    }

    #[test]
    fn messages_wait_for_their_computer() {
        let core = Core::open(&temp()).unwrap();
        let here = core.first_agent();
        let away = core.create_agent(Agent { name: "Desk".into(), works_on: "computer:abc123".into(), mode: "acceptEdits".into(), ..Agent::default() }).unwrap();
        assert_eq!(away.computer(), Some("abc123"));
        assert!(core.create_agent(Agent { name: "Bad".into(), mode: "bypassPermissions".into(), ..Agent::default() }).is_err());
        core.new_chat("d1", &away.id, "claude").unwrap();
        core.new_chat("h1", &here.id, "claude").unwrap();
        core.set_chat_mode("d1", "plan").unwrap();
        assert_eq!(core.chat("d1").unwrap().unwrap().mode.as_deref(), Some("plan"));
        core.add_waiting("d1", "rename the photos", &["p1".into()]).unwrap();
        core.add_waiting("h1", "not for a computer", &[]).unwrap();
        let on = |agent: &str| core.agent(agent).is_some_and(|a| a.computer() == Some("abc123"));
        let went = core.take_waiting(on).unwrap();
        assert_eq!(went, vec![("d1".to_owned(), "rename the photos".to_owned(), vec!["p1".to_owned()])]);
        assert!(core.take_waiting(on).unwrap().is_empty(), "each goes once");
        assert_eq!(core.take_waiting(|_| true).unwrap().len(), 1, "the other chat's stays for its own");
    }

    #[test]
    fn agents_chats_and_messages() {
        let dir = temp();
        let core = Core::open(&dir).unwrap();
        // A new device starts with the default agent.
        let first = core.first_agent();
        assert_eq!(first.name, "Pip");
        assert!(valid_agent_id(&first.id));
        assert_eq!(core.onboarding(), "hello");
        let renamed = core.update_agent(&first.id, Agent { name: "  Nova  ".into(), form: "bot".into(), ..first.clone() }).unwrap();
        assert_eq!(renamed.name, "Nova");
        assert!(core.update_agent(&first.id, Agent { form: "dragon".into(), ..Agent::default() }).is_err());
        assert!(core.update_agent(&first.id, Agent { finish: "sparkle".into(), ..Agent::default() }).is_err());
        let ghost = core.update_agent(&first.id, Agent { form: "ghost".into(), finish: "grainy".into(), ..renamed.clone() }).unwrap();
        assert_eq!((ghost.form.as_str(), ghost.finish.as_str()), ("ghost", "grainy"));
        core.update_agent(&first.id, renamed.clone()).unwrap();
        core.set_onboarding("brain").unwrap();
        assert_eq!(core.onboarding(), "brain");

        let bolt = core.create_agent(Agent { name: "Bolt".into(), color: "blue".into(), brain: "codex".into(), ..Agent::default() }).unwrap();
        assert_eq!(core.agents().unwrap().iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Nova", "Bolt"]);
        // A brain it knows; and Hermes works on its own gateway, not a linked computer (ADR-0070).
        assert!(core.create_agent(Agent { name: "Odd".into(), brain: "gpt".into(), ..Agent::default() }).is_err());
        assert!(core.create_agent(Agent { name: "Mess".into(), brain: "hermes".into(), works_on: "computer:abc".into(), ..Agent::default() }).is_err());
        let hermes = core.create_agent(Agent { name: "Mess".into(), brain: "hermes".into(), ..Agent::default() }).unwrap();
        core.delete_agent(&hermes.id).unwrap();
        core.order_agents(std::slice::from_ref(&bolt.id)).unwrap();
        assert_eq!(core.first_agent().id, bolt.id);
        assert_eq!(core.focus().id, bolt.id, "focus falls back to the first agent");
        core.set_focus(&first.id).unwrap();
        assert_eq!(core.focus().id, first.id);

        let c = core.new_chat("c1", &first.id, "claude").unwrap();
        core.new_chat("c2", &bolt.id, "codex").unwrap();
        core.add_message(&c.id, "user", "hi", &Value::Null).unwrap();
        core.add_message(&c.id, "assistant", "hello!", &serde_json::json!({"cost": 0.01})).unwrap();
        core.add_message(&c.id, "note", "tidied", &Value::Null).unwrap();
        assert_eq!(core.messages("c1").unwrap().len(), 3);
        let mine = core.chats(Some(&first.id), "user").unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].last.as_deref(), Some("hello!"), "notes aren't a chat's last message");
        core.add_message("c2", "assistant", "done", &Value::Null).unwrap();
        core.add_message("c2", "user", "the weather and my day", &serde_json::json!({ "routine": "Morning" })).unwrap();
        assert_eq!(core.chat("c2").unwrap().unwrap().last.as_deref(), Some("done"), "nor is a routine's prompt (T-092)");
        // Its latest turn was a routine's; c1's was the user's own.
        assert_eq!(core.last_turn_routine("c2").unwrap().as_deref(), Some("Morning"));
        assert_eq!(core.last_turn_routine("c1").unwrap(), None);
        assert_eq!(core.last_turn_routine("nope").unwrap(), None);
        // A run with nothing to tell leaves nothing: its prompt goes while it's the chat's latest message.
        let before = core.chat("c2").unwrap().unwrap().updated;
        core.add_message_at("c2", "user", "the weather and my day", &serde_json::json!({ "routine": "Morning" }), before + 60_000).unwrap();
        assert!(core.drop_routine_prompt("c2").unwrap());
        assert_eq!((core.messages("c2").unwrap().len(), core.chat("c2").unwrap().unwrap().updated), (2, before));
        core.add_message("c2", "card", "Sunny, 18°", &Value::Null).unwrap();
        assert!(!core.drop_routine_prompt("c2").unwrap(), "not once the run kept something else");
        assert!(!core.drop_routine_prompt("c1").unwrap(), "nor anything but a routine's prompt");
        core.mark_seen("c2").unwrap();
        assert_eq!(core.chats(None, "all").unwrap().len(), 2);
        // A reply is unread until the chat is seen (the user's own words and notes never are).
        assert_eq!((mine[0].unread, core.unread(&first.id).unwrap()), (1, 1));
        core.mark_seen("c1").unwrap();
        assert_eq!((core.chat("c1").unwrap().unwrap().unread, core.unread(&first.id).unwrap()), (0, 0));
        assert_eq!(core.unread(&bolt.id).unwrap(), 0);

        core.set_session("c1", "s-1").unwrap();
        core.set_context("c1", 90_000).unwrap();
        assert_eq!(core.chat("c1").unwrap().unwrap().context, 90_000);
        core.set_summary("c1", "We talked about water.").unwrap();
        let tidied = core.chat("c1").unwrap().unwrap();
        assert_eq!((tidied.session, tidied.context, tidied.summary.as_deref()), (None, 0, Some("We talked about water.")));

        core.delete_chat("c1").unwrap();
        assert!(core.messages("c1").unwrap().is_empty());
        // Deleting an agent takes its chats; the last one stays.
        core.delete_agent(&bolt.id).unwrap();
        assert!(core.chat("c2").unwrap().is_none());
        assert!(core.delete_agent(&first.id).is_err());
        drop(core);
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(uuid().len(), 36);
    }

    #[test]
    fn threads_between_agents() {
        let dir = temp();
        let core = Core::open(&dir).unwrap();
        let pip = core.first_agent().id;
        let nova = core.create_agent(Agent { name: "Nova".into(), ..Agent::default() }).unwrap().id;
        let talk = core.new_chat("u1", &pip, "claude").unwrap();
        // Nova's thread with Pip: made once, then found again.
        let t = core.thread(&nova, &pip, "claude", "Pip").unwrap();
        assert_eq!((t.kind.as_str(), t.peer.as_deref(), t.title.as_str()), ("thread", Some(pip.as_str()), "Pip"));
        assert_eq!(core.thread(&nova, &pip, "codex", "Pip").unwrap().id, t.id);
        // Threads aren't conversations with the user: they're listed apart.
        assert_eq!(core.chats(Some(&nova), "user").unwrap().len(), 0);
        assert_eq!(core.chats(Some(&nova), "thread").unwrap().len(), 1);
        assert_eq!(core.chats(None, "all").unwrap().len(), 2);
        // In a thread, only the other agent's message is new to the user, not Nova's own answer.
        let meta = serde_json::json!({ "from": pip, "kind": "ask" });
        core.add_message(&t.id, "mail", "what's on Saturday?", &meta).unwrap();
        core.add_message(&t.id, "assistant", "the hike", &Value::Null).unwrap();
        assert_eq!(core.unread(&nova).unwrap(), 1);
        assert_eq!(core.latest_mail(&nova), Some((pip.clone(), "ask".into(), "what's on Saturday?".into())));
        // In Pip's chat with the user, the answer's card counts, and so does what Pip makes of it; its own
        // outgoing card doesn't.
        core.add_message(&talk.id, "mail_out", "what's on Saturday?", &Value::Null).unwrap();
        core.add_message(&talk.id, "mail", "the hike", &Value::Null).unwrap();
        core.add_message(&talk.id, "assistant", "Nova says it's the hike.", &Value::Null).unwrap();
        assert_eq!(core.unread(&pip).unwrap(), 2);
        core.mark_seen(&t.id).unwrap();
        assert_eq!((core.unread(&nova).unwrap(), core.latest_mail(&nova)), (0, None));
        // Pausing is remembered.
        assert!(!core.agents_paused());
        core.set_agents_paused(true).unwrap();
        assert!(core.agents_paused());
        // Removing Pip takes the threads other agents had with it.
        core.delete_agent(&pip).unwrap();
        assert!(core.chat(&t.id).unwrap().is_none());
        drop(core);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reading_an_answer_clears_its_question() {
        let dir = temp();
        let core = Core::open(&dir).unwrap();
        let echo = core.first_agent().id;
        let nova = core.create_agent(Agent { name: "Nova".into(), ..Agent::default() }).unwrap().id;
        core.new_chat("u1", &echo, "claude").unwrap();
        core.new_chat("u2", &echo, "claude").unwrap();
        let t = core.thread(&nova, &echo, "claude", "Echo").unwrap();
        let mail = |id: i64, origin: &str, at: i64| {
            core.with(|db| {
                db.execute(
                    "INSERT INTO mail (id, chain, hop, sender, recipient, kind, text, origin_chat, thread, at)
                     VALUES (?1, 'c', 1, ?2, ?3, 'ask', 'q', ?4, ?5, ?6)",
                    params![id, echo, nova, origin, t.id, at],
                )
                .unwrap();
            });
            core.add_message_at(&t.id, "mail", "q", &serde_json::json!({ "mail": id, "from": echo, "kind": "ask" }), at).unwrap();
        };
        // Echo asked Nova from u1; later, from u2.
        let at = now() + 10;
        mail(1, "u1", at);
        mail(2, "u2", at + 20);
        assert_eq!(core.unread(&nova).unwrap(), 2);
        // Seeing u1 (where the question went from, and its answer landed) clears the first in Nova's thread.
        assert_eq!(core.seen_mail_from("u1").unwrap(), [(nova.clone(), t.id.clone())]);
        assert_eq!(core.unread(&nova).unwrap(), 1);
        assert!(core.seen_mail_from("u1").unwrap().is_empty(), "nothing more to clear");
        assert!(core.seen_mail_from("u2").unwrap().len() == 1);
        assert_eq!((core.unread(&nova).unwrap(), core.latest_mail(&nova)), (0, None));
        drop(core);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn agent_limit() {
        let dir = temp();
        let core = Core::open(&dir).unwrap();
        for i in 1..MAX_AGENTS {
            core.create_agent(Agent { name: format!("A{i}"), ..Agent::default() }).unwrap();
        }
        assert!(core.create_agent(Agent::default()).is_err());
        drop(core);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_agents_colour_is_a_palette_name_or_its_own() {
        let mut a = Agent { color: "#5AA0FF".into(), ..Agent::default() };
        a.validate().unwrap();
        assert_eq!(a.color, "#5aa0ff");
        for bad in ["#5aa0f", "5aa0ff", "#5aa0fg", "Blue", ""] {
            assert!(Agent { color: bad.into(), ..Agent::default() }.validate().is_err(), "{bad}");
        }
        assert!(Agent { color: "lime".into(), ..Agent::default() }.validate().is_ok());
    }

    #[test]
    fn shared_and_own_memories() {
        let dir = temp();
        let core = Core::open(&dir).unwrap();
        let a = core.first_agent().id;
        let b = core.create_agent(Agent { name: "Bolt".into(), ..Agent::default() }).unwrap().id;
        let name = core.remember(&a, "The user's name is Sam", true).unwrap();
        let own = core.remember(&a, "Sam likes the water app at 8 glasses", false).unwrap();
        core.remember(&b, "Bolt tracks the gym log", false).unwrap();
        let seen: Vec<(String, bool)> = core.memories(Some(&a)).unwrap().into_iter().map(|m| (m.text, m.shared)).collect();
        assert_eq!(seen, [("The user's name is Sam".to_owned(), true), ("Sam likes the water app at 8 glasses".to_owned(), false)]);
        assert_eq!(core.memories(Some(&b)).unwrap().len(), 2, "Bolt sees the shared fact and its own");
        assert_eq!(core.memories(None).unwrap().len(), 1, "the user's view: shared facts");
        assert_eq!(core.memories(Some(&a)).unwrap()[0].by.as_deref(), Some(a.as_str()));
        // An agent can't forget another agent's own facts, but can forget shared ones.
        assert!(!core.forget(Some(&b), own.id).unwrap());
        assert!(core.forget(Some(&b), name.id).unwrap());
        assert!(core.forget(None, own.id).unwrap());
        assert!(core.remember(&a, "", false).is_err());
        drop(core);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn migrates_a_single_agent_database() {
        let dir = temp();
        std::fs::create_dir_all(&dir).unwrap();
        {
            // core.db as ADR-0010 left it: one agent in settings, chats without an agent.
            let db = Connection::open(dir.join("core.db")).unwrap();
            db.execute_batch(
                "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE chats (id TEXT PRIMARY KEY, title TEXT NOT NULL, brain TEXT NOT NULL, session TEXT,
                   created INTEGER NOT NULL, updated INTEGER NOT NULL);
                 CREATE TABLE messages (id INTEGER PRIMARY KEY, chat TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
                   role TEXT NOT NULL, text TEXT NOT NULL, at INTEGER NOT NULL, meta TEXT);
                 CREATE TABLE memories (id INTEGER PRIMARY KEY, text TEXT NOT NULL, at INTEGER NOT NULL);
                 INSERT INTO settings VALUES ('agent', '{\"name\":\"Echo\",\"form\":\"blob\",\"color\":\"teal\",\"personality\":\"calm\",\"custom\":\"\",\"brain\":\"claude\"}');
                 INSERT INTO settings VALUES ('onboarding', '\"done\"');
                 INSERT INTO chats VALUES ('old', 'Water', 'claude', 'sess-1', 1, 2);
                 INSERT INTO messages (chat, role, text, at) VALUES ('old', 'user', 'log a glass', 2);
                 INSERT INTO memories (text, at) VALUES ('Sam lives in Springfield', 3);",
            )
            .unwrap();
        }
        let core = Core::open(&dir).unwrap();
        let echo = core.first_agent();
        assert_eq!((echo.name.as_str(), echo.form.as_str(), echo.color.as_str()), ("Echo", "blob", "teal"));
        assert_eq!(core.agents().unwrap().len(), 1);
        let chat = core.chat("old").unwrap().unwrap();
        assert_eq!((chat.agent.as_str(), chat.session.as_deref()), (echo.id.as_str(), Some("sess-1")));
        let ms = core.memories(Some(&echo.id)).unwrap();
        assert_eq!(ms.len(), 1);
        assert!(ms[0].shared, "old memories were about the user, so every agent shares them");
        assert_eq!(chat.unread, 0, "replies from before the migration count as seen");
        assert_eq!(echo.finish, "soft", "an old profile gets the default finish");
        assert_eq!(core.onboarding(), "done");
        drop(core);
        // Opening again changes nothing.
        let core = Core::open(&dir).unwrap();
        assert_eq!(core.agents().unwrap().len(), 1);
        assert_eq!(core.first_agent().id, echo.id);
        drop(core);
        let _ = std::fs::remove_dir_all(dir);
    }
}
