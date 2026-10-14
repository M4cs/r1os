//! The wardrobe (T-062): what agents' eyes can be and what they can wear, from the catalog the shell and the R1's host
//! draw (`shell/src/ui/wardrobe/wardrobe.json`, docs/specs/experience.md §3.1). r1d only needs its names: which eyes
//! and items there are, and the slot each item is worn in (at most one per slot).

use serde::Deserialize;
use std::sync::LazyLock;

const CATALOG: &str = include_str!("../../../shell/src/ui/wardrobe/wardrobe.json");

pub struct Wardrobe {
    /// The eyes an agent can have, in the designer's order (not the reactions' faces).
    pub eyes: Vec<String>,
    /// Where things are worn, in order, each with its items.
    pub slots: Vec<Slot>,
}

pub struct Slot {
    /// "On its head", "At its neck".
    pub label: String,
    pub items: Vec<String>,
}

#[derive(Deserialize)]
struct Raw {
    slots: Vec<RawSlot>,
    eyes: Vec<RawEye>,
    items: Vec<RawItem>,
}

#[derive(Deserialize)]
struct RawSlot {
    id: String,
    label: String,
}

#[derive(Deserialize)]
struct RawEye {
    id: String,
    #[serde(default)]
    hidden: bool,
}

#[derive(Deserialize)]
struct RawItem {
    id: String,
    slot: String,
}

static WARDROBE: LazyLock<Wardrobe> = LazyLock::new(|| {
    let raw: Raw = serde_json::from_str(CATALOG).expect("the wardrobe catalog");
    Wardrobe {
        eyes: raw.eyes.into_iter().filter(|e| !e.hidden).map(|e| e.id).collect(),
        slots: raw
            .slots
            .into_iter()
            .map(|s| Slot { items: raw.items.iter().filter(|i| i.slot == s.id).map(|i| i.id.clone()).collect(), label: s.label })
            .collect(),
    }
});

pub fn get() -> &'static Wardrobe {
    &WARDROBE
}

impl Wardrobe {
    /// The slot an item is worn in, if it's one of the wardrobe's.
    pub fn slot_of(&self, item: &str) -> Option<&Slot> {
        self.slots.iter().find(|s| s.items.iter().any(|i| i == item))
    }

    /// Every slot and its items, for a prompt or an error: "On its head: beanie, cap…. On its face: …".
    pub fn describe(&self) -> String {
        self.slots.iter().map(|s| format!("{}: {}.", s.label, s.items.join(", "))).collect::<Vec<_>>().join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_reads_and_its_names_are_unique() {
        let w = get();
        assert!(w.eyes.iter().any(|e| e == "classic"), "classic eyes are the default");
        let mut seen = std::collections::HashSet::new();
        for s in &w.slots {
            assert!(!s.items.is_empty(), "{} has nothing to wear", s.label);
            for i in &s.items {
                assert!(seen.insert(i.clone()), "{i} is in two slots");
            }
        }
        // Everything agents wore before the wardrobe still exists (ADR-0043's fifteen).
        for old in ["beanie", "cap", "crown", "party", "tophat", "bow", "headphones", "flower", "glasses", "shades", "monocle", "blush", "bowtie", "scarf", "bell"] {
            assert!(w.slot_of(old).is_some(), "{old} went missing");
        }
        for old in ["classic", "dots", "sparkle", "happy", "sleepy", "lashes", "cyclops", "visor"] {
            assert!(w.eyes.iter().any(|e| e == old), "{old} eyes went missing");
        }
    }

    #[test]
    fn an_agent_wears_one_thing_per_slot() {
        let mut a = crate::core::Agent { wear: vec!["crown".into(), "glasses".into(), "blush".into(), "wings".into(), "crown".into()], ..Default::default() };
        a.validate().unwrap();
        assert_eq!(a.wear, ["crown", "glasses", "blush", "wings"], "a repeat goes; a face and its cheeks are two places");
        a.wear = vec!["crown".into(), "beanie".into()];
        assert!(a.validate().unwrap_err().to_string().contains("one thing on its head"));
        a.wear = vec!["a sombrero".into()];
        assert!(a.validate().unwrap_err().to_string().contains("unknown wear"));
        a.wear.clear();
        a.eyes = "stars".into();
        a.validate().unwrap();
        a.eyes = "wink".into();
        assert!(a.validate().is_err(), "a reaction's face isn't eyes to keep");
    }

    #[test]
    fn the_catalog_draws_every_form() {
        let raw: serde_json::Value = serde_json::from_str(CATALOG).unwrap();
        let forms = raw["forms"].as_object().unwrap();
        for f in crate::core::FORMS {
            let form = &forms[*f];
            assert!(form["body"].is_string(), "{f} has no body");
            for pin in ["top", "brow", "face", "mouth", "cheek", "side", "neck", "back"] {
                assert!(!form["pins"][pin].is_null(), "{f} has no {pin} pin");
            }
        }
        assert_eq!(forms.len(), crate::core::FORMS.len(), "the catalog's forms are crate::core::FORMS");
    }
}
