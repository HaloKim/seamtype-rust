//! Character-trigram language model plus a word list with how common each word is, from
//! SCOWL (Data/english_words.txt: word<TAB>level, 10 = most common … 60). EnglishModel.swift.
//!
//! The letter trigrams come from macOS's /usr/share/dict/words, as on macOS (its 235k words
//! separate English spelling from Korean keys better than SCOWL's 119k); Windows has no such
//! file, so build.rs bakes Apple's counts (data/spelling_trigrams.txt) together with the word
//! table.

use std::sync::LazyLock;

use crate::tables::{FloatArray, WordTable};

/// Symbols: 0 = word boundary, 1...26 = a...z.
const SYMBOLS: usize = 27;
const SMOOTHING: f32 = 0.5;

/// Words up to this level count as English words (a score floor, protection from kana).
pub const KNOWN_LEVEL: i64 = 50;
/// Words up to this level are everyday English (you, sense, machine, for, got).
pub const COMMON_LEVEL: i64 = 20;
/// Level given to the common words (tech terms and slang SCOWL lacks): known, not "common".
pub const EXTRA_LEVEL: i64 = crate::english_lists::EXTRA_LEVEL;

static TRIGRAMS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/english_trigrams.bin"));
static LEVELS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/english_levels.bin"));

static SHARED: LazyLock<EnglishModel> = LazyLock::new(|| EnglishModel {
    trigrams: FloatArray::new(TRIGRAMS, 0, SYMBOLS * SYMBOLS * SYMBOLS),
    contexts: FloatArray::new(TRIGRAMS, SYMBOLS * SYMBOLS * SYMBOLS * 4, SYMBOLS * SYMBOLS),
    levels: WordTable::new(LEVELS),
});

pub struct EnglishModel {
    trigrams: FloatArray,
    contexts: FloatArray,
    /// SCOWL level of each word (lowercase, letters only).
    levels: WordTable,
}

/// The model every engine shares (the word list is only read once).
pub fn shared() -> &'static EnglishModel {
    &SHARED
}

/// [^, ^, letters..., $] with ^ and $ both encoded as 0.
fn encode(word: &str) -> Vec<usize> {
    let mut codes = vec![0, 0];
    for c in word.chars() {
        let v = c as u32;
        if (97..=122).contains(&v) {
            codes.push((v - 96) as usize);
        }
    }
    codes.push(0);
    codes
}

impl EnglishModel {
    pub fn word_count(&self) -> usize {
        self.levels.len()
    }

    /// Average natural-log probability per symbol (letters + end marker).
    /// Real English words land around -1.8…-2.4; keyboard mash around -3.5 or lower.
    pub fn average_log_probability(&self, token: &str) -> f64 {
        let n = SYMBOLS;
        let codes = encode(&token.to_lowercase());
        if codes.len() <= 3 {
            return -10.0;
        }
        let mut total = 0.0f64;
        for i in 2..codes.len() {
            let c = self.trigrams.get((codes[i - 2] * n + codes[i - 1]) * n + codes[i]);
            let t = self.contexts.get(codes[i - 2] * n + codes[i - 1]);
            total += ((c + SMOOTHING) / (t + SMOOTHING * n as f32)).ln() as f64;
        }
        total / (codes.len() - 2) as f64
    }

    fn listed(&self, word: &str) -> Option<i64> {
        self.levels.value(word).map(i64::from)
    }

    /// The word's level, also for regular inflections of a listed word (None: not a word).
    pub fn level(&self, token: &str) -> Option<i64> {
        let w: String = token.to_lowercase().chars().filter(|&c| c != '\'').collect();
        if let Some(level) = self.listed(&w) {
            return Some(level);
        }
        let mut best: Option<i64> = None;
        let mut consider = |stem: &str| {
            if let Some(l) = self.listed(stem) {
                best = Some(best.map_or(l, |b| b.min(l)));
            }
        };
        let count = w.chars().count();
        // Stems of 3+ letters: ths isn't the+s, ehs isn't eh+s.
        for suffix in ["s", "es", "ed", "d", "ing", "ly", "er", "ers", "est"] {
            if count > suffix.len() + 2 && w.ends_with(suffix) {
                let stem = &w[..w.len() - suffix.len()];
                consider(stem);
                consider(&format!("{stem}e"));
                // running → run, stopped → stop
                let mut rev = stem.chars().rev();
                if let (Some(last), Some(before)) = (rev.next(), rev.next()) {
                    if last == before {
                        consider(&stem[..stem.len() - last.len_utf8()]);
                    }
                }
            }
        }
        // stories → story, denied → deny, heavier → heavy
        for suffix in ["ies", "ied", "ier", "iest", "iers"] {
            if count > suffix.len() + 1 && w.ends_with(suffix) {
                consider(&format!("{}y", &w[..w.len() - suffix.len()]));
            }
        }
        best
    }

    /// A word people use (SCOWL level ≤ 50, or one of the common words).
    pub fn is_known_word(&self, token: &str) -> bool {
        self.level(token).map(|l| l <= KNOWN_LEVEL).unwrap_or(false)
    }

    /// Everyday English (level ≤ 20): kept English even right after Korean or Japanese.
    pub fn is_common_word(&self, token: &str) -> bool {
        self.level(token).map(|l| l <= COMMON_LEVEL).unwrap_or(false)
    }
}
