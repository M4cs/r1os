//! What each agent can and can't do, for its prompt and `list_agents` (Max, 2026-10-11: "agents are always aware of
//! their capabilities, and if they can't for example open a browser because they're on the R1 device, they should be
//! able to ask to offload a task to another agent that exists with the proper capabilities").
//!
//! - **Where it works:** the R1 (a sandboxed shell, the web, the R1's own browser (ADR-0054), the R1's own tools) or a
//!   linked computer (its folders, commands, the user's Chrome; ADR-0053), each browser unless its `browser` level is
//!   never.
//! - **Its connected apps** (ADR-0052): a Claude agent on the R1 has the claude.ai connectors that are connected and not
//!   turned off for it; a Codex agent has whatever the user connected on ChatGPT. Agents on a computer have none there.
//! - **Handing over:** what it can't do, which other agent can, and to ask them with `ask_agent` (crate::mail), or say
//!   what would make it possible (connect the app, open Chrome, link the computer).

use crate::core::Agent;
use crate::hub::Hub;

/// What one agent can reach.
pub struct Can {
    /// The computer it works on, by name ("Sam's PC").
    pub pc: Option<String>,
    /// The user's own Chrome there (ADR-0053).
    pub browser: bool,
    /// An agent on the R1: the Android apps installed there, in the app window (ADR-0061).
    pub android: bool,
    /// Its connected apps, by name ("Gmail").
    pub apps: Vec<String>,
    /// Ones the user added that need signing in first.
    pub sign_in: Vec<String>,
    /// Codex: whatever the user connected on ChatGPT.
    pub chatgpt: bool,
}

pub fn of(hub: &Hub, a: &Agent) -> Can {
    // Not whether it's linked this moment: the link comes up with the R1's screen, and a task handed over waits for it.
    let pc = a.computer().map(|key| crate::remote::computer_name(hub, key).unwrap_or_else(|| "the user's computer".into()));
    // A computer's agent: the user's Chrome there (Claude only). One on the R1: the R1's own browser (ADR-0054).
    let browser = hub.core.permission(&a.id, "browser") != "never" && (pc.is_none() || a.brain == "claude");
    let android = pc.is_none() && hub.core.permission(&a.id, "android") != "never";
    let (mut apps, mut sign_in) = (Vec::new(), Vec::new());
    if a.brain == "claude" && pc.is_none() {
        let off = hub.core.with(|db| crate::connections::off(db, &a.id)).unwrap_or_default();
        for s in crate::connections::servers(hub).into_iter().filter(|s| !off.contains(&s.name)) {
            let label = crate::connections::label(&s.name).to_owned();
            match s.status.as_str() {
                "connected" | "pending" => apps.push(label),
                "needs-auth" => sign_in.push(label),
                _ => {}
            }
        }
    }
    Can { pc, browser, android, apps, sign_in, chatgpt: a.brain == "codex" && a.computer().is_none() }
}

/// One line about what an agent can reach: "on Sam's PC: its files and commands, and the user's Chrome".
pub fn line(hub: &Hub, a: &Agent) -> String {
    let c = of(hub, a);
    let mut s = match &c.pc {
        Some(name) => format!(
            "works on {name} (while it's on): files in its folders, commands{}",
            if c.browser { ", and the user's own Chrome browser" } else { "" }
        ),
        None => format!(
            "works on the R1: a sandboxed shell and files, the web, {}{}and the R1's own tools (apps, camera, alarms, music…)",
            if c.browser { "the R1's own browser (it asks the user to sign in to sites themselves), " } else { "" },
            if c.android { "the Android apps installed on the R1, in an app window it drives while the user watches (the user approves each app, and every order, ride or payment, and answers Android's permission prompts and sign-ins themselves), " } else { "" }
        ),
    };
    if !c.apps.is_empty() {
        s.push_str(&format!("; connected apps: {}", c.apps.join(", ")));
    } else if c.chatgpt {
        s.push_str("; the apps on the user's ChatGPT account");
    }
    s
}

/// For an agent's prompt: what it can do, what it can't and who can, and how to hand a task over.
pub fn block(hub: &Hub, me: &Agent) -> String {
    let c = of(hub, me);
    let others: Vec<Agent> = hub.core.agents().unwrap_or_default().into_iter().filter(|a| a.id != me.id).collect();
    let mut cant = Vec::new();
    if !c.browser {
        cant.push("a web browser (it's turned off for you)");
    }
    if c.pc.is_none() {
        cant.push("the user's computer and their own signed-in browser there: its files, programs and apps");
    } else {
        cant.push("the user's connected apps (Gmail, Calendar…): they're the R1 agents'");
    }
    let mut out = format!("What you can do: you {}.\n", line(hub, me).replacen("works", "work", 1));
    if !c.sign_in.is_empty() {
        out.push_str(&format!(
            "Connected but not signed in yet: {} (the user signs in from Settings › Connections on the R1).\n",
            c.sign_in.join(", ")
        ));
    }
    out.push_str(&format!("What you can't: {}.\n", cant.join("; ")));
    if !others.is_empty() {
        out.push_str("What the other agents can do:\n");
        for o in &others {
            out.push_str(&format!("- {}: {}\n", o.name, line(hub, o)));
        }
    }
    out.push_str(
        "When a task needs something you can't do and another agent can, hand it over: ask_agent, with the task written \
         so it stands alone (what to do, for the user, and what to send back), and tell the user who you've asked; their \
         answer comes back to this chat for you to pass on. When nobody can, say what would make it possible (connecting \
         the app in Settings › Connections, opening Chrome on the PC, linking the computer, or giving an agent the \
         computer in its space). Never just say you can't.",
    );
    out
}
