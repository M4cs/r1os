//! The device's look (ADR-0043): dark or light, the accent, the font, each screen's background, and how the glance
//! screen is laid out. Kept in core.db (`look`), changed by Settings › Look and the agents' `set_look`, and sent to
//! the shell (`look {look}`), which draws everything from it.
//!
//! A change names only what changes (`PUT /api/look`): it's merged into the look, object by object, then checked
//! whole. Every value has a default, today's look, so a look from an older r1d (or none) reads the same.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub const MODES: &[&str] = &["dark", "light"];
/// The app palette's accents (the host's `Theme.COLORS`, packages/runtime styles.css).
pub const ACCENTS: &[&str] =
    &["orange", "amber", "yellow", "lime", "green", "teal", "cyan", "blue", "indigo", "purple", "pink", "red"];
/// The shell's typefaces: Rubik (bundled, ADR-0021), and the R1's own system fonts (no downloads).
pub const FONTS: &[&str] = &["rubik", "roboto", "flex", "source", "serif", "mono", "typewriter", "casual", "script", "smallcaps"];
/// The glance screen's clock faces.
pub const CLOCKS: &[&str] = &["bold", "light", "condensed", "stacked", "serif", "mono", "words", "dial"];
/// What a screen's background is.
pub const KINDS: &[&str] = &["plain", "tint", "gradient", "mesh", "texture", "photo"];
/// Soft gradients' ready-made colours (Arc-like, ADR-0043's update): a name and its two or three colours.
pub const PALETTES: &[(&str, &[&str])] = &[
    ("sunset", &["#ff8a3d", "#f06aa8", "#6b4cff"]),
    ("lagoon", &["#2fc4a8", "#3cc8e0", "#4e6bff"]),
    ("aurora", &["#3ddc84", "#8b8cf8", "#1d4ed8"]),
    ("peach", &["#ffb38a", "#ff6f91", "#ffc75f"]),
    ("dusk", &["#4ea1ff", "#b07cf6", "#f06aa8"]),
    ("forest", &["#a3d65c", "#2f8f5b", "#f5b83d"]),
    ("berry", &["#b07cf6", "#f2555a", "#ff8a3d"]),
    ("ink", &["#3a4a6b", "#7a5c99", "#1f2a44"]),
];
pub const GRADIENTS: &[&str] = &["glow", "dusk", "diagonal", "horizon"];
pub const TEXTURES: &[&str] = &["grain", "noise"];
/// Where the glance screen's time and status sit.
pub const ALIGNS: &[&str] = &["left", "center"];
/// Where the agent peeks up on the glance screen.
pub const PLACES: &[&str] = &["center", "left", "right", "hidden"];
/// What the glance screen can leave out.
pub const PARTS: &[&str] = &["date", "status", "alarm", "notifications", "playing", "widgets", "greeting"];
/// A photo background is darkened (or, in light mode, lightened) by at most this much.
pub const MAX_DIM: f64 = 0.85;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Look {
    pub mode: String,
    pub accent: String,
    pub font: String,
    pub glance: GlanceLook,
    pub home: HomeLook,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GlanceLook {
    pub background: Background,
    pub clock: String,
    pub align: String,
    pub agent: String,
    /// Parts not shown ([`PARTS`]).
    pub hide: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct HomeLook {
    pub background: Background,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Background {
    pub kind: String,
    /// The colour it's made from (tint, gradient, texture): an accent's name or `#rrggbb`; none: the device's accent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// A soft gradient's colours (`mesh`): one to three, each an accent's name or `#rrggbb`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<String>,
    /// How much fine grain is over it: 0–1 (a soft gradient's, a gradient's, a tint's, a texture's).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grain: Option<f64>,
    /// It moves with the wheel and the button (a gradient's glows drift and settle back).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow: Option<bool>,
    /// The gradient's or texture's style.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    /// A photo's media id (crate::media).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo: Option<String>,
    /// How much a photo or a soft gradient is darkened (lightened in light mode) so text stays readable: 0–0.85.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dim: Option<f64>,
}

impl Default for Look {
    fn default() -> Self {
        Look { mode: "dark".into(), accent: "orange".into(), font: "rubik".into(), glance: GlanceLook::default(), home: HomeLook::default() }
    }
}

impl Default for GlanceLook {
    fn default() -> Self {
        GlanceLook { background: Background::default(), clock: "bold".into(), align: "left".into(), agent: "center".into(), hide: Vec::new() }
    }
}

impl Default for Background {
    fn default() -> Self {
        Background { kind: "plain".into(), color: None, colors: Vec::new(), grain: None, flow: None, style: None, photo: None, dim: None }
    }
}

/// A colour: one of the accents' names, or `#rrggbb` (lowercased).
pub fn colour(what: &str, v: &str) -> Result<String, String> {
    let v = v.trim().to_ascii_lowercase();
    let hex = v.len() == 7 && v.starts_with('#') && v[1..].bytes().all(|b| b.is_ascii_hexdigit());
    if ACCENTS.contains(&v.as_str()) || hex {
        Ok(v)
    } else {
        Err(format!("{what} is one of {}, or a colour like #ff8a3d (not `{v}`)", ACCENTS.join(", ")))
    }
}

fn one_of(what: &str, v: &str, of: &[&str]) -> Result<(), String> {
    if of.contains(&v) { Ok(()) } else { Err(format!("{what} is one of {} (not `{v}`)", of.join(", "))) }
}

impl Background {
    fn check(&mut self, at: &str, is_photo: &dyn Fn(&str) -> bool) -> Result<(), String> {
        one_of(&format!("{at}.kind"), &self.kind, KINDS)?;
        if let Some(c) = &self.color {
            self.color = Some(colour(&format!("{at}.color"), c)?);
        }
        // Only tints, gradients and textures are made from one colour; a soft gradient from its own.
        if matches!(self.kind.as_str(), "plain" | "photo" | "mesh") {
            self.color = None;
        }
        if self.kind == "mesh" {
            if self.colors.is_empty() {
                self.colors = PALETTES[0].1.iter().map(|c| (*c).to_owned()).collect();
            }
            if self.colors.len() > 3 {
                return Err(format!("{at}.colors is one to three colours"));
            }
            self.colors = self.colors.iter().map(|c| colour(&format!("{at}.colors"), c)).collect::<Result<_, _>>()?;
        } else {
            self.colors.clear();
        }
        // Grain over everything but plain and photos; flow for gradients.
        let grain_default = match self.kind.as_str() {
            "mesh" => Some(0.5),
            "texture" => Some(0.7),
            "gradient" => Some(0.3),
            "tint" => Some(0.0),
            _ => None,
        };
        self.grain = grain_default.map(|d| self.grain.unwrap_or(d));
        if let Some(g) = self.grain
            && !(0.0..=1.0).contains(&g)
        {
            return Err(format!("{at}.grain is 0–1"));
        }
        self.flow = matches!(self.kind.as_str(), "mesh" | "gradient").then(|| self.flow.unwrap_or(false));
        match self.kind.as_str() {
            "gradient" => one_of(&format!("{at}.style"), self.style.get_or_insert_with(|| "glow".into()), GRADIENTS)?,
            "texture" => one_of(&format!("{at}.style"), self.style.get_or_insert_with(|| "grain".into()), TEXTURES)?,
            _ => self.style = None,
        }
        if self.kind == "photo" {
            let id = self.photo.as_deref().ok_or_else(|| format!("{at}.photo is the photo's id (a photo background needs one)"))?;
            if !is_photo(id) {
                return Err(format!("{at}.photo: there's no photo `{id}`"));
            }
        } else {
            self.photo = None;
        }
        if matches!(self.kind.as_str(), "photo" | "mesh") {
            let dim = self.dim.unwrap_or(if self.kind == "photo" { 0.45 } else { 0.4 });
            if !(0.0..=MAX_DIM).contains(&dim) {
                return Err(format!("{at}.dim is 0–{MAX_DIM}"));
            }
            self.dim = Some(dim);
        } else {
            self.dim = None;
        }
        Ok(())
    }
}

impl Look {
    /// Checks every value, filling what a kind implies (a gradient's style, a photo's dim). `is_photo`: whether a
    /// media id is one of the R1's photos.
    pub fn check(&mut self, is_photo: &dyn Fn(&str) -> bool) -> Result<(), String> {
        one_of("mode", &self.mode, MODES)?;
        self.accent = colour("accent", &self.accent)?;
        one_of("font", &self.font, FONTS)?;
        one_of("glance.clock", &self.glance.clock, CLOCKS)?;
        one_of("glance.align", &self.glance.align, ALIGNS)?;
        one_of("glance.agent", &self.glance.agent, PLACES)?;
        for p in &self.glance.hide {
            one_of("glance.hide", p, PARTS)?;
        }
        self.glance.hide.sort();
        self.glance.hide.dedup();
        self.glance.background.check("glance.background", is_photo)?;
        self.home.background.check("home.background", is_photo)?;
        Ok(())
    }

    /// The look with `change` merged in (objects by key; `null` puts a value back to its default), checked.
    pub fn changed(&self, change: &Value, is_photo: &dyn Fn(&str) -> bool) -> Result<Look, String> {
        let mut v = serde_json::to_value(self).map_err(|e| e.to_string())?;
        merge(&mut v, change);
        let mut look: Look = serde_json::from_value(v).map_err(|e| format!("not a look: {e}"))?;
        look.check(is_photo)?;
        Ok(look)
    }
}

/// `patch` merged into `base`: objects key by key, anything else replaced; `null` removes the key.
pub fn merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                if v.is_null() {
                    b.remove(k);
                } else if let Some(slot) = b.get_mut(k) {
                    merge(slot, v);
                } else {
                    b.insert(k.clone(), v.clone());
                }
            }
        }
        (b, p) => *b = p.clone(),
    }
}

/// Everything a look can be, for Settings › Look and the agents' tool.
pub fn options() -> Value {
    json!({
        "modes": MODES, "accents": ACCENTS, "fonts": FONTS, "clocks": CLOCKS, "kinds": KINDS, "gradients": GRADIENTS,
        "textures": TEXTURES, "aligns": ALIGNS, "places": PLACES, "parts": PARTS, "max_dim": MAX_DIM,
        "palettes": PALETTES.iter().map(|(n, c)| json!({ "name": n, "colors": c })).collect::<Vec<_>>(),
    })
}

/// The look in a few words, for an agent.
pub fn summary(l: &Look) -> String {
    let bg = |b: &Background| -> String {
        let mut s = b.kind.clone();
        if let Some(st) = &b.style {
            s = format!("{st} {s}");
        }
        if let Some(c) = &b.color {
            s = format!("{s} in {c}");
        }
        if !b.colors.is_empty() {
            s = format!("soft gradient of {}", b.colors.join(", "));
        }
        if b.flow == Some(true) {
            s = format!("{s}, moving with the wheel");
        }
        s
    };
    let mut out = Map::new();
    out.insert("mode".into(), json!(l.mode));
    out.insert("accent".into(), json!(l.accent));
    out.insert("font".into(), json!(l.font));
    out.insert("glance".into(), json!(format!(
        "{} clock on the {}, agent {}, {} background{}",
        l.glance.clock,
        l.glance.align,
        l.glance.agent,
        bg(&l.glance.background),
        if l.glance.hide.is_empty() { String::new() } else { format!(", hiding {}", l.glance.hide.join(", ")) }
    )));
    out.insert("home".into(), json!(format!("{} background", bg(&l.home.background))));
    Value::Object(out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_merges_and_is_checked() {
        let none = |_: &str| false;
        let l = Look::default().changed(&json!({ "accent": "purple", "glance": { "clock": "dial" } }), &none).unwrap();
        assert_eq!((l.accent.as_str(), l.glance.clock.as_str(), l.glance.align.as_str()), ("purple", "dial", "left"));
        assert!(Look::default().changed(&json!({ "font": "comic" }), &none).unwrap_err().contains("font"));
        // A gradient gets its default style; a null puts a value back.
        let g = l.changed(&json!({ "home": { "background": { "kind": "gradient" } }, "accent": null }), &none).unwrap();
        assert_eq!(g.home.background.style.as_deref(), Some("glow"));
        assert_eq!(g.accent, "orange");
        // A photo must be one.
        assert!(g.changed(&json!({ "glance": { "background": { "kind": "photo", "photo": "x" } } }), &none).is_err());
        let p = g.changed(&json!({ "glance": { "background": { "kind": "photo", "photo": "x" } } }), &|id| id == "x").unwrap();
        assert_eq!(p.glance.background.dim, Some(0.45));
        // Moving off a photo forgets it; plain forgets a colour.
        let back = p.changed(&json!({ "glance": { "background": { "kind": "tint", "color": "blue" } } }), &none).unwrap();
        assert_eq!(back.glance.background.photo, None);
        let plain = back.changed(&json!({ "glance": { "background": { "kind": "plain" } } }), &none).unwrap();
        assert_eq!(plain.glance.background.color, None);
    }

    #[test]
    fn soft_gradients_take_their_own_colours() {
        let none = |_: &str| false;
        let m = Look::default().changed(&json!({ "glance": { "background": { "kind": "mesh" } } }), &none).unwrap();
        let b = &m.glance.background;
        assert_eq!(b.colors.len(), 3, "a palette by default");
        assert_eq!((b.grain, b.flow, b.dim), (Some(0.5), Some(false), Some(0.4)));
        let c = m.changed(&json!({ "glance": { "background": { "colors": ["#FF0080", "teal"], "flow": true, "grain": 0.2 } } }), &none).unwrap();
        assert_eq!(c.glance.background.colors, vec!["#ff0080", "teal"]);
        assert!(m.changed(&json!({ "glance": { "background": { "colors": ["#ff00"] } } }), &none).is_err());
        assert!(m.changed(&json!({ "glance": { "background": { "colors": ["red", "red", "red", "red"] } } }), &none).is_err());
        // A custom accent.
        assert_eq!(Look::default().changed(&json!({ "accent": "#1A2B3C" }), &none).unwrap().accent, "#1a2b3c");
        // Back to plain: none of it stays.
        let p = c.changed(&json!({ "glance": { "background": { "kind": "plain" } } }), &none).unwrap();
        assert_eq!(p.glance.background, Background::default());
    }

    #[test]
    fn an_old_look_reads_as_today() {
        let l: Look = serde_json::from_value(json!({ "accent": "teal" })).unwrap();
        assert_eq!(l.glance, GlanceLook::default());
        assert_eq!(l.mode, "dark");
    }
}
