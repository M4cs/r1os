//! The agent's tools on the R1: a minimal MCP server on r1d (`POST /mcp`, JSON-RPC 2.0 over HTTP;
//! ADR-0007, ADR-0010).
//!
//! An agent process gets a bearer token mapped to its agent and chat. The agent CLI in the box
//! reaches http://127.0.0.1:7171/mcp (the box shares Android's network) with it. Besides its CLI's
//! own tools (web, files and a shell inside the jail), these are how it acts on the R1: build, edit,
//! use and manage the user's apps, open them, remember things (docs/specs/agents.md §5), and message
//! the user's other agents (§8, crate::mail). A turn another agent's message started can't use the
//! tools that delete or change things (crate::mail::refused). Changing the R1's look, its settings or an app's
//! settings and permissions asks the user on the R1 first, unless they've allowed that agent (crate::approvals).

use crate::hub::{Event, Hub, Source};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Who's calling a tool: the agent, and the chat whose turn it is ("" for a spare process that
/// hasn't been given a chat yet).
#[derive(Debug, Clone)]
pub struct Caller {
    pub agent: String,
    pub chat: String,
}

/// Live tool tokens → their caller.
#[derive(Default)]
pub struct Tokens(Mutex<HashMap<String, Caller>>);

impl Tokens {
    pub fn issue(&self, agent: &str, chat: &str) -> String {
        let t = crate::core::random_hex(16);
        self.0.lock().unwrap().insert(t.clone(), Caller { agent: agent.to_owned(), chat: chat.to_owned() });
        t
    }

    pub fn revoke(&self, token: &str) {
        self.0.lock().unwrap().remove(token);
    }

    /// A warm process started before its chat existed (a new chat) now belongs to `chat`.
    pub fn rebind(&self, token: &str, chat: &str) {
        if let Some(c) = self.0.lock().unwrap().get_mut(token) {
            chat.clone_into(&mut c.chat);
        }
    }

    pub fn caller(&self, token: &str) -> Option<Caller> {
        self.0.lock().unwrap().get(token).cloned()
    }
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({ "name": name, "description": description, "inputSchema": { "type": "object", "properties": properties, "required": required } })
}

fn tools() -> Value {
    let id = json!({ "type": "string", "description": "The app's id, from list_apps." });
    let big = json!({
        "type": "boolean",
        "description": "true for an app that needs more work: several views, real logic (calculations, combining web data, rules), or a lot of data. The build takes longer (up to about 10 minutes) and does more. Tell the user it's a bigger build."
    });
    let model = json!({
        "type": "string",
        "enum": crate::core::BUILD_MODELS,
        "description": "Only when the user asks for a model for this build; otherwise leave it out (the R1's setting)."
    });
    json!([
        tool("list_apps", "The user's apps on the R1: id, name, description. Newest or most recently changed first.", json!({}), &[]),
        tool(
            "describe_app",
            "One app in full: its actions (with their params, and which ask for confirmation), its queries, its data tables and its spec. Read this before using or editing an app you haven't looked at in this conversation.",
            json!({ "id": id }),
            &["id"],
        ),
        tool(
            "query_app",
            "Read an app's data with one of its named queries (describe_app lists them), e.g. today's total. Returns up to 50 rows.",
            json!({ "id": id, "query": { "type": "string" }, "params": { "type": "object", "description": "The query's params, if it has any." } }),
            &["id", "query"],
        ),
        tool(
            "run_action",
            "Do something in an app without opening it, e.g. log 2 glasses of water: runs one of its actions (describe_app lists them) with params. Actions marked confirm (deleting things) need the user's yes first: ask, then call again with confirmed: true. The user can undo it with undo_action.",
            json!({ "id": id, "action": { "type": "string" }, "params": { "type": "object" }, "confirmed": { "type": "boolean" } }),
            &["id", "action"],
        ),
        tool("undo_action", "Undo the last run_action in an app (within 10 minutes), when the user asks to undo it.", json!({ "id": id }), &["id"]),
        tool("open_app", "Open one of the user's apps on the R1's screen.", json!({ "id": id }), &["id"]),
        tool(
            "control_media",
            "What the R1 is playing (a station one of the user's r1os apps started, or music in an Android app like Spotify), and pause, resume, stop, skip to the next or go back to the previous. Without `op`, just says what's on.",
            json!({ "op": { "type": "string", "enum": ["pause", "resume", "stop", "toggle", "next", "previous"] } }),
            &[],
        ),
        tool(
            "play_music",
            "Play music, an artist, an album, a playlist or a podcast in one of the user's music apps on the R1 (Spotify, YouTube Music, Pocket Casts…). With `link` (an open.spotify.com or music.youtube.com page for it, found with a web search) it starts playing at once and r1os comes back; with only `query`, the app searches, and Spotify then just shows its results. Name the app only if the user did.",
            json!({
                "query": { "type": "string", "description": "What to play, as the user said it (\"daft punk\", \"lo-fi beats\", \"the latest hard fork episode\")." },
                "link": { "type": "string", "description": "Its page in the app: https://open.spotify.com/(track|album|playlist|artist|episode|show)/<id>, or a music.youtube.com link." },
                "app": { "type": "string", "description": "The music app's name, if the user named one." }
            }),
            &["query"],
        ),
        tool(
            "list_notifications",
            "The user's notifications from Android apps (messages, deliveries, reminders), newest first: app, title, text, when, and a key. Only what the apps posted and the user hasn't cleared.",
            json!({ "app": { "type": "string", "description": "Only this app's (its name, e.g. Signal)." } }),
            &[],
        ),
        tool(
            "open_notification",
            "Open a notification (list_notifications has the keys): its app comes up on the screen, where it was (a chat, a delivery).",
            json!({ "key": { "type": "string" } }),
            &["key"],
        ),
        tool(
            "reply_notification",
            "Reply to a message notification that has a reply box (can_reply in list_notifications), as typing in it would: the app sends it. Only when the user asks you to reply, with their words; if you're unsure what to send, ask first.",
            json!({ "key": { "type": "string" }, "text": { "type": "string", "description": "The reply, as the user wants it sent." } }),
            &["key", "text"],
        ),
        tool(
            "dismiss_notification",
            "Clear a notification (list_notifications has the keys), when the user asks.",
            json!({ "key": { "type": "string" } }),
            &["key"],
        ),
        tool(
            "list_android_apps",
            "The Android apps installed on the R1 (from Aurora Store, its app store): name and package. Separate from the user's own apps (list_apps).",
            json!({}),
            &[],
        ),
        tool(
            "open_android_app",
            "Open an installed Android app full screen on the R1 for the user (list_android_apps has the packages). With `link`, it opens on that page, ready for them: an app's own link (uber://?action=setPickup&pickup=my_location&dropoff[formatted_address]=…, spotify:album:…) or a web address the app opens. The user comes back to r1os with a double click.",
            json!({
                "package": { "type": "string", "description": "Its package, e.g. com.spotify.music." },
                "link": { "type": "string", "description": "An app's own link, or a web address an installed app opens; `package` can then be left out." }
            }),
            &[],
        ),
        tool(
            "show_android_app",
            "Open Aurora Store, the R1's app store for Android apps, on an app's page, where the user installs it with one tap (or opens or updates it). Needs the app's package name: if you're not sure of it, look it up on the web (the app's Google Play link has it after ?id=). Without a package, opens the store's home page.",
            json!({ "package": { "type": "string", "description": "The app's package, e.g. org.thoughtcrime.securesms for Signal." } }),
            &[],
        ),
        // The app window (crate::appwin, ADR-0061): an installed Android app you drive while the user watches.
        tool(
            "app_open",
            "Open an installed Android app (list_android_apps has the packages) in the app window: a screen of its own on the R1 that you drive and the user watches in your chat. Use it when there's no connector for the job, for things like finding a ride, building a food order or searching an app. An app already open on the R1's own screen comes into the window as it was. With `link`, the app opens on that page (spotify:search:rabbit%20hole, a web address the app claims); `package` can then be left out. The first time agents use an app, the user approves it on the R1. Returns its screen as lines with refs.",
            json!({
                "package": { "type": "string" },
                "link": { "type": "string", "description": "An app's own link, or a web address an installed app opens." },
                "restart": { "type": "boolean", "description": "Close the app first and start it afresh (it loses its place): only for an app that says it's offline while the R1 isn't, after app_wait didn't bring it back." }
            }),
            &[],
        ),
        tool("app_read", "The app window's screen as lines (role, words, state) with refs like [a12] for what you can tap or type into, and its picture. The lines come from what the app tells Android: a web page inside an app can lag behind the picture or still list what it hid. When they disagree, go by the picture (app_click at x,y). \"(off screen)\" marks what needs scrolling to, though a tap on it brings it in. A secure screen (a card number, a password) is withheld: ask the user to take over (app_ask_user).", json!({}), &[]),
        tool("app_screenshot", "A picture of the app window, as the user sees it (600x1200). Read it for what app_read can't name (pictures, maps, unlabelled buttons). A secure screen comes out black: hand those to the user (app_ask_user).", json!({}), &[]),
        tool(
            "app_click",
            "Tap an element in the app window by its ref (app_read), by its words (target: \"Search\"; the words on screen work best), or at x,y on the 600x1200 screen. It's tapped where it is now, brought into view first if it's mostly off the screen. It answers once the screen has settled (waiting out loading, up to 6 s), so don't send app_wait with it: only after an answer that says it's still loading. The first tap, typing or key in a task asks the user on the R1. A final step (placing an order, requesting a ride, paying, subscribing, sending a message) always asks the user, with a card that shows what's on the screen: set final: true for those; never press one the user didn't ask for.",
            json!({ "ref": { "type": "string" }, "target": { "type": "string", "description": "The element in words, when you don't have its ref." }, "x": { "type": "number" }, "y": { "type": "number" }, "final": { "type": "boolean", "description": "true for the step that commits: places an order, books, pays, subscribes or sends. Adding to a cart, opening a cart or going to checkout isn't one: it asks on the R1 every time." } }),
            &[],
        ),
        tool(
            "app_type",
            "Type into a field in the app window by its ref, by its words (target), or the focused one, replacing what's in it; submit: true presses Enter after (in a web page inside the app, the words go in as real keys). Never a password, card number or one-time code: the user types those (app_ask_user).",
            json!({ "ref": { "type": "string" }, "target": { "type": "string" }, "text": { "type": "string" }, "submit": { "type": "boolean" } }),
            &["text"],
        ),
        tool("app_scroll", "Scroll the app window's screen (or a list in it, by ref): up, down, left or right.", json!({ "direction": { "type": "string", "enum": ["up", "down", "left", "right"] }, "ref": { "type": "string" } }), &["direction"]),
        tool("app_back", "Android's back in the app window.", json!({}), &[]),
        tool(
            "app_find",
            "Scroll the app window (or the list `ref`) until something matching `target` shows, up to 8 scrolls, and return that screen with its ref: one call instead of scrolling and reading yourself. Matched by words, then by meaning, on the R1.",
            json!({ "target": { "type": "string" }, "direction": { "type": "string", "enum": ["down", "up", "left", "right"] }, "ref": { "type": "string" } }),
            &["target"],
        ),
        tool("app_wait", "Wait until the app window's screen settles (loading and animations done), up to `seconds` (default 3), then returns it like app_read. Only when an answer says the screen is still loading, or the picture shows it loading: every tap, typing and scroll already waits for its screen to settle.", json!({ "seconds": { "type": "number" } }), &[]),
        tool(
            "app_ask_user",
            "Hand the app to the user for something only they should do: signing in, a password or code, a card or payment screen, a CAPTCHA, Android's permission prompts (location, camera, notifications: never answer those yourself), or a choice only they can make. The R1 shows the app full screen with your words and waits (up to 5 minutes) until they say done; then read the screen again and carry on.",
            json!({ "why": { "type": "string", "description": "What they need to do, in a few words: \"Sign in to Uber\"." } }),
            &["why"],
        ),
        tool(
            "app_close",
            "Close the app window when you're done (the app's own state, like a cart, stays in the app). When the user wants the app themselves (\"let me use it\"), for_user: true moves it onto the R1's own screen as it is, instead of closing it.",
            json!({ "for_user": { "type": "boolean", "description": "true when the user takes the app to use it themselves." } }),
            &[],
        ),
        tool(
            "app_do",
            "Several app window steps in one call, in order (faster than one call each). App window tools run one at a time in the order sent, so read each answer before choosing what's next rather than sending several at once. Each step is {tool, ...its arguments} with tool one of open, click, type, scroll, back, read, wait. Name elements by their words (target) where you don't know their refs yet: [{tool: click, target: \"Search\"}, {tool: type, target: \"What do you want to listen to\", text, submit: true}]. Stops at the first that fails; returns each step's result and the last screen. Steps before the last don't read the screen (each says what it did, and if nothing changed), so batch what you already know: a form's fields, the choices on an item sheet, a search and its submit. Each act asks as alone.",
            json!({ "steps": { "type": "array", "items": { "type": "object" }, "description": "Up to 10 steps." } }),
            &["steps"],
        ),
        tool(
            "create_app",
            "Build a new app on the R1 from a brief: whatever the user asks for, e.g. trackers, counters, timers, logs, lists, converters, apps with live data from the web (weather, prices, rates, news feeds), wheel-driven tools and small games. Also widgets: a card on the glance screen (the screen the R1 wakes to) showing something at a glance, like the weather, today's water or the headlines; say it's a widget in the brief. Usually 10–30 seconds, longer for a bigger app; it then appears in the launcher (and a widget on the glance screen). An app that uses the web asks the user's permission the first time it opens.",
            json!({
                "brief": { "type": "string", "description": "What the app does, in the user's words, with any details they gave." },
                "model": model.clone(),
                "big": big.clone()
            }),
            &["brief"],
        ),
        tool(
            "edit_app",
            "Change one of the user's apps as they ask: add or remove features, fields, views, change how it looks or its name. Takes 10–30 seconds. Data in removed fields is deleted (undo_app_change brings it back).",
            json!({
                "id": id,
                "request": { "type": "string", "description": "The change, in the user's words, with every detail they gave." },
                "model": model.clone(),
                "big": big.clone()
            }),
            &["id", "request"],
        ),
        tool(
            "fix_app",
            "Repair an app that went wrong on the R1: rebuilds it with the problems its page reported (describe_app lists them). Takes 10–30 seconds. Use it when the user says an app is broken or an action failed.",
            json!({ "id": id, "model": model }),
            &["id"],
        ),
        tool(
            "set_app_settings",
            "Change an app's settings, e.g. the ticker a stock app shows, the town a weather app reads or a daily goal (describe_app lists them with their current values and allowed options). Use edit_app for anything else. It asks the user on the R1 first, unless they've allowed you (device_settings says what you may change).",
            json!({ "id": id, "values": { "type": "object", "description": "Setting name → new value." } }),
            &["id", "values"],
        ),
        tool(
            "set_app_permission",
            "Let one of the user's apps do what it asks for (describe_app lists its permissions: get data from a site, send reminders, run on a schedule, open links, copy, play audio), or stop it. Letting it always asks the user on the R1 (a permission is theirs to grant); stopping it asks unless they've allowed you. Use it when the user wants an app to work that's waiting for permissions, or asks to stop one. Give every permission the feature needs in one call: the user answers once, on one card.",
            json!({
                "id": id,
                "permissions": { "type": "array", "items": { "type": "string" }, "description": "Their ids, from describe_app (e.g. [\"notify\", \"schedule\"])." },
                "allow": { "type": "boolean" },
                "why": { "type": "string", "description": "A few words on why, shown on the R1's card." }
            }),
            &["id", "permissions", "allow"],
        ),
        tool(
            "device_settings",
            "The R1's own settings now and what each can be: the display's brightness (and automatic brightness), how long until the screen sleeps, the volume, the R1's sounds, the time zone, the model app builds use, and whether agents may wake each other. Also says what you may change on the R1 (the look, these settings, apps' settings and permissions) and whether each asks the user first.",
            json!({}),
            &[],
        ),
        tool(
            "battery",
            "The R1's battery now: its level, whether it's charging and from what, the current, and about how long until it's full (charging) or empty (not). Use it when the user asks about the battery or how long until the R1 is charged; give them the estimate as about, never exact.",
            json!({}),
            &[],
        ),
        tool(
            "change_device_settings",
            "Change the R1's own settings when the user asks (\"dim the screen\", \"turn the volume up\", \"keep the screen on longer\", \"no clicks\", \"stronger vibration\"). Give only what changes; device_settings lists them and their values. It asks the user on the R1 first, unless they've allowed you. Tell the user to answer on the R1 before you call it.",
            json!({
                "changes": { "type": "object", "description": "Setting → new value, e.g. {\"brightness\": 30} or {\"screen_timeout\": 120, \"sounds\": false}. brightness and volume are 0–100; screen_timeout is 15, 30, 60, 120, 300 or 600 seconds; haptics is off, light, medium or strong, and haptics_wheel true or false." },
                "why": { "type": "string", "description": "A few words on why, shown on the R1's card." }
            }),
            &["changes"],
        ),
        tool(
            "stop_build",
            "Stop the app build or change that's running, when the user asks to stop or cancel it. Nothing is installed.",
            json!({}),
            &[],
        ),
        tool(
            "set_alarm",
            "Set an alarm on the R1: it rings at a time of day (the R1's local time) until the user stops it, once or on the weekdays given. To change one, pass its id (list_alarms). For a note at a time rather than a ring, use set_reminder.",
            json!({
                "time": { "type": "string", "description": "e.g. 7:30, 07:30, 7am, 19:05" },
                "days": { "type": "array", "items": { "type": "string" }, "description": "Weekdays it repeats on (mon…sun, or weekdays, weekends, daily). Leave out to ring once." },
                "label": { "type": "string" },
                "id": { "type": "string", "description": "An alarm to change instead of a new one." },
                "on": { "type": "boolean", "description": "With id and no time: turn that alarm on or off." }
            }),
            &[],
        ),
        tool(
            "set_reminder",
            "Remind the user of something: at a time (the R1 wakes and shows your note at the top of the screen until they've seen it: give at, the R1's local time, or in_minutes), every so often (time with days: \"every weekday at 9, take vitamins\"), or with no time at all, on their to-do list (\"add milk to my list\": text only).",
            json!({
                "text": { "type": "string", "description": "The note, short: what to do (\"Call mom\", \"Buy milk\")." },
                "at": { "type": "string", "description": "Once: YYYY-MM-DD HH:MM, or HH:MM for the next one." },
                "in_minutes": { "type": "number" },
                "time": { "type": "string", "description": "Repeating: the time of day (9:00, 7pm), with days." },
                "days": { "type": "array", "items": { "type": "string" }, "description": "With time: the weekdays it repeats on (mon…sun, weekdays, weekends, daily)." }
            }),
            &["text"],
        ),
        tool(
            "tick_todo",
            "Tick a to-do off the user's list when they've done it (\"I got the milk\"), or put it back (done: false). list_alarms has the ids (todo: true).",
            json!({ "id": { "type": "string" }, "done": { "type": "boolean", "description": "Default true." } }),
            &["id"],
        ),
        tool(
            "list_media",
            "The photos and recordings on the R1 (taken with its camera, recorded with its recorder): ids, when, where from, labels, and recordings' transcripts (cut short). Newest first.",
            json!({
                "kind": { "type": "string", "enum": ["photo", "recording"] },
                "limit": { "type": "number", "description": "At most this many (default 20)." }
            }),
            &[],
        ),
        tool(
            "view_photo",
            "Look at one of the R1's photos (list_media has the ids): you get the image itself.",
            json!({ "id": { "type": "string" } }),
            &["id"],
        ),
        tool(
            "label_media",
            "Give a photo or recording a short label (\"receipt from lunch\", \"standup notes\"), shown in the Camera and Recorder apps.",
            json!({ "id": { "type": "string" }, "label": { "type": "string" } }),
            &["id", "label"],
        ),
        tool(
            "start_recording",
            "Start the R1's recorder (a meeting, a lecture, a voice memo): it records and transcribes on the device until stop_recording or the user stops it in the Recorder app.",
            json!({ "label": { "type": "string", "description": "What it is, short." } }),
            &[],
        ),
        tool("stop_recording", "Stop the R1's recorder. Its transcript is then in read_recording.", json!({}), &[]),
        tool(
            "read_recording",
            "A recording's whole transcript (list_media has the ids), to summarise it or answer questions about it. The transcript is speech recognition's: no punctuation, some words misheard.",
            json!({ "id": { "type": "string" } }),
            &["id"],
        ),
        tool(
            "journal_add",
            "Write in the user's journal on the R1: what they did, thought or want to remember from today, in their words (lightly tidied). Attach a photo or recording with media_id (list_media). For things to remember forever about the user, use remember instead.",
            json!({
                "text": { "type": "string" },
                "media_id": { "type": "string" },
                "at": { "type": "string", "description": "When it happened, if not now: the R1's local time, YYYY-MM-DD HH:MM." }
            }),
            &[],
        ),
        tool(
            "journal_search",
            "Find journal entries with all the given words (in the entry, or in its recording's transcript), optionally between two local dates (YYYY-MM-DD). Newest first.",
            json!({ "query": { "type": "string" }, "from": { "type": "string" }, "to": { "type": "string" } }),
            &["query"],
        ),
        tool(
            "journal_list",
            "The journal's entries from the last few days (default 7), newest first: to recap a day or a week.",
            json!({ "days": { "type": "number" } }),
            &[],
        ),
        tool(
            "set_timer",
            "Start a countdown timer on the R1 (\"ten minutes\", \"90 seconds for the eggs\"): it rings when it's done, even asleep, until the user stops it. pomodoro: true starts focus and break cycles instead (25 min focus, 5 min breaks, a 15 min long break after every 4th round, 4 rounds; pass any to change them); one at a time. With id (list_alarms has them) and action, changes that one: pause, resume, add (minutes; negative takes time off) or skip (a pomodoro's phase). For a time of day use set_alarm; for a note at a time, set_reminder.",
            json!({
                "minutes": { "type": "number", "description": "How long (fractions are fine), or with action add, how much more." },
                "seconds": { "type": "number" },
                "label": { "type": "string", "description": "What it's for, short (\"pasta\")." },
                "pomodoro": { "type": "boolean" },
                "focus": { "type": "number", "description": "A pomodoro's focus rounds, minutes." },
                "short_break": { "type": "number", "description": "Minutes." },
                "long_break": { "type": "number", "description": "Minutes." },
                "every": { "type": "number", "description": "A long break after every this many rounds." },
                "rounds": { "type": "number", "description": "Focus rounds in the set (1–12)." },
                "id": { "type": "string" },
                "action": { "type": "string", "enum": ["pause", "resume", "add", "skip"] }
            }),
            &[],
        ),
        tool(
            "set_routine",
            "Make a routine: you get `prompt` at a time of day (the R1's local time), once or on chosen weekdays, in the chat the user has with you now (their most recent with you), and your reply waits for them there (show: true also wakes the R1 and says so). For \"every weekday at 7:30, tell me the weather and what's on my day\". A run with nothing worth telling can say nothing, so it can be \"only if …\" (\"tell me if it'll rain\"). To change one, pass its id (list_routines); with id and only `on`, it turns it on or off.",
            json!({
                "time": { "type": "string", "description": "e.g. 7:30, 19:05" },
                "days": { "type": "array", "items": { "type": "string" }, "description": "Weekdays it runs on (mon…sun, or weekdays, weekends, daily). Leave out to run once." },
                "title": { "type": "string", "description": "What to call it, short (\"Morning briefing\")." },
                "prompt": { "type": "string", "description": "What you're asked each time, as an instruction to yourself." },
                "show": { "type": "boolean", "description": "Wake the R1 and say it's done (else it waits as unread)." },
                "id": { "type": "string" },
                "on": { "type": "boolean" }
            }),
            &[],
        ),
        tool("list_routines", "The R1's routines (with ids, whose they are, when next), and the local time now.", json!({}), &[]),
        tool("cancel_routine", "Remove a routine (list_routines has the ids).", json!({ "id": { "type": "string" } }), &["id"]),
        // The R1's browser (crate::browser, ADR-0054): like Claude in Chrome's tools.
        tool(
            "browser_open",
            "Open a web page in the R1's own browser (it shows on the R1's screen while the user's looking). For pages WebFetch can't read (they need JavaScript) and for doing things on a site. Returns its title and first words. Public web pages only.",
            json!({ "url": { "type": "string" } }),
            &["url"],
        ),
        tool("browser_read", "The open page as lines (role, name, value) with refs like [r12] for what you can click or type into. filter: interactive (default) or all.", json!({ "filter": { "type": "string", "enum": ["interactive", "all"] } }), &[]),
        tool("browser_text", "The open page's text (its main part), up to 20,000 characters.", json!({}), &[]),
        tool("browser_find", "Elements on the open page whose words match, with refs.", json!({ "query": { "type": "string" } }), &["query"]),
        tool(
            "browser_click",
            "Click an element by its ref (browser_read or browser_find), by its words (target: \"Sign in\", matched on the R1), or at x,y in the page. The first click, typing or key in a task asks the user on the R1.",
            json!({ "ref": { "type": "string" }, "target": { "type": "string", "description": "The element in words, when you don't have its ref." }, "x": { "type": "number" }, "y": { "type": "number" } }),
            &[],
        ),
        tool(
            "browser_type",
            "Type into a field by its ref, by its words (target: \"search\", matched on the R1), or the one that has focus; submit: true presses Enter after. Never a password, card number or code: ask the user (browser_ask_user).",
            json!({ "ref": { "type": "string" }, "target": { "type": "string", "description": "The field in words, when you don't have its ref." }, "text": { "type": "string" }, "submit": { "type": "boolean" } }),
            &["text"],
        ),
        tool("browser_select", "Choose an option in a list (a select) by its ref: the option's value or words.", json!({ "ref": { "type": "string" }, "value": { "type": "string" } }), &["ref", "value"]),
        tool("browser_key", "Press a key on the page: Enter, Tab, Escape, Backspace, Space, ArrowDown, ArrowUp, ArrowLeft, ArrowRight, PageDown, PageUp.", json!({ "key": { "type": "string" } }), &["key"]),
        tool("browser_scroll", "Scroll the open page up or down (amount: screens, default 1).", json!({ "direction": { "type": "string", "enum": ["up", "down"] }, "amount": { "type": "integer" } }), &["direction"]),
        tool("browser_screenshot", "A picture of the open page, as the user would see it.", json!({}), &[]),
        tool("browser_back", "Back to the page before.", json!({}), &[]),
        tool("browser_close", "Close the R1's browser when you're done with it.", json!({}), &[]),
        tool(
            "browser_do",
            "Several browser steps in one call, in order (faster than one call each): each step is {tool, ...its arguments} with tool one of open, click, type, select, key, scroll, back, read, text, find. Name elements by their words (target) where you don't know their refs yet, so a whole flow fits one call: [{tool: open, url}, {tool: type, target: \"search\", text, submit: true}, {tool: text}]. Stops at the first that fails; returns each step's result. The first act asks the user once, as alone.",
            json!({ "steps": { "type": "array", "items": { "type": "object" }, "description": "Up to 10: [{\"tool\": \"type\", \"ref\": \"r3\", \"text\": \"shoes\", \"submit\": true}, {\"tool\": \"read\"}]" } }),
            &["steps"],
        ),
        tool(
            "browser_ask_user",
            "Hand the browser to the user for something only they should do: signing in (\"Log in to Amazon\"), a password or two-factor code, a payment, a CAPTCHA. The R1 shows the page with your words and waits (up to 5 minutes) until they say done; then read the page again and carry on.",
            json!({ "why": { "type": "string", "description": "What they need to do, in a few words: \"Log in to Amazon\"." } }),
            &["why"],
        ),
        tool(
            "add_skill",
            "Add an Agent Skill for all the user's agents, when they ask (\"add the pdf skill from anthropics/skills\", a GitHub link to a skill or a repo of them). `source`: owner/repo, a GitHub URL or folder, a git URL, or a URL to a SKILL.md. If it holds several skills and you name none, you get their list: ask the user which, then call again with `skills`. The user approves on the R1 before anything is added (a skill can run scripts).",
            json!({
                "source": { "type": "string", "description": "Where the skill is: \"anthropics/skills\", \"https://github.com/owner/repo/tree/main/skills/x\"." },
                "skills": { "type": "array", "items": { "type": "string" }, "description": "Which of its skills, by name (when it has several)." },
                "why": { "type": "string" }
            }),
            &["source"],
        ),
        tool(
            "what_you_missed",
            "Android apps' notifications that came while the R1's screen was off, today: what's new since the user last caught up, and the rest of the day's. Each says the connection that can read more of it (Gmail's → the Gmail connector's tools): read the whole email or thread there (reading never asks) before summarising. For \"what did I miss\", \"catch me up\", and morning routines.",
            json!({}),
            &[],
        ),
        tool(
            "permission",
            "Not for you: the CLI asks it before a tool of the user's connected apps runs (reads go ahead; actions ask the user on the R1).",
            json!({
                "tool_name": { "type": "string" },
                "input": { "type": "object" },
                "tool_use_id": { "type": "string" }
            }),
            &["tool_name"],
        ),
        tool(
            "whats_up",
            "What's new and what's next on the R1, in one go: the user's unread replies, reminders that went off and today's, the next alarm, timers, today's routines, journal entries and recordings, Android apps' notifications, and what's playing. For \"catch me up\", \"what did I miss\" and morning briefings.",
            json!({}),
            &[],
        ),
        tool(
            "open_screen",
            &format!(
                "Open one of r1os's own built-in screens on the R1 for the user (they're always there; never say one isn't installed). By id: {}. For an app of the user's, open_app; for an Android app, open_android_app.",
                crate::screens::list()
            ),
            json!({
                "screen": { "type": "string", "description": "Its id (translator, camera, bluetooth…)." },
                "mine": { "type": "string", "description": "The Translator: the user's language, a code or name (en, Spanish)." },
                "theirs": { "type": "string", "description": "The Translator: the other person's language (pt, Portuguese)." }
            }),
            &["screen"],
        ),
        tool("list_alarms", "The R1's alarms, reminders and timers (with ids), its time zone and the local time now.", json!({}), &[]),
        tool("cancel_alarm", "Remove an alarm, a reminder or a timer (list_alarms has the ids).", json!({ "id": { "type": "string" } }), &["id"]),
        tool(
            "stop_alarm",
            "The alarm or timer that's ringing now: stop it, or snooze it (an alarm 10 minutes, a timer a minute more).",
            json!({ "snooze": { "type": "boolean" } }),
            &[],
        ),
        tool(
            "set_time_zone",
            "The R1's time zone follows its network by itself. Set one only when the user says the R1's time is wrong or asks for a zone (it then stops following the network); automatic: true makes it follow the network again. Alarms keep their local times. It asks the user on the R1 first, unless they've allowed you (device_settings says what you may change).",
            json!({
                "zone": { "type": "string", "description": "An IANA time zone id, e.g. America/New_York, Europe/London." },
                "automatic": { "type": "boolean" }
            }),
            &[],
        ),
        tool(
            "install_app",
            "Install an app from its whole spec (EneeSpec YAML): one the user shared, or one at a link they gave (fetch it first). An app with the same id is replaced and keeps its data. Permissions are asked for when it first opens.",
            json!({ "source": { "type": "string", "description": "The whole spec." } }),
            &["source"],
        ),
        tool("app_history", "An app's versions: when it was created and each change since, newest last.", json!({ "id": id }), &["id"]),
        tool(
            "home_apps",
            "The apps pinned to home (after the agents on the wheel, before the launcher). Without `apps`, lists them in order. With `apps`, pins those, in that order: to pin, unpin or reorder (the whole list). A mini-app by its id (list_apps), an Android app as `android:<package>` (list_android_apps), one of r1os's own tools as `tool:<id>` (camera, photos, recorder, journal, translator, browser, whats_up, alarms, reminders, timer, routines, clock). It asks the user on the R1 first, unless they've allowed you (device_settings says what you may change).",
            json!({ "apps": { "type": "array", "items": { "type": "string" }, "description": "In the order they show." } }),
            &[],
        ),
        tool(
            "set_agent_look",
            "Change how an agent looks when the user asks (\"put a crown on Pip\", \"give yourself sunglasses\", \"make Archer purple\"): its shape, colour, finish, eyes, and what it wears. Without `agent`, your own look. Give only what changes; `wear` is the whole list (at most one thing on its head, its face, its cheeks, at its neck and on its back; [] takes it all off). It asks the user on the R1 first, unless they've allowed you (device_settings says what you may change).",
            json!({
                "agent": { "type": "string", "description": "The agent's id or name (list_agents); none: you." },
                "form": { "type": "string", "enum": crate::core::FORMS },
                "color": { "type": "string", "description": format!("One of {}, or any colour as `#rrggbb`.", crate::look::ACCENTS.join(", ")) },
                "finish": { "type": "string", "enum": crate::core::FINISHES },
                "eyes": { "type": "string", "enum": crate::wardrobe::get().eyes },
                "wear": { "type": "array", "items": { "type": "string" }, "description": crate::wardrobe::get().describe() }
            }),
            &[],
        ),
        tool(
            "set_look",
            "The R1's look, when the user asks to change it: dark or light mode, the accent color, the font, each screen's background, and the glance screen (the screen it wakes to): its clock, where the time sits, where the agent peeks up, and what it shows. Give only what changes; without `change`, says what the look is now. Colours are an accent's name or any `#rrggbb` (the accent too). Backgrounds: `{kind: plain|tint|gradient|mesh|texture|photo, color?, colors?, style?, grain?, flow?, photo?, dim?}`: `mesh` is a soft, grainy gradient of one to three `colors` (palettes: sunset, lagoon, aurora, peach, dusk, forest, berry, ink, or any colours); `grain` 0–1 is how much fine grain is over it; `flow: true` lets it drift with the wheel; a gradient's style: glow, dusk, diagonal, horizon; a texture's: grain, noise; a photo needs its id from the R1's photos; `dim` 0–0.85 darkens a photo or a soft gradient. A change asks the user on the R1 first, unless they've allowed you (device_settings says what you may change).",
            json!({ "change": {
                "type": "object",
                "description": "Only what changes, e.g. {\"accent\": \"purple\"} or {\"glance\": {\"clock\": \"dial\", \"agent\": \"right\"}} or {\"home\": {\"background\": {\"kind\": \"gradient\", \"style\": \"dusk\"}}}.",
                "properties": {
                    "mode": { "type": "string", "enum": crate::look::MODES },
                    "accent": { "type": "string", "description": "An accent's name (orange, amber, yellow, lime, green, teal, cyan, blue, indigo, purple, pink, red) or any #rrggbb." },
                    "font": { "type": "string", "enum": crate::look::FONTS, "description": "rubik (r1os's own), roboto, flex (Roboto Flex), source (Source Sans), serif (Noto Serif), mono, typewriter, casual (handwritten), script, smallcaps" },
                    "glance": {
                        "type": "object",
                        "properties": {
                            "background": { "type": "object" },
                            "clock": { "type": "string", "enum": crate::look::CLOCKS, "description": "words: the time in words; dial: a clock face" },
                            "align": { "type": "string", "enum": crate::look::ALIGNS },
                            "agent": { "type": "string", "enum": crate::look::PLACES },
                            "hide": { "type": "array", "items": { "type": "string", "enum": crate::look::PARTS }, "description": "What the glance screen leaves out (the whole list)." }
                        }
                    },
                    "home": { "type": "object", "properties": { "background": { "type": "object" } } }
                }
            } }),
            &[],
        ),
        tool(
            "glance_widgets",
            "The widgets on the glance screen, the screen the R1 wakes to: apps' glance cards, one at a time (turning the wheel down shows the next). Without `apps`, lists them in order, with what each shows. With `apps`, shows those apps' cards in that order: to add, remove or reorder them (a widget taken off stays an app). A new app with a glance card joins by itself; edit_app gives an app one (\"add a glance card showing …\"). A change asks the user on the R1 first, unless they've allowed you (device_settings says what you may change).",
            json!({ "apps": { "type": "array", "items": { "type": "string" }, "description": "App ids, in the order to show them." } }),
            &[],
        ),
        tool(
            "undo_app_change",
            "Put an app back to its version before the last change (edit), bringing back data that change deleted. Only when the user asks.",
            json!({ "id": id }),
            &["id"],
        ),
        tool(
            "delete_app",
            "Remove an app. It goes to the trash with its data and can be restored for 7 days. Ask the user first, then call with confirmed: true.",
            json!({ "id": id, "confirmed": { "type": "boolean" } }),
            &["id", "confirmed"],
        ),
        tool(
            "restore_app",
            "Bring back an app the user removed in the last 7 days, with its data. Without an id, lists what's in the trash.",
            json!({ "id": { "type": "string" } }),
            &[],
        ),
        tool(
            "remember",
            "Remember a short fact for every future conversation; one fact per call, only things the user would want kept. \
             shared: true for facts about the user that every agent on the R1 should know (their name, people, places, \
             preferences, routines). Leave it false for your own notes: what you're working on with them, how they like you \
             to do your job.",
            json!({ "text": { "type": "string" }, "shared": { "type": "boolean" } }),
            &["text"],
        ),
        tool("list_memories", "Everything you remember, with ids: the facts every agent shares, then your own.", json!({}), &[]),
        tool("forget", "Forget one memory by id (list_memories), when the user asks.", json!({ "id": { "type": "integer" } }), &["id"]),
        tool(
            "read_feed",
            "Read an RSS or Atom feed (news, blogs, podcasts, releases): its newest items, with titles, dates, links, pictures and \
             short summaries. For headlines and news, read a news feed rather than fetching a site's front page, which many news \
             sites refuse to AI fetchers. Some: NPR https://feeds.npr.org/1001/rss.xml, BBC \
             https://feeds.bbci.co.uk/news/rss.xml (world: /news/world/rss.xml), New York Times \
             https://rss.nytimes.com/services/xml/rss/nyt/HomePage.xml, The Guardian https://www.theguardian.com/world/rss. \
             Name the source when you tell the user.",
            json!({
                "url": { "type": "string", "description": "The feed's https URL." },
                "limit": { "type": "number", "description": "At most this many items (default 10, up to 30)." }
            }),
            &["url"],
        ),
        tool(
            "show_card",
            "Show a card in the chat with your answer, drawn natively on the R1's screen: headlines, a forecast, numbers, a small chart, \
             steps or a picture. Then answer in a sentence or two: the card holds the details, links and sources, so don't repeat them. \
             `card` is one of: \
             - headlines {title?, items: 1-8 of {title, tagline?, source?, time?, url?, image?}}: news, search results, picks. Give \
             each story its url, the link read_feed or the web gave you (r1d adds a read_feed story's picture itself), so the user \
             can read it; tagline: one line on why it matters; image: an https picture URL, if you have one. \
             - link {title, url, source?, summary?, image?}: one page worth reading. \
             - weather {place, temp, condition, unit: C or F, high?, low?, feels?, note?, hours?: up to 8 of {time, temp, condition}, \
             days?: up to 7 of {day, high, low, condition}}. \
             - image {url (https), caption?, source?, link?}. \
             - stats {title?, items: 1-4 of {label, value, delta?, trend?: up, down or flat}}: prices, scores, totals. \
             - bars {title?, unit?, items: up to 10 of {label, value (0 or more)}}: amounts side by side. \
             - line {title?, unit?, points: 2-40 of {label, value}}: a trend over time. \
             - steps {title?, items: 2-8 of {title, detail?}}: how to do something, or how something flows. \
             - table {title?, columns: 1-3 headings, rows: up to 10, each a list of cells}. \
             The user clicks a card to see it whole and open its links. One card per answer, two at most.",
            json!({ "card": { "type": "object", "description": "The card: `type` (headlines, link, weather, image, stats, bars, line, steps or table) and its fields." } }),
            &["card"],
        ),
        tool(
            "list_agents",
            "The user's agents on the R1: names, what each is for, which one is you, and who's busy.",
            json!({}),
            &[],
        ),
        tool(
            "ask_agent",
            "Ask another of the user's agents something, or hand them a task: when it's their area (what they're for), when \
             it needs what they can do and you can't (a browser, the user's computer, an app you aren't connected to: \
             list_agents says what each can do), or when the user asks you to. Returns at once; it wakes them, and their answer comes back to this chat later as a message, with a turn for \
             you to pass it on. Write the question so it stands alone: they can't see this conversation.",
            json!({
                "to": { "type": "string", "description": "Their name (list_agents)." },
                "text": { "type": "string", "description": "The question, with what they need to answer it." }
            }),
            &["to", "text"],
        ),
        tool(
            "tell_agent",
            "Leave another of the user's agents a note without waking them: something they should know next time they talk \
             with the user (\"The user's sister is visiting Saturday\"). It costs nothing; nothing comes back.",
            json!({
                "to": { "type": "string", "description": "Their name (list_agents)." },
                "text": { "type": "string" }
            }),
            &["to", "text"],
        ),
    ])
}

pub async fn handle(State(hub): State<Arc<Hub>>, headers: HeaderMap, Json(req): Json<Value>) -> Response {
    let token = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    let Some(caller) = token.and_then(|t| hub.tool_tokens.caller(t)) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match answer(&hub, &caller, &req).await {
        Some(body) => Json(body).into_response(),
        None => StatusCode::ACCEPTED.into_response(), // a notification (e.g. notifications/initialized)
    }
}

/// A Codex turn's PreToolUse hook (crate::connections::codex_hook, ADR-0058), as the turn's tool token: what the tool
/// is and gets. Nothing back lets it run; a deny says why.
pub async fn hook(State(hub): State<Arc<Hub>>, headers: HeaderMap, Json(req): Json<Value>) -> Response {
    let token = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
    let Some(caller) = token.and_then(|t| hub.tool_tokens.caller(t)) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let tool = req["tool_name"].as_str().unwrap_or_default();
    let args = json!({ "tool_name": tool, "input": req["tool_input"] });
    let verdict = crate::connections::permission(&hub, &caller.agent, &caller.chat, &args).await;
    tracing::info!(agent = %caller.agent, tool, allowed = verdict["behavior"] == "allow", "codex hook");
    match crate::connections::hook_answer(&verdict) {
        Some(deny) => Json(deny).into_response(),
        None => StatusCode::OK.into_response(),
    }
}

/// One JSON-RPC request from an agent's CLI, as `caller`: its answer, or none for a notification. The R1's own CLIs
/// reach it at `/mcp`; an agent on a linked computer through the link (crate::link).
pub async fn answer(hub: &Arc<Hub>, caller: &Caller, req: &Value) -> Option<Value> {
    let hub = hub.clone();
    let caller = caller.clone();
    let id = req.get("id").cloned()?;
    let method = req["method"].as_str().unwrap_or_default();
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": req["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "r1os", "version": env!("CARGO_PKG_VERSION") },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => Ok(call(&hub, &caller, req["params"]["name"].as_str().unwrap_or(""), &req["params"]["arguments"]).await),
        other => Err(json!({ "code": -32601, "message": format!("method not found: {other}") })),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
    })
}

fn text(t: impl Into<String>, error: bool) -> Value {
    json!({ "content": [ { "type": "text", "text": t.into() } ], "isError": error })
}

/// Asks the user on the R1 unless this agent may go ahead (crate::approvals): `Some(refusal)` when it can't.
async fn denied(hub: &Arc<Hub>, caller: &Caller, ask: crate::approvals::Ask<'_>) -> Option<Value> {
    hub.approvals.gate(hub, &caller.agent, ask.chat(&caller.chat)).await.err().map(|why| text(why, true))
}

/// Apps by name for the card: "Weather, Water".
fn app_names(hub: &Hub, ids: &[String]) -> String {
    if ids.is_empty() {
        return "none".into();
    }
    let name = |p: &String| match (hub.apps.get(p), p.strip_prefix("android:")) {
        (Some(i), _) => i.app.app.name.clone(),
        (None, Some(pkg)) => pkg.rsplit('.').next().unwrap_or(pkg).to_owned(),
        _ => p.clone(),
    };
    ids.iter().map(name).collect::<Vec<_>>().join(", ")
}

/// Runs blocking app-store work off the async threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| anyhow::anyhow!("{e}"))?
}

fn arg<'a>(args: &'a Value, key: &str) -> &'a str {
    args[key].as_str().unwrap_or("").trim()
}

fn params(args: &Value) -> serde_json::Map<String, Value> {
    args["params"].as_object().cloned().unwrap_or_default()
}

/// Starts a create or edit build for the caller's chat: a card in the chat now, the outcome stored
/// in it later.
fn start_build(hub: &Arc<Hub>, caller: &Caller, task: crate::create::Task, model: Option<String>, big: bool) -> Value {
    let model = match crate::server::build_model(hub, model) {
        Ok(m) => m,
        Err(e) => return text(e, true),
    };
    let Some(build) = hub.builds.start() else {
        return text("Another app is being built right now. Try again when it's done.", true);
    };
    let edit_of = match &task {
        crate::create::Task::Edit { app, .. } => Some(app.clone()),
        crate::create::Task::Create { .. } => None,
    };
    let chat = caller.chat.as_str();
    hub.builds.in_chat(chat, build, edit_of.clone());
    hub.emit(Event::chat(&caller.agent, chat, "build", None).with_build(build).with_app(edit_of.clone()), Source::Api);
    let what = if edit_of.is_some() { "Started the change" } else { "Started building it" };
    let h = hub.clone();
    let chat = chat.to_owned();
    let agent = caller.agent.clone();
    tokio::spawn(async move {
        let built = crate::create::build(h.clone(), build, task, model, big).await;
        h.builds.finish();
        // Keep the outcome in the chat, so the card (and the home's preview) survive a reload.
        let (text, meta) = match built {
            Some(b) => {
                let name = h.apps.get(&b.app).map_or_else(|| b.app.clone(), |i| i.app.app.name.clone());
                let kind = if edit_of.is_some() { "edit" } else { "create" };
                let text = match (kind, b.said.is_empty()) {
                    ("edit", false) => format!("Updated {name}: {}", b.said),
                    ("edit", true) => format!("Updated {name}"),
                    _ => format!("Built {name}"),
                };
                (text, json!({ "build": build, "stage": "done", "app": b.app, "kind": kind, "deleted": b.deleted }))
            }
            None if h.builds.stop_asked(build) => (
                if edit_of.is_some() { "Stopped the change".to_owned() } else { "Stopped building".to_owned() },
                json!({ "build": build, "stage": "failed", "stopped": true, "app": edit_of }),
            ),
            None => (
                if edit_of.is_some() { "That change didn't work".to_owned() } else { "That build didn't work".to_owned() },
                json!({ "build": build, "stage": "failed", "app": edit_of }),
            ),
        };
        match h.core.add_message(&chat, "build", &text, &meta) {
            Ok(_) => h.unread_changed(&agent, &chat),
            Err(e) => tracing::warn!(chat = %chat, build, error = %e, "couldn't store the build result"),
        }
        h.builds.left_chat(&chat, build);
    });
    text(format!("{what} (build {build}). It usually takes 10–30 seconds; progress shows in this chat."), false)
}

/// One feed for `read_feed`, through the fetcher apps' web data uses: https, public addresses only, a few
/// redirects, 4 MB at most, 10 s. It asks as r1os, not as an AI fetcher, and only for feeds, which publishers
/// make for programs to read.
fn read_feed(fetcher: &crate::fetch::Fetcher, url: &str) -> anyhow::Result<crate::feed::Feed> {
    let r = fetcher.get(url, crate::fetch::Format::Feed, &|_| true).map_err(|e| anyhow::anyhow!("Couldn't read it: {e}."))?;
    anyhow::ensure!((200..300).contains(&r.status), "The feed answered HTTP {}.", r.status);
    crate::feed::parse(&String::from_utf8_lossy(&r.body)).map_err(|e| anyhow::anyhow!("That isn't an RSS or Atom feed ({e})."))
}

/// What describe_app adds about the app as it is on the R1: its settings' current values, what it may do
/// beyond its own data (the user decides that, never an agent), and its web data's latest values.
fn live_status(hub: &Hub, i: &crate::apps::Installed) -> String {
    let app = &i.app;
    let mut out = String::new();
    if !app.settings.is_empty() {
        let values = hub.apps.settings(&app.app.id).unwrap_or_default();
        out.push_str("\n\nSettings (set_app_settings changes them):\n");
        for (key, def) in &app.settings {
            let now = values.get(key).or_else(|| def.get("default")).cloned().unwrap_or(Value::Null);
            let options = def["options"].as_array().map(|o| format!(", one of {}", Value::Array(o.clone()))).unwrap_or_default();
            out.push_str(&format!("- {key} ({}{options}) = {now}\n", def["type"].as_str().unwrap_or("text")));
        }
    }
    let perms = crate::broker::states(&hub.core, app);
    if !perms.is_empty() {
        out.push_str("\nPermissions (the user decides on the app's screen; you can't grant them):\n");
        for p in &perms {
            let state = match p.state {
                "granted" => "allowed",
                "denied" => "not allowed (the user can turn it on in Manage apps)",
                "ask" => "not decided yet: the app asks when it opens",
                _ => "not available on the R1 yet",
            };
            out.push_str(&format!("- {}: {state}\n", p.label));
        }
    }
    let values = hub.services.values(&app.app.id);
    if !values.is_empty() {
        let mut json = serde_json::to_string(&values).unwrap_or_default();
        if json.len() > 2000 {
            let mut cut = 2000;
            while !json.is_char_boundary(cut) {
                cut -= 1;
            }
            json = format!("{}… (cut)", &json[..cut]);
        }
        out.push_str(&format!("\nIts web data, as last fetched (while it was open):\n{json}\n"));
    }
    out
}

/// What describe_app returns: the app's interface first, then its spec.
fn describe(i: &crate::apps::Installed) -> String {
    let app = &i.app;
    let mut out = format!("{} (`{}`): {}\n\nActions:\n", app.app.name, app.app.id, app.app.description.clone().unwrap_or_default());
    for (name, def) in &app.actions {
        let params: Vec<String> = def["params"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, d)| format!("{k}: {}{}", d["type"].as_str().unwrap_or("text"), if d["required"] == true { "" } else { "?" }))
                    .collect()
            })
            .unwrap_or_default();
        let confirm = if def.get("confirm").is_some() { " (asks for confirmation)" } else { "" };
        out.push_str(&format!("- {name}({}){confirm}\n", params.join(", ")));
    }
    out.push_str("\nQueries:\n");
    for q in &app.queries {
        let params: Vec<&str> = q.params.keys().map(String::as_str).collect();
        out.push_str(&format!("- {}({}) → {}\n", q.name, params.join(", "), q.columns.join(", ")));
    }
    out.push_str("\nTables:\n");
    for t in &app.tables {
        out.push_str(&format!("- {}({})\n", t.name, t.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")));
    }
    out.push_str(&format!("\nSpec:\n```yaml\n{}\n```", i.source.trim_end()));
    out
}

async fn call(hub: &Arc<Hub>, caller: &Caller, name: &str, args: &Value) -> Value {
    // A screen takes one step at a time, in the order they came: an agent may send several tools at once, and on one
    // screen they'd interleave. `app_do` and `browser_do` hold it across their steps (which come through call_inner).
    let _turn = if crate::appwin::is_tool(name) {
        Some(hub.app_window.turn.lock().await)
    } else if name.starts_with("browser_") {
        Some(hub.browser.turn.lock().await)
    } else {
        None
    };
    match call_inner(hub, caller, name, args).await {
        Ok(v) => v,
        Err(e) => text(format!("{e:#}"), true),
    }
}

async fn call_inner(hub: &Arc<Hub>, caller: &Caller, name: &str, args: &Value) -> anyhow::Result<Value> {
    // Another agent's message started this turn: nothing that deletes or changes things (ADR-0012).
    if hub.mail.woken(&caller.chat).is_some()
        && let Some(why) = crate::mail::refused(name, args)
    {
        tracing::info!(agent = %caller.agent, tool = name, "refused a tool in a turn another agent started");
        return Ok(text(why, true));
    }
    let id = arg(args, "id").to_owned();
    let installed = || hub.apps.get(&id).ok_or_else(|| anyhow::anyhow!("There's no app `{id}`. Use list_apps."));
    Ok(match name {
        "list_apps" => {
            let apps: Vec<Value> =
                hub.apps.list().into_iter().map(|a| json!({ "id": a.id, "name": a.name, "description": a.description })).collect();
            text(serde_json::to_string(&apps)?, false)
        }
        "describe_app" => {
            let i = installed()?;
            let (h, app) = (hub.clone(), i.clone());
            let live = blocking(move || Ok(live_status(&h, &app))).await?;
            let errors = hub.apps.errors(&id);
            let problems = if errors.is_empty() {
                String::new()
            } else {
                format!("\n\nWhat went wrong on the R1 since its last change (fix_app repairs it):\n{}", crate::apps::fix_request(&errors))
            };
            text(format!("{}{live}{problems}", describe(&i)), false)
        }
        "set_alarm" => {
            let (h, a, agent) = (hub.clone(), args.clone(), caller.agent.clone());
            let set = blocking(move || {
                h.core.with(|db| match (a["id"].as_str(), a["time"].as_str(), a["on"].as_bool()) {
                    (Some(id), None, Some(on)) => {
                        let c = crate::alarms::clock();
                        crate::alarms::turn(db, id, on, c.now_ms(), &|ms| c.offset_ms(ms))
                    }
                    _ => crate::alarms::alarm_from(db, &a, Some(&agent)),
                })
            })
            .await?;
            crate::alarms::changed(hub);
            let c = crate::alarms::clock();
            let when = match set.at {
                Some(at) => format!("next rings {}", crate::alarms::describe_time(at, &|ms| c.offset_ms(ms))),
                None => "off".into(),
            };
            let m = set.minute.unwrap_or(0);
            text(
                format!(
                    "Alarm `{}`: {:02}:{:02}, {}, {when} (the R1's time, {}).",
                    set.id,
                    m / 60,
                    m % 60,
                    crate::alarms::describe_days(set.days),
                    crate::alarms::zone()
                ),
                false,
            )
        }
        "set_reminder" => {
            let (h, a, agent) = (hub.clone(), args.clone(), caller.agent.clone());
            let set = blocking(move || h.core.with(|db| crate::alarms::reminder_from(db, &a, Some(&agent)))).await?;
            crate::alarms::changed(hub);
            let c = crate::alarms::clock();
            if set.is_todo() {
                return Ok(text(format!("On the to-do list (`{}`): \"{}\".", set.id, set.label), false));
            }
            let at = crate::alarms::describe_time(set.at.unwrap_or_default(), &|ms| c.offset_ms(ms));
            let repeats = if set.days != 0 { format!(", then {}", crate::alarms::describe_days(set.days)) } else { String::new() };
            text(format!("Reminder `{}` set for {at}{repeats} (the R1's time, {}).", set.id, crate::alarms::zone()), false)
        }
        "tick_todo" => {
            let (h, id, done) = (hub.clone(), arg(args, "id").to_owned(), args["done"].as_bool().unwrap_or(true));
            let t = blocking(move || h.core.with(|db| crate::alarms::tick(db, &id, done, crate::alarms::clock().now_ms()))).await?;
            crate::alarms::changed(hub);
            text(format!("{} \"{}\".", if t.done.is_some() { "Ticked off" } else { "Back on the list:" }, t.label), false)
        }
        "list_media" => {
            let kind = args["kind"].as_str().filter(|k| matches!(*k, "photo" | "recording")).map(str::to_owned);
            let limit = args["limit"].as_i64().unwrap_or(20).clamp(1, 100);
            let h = hub.clone();
            let items = blocking(move || Ok(h.core.with(|db| crate::media::list(db, kind.as_deref(), limit))?)).await?;
            let c = crate::alarms::clock();
            let list: Vec<Value> = items
                .iter()
                .map(|i| {
                    let mut v = json!({
                        "id": i.id, "kind": i.kind, "when": crate::alarms::describe_time(i.at, &|ms| c.offset_ms(ms)),
                        "source": i.source, "label": i.label,
                    });
                    if let Some(d) = i.duration_ms {
                        v["seconds"] = json!(d / 1000);
                    }
                    if let Some(t) = &i.transcript {
                        v["transcript"] = json!(t.chars().take(400).collect::<String>());
                    }
                    v
                })
                .collect();
            text(serde_json::to_string(&list)?, false)
        }
        "view_photo" => {
            let item = hub.core.with(|db| crate::media::get(db, &id))?.ok_or_else(|| anyhow::anyhow!("There's no photo `{id}`. Use list_media."))?;
            let data = crate::media::photo_b64(&hub.cfg.data, &item)?;
            let c = crate::alarms::clock();
            let about = format!("Photo `{}`, taken {}{}.", item.id, crate::alarms::describe_time(item.at, &|ms| c.offset_ms(ms)),
                if item.label.is_empty() { String::new() } else { format!(", labelled \"{}\"", item.label) });
            json!({ "content": [ { "type": "image", "data": data, "mimeType": item.mime }, { "type": "text", "text": about } ], "isError": false })
        }
        "label_media" => {
            let done = hub.core.with(|db| crate::media::set_label(db, &id, arg(args, "label")))?;
            anyhow::ensure!(done, "There's no photo or recording `{id}`. Use list_media.");
            hub.emit(Event::MediaChanged { kind: String::new() }, Source::Api);
            text("Labelled.", false)
        }
        "start_recording" => {
            let id = crate::recorder::Recorder::start(hub, arg(args, "label"))?;
            text(format!("Recording (`{id}`). It shows in the Recorder app; stop it with stop_recording."), false)
        }
        "stop_recording" => match crate::recorder::Recorder::stop(hub) {
            Some(id) => text(format!("Stopped. The recording is `{id}`; its transcript is ready in a moment (read_recording)."), false),
            None => text("The recorder isn't recording.", false),
        },
        "read_recording" => {
            let item = hub.core.with(|db| crate::media::get(db, &id))?.filter(|i| i.kind == "recording")
                .ok_or_else(|| anyhow::anyhow!("There's no recording `{id}`. Use list_media."))?;
            let c = crate::alarms::clock();
            text(
                format!(
                    "Recording `{}`{}, {} ({} s):
{}",
                    item.id,
                    if item.label.is_empty() { String::new() } else { format!(" \"{}\"", item.label) },
                    crate::alarms::describe_time(item.at, &|ms| c.offset_ms(ms)),
                    item.duration_ms.unwrap_or(0) / 1000,
                    item.transcript.as_deref().filter(|t| !t.is_empty()).unwrap_or("(no words were recognised)")
                ),
                false,
            )
        }
        "journal_add" => {
            let c = crate::alarms::clock();
            let at = match args["at"].as_str().filter(|s| !s.trim().is_empty()) {
                Some(s) => crate::alarms::parse_when(s, c.now_ms(), &|ms| c.offset_ms(ms))
                    .ok_or_else(|| anyhow::anyhow!("at: `{s}` isn't a local time like 2026-10-09 18:30."))?,
                None => c.now_ms(),
            };
            let media = args["media_id"].as_str().filter(|s| !s.is_empty());
            let e = hub.core.with(|db| crate::journal::add(db, arg(args, "text"), media, Some(&caller.agent), at))?;
            hub.emit(Event::JournalChanged, Source::Api);
            text(format!("Written ({}, {}).", e.id, crate::alarms::describe_time(e.at, &|ms| c.offset_ms(ms))), false)
        }
        "journal_search" | "journal_list" => {
            let c = crate::alarms::clock();
            let offset = |ms: i64| c.offset_ms(ms);
            let day = |s: Option<&str>| s.filter(|s| !s.is_empty()).and_then(|s| crate::alarms::parse_when(&format!("{s} 00:00"), c.now_ms(), &offset));
            let entries = if name == "journal_list" {
                let days = args["days"].as_f64().unwrap_or(7.0).clamp(1.0, 90.0);
                #[allow(clippy::cast_possible_truncation)]
                let from = c.now_ms() - (days * 86_400_000.0) as i64;
                hub.core.with(|db| crate::journal::search(db, "", Some(from), None, 100))?
            } else {
                let to = day(args["to"].as_str()).map(|t| t + 86_400_000);
                hub.core.with(|db| crate::journal::search(db, arg(args, "query"), day(args["from"].as_str()), to, 30))?
            };
            let list: Vec<Value> = entries
                .iter()
                .map(|e| json!({ "id": e.id, "when": crate::alarms::describe_time(e.at, &offset), "kind": e.kind, "text": e.text, "media_id": e.media }))
                .collect();
            text(if list.is_empty() { "Nothing found.".to_owned() } else { serde_json::to_string(&list)? }, false)
        }
        "set_routine" => {
            let (h, a, agent) = (hub.clone(), args.clone(), caller.agent.clone());
            let r = blocking(move || {
                let c = crate::alarms::clock();
                h.core.with(|db| crate::routines::from_args(db, &a, &agent, c.now_ms(), &|ms| c.offset_ms(ms)))
            })
            .await?;
            crate::routines::changed(hub);
            let c = crate::alarms::clock();
            let when = r.at.map_or_else(|| "off".to_owned(), |at| format!("next {}", crate::alarms::describe_time(at, &|ms| c.offset_ms(ms))));
            text(
                format!(
                    "Routine `{}` \"{}\": {:02}:{:02}, {}, {when} (the R1's time, {}).",
                    r.id,
                    r.title,
                    r.minute / 60,
                    r.minute % 60,
                    crate::alarms::describe_days(r.days),
                    crate::alarms::zone()
                ),
                false,
            )
        }
        "list_routines" => {
            let h = hub.clone();
            let list = blocking(move || Ok(h.core.with(crate::routines::list)?)).await?;
            let c = crate::alarms::clock();
            let offset = |ms: i64| c.offset_ms(ms);
            let v = json!({ "now": crate::alarms::describe_time(c.now_ms(), &offset), "routines": crate::routines::summary(&list, hub, &offset) });
            text(serde_json::to_string(&v)?, false)
        }
        "cancel_routine" => {
            let (h, id) = (hub.clone(), id.clone());
            let gone = blocking(move || Ok(h.core.with(|db| crate::routines::remove(db, &id))?)).await?;
            anyhow::ensure!(gone, "There's no routine `{}`. Use list_routines.", arg(args, "id"));
            crate::routines::changed(hub);
            text("Removed.", false)
        }
        "browser_do" => {
            let steps = args["steps"].as_array().cloned().unwrap_or_default();
            if steps.is_empty() || steps.len() > 10 {
                return Ok(text("Give 1 to 10 steps.", true));
            }
            let mut out = Vec::new();
            for (i, step) in steps.iter().enumerate() {
                let op = step["tool"].as_str().unwrap_or_default().trim_start_matches("browser_").to_owned();
                if !matches!(op.as_str(), "open" | "click" | "type" | "select" | "key" | "scroll" | "back" | "read" | "text" | "find") {
                    out.push(format!("{}. `{op}` isn't a step: stopped.", i + 1));
                    break;
                }
                let r = Box::pin(call_inner(hub, caller, &format!("browser_{op}"), step)).await?;
                let failed = r["isError"] == true;
                let said = r["content"][0]["text"].as_str().unwrap_or_default().to_owned();
                out.push(format!("{}. {op}: {said}", i + 1));
                if failed {
                    out.push("Stopped there.".into());
                    break;
                }
            }
            text(out.join("\n"), false)
        }
        n if n.starts_with("browser_") => {
            let op = &n["browser_".len()..];
            let mut a = args.clone();
            if op == "open" {
                match crate::browser::allowed_url(arg(args, "url")) {
                    Ok(u) => a["url"] = json!(u),
                    Err(why) => return Ok(text(why, true)),
                }
            }
            // An element named in words instead of a ref: System One finds it on the page (on the R1, no model).
            if matches!(op, "click" | "type" | "select")
                && args["ref"].as_str().is_none_or(str::is_empty)
                && args["x"].is_null()
                && let Some(target) = args["target"].as_str().filter(|t| !t.trim().is_empty())
            {
                match crate::browser::resolve(hub, &caller.agent, &caller.chat, op, target).await {
                    Some(r) => a["ref"] = json!(r),
                    None => return Ok(text(format!("Nothing on the page fits “{target}”: browser_read for its ref."), true)),
                }
            }
            if crate::browser::acts(op) {
                // What it would do, in words for the card: the element's name and the site.
                let d = crate::browser::call(hub, &caller.agent, &caller.chat, "describe", json!({ "ref": a["ref"], "x": a["x"], "y": a["y"] })).await.unwrap_or_default();
                let name = d["name"].as_str().unwrap_or_default();
                if op == "click"
                    && let Some(no) = crate::appwin::dry_refusal(hub, &caller.chat, name, false)
                {
                    return Ok(text(no, true));
                }
                let short = |s: &str| -> String { s.chars().take(60).collect() };
                let what = match op {
                    "click" if !name.is_empty() => format!("click “{}”", short(name)),
                    "click" => "click on the page".to_owned(),
                    "type" => format!("type “{}”{}", short(arg(args, "text")), if name.is_empty() { String::new() } else { format!(" into {}", short(name)) }),
                    "select" => format!("choose “{}”", short(arg(args, "value"))),
                    _ => format!("press {}", arg(args, "key")),
                };
                if let Err(why) = crate::browser::may_act(hub, &caller.agent, &caller.chat, &what, d["site"].as_str().unwrap_or_default()).await {
                    return Ok(text(why, true));
                }
            }
            if op == "ask_user" {
                crate::scheduler::wake_display().await;
            }
            tracing::info!(agent = %caller.agent, op, "browser");
            crate::browser::reply(op, crate::browser::call(hub, &caller.agent, &caller.chat, op, a).await)
        }
        // Skills (crate::skills, T-086): listed without asking; added once the user says yes on the R1.
        "add_skill" => {
            let source = args["source"].as_str().unwrap_or_default().trim().to_owned();
            let mut pick: Vec<String> = args["skills"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
            if pick.is_empty() {
                let found = match crate::skills::in_source(&source).await {
                    Ok(f) => f,
                    Err(e) => return Ok(text(format!("Couldn't read that source: {e:#}"), true)),
                };
                if found.len() > 1 {
                    let list: Vec<String> = found.iter().map(|(n, d)| format!("- {n}: {d}")).collect();
                    return Ok(text(format!("It has {} skills; ask the user which, then call add_skill again with `skills`:\n{}", found.len(), list.join("\n")), false));
                }
                pick = vec![found[0].0.clone()];
            }
            let names = pick.join(", ");
            let ask = crate::approvals::Ask::new(
                "connections",
                format!("add the skill{} {names} for your agents", if pick.len() > 1 { "s" } else { "" }),
                format!("from {source} · a skill can run scripts in the agents' box"),
            )
            .why(args["why"].as_str().unwrap_or_default());
            if let Some(refused) = denied(hub, caller, ask).await {
                return Ok(refused);
            }
            match crate::skills::add(hub, &source, &pick).await {
                Ok(crate::skills::Added::Added(added)) => text(format!("Added {} for every agent. Claude agents can use it now; it's in Settings › Connections on the R1.", added.join(", ")), false),
                Ok(crate::skills::Added::Choose(_)) => text("Name the skills to add.", true),
                Err(e) => text(format!("Couldn't add it: {e:#}"), true),
            }
        }
        "what_you_missed" => {
            let h = hub.clone();
            let v = blocking(move || crate::missed::for_agent(&h)).await?;
            text(serde_json::to_string(&v)?, false)
        }
        // The CLI's permission prompt for a connection's tool (crate::connections, ADR-0052).
        "permission" => {
            let decision = crate::connections::permission(hub, &caller.agent, &caller.chat, args).await;
            tracing::info!(agent = %caller.agent, tool = args["tool_name"].as_str().unwrap_or(""), behavior = decision["behavior"].as_str().unwrap_or(""), "connection permission");
            text(decision.to_string(), false)
        }
        "whats_up" => {
            let h = hub.clone();
            let v = blocking(move || crate::whatsup::gather(&h)).await?;
            text(serde_json::to_string(&v)?, false)
        }
        "open_screen" => {
            let name = arg(args, "screen");
            let s = crate::screens::named(name).ok_or_else(|| anyhow::anyhow!("There's no screen `{name}`: they're {}", crate::screens::list()))?;
            let target = crate::screens::target(s, args["mine"].as_str(), args["theirs"].as_str()).map_err(anyhow::Error::msg)?;
            hub.emit(Event::OpenScreen { target: target.clone() }, Source::Api);
            let langs = match (target["mine"].as_str(), target["theirs"].as_str()) {
                (None, None) => String::new(),
                (m, t) => format!(" ({} ⇄ {})", m.unwrap_or("as it was"), t.unwrap_or("as it was")),
            };
            text(format!("Opened {} on the R1{langs}.", s.id.replace('_', "'s ")), false)
        }
        "set_timer" => {
            let (h, a, agent) = (hub.clone(), args.clone(), caller.agent.clone());
            let now = crate::alarms::clock().now_ms();
            let t = blocking(move || {
                h.core.with(|db| match a["id"].as_str().filter(|s| !s.is_empty()) {
                    Some(id) => crate::timers::op(db, id, a["action"].as_str().unwrap_or_default(), &a, now),
                    None => crate::timers::from_args(db, &a, Some(&agent), now).map(Some),
                })
            })
            .await?;
            crate::timers::changed(hub);
            text(t.map_or_else(|| "That was the pomodoro's last phase: it's done.".into(), |t| crate::timers::said(&t, now)), false)
        }
        "list_alarms" => {
            let h = hub.clone();
            let (list, timers) =
                blocking(move || Ok(h.core.with(|db| -> rusqlite::Result<_> { Ok((crate::alarms::list(db)?, crate::timers::list(db)?)) })?)).await?;
            let c = crate::alarms::clock();
            let offset = |ms: i64| c.offset_ms(ms);
            let v = json!({
                "now": crate::alarms::describe_time(c.now_ms(), &offset),
                "zone": crate::alarms::zone(),
                "alarms": crate::alarms::summary(&list, &offset),
                "timers": crate::timers::summary(&timers, c.now_ms()),
            });
            text(serde_json::to_string(&v)?, false)
        }
        "cancel_alarm" => {
            let (h, id) = (hub.clone(), id.clone());
            if let Ok(true) = crate::timers::end_ring(hub, &id, false) {
                return Ok(text("Stopped.", false));
            }
            let (alarm, timer) = blocking(move || {
                Ok(h.core.with(|db| -> rusqlite::Result<_> { Ok((crate::alarms::remove(db, &id)?, crate::timers::remove(db, &id)?)) })?)
            })
            .await?;
            anyhow::ensure!(alarm || timer, "There's no alarm, reminder or timer `{}`. Use list_alarms.", arg(args, "id"));
            if alarm {
                crate::alarms::changed(hub);
            } else {
                crate::timers::changed(hub);
            }
            text("Removed.", false)
        }
        "stop_alarm" => {
            let h = hub.clone();
            let (alarm, timer) =
                blocking(move || Ok(h.core.with(|db| -> rusqlite::Result<_> { Ok((crate::alarms::ringing(db)?, crate::timers::ringing(db)?)) })?)).await?;
            let snooze = args["snooze"].as_bool().unwrap_or(false);
            match (alarm, timer) {
                (Some(a), _) => {
                    crate::alarms::end_ring(hub, &a.id, snooze)?;
                    text(if snooze { "Snoozed for 10 minutes." } else { "Stopped." }, false)
                }
                (None, Some(t)) => {
                    crate::timers::end_ring(hub, &t.id, snooze)?;
                    text(if snooze { "A minute more." } else { "Stopped." }, false)
                }
                (None, None) => text("Nothing is ringing.", false),
            }
        }
        "set_time_zone" => {
            let detail = match (args["zone"].as_str().filter(|z| !z.is_empty()), args["automatic"].as_bool()) {
                (Some(zone), _) => {
                    anyhow::ensure!(crate::alarms::zone_ok(zone), "`{zone}` isn't a time zone id like America/New_York.");
                    format!("to {zone}")
                }
                (None, Some(true)) => "to follow the network".into(),
                (None, Some(false)) => "to stay as it is".into(),
                (None, None) => anyhow::bail!("Give zone, or automatic."),
            };
            if let Some(no) = denied(hub, caller, crate::approvals::Ask::new("device", "change the time zone", detail)).await {
                return Ok(no);
            }
            let how = match (args["zone"].as_str().filter(|z| !z.is_empty()), args["automatic"].as_bool()) {
                (Some(zone), _) => {
                    crate::alarms::set_zone(hub, zone).await?;
                    "picked by the user"
                }
                (None, Some(on)) => {
                    crate::alarms::set_zone_auto(hub, on).await?;
                    if on { "following the network" } else { "kept as it is" }
                }
                (None, None) => anyhow::bail!("Give zone, or automatic."),
            };
            let c = crate::alarms::clock();
            text(
                format!(
                    "The R1's time zone is {} ({how}); it's {} there.",
                    crate::alarms::zone(),
                    crate::alarms::describe_time(c.now_ms(), &|ms| c.offset_ms(ms))
                ),
                false,
            )
        }
        "stop_build" => {
            if hub.builds.stop(None) {
                text("Stopping it. Nothing will be installed.", false)
            } else {
                text("No build is running.", false)
            }
        }
        "set_app_settings" => {
            let i = installed()?;
            let values = args["values"].as_object().cloned().unwrap_or_default();
            anyhow::ensure!(!values.is_empty(), "Give values: setting name → new value.");
            hub.apps.check_settings(&id, &values)?;
            let ask = crate::approvals::Ask::new("apps", format!("change {}'s settings", i.app.app.name), crate::approvals::words(&Value::Object(values.clone())));
            if let Some(no) = denied(hub, caller, ask).await {
                return Ok(no);
            }
            let (h, app) = (hub.clone(), id.clone());
            let now = blocking(move || h.apps.set_settings(&app, &values)).await?;
            hub.scheduler.changed(&id, &["settings".to_owned()]);
            hub.emit(Event::AppChanged { app: id.clone(), tables: vec!["settings".into()] }, Source::Api);
            text(format!("Settings now: {}", serde_json::to_string(&now)?), false)
        }
        "query_app" => {
            installed()?;
            let (h, q, p) = (hub.clone(), arg(args, "query").to_owned(), params(args));
            let rows = blocking(move || h.apps.query(&id, &q, &p)).await?;
            let total = rows.len();
            let shown: Vec<_> = rows.into_iter().take(50).collect();
            let more = if total > 50 { format!(" (first 50 of {total})") } else { String::new() };
            text(format!("{}{more}", serde_json::to_string(&shown)?), false)
        }
        "run_action" => {
            let i = installed()?;
            let action = arg(args, "action").to_owned();
            let def = i.app.actions.get(&action).ok_or_else(|| anyhow::anyhow!("`{id}` has no action `{action}` (describe_app lists them)"))?;
            if let Some(c) = def.get("confirm")
                && args["confirmed"] != true
            {
                let ask = c.as_str().map_or_else(|| "This changes data that can't be put back easily.".to_owned(), str::to_owned);
                return Ok(text(format!("This action asks first: \"{ask}\" Ask the user, then call again with confirmed: true."), true));
            }
            let (h, a, p) = (hub.clone(), action.clone(), params(args));
            let app = id.clone();
            let run = blocking(move || h.apps.run_action(&app, &a, &p)).await?;
            hub.scheduler.changed(&id, &run.changed_tables);
            if !run.changed_tables.is_empty() {
                hub.emit(Event::AppChanged { app: id.clone(), tables: run.changed_tables.clone() }, Source::Api);
            }
            let said = if run.said.is_empty() { String::new() } else { format!(" The app says: {}.", run.said.join("; ")) };
            let undo = if run.undoable { " The user can undo it." } else { "" };
            let played = crate::player::perform(hub, &i.app, &run.media);
            let played = if played.is_empty() { String::new() } else { format!(" Player: {}.", played.join("; ")) };
            let skipped = if run.skipped.is_empty() {
                String::new()
            } else {
                format!(" Its {} step(s) run only on the app's page: open_app if the user wants them.", run.skipped.join(", "))
            };
            text(format!("Done: {action} in {} ({} rows changed).{said}{undo}{played}{skipped}", i.app.app.name, run.rows_affected), false)
        }
        "undo_action" => {
            let (h, app) = (hub.clone(), id.clone());
            let (action, exec) = blocking(move || h.apps.undo_run(&app)).await?;
            hub.scheduler.changed(&id, &exec.changed_tables);
            if !exec.changed_tables.is_empty() {
                hub.emit(Event::AppChanged { app: id.clone(), tables: exec.changed_tables.clone() }, Source::Api);
            }
            text(format!("Undid {action}."), false)
        }
        "open_app" => {
            let i = installed()?;
            hub.emit(Event::Open { app: id.clone() }, Source::Api);
            text(format!("Opened {} on the screen.", i.app.app.name), false)
        }
        "control_media" => {
            let op = arg(args, "op");
            let before = hub.player.now();
            if !hub.player.busy() {
                // An Android app's music (R7): the host does what's asked of its media session.
                let Some(m) = hub.android.media() else { return Ok(text("Nothing is playing.", false)) };
                anyhow::ensure!(
                    op.is_empty() || ["pause", "resume", "stop", "toggle", "next", "previous"].contains(&op),
                    "op is pause, resume, stop, toggle, next or previous"
                );
                if !op.is_empty() {
                    hub.emit(Event::AndroidMedia { op: op.to_owned(), query: None, package: Some(m.package.clone()), uri: None }, Source::Api);
                }
                let what = [m.title.as_str(), m.artist.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" by ");
                return Ok(text(
                    match op {
                        "" => format!("{} is {} {what}.", m.app, if m.playing { "playing" } else { "paused on" }),
                        other => format!("Asked {} to {other} ({what}).", m.app),
                    },
                    false,
                ));
            }
            anyhow::ensure!(["", "pause", "resume", "stop", "toggle"].contains(&op), "the R1's own player can pause, resume, stop or toggle");
            if !op.is_empty() {
                anyhow::ensure!(["pause", "resume", "stop", "toggle"].contains(&op), "op is pause, resume, stop or toggle");
                hub.player.control(hub, None, op);
            }
            let app = hub.apps.get(&before.app).map_or(before.app.clone(), |i| i.app.app.name.clone());
            let what = [before.title.as_str(), before.song.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(": ");
            text(
                match op {
                    "" => format!("{app} is {} {what}.", before.state),
                    "stop" => format!("Stopped {what} ({app})."),
                    other => format!("Asked {app}'s player to {other} {what}."),
                },
                false,
            )
        }
        "play_music" => {
            let query = arg(args, "query");
            anyhow::ensure!(!query.is_empty(), "Say what to play.");
            let named = arg(args, "app");
            let app = hub.android.music_app(Some(named).filter(|n| !n.is_empty())).ok_or_else(|| {
                if named.is_empty() {
                    anyhow::anyhow!("There's no music app on the R1: Spotify or YouTube Music would do (show_android_app opens Aurora on one).")
                } else {
                    anyhow::anyhow!("{named} isn't installed on the R1, or can't play from a search. list_android_apps has what's there.")
                }
            })?;
            let uri = match arg(args, "link") {
                "" => None,
                link => Some(crate::android::music_uri(link).ok_or_else(|| anyhow::anyhow!("link: an open.spotify.com or music.youtube.com page, not `{link}`"))?),
            };
            // A Spotify link is Spotify's: whatever app was named or playing.
            let package = if uri.as_deref().is_some_and(|u| u.starts_with("spotify:")) { "com.spotify.music".to_owned() } else { app.package.clone() };
            // Spotify plays a search only for apps it knows; from anywhere else it shows results. So it needs the link.
            anyhow::ensure!(
                uri.is_some() || package != "com.spotify.music",
                "Spotify won't play from a search here, only show results. Find its open.spotify.com page for \"{query}\" (an artist, album, playlist or song) with a web search, and call play_music again with it as `link`."
            );
            anyhow::ensure!(hub.android.get(&package).is_some(), "Spotify isn't installed on the R1.");
            let said = if uri.is_some() { "to play" } else { "to find and play" };
            hub.emit(Event::AndroidMedia { op: "play".into(), query: Some(query.to_owned()), package: Some(package), uri }, Source::Api);
            text(format!("Asked {} {said} \"{query}\". control_media says what came on.", app.name), false)
        }
        "list_notifications" => {
            let only = arg(args, "app").to_lowercase();
            let now = crate::core::now();
            let notes: Vec<Value> = hub
                .android
                .notes()
                .into_iter()
                .filter(|n| only.is_empty() || n.app.to_lowercase() == only || n.package == only)
                .take(20)
                .map(|n| {
                    let mins = (now - n.at).max(0) / 60_000;
                    json!({ "key": n.key, "app": n.app, "title": n.title, "text": n.text, "minutes_ago": mins, "can_reply": n.can_reply })
                })
                .collect();
            if notes.is_empty() {
                text("No notifications.", false)
            } else {
                text(serde_json::to_string(&notes)?, false)
            }
        }
        "reply_notification" => {
            let key = arg(args, "key");
            let words = arg(args, "text").trim().to_owned();
            anyhow::ensure!(!words.is_empty(), "Say what to reply.");
            let n = hub.android.note(key).ok_or_else(|| anyhow::anyhow!("There's no notification `{key}` (list_notifications)."))?;
            anyhow::ensure!(n.can_reply, "{}'s notification has no reply box: open_notification and reply in the app.", n.app);
            hub.emit(Event::AndroidNotice { op: "reply".into(), key: n.key.clone(), text: Some(words) }, Source::Api);
            text(format!("Sent the reply through {}.", n.app), false)
        }
        "open_notification" | "dismiss_notification" => {
            let key = arg(args, "key");
            let n = hub.android.note(key).ok_or_else(|| anyhow::anyhow!("There's no notification `{key}` (list_notifications)."))?;
            let op = if name == "open_notification" { "open" } else { "dismiss" };
            hub.emit(Event::AndroidNotice { op: op.into(), key: n.key.clone(), text: None }, Source::Api);
            text(if op == "open" { format!("Opened {} on the screen.", n.app) } else { format!("Cleared it ({}).", n.app) }, false)
        }
        "list_android_apps" => {
            let apps: Vec<Value> = hub
                .android
                .list()
                .into_iter()
                .map(|a| {
                    let store = a.package == crate::android::STORE;
                    json!({ "name": a.name, "package": a.package, "store": store.then_some(true) })
                })
                .collect();
            if apps.is_empty() {
                text("No Android apps are installed yet (or the R1's shell hasn't listed them). show_android_app finds one to install.", false)
            } else {
                text(serde_json::to_string(&apps)?, false)
            }
        }
        "app_find" => {
            let target = arg(args, "target").trim().to_owned();
            if target.is_empty() {
                return Ok(text("Give the words to find (target).", true));
            }
            let dir = args["direction"].as_str().unwrap_or("down");
            let (agent, chat) = (caller.agent.as_str(), caller.chat.as_str());
            let mut screen = match crate::appwin::call(hub, agent, chat, "read", json!({})).await {
                Ok(v) => v,
                Err(e) => return Ok(text(e, true)),
            };
            let mut still = 0;
            for i in 0..=crate::appwin::FIND_SCROLLS {
                let els = crate::browser::elements(screen["text"].as_str().unwrap_or_default());
                if let Some(e) = crate::appwin::pick(hub, &target, &els) {
                    let found = format!("Found “{}” [{}].\n{}", e.name, e.r#ref, screen["text"].as_str().unwrap_or_default());
                    screen["text"] = json!(found);
                    hub.app_window.saw(chat, &screen);
                    return Ok(crate::appwin::reply("find", Ok(screen)));
                }
                if i == crate::appwin::FIND_SCROLLS {
                    break;
                }
                let before = crate::appwin::lines(&screen);
                screen = match crate::appwin::call(hub, agent, chat, "scroll", json!({ "direction": dir, "ref": args["ref"] })).await {
                    Ok(v) => v,
                    Err(e) => return Ok(text(e, true)),
                };
                // The end of the list: scrolling changed nothing, twice running.
                still = if crate::appwin::lines(&screen) == before { still + 1 } else { 0 };
                if still >= crate::appwin::FIND_STILL {
                    let said = format!("“{target}” isn't here: scrolled {dir} to the end without it. Try other words, or another part of the app.\n{}", screen["text"].as_str().unwrap_or_default());
                    screen["text"] = json!(said);
                    return Ok(crate::appwin::reply("find", Ok(screen)));
                }
            }
            let said = format!("“{target}” didn't show after {} scrolls {dir}. Try other words, or another part of the app.\n{}", crate::appwin::FIND_SCROLLS, screen["text"].as_str().unwrap_or_default());
            screen["text"] = json!(said);
            crate::appwin::reply("find", Ok(screen))
        }
        "app_do" => {
            let steps = args["steps"].as_array().cloned().unwrap_or_default();
            if steps.is_empty() || steps.len() > 10 {
                return Ok(text("Give 1 to 10 steps.", true));
            }
            let mut out = Vec::new();
            let mut picture = None;
            for (i, step) in steps.iter().enumerate() {
                let op = step["tool"].as_str().unwrap_or_default().trim_start_matches("app_").to_owned();
                if !matches!(op.as_str(), "open" | "click" | "type" | "scroll" | "back" | "read" | "wait" | "find") {
                    out.push(format!("{}. `{op}` isn't a step: stopped.", i + 1));
                    break;
                }
                // A step that acts, before the last, answers without reading the screen (T-147): only the last step's
                // screen is shown, and each read cost 1 to 2 s on Uber Eats' pages.
                let mut step = step.clone();
                if i + 1 < steps.len() && matches!(op.as_str(), "click" | "type" | "scroll" | "back") {
                    step["quick"] = json!(true);
                }
                let r = Box::pin(call_inner(hub, caller, &format!("app_{op}"), &step)).await?;
                let failed = r["isError"] == true;
                let content = r["content"].as_array().cloned().unwrap_or_default();
                if let Some(img) = content.iter().find(|c| c["type"] == "image") {
                    picture = Some(img.clone());
                }
                let said = content.iter().find(|c| c["type"] == "text").and_then(|c| c["text"].as_str()).unwrap_or_default();
                // The last step's screen in full; the ones before by what they did (not the "[App]" line).
                let said = if failed || i + 1 == steps.len() {
                    said.to_owned()
                } else {
                    said.lines().filter(|l| !(l.starts_with('[') && l.ends_with(']'))).take(2).collect::<Vec<_>>().join(" ")
                };
                out.push(format!("{}. {op}: {said}", i + 1));
                if failed {
                    out.push("Stopped there.".into());
                    break;
                }
            }
            let mut content = vec![json!({ "type": "text", "text": out.join("\n") })];
            content.extend(picture);
            json!({ "content": content, "isError": false })
        }
        "app_open" | "app_read" | "app_screenshot" | "app_click" | "app_type" | "app_scroll" | "app_back" | "app_wait" | "app_ask_user"
        | "app_close" => {
            let op = &name["app_".len()..];
            let mut a = args.clone();
            if op == "close" && args["for_user"] == true {
                a = json!({ "give": true });
            }
            if op == "open" {
                let link = arg(args, "link");
                let mut package = arg(args, "package").to_owned();
                if !link.is_empty() {
                    if let Err(why) = crate::appwin::allowed_link(link) {
                        return Ok(text(why, true));
                    }
                    if package.is_empty() {
                        match crate::appwin::call(hub, &caller.agent, &caller.chat, "resolve", json!({ "link": link })).await {
                            Ok(v) if v["package"].as_str().is_some_and(|p| !p.is_empty()) => package = v["package"].as_str().unwrap_or_default().to_owned(),
                            Ok(_) => return Ok(text("No app on the R1 opens that link.", true)),
                            Err(why) => return Ok(text(why, true)),
                        }
                    }
                }
                if package.is_empty() {
                    return Ok(text("Give the app's package (list_android_apps) or a link it opens.", true));
                }
                let package = package.as_str();
                a["package"] = json!(package);
                if package == crate::android::STORE {
                    return Ok(text("Not Aurora Store: show_android_app opens it for the user.", true));
                }
                let Some(app) = hub.android.get(package) else {
                    return Ok(text(format!("`{package}` isn't installed on the R1: list_android_apps has what's there (show_android_app finds one to install)."), true));
                };
                if let Err(why) = crate::appwin::may_use(hub, &caller.agent, &caller.chat, package, &app.name).await {
                    return Ok(text(why, true));
                }
                a["name"] = json!(app.name);
            }
            // A ref goes with the name it had on the screen this chat saw: if the screen's been read since (a step
            // before it in app_do, tools sent at once), the host goes by the name, as refs are numbered per read.
            if matches!(op, "click" | "type")
                && let Some(r) = args["ref"].as_str().filter(|r| !r.is_empty())
                && args["target"].as_str().is_none_or(str::is_empty)
                && let Some(name) = hub.app_window.name_of(&caller.chat, r)
            {
                a["target"] = json!(name);
                a["named"] = json!(true);
            }
            // The same act on a screen that hasn't changed: it didn't work the first time.
            if crate::appwin::acts(op) {
                let act = format!("{op} {} {} {} {} {}", args["ref"], args["target"], args["x"], args["y"], args["text"]);
                let times = hub.app_window.again(&caller.chat, &act);
                if times >= 3 {
                    return Ok(text("You've done exactly this three times on this same screen and it hasn't changed: do something else (its position on the picture, another element, scrolling, app_wait, or ask the user).", true));
                }
            }
            // An element named in words instead of a ref: System One finds it on the screen as it is now (on the R1, no
            // model), as in the browser.
            if matches!(op, "click" | "type")
                && args["ref"].as_str().is_none_or(str::is_empty)
                && args["x"].is_null()
                && let Some(target) = args["target"].as_str().filter(|t| !t.trim().is_empty())
                && let Some(r) = crate::appwin::resolve(hub, &caller.agent, &caller.chat, op, target).await
            {
                a["ref"] = json!(r);
            }
            if crate::appwin::acts(op) {
                // What it would do, in words for the card: the element's words and the app; and for a tap by place, every
                // button on the screen and those by the finger. The screen's own words come only when a card needs them.
                let placed = op == "click" && a["ref"].as_str().is_none_or(str::is_empty) && a["target"].as_str().is_none_or(str::is_empty) && !args["x"].is_null();
                let d = crate::appwin::call(hub, &caller.agent, &caller.chat, "describe", json!({ "ref": a["ref"], "target": a["target"], "x": a["x"], "y": a["y"], "full": placed }))
                    .await
                    .unwrap_or_default();
                // The screen's words for a final step's card, fetched only then.
                let screen_words = || async {
                    if let Some(s) = d["screen"].as_str() {
                        return s.to_owned();
                    }
                    crate::appwin::call(hub, &caller.agent, &caller.chat, "describe", json!({ "full": true }))
                        .await
                        .ok()
                        .and_then(|v| v["screen"].as_str().map(str::to_owned))
                        .unwrap_or_default()
                };
                if op == "type" && d["password"] == true {
                    return Ok(text("That's a password, card or code field: ask the user to type it themselves (app_ask_user).", true));
                }
                // Named, and not on the screen after a few seconds' wait: nothing is tapped. Were it to turn up a moment
                // later, the tap would land on something the guards and the card never saw.
                if op == "click"
                    && d["found"] == false
                    && args["x"].is_null()
                    && let Some(named) = a["target"].as_str().filter(|t| !t.trim().is_empty())
                {
                    return Ok(text(format!("No “{named}” on the screen (waited a few seconds): read it again (app_read), scroll to it, or tap its place on the picture."), true));
                }
                if d["prompt"] == true {
                    return Ok(text(crate::appwin::PROMPT, true));
                }
                let label = d["name"].as_str().unwrap_or_default();
                let app = d["app"].as_str().filter(|s| !s.is_empty()).unwrap_or("the app");
                let short = |s: &str| -> String { s.chars().take(60).collect() };
                let what = match op {
                    "click" if !label.is_empty() => format!("tap “{}”", short(label)),
                    "click" => "tap the screen".to_owned(),
                    "type" => format!("type “{}”{}", short(arg(args, "text")), if label.is_empty() { String::new() } else { format!(" into {}", short(label)) }),
                    _ => format!("press {}", arg(args, "key")),
                };
                if let Err(why) = crate::appwin::may_act(hub, &caller.agent, &caller.chat, &what, app).await {
                    return Ok(text(why, true));
                }
                if op == "click"
                    && let Some(no) = crate::appwin::dry_refusal(hub, &caller.chat, label, args["final"] == true)
                {
                    return Ok(text(no, true));
                }
                // A tap by place (x,y): which button is under the finger can't be told when a web page's words lag, so
                // a screen with a final step on it asks on the card every time, and a test refuses it (2026-10-13: a tap
                // meant for a bread choice put a sandwich in the cart).
                if placed {
                    let words = |k: &str| -> Vec<String> { d[k].as_array().into_iter().flatten().filter_map(|b| b.as_str().map(str::to_owned)).collect() };
                    match crate::appwin::by_place(&words("buttons"), &words("near"), hub.app_window.is_dry(&caller.chat)) {
                        Some(no) if hub.app_window.is_dry(&caller.chat) => return Ok(text(no, true)),
                        Some(what) => {
                            let app = d["app"].as_str().filter(|s| !s.is_empty()).unwrap_or("the app");
                            let screen = screen_words().await;
                            if let Err(why) = crate::appwin::commit(hub, &caller.agent, &caller.chat, app, &what, &screen).await {
                                return Ok(text(why, true));
                            }
                        }
                        None => {}
                    }
                }
                if op == "click" && (args["final"] == true || crate::appwin::is_final(label)) {
                    let screen = screen_words().await;
                    if let Err(why) = crate::appwin::commit(hub, &caller.agent, &caller.chat, app, &what, &screen).await {
                        return Ok(text(why, true));
                    }
                }
            }
            if op == "ask_user" {
                crate::scheduler::wake_display().await;
            }
            tracing::info!(agent = %caller.agent, op, "app window");
            let mut r = crate::appwin::call(hub, &caller.agent, &caller.chat, op, a.clone()).await;
            if let Ok(v) = &mut r {
                hub.app_window.saw(&caller.chat, v);
                // Opening an app: what's been learned about it comes first.
                if op == "open"
                    && let Some(tips) = a["package"].as_str().and_then(crate::appwin::tips)
                    && let Some(t) = v["text"].as_str()
                {
                    let with = format!("{tips}\n\n{t}");
                    v["text"] = json!(with);
                }
            }
            crate::appwin::reply(op, r)
        }
        "open_android_app" => {
            let package = arg(args, "package");
            let link = arg(args, "link");
            if !link.is_empty() {
                let opened = crate::android::open_link(link, Some(package).filter(|p| !p.is_empty())).await?;
                let name = hub.android.get(&opened).map_or(opened, |a| a.name);
                return Ok(text(format!("Opened {link} in {name} on the screen."), false));
            }
            anyhow::ensure!(!package.is_empty(), "package or link");
            crate::android::open(package).await?;
            let name = hub.android.get(package).map_or_else(|| package.to_owned(), |a| a.name);
            text(format!("Opened {name} on the screen."), false)
        }
        "show_android_app" => {
            let package = Some(arg(args, "package")).filter(|p| !p.is_empty());
            crate::android::store(package).await?;
            match package.and_then(|p| hub.android.get(p)) {
                Some(a) => text(format!("{} is already installed: Aurora Store shows its page (open or update it there).", a.name), false),
                None if package.is_some() => text("Aurora Store shows its page: the user taps Install there.", false),
                None => text("Aurora Store is open.", false),
            }
        }
        "create_app" => {
            let brief = arg(args, "brief").to_owned();
            anyhow::ensure!(!brief.is_empty(), "A brief is required.");
            start_build(hub, caller, crate::create::Task::Create { brief }, args["model"].as_str().map(str::to_owned), args["big"] == true)
        }
        "edit_app" => {
            installed()?;
            let request = arg(args, "request").to_owned();
            anyhow::ensure!(!request.is_empty(), "Say what to change.");
            start_build(hub, caller, crate::create::Task::Edit { app: id, request }, args["model"].as_str().map(str::to_owned), args["big"] == true)
        }
        "install_app" => {
            let source = arg(args, "source").to_owned();
            anyhow::ensure!(!source.trim().is_empty() && source.len() <= 256 * 1024, "Give the whole spec (EneeSpec YAML).");
            let h = hub.clone();
            let installed = blocking(move || Ok(h.apps.install_recorded(&source, "installed by an agent"))).await?;
            match installed {
                Ok((summary, _)) => {
                    hub.emit(Event::AppInstalled { app: summary.id.clone() }, Source::Api);
                    let problems = hub.apps.get(&summary.id).map(|i| crate::broker::r1_problems(&i.app)).unwrap_or_default();
                    let note = if problems.is_empty() {
                        String::new()
                    } else {
                        let list: Vec<String> = problems.iter().map(|d| format!("{}: {}", d.path, d.message)).collect();
                        format!(" Parts of it won't run on the R1: {}. edit_app can take them out.", list.join("; "))
                    };
                    text(format!("Installed {} (`{}`).{note}", summary.name, summary.id), false)
                }
                Err(rejected) => {
                    let errors: Vec<String> =
                        rejected.diagnostics.iter().take(5).map(|d| format!("{}: {}", d.path, d.message)).collect();
                    text(format!("It doesn't compile: {}", errors.join("; ")), true)
                }
            }
        }
        "fix_app" => {
            installed()?;
            let errors = hub.apps.errors(&id);
            anyhow::ensure!(!errors.is_empty(), "Nothing has gone wrong with it on the R1 since its last change. Ask the user what's wrong, then use edit_app.");
            start_build(hub, caller, crate::create::Task::Edit { app: id, request: crate::apps::fix_request(&errors) }, args["model"].as_str().map(str::to_owned), false)
        }
        "app_history" => {
            installed()?;
            let versions: Vec<Value> = hub
                .apps
                .versions(&id)
                .into_iter()
                .map(|v| json!({ "at": v.at, "kind": v.kind, "request": v.request, "said": v.said, "deleted": v.deleted }))
                .collect();
            if versions.is_empty() {
                text("No history yet: it hasn't been changed since history started.", false)
            } else {
                text(serde_json::to_string(&versions)?, false)
            }
        }
        "home_apps" => {
            let h = hub.clone();
            let mut apps: Option<Vec<String>> = args.get("apps").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect());
            if let Some(a) = &apps {
                let checked = crate::server::checked_pins(hub, a)?;
                if let Some(no) = denied(hub, caller, crate::approvals::Ask::new("look", "change the apps on home", app_names(hub, &checked))).await {
                    return Ok(no);
                }
                apps = Some(checked);
            }
            let pins = blocking(move || match apps {
                Some(a) => crate::server::set_home_pins(&h, &a),
                None => Ok(h.core.home_pins()),
            })
            .await?;
            let named: Vec<String> = pins
                .iter()
                .map(|p| match hub.apps.get(p) {
                    Some(i) => format!("{} ({p})", i.app.app.name),
                    None => p.clone(),
                })
                .collect();
            text(if named.is_empty() { "Nothing is pinned to home.".to_owned() } else { format!("Pinned to home: {}", named.join(", ")) }, false)
        }
        "set_agent_look" => {
            let who = args["agent"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
            let agents = hub.core.agents()?;
            let mut a = match &who {
                Some(w) => agents.into_iter().find(|a| a.id == *w || a.name.eq_ignore_ascii_case(w)),
                None => agents.into_iter().find(|a| a.id == caller.agent),
            }
            .ok_or_else(|| anyhow::anyhow!("There's no agent `{}`. Use list_agents.", who.as_deref().unwrap_or("")))?;
            for k in ["form", "color", "finish", "eyes"] {
                if let Some(v) = args[k].as_str() {
                    match k {
                        "form" => a.form = v.to_owned(),
                        "color" => a.color = v.to_owned(),
                        "finish" => a.finish = v.to_owned(),
                        _ => a.eyes = v.to_owned(),
                    }
                }
            }
            if let Some(w) = args["wear"].as_array() {
                a.wear = w.iter().filter_map(Value::as_str).map(str::to_owned).collect();
            }
            anyhow::ensure!(crate::look::ACCENTS.contains(&a.color.as_str()), "`{}` isn't a colour: {}", a.color, crate::look::ACCENTS.join(", "));
            a.clone().validate()?;
            let title = if a.id == caller.agent { "change its own look".to_owned() } else { format!("change {}'s look", a.name) };
            let mut change = serde_json::Map::new();
            for k in ["form", "color", "finish", "eyes", "wear"] {
                if !args[k].is_null() {
                    change.insert(k.to_owned(), args[k].clone());
                }
            }
            if let Some(no) = denied(hub, caller, crate::approvals::Ask::new("look", title, crate::approvals::words(&Value::Object(change)))).await {
                return Ok(no);
            }
            let id = a.id.clone();
            let saved = hub.core.update_agent(&id, a)?;
            hub.emit(Event::AgentsChanged, Source::Api);
            let wear = if saved.wear.is_empty() { "nothing".to_owned() } else { saved.wear.join(", ") };
            text(format!("{} is now a {} {} {} with {} eyes, wearing {wear}.", saved.name, saved.finish, saved.color, saved.form, saved.eyes), false)
        }
        "set_look" => {
            let change = args.get("change").cloned().filter(|c| c.as_object().is_some_and(|m| !m.is_empty()));
            if let Some(c) = &change {
                crate::server::checked_look(hub, c)?;
                if let Some(no) = denied(hub, caller, crate::approvals::Ask::new("look", "change the R1's look", crate::approvals::words(c))).await {
                    return Ok(no);
                }
            }
            let h = hub.clone();
            let look = blocking(move || match change {
                Some(c) => crate::server::set_look(&h, &c),
                None => Ok(h.core.look()),
            })
            .await?;
            text(crate::look::summary(&look), false)
        }
        "glance_widgets" => {
            let mut apps: Option<Vec<String>> =
                args.get("apps").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect());
            if let Some(a) = &apps {
                let checked = crate::glance::checked_apps(hub, a)?;
                if let Some(no) = denied(hub, caller, crate::approvals::Ask::new("look", "change the glance screen's widgets", app_names(hub, &checked))).await {
                    return Ok(no);
                }
                apps = Some(checked);
            }
            let h = hub.clone();
            let widgets = blocking(move || {
                if let Some(apps) = apps {
                    h.glance.set_apps(&h, &apps)?;
                }
                Ok(h.glance.widgets(&h))
            })
            .await?;
            let shows = |w: &Value| w.get("card").map(crate::cards::summary).unwrap_or_else(|| w["note"].as_str().unwrap_or_default().to_owned());
            let list: Vec<Value> = widgets.iter().map(|w| json!({ "app": w["app"], "name": w["name"], "shows": shows(w) })).collect();
            text(if list.is_empty() { "No widgets on the glance screen.".to_owned() } else { serde_json::to_string(&list)? }, false)
        }
        "battery" => text(serde_json::to_string(&crate::device::battery().await)?, false),
        "device_settings" => {
            let mut settings = hub.device_settings.all(hub);
            let unknown = !settings.contains_key("brightness");
            let may: serde_json::Map<String, Value> = crate::approvals::AREAS
                .iter()
                .map(|a| {
                    let level = hub.core.permission(&caller.agent, a);
                    let how = match level.as_str() {
                        "allow" => "yes",
                        "never" => "no (the user can allow it in your space on the R1)",
                        _ => "after the user approves it on the R1",
                    };
                    (crate::approvals::area_words(a).to_owned(), json!(how))
                })
                .collect();
            if unknown {
                settings.insert("note".into(), json!("The R1's screen app hasn't said its brightness, timeout and volume yet."));
            }
            text(
                serde_json::to_string(&json!({ "settings": settings, "options": crate::device_settings::options(), "you may change": may }))?,
                false,
            )
        }
        "change_device_settings" => {
            let changes = args["changes"].as_object().cloned().unwrap_or_default();
            let checked = match crate::device_settings::check(&changes) {
                Ok(c) => c,
                Err(e) => return Ok(text(e, true)),
            };
            let said = checked.words.join(", ");
            let ask = crate::approvals::Ask::new("device", "change the R1's settings", said.clone()).why(arg(args, "why"));
            if let Some(no) = denied(hub, caller, ask).await {
                return Ok(no);
            }
            crate::device_settings::apply(hub, checked).await?;
            text(format!("Done: {said}."), false)
        }
        "set_app_permission" => {
            let i = installed()?;
            let name = i.app.app.name.clone();
            // `permissions` (a list), or one as `permission`.
            let mut ids: Vec<String> = args["permissions"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
            if let Some(one) = args["permission"].as_str() {
                ids.push(one.to_owned());
            }
            ids.dedup();
            anyhow::ensure!(!ids.is_empty(), "Give permissions: their ids, from describe_app.");
            let mut labels = Vec::new();
            let mut reasons = Vec::new();
            for perm in &ids {
                let p = i
                    .app
                    .permissions
                    .iter()
                    .find(|p| &p.id == perm)
                    .ok_or_else(|| anyhow::anyhow!("{name} doesn't ask for `{perm}` (describe_app lists what it asks for)."))?;
                anyhow::ensure!(crate::broker::ON_R1.contains(&perm.as_str()), "The R1 can't grant `{perm}` yet.");
                labels.push(crate::broker::label(p));
                if !p.reason.trim().is_empty() {
                    reasons.push(format!("\"{}\"", p.reason.trim()));
                }
            }
            let allow = args["allow"].as_bool().ok_or_else(|| anyhow::anyhow!("Give allow: true or false."))?;
            let label = crate::broker::and_list(&labels);
            let ask = if allow {
                let detail = if reasons.is_empty() { String::new() } else { format!("{name}: {}", reasons.join(" · ")) };
                crate::approvals::Ask::new("apps", format!("let {name} {label}"), detail).every_time()
            } else {
                crate::approvals::Ask::new("apps", format!("stop letting {name} {label}"), String::new())
            };
            if let Some(no) = denied(hub, caller, ask.why(arg(args, "why"))).await {
                return Ok(no);
            }
            let (h, app) = (hub.clone(), id.clone());
            let decisions: serde_json::Map<String, Value> = ids.iter().map(|p| (p.clone(), json!(allow))).collect();
            blocking(move || {
                let i = h.apps.get(&app).ok_or_else(|| anyhow::anyhow!("{app} was removed"))?;
                crate::broker::decide(&h.core, &i.app, &decisions, "approval")?;
                h.services.refresh(&h, &app);
                Ok(())
            })
            .await?;
            hub.emit(Event::AppPermissions { app: id.clone() }, Source::Api);
            text(if allow { format!("{name} may {label} now.") } else { format!("{name} can't {label} any more.") }, false)
        }
        "undo_app_change" => {
            installed()?;
            let (h, app) = (hub.clone(), id.clone());
            let (summary, undone, migration) = blocking(move || h.apps.undo(&app)).await?;
            hub.emit(Event::AppInstalled { app: id.clone() }, Source::Api);
            let restored = migration.map(|m| m.restored).filter(|r| !r.is_empty()).map(|r| format!(" Brought back: {}.", r.join(", "))).unwrap_or_default();
            let what = if undone.said.is_empty() { undone.request } else { undone.said };
            text(format!("Undid the last change to {} ({what}).{restored}", summary.name), false)
        }
        "delete_app" => {
            let i = installed()?;
            if args["confirmed"] != true {
                return Ok(text(format!("Ask the user first: remove {}? It can be restored for 7 days.", i.app.app.name), true));
            }
            let (h, app) = (hub.clone(), id.clone());
            blocking(move || h.apps.remove(&app)).await?;
            hub.services.forget(&id);
            hub.emit(Event::AppRemoved { app: id.clone() }, Source::Api);
            text(format!("Removed {}. It can be restored for 7 days.", i.app.app.name), false)
        }
        "restore_app" => {
            if id.is_empty() {
                let trashed: Vec<Value> = hub.apps.trashed().into_iter().map(|(id, at, name)| json!({ "id": id, "name": name, "removed_at": at })).collect();
                return Ok(text(if trashed.is_empty() { "The trash is empty.".to_owned() } else { serde_json::to_string(&trashed)? }, false));
            }
            let (h, app) = (hub.clone(), id.clone());
            let summary = blocking(move || h.apps.restore_trashed(&app)).await?;
            hub.emit(Event::AppInstalled { app: id.clone() }, Source::Api);
            text(format!("Restored {} with its data.", summary.name), false)
        }
        "remember" => {
            let shared = args["shared"] == true;
            let m = hub.core.remember(&caller.agent, arg(args, "text"), shared)?;
            text(format!("Remembered (#{}{}).", m.id, if shared { ", shared with every agent" } else { "" }), false)
        }
        "read_feed" => {
            let url = arg(args, "url").to_owned();
            anyhow::ensure!(!url.is_empty(), "Give the feed's URL.");
            let limit = usize::try_from(args["limit"].as_u64().unwrap_or(10).clamp(1, 30)).unwrap_or(10);
            let fetcher = hub.services.fetcher();
            let feed = blocking(move || read_feed(&fetcher, &url)).await?;
            crate::cards::remember_pictures(&feed.items);
            let items: Vec<Value> = feed
                .items
                .iter()
                .take(limit)
                .map(|i| json!({ "title": i.title, "published": i.published, "link": i.link, "image": i.image, "summary": i.summary }))
                .collect();
            if items.is_empty() {
                text(format!("{} has no items right now.", if feed.title.is_empty() { "The feed" } else { feed.title.as_str() }), false)
            } else {
                text(json!({ "feed": feed.title, "items": items }).to_string(), false)
            }
        }
        "show_card" => {
            // The card itself, or (as models sometimes send it) its fields at the top.
            let given = if args.get("type").is_some() { args } else { &args["card"] };
            let mut card = crate::cards::check(given).map_err(|e| anyhow::anyhow!("That card isn't right: {e}. Fix it and call show_card again."))?;
            crate::cards::fill_pictures(&mut card);
            let summary = crate::cards::summary(&card);
            hub.core.add_message(&caller.chat, "card", &summary, &json!({ "card": card }))?;
            hub.emit(Event::chat(&caller.agent, &caller.chat, "card", Some(summary)).with_card(card), Source::Api);
            text("It's on the screen. Now answer in a sentence or two, without repeating what the card shows.", false)
        }
        "list_agents" => text(crate::mail::list_agents(hub, caller), false),
        "ask_agent" => text(crate::mail::ask(hub, caller, arg(args, "to"), arg(args, "text"))?, false),
        "tell_agent" => text(crate::mail::tell(hub, caller, arg(args, "to"), arg(args, "text"))?, false),
        "list_memories" => {
            let ms: Vec<Value> =
                hub.core.memories(Some(&caller.agent))?.into_iter().map(|m| json!({ "id": m.id, "text": m.text, "shared": m.shared })).collect();
            text(if ms.is_empty() { "Nothing yet.".to_owned() } else { serde_json::to_string(&ms)? }, false)
        }
        "forget" => {
            let n = args["id"].as_i64().ok_or_else(|| anyhow::anyhow!("forget needs a memory id (list_memories)"))?;
            text(if hub.core.forget(Some(&caller.agent), n)? { "Forgotten." } else { "No memory of yours with that id." }, false)
        }
        other => text(format!("Unknown tool `{other}`."), true),
    })
}
