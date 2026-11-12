//! Strict, single-sequence BERT WordPiece inference. No regex programs or tokenizer plugins.
use super::invalid;
use kortex_core::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use unicode_general_category::{GeneralCategory as G, get_general_category as category};
use unicode_normalization::UnicodeNormalization;

pub(super) struct BertTokenizer {
    vocab: BTreeMap<String, i64>,
    special: Vec<String>,
    lowercase: bool,
    accents: bool,
    chinese: bool,
    clean: bool,
    max_word: usize,
    cls: i64,
    sep: i64,
    unk: i64,
    pub pad: i64,
}
fn boolean(value: &Value, name: &str) -> Result<bool> {
    value[name]
        .as_bool()
        .ok_or_else(|| invalid(format!("invalid BERT tokenizer {name}")))
}
impl BertTokenizer {
    pub fn load(bytes: &[u8], size: usize, pad: usize) -> Result<Self> {
        let v: Value = serde_json::from_slice(bytes).map_err(invalid)?;
        if v["version"] != "1.0"
            || v["model"]["type"] != "WordPiece"
            || v["model"]["unk_token"] != "[UNK]"
            || v["model"]["continuing_subword_prefix"] != "##"
            || v["normalizer"]["type"] != "BertNormalizer"
            || v["pre_tokenizer"]["type"] != "BertPreTokenizer"
            || v["post_processor"]["type"] != "TemplateProcessing"
            || !v["decoder"].is_null() && v["decoder"]["type"] != "WordPiece"
        {
            return Err(invalid(
                "only fixed BERT WordPiece components are supported",
            ));
        }
        let vocabulary = v["model"]["vocab"]
            .as_object()
            .ok_or_else(|| invalid("missing vocabulary"))?;
        if vocabulary.len() != size {
            return Err(invalid("vocabulary must match encoder exactly"));
        }
        let mut vocab = BTreeMap::new();
        let mut ids = BTreeSet::new();
        for (token, id) in vocabulary {
            let id = id
                .as_i64()
                .filter(|id| *id >= 0 && (*id as usize) < size)
                .ok_or_else(|| invalid("vocabulary ID out of bounds"))?;
            if token.is_empty() || token.len() > 1024 || !ids.insert(id) {
                return Err(invalid("invalid or duplicate vocabulary ID"));
            }
            vocab.insert(token.clone(), id);
        }
        let token = |name: &str| {
            vocab
                .get(name)
                .copied()
                .ok_or_else(|| invalid(format!("missing {name}")))
        };
        let (cls, sep, unk, pad_id) = (
            token("[CLS]")?,
            token("[SEP]")?,
            token("[UNK]")?,
            token("[PAD]")?,
        );
        if pad_id != pad as i64 {
            return Err(invalid("padding ID differs from model config"));
        }
        let processor = &v["post_processor"];
        let canonical_single = serde_json::json!([
            {"SpecialToken":{"id":"[CLS]","type_id":0}},
            {"Sequence":{"id":"A","type_id":0}},
            {"SpecialToken":{"id":"[SEP]","type_id":0}}
        ]);
        if processor["single"] != canonical_single {
            return Err(invalid("noncanonical BERT single template"));
        }
        for (name, id) in [("[CLS]", cls), ("[SEP]", sep)] {
            let special = &processor["special_tokens"][name];
            if special["id"] != name
                || special["ids"] != serde_json::json!([id])
                || special["tokens"] != serde_json::json!([name])
            {
                return Err(invalid("BERT template special-token ID mismatch"));
            }
        }
        let mut special = vec![];
        for added in v["added_tokens"]
            .as_array()
            .ok_or_else(|| invalid("missing added_tokens array"))?
        {
            let text = added["content"]
                .as_str()
                .ok_or_else(|| invalid("invalid added token"))?;
            if !["[PAD]", "[UNK]", "[CLS]", "[SEP]", "[MASK]"].contains(&text)
                || added["id"].as_i64() != vocab.get(text).copied()
                || added["special"] != true
                || added["normalized"] != false
                || added["single_word"] != false
                || added["lstrip"] != false
                || added["rstrip"] != false
                || special.iter().any(|s| s == text)
            {
                return Err(invalid("unsupported added-token behavior"));
            }
            special.push(text.to_owned());
        }
        let lowercase = boolean(&v["normalizer"], "lowercase")?;
        let accents = if v["normalizer"]["strip_accents"].is_null() {
            lowercase
        } else {
            boolean(&v["normalizer"], "strip_accents")?
        };
        let max_word = v["model"]["max_input_chars_per_word"]
            .as_u64()
            .filter(|n| (1..=1000).contains(n))
            .ok_or_else(|| invalid("invalid WordPiece word bound"))?
            as usize;
        Ok(Self {
            vocab,
            special,
            lowercase,
            accents,
            chinese: boolean(&v["normalizer"], "handle_chinese_chars")?,
            clean: boolean(&v["normalizer"], "clean_text")?,
            max_word,
            cls,
            sep,
            unk,
            pad: pad_id,
        })
    }
    pub fn encode(&self, text: &str, max: usize) -> Vec<i64> {
        let mut ids = vec![self.cls];
        let mut rest = text;
        while !rest.is_empty() && ids.len() < max - 1 {
            // Added special tokens are matched before normalization, as required by BERT artifacts.
            let next = self
                .special
                .iter()
                .filter_map(|s| rest.find(s).map(|pos| (pos, s)))
                .min_by_key(|(pos, _)| *pos);
            let (normal, special) =
                next.map_or((rest, None), |(pos, s)| (&rest[..pos], Some((pos, s))));
            self.normal_tokens(normal, &mut ids, max - 1);
            if let Some((pos, special)) = special {
                if ids.len() < max - 1 {
                    ids.push(self.vocab[special]);
                }
                rest = &rest[pos + special.len()..];
            } else {
                break;
            }
        }
        ids.truncate(max - 1);
        ids.push(self.sep);
        ids
    }
    fn normal_tokens(&self, text: &str, ids: &mut Vec<i64>, limit: usize) {
        let mut normalized = String::new();
        for c in text.chars() {
            if self.clean
                && (c == '\0'
                    || c == '\u{fffd}'
                    || matches!(category(c), G::Control | G::Format)
                        && !['\t', '\n', '\r'].contains(&c))
            {
                continue;
            }
            if self.clean && (c.is_whitespace() || category(c) == G::SpaceSeparator) {
                normalized.push(' ');
            } else if self.chinese && is_chinese(c) {
                normalized.push(' ');
                normalized.push(c);
                normalized.push(' ');
            } else {
                normalized.push(c);
            }
        }
        if self.lowercase {
            normalized = normalized.chars().flat_map(char::to_lowercase).collect();
        }
        if self.accents {
            normalized = normalized
                .nfd()
                .filter(|c| category(*c) != G::NonspacingMark)
                .collect();
        }
        let mut word = String::new();
        for c in normalized.chars().chain(std::iter::once(' ')) {
            let punctuation = is_punctuation(c);
            if c.is_whitespace() || punctuation {
                if !word.is_empty() {
                    self.wordpiece(&word, ids, limit);
                    word.clear();
                }
                if punctuation {
                    self.wordpiece(&c.to_string(), ids, limit);
                }
                if ids.len() >= limit {
                    return;
                }
            } else {
                word.push(c);
            }
        }
    }
    fn wordpiece(&self, word: &str, ids: &mut Vec<i64>, limit: usize) {
        if ids.len() >= limit {
            return;
        }
        let chars: Vec<_> = word
            .char_indices()
            .map(|(pos, _)| pos)
            .chain(std::iter::once(word.len()))
            .collect();
        if chars.len() - 1 > self.max_word {
            ids.push(self.unk);
            return;
        }
        let mut pieces = vec![];
        let mut start = 0;
        while start < chars.len() - 1 {
            let found = ((start + 1)..chars.len()).rev().find_map(|end| {
                let part = &word[chars[start]..chars[end]];
                let piece = if start == 0 {
                    part.to_owned()
                } else {
                    format!("##{part}")
                };
                self.vocab.get(&piece).map(|id| (end, *id))
            });
            if let Some((end, id)) = found {
                pieces.push(id);
                start = end;
            } else {
                pieces.clear();
                pieces.push(self.unk);
                break;
            }
        }
        ids.extend(pieces.into_iter().take(limit - ids.len()));
    }
}
fn is_punctuation(c: char) -> bool {
    matches!(c as u32, 33..=47 | 58..=64 | 91..=96 | 123..=126)
        || matches!(
            category(c),
            G::ConnectorPunctuation
                | G::DashPunctuation
                | G::OpenPunctuation
                | G::ClosePunctuation
                | G::InitialPunctuation
                | G::FinalPunctuation
                | G::OtherPunctuation
        )
}
fn is_chinese(c: char) -> bool {
    matches!(c as u32, 0x4e00..=0x9fff | 0x3400..=0x4dbf | 0x20000..=0x2a6df | 0x2a700..=0x2b73f | 0x2b740..=0x2b81f | 0x2b820..=0x2ceaf | 0xf900..=0xfaff | 0x2f800..=0x2fa1f)
}
