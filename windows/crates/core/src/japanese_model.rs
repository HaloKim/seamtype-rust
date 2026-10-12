//! Kana trigram model for judging how Japanese a romaji string reads. Counts come
//! from Japanese sentences (Tatoeba, CC-BY 2.0 FR) turned into the romaji people
//! type and back into kana (tools/build_kana_model.py). JapaneseModel.swift.
//!
//! Clean romaji isn't enough: "hello" composes to へっぉ, which this model scores
//! as very un-Japanese (っぉ essentially never occurs).

use std::collections::HashSet;
use std::sync::LazyLock;

use crate::romaji::{self, RomajiResult};
use crate::tables::{read_f32, read_u32, FloatArray, SparseFloats};

#[derive(Clone, Debug)]
pub struct JapaneseScore {
    /// Average log-probability per keystroke, comparable with the English and Korean models.
    pub average_log_probability: f64,
    pub composition: RomajiResult,
}

pub const STRAY_LETTER_LOG_P: f64 = -13.0;
pub const PENDING_LOG_P: f64 = -3.5;

/// Non-letters that belong inside a Japanese phrase: ん (n'), ー, and the full-width symbols
/// (、。？！「」・…).
pub static IN_PHRASE_SYMBOLS: LazyLock<HashSet<char>> = LazyLock::new(|| {
    let mut set: HashSet<char> = romaji::SYMBOLS.keys().copied().collect();
    set.insert('\'');
    set
});

static KANA_MODEL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/kana_model.bin"));

static SHARED: LazyLock<Option<JapaneseModel>> = LazyLock::new(|| JapaneseModel::from_bytes(KANA_MODEL));

/// Symbol "a" in the model, or the start/end markers.
const START: char = '^';
const END: char = '$';

pub struct JapaneseModel {
    symbols: Vec<u32>,
    total: f32,
    vocabulary: f32,
    unigrams: FloatArray,
    bigram_contexts: FloatArray,
    bigrams: FloatArray,
    trigram_contexts: FloatArray,
    trigrams: SparseFloats,
}

/// The kana model, or None when its data is missing.
pub fn shared() -> Option<&'static JapaneseModel> {
    SHARED.as_ref()
}

impl JapaneseModel {
    fn from_bytes(bytes: &'static [u8]) -> Option<JapaneseModel> {
        let n = read_u32(bytes, 0) as usize;
        let symbols: Vec<u32> = (0..n).map(|i| read_u32(bytes, 4 + i * 4)).collect();
        let mut at = 4 + n * 4;
        let total = read_f32(bytes, at);
        let vocabulary = read_f32(bytes, at + 4);
        at += 8;
        let unigrams = FloatArray::new(bytes, at, n);
        at += n * 4;
        let bigram_contexts = FloatArray::new(bytes, at, n);
        at += n * 4;
        let bigrams = FloatArray::new(bytes, at, n * n);
        at += n * n * 4;
        let trigram_contexts = FloatArray::new(bytes, at, n * n);
        at += n * n * 4;
        let ntri = read_u32(bytes, at) as usize;
        let trigrams = SparseFloats::new(bytes, at + 4, ntri);
        if total <= 0.0 {
            return None;
        }
        Some(JapaneseModel { symbols, total, vocabulary, unigrams, bigram_contexts, bigrams, trigram_contexts, trigrams })
    }

    fn id(&self, c: char) -> Option<usize> {
        self.symbols.binary_search(&(c as u32)).ok()
    }

    /// Interpolated trigram probability of `c` after `a b`. A count of zero is a missing
    /// entry, as in the Swift dictionaries (every stored count is positive).
    fn probability(&self, a: char, b: char, c: char) -> f64 {
        let n = self.symbols.len();
        let (a, b, c) = (self.id(a), self.id(b), self.id(c));
        let unigram = c.map(|c| self.unigrams.get(c)).unwrap_or(0.0);
        let p1: f32 = (unigram + 1.0) / (self.total + self.vocabulary);
        let p2: f32 = match b.map(|b| self.bigram_contexts.get(b)) {
            Some(context) if context > 0.0 => {
                let count = match (b, c) {
                    (Some(b), Some(c)) => self.bigrams.get(b * n + c),
                    _ => 0.0,
                };
                count / context
            }
            _ => 0.0,
        };
        let p3: f32 = match (a, b) {
            (Some(a), Some(b)) if self.trigram_contexts.get(a * n + b) > 0.0 => {
                let context = self.trigram_contexts.get(a * n + b);
                let count = c
                    .and_then(|c| self.trigrams.get(((a * n + b) * n + c) as u32))
                    .unwrap_or(0.0);
                count / context
            }
            _ => 0.0,
        };
        (0.6f32 * p3 + 0.3f32 * p2 + 0.1f32 * p1) as f64
    }

    /// Sum of log-probabilities of the kana (and the end of word unless `partial`).
    pub fn log_probability(&self, kana: &str, partial: bool) -> f64 {
        let mut seq: Vec<char> = vec![START, START];
        seq.extend(kana.chars());
        if !partial {
            seq.push(END);
        }
        let mut sum = 0.0;
        for i in 2..seq.len() {
            sum += self.probability(seq[i - 2], seq[i - 1], seq[i]).ln();
        }
        sum
    }

    pub fn score(&self, keys: &str, partial: bool) -> Option<JapaneseScore> {
        if keys.is_empty()
            || !keys
                .chars()
                .all(|c| c.is_ascii() && (c.is_ascii_alphabetic() || IN_PHRASE_SYMBOLS.contains(&c)))
        {
            return None;
        }
        let composition = romaji::compose(keys, partial);
        let pending = composition.pending.chars().count();
        let text: Vec<char> = composition.text.chars().collect();
        let kana: String = text[..text.len().saturating_sub(pending)]
            .iter()
            .filter(|&&c| !c.is_ascii() && !romaji::PUNCTUATION.contains(&c))
            .collect();
        let mut total = self.log_probability(&kana, partial);
        total += composition.stray_letters as f64 * STRAY_LETTER_LOG_P;
        if !composition.pending.is_empty() {
            total += PENDING_LOG_P;
        }
        let count = keys.chars().count() as f64;
        Some(JapaneseScore { average_log_probability: total / count, composition })
    }
}
