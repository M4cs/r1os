//! What's up (R7): what's new and what's next, in one place, with no agent and no network. Replies you haven't read,
//! reminders that went off, the next alarm, timers, today's routines, today's journal and recordings, Android apps'
//! notifications, and what's playing. Tools › What's up shows it (a triple press can open it, and System One does for
//! "what's up"); agents read it with `whats_up`, so a morning routine can start there.

use crate::hub::Hub;
use serde_json::{Value, json};

const DAY_MS: i64 = 86_400_000;

/// Everything, now.
pub fn gather(hub: &Hub) -> anyhow::Result<Value> {
    let c = crate::alarms::clock();
    let now = c.now_ms();
    let offset = |ms: i64| c.offset_ms(ms);
    // Today, in the R1's own time.
    let today = (now + offset(now)).div_euclid(DAY_MS) * DAY_MS - offset(now);
    let tomorrow = today + DAY_MS;
    let when = |ms: i64| crate::alarms::describe_time(ms, &offset);

    let mut unread = Vec::new();
    for a in hub.core.agents()? {
        for ch in hub.core.chats(Some(&a.id), "user")?.into_iter().filter(|c| c.unread > 0) {
            unread.push(json!({ "agent": a.id, "name": a.name, "chat": ch.id, "title": ch.title, "count": ch.unread, "last": ch.last, "at": ch.updated }));
        }
    }
    // Newest first: the glance screen shows the agent with the newest reply.
    unread.sort_by_key(|u| std::cmp::Reverse(u["at"].as_i64().unwrap_or(0)));

    let alarms = hub.core.with(crate::alarms::list)?;
    let next_alarm = alarms.iter().filter(|a| a.kind == "alarm" && a.on).filter_map(|a| a.at.map(|at| (at, a))).min_by_key(|(at, _)| *at).map(|(at, a)| {
        json!({ "id": a.id, "label": a.label, "at": at, "when": when(at) })
    });
    let went_off: Vec<Value> = alarms.iter().filter(|a| a.kind == "reminder" && a.fired.is_some()).map(|a| json!({ "id": a.id, "text": a.label, "at": a.fired })).collect();
    // Apps' reminders that went off while you were away (T-049).
    let app_reminders: Vec<Value> = hub
        .core
        .with(|db| crate::notices::unseen(db, now))?
        .into_iter()
        .map(|n| json!({ "id": n.id, "app": n.app, "name": hub.apps.get(&n.app).map_or(n.app.clone(), |i| i.app.app.name.clone()), "title": n.title, "body": n.body, "at": n.at, "when": when(n.at) }))
        .collect();
    let later: Vec<Value> = alarms
        .iter()
        .filter(|a| a.kind == "reminder" && a.at.is_some_and(|t| t < tomorrow))
        .map(|a| json!({ "id": a.id, "text": a.label, "at": a.at, "when": a.at.map(when) }))
        .collect();

    let timers: Vec<Value> = hub
        .core
        .with(crate::timers::list)?
        .into_iter()
        .map(|t| json!({ "id": t.id, "title": t.title, "ends": t.ends, "left": t.left, "ringing": t.ringing.is_some(), "phase": t.phase }))
        .collect();

    let routines: Vec<Value> = hub
        .core
        .with(crate::routines::list)?
        .into_iter()
        .filter(|r| r.on && r.at.is_some_and(|t| t < tomorrow))
        .map(|r| json!({ "id": r.id, "title": r.title, "at": r.at, "when": r.at.map(when) }))
        .collect();

    let journal: Vec<crate::journal::Entry> = hub.core.with(|db| crate::journal::list(db, None, 30))?.into_iter().filter(|e| e.at >= today).collect();
    let recordings: Vec<crate::media::Item> = hub.core.with(|db| crate::media::list(db, Some("recording"), 30))?.into_iter().filter(|m| m.at >= today).collect();
    let notes = hub.android.notes();
    // What came while the display was off, since you last caught up (crate::missed).
    let missed = crate::missed::summary(hub).unwrap_or_else(|_| json!({ "count": 0, "apps": [] }));

    let joined = |parts: &[&str], sep: &str| parts.iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(sep);
    let playing = if hub.player.busy() {
        let p = hub.player.now();
        let app = hub.apps.get(&p.app).map_or(p.app.clone(), |i| i.app.app.name.clone());
        let title = joined(&[&p.title, &p.song], ": ");
        Some(json!({ "app": app, "title": title, "playing": p.state == "playing" }))
    } else {
        hub.android.media().map(|m| {
            let title = joined(&[&m.title, &m.artist], " by ");
            json!({ "app": m.app, "title": title, "playing": m.playing })
        })
    };

    Ok(json!({
        "now": now,
        "time": when(now),
        "unread": unread,
        "reminders": { "went_off": went_off, "today": later, "apps": app_reminders },
        "next_alarm": next_alarm,
        "timers": timers,
        "routines": routines,
        "journal": { "count": journal.len(), "last": journal.first().map(|e| e.text.chars().take(120).collect::<String>()) },
        "recordings": { "count": recordings.len(), "last": recordings.first().map(|m| m.label.clone()) },
        "notifications": { "count": notes.len(), "newest": notes.first().map(|n| json!({ "app": n.app, "title": n.title })) },
        "playing": playing,
        "missed": missed,
    }))
}
