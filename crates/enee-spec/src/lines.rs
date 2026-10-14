//! YAML path → 1-based source line, for diagnostics and the sourcemap. A second, cheap pass over the parser's
//! event stream (the value tree itself comes from serde-saphyr).

use std::collections::HashMap;

use serde_saphyr::granit_parser::{Event, Parser};

use crate::util::{child, index};

#[derive(Default)]
pub struct Lines(HashMap<String, usize>);

enum Frame {
    Map {
        path: String,
        key: Option<String>,
    },
    Seq {
        path: String,
        next: usize,
    },
    /// A complex (non-scalar) mapping key; its contents are not addressable.
    Skip,
}

impl Lines {
    pub fn new(src: &str) -> Self {
        let mut map = HashMap::new();
        let mut stack: Vec<Frame> = Vec::new();
        for event in Parser::new_from_str(src) {
            let Ok((event, span)) = event else { break };
            let line = span.start.line();
            match event {
                Event::Scalar(value, ..) => {
                    if let Some(Frame::Map { path, key: key @ None }) = stack.last_mut() {
                        let at = child(path, &value);
                        map.entry(at).or_insert(line);
                        *key = Some(value.into_owned());
                    } else {
                        if let Some(at) = slot(&stack) {
                            map.entry(at).or_insert(line);
                        }
                        advance(&mut stack);
                    }
                }
                Event::Alias(_) => {
                    if let Some(at) = slot(&stack) {
                        map.entry(at).or_insert(line);
                    }
                    advance(&mut stack);
                }
                Event::MappingStart(..) | Event::SequenceStart(..) => {
                    let is_map = matches!(event, Event::MappingStart(..));
                    let frame = match stack.last() {
                        Some(Frame::Map { key: None, .. }) | Some(Frame::Skip) => Frame::Skip,
                        _ => {
                            let at = slot(&stack).unwrap_or_default();
                            map.entry(at.clone()).or_insert(line);
                            if is_map { Frame::Map { path: at, key: None } } else { Frame::Seq { path: at, next: 0 } }
                        }
                    };
                    stack.push(frame);
                }
                Event::MappingEnd | Event::SequenceEnd => {
                    stack.pop();
                    advance(&mut stack);
                }
                _ => {}
            }
        }
        Lines(map)
    }

    /// The line of `path`, or of its closest addressable ancestor.
    pub fn get(&self, path: &str) -> Option<usize> {
        let mut p = path;
        loop {
            if let Some(line) = self.0.get(p) {
                return Some(*line);
            }
            let cut = p.rfind(['.', '['])?;
            p = &p[..cut];
        }
    }
}

/// Path of the node that the next event starts.
fn slot(stack: &[Frame]) -> Option<String> {
    match stack.last() {
        None => Some(String::new()),
        Some(Frame::Map { path, key: Some(k) }) => Some(child(path, k)),
        Some(Frame::Seq { path, next }) => Some(index(path, *next)),
        Some(Frame::Map { key: None, .. }) | Some(Frame::Skip) => None,
    }
}

/// A value finished: move the parent to its next slot.
fn advance(stack: &mut [Frame]) {
    match stack.last_mut() {
        Some(Frame::Map { key, .. }) => *key = None,
        Some(Frame::Seq { next, .. }) => *next += 1,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_paths_to_lines() {
        let src = "app:\n  id: x\nviews:\n  main:\n    - stat: { label: A, value: 1 }\n    - list:\n        data: q\n        row_actions: [ { icon: t } ]\n";
        let l = Lines::new(src);
        assert_eq!(l.get("app.id"), Some(2));
        assert_eq!(l.get("views.main[0].stat.value"), Some(5));
        assert_eq!(l.get("views.main[1]"), Some(6));
        assert_eq!(l.get("views.main[1].list.data"), Some(7));
        assert_eq!(l.get("views.main[1].list.row_actions[0].icon"), Some(8));
        assert_eq!(l.get("views.main[1].list.nope"), Some(6));
    }
}
