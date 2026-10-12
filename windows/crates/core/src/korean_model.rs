//! Syllable-unigram model for Korean: enough to tell "안녕하세요" from the accidental
//! Hangul that English produces on a 2-set layout (e.g. "the" → "솓"). KoreanModel.swift.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use crate::hangul::{self, Composition, Unit};
use crate::jamo;
use crate::korean_syllables::{TIER1, TIER2, TIER3};
use crate::tables::WordTable;

#[derive(Clone, Debug)]
pub struct KoreanScore {
    /// Average log-probability per keystroke.
    pub average_log_probability: f64,
    pub composition: Composition,
}

/// Where syllable probabilities come from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// The hand-curated frequency tiers in korean_syllables.rs.
    Tiers,
    /// Counts from Korean sentences (Data/korean_syllables.txt, Tatoeba, CC-BY 2.0 FR).
    Corpus,
    /// Whichever of the two rates a syllable higher.
    Max,
}

impl Source {
    pub fn parse(s: &str) -> Option<Source> {
        match s {
            "tiers" => Some(Source::Tiers),
            "corpus" => Some(Source::Corpus),
            "max" => Some(Source::Max),
            _ => None,
        }
    }
}

pub const TIER1_LOG_P: f64 = -5.0;
pub const TIER2_LOG_P: f64 = -6.5;
pub const TIER3_LOG_P: f64 = -8.0;
pub const UNKNOWN_SYLLABLE_LOG_P: f64 = -11.0;
pub const STRAY_JAMO_LOG_P: f64 = -13.0;
/// Typing the first consonant of the next syllable is normal mid-word.
pub const PENDING_CONSONANT_LOG_P: f64 = -3.5;
/// ㅋㅋ, ㅎㅎ, ㅠㅠ and friends.
pub const CHAT_JAMO_LOG_P: f64 = -4.0;
const CHAT_JAMO: &str = "ㅋㅎㅠㅜㄷㅇㄱㅅㅂㄴ";

/// Words seen in Korean sentences (Data/korean_words.txt, Tatoeba): 나는, 무슨, 내가…
static CORPUS_WORDS: LazyLock<WordTable> =
    LazyLock::new(|| WordTable::new(include_bytes!(concat!(env!("OUT_DIR"), "/korean_words.bin"))));
/// Korean words of two or more syllables, loanwords included (FrequencyWords). Only for
/// splitting `hello월드`.
static VOCABULARY: LazyLock<WordTable> =
    LazyLock::new(|| WordTable::new(include_bytes!(concat!(env!("OUT_DIR"), "/korean_vocabulary.bin"))));
static SYLLABLE_COUNTS: &str = include_str!(concat!(env!("OUT_DIR"), "/korean_syllables.txt"));

pub struct KoreanModel {
    tiers: HashMap<char, f64>,
}

static SHARED: LazyLock<Arc<KoreanModel>> = LazyLock::new(|| Arc::new(KoreanModel::new(Source::Max)));

impl Default for KoreanModel {
    fn default() -> Self {
        KoreanModel::new(Source::Max)
    }
}

fn is_syllable(c: char) -> bool {
    ('가'..='힣').contains(&c)
}

impl KoreanModel {
    pub fn new(source: Source) -> KoreanModel {
        let mut map: HashMap<char, f64> = HashMap::new();
        if source != Source::Corpus {
            // Later (more frequent) tiers overwrite earlier ones.
            for (syllables, log_p) in [(TIER3, TIER3_LOG_P), (TIER2, TIER2_LOG_P), (TIER1, TIER1_LOG_P)] {
                for s in syllables.chars().filter(|&c| is_syllable(c)) {
                    map.insert(s, log_p);
                }
            }
        }
        if source != Source::Tiers {
            let counts = corpus_counts();
            if !counts.is_empty() {
                let total = counts.values().map(|&n| n as f64).sum::<f64>();
                let (smoothing, possible) = (0.5, 11172.0);
                for (&s, &n) in &counts {
                    let log_p = ((n as f64 + smoothing) / (total + smoothing * possible)).ln();
                    let entry = map.entry(s).or_insert(f64::NEG_INFINITY);
                    *entry = entry.max(log_p);
                }
            }
        }
        KoreanModel { tiers: map }
    }

    /// The default model (tiers and corpus counts), built once: every `KoreanModel()` of the
    /// Swift code is this same model.
    pub fn shared() -> Arc<KoreanModel> {
        SHARED.clone()
    }

    /// A word that occurs in real Korean text (the composed Hangul, not the keys).
    pub fn is_known_word(&self, hangul: &str) -> bool {
        CORPUS_WORDS.contains(hangul)
    }

    /// A word in the wider vocabulary (loanwords like 월드, 서버), for English + Korean splits.
    pub fn is_vocabulary_word(&self, hangul: &str) -> bool {
        CORPUS_WORDS.contains(hangul) || VOCABULARY.contains(hangul)
    }

    pub fn syllable_log_probability(&self, syllable: char) -> f64 {
        self.tiers.get(&syllable).copied().unwrap_or(UNKNOWN_SYLLABLE_LOG_P)
    }

    /// Whether every syllable is one people actually use.
    pub(crate) fn all_syllables_known(&self, composition: &Composition) -> bool {
        composition.syllables().all(|s| self.syllable_log_probability(s) > UNKNOWN_SYLLABLE_LOG_P)
    }

    /// `partial`: the token is still being typed, so a trailing lone consonant is expected
    /// rather than penalized.
    pub fn score(&self, keys: &str, partial: bool) -> Option<KoreanScore> {
        let composition = hangul::compose(keys)?;
        if keys.is_empty() {
            return None;
        }
        let units = &composition.units;
        let all_chat = units.len() >= 2
            && units.iter().all(|u| matches!(u, Unit::Jamo(j) if CHAT_JAMO.contains(*j)));
        let mut total = 0.0;
        for (i, unit) in units.iter().enumerate() {
            match *unit {
                Unit::Syllable(s) => total += self.syllable_log_probability(s),
                Unit::Jamo(j) => {
                    if all_chat {
                        total += CHAT_JAMO_LOG_P;
                    } else if partial && i == units.len() - 1 && jamo::is_consonant(j) {
                        total += PENDING_CONSONANT_LOG_P;
                    } else {
                        total += STRAY_JAMO_LOG_P;
                    }
                }
            }
        }
        let count = keys.chars().count() as f64;
        Some(KoreanScore { average_log_probability: total / count, composition })
    }
}

fn corpus_counts() -> HashMap<char, i64> {
    let mut counts = HashMap::new();
    for line in SYLLABLE_COUNTS.lines().filter(|l| !l.is_empty()) {
        let parts: Vec<&str> = line.split('\t').filter(|f| !f.is_empty()).collect();
        if parts.len() == 2 {
            if let (Some(s), Ok(n)) = (parts[0].chars().next(), parts[1].parse::<i64>()) {
                counts.insert(s, n);
            }
        }
    }
    counts
}

/// The number of words the corpus list holds (for the self-test).
pub fn corpus_word_count() -> usize {
    CORPUS_WORDS.len()
}
