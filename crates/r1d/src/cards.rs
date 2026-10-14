//! Cards in chats (ADR-0039): what an agent shows with its answer, beyond words. Headlines with a picture and a
//! link, today's weather, a few numbers, a small chart, the steps of something. The idea is json-render's: a
//! catalog of components the model fills in, checked before anything renders. Here the catalog is flat (one card
//! is one component), checked in r1d, and drawn natively by the host, as the rest of the chat is.
//!
//! [`check`] takes what the agent sent and returns the card as it's kept (strings trimmed and cut, unknown fields
//! dropped, links and pictures checked), or what's wrong in words the agent can fix. [`summary`] is its text, for
//! wherever a card can't be drawn (the chat list's preview, the reply card over an Android app).

use serde_json::{Map, Value, json};

/// The catalog. The agent's tool lists these, with their fields (crate::mcp).
pub const TYPES: &[&str] = &["headlines", "link", "weather", "image", "stats", "bars", "line", "steps", "table"];

/// The weather icons the host draws (Phosphor names).
pub const WEATHER_ICONS: &[&str] = &[
    "sun",
    "moon",
    "cloud-sun",
    "cloud-moon",
    "cloud",
    "cloud-rain",
    "cloud-snow",
    "cloud-lightning",
    "cloud-fog",
    "wind",
    "snowflake",
    "drop",
    "thermometer",
];

/// A card as JSON, at most this long (kept in the chat's messages).
const MAX_BYTES: usize = 16 * 1024;

type Out = Map<String, Value>;

/// The card as it's kept, or what's wrong with it.
pub fn check(card: &Value) -> Result<Value, String> {
    let c = card.as_object().ok_or("a card is an object with a `type`")?;
    let kind = c.get("type").and_then(Value::as_str).unwrap_or_default();
    let mut out = Out::new();
    out.insert("type".into(), json!(kind));
    let f = Fields { c, path: String::new() };
    match kind {
        "headlines" => {
            f.text_opt(&mut out, "title", 60);
            f.list(&mut out, "items", 1, 8, |i, o| {
                i.text(o, "title", 160)?;
                i.text_opt(o, "tagline", 220);
                i.text_opt(o, "source", 40);
                i.text_opt(o, "time", 30);
                i.link_opt(o, "url")?;
                i.image_opt(o, "image")?;
                Ok(())
            })?;
        }
        "link" => {
            f.text(&mut out, "title", 160)?;
            f.link(&mut out, "url")?;
            f.text_opt(&mut out, "source", 40);
            f.text_opt(&mut out, "summary", 400);
            f.image_opt(&mut out, "image")?;
        }
        "weather" => {
            f.text(&mut out, "place", 60)?;
            f.number(&mut out, "temp")?;
            f.text(&mut out, "condition", 40)?;
            let unit = c.get("unit").and_then(Value::as_str).unwrap_or("C").to_ascii_uppercase();
            if unit != "C" && unit != "F" {
                return Err("weather.unit is \"C\" or \"F\"".into());
            }
            out.insert("unit".into(), json!(unit));
            out.insert("icon".into(), json!(weather_icon(c.get("icon"), c.get("condition"))));
            for k in ["high", "low", "feels"] {
                f.number_opt(&mut out, k)?;
            }
            f.text_opt(&mut out, "note", 120);
            f.list_opt(&mut out, "hours", 0, 8, |i, o| {
                i.text(o, "time", 10)?;
                i.number(o, "temp")?;
                o.insert("icon".into(), json!(weather_icon(i.c.get("icon"), i.c.get("condition"))));
                Ok(())
            })?;
            f.list_opt(&mut out, "days", 0, 7, |i, o| {
                i.text(o, "day", 10)?;
                i.number(o, "high")?;
                i.number(o, "low")?;
                o.insert("icon".into(), json!(weather_icon(i.c.get("icon"), i.c.get("condition"))));
                Ok(())
            })?;
        }
        "image" => {
            f.image(&mut out, "url")?;
            f.text_opt(&mut out, "caption", 160);
            f.text_opt(&mut out, "source", 40);
            f.link_opt(&mut out, "link")?;
        }
        "stats" => {
            f.text_opt(&mut out, "title", 60);
            f.list(&mut out, "items", 1, 4, |i, o| {
                i.text(o, "label", 40)?;
                i.text_or_number(o, "value", 24)?;
                i.text_opt(o, "delta", 16);
                if let Some(t) = i.c.get("trend").and_then(Value::as_str) {
                    if !matches!(t, "up" | "down" | "flat") {
                        return Err(format!("{}trend is up, down or flat", i.path));
                    }
                    o.insert("trend".into(), json!(t));
                }
                Ok(())
            })?;
        }
        "bars" | "line" => {
            f.text_opt(&mut out, "title", 60);
            f.text_opt(&mut out, "unit", 12);
            let (key, min, max) = if kind == "bars" { ("items", 1, 10) } else { ("points", 2, 40) };
            f.list(&mut out, key, min, max, |i, o| {
                i.text(o, "label", 30)?;
                i.number(o, "value")?;
                Ok(())
            })?;
            if kind == "bars" && out["items"].as_array().is_some_and(|a| a.iter().any(|x| x["value"].as_f64() < Some(0.0))) {
                return Err("bars' values are 0 or more (a line takes any)".into());
            }
        }
        "steps" => {
            f.text_opt(&mut out, "title", 60);
            f.list(&mut out, "items", 2, 8, |i, o| {
                i.text(o, "title", 60)?;
                i.text_opt(o, "detail", 140);
                Ok(())
            })?;
        }
        "table" => {
            f.text_opt(&mut out, "title", 60);
            let columns = strings(c.get("columns"), 20).ok_or("table.columns is a list of 1–3 headings")?;
            if !(1..=3).contains(&columns.len()) {
                return Err("table.columns is a list of 1–3 headings".into());
            }
            let rows = c.get("rows").and_then(Value::as_array).ok_or("table.rows is a list of rows, each a list of cells")?;
            if !(1..=10).contains(&rows.len()) {
                return Err("table.rows has 1–10 rows".into());
            }
            let mut kept = Vec::new();
            for (n, r) in rows.iter().enumerate() {
                let cells = strings(Some(r), 40).filter(|c| c.len() == columns.len());
                kept.push(cells.ok_or_else(|| format!("table.rows[{n}] has {} cells (one per column)", columns.len()))?);
            }
            out.insert("columns".into(), json!(columns));
            out.insert("rows".into(), json!(kept));
        }
        "" => return Err(format!("a card needs a `type`: one of {}", TYPES.join(", "))),
        other => return Err(format!("there's no `{other}` card: the types are {}", TYPES.join(", "))),
    }
    let card = Value::Object(out);
    if card.to_string().len() > MAX_BYTES {
        return Err(format!("the card is over {} KB: fewer or shorter items", MAX_BYTES / 1024));
    }
    Ok(card)
}

/// A card in words: wherever it can't be drawn.
pub fn summary(card: &Value) -> String {
    let s = |k: &str| card[k].as_str().unwrap_or_default().to_owned();
    let titles = |key: &str, field: &str| -> String {
        card[key].as_array().map(|a| a.iter().filter_map(|i| i[field].as_str()).collect::<Vec<_>>().join(" · ")).unwrap_or_default()
    };
    let lead = |t: String, rest: String| if t.is_empty() { rest } else { format!("{t}: {rest}") };
    match card["type"].as_str().unwrap_or_default() {
        "headlines" => lead(s("title"), titles("items", "title")),
        "link" => s("title"),
        "weather" => {
            let unit = s("unit");
            let temp = card["temp"].as_f64().map(|t| format!("{}°{unit}", t.round())).unwrap_or_default();
            format!("{}: {temp}, {}", s("place"), s("condition").to_lowercase())
        }
        "image" => {
            let c = s("caption");
            if c.is_empty() { "A picture".into() } else { c }
        }
        "stats" => lead(
            s("title"),
            card["items"]
                .as_array()
                .map(|a| a.iter().map(|i| format!("{} {}", i["label"].as_str().unwrap_or_default(), value_text(&i["value"]))).collect::<Vec<_>>().join(" · "))
                .unwrap_or_default(),
        ),
        "bars" => lead(s("title"), titles("items", "label")),
        "line" => {
            let t = s("title");
            if t.is_empty() { "A chart".into() } else { t }
        }
        "steps" => lead(s("title"), titles("items", "title")),
        "table" => {
            let t = s("title");
            if t.is_empty() { "A table".into() } else { t }
        }
        _ => String::new(),
    }
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// The icon a weather card shows: the agent's, if it's one the host draws, else read from the condition's words.
fn weather_icon(icon: Option<&Value>, condition: Option<&Value>) -> &'static str {
    if let Some(i) = icon.and_then(Value::as_str)
        && let Some(known) = WEATHER_ICONS.iter().find(|k| **k == i)
    {
        return known;
    }
    let c = condition.and_then(Value::as_str).unwrap_or_default().to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| c.contains(w));
    if has(&["thunder", "storm", "lightning"]) {
        "cloud-lightning"
    } else if has(&["snow", "sleet", "blizzard", "flurr"]) {
        "cloud-snow"
    } else if has(&["rain", "shower", "drizzle"]) {
        "cloud-rain"
    } else if has(&["fog", "mist", "haze", "smoke"]) {
        "cloud-fog"
    } else if has(&["wind", "breez", "gust"]) {
        "wind"
    } else if has(&["partly", "partial", "few clouds", "scattered", "broken"]) {
        "cloud-sun"
    } else if has(&["cloud", "overcast"]) {
        "cloud"
    } else if has(&["clear night", "night"]) {
        "moon"
    } else {
        "sun"
    }
}

fn strings(v: Option<&Value>, max: usize) -> Option<Vec<String>> {
    v?.as_array()?.iter().map(|x| match x {
        Value::String(s) => Some(cut(s, max)),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }).collect()
}

/// Trimmed, one line, at most `max` characters (with an ellipsis when cut).
fn cut(s: &str, max: usize) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut kept: String = one.chars().take(max.saturating_sub(1)).collect();
    kept = kept.trim_end().to_owned();
    kept.push('…');
    kept
}

/// A picture: an https URL on the web (r1d fetches it for the host, [`crate::fetch`]).
pub fn image_ok(raw: &str) -> Result<String, String> {
    let url = crate::fetch::link_ok(raw)?;
    if !url.starts_with("https://") {
        return Err("pictures need an https URL".into());
    }
    Ok(url)
}

/// The pictures of stories `read_feed` returned lately, by link: a headline the agent shows without one gets its
/// story's ([`fill_pictures`]). Models tend to leave long image URLs out; this way they needn't copy them.
static RECENT: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());
const RECENT_MAX: usize = 300;

/// `read_feed`'s stories' pictures, for [`fill_pictures`].
pub fn remember_pictures(items: &[crate::feed::Item]) {
    let mut recent = RECENT.lock().unwrap();
    for i in items {
        if let (Some(link), Some(image)) = (&i.link, &i.image) {
            recent.retain(|(l, _)| l != link);
            recent.push((link.clone(), image.clone()));
        }
    }
    let over = recent.len().saturating_sub(RECENT_MAX);
    recent.drain(..over);
}

/// Headlines without a picture get their story's, when `read_feed` saw it.
pub fn fill_pictures(card: &mut Value) {
    let Some(items) = card.get_mut("items").and_then(Value::as_array_mut) else { return };
    let recent = RECENT.lock().unwrap();
    for item in items {
        if item.get("image").is_some() {
            continue;
        }
        let Some(url) = item.get("url").and_then(Value::as_str) else { continue };
        if let Some((_, image)) = recent.iter().rev().find(|(l, _)| l == url) {
            item["image"] = json!(image);
        }
    }
}

/// Pictures kept for cards, at most this many (the oldest go first).
const CACHE_FILES: usize = 300;

/// A card's picture for the host (`GET /api/cards/image`): fetched once by r1d's fetcher (https, public addresses
/// only, 5 MB at most, so a card can't make the host reach the R1's own network), then kept in `<data>/cache/cards`.
/// Blocking. The bytes and their type.
pub fn image(data: &std::path::Path, fetcher: &crate::fetch::Fetcher, url: &str) -> Result<(Vec<u8>, &'static str), String> {
    use std::hash::{Hash, Hasher};
    let url = image_ok(url)?;
    let key = {
        let mut a = std::hash::DefaultHasher::new();
        url.hash(&mut a);
        let mut b = std::hash::DefaultHasher::new();
        (url.len(), url.chars().rev().collect::<String>()).hash(&mut b);
        format!("{:016x}{:016x}", a.finish(), b.finish())
    };
    let dir = data.join("cache").join("cards");
    let path = dir.join(&key);
    if let Ok(bytes) = std::fs::read(&path)
        && let Some(kind) = image_kind(&bytes)
    {
        return Ok((bytes, kind));
    }
    let r = fetcher.get(&smaller(&url), crate::fetch::Format::Image, &|_| true).map_err(|e| e.to_string())?;
    if !(200..300).contains(&r.status) {
        return Err(format!("the picture answered HTTP {}", r.status));
    }
    let kind = image_kind(&r.body).ok_or("that isn't a picture the R1 shows (JPEG, PNG, WebP or GIF)")?;
    if std::fs::create_dir_all(&dir).is_ok() && std::fs::write(&path, &r.body).is_ok() {
        trim_cache(&dir);
    }
    Ok((r.body, kind))
}

/// The picture at a size for the R1, where the image's CDN takes one in its URL: Brightspot's (NPR's, and many news
/// sites') `/dims3/…/resize/WxH!/` gives the original, 10 MB for some of NPR's (2026-10-10); `/resize/800x/` 200 KB.
fn smaller(url: &str) -> String {
    let Ok(mut u) = url::Url::parse(url) else { return url.to_owned() };
    if !u.host_str().is_some_and(|h| h.ends_with("brightspotcdn.com")) || !u.path().contains("/dims3/") {
        return url.to_owned();
    }
    let path: Vec<&str> = u.path().split('/').collect();
    let Some(at) = path.iter().position(|s| *s == "resize") else { return url.to_owned() };
    let mut out: Vec<String> = path.iter().map(|s| (*s).to_owned()).collect();
    if let Some(size) = out.get_mut(at + 1) {
        *size = "800x".into();
    }
    u.set_path(&out.join("/"));
    u.into()
}

/// What the bytes are, by their first few.
fn image_kind(b: &[u8]) -> Option<&'static str> {
    match b {
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    }
}

fn trim_cache(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> =
        rd.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    if files.len() <= CACHE_FILES {
        return;
    }
    files.sort();
    for (_, p) in files.iter().take(files.len() - CACHE_FILES) {
        let _ = std::fs::remove_file(p);
    }
}

/// One object's fields, read into the card as it's kept; `path` names it in errors (`items[2].`).
struct Fields<'a> {
    c: &'a Map<String, Value>,
    path: String,
}

impl Fields<'_> {
    fn text(&self, out: &mut Out, k: &str, max: usize) -> Result<(), String> {
        match self.c.get(k).and_then(Value::as_str).map(|s| cut(s, max)).filter(|s| !s.is_empty()) {
            Some(s) => {
                out.insert(k.into(), json!(s));
                Ok(())
            }
            None => Err(format!("{}{k} is needed (text)", self.path)),
        }
    }

    fn text_opt(&self, out: &mut Out, k: &str, max: usize) {
        if let Some(s) = self.c.get(k).and_then(Value::as_str).map(|s| cut(s, max)).filter(|s| !s.is_empty()) {
            out.insert(k.into(), json!(s));
        }
    }

    fn text_or_number(&self, out: &mut Out, k: &str, max: usize) -> Result<(), String> {
        match self.c.get(k) {
            Some(Value::Number(n)) => {
                out.insert(k.into(), json!(n));
                Ok(())
            }
            Some(Value::String(s)) if !s.trim().is_empty() => {
                out.insert(k.into(), json!(cut(s, max)));
                Ok(())
            }
            _ => Err(format!("{}{k} is needed (a number or short text)", self.path)),
        }
    }

    fn number(&self, out: &mut Out, k: &str) -> Result<(), String> {
        match self.c.get(k).and_then(Value::as_f64).filter(|n| n.is_finite()) {
            Some(n) => {
                out.insert(k.into(), json!(n));
                Ok(())
            }
            None => Err(format!("{}{k} is needed (a number)", self.path)),
        }
    }

    fn number_opt(&self, out: &mut Out, k: &str) -> Result<(), String> {
        match self.c.get(k) {
            None | Some(Value::Null) => Ok(()),
            Some(_) => self.number(out, k),
        }
    }

    fn link(&self, out: &mut Out, k: &str) -> Result<(), String> {
        let raw = self.c.get(k).and_then(Value::as_str).ok_or_else(|| format!("{}{k} is needed (a link)", self.path))?;
        let url = crate::fetch::link_ok(raw).map_err(|e| format!("{}{k}: {e}", self.path))?;
        out.insert(k.into(), json!(url));
        Ok(())
    }

    fn link_opt(&self, out: &mut Out, k: &str) -> Result<(), String> {
        match self.c.get(k).and_then(Value::as_str) {
            Some(s) if !s.trim().is_empty() => self.link(out, k),
            _ => Ok(()),
        }
    }

    fn image(&self, out: &mut Out, k: &str) -> Result<(), String> {
        let raw = self.c.get(k).and_then(Value::as_str).ok_or_else(|| format!("{}{k} is needed (a picture's https URL)", self.path))?;
        let url = image_ok(raw).map_err(|e| format!("{}{k}: {e}", self.path))?;
        out.insert(k.into(), json!(url));
        Ok(())
    }

    /// A picture that won't load is no reason to lose the card: a bad one is left out.
    fn image_opt(&self, out: &mut Out, k: &str) -> Result<(), String> {
        if let Some(url) = self.c.get(k).and_then(Value::as_str).and_then(|s| image_ok(s).ok()) {
            out.insert(k.into(), json!(url));
        }
        Ok(())
    }

    fn list_opt(&self, out: &mut Out, k: &str, min: usize, max: usize, each: impl Fn(&Fields, &mut Out) -> Result<(), String>) -> Result<(), String> {
        match self.c.get(k) {
            None | Some(Value::Null) => Ok(()),
            Some(_) => self.list(out, k, min, max, each),
        }
    }

    fn list(&self, out: &mut Out, k: &str, min: usize, max: usize, each: impl Fn(&Fields, &mut Out) -> Result<(), String>) -> Result<(), String> {
        let items = self.c.get(k).and_then(Value::as_array).ok_or_else(|| format!("{}{k} is needed (a list of {min}–{max})", self.path))?;
        if items.len() < min {
            return Err(format!("{}{k} needs at least {min}", self.path));
        }
        let mut kept = Vec::new();
        for (n, item) in items.iter().take(max).enumerate() {
            let obj = item.as_object().ok_or_else(|| format!("{}{k}[{n}] is an object", self.path))?;
            let f = Fields { c: obj, path: format!("{}{k}[{n}].", self.path) };
            let mut o = Out::new();
            each(&f, &mut o)?;
            kept.push(Value::Object(o));
        }
        out.insert(k.into(), json!(kept));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headlines() {
        let card = check(&json!({
            "type": "headlines",
            "title": "Top news",
            "items": [
                { "title": "  A big   thing happened ", "tagline": "Why it matters", "source": "NPR",
                  "url": "https://www.npr.org/a", "image": "https://media.npr.org/a.jpg", "extra": 1 },
                { "title": "Another", "url": "http://192.168.1.2/x", "image": "http://insecure.example/x.jpg" }
            ]
        }));
        let err = card.unwrap_err();
        assert!(err.contains("items[1].url"), "a link on the LAN is refused: {err}");

        let card = check(&json!({
            "type": "headlines",
            "items": [{ "title": "  A big   thing happened ", "url": "https://www.npr.org/a", "image": "http://insecure.example/x.jpg", "extra": 1 }]
        }))
        .unwrap();
        assert_eq!(card["items"][0]["title"], "A big thing happened", "trimmed to one line");
        assert!(card["items"][0].get("image").is_none(), "an http picture is left out, not an error");
        assert!(card["items"][0].get("extra").is_none(), "unknown fields are dropped");
        assert_eq!(summary(&card), "A big thing happened");
    }

    #[test]
    fn smaller_pictures() {
        let npr = "https://npr.brightspotcdn.com/dims3/default/strip/false/crop/5472x3648+0+0/resize/5472x3648!/?url=http%3A%2F%2Fnpr-brightspot.s3.amazonaws.com%2Ff0%2Fa.jpg";
        assert_eq!(
            smaller(npr),
            "https://npr.brightspotcdn.com/dims3/default/strip/false/crop/5472x3648+0+0/resize/800x/?url=http%3A%2F%2Fnpr-brightspot.s3.amazonaws.com%2Ff0%2Fa.jpg"
        );
        assert_eq!(smaller("https://ichef.bbci.co.uk/ace/standard/240/a.jpg"), "https://ichef.bbci.co.uk/ace/standard/240/a.jpg");
    }

    #[test]
    fn pictures_from_the_feed() {
        let story = crate::feed::Item {
            link: Some("https://www.npr.org/story-1".into()),
            image: Some("https://media.npr.org/1.jpg".into()),
            ..Default::default()
        };
        remember_pictures(&[story]);
        let mut card = check(&json!({ "type": "headlines", "items": [
            { "title": "One", "url": "https://www.npr.org/story-1" },
            { "title": "Two", "url": "https://www.npr.org/story-2" },
            { "title": "Three", "url": "https://www.npr.org/story-1", "image": "https://other.example/x.png" }
        ] }))
        .unwrap();
        fill_pictures(&mut card);
        assert_eq!(card["items"][0]["image"], "https://media.npr.org/1.jpg");
        assert!(card["items"][1].get("image").is_none());
        assert_eq!(card["items"][2]["image"], "https://other.example/x.png", "the agent's own picture stays");
    }

    #[test]
    fn weather() {
        let card = check(&json!({
            "type": "weather", "place": "Brooklyn", "temp": 18.4, "condition": "Light rain showers", "unit": "f",
            "high": 21, "low": 12, "hours": [{ "time": "3pm", "temp": 19, "condition": "Partly cloudy" }]
        }))
        .unwrap();
        assert_eq!(card["unit"], "F");
        assert_eq!(card["icon"], "cloud-rain", "from the condition's words");
        assert_eq!(card["hours"][0]["icon"], "cloud-sun");
        assert_eq!(summary(&card), "Brooklyn: 18°F, light rain showers");
        assert!(check(&json!({ "type": "weather", "place": "X", "condition": "Sunny" })).unwrap_err().contains("temp"));
        assert!(check(&json!({ "type": "weather", "place": "X", "temp": 3, "condition": "Sunny", "unit": "K" })).is_err());
    }

    #[test]
    fn the_rest() {
        assert!(check(&json!({ "type": "bars", "items": [{ "label": "a", "value": -1 }] })).is_err());
        assert!(check(&json!({ "type": "line", "points": [{ "label": "a", "value": 1 }] })).unwrap_err().contains("at least 2"));
        let t = check(&json!({ "type": "table", "columns": ["Phone", "Price"], "rows": [["R1", 199], ["Other", "n/a"]] })).unwrap();
        assert_eq!(t["rows"][0][1], "199");
        assert!(check(&json!({ "type": "table", "columns": ["a"], "rows": [["x", "y"]] })).is_err());
        let s = check(&json!({ "type": "stats", "items": [{ "label": "BTC", "value": 61234.5, "delta": "+2%", "trend": "up" }] })).unwrap();
        assert_eq!(summary(&s), "BTC 61234.5");
        assert!(check(&json!({ "type": "stats", "items": [{ "label": "x", "value": 1, "trend": "sideways" }] })).is_err());
        assert!(check(&json!({ "type": "image", "url": "http://x.example/a.png" })).is_err(), "pictures are https");
        assert!(check(&json!({ "type": "poster" })).unwrap_err().contains("headlines"));
        assert!(check(&json!({ "title": "no type" })).unwrap_err().contains("type"));
        let long = "word ".repeat(100);
        let l = check(&json!({ "type": "link", "title": long, "url": "https://example.com" })).unwrap();
        assert!(l["title"].as_str().unwrap().chars().count() <= 160 && l["title"].as_str().unwrap().ends_with('…'));
    }
}
