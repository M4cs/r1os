//! Android apps on the R1 (ADR-0013, roadmap R4): the ones installed, opening one, and Aurora Store's page for
//! one, where the user installs it with a tap. Agents use these through their tools (crate::mcp), and System One
//! opens one by name ("open spotify").
//!
//! Only the host knows the apps' names (Android's PackageManager): it sends r1d the list when it connects and
//! whenever one is installed, updated or removed (`PUT /api/android/apps`). r1d opens them itself, with
//! `cmd activity`, so an agent's tool learns whether it worked.
//!
//! **Their music** (R7): the host tells r1d what an Android app is playing (`PUT /api/android/media`, from its media
//! sessions), and does what's asked of it (`android_media` events): pause, resume, skip, and play something found by
//! a search in a music app ([`MUSIC`]), through the app's own media session or its public "play from search"
//! intent. Never by working its screen.

use serde::{Deserialize, Serialize};
use std::sync::RwLock;

/// Aurora Store (ADR-0013), a privileged app in r1os's image: Google Play's catalog without a Google account.
pub const STORE: &str = "com.aurora.store";
const STORE_ACTIVITY: &str = "com.aurora.store/.ComposeActivity";
/// The host itself, never one of "the Android apps".
const HOST: &str = "dev.r1os.host";

/// Music and podcast apps that play from a search ("play Daft Punk on Spotify"), as Google's assistant asks them to:
/// in order of preference when no app is named and none is playing.
pub const MUSIC: &[&str] = &[
    "com.spotify.music",
    "com.google.android.apps.youtube.music",
    "com.apple.android.music",
    "com.amazon.mp3",
    "deezer.android.app",
    "com.aspiro.tidal",
    "com.soundcloud.android",
    "com.pandora.android",
    "au.com.shiftyjelly.pocketcasts",
];

/// What an Android app is playing, as the host's media sessions say.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Playing {
    pub package: String,
    /// The app's name.
    pub app: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    /// Playing now, rather than paused.
    pub playing: bool,
}

/// An Android app with a launcher entry, as the host saw it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AndroidApp {
    pub package: String,
    pub name: String,
    /// Part of the image (Aurora, Files, Settings) rather than installed since.
    #[serde(default)]
    pub system: bool,
}

#[derive(Default)]
pub struct Android {
    apps: RwLock<Vec<AndroidApp>>,
    /// Android apps' notifications, as the host's listener sees them (newest last).
    notes: RwLock<Vec<Note>>,
    /// What an Android app is playing (or paused on), if anything.
    media: RwLock<Option<Playing>>,
}

/// An Android app's notification (Signal's message, a delivery update), as the host's listener read it. Kept in
/// memory only, for the agents (`list_notifications`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Note {
    /// Android's key for it: what opening or dismissing it names.
    pub key: String,
    pub package: String,
    /// The app's name.
    pub app: String,
    pub title: String,
    #[serde(default)]
    pub text: String,
    /// When it was posted (ms since the epoch).
    pub at: i64,
    /// It has a reply box (a messaging app's): `reply_notification` answers it.
    #[serde(default)]
    pub can_reply: bool,
}

/// The most notifications kept.
const NOTES_MAX: usize = 50;

impl Android {
    /// The host's latest list.
    pub fn set(&self, mut apps: Vec<AndroidApp>) {
        apps.retain(|a| a.package != HOST && valid_package(&a.package));
        apps.sort_by_key(|a| a.name.to_lowercase());
        apps.dedup_by(|a, b| a.package == b.package);
        *self.apps.write().unwrap() = apps;
    }

    pub fn list(&self) -> Vec<AndroidApp> {
        self.apps.read().unwrap().clone()
    }

    pub fn get(&self, package: &str) -> Option<AndroidApp> {
        self.apps.read().unwrap().iter().find(|a| a.package == package).cloned()
    }

    /// The host's listener: a notification posted (or updated), removed, or all of them afresh.
    pub fn note_posted(&self, n: Note) {
        let mut notes = self.notes.write().unwrap();
        notes.retain(|x| x.key != n.key);
        notes.push(n);
        let over = notes.len().saturating_sub(NOTES_MAX);
        notes.drain(..over);
    }

    pub fn note_removed(&self, key: &str) {
        self.notes.write().unwrap().retain(|x| x.key != key);
    }

    pub fn notes_reset(&self, mut all: Vec<Note>) {
        all.sort_by_key(|n| n.at);
        let over = all.len().saturating_sub(NOTES_MAX);
        all.drain(..over);
        *self.notes.write().unwrap() = all;
    }

    /// Newest first.
    pub fn notes(&self) -> Vec<Note> {
        self.notes.read().unwrap().iter().rev().cloned().collect()
    }

    pub fn note(&self, key: &str) -> Option<Note> {
        self.notes.read().unwrap().iter().find(|n| n.key == key).cloned()
    }

    /// The host's media sessions: what's playing now, or nothing.
    pub fn set_media(&self, m: Option<Playing>) {
        *self.media.write().unwrap() = m.filter(|m| valid_package(&m.package));
    }

    pub fn media(&self) -> Option<Playing> {
        self.media.read().unwrap().clone()
    }

    /// The music app to play from a search in: the one named (if it's installed and can), else the one playing (or
    /// paused), else the first of [`MUSIC`] installed.
    pub fn music_app(&self, named: Option<&str>) -> Option<AndroidApp> {
        if let Some(n) = named.filter(|n| !n.trim().is_empty()) {
            return self.named(n).filter(|a| MUSIC.contains(&a.package.as_str()));
        }
        if let Some(m) = self.media().and_then(|m| self.get(&m.package)).filter(|a| MUSIC.contains(&a.package.as_str())) {
            return Some(m);
        }
        MUSIC.iter().find_map(|p| self.get(p))
    }

    /// The installed app called `name` (its label, any case, ignoring spaces and dots: "pocket casts", "k9 mail").
    pub fn named(&self, name: &str) -> Option<AndroidApp> {
        let want = squash(name);
        if want.is_empty() {
            return None;
        }
        self.apps.read().unwrap().iter().find(|a| squash(&a.name) == want).cloned()
    }
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// A music app's link as what plays it at once: an open.spotify.com page as Spotify's own link with `:play` on the end
/// (Spotify starts it; a search from another app only shows results), a music.youtube.com link as it is. None for
/// anything else.
pub fn music_uri(link: &str) -> Option<String> {
    let link = link.trim();
    if let Some(rest) = link.strip_prefix("https://open.spotify.com/") {
        let rest = rest.split(['?', '#']).next().unwrap_or_default();
        // Localised pages put a language first: /intl-de/track/<id>.
        let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty() && !p.starts_with("intl-")).collect();
        let [kind, id] = parts[..] else { return None };
        let ok = ["track", "album", "playlist", "artist", "episode", "show"].contains(&kind) && !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric());
        return ok.then(|| format!("spotify:{kind}:{id}:play"));
    }
    let ok = link.starts_with("https://music.youtube.com/") && link.len() <= 300 && !link.chars().any(|c| c.is_whitespace() || c == '"' || c == '\'');
    ok.then(|| link.to_owned())
}

/// A Java package name (`org.example.app`): the only thing r1d puts in an intent for an app.
pub fn valid_package(p: &str) -> bool {
    let parts: Vec<&str> = p.split('.').collect();
    p.len() <= 255
        && parts.len() >= 2
        && parts.iter().all(|s| {
            s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// `cmd activity start` prints its errors (no such app, no such activity) and still exits 0.
async fn start(args: &[&str]) -> anyhow::Result<()> {
    anyhow::ensure!(cfg!(target_os = "android"), "not on Android");
    let mut all = vec!["activity", "start"];
    all.extend_from_slice(args);
    // `cmd activity` rather than `am`: `am` is a script around it, and a deadline kill must reach `cmd`.
    let o = crate::tool::run("cmd", &all, crate::tool::DEADLINE).await?;
    // "Error type 3" first, then the reason ("Error: Activity class {…} does not exist."): the last one says it.
    let said = format!("{}\n{}", o.stdout, o.stderr);
    if let Some(err) = said.lines().rfind(|l| l.starts_with("Error") || l.contains("Exception")) {
        anyhow::bail!("{}", err.trim());
    }
    anyhow::ensure!(o.ok, "{}", o.stderr.trim());
    Ok(())
}

/// Opens an installed app the way a launcher does: its launcher activity, bringing its task back if it has one.
pub async fn open(package: &str) -> anyhow::Result<()> {
    anyhow::ensure!(valid_package(package) && package != HOST, "not an app's package: {package}");
    anyhow::ensure!(cfg!(target_os = "android"), "not on Android");
    let o = crate::tool::run(
        "cmd",
        &["package", "resolve-activity", "--brief", "-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER", package],
        crate::tool::DEADLINE,
    )
    .await?;
    let component = o.stdout.lines().map(str::trim).rfind(|l| l.starts_with(&format!("{package}/")));
    let Some(component) = component else { anyhow::bail!("{package} isn't installed (or has nothing to open)") };
    // FLAG_ACTIVITY_NEW_TASK | FLAG_ACTIVITY_RESET_TASK_IF_NEEDED, as Launcher3 starts apps.
    start(&["-a", "android.intent.action.MAIN", "-c", "android.intent.category.LAUNCHER", "-n", component, "-f", "0x10200000"]).await
}

/// Opens a link full screen in the installed app that claims it (T-112): an app's own (`uber://?action=setPickup…`,
/// `spotify:…`) or a web address an app opens, for the user to carry on there. `package` keeps it to that app. Never a
/// link no app claims (r1os has no browser app for it to fall to), nor one several claim without `package`. Returns the
/// package that opened it.
pub async fn open_link(link: &str, package: Option<&str>) -> anyhow::Result<String> {
    crate::appwin::allowed_link(link).map_err(|e| anyhow::anyhow!(e))?;
    if let Some(p) = package {
        anyhow::ensure!(valid_package(p) && p != HOST, "not an app's package: {p}");
    }
    anyhow::ensure!(cfg!(target_os = "android"), "not on Android");
    let link = link.trim();
    let mut q = vec!["package", "resolve-activity", "--brief", "-a", "android.intent.action.VIEW", "-c", "android.intent.category.BROWSABLE", "-d", link];
    q.extend(package);
    let o = crate::tool::run("cmd", &q, crate::tool::DEADLINE).await?;
    let Some(component) = o.stdout.lines().map(str::trim).rfind(|l| l.contains('/') && !l.contains(' ')) else {
        anyhow::bail!("no installed app opens {link}")
    };
    let pkg = component.split('/').next().unwrap_or_default().to_owned();
    // Android's chooser: more than one app claims it.
    anyhow::ensure!(pkg != "android", "more than one app opens {link}: say which (package)");
    anyhow::ensure!(pkg != HOST, "that link is r1os's own");
    // FLAG_ACTIVITY_NEW_TASK, as a link tapped anywhere opens.
    start(&["-a", "android.intent.action.VIEW", "-c", "android.intent.category.BROWSABLE", "-d", link, "-n", component, "-f", "0x10000000"]).await?;
    Ok(pkg)
}

/// Aurora Store on an app's page (install, open or update it there), or on its home page with no package.
/// SHOW_APP_INFO rather than a market:// link: Aurora asks before following those (ads use them), not this.
pub async fn store(package: Option<&str>) -> anyhow::Result<()> {
    // FLAG_ACTIVITY_NEW_TASK | FLAG_ACTIVITY_CLEAR_TASK: a running Aurora shows the new page, not its last one.
    let mut args = vec!["-n", STORE_ACTIVITY, "-f", "0x10008000"];
    if let Some(p) = package {
        anyhow::ensure!(valid_package(p), "not an app's package: {p}");
        args.extend(["-a", "android.intent.action.SHOW_APP_INFO", "--es", "android.intent.extra.PACKAGE_NAME", p]);
    } else {
        args.extend(["-a", "android.intent.action.MAIN"]);
    }
    start(&args).await.map_err(|e| {
        if e.to_string().contains("does not exist") { anyhow::anyhow!("Aurora Store isn't installed on this R1") } else { e }
    })
}

/// Aurora may install apps: it holds INSTALL_PACKAGES (a privileged app in the image), but its first run also
/// wants Android's "install unknown apps" switch (the REQUEST_INSTALL_PACKAGES app-op), which only Android's
/// settings flip, so r1d does, as it grants the host its own (v0.2.11). Nothing to do without Aurora.
pub async fn grant_store() {
    if !cfg!(target_os = "android") {
        return;
    }
    match crate::tool::run("cmd", &["appops", "set", STORE, "REQUEST_INSTALL_PACKAGES", "allow"], crate::tool::DEADLINE).await {
        Ok(o) if o.ok => {}
        Ok(o) if format!("{}{}", o.stdout, o.stderr).contains("No UID") => {}
        Ok(o) => tracing::warn!(error = %o.stderr.trim(), "couldn't let Aurora install apps"),
        Err(e) => tracing::warn!(error = %e, "couldn't let Aurora install apps"),
    }
}

/// The host's notification listener (`NotificationListener`) may read Android apps' notifications. A fresh /data
/// grants it from the image (`config_defaultListenerAccessPackages`); an R1 set up before gets it here, at startup.
pub async fn allow_listener() {
    if !cfg!(target_os = "android") {
        return;
    }
    const LISTENER: &str = "dev.r1os.host/dev.r1os.host.NotificationListener";
    match crate::tool::run("cmd", &["notification", "allow_listener", LISTENER], crate::tool::DEADLINE).await {
        Ok(o) if o.ok => {}
        Ok(o) => tracing::warn!(error = %format!("{}{}", o.stdout, o.stderr).trim(), "couldn't let the host read notifications"),
        Err(e) => tracing::warn!(error = %e, "couldn't let the host read notifications"),
    }
}

/// The host compiled ahead of time (T-100), once: a fresh image's copy, or one adb installed without
/// `tools/deploy.sh`, runs interpreted until Android's idle dexopt otherwise. The image can't preopt it (a
/// privileged app's dex would be rewritten, and its signature with it). A few seconds, only when it isn't yet.
pub async fn compile_host() {
    if !cfg!(target_os = "android") {
        return;
    }
    let status = match crate::tool::run("dumpsys", &["package", "dev.r1os.host"], std::time::Duration::from_secs(8)).await {
        Ok(o) => o.stdout,
        Err(_) => return,
    };
    if status.contains("[status=speed]") || !status.contains("[status=") {
        return;
    }
    match crate::tool::run("cmd", &["package", "compile", "-m", "speed", "-f", "dev.r1os.host"], std::time::Duration::from_secs(120)).await {
        Ok(o) if o.ok => tracing::info!("the host: compiled ahead of time"),
        Ok(o) => tracing::warn!(error = %format!("{}{}", o.stdout, o.stderr).trim(), "couldn't compile the host"),
        Err(e) => tracing::warn!(error = %e, "couldn't compile the host"),
    }
}

/// A key for the Android app in front: the side button's click is DPAD center there (Q24).
pub async fn key(name: &str) -> anyhow::Result<()> {
    let code = match name {
        "center" => "23",
        "back" => "4",
        other => anyhow::bail!("unknown key '{other}'"),
    };
    anyhow::ensure!(cfg!(target_os = "android"), "not on Android");
    // `cmd input` rather than `input`: `input` is a script around it, and a deadline kill must reach `cmd`.
    let o = crate::tool::run("cmd", &["input", "keyevent", code], crate::tool::DEADLINE).await?;
    anyhow::ensure!(o.ok, "{}", o.stderr.trim());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(package: &str, name: &str) -> AndroidApp {
        AndroidApp { package: package.into(), name: name.into(), system: false }
    }

    fn note(key: &str, at: i64) -> Note {
        Note { key: key.into(), package: "org.example.chat".into(), app: "Chat".into(), title: key.into(), text: String::new(), at, can_reply: false }
    }

    #[test]
    fn notifications() {
        let a = Android::default();
        a.note_posted(note("a", 1));
        a.note_posted(note("b", 2));
        a.note_posted(Note { text: "again".into(), ..note("a", 3) });
        let keys: Vec<String> = a.notes().into_iter().map(|n| n.key).collect();
        assert_eq!(keys, ["a", "b"], "newest first; an update replaces it");
        assert_eq!(a.note("a").unwrap().text, "again");
        a.note_removed("a");
        assert!(a.note("a").is_none());
        a.notes_reset((0..80).map(|i| note(&format!("n{i}"), i)).collect());
        assert_eq!(a.notes().len(), NOTES_MAX, "the newest 50");
        assert_eq!(a.notes()[0].key, "n79");
    }

    #[test]
    fn the_music_app() {
        let a = Android::default();
        assert!(a.music_app(None).is_none(), "none installed");
        a.set(vec![app("com.spotify.music", "Spotify"), app("au.com.shiftyjelly.pocketcasts", "Pocket Casts"), app("org.example.notes", "Notes")]);
        assert_eq!(a.music_app(None).unwrap().package, "com.spotify.music", "the first of MUSIC");
        assert_eq!(a.music_app(Some("pocket casts")).unwrap().package, "au.com.shiftyjelly.pocketcasts");
        assert!(a.music_app(Some("notes")).is_none(), "not a music app");
        assert!(a.music_app(Some("tidal")).is_none(), "not installed");
        a.set_media(Some(Playing { package: "au.com.shiftyjelly.pocketcasts".into(), app: "Pocket Casts".into(), title: "".into(), artist: "".into(), playing: false }));
        assert_eq!(a.music_app(None).unwrap().package, "au.com.shiftyjelly.pocketcasts", "the one playing");
    }

    #[test]
    fn music_links() {
        assert_eq!(music_uri("https://open.spotify.com/playlist/37i9dQZF1DWWQRwui0ExPn?si=abc").as_deref(), Some("spotify:playlist:37i9dQZF1DWWQRwui0ExPn:play"));
        assert_eq!(music_uri("https://open.spotify.com/intl-de/artist/4tZwfgrHOc3mvqYlEYSvVi").as_deref(), Some("spotify:artist:4tZwfgrHOc3mvqYlEYSvVi:play"));
        assert!(music_uri("https://open.spotify.com/user/abc").is_none());
        assert!(music_uri("https://open.spotify.com/track/a;b").is_none());
        assert!(music_uri("https://evil.example/open.spotify.com/track/abc").is_none());
        assert_eq!(music_uri("https://music.youtube.com/watch?v=abc").as_deref(), Some("https://music.youtube.com/watch?v=abc"));
    }

    #[test]
    fn packages() {
        assert!(valid_package("com.aurora.store"));
        assert!(valid_package("org.thoughtcrime.securesms"));
        assert!(valid_package("com.k9mail_2.app"));
        assert!(!valid_package("spotify"), "one part");
        assert!(!valid_package("com..x"));
        assert!(!valid_package("com.9x"), "a part starts with a letter");
        assert!(!valid_package("com.x -n evil/.A"), "nothing an argument could hide in");
        assert!(!valid_package("com.x;rm"));
    }

    #[test]
    fn the_list_and_names() {
        let a = Android::default();
        a.set(vec![app("com.spotify.music", "Spotify"), app(HOST, "r1os"), app("bad pkg", "Bad"), app("au.com.shiftyjelly.pocketcasts", "Pocket Casts")]);
        let names: Vec<String> = a.list().into_iter().map(|x| x.name).collect();
        assert_eq!(names, ["Pocket Casts", "Spotify"], "sorted, without the host or a bad package");
        assert_eq!(a.named("spotify").unwrap().package, "com.spotify.music");
        assert_eq!(a.named("pocketcasts").unwrap().package, "au.com.shiftyjelly.pocketcasts");
        assert!(a.named("music").is_none(), "a whole name, not part of one");
        assert!(a.named("").is_none());
    }
}
