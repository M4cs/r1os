//! BERT's uncased WordPiece, as MiniLM was trained with (ADR-0032): lowercase, split on whitespace and
//! punctuation, then each word into the longest pieces in the vocabulary (`##` marks a piece that continues a
//! word). Small enough to write here rather than pull in a tokenizer library.

use std::collections::HashMap;

pub struct WordPiece {
    vocab: HashMap<String, i64>,
    unk: i64,
    cls: i64,
    sep: i64,
}

impl WordPiece {
    /// From a `vocab.txt` (one piece per line; the line number is its id).
    pub fn from_vocab(text: &str) -> anyhow::Result<Self> {
        let vocab: HashMap<String, i64> =
            text.lines().enumerate().map(|(i, l)| (l.trim_end().to_owned(), i64::try_from(i).unwrap_or(0))).collect();
        let id = |t: &str| vocab.get(t).copied().ok_or_else(|| anyhow::anyhow!("the vocab has no {t}"));
        Ok(WordPiece { unk: id("[UNK]")?, cls: id("[CLS]")?, sep: id("[SEP]")?, vocab })
    }

    /// `[CLS] pieces… [SEP]`, at most `max` ids.
    pub fn encode(&self, text: &str, max: usize) -> Vec<i64> {
        let mut ids = vec![self.cls];
        'words: for word in words(text) {
            let chars: Vec<char> = word.chars().collect();
            if chars.len() > 100 {
                ids.push(self.unk);
                continue;
            }
            let mut pieces = Vec::new();
            let mut start = 0;
            while start < chars.len() {
                let mut end = chars.len();
                let mut found = None;
                while start < end {
                    let piece: String = chars[start..end].iter().collect();
                    let key = if start > 0 { format!("##{piece}") } else { piece };
                    if let Some(id) = self.vocab.get(&key) {
                        found = Some(*id);
                        break;
                    }
                    end -= 1;
                }
                let Some(id) = found else {
                    ids.push(self.unk);
                    continue 'words;
                };
                pieces.push(id);
                start = end;
            }
            ids.extend(pieces);
            if ids.len() >= max - 1 {
                break;
            }
        }
        ids.truncate(max - 1);
        ids.push(self.sep);
        ids
    }
}

/// Lowercased words, with each punctuation mark a word of its own (BERT's basic tokenizer, for the text a
/// speech recognizer writes: no accents to strip, no Chinese characters to space).
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_whitespace() || c.is_control() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else if c.is_ascii_punctuation() || (!c.is_alphanumeric() && !c.is_whitespace()) {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(c.to_string());
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_into_the_longest_pieces() {
        let vocab = "[PAD]\n[UNK]\n[CLS]\n[SEP]\nwhat\nam\ni\nlook\n##ing\nat\n?\nun\n##aff\n##able\n";
        let wp = WordPiece::from_vocab(vocab).unwrap();
        assert_eq!(wp.encode("What am I looking at?", 64), vec![2, 4, 5, 6, 7, 8, 9, 10, 3]);
        assert_eq!(wp.encode("unaffable", 64), vec![2, 11, 12, 13, 3]);
        assert_eq!(wp.encode("zzz look", 64), vec![2, 1, 7, 3], "an unknown word is [UNK]");
        assert_eq!(wp.encode("look look look look", 4), vec![2, 7, 7, 3], "cut to fit");
    }
}
