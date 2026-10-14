//! HTTP + WebSocket on 127.0.0.1 (docs/specs/r1d.md).
//!
//! - The shell's static files, with an index.html fallback.
//! - `/api/*` for the shell: state, motor, opening allowlisted Android settings screens.
//! - `/ws`: the event stream (a `hello` snapshot, then stamped events).
//! - `/harness/*` (only with `--harness`): input injection, the event log, screenshots.
//!
//! Security gap (accepted for R1, closed in R4 by a unix socket in r1d's own SELinux domain): any
//! process on the R1 can reach 127.0.0.1:7171. Today the only other apps are Android's own.

use crate::device;
use crate::gesture::Raw;
use crate::hub::{Event, Hub, Source, Stamped};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tower_http::services::{ServeDir, ServeFile};

type Shared = Arc<Hub>;

pub async fn serve(hub: Shared) -> anyhow::Result<()> {
    let mut app = Router::new()
        .route("/token", get(host_token))
        .route("/api/state", get(state))
        .route("/api/motor", post(set_motor))
        .route("/api/android/open", post(open_screen))
        .route("/api/android/apps", get(android_apps).put(android_apps_set))
        .route("/api/android/key", post(android_key))
        .route("/api/android/notifications", get(android_notes).post(android_notes_set))
        .route("/api/android/media", get(android_media).put(android_media_set))
        .route("/api/cards/image", get(card_image))
        .route("/api/android/overlay", post(android_overlay))
        .route("/api/android/settings-access", post(android_settings_access))
        .route("/api/android/bluetooth-access", post(android_bluetooth_access))
        .route("/api/android/front", get(android_front))
        .route("/api/android/permissions", post(android_permissions))
        .route("/api/display", post(display_set))
        .route(
            "/api/media",
            get(media_list).post(media_add).layer(axum::extract::DefaultBodyLimit::max(crate::media::MAX_PHOTO + 4096)),
        )
        .route("/api/media/{id}", get(media_get).put(media_label).delete(media_remove))
        .route("/api/media/{id}/file", get(media_file))
        .route("/api/media/{id}/peaks", get(media_peaks))
        .route("/api/journal/listen", post(journal_listen).delete(journal_unlisten))
        .route("/api/journal", get(journal_list).post(journal_add))
        .route("/api/journal/{id}", put(journal_edit).delete(journal_remove))
        .route("/api/recorder", get(recorder_get).put(recorder_put))
        .route("/api/recorder/start", post(recorder_start))
        .route("/api/recorder/stop", post(recorder_stop))
        .route("/api/recorder/mark", post(recorder_mark))
        .route("/api/translator", get(translator_get).post(translator_start).delete(translator_stop))
        .route("/api/translator/text", post(translator_text))
        .route("/api/translator/speech/{id}", get(translator_speech))
        .route("/api/alarms", get(alarms_list).post(alarms_add))
        .route("/api/alarms/{id}", axum::routing::delete(alarms_remove))
        .route("/api/alarms/{id}/on", post(alarms_on))
        .route("/api/alarms/{id}/stop", post(alarms_stop))
        .route("/api/alarms/{id}/snooze", post(alarms_snooze))
        .route("/api/alarms/{id}/seen", post(alarms_seen))
        .route("/api/alarms/{id}/done", post(alarms_done))
        .route("/api/alarms/clear_done", post(alarms_clear_done))
        .route("/api/lock", get(lock_get).put(lock_set))
        .route("/api/lock/unlock", post(lock_unlock))
        .route("/api/lock/check", post(lock_check))
        .route("/api/lock/off", post(lock_off))
        .route("/api/lock/grace", post(lock_grace))
        .route("/api/routines", get(routines_list).post(routines_add))
        .route("/api/routines/{id}", axum::routing::delete(routines_remove))
        .route("/api/routines/{id}/on", post(routines_on))
        .route("/api/routines/{id}/run", post(routines_run))
        .route("/api/whatsup", get(whatsup))
        .route("/api/missed", get(missed_get).delete(missed_clear))
        .route("/api/browser/result", post(browser_result))
        .route("/api/apps/window/result", post(app_window_result))
        .route("/harness/app_window", post(harness_app_window))
        .route("/harness/dry_run", post(harness_dry_run))
        .route("/harness/commit_card", post(harness_commit_card))
        .route("/api/browser/command", post(browser_command))
        .route("/harness/browser", post(harness_browser))
        .route("/harness/perf", get(harness_perf).delete(harness_perf_reset))
        .route("/harness/abilities/{agent}", get(abilities_get))
        .route("/api/missed/seen", post(missed_seen))
        .route("/api/notices", get(notices_get))
        .route("/api/notices/{id}/seen", post(notices_seen))
        .route("/api/connections", get(connections_get))
        .route("/api/connections/check", post(connections_check))
        .route("/api/connections/login", post(connections_login))
        .route("/api/connections/agent", post(connections_agent))
        .route("/api/connections/always", post(connections_always))
        .route("/api/connections/add", post(connections_add))
        .route("/api/connections/remove", post(connections_remove))
        .route("/api/timers", get(timers_list).post(timers_add))
        .route("/api/timers/{id}", axum::routing::delete(timers_remove))
        .route("/api/timers/{id}/{op}", post(timers_op))
        .route("/api/time", get(time_get).post(time_set))
        .route("/api/time/auto", post(time_auto))
        .route("/api/screen", post(screen))
        .route("/api/voice", get(voice_status))
        .route("/api/voice/talk", post(voice_talk))
        .route("/api/mic", get(mic_ws))
        .route("/api/apps", get(apps_list).post(apps_install))
        .route("/api/apps/{id}", get(app_get).delete(app_remove))
        .route("/api/apps/{id}/history", get(app_history))
        .route("/api/apps/{id}/export", get(app_export))
        .route("/api/apps/{id}/edit", post(app_edit))
        .route("/api/apps/{id}/undo", post(app_undo))
        .route("/api/apps/{id}/restore", post(app_restore))
        .route("/api/trash", get(trash_list))
        .route("/api/backup", get(backup_list))
        .route("/api/backup/file", get(backup_file))
        .route("/api/apps/{id}/init", post(app_init))
        .route("/api/apps/{id}/query", post(app_query))
        .route("/api/apps/{id}/step", post(app_step))
        .route("/api/apps/{id}/settings", get(app_settings).post(app_set_settings))
        .route("/api/apps/{id}/erase", post(app_erase))
        .route("/api/apps/{id}/state", post(app_state))
        .route("/api/apps/{id}/open", post(app_open))
        .route("/api/apps/{id}/close", post(app_close))
        .route("/api/apps/{id}/services", get(app_services))
        .route("/api/apps/{id}/services/vars", post(app_service_vars))
        .route("/api/apps/{id}/permissions", get(app_permissions).post(app_decide))
        .route("/api/apps/{id}/session", post(app_session))
        .route("/api/apps/{id}/open_url", post(app_open_url))
        .route("/api/apps/{id}/media", post(app_media))
        .route("/api/apps/{id}/fetch", post(app_fetch))
        .route("/api/apps/{id}/player", get(app_player))
        .route("/api/player", get(player_now).put(player_report))
        .route("/api/player/stream/{id}", get(player_stream))
        .route("/api/apps/{id}/run", post(app_run))
        .route("/api/apps/{id}/undo_run", post(app_undo_run))
        .route("/api/apps/{id}/errors", get(app_errors).post(app_report_error).delete(app_clear_errors))
        .route("/api/apps/{id}/fix", post(app_fix))
        .route("/api/builds/model", get(build_model_get).put(build_model_put))
        .route("/api/glance", get(glance_get).put(glance_put))
        .route("/api/look", get(look_get).put(look_put))
        .route("/api/battery", post(battery_estimate))
        .route("/api/home", get(home_get).put(home_put))
        .route("/api/link", get(link_get))
        .route("/api/link/pair", post(link_pair))
        .route("/api/link/scan", post(link_scan).layer(axum::extract::DefaultBodyLimit::max(4 << 20)))
        .route("/api/link/ping", post(link_ping))
        .route("/api/link/close", post(link_close))
        .route("/api/link/{id}", axum::routing::delete(link_unlink))
        .route("/api/approvals", get(approvals_get))
        .route("/api/approvals/heard", post(approval_heard))
        .route("/api/approvals/{id}", post(approval_answer))
        .route("/api/device", get(device_get).post(device_report))
        .route("/api/agents/{id}/can", put(agent_can))
        .route("/api/apps/{id}/copy", post(app_copy))
        .route("/api/create", post(create_app))
        .route("/api/builds/stop", post(build_stop))
        .route("/api/agents", get(agents_list).post(agents_new))
        .route("/api/agents/order", put(agents_order))
        .route("/api/agents/paused", put(agents_paused))
        .route("/api/mail", get(mail_list))
        .route("/api/agents/{id}", get(agent_get).put(agent_put).delete(agent_delete))
        .route("/api/memories", get(memories_list))
        .route("/api/memories/{id}", delete(memory_delete))
        .route("/api/onboarding", get(onboarding_get).put(onboarding_put))
        .route("/api/wifi", get(wifi_get))
        .route("/api/wifi/scan", post(wifi_scan))
        .route("/api/wifi/connect", post(wifi_connect))
        .route("/api/accounts", get(accounts_get))
        .route("/api/accounts/{brain}/login", post(account_login))
        .route("/api/accounts/claude/code", post(account_code))
        .route("/api/device/change", axum::routing::put(device_change))
        .route("/api/power", post(power))
        .route("/api/accounts/{brain}/logout", post(account_logout))
        .route("/api/qr", get(qr))
        .route("/api/share", post(share_start).delete(share_stop))
        .route("/api/boot", get(boot_get).put(boot_put))
        .route("/api/bootanim", post(bootanim_save).layer(axum::extract::DefaultBodyLimit::max(24 << 20)))
        .route("/api/scan", post(scan).layer(axum::extract::DefaultBodyLimit::max(4 << 20)))
        .route("/api/scan/install", post(scan_install))
        .route("/api/chats", get(chats_list).post(chats_new))
        .route("/api/chats/warm", post(chats_warm))
        .route("/api/chats/{id}", get(chat_get).delete(chat_delete))
        .route("/api/chats/{id}/messages", post(chat_send))
        .route("/api/chats/{id}/seen", post(chat_seen))
        .route("/api/chats/{id}/stop", post(chat_stop))
        .route("/api/skills", get(skills_get).post(skills_add))
        .route("/api/skills/on", post(skills_on))
        .route("/api/skills/{name}", axum::routing::delete(skills_remove))
        .route("/mcp", post(crate::mcp::handle))
        .route("/hooks/pre-tool-use", post(crate::mcp::hook))
        .route("/ws", get(ws));
    if hub.cfg.harness {
        app = app
            .route("/harness/state", get(harness_state))
            .route("/harness/events", get(harness_events))
            .route("/harness/wheel", post(harness_wheel))
            .route("/harness/button", post(harness_button))
            .route("/harness/tilt", post(harness_tilt))
            .route("/harness/rest", post(harness_rest))
            .route("/harness/screenshot", get(harness_screenshot))
            .route("/harness/jail", get(harness_jail))
            .route("/harness/say", post(harness_say))
            .route("/harness/system1", post(harness_system1))
        .route("/harness/summarize", post(harness_summarize))
        .route("/harness/tidy", post(harness_tidy))
            .route("/harness/translate", post(harness_translate))
            .route("/harness/open", post(harness_open))
            .route("/harness/bootanim", post(harness_bootanim))
            .route("/harness/net", get(harness_net))
            .route("/harness/trigger", post(harness_trigger))
            .route("/harness/hear", post(harness_hear))
            .route("/harness/tts", post(harness_tts))
            .route("/harness/paradee", post(harness_paradee));
    }
    // Every route above takes a token on the R1 (ADR-0026); the shell's static files don't.
    let app = app.layer(axum::middleware::from_fn_with_state(hub.clone(), crate::auth::layer));
    let shell = ServeDir::new(&hub.cfg.shell).fallback(ServeFile::new(hub.cfg.shell.join("index.html")));
    let app = app.fallback_service(shell).with_state(hub.clone());
    // Linked computers call allowed routes through it (crate::link).
    hub.link.set_router(app.clone());

    let listener = tokio::net::TcpListener::bind(hub.cfg.listen).await?;
    tracing::info!(addr = %hub.cfg.listen, shell = %hub.cfg.shell.display(), harness = hub.cfg.harness, "serving");
    axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await?;
    Ok(())
}

fn snapshot(hub: &Hub) -> Value {
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_s": hub.started.elapsed().as_secs(),
        "sim": hub.cfg.sim,
        "harness": hub.cfg.harness,
        "devices": hub.devices,
        "motor": { "deg": hub.motor.get(), "moving": hub.motor.moving(), "sim": hub.motor.is_sim() },
        "system": device::system(),
        "stats": hub.stats(),
        "clients": hub.clients(),
    })
}

async fn state(State(hub): State<Shared>) -> Json<Value> {
    Json(snapshot(&hub))
}

#[derive(Deserialize)]
struct MotorReq {
    /// Turn to this (0 faces you, 180 the world).
    #[serde(default)]
    deg: Option<u16>,
    /// `you` (seated against the stop) or `world`.
    #[serde(default)]
    face: Option<String>,
    /// Re-home against the stop at 0, then go to `deg` (or where it was).
    #[serde(default)]
    home: bool,
}

/// The camera's motor (ADR-0046): moves one at a time, the latest target winning; `motor {deg, moving}` says where it's
/// going, and again when it comes to rest.
async fn set_motor(State(hub): State<Shared>, Json(req): Json<MotorReq>) -> Response {
    let deg = match (req.home, req.face.as_deref(), req.deg) {
        (true, _, d) => hub.motor.home(d.or_else(|| hub.motor.get()).unwrap_or(90)),
        (false, Some("you"), _) => hub.motor.face_user(),
        (false, Some("world"), _) => hub.motor.set(crate::device::WORLD),
        (false, None, Some(d)) => hub.motor.set(d),
        _ => return error(StatusCode::BAD_REQUEST, "give deg, face (you or world), or home".into()),
    };
    let moving = hub.motor.moving();
    hub.emit(Event::Motor { deg, moving }, Source::Api);
    Json(json!({ "deg": deg, "moving": moving })).into_response()
}

#[derive(Deserialize)]
struct OpenReq {
    #[serde(default)]
    screen: Option<String>,
    /// An installed Android app, by package (crate::android).
    #[serde(default)]
    package: Option<String>,
    /// Aurora Store: `true` for its home, or a package for that app's page.
    #[serde(default)]
    store: Option<Value>,
    /// A link, opened in the installed app that claims it (`package`, if given, keeps it to that one) (T-112).
    #[serde(default)]
    link: Option<String>,
}

/// Opens an Android settings screen, or `home`: r1os's own (the shell's double click from another app,
/// which Android won't let a background app do for itself); or an installed Android app (`package`), or
/// Aurora Store (`store`: `true`, or a package for its page). Allowlisted: the shell can't start arbitrary
/// intents, only those, with a valid package name.
async fn open_screen(Json(req): Json<OpenReq>) -> Response {
    let opened = match (&req.package, &req.store) {
        _ if req.link.is_some() => {
            let link = req.link.as_deref().unwrap_or_default();
            Some(crate::android::open_link(link, req.package.as_deref()).await.map(|p| json!({ "opened": p, "link": link })))
        }
        (Some(p), _) => Some(crate::android::open(p).await.map(|()| json!({ "opened": p }))),
        (None, Some(Value::String(p))) => Some(crate::android::store(Some(p)).await.map(|()| json!({ "store": p }))),
        (None, Some(Value::Bool(true))) => Some(crate::android::store(None).await.map(|()| json!({ "store": true }))),
        _ => None,
    };
    if let Some(r) = opened {
        return match r {
            Ok(v) => Json(v).into_response(),
            Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        };
    }
    let Some(screen) = req.screen else { return error(StatusCode::BAD_REQUEST, "screen, package or store".into()) };
    let (action, category) = match screen.as_str() {
        "home" => ("android.intent.action.MAIN", Some("android.intent.category.HOME")),
        "settings" => ("android.settings.SETTINGS", None),
        "wifi" => ("android.settings.WIFI_SETTINGS", None),
        "bluetooth" => ("android.settings.BLUETOOTH_SETTINGS", None),
        "developer" => ("android.settings.APPLICATION_DEVELOPMENT_SETTINGS", None),
        "about" => ("android.settings.DEVICE_INFO_SETTINGS", None),
        other => return error(StatusCode::BAD_REQUEST, format!("unknown screen '{other}'")),
    };
    if !cfg!(target_os = "android") {
        return error(StatusCode::NOT_IMPLEMENTED, "not on Android".into());
    }
    let mut args = vec!["activity", "start", "-a", action];
    if let Some(c) = category {
        args.extend(["-c", c]);
    }
    // `cmd activity` rather than `am`: `am` is a script around it, and a deadline kill must reach `cmd`.
    match crate::tool::run("cmd", &args, crate::tool::DEADLINE).await {
        Ok(o) if o.ok => Json(json!({ "opened": action })).into_response(),
        Ok(o) => error(StatusCode::INTERNAL_SERVER_ERROR, o.stderr),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct ImageQuery {
    url: String,
}

/// A picture on a chat's card (crate::cards), fetched and kept by r1d: the host never fetches the web itself.
async fn card_image(State(hub): State<Shared>, Query(q): Query<ImageQuery>) -> Response {
    let (data, fetcher) = (hub.cfg.data.clone(), hub.services.fetcher());
    match tokio::task::spawn_blocking(move || crate::cards::image(&data, &fetcher, &q.url)).await {
        Ok(Ok((bytes, kind))) => ([(axum::http::header::CONTENT_TYPE, kind), (axum::http::header::CACHE_CONTROL, "max-age=86400")], bytes).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_GATEWAY, e),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// The Android apps installed (crate::android), as the host last listed them.
async fn android_apps(State(hub): State<Shared>) -> Json<Value> {
    Json(json!({ "apps": hub.android.list() }))
}

#[derive(Deserialize)]
struct AndroidAppsReq {
    apps: Vec<crate::android::AndroidApp>,
}

/// The host's list of Android apps: when it connects, and when one is installed, updated or removed.
async fn android_apps_set(State(hub): State<Shared>, Json(req): Json<AndroidAppsReq>) -> Json<Value> {
    hub.android.set(req.apps);
    Json(json!({ "apps": hub.android.list().len() }))
}

/// Android apps' notifications (crate::android), newest first.
async fn android_notes(State(hub): State<Shared>) -> Json<Value> {
    Json(json!({ "notifications": hub.android.notes() }))
}

#[derive(Deserialize)]
struct NotesReq {
    /// `posted`, `removed` or `reset`.
    op: String,
    #[serde(default)]
    note: Option<crate::android::Note>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    notes: Vec<crate::android::Note>,
}

/// The host's notification listener: one posted or removed, or all of them when it connects.
async fn android_notes_set(State(hub): State<Shared>, Json(req): Json<NotesReq>) -> Response {
    match (req.op.as_str(), req.note, req.key) {
        ("posted", Some(n), _) => {
            // With the display off, it's one you missed (crate::missed): kept for a day, for what you missed.
            if !hub.services.display_on()
                && let Err(e) = hub.core.with(|db| crate::missed::record(db, &n, crate::core::now()))
            {
                tracing::warn!(error = %format!("{e:#}"), "missed: couldn't keep a notification");
            }
            hub.android.note_posted(n)
        }
        ("removed", _, Some(k)) => hub.android.note_removed(&k),
        ("reset", _, _) => hub.android.notes_reset(req.notes),
        (op, _, _) => return error(StatusCode::BAD_REQUEST, format!("`{op}` needs its note or key")),
    }
    Json(json!({ "notifications": hub.android.notes().len() })).into_response()
}

#[derive(Deserialize)]
struct KeyReq {
    key: String,
}

/// A key for the Android app in front (`center`, `back`): the shell's click there is DPAD center (Q24).
async fn android_key(Json(req): Json<KeyReq>) -> Response {
    match crate::android::key(&req.key).await {
        Ok(()) => Json(json!({ "key": req.key })).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct ScreenReq {
    on: bool,
}

/// Turns the display off (KEYCODE_SLEEP) or on (KEYCODE_WAKEUP). The side button no longer does
/// (it's BUTTON_1 + WAKE in r1os-base's keylayout), so the shell asks: home's Sleep control, Launcher → Sleep.
async fn screen(Json(req): Json<ScreenReq>) -> Response {
    if !cfg!(target_os = "android") {
        return error(StatusCode::NOT_IMPLEMENTED, "not on Android".into());
    }
    let key = if req.on { "224" } else { "223" };
    // `cmd input` rather than `input`: `input` is a script around it, and a deadline kill must reach `cmd`.
    match crate::tool::run("cmd", &["input", "keyevent", key], crate::tool::DEADLINE).await {
        Ok(o) if o.ok => Json(json!({ "on": req.on })).into_response(),
        Ok(o) => error(StatusCode::INTERNAL_SERVER_ERROR, o.stderr),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ---- voice (crate::voice) ---------------------------------------------------------------------

async fn voice_status(State(hub): State<Shared>) -> Json<Value> {
    Json(hub.voice.get().map_or_else(|| json!({ "stt": "off" }), |v| v.status()))
}

/// A talk started by touch (T-156): when it began, and which it is, so a guard ends only its own.
static TOUCH_TALK: std::sync::Mutex<Option<(std::time::Instant, u64)>> = std::sync::Mutex::new(None);
static TOUCH_TALKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// A talk by touch nobody sends goes by itself after this: the mic isn't left open.
const TOUCH_TALK_MAX: Duration = Duration::from_secs(60);

/// Talking by touch (T-156; docked, the mic on home): `start` is the side button pressed and held, `send` let go, and
/// `cancel` drops what was heard (crate::voice). The button's own edges and hold, straight to the hold, so crate::voice
/// and the shell can't tell them apart and no tap or click is made of it.
async fn voice_talk(State(hub): State<Shared>, Json(req): Json<Value>) -> Json<Value> {
    match req["op"].as_str().unwrap_or_default() {
        "start" => {
            let id = TOUCH_TALKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            {
                let mut t = TOUCH_TALK.lock().unwrap();
                if t.is_some() {
                    return Json(json!({ "talking": true }));
                }
                *t = Some((std::time::Instant::now(), id));
            }
            hub.emit(Event::Button { down: true }, Source::Api);
            hub.emit(Event::Gesture { kind: crate::gesture::GestureKind::HoldStart, held_ms: None, woke: false, shortcut: None }, Source::Api);
            let h = hub.clone();
            tokio::spawn(async move {
                tokio::time::sleep(TOUCH_TALK_MAX).await;
                let mine = TOUCH_TALK.lock().unwrap().as_ref().is_some_and(|(_, i)| *i == id);
                if mine {
                    touch_talk_end(&h, true);
                }
            });
            Json(json!({ "talking": true }))
        }
        "send" => Json(json!({ "talking": false, "ended": touch_talk_end(&hub, true) })),
        "cancel" => {
            hub.emit(Event::TalkCancel, Source::Api);
            Json(json!({ "talking": false, "ended": touch_talk_end(&hub, false) }))
        }
        _ => Json(json!({ "error": "op: start, send or cancel" })),
    }
}

/// A talk by touch let go: sent (the hold's end, as the button's), or after a cancel, only the button up.
fn touch_talk_end(hub: &Hub, send: bool) -> bool {
    let Some((began, _)) = TOUCH_TALK.lock().unwrap().take() else { return false };
    hub.emit(Event::Button { down: false }, Source::Api);
    if send {
        let held = Some(began.elapsed().as_millis() as u64);
        hub.emit(Event::Gesture { kind: crate::gesture::GestureKind::HoldEnd, held_ms: held, woke: false, shortcut: None }, Source::Api);
    }
    true
}

/// The simulator page's mic: "start"/"stop" go out, 16 kHz mono s16le PCM comes back as binary.
/// (On the R1 the host app owns the mic, over crate::voice::MIC_ADDR.)
async fn mic_ws(State(hub): State<Shared>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| async move {
        let Some(voice) = hub.voice.get().cloned() else { return };
        sim_mic(&voice, socket).await;
        voice.detach_mic();
    })
}

async fn sim_mic(voice: &crate::voice::Voice, mut socket: WebSocket) {
    let mut cmds = voice.attach_mic();
    let mut carry = None;
    loop {
        tokio::select! {
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { return }; // another mic owner took over
                let t = if cmd == crate::voice::MicCmd::Start { "start" } else { "stop" };
                if socket.send(Message::Text(t.into())).await.is_err() {
                    return;
                }
            }
            msg = socket.recv() => match msg {
                Some(Ok(Message::Binary(b))) => voice.audio(crate::voice::pcm16(&b, &mut carry)),
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                Some(Ok(_)) => {}
            }
        }
    }
}

async fn ws(State(hub): State<Shared>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| client(hub, socket))
}

async fn client(hub: Shared, mut socket: WebSocket) {
    let mut rx = hub.subscribe();
    let hello = json!({ "type": "hello", "state": snapshot(&hub) });
    if socket.send(Message::Text(hello.to_string().into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Ok(ev) => {
                    if socket.send(Message::Text(serde_json::to_string(&ev).unwrap_or_default().into())).await.is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => tracing::warn!(n, "ws client lagged"),
                Err(_) => return,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {} // the socket is events-only; commands go over HTTP
            },
        }
    }
}

// ---- apps (docs/specs/apps.md, ADR-0006) -----------------------------------------------------------
//
// The shell renders apps itself and calls these for data. Queries and steps are named/indexed: r1d
// looks the SQL up in the compiled app (crate::apps), so the caller can't run its own SQL.

/// Runs blocking app-store work (SQLite) off the async threads; maps errors to 400 { error }.
async fn blocking<T: Serialize + Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> Response {
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn apps_list(State(hub): State<Shared>) -> Json<Value> {
    Json(json!({ "apps": hub.apps.list() }))
}

#[derive(Deserialize)]
struct InstallReq {
    source: String,
    /// An export's rows by table, and its settings (T-048): put into the app once it's installed.
    #[serde(default)]
    data: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    settings: Option<serde_json::Map<String, Value>>,
}

/// Compiles and installs a spec. 422 with the compiler's diagnostics if it has errors.
async fn apps_install(State(hub): State<Shared>, Json(req): Json<InstallReq>) -> Response {
    let h = hub.clone();
    let installed = tokio::task::spawn_blocking(move || {
        let (summary, warnings) = h.apps.install_recorded(&req.source, "installed from a file")?;
        // An export with its data (T-048): the rows and settings go in now; a failure leaves the app installed.
        let imported = (req.data.is_some() || req.settings.is_some()).then(|| {
            h.apps.import_data(&summary.id, &req.data.unwrap_or_default(), &req.settings.unwrap_or_default()).map_err(|e| format!("{e:#}"))
        });
        Ok::<_, crate::apps::Rejected>((summary, warnings, imported))
    })
    .await;
    match installed {
        Ok(Ok((summary, warnings, imported))) => {
            hub.emit(Event::AppInstalled { app: summary.id.clone() }, Source::Api);
            let mut out = json!({ "ok": true, "app": summary, "warnings": warnings });
            match imported {
                Some(Ok(rows)) => out["imported"] = json!(rows),
                Some(Err(e)) => out["import_error"] = json!(e),
                None => {}
            }
            Json(out).into_response()
        }
        Ok(Err(rejected)) => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "ok": false, "diagnostics": rejected.diagnostics })))
                .into_response()
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// An app with its data (T-048): `{enee_export, source, data, settings}`, which `POST /api/apps` installs back.
async fn app_export(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    blocking(move || hub.apps.export(&id)).await
}

async fn app_get(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    match hub.apps.get(&id) {
        Some(i) => Json(json!({ "app": i.app, "source": i.source })).into_response(),
        None => error(StatusCode::NOT_FOUND, format!("no app `{id}`")),
    }
}

async fn app_remove(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    let h = hub.clone();
    let id2 = id.clone();
    let r = blocking(move || h.apps.remove(&id2).map(|()| json!({ "removed": true }))).await;
    if r.status().is_success() {
        hub.services.forget(&id);
        let _ = hub.core.with(|db| crate::notices::forget_app(db, &id));
        hub.emit(Event::AppRemoved { app: id }, Source::Api);
    }
    r
}

/// Apps' reminders that went off and haven't been seen (T-049), newest first, each with its app's name, icon and
/// accent so the shell can show it at once.
async fn notices_get(State(hub): State<Shared>) -> Response {
    blocking(move || {
        let list = hub.core.with(|db| crate::notices::unseen(db, crate::core::now()))?;
        let notices: Vec<Value> = list
            .into_iter()
            .map(|n| {
                let app = hub.apps.get(&n.app);
                let mut v = json!(n);
                v["name"] = json!(app.as_ref().map_or(n.app.clone(), |i| i.app.app.name.clone()));
                v["icon"] = json!(app.as_ref().map_or("bell".to_owned(), |i| i.app.app.icon.clone()));
                if let Some(a) = app.and_then(|i| i.app.app.accent.clone()) {
                    v["accent"] = json!(a);
                }
                v
            })
            .collect();
        Ok(json!({ "notices": notices }))
    })
    .await
}

async fn notices_seen(State(hub): State<Shared>, Path(id): Path<i64>) -> Response {
    blocking(move || Ok(json!({ "seen": hub.core.with(|db| crate::notices::seen(db, id))? }))).await
}

/// A token for the app's page (ADR-0026): the host hands it to the app's WebView, and it reaches only that
/// app's runtime endpoints.
async fn app_session(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    if hub.apps.get(&id).is_none() {
        return error(StatusCode::NOT_FOUND, format!("no app `{id}`"));
    }
    Json(json!({ "token": hub.auth.app_token(&id) })).into_response()
}

#[derive(Deserialize)]
struct ErrorReq {
    code: String,
    message: String,
    /// The action or view, if the page knows.
    #[serde(default)]
    place: String,
}

/// The app's page hit a problem (its runtime's error): kept for a fix.
async fn app_report_error(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<ErrorReq>) -> Response {
    let h = hub.clone();
    let app = id.clone();
    match tokio::task::spawn_blocking(move || h.apps.record_error(&app, &req.code, &req.message, &req.place)).await {
        Ok(Ok(count)) => {
            hub.emit(Event::AppErrors { app: id, count }, Source::Api);
            Json(json!({ "count": count })).into_response()
        }
        Ok(Err(e)) => error(StatusCode::NOT_FOUND, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn app_errors(State(hub): State<Shared>, Path(id): Path<String>) -> Json<Value> {
    Json(json!({ "errors": hub.apps.errors(&id) }))
}

/// Forgets what went wrong (it's been dealt with another way).
async fn app_clear_errors(State(hub): State<Shared>, Path(id): Path<String>) -> Json<Value> {
    hub.apps.clear_errors(&id);
    hub.emit(Event::AppErrors { app: id, count: 0 }, Source::Api);
    Json(json!({ "errors": [] }))
}

#[derive(Deserialize, Default)]
struct FixReq {
    model: Option<String>,
}

/// A fix: an edit build asked to repair what the app's page reported.
async fn app_fix(State(hub): State<Shared>, Path(id): Path<String>, body: Option<Json<FixReq>>) -> Response {
    let errors = hub.apps.errors(&id);
    if errors.is_empty() {
        return error(StatusCode::CONFLICT, "nothing has gone wrong with it".into());
    }
    let model = body.and_then(|b| b.0.model);
    start_edit(&hub, id, crate::apps::fix_request(&errors), model)
}

/// The model app builds use, and the ones they can.
/// Linked computers (ADR-0048): this R1's key, the computers it trusts, and which are connected.
async fn link_get(State(hub): State<Shared>) -> Json<Value> {
    Json(hub.link.status(&hub).await)
}

#[derive(Deserialize)]
struct LinkPairReq {
    /// The computer's QR, as text (`r1os-link:1?…`).
    code: String,
}

async fn link_pair(State(hub): State<Shared>, Json(req): Json<LinkPairReq>) -> Response {
    match hub.link.begin_pair(&hub, &req.code).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

/// A frame of the camera's preview (JPEG or PNG): a QR in it is read, and an r1os link code starts pairing (the
/// computer confirms; `link {state}` says how it went).
async fn link_scan(State(hub): State<Shared>, body: axum::body::Bytes) -> Response {
    let t = std::time::Instant::now();
    let found = match tokio::task::spawn_blocking(move || crate::link::decode_qr(&body)).await {
        Ok(Ok(f)) => f,
        Ok(Err(e)) => return error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let decode_ms = t.elapsed().as_millis();
    match found {
        None => Json(json!({ "found": false, "decode_ms": decode_ms })).into_response(),
        Some(text) if !text.starts_with("r1os-link:") => Json(json!({ "found": true, "link": false, "decode_ms": decode_ms })).into_response(),
        Some(text) => match hub.link.begin_pair(&hub, &text).await {
            Ok(v) => Json(json!({ "found": true, "link": true, "decode_ms": decode_ms, "paired": v })).into_response(),
            Err(e) => Json(json!({ "found": true, "link": true, "decode_ms": decode_ms, "error": format!("{e:#}") })).into_response(),
        },
    }
}

#[derive(Deserialize)]
struct LinkPingReq {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    n: Option<usize>,
    /// Only through a relay: "away from home", tried at home.
    #[serde(default)]
    relay_only: bool,
}

/// Round trips to a linked computer (phase 0's measurement).
async fn link_ping(State(hub): State<Shared>, Json(req): Json<LinkPingReq>) -> Response {
    let reach = if req.relay_only { r1_link::Reach::RelayOnly } else { r1_link::Reach::Anywhere };
    match hub.link.ping(&hub, req.id.as_deref(), req.n.unwrap_or(10), reach).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

/// Lets the link go (battery: nothing kept alive).
async fn link_close(State(hub): State<Shared>) -> Json<Value> {
    hub.link.close().await;
    Json(json!({ "closed": true }))
}

async fn link_unlink(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    match hub.link.unlink(&hub, &id).await {
        Ok(()) => Json(json!({ "unlinked": true })).into_response(),
        Err(e) => error(StatusCode::NOT_FOUND, format!("{e:#}")),
    }
}

/// Agents' requests waiting for your answer (ADR-0045), oldest first.
async fn approvals_get(State(hub): State<Shared>) -> Json<Value> {
    Json(json!({ "approvals": hub.approvals.pending() }))
}

#[derive(Deserialize)]
struct AnswerReq {
    allow: bool,
    /// Allow this agent this area from now on, without asking.
    #[serde(default)]
    always: bool,
}

/// Your answer on the R1's approval card (only the shell's own UI calls this).
/// What you said on an approval card (T-055): `{answer: "allow" | "always" | "deny" | null}`. It only says which; the
/// card answers as if you'd picked it.
async fn approval_heard(State(hub): State<Shared>, Json(req): Json<System1Req>) -> Json<Value> {
    let answer = tokio::task::spawn_blocking(move || crate::approvals::heard(&req.text, |p, names| hub.system1.closeness(p, names))).await.ok().flatten();
    Json(json!({ "answer": answer }))
}

async fn approval_answer(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<AnswerReq>) -> Response {
    blocking(move || {
        hub.approvals.answer(&hub, &id, req.allow, req.always)?;
        Ok(json!({ "answered": true }))
    })
    .await
}

/// The R1's own settings (ADR-0045): what the host reports and r1d's, and what they can be.
async fn device_get(State(hub): State<Shared>) -> Json<Value> {
    Json(json!({ "device": hub.device_settings.all(&hub), "options": crate::device_settings::options() }))
}

/// The host's report of Android's settings (brightness, timeout, volume, sounds).
async fn device_report(State(hub): State<Shared>, Json(v): Json<serde_json::Map<String, Value>>) -> Json<Value> {
    hub.device_settings.report(&v);
    Json(json!({ "ok": true }))
}

#[derive(Deserialize)]
struct CanReq {
    area: String,
    level: String,
}

/// What an agent may change (ADR-0045): an area's level (its space › What it can change).
async fn agent_can(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<CanReq>) -> Response {
    blocking(move || {
        anyhow::ensure!(crate::approvals::AREAS.contains(&req.area.as_str()), "unknown area `{}`", req.area);
        hub.core.set_permission(&id, &req.area, &req.level)?;
        hub.emit(Event::AgentsChanged, Source::Api);
        Ok(json!({ "can": hub.core.agent(&id).map(|a| a.can).unwrap_or_default() }))
    })
    .await
}

/// The apps pinned to home (ADR-0044), in order: `{pins: [app id | "android:<package>" | "tool:<id>"]}` (r1os's tools:
/// T-091).
async fn home_get(State(hub): State<Shared>) -> Json<Value> {
    Json(json!({ "pins": hub.core.home_pins() }))
}

#[derive(Deserialize)]
struct HomeReq {
    pins: Vec<String>,
}

/// Pins these apps to home, in this order (the shell's edit mode, Manage apps, the launcher's long press).
async fn home_put(State(hub): State<Shared>, Json(req): Json<HomeReq>) -> Response {
    blocking(move || {
        let pins = set_home_pins(&hub, &req.pins)?;
        Ok(json!({ "pins": pins }))
    })
    .await
}

/// The most apps pinned to home.
pub const MAX_PINS: usize = 12;

/// Checks, keeps and sends the apps pinned to home: installed mini-apps, and Android apps by package.
pub fn set_home_pins(hub: &crate::hub::Hub, pins: &[String]) -> anyhow::Result<Vec<String>> {
    let out = checked_pins(hub, pins)?;
    hub.core.set_home_pins(&out)?;
    hub.emit(Event::Home { pins: out.clone() }, Source::Api);
    Ok(out)
}

/// Apps to pin to home, checked: installed (or an Android package's name), no repeats, not too many.
pub fn checked_pins(hub: &crate::hub::Hub, pins: &[String]) -> anyhow::Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for p in pins {
        let p = p.trim();
        match (p.strip_prefix("android:"), p.strip_prefix("tool:")) {
            (Some(pkg), _) => anyhow::ensure!(
                !pkg.is_empty() && pkg.split('.').count() >= 2 && pkg.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_'),
                "`{pkg}` isn't an Android package name"
            ),
            (None, Some(tool)) => anyhow::ensure!(crate::screens::pinnable(tool), "there's no tool `{tool}`"),
            (None, None) => anyhow::ensure!(hub.apps.get(p).is_some(), "there's no app `{p}`"),
        }
        if !out.iter().any(|o| o == p) {
            out.push(p.to_owned());
        }
    }
    anyhow::ensure!(out.len() <= MAX_PINS, "home takes {MAX_PINS} apps at most");
    Ok(out)
}

/// The device's look (ADR-0043), and everything it can be.
async fn look_get(State(hub): State<Shared>) -> Json<Value> {
    Json(json!({ "look": hub.core.look(), "options": crate::look::options() }))
}

/// Changes the look: only what changes, merged in (`null` puts a value back to its default), then checked whole.
/// The host passes Android's estimate of the time to full charge as the battery changes (T-152): `{charge_ms}`, or null
/// when Android hasn't one (not charging, or not enough seen yet).
async fn battery_estimate(Json(b): Json<Value>) -> Json<Value> {
    device::set_android_estimate(b["charge_ms"].as_i64());
    Json(json!({ "ok": true }))
}

async fn look_put(State(hub): State<Shared>, Json(change): Json<Value>) -> Response {
    blocking(move || {
        let look = set_look(&hub, &change)?;
        Ok(json!({ "look": look }))
    })
    .await
}

/// Merges, checks, saves and sends a change to the look (Settings › Look, the agents' `set_look`).
pub fn set_look(hub: &crate::hub::Hub, change: &Value) -> anyhow::Result<crate::look::Look> {
    let look = checked_look(hub, change)?;
    hub.core.set_look(&look)?;
    hub.emit(Event::Look { look: serde_json::to_value(&look)? }, Source::Api);
    Ok(look)
}

/// The look with a change made, checked, but not saved.
pub fn checked_look(hub: &crate::hub::Hub, change: &Value) -> anyhow::Result<crate::look::Look> {
    let is_photo = |id: &str| hub.core.with(|db| crate::media::get(db, id)).ok().flatten().is_some_and(|m| m.kind == "photo");
    hub.core.look().changed(change, &is_photo).map_err(|e| anyhow::anyhow!(e))
}

/// The glance screen's widgets, in order (ADR-0042): `{widgets: [{app, name, icon, accent?, card?, note?}]}`.
async fn glance_get(State(hub): State<Shared>) -> Response {
    blocking(move || Ok(json!({ "widgets": hub.glance.widgets(&hub) }))).await
}

#[derive(Deserialize)]
struct GlanceReq {
    /// The apps whose cards show, in order.
    apps: Vec<String>,
}

/// Which apps' cards show on the glance screen, and in what order.
async fn glance_put(State(hub): State<Shared>, Json(req): Json<GlanceReq>) -> Response {
    blocking(move || {
        hub.glance.set_apps(&hub, &req.apps)?;
        Ok(json!({ "widgets": hub.glance.widgets(&hub) }))
    })
    .await
}

async fn build_model_get(State(hub): State<Shared>) -> Json<Value> {
    Json(json!({ "model": hub.core.build_model(), "models": crate::core::BUILD_MODELS }))
}

#[derive(Deserialize)]
struct ModelReq {
    model: String,
}

async fn build_model_put(State(hub): State<Shared>, Json(req): Json<ModelReq>) -> Response {
    match hub.core.set_build_model(&req.model) {
        Ok(()) => build_model_get(State(hub)).await.into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

/// The model a build asked for (one of BUILD_MODELS), else the R1's setting.
pub fn build_model(hub: &Hub, asked: Option<String>) -> Result<String, String> {
    match asked.map(|m| m.trim().to_ascii_lowercase()).filter(|m| !m.is_empty()) {
        Some(m) if crate::core::BUILD_MODELS.contains(&m.as_str()) => Ok(m),
        Some(m) => Err(format!("`{m}` isn't one of {}", crate::core::BUILD_MODELS.join(", "))),
        None => Ok(hub.core.build_model()),
    }
}

#[derive(Deserialize)]
struct RunReq {
    action: String,
    #[serde(default)]
    params: serde_json::Map<String, Value>,
}

/// Runs an app's action without opening it (System One's "log two glasses", ADR-0032), as the agent's
/// `run_action` does: `{said, undoable}`. Actions that ask first aren't run here.
async fn app_run(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<RunReq>) -> Response {
    let Some(i) = hub.apps.get(&id) else { return error(StatusCode::NOT_FOUND, format!("no app `{id}`")) };
    match i.app.actions.get(&req.action) {
        None => return error(StatusCode::NOT_FOUND, format!("`{id}` has no action `{}`", req.action)),
        Some(def) if def.get("confirm").is_some() => return error(StatusCode::CONFLICT, "that action asks first".into()),
        Some(_) => {}
    }
    let h = hub.clone();
    let app = id.clone();
    match tokio::task::spawn_blocking(move || h.apps.run_action(&app, &req.action, &req.params)).await {
        Ok(Ok(run)) => {
            hub.scheduler.changed(&id, &run.changed_tables);
            if !run.changed_tables.is_empty() {
                hub.emit(Event::AppChanged { app: id.clone(), tables: run.changed_tables.clone() }, Source::Api);
            }
            let mut said = run.said;
            said.extend(crate::player::perform(&hub, &i.app, &run.media));
            Json(json!({ "said": said, "undoable": run.undoable, "name": i.app.app.name })).into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Puts the app's last run back (within 10 minutes).
async fn app_undo_run(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    let h = hub.clone();
    let app = id.clone();
    match tokio::task::spawn_blocking(move || h.apps.undo_run(&app)).await {
        Ok(Ok((action, exec))) => {
            if !exec.changed_tables.is_empty() {
                hub.scheduler.changed(&id, &exec.changed_tables);
                hub.emit(Event::AppChanged { app: id.clone(), tables: exec.changed_tables }, Source::Api);
            }
            Json(json!({ "undone": action })).into_response()
        }
        Ok(Err(e)) => error(StatusCode::CONFLICT, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct SideStepReq {
    /// The step that asked (the runtime's StepRef).
    action: String,
    step: usize,
    #[serde(default)]
    url: String,
    #[serde(default)]
    text: String,
}

/// The app has step `index` of `action`, and it's a `kind` step.
fn side_step(app: &enee_spec::compiled::CompiledApp, req: &SideStepReq, kind: &str) -> Result<(), String> {
    step_is(app, &req.action, req.step, kind)
}

fn step_is(app: &enee_spec::compiled::CompiledApp, action: &str, index: usize, kind: &str) -> Result<(), String> {
    let step = app.actions.get(action).and_then(|a| a.get("steps")).and_then(Value::as_array).and_then(|s| s.get(index));
    match step {
        Some(s) if s.get(kind).is_some() => Ok(()),
        _ => Err(format!("step {index} of `{action}` isn't a {kind} step")),
    }
}

#[derive(Deserialize)]
struct MediaReq {
    /// `play`, `pause`, `resume`, `stop` or `toggle`.
    op: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    subtitle: String,
    /// The step that asked (the runtime's StepRef).
    action: String,
    step: usize,
}

/// An app's player step (ADR-0040): r1d checks the step, the app's `audio.play` permission and, to play, the URL
/// (the web only, and the hosts the permission names, if it names any); the host's player does it.
async fn app_media(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<MediaReq>) -> Response {
    let Some(i) = hub.apps.get(&id) else { return error(StatusCode::NOT_FOUND, format!("no app `{id}`")) };
    let op = req.op.as_str();
    if !["play", "pause", "resume", "stop", "toggle"].contains(&op) {
        return error(StatusCode::BAD_REQUEST, format!("unknown player step `{op}`"));
    }
    if let Err(e) = step_is(&i.app, &req.action, req.step, op) {
        return error(StatusCode::BAD_REQUEST, e);
    }
    if !crate::broker::allows(&hub.core, &i.app, "audio.play") {
        return error(StatusCode::FORBIDDEN, "this app isn't allowed to play audio".into());
    }
    if op != "play" {
        hub.player.control(&hub, Some(&id), op);
        return Json(json!({ "ok": true })).into_response();
    }
    let url = match crate::fetch::link_ok(&req.url) {
        Ok(u) => u,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)).unwrap_or_default();
    if !crate::broker::allows_audio_host(&i.app, &host) {
        return error(StatusCode::FORBIDDEN, format!("this app may only play from the hosts it names, not {host}"));
    }
    hub.core.grant_used(&id, "audio.play");
    let cut = |s: &str| s.chars().take(160).collect::<String>();
    hub.player.play(&hub, &id, &url, &cut(&req.title), &cut(&req.subtitle));
    Json(json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
struct FetchReq {
    url: String,
    /// `json` (the default), `text` or `feed`.
    #[serde(default, rename = "as")]
    read_as: Option<String>,
}

/// Requests per app per minute through `fetch` (the logic lane), before it's told to slow down.
const FETCHES_PER_MINUTE: u32 = 120;
static FETCHES: std::sync::Mutex<Option<std::collections::HashMap<String, (std::time::Instant, u32)>>> = std::sync::Mutex::new(None);

/// The web for an app's logic (`r1.fetch`, ADR-0041): only the hosts its `net.http` lists (each redirect's too),
/// through the apps' fetcher. `{status, body}`: JSON, text (64 KB at most) or a feed's items.
async fn app_fetch(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<FetchReq>) -> Response {
    let Some(i) = hub.apps.get(&id) else { return error(StatusCode::NOT_FOUND, format!("no app `{id}`")) };
    let host = url::Url::parse(&req.url).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)).unwrap_or_default();
    if !crate::broker::allows_host(&hub.core, &i.app, &host) {
        return error(StatusCode::FORBIDDEN, format!("the app's net.http doesn't allow {host}"));
    }
    {
        let mut guard = FETCHES.lock().unwrap();
        let map = guard.get_or_insert_with(std::collections::HashMap::new);
        let now = std::time::Instant::now();
        let entry = map.entry(id.clone()).or_insert((now, 0));
        if now.duration_since(entry.0) > std::time::Duration::from_secs(60) {
            *entry = (now, 0);
        }
        entry.1 += 1;
        if entry.1 > FETCHES_PER_MINUTE {
            return error(StatusCode::TOO_MANY_REQUESTS, format!("more than {FETCHES_PER_MINUTE} requests a minute"));
        }
    }
    let hosts: Vec<String> = i.app.permissions.iter().filter(|p| p.id == "net.http").flat_map(crate::broker::hosts).collect();
    let format = match req.read_as.as_deref() {
        Some("text") => crate::fetch::Format::Text,
        Some("feed") => crate::fetch::Format::Feed,
        _ => crate::fetch::Format::Json,
    };
    let fetcher = hub.services.fetcher();
    let url = req.url.clone();
    let got = tokio::task::spawn_blocking(move || {
        let allowed = |h: &str| hosts.iter().any(|x| x.eq_ignore_ascii_case(h));
        fetcher.get(&url, format, &allowed)
    })
    .await;
    let r = match got {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return error(StatusCode::BAD_GATEWAY, e.to_string()),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    hub.core.grant_used(&id, "net.http");
    let body = match format {
        crate::fetch::Format::Json => match serde_json::from_slice::<Value>(&r.body) {
            Ok(v) => v,
            Err(_) => return error(StatusCode::BAD_GATEWAY, format!("the response isn't JSON (HTTP {})", r.status)),
        },
        crate::fetch::Format::Feed => match crate::feed::parse(&String::from_utf8_lossy(&r.body)) {
            Ok(f) => json!({ "title": f.title, "link": f.link, "items": f.items }),
            Err(e) => return error(StatusCode::BAD_GATEWAY, e),
        },
        _ => {
            let text = String::from_utf8_lossy(&r.body);
            json!(text.chars().take(64 * 1024).collect::<String>())
        }
    };
    Json(json!({ "status": r.status, "body": body })).into_response()
}

/// What the player is doing, as this app's page binds it (`/player`).
async fn app_player(State(hub): State<Shared>, Path(id): Path<String>) -> Json<Value> {
    Json(json!({ "player": hub.player.now().for_app(&id) }))
}

/// What the player is doing, whichever app's it is (the glance screen).
async fn player_now(State(hub): State<Shared>) -> Json<Value> {
    Json(json!({ "player": hub.player.now() }))
}

#[derive(Deserialize)]
struct PlayerReport {
    state: String,
    #[serde(default)]
    error: Option<String>,
}

/// The host's player says what it's doing: `playing`, `paused`, `loading`, `stopped` or `error`.
async fn player_report(State(hub): State<Shared>, Json(req): Json<PlayerReport>) -> Response {
    if !["playing", "paused", "loading", "stopped", "error"].contains(&req.state.as_str()) {
        return error(StatusCode::BAD_REQUEST, format!("unknown player state `{}`", req.state));
    }
    hub.player.report(&hub, &req.state, req.error.as_deref());
    Json(json!({ "ok": true })).into_response()
}

/// The stream the host plays (crate::player::relay).
async fn player_stream(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    match crate::player::relay(hub.clone(), &id).await {
        Ok((kind, body)) => ([(axum::http::header::CONTENT_TYPE, kind)], body).into_response(),
        Err(e) => error(StatusCode::BAD_GATEWAY, e),
    }
}

/// An app's `open_url` step: r1d checks the step, the app's `open.url` permission and the link (the web only),
/// and the native shell opens it in its link viewer.
async fn app_open_url(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<SideStepReq>) -> Response {
    let Some(i) = hub.apps.get(&id) else { return error(StatusCode::NOT_FOUND, format!("no app `{id}`")) };
    if let Err(e) = side_step(&i.app, &req, "open_url") {
        return error(StatusCode::BAD_REQUEST, e);
    }
    if !crate::broker::allows(&hub.core, &i.app, "open.url") {
        return error(StatusCode::FORBIDDEN, "this app isn't allowed to open links".into());
    }
    let url = match crate::fetch::link_ok(&req.url) {
        Ok(u) => u,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    hub.core.grant_used(&id, "open.url");
    hub.emit(Event::OpenLink { app: id, url }, Source::Api);
    Json(json!({ "ok": true })).into_response()
}

/// An app's `copy` step: r1d checks the step and the app's `clipboard.write` permission, and the native shell
/// copies the text.
async fn app_copy(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<SideStepReq>) -> Response {
    let Some(i) = hub.apps.get(&id) else { return error(StatusCode::NOT_FOUND, format!("no app `{id}`")) };
    if let Err(e) = side_step(&i.app, &req, "copy") {
        return error(StatusCode::BAD_REQUEST, e);
    }
    if !crate::broker::allows(&hub.core, &i.app, "clipboard.write") {
        return error(StatusCode::FORBIDDEN, "this app isn't allowed to copy".into());
    }
    if req.text.len() > 20_000 {
        return error(StatusCode::BAD_REQUEST, "that's too much text to copy".into());
    }
    hub.core.grant_used(&id, "clipboard.write");
    hub.emit(Event::Copy { app: id, text: req.text }, Source::Api);
    Json(json!({ "ok": true })).into_response()
}

/// The shell opened the app's screen: its services start (crate::services). Sleep and leaving close it.
async fn app_open(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    blocking(move || {
        anyhow::ensure!(hub.apps.get(&id).is_some(), "no app `{id}`");
        hub.services.open(&hub, &id);
        hub.scheduler.view(&id, true);
        Ok(json!({ "open": true }))
    })
    .await
}

async fn app_close(State(hub): State<Shared>, Path(id): Path<String>) -> Json<Value> {
    hub.services.close(&id);
    hub.scheduler.view(&id, false);
    Json(json!({ "open": false }))
}

/// The app's live values now (its page asks once it's loaded; changes come as `app_services`).
async fn app_services(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    blocking(move || Ok(json!({ "values": hub.services.values(&id) }))).await
}

#[derive(Deserialize)]
struct VarsReq {
    values: serde_json::Map<String, Value>,
}

/// The page's values that its service URLs bind (`{{state.ticker}}`), when they change.
async fn app_service_vars(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<VarsReq>) -> Response {
    blocking(move || {
        hub.services.set_vars(&hub, &id, req.values);
        Ok(json!({ "ok": true }))
    })
    .await
}

/// The app's permissions with the user's decisions (crate::broker).
async fn app_permissions(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    blocking(move || {
        let installed = hub.apps.get(&id).ok_or_else(|| anyhow::anyhow!("no app `{id}`"))?;
        Ok(json!({ "permissions": crate::broker::states(&hub.core, &installed.app) }))
    })
    .await
}

#[derive(Deserialize)]
struct DecideReq {
    /// `{perm: true|false}`.
    decisions: serde_json::Map<String, Value>,
    /// `consent` (the card on opening the app) or `settings` (its page).
    #[serde(default)]
    via: Option<String>,
}

/// The user's decisions, from the shell's own screens (the consent card, the app's page). Nothing else
/// grants: not the app's page, not an agent (it has no tool for it).
async fn app_decide(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<DecideReq>) -> Response {
    let h = hub.clone();
    let app = id.clone();
    let r = blocking(move || {
        let installed = h.apps.get(&app).ok_or_else(|| anyhow::anyhow!("no app `{app}`"))?;
        let via = match req.via.as_deref() {
            Some("settings") => "settings",
            _ => "consent",
        };
        let n = crate::broker::decide(&h.core, &installed.app, &req.decisions, via)?;
        tracing::info!(app = %app, decided = n, via, "permissions decided");
        h.services.refresh(&h, &app);
        Ok(json!({ "permissions": crate::broker::states(&h.core, &installed.app) }))
    })
    .await;
    if r.status().is_success() {
        hub.emit(Event::AppPermissions { app: id }, Source::Api);
    }
    r
}

/// Host.init: opens (and migrates) the app's database; returns its settings and kept state.
async fn app_init(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    blocking(move || {
        let settings = hub.apps.settings(&id)?;
        Ok(json!({ "settings": settings, "state": hub.apps.kept_state(&id) }))
    })
    .await
}

#[derive(Deserialize)]
struct QueryReq {
    name: String,
    #[serde(default)]
    params: serde_json::Map<String, Value>,
}

async fn app_query(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<QueryReq>) -> Response {
    blocking(move || hub.apps.query(&id, &req.name, &req.params)).await
}

#[derive(Deserialize)]
struct StepReq {
    action: String,
    step: usize,
    #[serde(default)]
    params: serde_json::Map<String, Value>,
}

async fn app_step(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<StepReq>) -> Response {
    let h = hub.clone();
    let app = id.clone();
    match tokio::task::spawn_blocking(move || h.apps.step(&app, &req.action, req.step, &req.params)).await {
        Ok(Ok(exec)) => {
            if !exec.changed_tables.is_empty() {
                hub.scheduler.changed(&id, &exec.changed_tables);
                hub.emit(Event::AppChanged { app: id, tables: exec.changed_tables.clone() }, Source::Api);
            }
            Json(json!({ "changedTables": exec.changed_tables, "rowsAffected": exec.rows_affected })).into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn app_settings(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    blocking(move || hub.apps.settings(&id)).await
}

#[derive(Deserialize)]
struct SettingsReq {
    values: serde_json::Map<String, Value>,
}

/// Changes the app's settings (its page in Manage apps, or the agent's `set_app_settings`). The open app
/// re-reads them (`app_changed` on `settings`), and its services refetch if their URLs bind them.
async fn app_set_settings(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<SettingsReq>) -> Response {
    let h = hub.clone();
    let app = id.clone();
    let r = blocking(move || h.apps.set_settings(&app, &req.values)).await;
    if r.status().is_success() {
        hub.scheduler.changed(&id, &["settings".to_owned()]);
        hub.emit(Event::AppChanged { app: id, tables: vec!["settings".into()] }, Source::Api);
    }
    r
}

/// Starts the app over: its data, settings and kept state are erased (the database stays in its snapshots).
async fn app_erase(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    let h = hub.clone();
    let app = id.clone();
    let r = blocking(move || h.apps.erase_data(&app)).await;
    // The tables come back empty: an open app re-runs its queries.
    if r.status().is_success()
        && let Some(installed) = hub.apps.get(&id)
    {
        let mut tables: Vec<String> = installed.app.tables.iter().map(|t| t.name.clone()).collect();
        tables.push("settings".into());
        hub.emit(Event::AppChanged { app: id, tables }, Source::Api);
    }
    r
}

#[derive(Deserialize)]
struct StateReq {
    values: serde_json::Map<String, Value>,
}

async fn app_state(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<StateReq>) -> Response {
    blocking(move || {
        hub.apps.save_state(&id, &req.values).map(|()| {
            hub.glance.dirty(&id);
            json!({ "saved": true })
        })
    })
    .await
}

#[derive(Deserialize, Default)]
struct StopReq {
    /// The build to stop (default: whichever is running).
    build: Option<u64>,
}

/// Stops the running build: nothing is installed (`build {stage: failed, stopped: true}`).
async fn build_stop(State(hub): State<Shared>, body: Option<Json<StopReq>>) -> Json<Value> {
    let req = body.map(|Json(r)| r).unwrap_or_default();
    Json(json!({ "stopped": hub.builds.stop(req.build) }))
}

#[derive(Deserialize)]
struct CreateReq {
    brief: String,
    /// Claude model alias for the build (default sonnet).
    model: Option<String>,
    /// A bigger app (ADR-0041): longer turns, more effort.
    #[serde(default)]
    big: bool,
}

/// Starts building an app from `brief` in the agent box. Progress arrives as `build` events on /ws.
async fn create_app(State(hub): State<Shared>, Json(req): Json<CreateReq>) -> Response {
    let brief = req.brief.trim().to_owned();
    let big = req.big;
    if brief.is_empty() || brief.len() > 2000 {
        return error(StatusCode::BAD_REQUEST, "the brief must be 1–2000 characters".into());
    }
    let model = match build_model(&hub, req.model) {
        Ok(m) => m,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let Some(id) = hub.builds.start() else {
        return error(StatusCode::CONFLICT, "another app is being built".into());
    };
    let h = hub.clone();
    tokio::spawn(async move {
        crate::create::run(h.clone(), id, brief, model, big).await;
        h.builds.finish();
    });
    Json(json!({ "build": id })).into_response()
}

// ---- agents, memory (docs/specs/agents.md §6) -----------------------------------------------------

/// An agent as the shell sees it: its profile, what it's doing now, and in which chat; the agent whose
/// message it's answering (`by`), and the newest message from another agent the user hasn't seen
/// (`mail: {from, kind, text}`; ADR-0012).
fn agent_view(hub: &Hub, a: &crate::core::Agent) -> Value {
    let (state, chat) = hub.agent_state(&a.id);
    let mut v = serde_json::to_value(a).unwrap_or_default();
    v["state"] = json!(state);
    v["unread"] = json!(hub.core.unread(&a.id).unwrap_or(0));
    if let Some(c) = chat {
        if let Some(w) = hub.mail.woken(&c) {
            v["by"] = json!(w.by);
        }
        v["chat"] = json!(c);
    }
    if let Some(mail) = crate::mail::latest(hub, &a.id) {
        v["mail"] = mail;
    }
    // It works on a linked computer: which, and whether it's there now.
    if let Some(key) = a.computer() {
        v["computer"] = json!({ "name": crate::remote::computer_name(hub, key), "connected": hub.link.connected(key) });
    }
    v
}

async fn agents_list(State(hub): State<Shared>) -> Response {
    blocking(move || {
        let agents: Vec<Value> = hub.core.agents()?.iter().map(|a| agent_view(&hub, a)).collect();
        Ok(json!({
            "agents": agents,
            "focus": hub.core.focus().id,
            "max": crate::core::MAX_AGENTS,
            "paused": hub.core.agents_paused(),
        }))
    })
    .await
}

#[derive(Deserialize)]
struct PausedReq {
    paused: bool,
}

/// Settings › Agents: the user pauses agents waking each other (ADR-0012). Messages are still delivered,
/// as badges; turns already queued don't start.
async fn agents_paused(State(hub): State<Shared>, Json(req): Json<PausedReq>) -> Response {
    match hub.core.set_agents_paused(req.paused) {
        Ok(()) => {
            hub.emit(Event::AgentsChanged, Source::Api);
            Json(json!({ "paused": req.paused })).into_response()
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct MailQuery {
    agent: Option<String>,
    limit: Option<i64>,
}

/// Messages between agents, newest first (ADR-0012): to or from `?agent=`, or everyone's.
async fn mail_list(State(hub): State<Shared>, Query(q): Query<MailQuery>) -> Response {
    blocking(move || {
        let limit = q.limit.unwrap_or(50).clamp(1, 500);
        let mail = hub.core.with(|db| crate::mail::list(db, q.agent.as_deref(), limit))?;
        Ok(json!({ "mail": mail, "paused": hub.core.agents_paused() }))
    })
    .await
}

/// A new agent. It becomes the focus, so home lands on it.
async fn agents_new(State(hub): State<Shared>, Json(agent): Json<crate::core::Agent>) -> Response {
    let h = hub.clone();
    let made = tokio::task::spawn_blocking(move || {
        let a = h.core.create_agent(agent)?;
        h.core.set_focus(&a.id)?;
        crate::jail::Jail::for_agent(&a.id)?.prepare()?;
        anyhow::Ok(a)
    })
    .await;
    match made {
        Ok(Ok(a)) => {
            hub.emit(Event::AgentsChanged, Source::Api);
            Json(json!({ "agent": agent_view(&hub, &a) })).into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn agent_get(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    match hub.core.agent(&id) {
        Some(a) => Json(json!({ "agent": agent_view(&hub, &a) })).into_response(),
        None => error(StatusCode::NOT_FOUND, format!("no agent `{id}`")),
    }
}

async fn agent_put(State(hub): State<Shared>, Path(id): Path<String>, Json(agent): Json<crate::core::Agent>) -> Response {
    match hub.core.update_agent(&id, agent) {
        Ok(a) => {
            // Its warm processes carry the old persona in their prompt.
            hub.sessions.clear(&hub, Some(&id)).await;
            hub.emit(Event::AgentsChanged, Source::Api);
            Json(json!({ "agent": agent_view(&hub, &a) })).into_response()
        }
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

/// Deletes an agent: its chats, its own memories and its files (workspace, chat stores).
async fn agent_delete(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    if hub.agent_state(&id).0 != "idle" {
        return error(StatusCode::CONFLICT, "it's busy right now; try again when it's done".into());
    }
    if let Err(e) = hub.core.delete_agent(&id) {
        return error(StatusCode::BAD_REQUEST, format!("{e:#}"));
    }
    hub.sessions.clear(&hub, Some(&id)).await;
    let removed = tokio::task::spawn_blocking(move || crate::jail::Jail::for_agent(&id)?.remove_files()).await;
    if let Ok(Err(e)) = removed {
        tracing::warn!(error = %format!("{e:#}"), "couldn't remove a deleted agent's files");
    }
    hub.emit(Event::AgentsChanged, Source::Api);
    Json(json!({ "deleted": true })).into_response()
}

#[derive(Deserialize)]
struct OrderReq {
    ids: Vec<String>,
}

async fn agents_order(State(hub): State<Shared>, Json(req): Json<OrderReq>) -> Response {
    match hub.core.order_agents(&req.ids) {
        Ok(()) => {
            hub.emit(Event::AgentsChanged, Source::Api);
            Json(json!({ "ordered": true })).into_response()
        }
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct AgentQuery {
    agent: Option<String>,
}

/// Memory: the shared facts, plus one agent's own with `?agent=`.
async fn memories_list(State(hub): State<Shared>, Query(q): Query<AgentQuery>) -> Response {
    blocking(move || hub.core.memories(q.agent.as_deref()).map(|m| json!({ "memories": m }))).await
}

/// The user forgets a fact (any scope). Processes started before keep it in their prompt until
/// they restart (crate::session).
async fn memory_delete(State(hub): State<Shared>, Path(id): Path<i64>) -> Response {
    blocking(move || hub.core.forget(None, id).map(|f| json!({ "forgotten": f }))).await
}

// ---- onboarding, Wi-Fi, accounts, chats (docs/specs/experience.md §4) -----------------------------

async fn onboarding_get(State(hub): State<Shared>) -> Json<Value> {
    Json(json!({ "step": hub.core.onboarding() }))
}

#[derive(Deserialize)]
struct StepPut {
    step: String,
}

async fn onboarding_put(State(hub): State<Shared>, Json(req): Json<StepPut>) -> Response {
    match hub.core.set_onboarding(&req.step) {
        Ok(()) => Json(json!({ "step": req.step })).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

async fn wifi_get() -> Response {
    let (status, networks) = tokio::join!(crate::wifi::status(), crate::wifi::networks());
    match status {
        Ok(status) => Json(json!({ "status": status, "networks": networks.unwrap_or_default() })).into_response(),
        Err(e) => error(StatusCode::NOT_IMPLEMENTED, format!("{e:#}")),
    }
}

async fn wifi_scan() -> Response {
    match crate::wifi::scan().await {
        Ok(()) => Json(json!({ "scanning": true })).into_response(),
        Err(e) => error(StatusCode::NOT_IMPLEMENTED, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct WifiConnect {
    ssid: String,
    security: String,
    password: Option<String>,
}

async fn wifi_connect(Json(req): Json<WifiConnect>) -> Response {
    match crate::wifi::connect(&req.ssid, &req.security, req.password.as_deref()).await {
        Ok(()) => Json(json!({ "connecting": req.ssid })).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

async fn accounts_get(State(hub): State<Shared>) -> Json<Value> {
    Json(json!({ "accounts": hub.accounts.status().await }))
}

#[derive(Deserialize, Default)]
struct LoginReq {
    /// `computer`: a linked computer signs Claude in (no LAN page: it opens the link and passes the code back).
    #[serde(default)]
    via: Option<String>,
}

async fn account_login(State(hub): State<Shared>, Path(brain): Path<String>, body: Option<Json<LoginReq>>) -> Response {
    let via_computer = body.and_then(|b| b.0.via).as_deref() == Some("computer");
    let r = match brain.as_str() {
        "codex" => hub.accounts.codex_login(hub.clone()).await,
        "claude" if via_computer => hub.accounts.claude_login(hub.clone(), None).await,
        "claude" => match crate::wifi::status().await.ok().and_then(|s| s.ip) {
            Some(ip) => hub.accounts.claude_login(hub.clone(), Some(&ip)).await,
            None => Err(anyhow::anyhow!("connect to Wi-Fi first: your phone opens the pairing page over it")),
        },
        other => Err(anyhow::anyhow!("unknown brain `{other}`")),
    };
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct CodeReq {
    token: String,
    code: String,
}

/// Claude's sign-in code, from a linked computer (never logged; it goes to the CLI's stdin).
async fn account_code(State(hub): State<Shared>, Json(req): Json<CodeReq>) -> Response {
    match hub.accounts.code_from_computer(&hub, &req.token, &req.code).await {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct DeviceChangeReq {
    changes: serde_json::Map<String, Value>,
}

/// The R1's own settings, changed by the user from a linked computer (ADR-0048; an agent's change goes through
/// crate::approvals instead): checked, then made (the host's through `device_set`).
async fn device_change(State(hub): State<Shared>, Json(req): Json<DeviceChangeReq>) -> Response {
    let checked = match crate::device_settings::check(&req.changes) {
        Ok(c) => c,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    match crate::device_settings::apply(&hub, checked).await {
        Ok(()) => Json(json!({ "device": hub.device_settings.all(&hub) })).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct PowerReq {
    /// `off` or `restart`.
    action: String,
}

/// IPowerManager's `reboot` and `shutdown` on Android 15 (android-15.0.0_r36: `FIRST_CALL_TRANSACTION + 49` and `+ 51`,
/// as its AIDL compiler numbers them). Another Android numbers them differently, so they're only sent on SDK 35.
const POWER_REBOOT: &str = "50";
const POWER_SHUTDOWN: &str = "52";
const POWER_SDK: &str = "35";

/// Turning the R1 off or restarting it (Settings › Power; never a linked computer's or an agent's). r1d calls Android's
/// power service itself (`service call power`, as root, which the service lets reboot): `svc power` can't run in r1d's
/// SELinux domain (it starts `app_process`), and image v0.2.14 lets the domain find `power_service`. Still here a few
/// seconds later, it didn't happen: an error says why.
async fn power(Json(req): Json<PowerReq>) -> Response {
    // `reboot(confirm, reason, wait)` / `shutdown(confirm, reason, wait)`: no dialog, and don't wait for it.
    let (code, reason) = match req.action.as_str() {
        "off" => (POWER_SHUTDOWN, "userrequested"),
        "restart" => (POWER_REBOOT, "r1os"),
        _ => return error(StatusCode::BAD_REQUEST, "action is off or restart".into()),
    };
    if !cfg!(target_os = "android") {
        return error(StatusCode::NOT_IMPLEMENTED, "only the R1 turns off".into());
    }
    let sdk = crate::tool::run("getprop", &["ro.build.version.sdk"], crate::tool::DEADLINE).await.map(|o| o.stdout.trim().to_owned()).unwrap_or_default();
    if sdk != POWER_SDK {
        return error(StatusCode::NOT_IMPLEMENTED, format!("r1d knows Android 15's power service, not SDK {sdk}'s"));
    }
    tracing::info!(action = %req.action, "power: asked");
    let out = crate::tool::run("service", &["call", "power", code, "i32", "0", "s16", reason, "i32", "0"], crate::tool::DEADLINE).await;
    tracing::info!(out = ?out.as_ref().map(|o| (o.ok, o.stdout.trim().to_owned(), o.stderr.trim().to_owned())), "power: called");
    // Going down takes a moment; still here after it, it didn't happen.
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    let why = match out {
        Ok(o) => format!("{} {}", o.stdout.trim(), o.stderr.trim()).trim().to_owned(),
        Err(e) => e.to_string(),
    };
    error(StatusCode::INTERNAL_SERVER_ERROR, format!("Android didn't {}: {why}", if code == POWER_REBOOT { "restart" } else { "turn off" }))
}

async fn account_logout(State(hub): State<Shared>, Path(brain): Path<String>) -> Response {
    match hub.accounts.logout(hub.clone(), &brain).await {
        Ok(()) => Json(json!({ "signed_out": brain })).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct QrQuery {
    text: String,
}

// ---- the boot (crate::boot, T-029, T-101) ------------------------------------------------------------------------

/// Verbose boot, and the boot animations kept.
async fn boot_get(State(hub): State<Shared>) -> Json<Value> {
    Json(crate::boot::state(&hub.cfg.data))
}

/// `{verbose}`: verbose boot on or off, from the next boot.
async fn boot_put(State(hub): State<Shared>, Json(req): Json<Value>) -> Response {
    let Some(on) = req["verbose"].as_bool() else { return error(StatusCode::BAD_REQUEST, "verbose: true or false".into()) };
    match crate::boot::set_verbose(on) {
        Ok(()) => Json(crate::boot::state(&hub.cfg.data)).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct BootanimQuery {
    #[serde(default)]
    default: Option<u8>,
}

/// The host's drawing of the boot animation (a zip): yours, or `?default=1` r1os's own for the image.
async fn bootanim_save(State(hub): State<Shared>, Query(q): Query<BootanimQuery>, body: axum::body::Bytes) -> Response {
    let data = hub.cfg.data.clone();
    match tokio::task::spawn_blocking(move || crate::boot::save(&data, &body, q.default == Some(1))).await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Asks the host to draw the boot animation now (`{default: true}`: r1os's own cast, for the image).
async fn harness_bootanim(State(hub): State<Shared>, Json(req): Json<Value>) -> Json<Value> {
    hub.emit(Event::Bootanim { default: req["default"].as_bool().unwrap_or(false) }, Source::Sim);
    Json(json!({ "asked": true }))
}

// ---- sharing by QR (crate::share, ADR-0056, T-071) -----------------------------------------------------------

/// Shares one photo, recording or app on the Wi-Fi while its code shows: `{url, name, expires_s}`.
async fn share_start(State(hub): State<Shared>, Json(req): Json<Value>) -> Response {
    match hub.shares.start(&hub, &req).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

/// The code's screen closed: the share goes, and nothing listens.
async fn share_stop(State(hub): State<Shared>) -> Json<Value> {
    hub.shares.stop();
    Json(json!({ "stopped": true }))
}

/// A camera frame: `{found: false}`, or what its QR is (`kind`: link, app, file or text; or `error`).
async fn scan(State(hub): State<Shared>, body: axum::body::Bytes) -> Response {
    let h = hub.clone();
    let read = tokio::task::spawn_blocking(move || -> anyhow::Result<Value> {
        let Some(text) = crate::link::decode_qr(&body)? else { return Ok(json!({ "found": false })) };
        let mut v = h.shares.scan(&text).unwrap_or_else(|e| json!({ "error": format!("{e:#}") }));
        v["found"] = json!(true);
        Ok(v)
    })
    .await;
    match read {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// You said yes to an app a code brought: installed, as any app (`{token}` from /api/scan).
async fn scan_install(State(hub): State<Shared>, Json(req): Json<Value>) -> Response {
    let source = match hub.shares.take(req["token"].as_str().unwrap_or_default()) {
        Ok(s) => s,
        Err(e) => return error(StatusCode::NOT_FOUND, format!("{e:#}")),
    };
    let h = hub.clone();
    match tokio::task::spawn_blocking(move || h.apps.install_recorded(&source, "installed from a QR code")).await {
        Ok(Ok((summary, warnings))) => {
            hub.emit(Event::AppInstalled { app: summary.id.clone() }, Source::Api);
            Json(json!({ "ok": true, "app": summary, "warnings": warnings })).into_response()
        }
        Ok(Err(rejected)) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "ok": false, "diagnostics": rejected.diagnostics }))).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn qr(Query(q): Query<QrQuery>) -> Response {
    match crate::accounts::qr_svg(&q.text) {
        Ok(svg) => ([(header::CONTENT_TYPE, "image/svg+xml")], svg).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

async fn app_history(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    blocking(move || {
        hub.apps.get(&id).ok_or_else(|| anyhow::anyhow!("no app `{id}`"))?;
        let versions: Vec<Value> = hub
            .apps
            .versions(&id)
            .into_iter()
            .map(|v| json!({ "at": v.at, "kind": v.kind, "request": v.request, "said": v.said, "deleted": v.deleted }))
            .collect();
        Ok(json!({ "versions": versions }))
    })
    .await
}

#[derive(Deserialize)]
struct EditReq {
    request: String,
    /// Claude model alias for the build (default sonnet).
    model: Option<String>,
}

/// Starts an edit build of an installed app (progress as `build` events with `kind: "edit"`).
async fn app_edit(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<EditReq>) -> Response {
    let request = req.request.trim().to_owned();
    if request.is_empty() || request.len() > 2000 {
        return error(StatusCode::BAD_REQUEST, "say what to change (up to 2000 characters)".into());
    }
    start_edit(&hub, id, request, req.model)
}

/// Starts an edit build of an installed app (its page in Manage apps: a change, or a fix).
fn start_edit(hub: &Shared, id: String, request: String, model: Option<String>) -> Response {
    if hub.apps.get(&id).is_none() {
        return error(StatusCode::NOT_FOUND, format!("no app `{id}`"));
    }
    let model = match build_model(hub, model) {
        Ok(m) => m,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };
    let Some(build) = hub.builds.start() else {
        return error(StatusCode::CONFLICT, "an app is being built right now".into());
    };
    let h = hub.clone();
    tokio::spawn(async move {
        crate::create::build(h.clone(), build, crate::create::Task::Edit { app: id, request }, model, false).await;
        h.builds.finish();
    });
    (StatusCode::ACCEPTED, Json(json!({ "build": build }))).into_response()
}

/// Puts the app back to its version before the last change, with the data that change deleted.
async fn app_undo(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    let h = hub.clone();
    let app = id.clone();
    match tokio::task::spawn_blocking(move || h.apps.undo(&app)).await {
        Ok(Ok((summary, undone, migration))) => {
            hub.emit(Event::AppInstalled { app: id }, Source::Api);
            Json(json!({
                "app": summary,
                "undone": { "at": undone.at, "kind": undone.kind, "request": undone.request, "said": undone.said },
                "restored": migration.map(|m| m.restored).unwrap_or_default(),
            }))
            .into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn trash_list(State(hub): State<Shared>) -> Json<Value> {
    let apps: Vec<Value> = hub.apps.trashed().into_iter().map(|(id, at, name)| json!({ "id": id, "name": name, "removed_at": at })).collect();
    Json(json!({ "apps": apps, "keep_days": crate::apps::TRASH_KEEP.as_secs() / 86400 }))
}

/// A backup's list of files, core.db snapshotted for it (crate::backup).
async fn backup_list(State(hub): State<Shared>) -> Response {
    blocking(move || crate::backup::manifest(&hub)).await
}

#[derive(Deserialize)]
struct BackupPart {
    path: String,
    #[serde(default)]
    offset: u64,
    len: Option<u64>,
}

/// A part of one of a backup's files, as bytes.
async fn backup_file(State(hub): State<Shared>, Query(q): Query<BackupPart>) -> Response {
    let data = hub.cfg.data.clone();
    match tokio::task::spawn_blocking(move || crate::backup::read(&data, &q.path, q.offset, q.len.unwrap_or(crate::backup::PART))).await {
        Ok(Ok(bytes)) => ([(axum::http::header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Brings an app back from the trash.
async fn app_restore(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    let h = hub.clone();
    let app = id.clone();
    match tokio::task::spawn_blocking(move || h.apps.restore_trashed(&app)).await {
        Ok(Ok(summary)) => {
            hub.emit(Event::AppInstalled { app: id }, Source::Api);
            Json(json!({ "app": summary })).into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct ChatsQuery {
    agent: Option<String>,
    /// user (default) · thread · all
    kind: Option<String>,
}

/// Chats, newest first: one agent's with `?agent=`, or everyone's. Conversations with the user by
/// default; `?kind=thread` lists an agent's threads with other agents (ADR-0012), `all` both.
async fn chats_list(State(hub): State<Shared>, Query(q): Query<ChatsQuery>) -> Response {
    blocking(move || {
        let kind = q.kind.as_deref().unwrap_or("user");
        anyhow::ensure!(["user", "thread", "all"].contains(&kind), "kind is user, thread or all");
        hub.core.chats(q.agent.as_deref(), kind).map(|c| json!({ "chats": c }))
    })
    .await
}

#[derive(Deserialize, Default)]
struct NewChat {
    /// The agent to talk to (default: the focused one).
    agent: Option<String>,
    /// Override the agent's brain for this chat.
    brain: Option<String>,
    /// Override the permission mode it starts in (an agent on a computer; crate::core::MODES).
    mode: Option<String>,
}

async fn chats_new(State(hub): State<Shared>, body: Option<Json<NewChat>>) -> Response {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    blocking(move || {
        let agent = req.agent.and_then(|a| hub.core.agent(&a)).unwrap_or_else(|| hub.core.focus());
        let brain = req.brain.unwrap_or_else(|| agent.brain.clone());
        anyhow::ensure!(["claude", "codex"].contains(&brain.as_str()), "unknown brain `{brain}`");
        let mut chat = hub.core.new_chat(&crate::core::random_hex(8), &agent.id, &brain)?;
        // An agent on a computer: the chat keeps the mode it starts in (its agent's, unless asked otherwise).
        if agent.computer().is_some() {
            let mode = req.mode.unwrap_or_else(|| agent.mode.clone());
            hub.core.set_chat_mode(&chat.id, &mode)?;
            chat.mode = Some(mode);
        }
        Ok(json!({ "chat": chat }))
    })
    .await
}

#[derive(Deserialize, Default)]
struct WarmReq {
    chat: Option<String>,
    /// For a new chat: whose spare to start (default: the focused agent).
    agent: Option<String>,
}

/// Starts (or keeps) a warm agent process for `chat`, or a spare for a new chat of `agent`
/// (crate::session). The shell calls it when you start talking, so the CLI's start overlaps your
/// speech.
async fn chats_warm(State(hub): State<Shared>, body: Option<Json<WarmReq>>) -> Response {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let chat = req.chat.filter(|c| !c.is_empty());
    let h = hub.clone();
    tokio::spawn(async move {
        let agent = req.agent.unwrap_or_else(|| h.core.focus().id);
        if let Err(e) = h.sessions.warm(&h, &agent, chat.as_deref()).await {
            tracing::info!(chat = ?chat, error = %format!("{e:#}"), "couldn't warm a session");
        }
    });
    (StatusCode::ACCEPTED, Json(json!({ "accepted": true }))).into_response()
}

async fn chat_get(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    blocking(move || {
        let chat = hub.core.chat(&id)?.ok_or_else(|| anyhow::anyhow!("no chat `{id}`"))?;
        let busy = hub.chat_busy.lock().unwrap().contains(&id);
        // A build an agent started here that's still running: the chat pins its card at the bottom.
        let building = hub.builds.of_chat(&id).map(|(build, app)| json!({ "build": build, "app": app }));
        Ok(json!({ "chat": chat, "messages": hub.core.messages(&id)?, "busy": busy, "building": building }))
    })
    .await
}

/// The user is looking at the chat: its replies are seen (`unread` event with the agent's new total), and
/// so are the messages between agents it shows, in the other agents' threads too (ADR-0012).
/// The agents' skills (crate::skills, T-086): `{skills: [{name, description, from, on}], agents: [{id, name, brain, off}]}`.
async fn skills_get(State(hub): State<Shared>) -> Response {
    blocking(move || crate::skills::state(&hub)).await
}

#[derive(Deserialize)]
struct SkillsAdd {
    source: String,
    #[serde(default)]
    skills: Vec<String>,
}

/// Adds skills for every agent: `{added: [names]}`, or `{choose: [{name, description}]}` when the source holds
/// several and none were named.
async fn skills_add(State(hub): State<Shared>, Json(req): Json<SkillsAdd>) -> Response {
    match crate::skills::add(&hub, &req.source, &req.skills).await {
        Ok(a) => Json(json!(a)).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct SkillsOn {
    /// An agent's id; none for everyone.
    agent: Option<String>,
    skill: String,
    on: bool,
}

/// A skill on or off, for everyone or one agent; the agents' warm processes restart with it.
async fn skills_on(State(hub): State<Shared>, Json(req): Json<SkillsOn>) -> Response {
    let agent = req.agent.clone().unwrap_or_else(|| crate::skills::ALL.to_owned());
    if agent != crate::skills::ALL && hub.core.agent(&agent).is_none() {
        return error(StatusCode::NOT_FOUND, format!("no agent `{agent}`"));
    }
    if let Err(e) = hub.core.with(|db| crate::skills::set_on(db, &agent, &req.skill, req.on)) {
        return error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"));
    }
    hub.sessions.clear(&hub, req.agent.as_deref()).await;
    hub.emit(Event::ConnectionsChanged, Source::Api);
    skills_get(State(hub)).await
}

/// Removes an added skill (the claude.ai account's can only be turned off).
async fn skills_remove(State(hub): State<Shared>, Path(name): Path<String>) -> Response {
    match crate::skills::remove(&hub, &name).await {
        Ok(()) => skills_get(State(hub)).await,
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

/// Latency marks (crate::perf, `r1h perf`): `{stats: {name: {n, last, p50, p90, max}}, recent}`.
async fn harness_perf(State(hub): State<Shared>) -> Json<Value> {
    Json(hub.perf.summary())
}

async fn harness_perf_reset(State(hub): State<Shared>) -> Json<Value> {
    hub.perf.reset();
    Json(json!({ "ok": true }))
}

/// Stops the chat's running turn (crate::stops): `{stopped}`, false when none was running. Its approval cards and its
/// ask in the R1's browser go with it.
async fn chat_stop(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    let stopped = hub.stops.stop(&id);
    if stopped {
        tracing::info!(chat = %id, "stopping a turn");
        hub.approvals.cancel_chat(&hub, &id);
        crate::browser::stopped(&hub, &id);
        crate::appwin::stopped(&hub, &id);
    }
    Json(json!({ "stopped": stopped })).into_response()
}

async fn chat_seen(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    let chat = match hub.core.chat(&id) {
        Ok(Some(c)) => c,
        Ok(None) => return error(StatusCode::NOT_FOUND, format!("no chat `{id}`")),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    };
    if chat.unread > 0 {
        if let Err(e) = hub.core.mark_seen(&id) {
            return error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"));
        }
        hub.unread_changed(&chat.agent, &id);
    }
    match hub.core.seen_mail_from(&id) {
        Ok(threads) => {
            for (agent, thread) in threads {
                hub.unread_changed(&agent, &thread);
            }
        }
        Err(e) => tracing::warn!(chat = %id, error = %format!("{e:#}"), "couldn't mark agents' messages seen"),
    }
    Json(json!({ "seen": true })).into_response()
}

async fn chat_delete(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    if hub.chat_busy.lock().unwrap().contains(&id) {
        return error(StatusCode::CONFLICT, "still answering in that chat".into());
    }
    hub.sessions.drop_chat(&hub, &id).await;
    blocking(move || {
        let agent = hub.core.chat(&id)?.map(|c| c.agent);
        hub.core.delete_chat(&id)?;
        // Its unread replies went with it: the agent's badge drops.
        if let Some(a) = agent {
            hub.unread_changed(&a, &id);
        }
        Ok(json!({ "deleted": true }))
    })
    .await
}

#[derive(Deserialize)]
struct SendReq {
    text: String,
    /// The app on screen when the user spoke: the agent is told (not stored in the transcript).
    app: Option<String>,
    /// The user spoke over an Android app (ADR-0027): r1d looks up which, and the agent is told.
    #[serde(default)]
    android: bool,
    /// Photos sent with it (media ids, ADR-0031).
    #[serde(default)]
    images: Vec<String>,
}

/// The Android app in front (its package), from `dumpsys window`'s focused app; None for r1os itself.
async fn focused_android_app() -> Option<String> {
    if !cfg!(target_os = "android") {
        return None;
    }
    let out = crate::tool::run("dumpsys", &["window", "displays"], crate::tool::DEADLINE).await.ok()?;
    let line = out.stdout.lines().find(|l| l.contains("mFocusedApp="))?;
    let package = line.split_whitespace().find(|w| w.contains('/'))?.split('/').next()?.to_owned();
    (package != "dev.r1os.host").then_some(package)
}

/// The display went on or off (the shell's screen broadcasts): background work waits while it's off.
async fn display_set(State(hub): State<Shared>, Json(req): Json<OnReq>) -> Json<Value> {
    hub.services.set_display(req.on);
    // Idle warm processes are let go sooner with the display off: the reaper looks again (T-035).
    if !req.on {
        hub.sessions.nudge();
    }
    // The PIN lock (ADR-0016): locked as the display goes off when a PIN is set, or once it's been off longer than the
    // grace ("lock after"), which waking checks.
    let now = crate::alarms::clock().now_ms();
    if req.on {
        hub.pin.display_on(&hub, now);
    } else {
        hub.pin.display_off(&hub, now);
    }
    // Linked computers: dialled while the screen's on, let go after it's off (ADR-0048, Q40).
    crate::link::Link::display(&hub, req.on);
    // The glance screen's cards: "5 min ago" and "today" moved on while it was off (ADR-0042).
    if req.on {
        hub.glance.all();
    }
    Json(json!({ "on": req.on }))
}

// ---- photos and recordings (crate::media, ADR-0031) ---------------------------------------------------

#[derive(Deserialize)]
struct MediaQuery {
    kind: Option<String>,
    limit: Option<i64>,
    /// Where a posted photo came from (`camera`, `chat`, `look`).
    source: Option<String>,
}

async fn media_list(State(hub): State<Shared>, Query(q): Query<MediaQuery>) -> Response {
    match hub.core.with(|db| crate::media::list(db, q.kind.as_deref(), q.limit.unwrap_or(100))) {
        Ok(items) => Json(json!({ "media": items })).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// A photo from the host's camera: the JPEG is the body.
async fn media_add(State(hub): State<Shared>, Query(q): Query<MediaQuery>, body: axum::body::Bytes) -> Response {
    let source = q.source.unwrap_or_else(|| "camera".into());
    let data = hub.cfg.data.clone();
    let h = hub.clone();
    match tokio::task::spawn_blocking(move || h.core.with(|db| crate::media::add_photo(db, &data, &body, &source))).await {
        Ok(Ok(item)) => {
            hub.emit(Event::MediaChanged { kind: "photo".into() }, Source::Api);
            Json(json!({ "media": item })).into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn media_get(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    match hub.core.with(|db| crate::media::get(db, &id)) {
        Ok(Some(item)) => Json(json!({ "media": item })).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, format!("no media `{id}`")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct LabelReq {
    label: String,
}

async fn media_label(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<LabelReq>) -> Response {
    match hub.core.with(|db| crate::media::set_label(db, &id, &req.label)) {
        Ok(true) => media_get(State(hub), Path(id)).await,
        Ok(false) => error(StatusCode::NOT_FOUND, format!("no media `{id}`")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn media_remove(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    let data = hub.cfg.data.clone();
    let kind = hub.core.with(|db| crate::media::get(db, &id)).ok().flatten().map(|i| i.kind);
    match hub.core.with(|db| crate::media::remove(db, &data, &id)) {
        Ok(true) => {
            hub.emit(Event::MediaChanged { kind: kind.unwrap_or_default() }, Source::Api);
            Json(json!({ "removed": id })).into_response()
        }
        Ok(false) => error(StatusCode::NOT_FOUND, format!("no media `{id}`")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

/// The file itself (ranges work, so a recording can be played from anywhere in it).
async fn media_file(State(hub): State<Shared>, Path(id): Path<String>, req: axum::extract::Request) -> Response {
    let Ok(Some(item)) = hub.core.with(|db| crate::media::get(db, &id)) else {
        return error(StatusCode::NOT_FOUND, format!("no media `{id}`"));
    };
    let path = crate::media::file(&hub.cfg.data, &item);
    match tower::ServiceExt::oneshot(ServeFile::new(path), req).await {
        Ok(r) => r.into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct PeaksQuery {
    per_s: Option<u32>,
}

/// A recording's loudness over time (`?per_s=`, default 20): `{per_s, peaks}`.
async fn media_peaks(State(hub): State<Shared>, Path(id): Path<String>, Query(q): Query<PeaksQuery>) -> Response {
    let Ok(Some(item)) = hub.core.with(|db| crate::media::get(db, &id)) else {
        return error(StatusCode::NOT_FOUND, format!("no media `{id}`"));
    };
    let per_s = q.per_s.unwrap_or(20).clamp(1, 100);
    let data = hub.cfg.data.clone();
    match tokio::task::spawn_blocking(move || crate::media::peaks(&data, &item, per_s)).await {
        Ok(Ok(peaks)) => Json(json!({ "per_s": per_s, "peaks": peaks })).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ---- the Journal (crate::journal, ADR-0033) ----------------------------------------------------------

/// The Journal's screen opened: a hold records a voice note (crate::notes, ADR-0037), for 15 minutes or
/// until it closes.
async fn journal_listen(State(hub): State<Shared>) -> Json<Value> {
    // The screen opened last owns push-to-talk: a Translator lease left behind (a restarted host app never
    // ended it) would otherwise take the hold.
    if hub.translator.active() {
        hub.translator.stop(&hub);
    }
    hub.notes.start();
    Json(json!({ "ok": true }))
}

/// The Journal's screen closed: a hold talks to the agent again.
async fn journal_unlisten(State(hub): State<Shared>) -> Json<Value> {
    hub.notes.stop();
    Json(json!({ "ok": true }))
}

#[derive(Deserialize)]
struct JournalQuery {
    before: Option<i64>,
    limit: Option<i64>,
    q: Option<String>,
}

/// Entries, newest first (`before` pages back), each with its photo or recording; `q` searches.
async fn journal_list(State(hub): State<Shared>, Query(q): Query<JournalQuery>) -> Response {
    let limit = q.limit.unwrap_or(60);
    let found = hub.core.with(|db| -> rusqlite::Result<Vec<Value>> {
        let entries = match q.q.as_deref().filter(|s| !s.trim().is_empty()) {
            Some(query) => crate::journal::search(db, query, None, q.before, limit)?,
            None => crate::journal::list(db, q.before, limit)?,
        };
        Ok(entries
            .into_iter()
            .map(|e| {
                let media = e.media.as_deref().and_then(|m| crate::media::get(db, m).ok().flatten());
                let mut v = json!(e);
                v["media_item"] = json!(media);
                v
            })
            .collect())
    });
    match found {
        Ok(entries) => Json(json!({ "entries": entries })).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct JournalReq {
    #[serde(default)]
    text: String,
    media: Option<String>,
}

async fn journal_add(State(hub): State<Shared>, Json(req): Json<JournalReq>) -> Response {
    match hub.core.with(|db| crate::journal::add(db, &req.text, req.media.as_deref(), None, crate::media::now_ms())) {
        Ok(e) => {
            hub.emit(Event::JournalChanged, Source::Api);
            Json(json!({ "entry": e })).into_response()
        }
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

async fn journal_edit(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<JournalReq>) -> Response {
    match hub.core.with(|db| crate::journal::set_text(db, &id, &req.text)) {
        Ok(true) => {
            hub.emit(Event::JournalChanged, Source::Api);
            Json(json!({ "ok": true })).into_response()
        }
        Ok(false) => error(StatusCode::NOT_FOUND, format!("no entry `{id}`")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn journal_remove(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    match hub.core.with(|db| crate::journal::remove(db, &id)) {
        Ok(true) => {
            hub.emit(Event::JournalChanged, Source::Api);
            Json(json!({ "removed": id })).into_response()
        }
        Ok(false) => error(StatusCode::NOT_FOUND, format!("no entry `{id}`")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// What the Recorder is doing (crate::recorder).
async fn recorder_get(State(hub): State<Shared>) -> Json<Value> {
    let summaries = hub.core.recorder_summaries();
    Json(match hub.recorder.current() {
        Some(n) => json!({ "recording": { "id": n.id, "seconds": n.started.elapsed().as_secs() }, "summaries": summaries }),
        None => json!({ "recording": null, "summaries": summaries }),
    })
}

#[derive(Deserialize)]
struct RecorderPut {
    /// Title and sum up each tape when it stops (T-068).
    summaries: bool,
}

async fn recorder_put(State(hub): State<Shared>, Json(req): Json<RecorderPut>) -> Response {
    match hub.core.set_recorder_summaries(req.summaries) {
        Ok(()) => recorder_get(State(hub)).await.into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

#[derive(Deserialize, Default)]
struct MarkReq {
    /// A photo taken at this moment (its media id).
    photo: Option<String>,
}

/// Marks this moment of the recording (T-068).
async fn recorder_mark(State(hub): State<Shared>, body: Option<Json<MarkReq>>) -> Response {
    let photo = body.and_then(|b| b.0.photo).filter(|p| !p.is_empty());
    match crate::recorder::Recorder::mark(&hub, photo) {
        Ok(mark) => Json(json!({ "mark": mark })).into_response(),
        Err(e) => error(StatusCode::CONFLICT, format!("{e:#}")),
    }
}

#[derive(Deserialize, Default)]
struct RecordReq {
    #[serde(default)]
    label: String,
}

async fn recorder_start(State(hub): State<Shared>, body: Option<Json<RecordReq>>) -> Response {
    let label = body.map(|b| b.0.label).unwrap_or_default();
    match crate::recorder::Recorder::start(&hub, &label) {
        Ok(id) => Json(json!({ "id": id })).into_response(),
        Err(e) => error(StatusCode::CONFLICT, format!("{e:#}")),
    }
}

async fn recorder_stop(State(hub): State<Shared>) -> Response {
    match crate::recorder::Recorder::stop(&hub) {
        Some(id) => Json(json!({ "id": id })).into_response(),
        None => error(StatusCode::CONFLICT, "not recording".into()),
    }
}

// ---- the Translator (crate::translator, ADR-0034) -------------------------------------------------------

/// Its languages (with what each has on the device), and whether it's open.
async fn translator_get(State(hub): State<Shared>) -> Json<Value> {
    Json(hub.translator.status())
}

#[derive(Deserialize)]
struct TranslatorReq {
    mine: String,
    theirs: String,
}

/// The screen opened (or the languages changed): push-to-talk is the Translator's for a while, and its
/// ears start loading.
async fn translator_start(State(hub): State<Shared>, Json(req): Json<TranslatorReq>) -> Response {
    hub.notes.stop();
    match hub.translator.start(&hub, &req.mine, &req.theirs) {
        Ok(()) => Json(hub.translator.status()).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

/// The screen closed: push-to-talk is back, and the models are dropped.
async fn translator_stop(State(hub): State<Shared>) -> Json<Value> {
    hub.translator.stop(&hub);
    Json(json!({ "ok": true }))
}

#[derive(Deserialize)]
struct TranslateTextReq {
    text: String,
    from: String,
    to: String,
    #[serde(default)]
    speak: bool,
}

/// Typed text, translated (and spoken, with `speak`): `{text, via, speech?, ms}`.
async fn translator_text(State(hub): State<Shared>, Json(req): Json<TranslateTextReq>) -> Response {
    let h = hub.clone();
    match tokio::task::spawn_blocking(move || h.translator.text(&req.text, &req.from, &req.to, req.speak)).await {
        Ok(Ok(turn)) => Json(turn).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_GATEWAY, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// A turn's spoken translation (WAV), while it's among the last few.
async fn translator_speech(State(hub): State<Shared>, Path(id): Path<u64>) -> Response {
    match hub.translator.speech(id) {
        Some(wav) => ([(axum::http::header::CONTENT_TYPE, "audio/wav")], wav.as_ref().clone()).into_response(),
        None => error(StatusCode::NOT_FOUND, format!("no speech {id}")),
    }
}

#[derive(Deserialize)]
struct TranslateBenchReq {
    path: String,
    #[serde(default = "english")]
    mine: String,
    #[serde(default)]
    theirs: String,
    /// Whisper's threads (default: the Translator's).
    threads: Option<i32>,
    /// Into the open Translator instead, as if just said (its screen shows the turn).
    #[serde(default)]
    live: bool,
}

fn english() -> String {
    "en".into()
}

/// A 16 kHz WAV on the device through the whole Translator, timed by stage.
async fn harness_translate(State(hub): State<Shared>, Json(req): Json<TranslateBenchReq>) -> Response {
    if req.live {
        return match hub.translator.hear_file(&hub, std::path::Path::new(&req.path)) {
            Ok(()) => Json(json!({ "ok": true })).into_response(),
            Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        };
    }
    let h = hub.clone();
    match tokio::task::spawn_blocking(move || h.translator.bench(std::path::Path::new(&req.path), &req.mine, &req.theirs, req.threads)).await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ---- alarms and reminders (crate::alarms, ADR-0028) ---------------------------------------------------

/// Alarms and reminders, the R1's time zone, and now (ms).
async fn alarms_list(State(hub): State<Shared>) -> Response {
    // To-dos ticked a day ago go (T-069).
    let now = crate::alarms::clock().now_ms();
    let list = hub.core.with(|db| -> rusqlite::Result<_> {
        crate::alarms::clear_done(db, Some(now - 86_400_000))?;
        crate::alarms::list(db)
    });
    match list {
        Ok(list) => Json(json!({ "alarms": list, "zone": crate::alarms::zone(), "now": crate::alarms::clock().now_ms() })).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// `{"kind": "alarm", "time": "7:30", "days": ["mon"], "label": …}` or `{"kind": "reminder", "text": …, "at" | "in_minutes": …}`.
async fn alarms_add(State(hub): State<Shared>, Json(req): Json<Value>) -> Response {
    let made = hub.core.with(|db| match req["kind"].as_str() {
        Some("reminder") => crate::alarms::reminder_from(db, &req, None),
        _ => crate::alarms::alarm_from(db, &req, None),
    });
    match made {
        Ok(a) => {
            crate::alarms::changed(&hub);
            Json(json!({ "alarm": a })).into_response()
        }
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

async fn alarms_remove(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    match hub.core.with(|db| crate::alarms::remove(db, &id)) {
        Ok(true) => {
            crate::alarms::changed(&hub);
            Json(json!({ "removed": id })).into_response()
        }
        Ok(false) => error(StatusCode::NOT_FOUND, format!("no alarm `{id}`")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct OnReq {
    on: bool,
}

/// An alarm on or off (the Alarms screen).
async fn alarms_on(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<OnReq>) -> Response {
    let c = crate::alarms::clock();
    match hub.core.with(|db| crate::alarms::turn(db, &id, req.on, c.now_ms(), &|ms| c.offset_ms(ms))) {
        Ok(a) => {
            crate::alarms::changed(&hub);
            Json(json!({ "alarm": a })).into_response()
        }
        Err(e) => error(StatusCode::NOT_FOUND, format!("{e:#}")),
    }
}

/// The ringing alarm: a click on the alarm screen stops it, a double click snoozes it.
async fn alarms_stop(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    end_ring(&hub, &id, false)
}

async fn alarms_snooze(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    end_ring(&hub, &id, true)
}

fn end_ring(hub: &Hub, id: &str, snooze: bool) -> Response {
    match crate::alarms::end_ring(hub, id, snooze) {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => error(StatusCode::NOT_FOUND, format!("no alarm `{id}`")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct DoneReq {
    #[serde(default = "yes")]
    done: bool,
}

fn yes() -> bool {
    true
}

/// A to-do ticked off, or back on the list (T-069).
async fn alarms_done(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<DoneReq>) -> Response {
    match hub.core.with(|db| crate::alarms::tick(db, &id, req.done, crate::alarms::clock().now_ms())) {
        Ok(a) => {
            crate::alarms::changed(&hub);
            Json(json!({ "alarm": a })).into_response()
        }
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

/// The ticked to-dos off the list now.
async fn alarms_clear_done(State(hub): State<Shared>) -> Response {
    match hub.core.with(|db| crate::alarms::clear_done(db, None)) {
        Ok(n) => {
            crate::alarms::changed(&hub);
            Json(json!({ "cleared": n })).into_response()
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// A reminder that went off was seen (its note was dismissed): it's done.
async fn alarms_seen(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    match hub.core.with(|db| crate::alarms::seen(db, &id)) {
        Ok(_) => {
            hub.emit(Event::AlarmsChanged, Source::Api);
            Json(json!({ "ok": true })).into_response()
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ---- the PIN lock (crate::pin, ADR-0016) -------------------------------------------------------------
// Not on a linked computer's allowlist (crate::link): the PIN is set and dialled on the R1 itself.

#[derive(Deserialize)]
struct PinReq {
    #[serde(default)]
    pin: String,
    current: Option<String>,
}

/// `{set, locked, wait_s}`.
async fn lock_get(State(hub): State<Shared>) -> Json<Value> {
    Json(hub.pin.view(&hub.core, crate::alarms::clock().now_ms()))
}

/// What a refused code answers: 403 with the dial's words, and how long the next try waits.
fn refused(r: &crate::pin::Refused) -> Response {
    let wait_s = match r {
        crate::pin::Refused::Wrong { wait_s } | crate::pin::Refused::Wait { wait_s } => *wait_s,
        _ => 0,
    };
    let status = if matches!(r, crate::pin::Refused::Malformed | crate::pin::Refused::NotSet) { StatusCode::BAD_REQUEST } else { StatusCode::FORBIDDEN };
    (status, Json(json!({ "error": r.to_string(), "wait_s": wait_s }))).into_response()
}

/// The PIN dialled on the lock screen: unlocked, or why not. Hashing takes ~0.1 s, off the async threads.
async fn lock_unlock(State(hub): State<Shared>, Json(req): Json<PinReq>) -> Response {
    let h = hub.clone();
    match tokio::task::spawn_blocking(move || h.pin.unlock(&h, &req.pin, crate::alarms::clock().now_ms())).await {
        Ok(Ok(())) => Json(json!({ "locked": false })).into_response(),
        Ok(Err(r)) => refused(&r),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Whether a code is the PIN (Settings › Lock asks for the current one first); a wrong one counts.
async fn lock_check(State(hub): State<Shared>, Json(req): Json<PinReq>) -> Response {
    let h = hub.clone();
    match tokio::task::spawn_blocking(move || crate::pin::check(&h.core, &req.pin, crate::alarms::clock().now_ms())).await {
        Ok(Ok(())) => Json(json!({ "ok": true })).into_response(),
        Ok(Err(r)) => refused(&r),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// A new PIN `{pin, current?}` (the current one when one is set).
async fn lock_set(State(hub): State<Shared>, Json(req): Json<PinReq>) -> Response {
    let h = hub.clone();
    match tokio::task::spawn_blocking(move || h.pin.set(&h, &req.pin, req.current.as_deref(), crate::alarms::clock().now_ms())).await {
        Ok(Ok(())) => Json(json!({ "set": true })).into_response(),
        Ok(Err(r)) => refused(&r),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct GraceReq {
    grace_s: i64,
}

/// "Lock after" `{grace_s}`: how long the display may be off before it locks (Settings › Lock, on the R1 only).
async fn lock_grace(State(hub): State<Shared>, Json(req): Json<GraceReq>) -> Response {
    match hub.pin.set_grace(&hub, req.grace_s) {
        Ok(()) => Json(json!({ "grace_s": req.grace_s })).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

/// No PIN any more `{current}`.
async fn lock_off(State(hub): State<Shared>, Json(req): Json<PinReq>) -> Response {
    let h = hub.clone();
    let current = req.current.unwrap_or(req.pin);
    match tokio::task::spawn_blocking(move || h.pin.off(&h, &current, crate::alarms::clock().now_ms())).await {
        Ok(Ok(())) => Json(json!({ "set": false })).into_response(),
        Ok(Err(r)) => refused(&r),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ---- routines and what's up (crate::routines, crate::whatsup; R7) ---------------------------------------

/// Routines (by time of day), and now (ms).
async fn routines_list(State(hub): State<Shared>) -> Response {
    match hub.core.with(crate::routines::list) {
        Ok(list) => Json(json!({ "routines": list, "now": crate::alarms::clock().now_ms() })).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// `{agent?, time, days?, title?, prompt, show?}` makes one (the focused agent's unless `agent` says); with `id`, changes
/// that one.
async fn routines_add(State(hub): State<Shared>, Json(req): Json<Value>) -> Response {
    let agent = req["agent"].as_str().and_then(|a| hub.core.agent(a)).unwrap_or_else(|| hub.core.focus());
    let c = crate::alarms::clock();
    match hub.core.with(|db| crate::routines::from_args(db, &req, &agent.id, c.now_ms(), &|ms| c.offset_ms(ms))) {
        Ok(r) => {
            crate::routines::changed(&hub);
            Json(json!({ "routine": r })).into_response()
        }
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

async fn routines_remove(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    match hub.core.with(|db| crate::routines::remove(db, &id)) {
        Ok(true) => {
            crate::routines::changed(&hub);
            Json(json!({ "removed": id })).into_response()
        }
        Ok(false) => error(StatusCode::NOT_FOUND, format!("no routine `{id}`")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn routines_on(State(hub): State<Shared>, Path(id): Path<String>, Json(req): Json<OnReq>) -> Response {
    let c = crate::alarms::clock();
    match hub.core.with(|db| crate::routines::turn(db, &id, req.on, c.now_ms(), &|ms| c.offset_ms(ms))) {
        Ok(r) => {
            crate::routines::changed(&hub);
            Json(json!({ "routine": r })).into_response()
        }
        Err(e) => error(StatusCode::NOT_FOUND, format!("{e:#}")),
    }
}

/// Runs one now (to try it), as its schedule would.
async fn routines_run(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    match hub.core.with(|db| crate::routines::get(db, &id)) {
        Ok(Some(r)) => {
            tokio::spawn(crate::routines::run(hub.clone(), r));
            (StatusCode::ACCEPTED, Json(json!({ "running": id }))).into_response()
        }
        Ok(None) => error(StatusCode::NOT_FOUND, format!("no routine `{id}`")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// What an agent's prompt says it can and can't do (crate::abilities): for checking, no model call.
async fn abilities_get(State(hub): State<Shared>, Path(agent): Path<String>) -> Response {
    let found = hub.core.agents().unwrap_or_default().into_iter().find(|a| a.id == agent || a.name.eq_ignore_ascii_case(&agent));
    match found {
        Some(a) => Json(json!({ "agent": a.name, "line": crate::abilities::line(&hub, &a), "block": crate::abilities::block(&hub, &a) })).into_response(),
        None => error(StatusCode::NOT_FOUND, format!("no agent `{agent}`")),
    }
}

#[derive(Deserialize)]
struct BrowserResult {
    id: String,
    ok: bool,
    #[serde(default)]
    result: Value,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct BrowserCommand {
    text: String,
}

/// What you said with the R1's browser on screen: System One does it at once (`{handled: true, said}`), or it's the
/// agent's (`{handled: false}`).
async fn browser_command(State(hub): State<Shared>, Json(req): Json<BrowserCommand>) -> Response {
    match crate::browser::command(&hub, &req.text).await {
        Ok(Some(said)) => Json(json!({ "handled": true, "said": said })).into_response(),
        Ok(None) => Json(json!({ "handled": false })).into_response(),
        Err(e) => Json(json!({ "handled": true, "said": e })).into_response(),
    }
}

/// Any browser op, as an agent's tool would ask it (`{op, args}`), and what the host answered: for testing.
async fn harness_browser(State(hub): State<Shared>, Json(req): Json<Value>) -> Response {
    let op = req["op"].as_str().unwrap_or_default().to_owned();
    // With an agent and a chat, as that chat's turn: its chat shows the preview (T-137).
    let (agent, chat) = (req["agent"].as_str().unwrap_or_default(), req["chat"].as_str().unwrap_or_default());
    match crate::browser::call(&hub, agent, chat, &op, req["args"].clone()).await {
        Ok(v) => Json(json!({ "ok": true, "result": v })).into_response(),
        Err(e) => Json(json!({ "ok": false, "error": e })).into_response(),
    }
}

/// A test chat that presses nothing that adds to a cart or finishes anything (crate::appwin::dry_refusal):
/// `{chat, on}`. tools/eval-control.py marks its chats so.
async fn harness_dry_run(State(hub): State<Shared>, Json(req): Json<Value>) -> Json<Value> {
    let chat = req["chat"].as_str().unwrap_or_default();
    hub.app_window.set_dry(chat, req["on"].as_bool().unwrap_or(true));
    Json(json!({ "chat": chat, "dry": hub.app_window.is_dry(chat) }))
}

/// The host's answer to a browser request (crate::browser, ADR-0054).
async fn browser_result(State(hub): State<Shared>, Json(req): Json<BrowserResult>) -> Response {
    let r = if req.ok { Ok(req.result) } else { Err(req.error.unwrap_or_else(|| "it didn't work".into())) };
    Json(json!({ "delivered": hub.browser.answer(&req.id, r) })).into_response()
}

/// The host's answer to an app window request (crate::appwin, ADR-0061).
async fn app_window_result(State(hub): State<Shared>, Json(req): Json<BrowserResult>) -> Response {
    let r = if req.ok { Ok(req.result) } else { Err(req.error.unwrap_or_else(|| "it didn't work".into())) };
    Json(json!({ "delivered": hub.app_window.answer(&req.id, r) })).into_response()
}

/// Dev: an app window op straight to the host (`{op, args}`, and `agent`/`chat` to act as one's turn, so its chat shows
/// the preview), to try it without a model. No cards: those are the tools' (crate::mcp).
async fn harness_app_window(State(hub): State<Shared>, Json(req): Json<Value>) -> Response {
    let op = req["op"].as_str().unwrap_or_default().to_owned();
    let (agent, chat) = (req["agent"].as_str().unwrap_or_default(), req["chat"].as_str().unwrap_or_default());
    match crate::appwin::call(&hub, agent, chat, &op, req["args"].clone()).await {
        Ok(v) => Json(json!({ "ok": true, "result": v })).into_response(),
        Err(e) => Json(json!({ "ok": false, "error": e })).into_response(),
    }
}

/// A final step's card for the app window's screen as it is (T-113): what the user would be asked before an order,
/// to see on a real checkout. Nothing is pressed whatever the answer: `{allowed, order}`.
async fn harness_commit_card(State(hub): State<Shared>, Json(req): Json<Value>) -> Response {
    let (agent, chat) = (req["agent"].as_str().unwrap_or_default(), req["chat"].as_str().unwrap_or_default());
    let step = req["step"].as_str().unwrap_or("tap “Place order” (a test: nothing will be pressed)");
    let screen = crate::appwin::call(&hub, agent, chat, "describe", json!({ "full": true })).await.ok();
    let words = screen.as_ref().and_then(|v| v["screen"].as_str()).unwrap_or_default().to_owned();
    let app = screen.as_ref().and_then(|v| v["app"].as_str()).filter(|s| !s.is_empty()).unwrap_or("the app").to_owned();
    let ask = crate::appwin::commit_ask(&hub, agent, chat, &app, step, &words).await;
    let order = ask.order.clone();
    let allowed = hub.approvals.gate(&hub, agent, ask).await.is_ok();
    Json(json!({ "allowed": allowed, "pressed": false, "order": order })).into_response()
}

// ---- what you missed (crate::missed, R8) -------------------------------------------------------------

/// What came while the display was off, today: `{since, summary, missed: [...]}`.
async fn missed_get(State(hub): State<Shared>) -> Response {
    let h = hub.clone();
    blocking(move || {
        let now = crate::core::now();
        let all = h.core.with(|db| crate::missed::list(db, now))?;
        Ok(json!({ "since": crate::missed::caught_up(&h), "summary": crate::missed::summary(&h)?, "missed": all }))
    })
    .await
}

/// Forgets them all.
async fn missed_clear(State(hub): State<Shared>) -> Response {
    let h = hub.clone();
    blocking(move || {
        h.core.with(crate::missed::clear)?;
        Ok(json!({ "cleared": true }))
    })
    .await
}

/// You've seen them (What's up showed them): what comes after is new.
async fn missed_seen(State(hub): State<Shared>) -> Response {
    let h = hub.clone();
    blocking(move || {
        crate::missed::set_caught_up(&h, crate::core::now())?;
        Ok(json!({ "seen": true }))
    })
    .await
}

// ---- connections (crate::connections, ADR-0052) -------------------------------------------------------

async fn connections_get(State(hub): State<Shared>) -> Response {
    match crate::connections::state(&hub) {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

/// Looks again (`claude mcp list`, no model call), then answers as `GET` does.
async fn connections_check(State(hub): State<Shared>) -> Response {
    if let Err(e) = crate::connections::check(&hub).await {
        return error(StatusCode::BAD_GATEWAY, format!("{e:#}"));
    }
    connections_get(State(hub)).await
}

#[derive(Deserialize)]
struct ConnectionLogin {
    server: String,
}

/// A connector's sign-in link (a claude.ai page), for a QR code or the PC's browser.
async fn connections_login(State(hub): State<Shared>, Json(req): Json<ConnectionLogin>) -> Response {
    match crate::connections::login_link(&hub, &req.server).await {
        Ok(url) => Json(json!({ "url": url })).into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct ConnectionAdd {
    name: String,
    url: String,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    header: Option<String>,
}

/// Your own server by URL, with its API key (from the companion): the CLI keeps it.
async fn connections_add(State(hub): State<Shared>, Json(req): Json<ConnectionAdd>) -> Response {
    match crate::connections::add(&hub, req.name.trim(), req.url.trim(), req.key.as_deref(), req.header.as_deref()).await {
        Ok(()) => connections_get(State(hub)).await,
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

/// Removes one of your own servers.
async fn connections_remove(State(hub): State<Shared>, Json(req): Json<ConnectionLogin>) -> Response {
    match crate::connections::remove(&hub, &req.server).await {
        Ok(()) => connections_get(State(hub)).await,
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct ConnectionAgent {
    agent: String,
    server: String,
    on: bool,
}

/// A connection on or off for an agent.
async fn connections_agent(State(hub): State<Shared>, Json(req): Json<ConnectionAgent>) -> Response {
    if hub.core.agent(&req.agent).is_none() {
        return error(StatusCode::NOT_FOUND, format!("no agent `{}`", req.agent));
    }
    match hub.core.with(|db| crate::connections::set_on(db, &req.agent, &req.server, req.on)) {
        Ok(()) => {
            hub.emit(Event::ConnectionsChanged, Source::Api);
            connections_get(State(hub)).await
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

#[derive(Deserialize)]
struct ConnectionAlways {
    agent: String,
    tool: String,
    allow: bool,
}

/// "Always" for an agent's tool, taken back (or given, from the companion).
async fn connections_always(State(hub): State<Shared>, Json(req): Json<ConnectionAlways>) -> Response {
    if hub.core.agent(&req.agent).is_none() {
        return error(StatusCode::NOT_FOUND, format!("no agent `{}`", req.agent));
    }
    match hub.core.with(|db| crate::connections::set_always(db, &req.agent, &req.tool, req.allow)) {
        Ok(()) => {
            hub.emit(Event::ConnectionsChanged, Source::Api);
            connections_get(State(hub)).await
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    }
}

/// What's new and what's next (crate::whatsup).
async fn whatsup(State(hub): State<Shared>) -> Response {
    let h = hub.clone();
    match tokio::task::spawn_blocking(move || crate::whatsup::gather(&h)).await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ---- timers and pomodoros (crate::timers, ADR-0050) ---------------------------------------------------

/// Timers (ringing first, then the soonest to end), and now (ms).
async fn timers_list(State(hub): State<Shared>) -> Response {
    match hub.core.with(crate::timers::list) {
        Ok(list) => Json(json!({ "timers": list, "now": crate::alarms::clock().now_ms() })).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// `{"minutes": 10, "seconds": 30, "label": …, "agent"?}`, or `{"pomodoro": true, "focus": 25, "short_break": 5,
/// "long_break": 15, "every": 4, "rounds": 4}` (minutes; any left out are the usual).
async fn timers_add(State(hub): State<Shared>, Json(req): Json<Value>) -> Response {
    let now = crate::alarms::clock().now_ms();
    match hub.core.with(|db| crate::timers::from_args(db, &req, req["agent"].as_str().filter(|a| !a.is_empty()), now)) {
        Ok(t) => {
            crate::timers::changed(&hub);
            Json(json!({ "timer": t })).into_response()
        }
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

/// `pause`, `resume`, `add` (`{"seconds"}` or `{"minutes"}`; a minute if neither), `skip` (a pomodoro's phase) and
/// `stop`. A ringing one stopped, or given more (the alarm screen's click and double click), stops ringing.
async fn timers_op(State(hub): State<Shared>, Path((id, op)): Path<(String, String)>, body: axum::body::Bytes) -> Response {
    let args: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let ringing = hub.core.with(|db| crate::timers::get(db, &id)).ok().flatten().is_some_and(|t| t.ringing.is_some());
    if op == "stop" || (op == "add" && ringing) {
        return match crate::timers::end_ring(&hub, &id, op == "add") {
            Ok(true) => Json(json!({ "ok": true })).into_response(),
            Ok(false) => error(StatusCode::NOT_FOUND, format!("no timer `{id}` ringing")),
            Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
        };
    }
    let now = crate::alarms::clock().now_ms();
    match hub.core.with(|db| crate::timers::op(db, &id, &op, &args, now)) {
        Ok(t) => {
            crate::timers::changed(&hub);
            Json(json!({ "timer": t })).into_response()
        }
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

/// Cancelled (one ringing stops ringing).
async fn timers_remove(State(hub): State<Shared>, Path(id): Path<String>) -> Response {
    if let Ok(true) = crate::timers::end_ring(&hub, &id, false) {
        return Json(json!({ "removed": id })).into_response();
    }
    match hub.core.with(|db| crate::timers::remove(db, &id)) {
        Ok(true) => {
            crate::timers::changed(&hub);
            Json(json!({ "removed": id })).into_response()
        }
        Ok(false) => error(StatusCode::NOT_FOUND, format!("no timer `{id}`")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn time_get(State(hub): State<Shared>) -> Json<Value> {
    let c = crate::alarms::clock();
    let now = c.now_ms();
    Json(json!({
        "zone": crate::alarms::zone(),
        "automatic": hub.core.zone_auto(),
        "now": now,
        "local": crate::alarms::describe_time(now, &|ms| c.offset_ms(ms)),
    }))
}

#[derive(Deserialize)]
struct ZoneReq {
    /// A zone the user picked (it stops following the network).
    zone: Option<String>,
    /// Follow the network (looked up now), or not.
    automatic: Option<bool>,
}

/// Sets the R1's time zone (an Olson id), or whether it follows the network (ADR-0030).
async fn time_set(State(hub): State<Shared>, Json(req): Json<ZoneReq>) -> Response {
    let done = match (req.zone, req.automatic) {
        (Some(zone), _) => crate::alarms::set_zone(&hub, &zone).await,
        (None, Some(on)) => crate::alarms::set_zone_auto(&hub, on).await,
        (None, None) => Err(anyhow::anyhow!("give zone or automatic")),
    };
    match done {
        Ok(()) => time_get(State(hub)).await.into_response(),
        Err(e) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
    }
}

/// A network came up (the shell says): the time zone is looked up, if it's automatic (at most every 10 min).
async fn time_auto(State(hub): State<Shared>) -> Response {
    match crate::alarms::auto_zone(&hub, false).await {
        Ok(changed) => Json(json!({ "changed": changed, "zone": crate::alarms::zone() })).into_response(),
        Err(e) => error(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

/// The host's runtime permissions (ADR-0031): the camera and the mic. Only the system grants them without asking,
/// and asking would be Android UI (ADR-0014), so r1d grants them, to the host only. At boot r1d can start before
/// Android's package service: it tries again for a couple of minutes ("Can't find service: package", 2026-10-10).
pub async fn grant_host() -> Vec<String> {
    let mut granted = Vec::new();
    if !cfg!(target_os = "android") {
        return granted;
    }
    for attempt in 0..24 {
        granted.clear();
        let mut early = false;
        for perm in ["android.permission.CAMERA", "android.permission.RECORD_AUDIO"] {
            match crate::tool::run("pm", &["grant", "dev.r1os.host", perm], crate::tool::DEADLINE).await {
                Ok(o) if o.ok => granted.push(perm.to_owned()),
                Ok(o) if o.stderr.contains("Can't find service") || o.stdout.contains("Can't find service") => early = true,
                Ok(o) => tracing::warn!(perm, error = %o.stderr.trim(), "couldn't grant the host a permission"),
                Err(e) => tracing::warn!(perm, error = %e, "couldn't grant the host a permission"),
            }
        }
        if !early {
            break;
        }
        if attempt == 23 {
            tracing::warn!("couldn't grant the host its permissions: Android's package service never came up");
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
    crate::android::grant_store().await;
    crate::android::allow_listener().await;
    crate::android::compile_host().await;
    granted
}

async fn android_permissions() -> Json<Value> {
    Json(json!({ "granted": grant_host().await }))
}

/// The last Android app [`android_front`] saw: by the time you've finished speaking, the host's own activity can
/// be on top of it (ADR-0027).
static LAST_FRONT: std::sync::Mutex<Option<(String, std::time::Instant)>> = std::sync::Mutex::new(None);

/// That app, if it was seen in the last few minutes.
fn recent_front() -> Option<String> {
    let last = LAST_FRONT.lock().unwrap();
    last.as_ref().filter(|(_, at)| at.elapsed().as_secs() < 300).map(|(p, _)| p.clone())
}

/// The Android app in front, and whether the host's overlay shows over it (ADR-0027): Android hides apps'
/// overlays while a window that asks it to is on screen (its own settings, the keyguard, permission prompts).
async fn android_front() -> Response {
    if !cfg!(target_os = "android") {
        return Json(json!({ "package": null, "overlay": false })).into_response();
    }
    let package = focused_android_app().await;
    if let Some(p) = &package {
        *LAST_FRONT.lock().unwrap() = Some((p.clone(), std::time::Instant::now()));
    }
    let overlay = match crate::tool::run("dumpsys", &["window", "windows"], crate::tool::DEADLINE).await {
        Ok(o) if o.ok => !hides_overlays(&o.stdout),
        _ => false,
    };
    Json(json!({ "package": package, "overlay": overlay })).into_response()
}

/// Whether a window on screen hides other apps' overlays (`HIDE_NON_SYSTEM_OVERLAY_WINDOWS` in its private
/// flags), from `dumpsys window windows`: each window's block has its flags (`pfl=`), then `isOnScreen=`.
fn hides_overlays(dump: &str) -> bool {
    let mut hides = false;
    for line in dump.lines().map(str::trim) {
        if line.starts_with("Window #") {
            hides = false;
        } else if line.contains("pfl=") && line.contains("HIDE_NON_SYSTEM_OVERLAY_WINDOWS") {
            hides = true;
        } else if hides && line == "isOnScreen=true" {
            return true;
        }
    }
    false
}

/// The host may draw over other apps (ADR-0027: talking and volume over Android apps). It's an app-op only
/// the system grants; r1d grants it to the host, and only that.
async fn android_overlay() -> Response {
    if !cfg!(target_os = "android") {
        return error(StatusCode::NOT_IMPLEMENTED, "not on Android".into());
    }
    match crate::tool::run("cmd", &["appops", "set", "dev.r1os.host", "SYSTEM_ALERT_WINDOW", "allow"], crate::tool::DEADLINE).await {
        Ok(o) if o.ok => Json(json!({ "overlay": true })).into_response(),
        Ok(o) => error(StatusCode::INTERNAL_SERVER_ERROR, o.stderr),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// The host may change the display's brightness and timeout (ADR-0045: Settings › Display & sound, agents' changes
/// you approve). WRITE_SETTINGS is a preinstalled app's; if Android hasn't given its app-op, r1d does, to the host only.
async fn android_settings_access() -> Response {
    if !cfg!(target_os = "android") {
        return error(StatusCode::NOT_IMPLEMENTED, "not on Android".into());
    }
    match crate::tool::run("cmd", &["appops", "set", "dev.r1os.host", "WRITE_SETTINGS", "allow"], crate::tool::DEADLINE).await {
        Ok(o) if o.ok => Json(json!({ "settings": true })).into_response(),
        Ok(o) => error(StatusCode::INTERNAL_SERVER_ERROR, o.stderr),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// What an Android app is playing (R7), as the host last said: `{media}` (null when nothing).
async fn android_media(State(hub): State<Shared>) -> Json<Value> {
    Json(json!({ "media": hub.android.media() }))
}

#[derive(Deserialize)]
struct MediaReport {
    media: Option<crate::android::Playing>,
}

/// The host's media sessions changed: `{media}`, what's playing now (or `null`).
async fn android_media_set(State(hub): State<Shared>, Json(r): Json<MediaReport>) -> Json<Value> {
    hub.android.set_media(r.media);
    Json(json!({ "ok": true }))
}

/// The host lists, scans for and pairs Bluetooth devices (Settings › Bluetooth, R7): its two runtime permissions,
/// which a preinstalled app doesn't ask the user for. r1d grants them, to the host only.
async fn android_bluetooth_access() -> Response {
    if !cfg!(target_os = "android") {
        return error(StatusCode::NOT_IMPLEMENTED, "not on Android".into());
    }
    for p in ["android.permission.BLUETOOTH_CONNECT", "android.permission.BLUETOOTH_SCAN"] {
        match crate::tool::run("pm", &["grant", "dev.r1os.host", p], crate::tool::DEADLINE).await {
            Ok(o) if o.ok => {}
            Ok(o) => return error(StatusCode::INTERNAL_SERVER_ERROR, o.stderr),
            Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        }
    }
    Json(json!({ "bluetooth": true })).into_response()
}

/// Starts a turn; the reply streams as `chat` events. 409 while this chat's last turn is running.
async fn chat_send(State(hub): State<Shared>, Path(id): Path<String>, headers: axum::http::HeaderMap, Json(req): Json<SendReq>) -> Response {
    // Typed on a linked computer (crate::link names it, ADR-0051).
    let computer = headers.get(crate::link::FROM_COMPUTER).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let text = req.text.trim().to_owned();
    if text.is_empty() || text.len() > 8000 {
        return error(StatusCode::BAD_REQUEST, "say something (up to 8000 characters)".into());
    }
    let agent = match hub.core.chat(&id) {
        Ok(Some(c)) => c.agent,
        Ok(None) => return error(StatusCode::NOT_FOUND, format!("no chat `{id}`")),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
    };
    if !hub.chat_busy.lock().unwrap().insert(id.clone()) {
        return error(StatusCode::CONFLICT, "still answering the last message".into());
    }
    hub.emit(Event::chat(&agent, &id, "user", Some(text.clone())), Source::Api);
    let h = hub.clone();
    tokio::spawn(async move {
        // Over an Android app (ADR-0027): which one, from the window manager.
        let on_screen = match (req.app, req.android) {
            _ if computer.is_some() => computer.map(crate::chat::OnScreen::Computer),
            (Some(app), _) => Some(crate::chat::OnScreen::App(app)),
            (None, true) => focused_android_app().await.or_else(recent_front).map(crate::chat::OnScreen::Android),
            (None, false) => None,
        };
        if let Err(e) = crate::chat::turn(h.clone(), id.clone(), text, on_screen, req.images).await {
            tracing::warn!(chat = %id, error = %format!("{e:#}"), "chat turn failed");
        }
        h.chat_busy.lock().unwrap().remove(&id);
    });
    (StatusCode::ACCEPTED, Json(json!({ "accepted": true }))).into_response()
}

// ---- harness ----------------------------------------------------------------------------------

async fn harness_state(State(hub): State<Shared>) -> Json<Value> {
    let mut v = snapshot(&hub);
    let events = hub.events_after(0);
    let tail: Vec<&Stamped> = events.iter().rev().take(50).rev().collect();
    v["events"] = json!(tail);
    Json(v)
}

#[derive(Deserialize)]
struct After {
    #[serde(default)]
    after: u64,
}

async fn harness_events(State(hub): State<Shared>, Query(q): Query<After>) -> Json<Vec<Stamped>> {
    Json(hub.events_after(q.after))
}

#[derive(Deserialize)]
struct WheelReq {
    delta: i32,
}

/// Simulated detents, 40 ms apart (about a brisk spin).
async fn harness_wheel(State(hub): State<Shared>, Json(req): Json<WheelReq>) -> Json<Value> {
    let step = req.delta.signum();
    for _ in 0..req.delta.unsigned_abs().min(100) {
        hub.inject(Raw::Wheel(step));
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    Json(json!({ "ok": true }))
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ButtonAction {
    Down,
    Up,
    Tap,
    Double,
    Hold,
}

#[derive(Deserialize)]
struct ButtonReq {
    action: ButtonAction,
    /// How long `hold` keeps the button down (default 1000).
    ms: Option<u64>,
}

/// How the R1 is held, for the host (T-156): `{side: 1 | -1 | 0}`, or `{side: null}` back to its accelerometer.
async fn harness_tilt(State(hub): State<Shared>, Json(req): Json<Value>) -> Json<Value> {
    let side = req["side"].as_i64().map(|s| s.clamp(-1, 1) as i32);
    hub.emit(Event::Tilt { side }, Source::Sim);
    Json(json!({ "side": side }))
}

/// The glance screen's rest held at a step for the host (T-157): 0 awake, 1 still, 2 low, `null` its own timer again;
/// `brightness` (0..1) tries another low backlight.
async fn harness_rest(State(hub): State<Shared>, Json(req): Json<Value>) -> Json<Value> {
    let step = req["step"].as_i64().map(|s| s.clamp(0, 2) as i32);
    let brightness = req["brightness"].as_f64().map(|b| b.clamp(0.0, 1.0));
    hub.emit(Event::Rest { step, brightness }, Source::Sim);
    Json(json!({ "step": step, "brightness": brightness }))
}

/// Plays raw edges with realistic timing, then returns once they're all in.
async fn harness_button(State(hub): State<Shared>, Json(req): Json<ButtonReq>) -> Json<Value> {
    let edge = |down| hub.inject(Raw::Button(down));
    let wait = |ms| tokio::time::sleep(Duration::from_millis(ms));
    match req.action {
        ButtonAction::Down => edge(true),
        ButtonAction::Up => edge(false),
        ButtonAction::Tap => {
            edge(true);
            wait(90).await;
            edge(false);
        }
        ButtonAction::Double => {
            edge(true);
            wait(90).await;
            edge(false);
            wait(110).await;
            edge(true);
            wait(90).await;
            edge(false);
        }
        ButtonAction::Hold => {
            edge(true);
            wait(req.ms.unwrap_or(1000).min(30_000)).await;
            edge(false);
        }
    }
    Json(json!({ "ok": true }))
}

/// The R1's framebuffer as PNG (`screencap -p`; r1d runs as root). Simulator: 501.
async fn harness_screenshot() -> Response {
    if !cfg!(target_os = "android") {
        return error(StatusCode::NOT_IMPLEMENTED, "screenshots need the device; use the browser in the simulator".into());
    }
    // With a deadline: SurfaceFlinger hands the capture back through a binder call into the tool, which
    // image v0.2.5's policy refuses for r1d's domain (`surfaceflinger → r1d binder call`), so screencap
    // hung forever. v0.2.6 allows it; until then this fails in 8 s and `r1h screenshot` uses adb's own.
    match crate::tool::run_bytes("screencap", &["-p"], std::time::Duration::from_secs(8)).await {
        Ok((png, _)) if png.ends_with(&[0xAE, 0x42, 0x60, 0x82]) => ([(header::CONTENT_TYPE, "image/png")], png).into_response(),
        Ok(_) => error(StatusCode::GATEWAY_TIMEOUT, "screencap gave no image (SurfaceFlinger's call-back refused: image v0.2.6 fixes it)".into()),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct SayReq {
    text: String,
    ms_per_word: Option<u64>,
}

#[derive(Deserialize)]
struct LaunchReq {
    app: String,
}

/// Opens an app on the R1, as the agent's `open_app` does.
async fn harness_open(State(hub): State<Shared>, Json(req): Json<LaunchReq>) -> Response {
    if hub.apps.get(&req.app).is_none() {
        return error(StatusCode::NOT_FOUND, format!("no app `{}`", req.app));
    }
    hub.emit(Event::Open { app: req.app }, Source::Sim);
    Json(json!({ "opened": true })).into_response()
}

#[derive(Deserialize)]
struct TriggerReq {
    app: String,
    trigger: String,
}

/// Runs one of an app's triggers now (a reminder, without waiting for it). Its notifications show as usual.
async fn harness_trigger(State(hub): State<Shared>, Json(req): Json<TriggerReq>) -> Response {
    let h = hub.clone();
    let fired = tokio::task::spawn_blocking(move || h.scheduler.fire_now(&h, &req.app, &req.trigger)).await;
    match fired {
        Ok(Ok(notices)) => {
            let n = notices.len();
            for notice in notices {
                crate::scheduler::notify(&hub, notice).await;
            }
            Json(json!({ "fired": true, "notified": n })).into_response()
        }
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct NetReq {
    host: String,
}

/// Network from r1d itself, step by step: the host's addresses, then a TCP connect to each (port 443).
async fn harness_net(axum::extract::Query(req): axum::extract::Query<NetReq>) -> Response {
    blocking(move || {
        use std::net::ToSocketAddrs;
        let addrs: Vec<std::net::SocketAddr> = match (req.host.as_str(), 443).to_socket_addrs() {
            Ok(a) => a.collect(),
            Err(e) => return Ok(json!({ "resolve": format!("{e} ({:?})", e.raw_os_error()) })),
        };
        let connects: Vec<Value> = addrs
            .iter()
            .map(|a| match std::net::TcpStream::connect_timeout(a, std::time::Duration::from_secs(5)) {
                Ok(_) => json!({ "addr": a.to_string(), "ok": true }),
                Err(e) => json!({ "addr": a.to_string(), "error": e.to_string(), "os": e.raw_os_error() }),
            })
            .collect();
        Ok(json!({ "addrs": addrs.iter().map(|a| a.to_string()).collect::<Vec<_>>(), "connect": connects }))
    })
    .await
}

/// A scripted utterance through the same `voice` events a real hold produces (no mic needed).
async fn harness_say(State(hub): State<Shared>, Json(req): Json<SayReq>) -> Json<Value> {
    let words = req.text.split_whitespace().count();
    tokio::spawn(crate::voice::say(hub.clone(), req.text, req.ms_per_word.unwrap_or(240)));
    Json(json!({ "accepted": true, "words": words }))
}

#[derive(Deserialize)]
struct System1Req {
    text: String,
}

#[derive(Deserialize)]
struct TidyReq {
    chat: String,
}

/// Tidies a chat now (crate::chat::tidy), however short: the note it continues from, and a fresh session next
/// turn (T-003: a real one waits for 100k tokens of context).
async fn harness_tidy(State(hub): State<Shared>, Json(req): Json<TidyReq>) -> Response {
    let t = std::time::Instant::now();
    match crate::chat::tidy(hub.clone(), req.chat.clone()).await {
        Ok(()) => {
            let chat = hub.core.chat(&req.chat).ok().flatten();
            Json(json!({ "ms": t.elapsed().as_millis(), "session": chat.as_ref().and_then(|c| c.session.clone()), "summary": chat.and_then(|c| c.summary) })).into_response()
        }
        Err(e) => error(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

/// The Recorder's title and summary for some words (T-068), without a tape: `{title, summary, ms}`.
async fn harness_summarize(State(hub): State<Shared>, Json(req): Json<System1Req>) -> Response {
    let t = std::time::Instant::now();
    match crate::recorder::summary_of(&hub, &req.text).await {
        Ok((title, summary)) => Json(json!({ "title": title, "summary": summary, "ms": t.elapsed().as_millis() })).into_response(),
        Err(e) => error(StatusCode::BAD_GATEWAY, format!("{e:#}")),
    }
}

/// What System One makes of some words, without saying them: `{intent, status}`.
async fn harness_system1(State(hub): State<Shared>, Json(req): Json<System1Req>) -> Json<Value> {
    let h = hub.clone();
    let t = std::time::Instant::now();
    let intent = tokio::task::spawn_blocking(move || h.system1.decide(&h, &req.text)).await.ok().flatten();
    Json(json!({ "intent": intent, "ms": t.elapsed().as_millis(), "status": hub.system1.status() }))
}

#[derive(Deserialize)]
struct HearReq {
    path: String,
    /// Benchmark another model directory instead of the loaded one (loaded fresh, then dropped).
    model_dir: Option<String>,
    /// Threads for that model (default 2, as r1d runs it).
    threads: Option<i32>,
}

/// Transcribes a WAV on r1d's filesystem: the speech-to-text benchmark. With `model_dir` it also
/// times loading that model, for comparing variants without restarting r1d.
async fn harness_hear(State(hub): State<Shared>, Json(req): Json<HearReq>) -> Response {
    let loaded = hub.voice.get().and_then(|v| v.stt());
    let run = move || -> anyhow::Result<Value> {
        let (stt, load_ms) = match &req.model_dir {
            Some(dir) => {
                let t = std::time::Instant::now();
                let s = std::sync::Arc::new(crate::voice::Stt::load(std::path::Path::new(dir), req.threads.unwrap_or(2))?);
                (s, Some(t.elapsed().as_millis()))
            }
            None => (loaded.ok_or_else(|| anyhow::anyhow!("speech-to-text isn't loaded (GET /api/voice)"))?, None),
        };
        let t = stt.transcribe(&req.path)?;
        Ok(json!({
            "text": t.text,
            "partials": t.partials.iter().map(|(at, p)| json!({ "at_s": at, "text": p })).collect::<Vec<_>>(),
            "audio_s": t.audio_s,
            "compute_ms": t.compute.as_millis(),
            "rtf": t.compute.as_secs_f32() / t.audio_s.max(0.001),
            "load_ms": load_ms,
            "model": stt.model,
        }))
    };
    match tokio::task::spawn_blocking(run).await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, e.to_string()),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct TtsReq {
    /// A voice package on r1d's filesystem (crate::tts).
    dir: String,
    texts: Vec<String>,
    /// Default 2.
    threads: Option<i32>,
    #[serde(default)]
    sid: i32,
    speed: Option<f32>,
    steps: Option<i32>,
    lang: Option<String>,
    /// Pocket's reference clip (relative to `dir`, or absolute).
    reference: Option<String>,
    /// Writes `{out}-{n}.wav` for each text.
    out: Option<String>,
    /// cpu (default) · xnnpack · nnapi
    provider: Option<String>,
}

/// Times a voice package on the R1 (the TTS benchmark, Q14): its load, then each text after a warm-up,
/// with each sentence's compute against its audio length. Loaded fresh, then dropped.
async fn harness_tts(Json(req): Json<TtsReq>) -> Response {
    let run = move || -> anyhow::Result<Value> {
        anyhow::ensure!(!req.texts.is_empty() && req.texts.len() <= 20, "give 1–20 texts");
        let o = crate::tts::Options {
            sid: req.sid,
            speed: req.speed,
            steps: req.steps,
            lang: req.lang,
            reference: req.reference.map(std::path::PathBuf::from),
            provider: req.provider,
        };
        let out = req.out.as_deref().map(std::path::Path::new);
        let b = crate::tts::bench(std::path::Path::new(&req.dir), req.threads.unwrap_or(2), &o, &req.texts, out)?;
        Ok(serde_json::to_value(b)?)
    };
    match tokio::task::spawn_blocking(run).await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
struct ParadeeReq {
    model: String,
    /// Kokoro phoneme ids, one list per sentence (misaki's G2P on a PC, for now).
    ids: Vec<Vec<i64>>,
    threads: Option<usize>,
    out: Option<String>,
}

/// Times Paradee-8M on the R1 (crate::tts::paradee_bench): the TTS benchmark's one model that isn't sherpa-onnx's.
async fn harness_paradee(State(hub): State<Shared>, Json(req): Json<ParadeeReq>) -> Response {
    let lib = crate::system1::paths(&hub.cfg.data).1;
    let run = move || -> anyhow::Result<Value> {
        anyhow::ensure!(!req.ids.is_empty() && req.ids.len() <= 20 && req.ids.iter().all(|s| s.len() < 510), "give 1–20 lists of up to 509 ids");
        let out = req.out.as_deref().map(std::path::Path::new);
        let b = crate::tts::paradee_bench(lib.as_deref(), std::path::Path::new(&req.model), req.threads.unwrap_or(2), &req.ids, out)?;
        Ok(serde_json::to_value(b)?)
    };
    match tokio::task::spawn_blocking(run).await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => error(StatusCode::BAD_REQUEST, format!("{e:#}")),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Runs a fixed probe inside the agent jail: who it is, its seccomp state and cgroup, and whether
/// forbidden operations really fail. For checking crate::jail on the device.
async fn harness_jail() -> Response {
    const PROBE: &str = "id; echo context: $(cat /proc/self/attr/current); grep -E '^(Seccomp|NoNewPrivs|CapEff|CapBnd)' /proc/self/status;         echo cgroup: $(grep memory /proc/self/cgroup);         mkdir -p /tmp/x; mount -t tmpfs t /tmp/x 2>&1 | head -1;         chroot / true 2>&1 | head -1; unshare -m true 2>&1 | head -1;         ls /dev | tr '\n' ' '; echo; ls /data 2>&1 | head -1;         wget -q -O /dev/null -T 5 https://api.anthropic.com/ 2>&1 | head -1; echo net-exit: $?";
    let jail = crate::jail::Jail::default();
    let cmd = jail.command("/bin/sh", &["-c", PROBE], "/work", &[]);
    let out = match cmd {
        Ok(mut c) => c.stdin(std::process::Stdio::null()).output().await,
        Err(e) => return error(StatusCode::NOT_IMPLEMENTED, format!("{e:#}")),
    };
    match out {
        Ok(o) => Json(json!({
            "status": o.status.code(),
            "stdout": String::from_utf8_lossy(&o.stdout),
            "stderr": String::from_utf8_lossy(&o.stderr),
        }))
        .into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, format!("spawn: {e}")),
    }
}

/// The per-boot token, for the host app only: r1d checks whose socket asked (ADR-0036).
async fn host_token(State(hub): State<Shared>, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>) -> Response {
    match hub.auth.for_host(peer, hub.cfg.listen.port()).await {
        Ok(t) => Json(json!({ "token": t })).into_response(),
        Err(e) => error(StatusCode::FORBIDDEN, e.into()),
    }
}

fn error(status: StatusCode, msg: String) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlays_hide_under_windows_that_ask() {
        let settings = "  Window #2 Window{1 u0 NotificationShade}:
      pfl=OPTIMIZE_MEASURE HIDE_NON_SYSTEM_OVERLAY_WINDOWS
    isOnScreen=false
  Window #6 Window{2 u0 com.android.settings/.Settings}:
      pfl=OPTIMIZE_MEASURE HIDE_NON_SYSTEM_OVERLAY_WINDOWS FIT_INSETS_CONTROLLED
    mHasSurface=true isReadyForDisplay()=true
    isOnScreen=true
";
        assert!(hides_overlays(settings));
        let files = "  Window #2 Window{1 u0 NotificationShade}:
      pfl=OPTIMIZE_MEASURE HIDE_NON_SYSTEM_OVERLAY_WINDOWS
    isOnScreen=false
  Window #6 Window{2 u0 com.android.documentsui/.files.FilesActivity}:
      pfl=OPTIMIZE_MEASURE FIT_INSETS_CONTROLLED
    isOnScreen=true
";
        assert!(!hides_overlays(files));
    }
}
