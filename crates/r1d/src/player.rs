//! The R1's player for apps (ADR-0040): an app's `play` step (a radio station, a podcast's stream) plays on in the
//! background, from the host's media service, after the user leaves the app.
//!
//! - **r1d decides:** an app's page asks (`POST /api/apps/{id}/media`); r1d checks the step, the app's `audio.play`
//!   permission and the URL (the web only, and the hosts it declares, if any), and tells the host (`media` events).
//! - **The host plays** (its media service: MediaPlayer, MediaSession, audio focus, the glance's controls) and says
//!   what it's doing (`PUT /api/player`). r1d keeps that and tells the app's page (`app_player`, its `/player`).
//! - **Streams go through r1d** ([`relay`]) unless they're HLS: the host only speaks cleartext to r1d itself, so an
//!   `http://` station plays, the same public-addresses-only check applies to every hop, and Icecast's song titles
//!   (`icy-metaint`) become `player.song`.
//!
//! One player for the R1: a new `play` replaces what was playing, whichever app it was.

use crate::hub::{Event, Hub, Source};
use axum::body::{Body, Bytes};
use serde::Serialize;
use serde_json::{Value, json};
use std::io::Read;
use std::pin::Pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

/// What the player is doing, as pages bind it (`{{player.…}}`, enee-spec's PLAYER_FIELDS) and the glance shows it.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct State {
    /// The app playing (empty: nothing).
    pub app: String,
    /// `playing`, `paused`, `loading`, `stopped` or `error`.
    pub state: String,
    pub playing: bool,
    pub title: String,
    pub subtitle: String,
    /// The song a station says is on (Icecast's `StreamTitle`).
    pub song: String,
    /// What the app asked to play.
    pub url: String,
    pub error: String,
}

impl State {
    fn stopped() -> State {
        State { state: "stopped".into(), ..State::default() }
    }

    /// What `app`'s page sees: the player, if it's this app's; else nothing playing.
    pub fn for_app(&self, app: &str) -> Value {
        let s = if self.app == app { self.clone() } else { State::stopped() };
        json!({ "playing": s.playing, "state": s.state, "title": s.title, "subtitle": s.subtitle, "song": s.song, "url": s.url, "error": s.error })
    }
}

#[derive(Default)]
pub struct Player {
    state: Mutex<State>,
    /// The stream [`relay`] serves now: its id (in the host's URL) and where it really is.
    stream: Mutex<Option<(String, String)>>,
    /// Bumped by every `play` and `stop`: an older relay notices and ends.
    generation: AtomicU64,
}

impl Player {
    /// Something is playing, paused or loading (not stopped, and not failed).
    pub fn busy(&self) -> bool {
        matches!(self.state.lock().unwrap().state.as_str(), "playing" | "paused" | "loading")
    }

    pub fn now(&self) -> State {
        let s = self.state.lock().unwrap().clone();
        if s.state.is_empty() { State::stopped() } else { s }
    }

    fn set(&self, hub: &Hub, f: impl FnOnce(&mut State)) {
        let snapshot = {
            let mut s = self.state.lock().unwrap();
            f(&mut s);
            s.playing = s.state == "playing";
            s.clone()
        };
        if !snapshot.app.is_empty() {
            hub.emit(Event::AppPlayer { app: snapshot.app.clone(), player: snapshot.for_app(&snapshot.app) }, Source::Api);
        }
        hub.emit(Event::Player { player: serde_json::to_value(&snapshot).unwrap_or_default() }, Source::Api);
    }

    /// An app's `play` step, checked: tells the host what to play.
    pub fn play(&self, hub: &Hub, app: &str, url: &str, title: &str, subtitle: &str) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let hls = url::Url::parse(url).is_ok_and(|u| u.path().to_ascii_lowercase().ends_with(".m3u8"));
        let playable = if hls {
            *self.stream.lock().unwrap() = None;
            url.to_owned()
        } else {
            let id = format!("{generation}-{}", crate::core::random_hex(8));
            *self.stream.lock().unwrap() = Some((id.clone(), url.to_owned()));
            format!("http://127.0.0.1:{}/api/player/stream/{id}", hub.cfg.listen.port())
        };
        self.set(hub, |s| {
            *s = State {
                app: app.to_owned(),
                state: "loading".into(),
                title: title.to_owned(),
                subtitle: subtitle.to_owned(),
                url: url.to_owned(),
                ..State::default()
            };
        });
        hub.emit(
            Event::Media { op: "play".into(), app: app.to_owned(), url: Some(playable), title: Some(title.to_owned()), subtitle: Some(subtitle.to_owned()) },
            Source::Api,
        );
    }

    /// Pause, resume, stop or toggle what's playing (an app's step acts only on its own; an agent's on anything).
    pub fn control(&self, hub: &Hub, app: Option<&str>, op: &str) -> bool {
        let now = self.now();
        if !self.busy() || app.is_some_and(|a| a != now.app) {
            return false;
        }
        if op == "stop" {
            self.generation.fetch_add(1, Ordering::SeqCst);
            *self.stream.lock().unwrap() = None;
        }
        hub.emit(Event::Media { op: op.into(), app: now.app, url: None, title: None, subtitle: None }, Source::Api);
        true
    }

    /// What the host's player says it's doing.
    pub fn report(&self, hub: &Hub, state: &str, error: Option<&str>) {
        if state == "stopped" {
            self.generation.fetch_add(1, Ordering::SeqCst);
            *self.stream.lock().unwrap() = None;
        }
        self.set(hub, |s| {
            s.state = state.to_owned();
            s.error = error.unwrap_or_default().to_owned();
            if state == "stopped" {
                s.song.clear();
            }
        });
    }

    fn song(&self, hub: &Hub, generation: u64, song: String) {
        if self.generation.load(Ordering::SeqCst) != generation || self.state.lock().unwrap().song == song {
            return;
        }
        self.set(hub, |s| s.song = song);
    }

    /// The stream for `id`, if it's the one playing: where it is, and its generation.
    fn stream(&self, id: &str) -> Option<(String, u64)> {
        let s = self.stream.lock().unwrap();
        s.as_ref().filter(|(i, _)| i == id).map(|(_, url)| (url.clone(), self.generation.load(Ordering::SeqCst)))
    }
}

/// An action's player steps run without its page (an agent's `run_action`, System One's run): the same checks as the
/// page's (`audio.play` granted, the link on the web, the declared hosts), then the player. A line for each.
pub fn perform(hub: &Hub, app: &enee_spec::compiled::CompiledApp, steps: &[crate::apps::MediaStep]) -> Vec<String> {
    let id = &app.app.id;
    let mut out = Vec::new();
    for s in steps {
        if !crate::broker::allows(&hub.core, app, "audio.play") {
            out.push("it isn't allowed to play audio yet (the user allows it when they open the app)".into());
            break;
        }
        if s.op != "play" {
            if hub.player.control(hub, Some(id), &s.op) {
                out.push(format!("the player: {}", s.op));
            }
            continue;
        }
        let url = match crate::fetch::link_ok(&s.url) {
            Ok(u) => u,
            Err(e) => {
                out.push(format!("couldn't play it: {e}"));
                continue;
            }
        };
        let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)).unwrap_or_default();
        if !crate::broker::allows_audio_host(app, &host) {
            out.push(format!("it may only play from the hosts it names, not {host}"));
            continue;
        }
        hub.core.grant_used(id, "audio.play");
        hub.player.play(hub, id, &url, &s.title, &s.subtitle);
        out.push(format!("playing {}", if s.title.is_empty() { url.as_str() } else { s.title.as_str() }));
    }
    out
}

/// Body chunks from the relay's thread.
struct Chunks(tokio::sync::mpsc::Receiver<Result<Bytes, std::io::Error>>);

impl futures_core::Stream for Chunks {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}

/// `GET /api/player/stream/{id}`: the station, fetched by r1d (any public http(s) address, each redirect's too), its
/// Icecast metadata taken out (its song titles go to [`State::song`]). Ends when another `play` or a `stop` comes, the
/// station stops, or the host hangs up.
pub async fn relay(hub: std::sync::Arc<Hub>, id: &str) -> Result<(String, Body), String> {
    let (upstream, generation) = hub.player.stream(id).ok_or("that stream isn't playing")?;
    let (head_tx, head_rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(8);
    std::thread::Builder::new()
        .name("r1d-relay".into())
        .spawn(move || {
            let response = crate::fetch::stream_agent().get(&upstream).header("icy-metadata", "1").call();
            let mut response = match response {
                Ok(r) if r.status().is_success() => r,
                Ok(r) => {
                    let _ = head_tx.send(Err(format!("the station answered HTTP {}", r.status().as_u16())));
                    return;
                }
                Err(e) => {
                    let _ = head_tx.send(Err(e.to_string()));
                    return;
                }
            };
            let header = |n: &str| response.headers().get(n).and_then(|v| v.to_str().ok()).map(str::to_owned);
            let kind = header("content-type").unwrap_or_else(|| "audio/mpeg".into());
            let metaint = header("icy-metaint").and_then(|m| m.trim().parse::<usize>().ok()).filter(|m| *m > 0);
            if head_tx.send(Ok(kind)).is_err() {
                return;
            }
            let mut icy = metaint.map(Icy::new);
            let mut reader = response.body_mut().as_reader();
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                if hub.player.generation.load(Ordering::SeqCst) != generation {
                    break;
                }
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let audio = match icy.as_mut() {
                    Some(icy) => {
                        let (audio, title) = icy.feed(&buf[..n]);
                        if let Some(t) = title {
                            hub.player.song(&hub, generation, t);
                        }
                        audio
                    }
                    None => buf[..n].to_vec(),
                };
                if !audio.is_empty() && tx.blocking_send(Ok(Bytes::from(audio))).is_err() {
                    break;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    let kind = head_rx.await.map_err(|_| "the stream ended before it started".to_owned())??;
    Ok((kind, Body::from_stream(Chunks(rx))))
}

/// Icecast's in-band metadata: after every `metaint` bytes of audio, a length byte (×16) and that much text
/// (`StreamTitle='Artist - Song';`). [`Icy::feed`] returns the audio alone, and a title when one arrives.
struct Icy {
    metaint: usize,
    /// Audio bytes left before the next length byte.
    audio_left: usize,
    /// Metadata bytes left to read, and what's been read.
    meta_left: usize,
    meta: Vec<u8>,
}

impl Icy {
    fn new(metaint: usize) -> Icy {
        Icy { metaint, audio_left: metaint, meta_left: 0, meta: Vec::new() }
    }

    fn feed(&mut self, mut data: &[u8]) -> (Vec<u8>, Option<String>) {
        let mut audio = Vec::with_capacity(data.len());
        let mut title = None;
        while !data.is_empty() {
            if self.audio_left > 0 {
                let n = self.audio_left.min(data.len());
                audio.extend_from_slice(&data[..n]);
                self.audio_left -= n;
                data = &data[n..];
            } else if self.meta_left == 0 && self.meta.is_empty() {
                // The length byte.
                self.meta_left = usize::from(data[0]) * 16;
                data = &data[1..];
                if self.meta_left == 0 {
                    self.audio_left = self.metaint;
                }
            } else {
                let n = self.meta_left.min(data.len());
                self.meta.extend_from_slice(&data[..n]);
                self.meta_left -= n;
                data = &data[n..];
                if self.meta_left == 0 {
                    title = stream_title(&self.meta).or(title);
                    self.meta.clear();
                    self.audio_left = self.metaint;
                }
            }
        }
        (audio, title)
    }
}

/// `StreamTitle='…';` from a metadata block.
fn stream_title(meta: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(meta);
    let start = text.find("StreamTitle='")? + "StreamTitle='".len();
    let rest = &text[start..];
    let end = rest.find("';").unwrap_or(rest.trim_end_matches('\0').len());
    let title = rest[..end].trim();
    (!title.is_empty()).then(|| title.chars().take(160).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icy_metadata_comes_out() {
        let mut icy = Icy::new(4);
        let meta = b"StreamTitle='Daft Punk - One More Time';\0\0\0\0\0\0\0\0";
        assert_eq!(meta.len() % 16, 0);
        let mut stream = b"abcd".to_vec();
        stream.push((meta.len() / 16) as u8);
        stream.extend_from_slice(meta);
        stream.extend_from_slice(b"efgh");
        stream.push(0);
        stream.extend_from_slice(b"ij");
        // Fed in awkward pieces, as reads arrive.
        let mut audio = Vec::new();
        let mut titles = Vec::new();
        for piece in stream.chunks(3) {
            let (a, t) = icy.feed(piece);
            audio.extend(a);
            titles.extend(t);
        }
        assert_eq!(audio, b"abcdefghij");
        assert_eq!(titles, ["Daft Punk - One More Time"]);
    }

    #[test]
    fn what_a_page_sees() {
        let s = State { app: "radio".into(), state: "playing".into(), playing: true, title: "KEXP".into(), ..State::default() };
        assert_eq!(s.for_app("radio")["title"], "KEXP");
        assert_eq!(s.for_app("other")["state"], "stopped", "another app's player isn't this one's");
    }
}
