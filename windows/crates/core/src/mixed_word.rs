//! An English word with a Korean particle or ending typed straight after it,
//! e.g. `githubdp` → `github에`, `APIfmf` → `API를`, `commitgotj` → `commit해서`,
//! and the engine that decides such words (MixedWord.swift).

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use crate::decision::{Decision, DecisionContext, DecisionEngine, Language};
use crate::english_model::EnglishModel;
use crate::hangul;
use crate::jamo::dubeolsik;
use crate::korean_model::KoreanModel;
use crate::romaji;
use crate::segmenter::JapaneseSegmenter;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MixedWord {
    pub english: String,
    pub korean_keys: String,
    pub korean: String,
}

impl MixedWord {
    pub fn text(&self) -> String {
        format!("{}{}", self.english, self.korean)
    }
}

/// Particles and endings that commonly attach directly to a (loan)word.
pub static SUFFIXES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    "
    에 에서 에게 에는 에서는 에도 에서도 의 을 를 은 는 이 가 도 로 으로 와 과 랑 이랑 하고 한테 까지 부터 만 보다
    처럼 이나 나 이야 야 이에요 예요 이다 입니다 인데 이라 이라고 라고 으로는 로는 으로도 로도 이고 이지 이랑은
    했어 했다 했어요 했는데 했고 했지 했음 해서 해야 하는 하면 하고 하기 하게 하다 해 해요 합니다 할 한 함 하자 해줘 해봐
    됐어 됐다 된 되는 되면 돼 돼요 됩니다 같은 같아 적 적인 적으로 스럽게 같이 들 들이 들을 들은 에요 이요 를요 을요
    같은건 같은거 같은게 같은걸 같은데 같다 같네 같아서 같은지 같고 같아요 인가 인지 인데요 인건 인게 인거
    이라는 라는 이란 에선 으론 로서 으로서
    "
    .split_whitespace()
    .collect()
});

/// Particles that follow a single letter (x에, n이, a를, X축의 X): case particles only, not
/// endings like 해/한, so short English (ago) doesn't split.
static SINGLE_LETTER_PARTICLES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    "
    에 에서 에게 에는 에서는 에도 의 을 를 은 는 이 가 도 로 으로 와 과 랑 이랑 하고 한테 까지 부터 만 보다
    처럼 이나 나 이라 이라고 라고 이랑은 으로는 로는 같은 같이
    "
    .split_whitespace()
    .collect()
});

/// A single letter and a particle: `xdp` → `x에`, `Xfmf` → `X를`. One key on its own can't
/// make a Hangul syllable before another consonant, so the keys never read as Korean.
pub fn split_single_letter(keys: &str) -> Option<MixedWord> {
    let mut chars = keys.chars();
    let first = chars.next()?;
    if keys.chars().count() < 2 || !first.is_ascii() || !first.is_alphabetic() {
        return None;
    }
    let suffix_keys: String = chars.collect();
    if !suffix_keys.chars().next().map(dubeolsik::starts_syllable).unwrap_or(false) {
        return None;
    }
    let composed = hangul::compose(&suffix_keys)?;
    let text = composed.text();
    if composed.has_stray_jamo() || !SINGLE_LETTER_PARTICLES.contains(text.as_str()) {
        return None;
    }
    // 도 and 나 are the keys eh and sk, common English endings (heh, meh, yeh, tsk): after a
    // lowercase letter they stay English. A capital (X도) was typed on purpose.
    if first.is_lowercase() && (text == "도" || text == "나") {
        return None;
    }
    Some(MixedWord { english: first.to_string(), korean_keys: suffix_keys, korean: text })
}

/// Splits off the longest Korean particle/ending at the end of `keys`.
/// Only the shape is checked here; whether the front really is English is
/// up to the caller (see `SplittingEngine`).
pub fn split(keys: &str) -> Option<MixedWord> {
    let chars: Vec<char> = keys.chars().collect();
    // Front needs at least 2 letters; try the longest suffix first.
    if chars.len() < 3 {
        return None;
    }
    for start in 2..chars.len() {
        if !dubeolsik::starts_syllable(chars[start]) {
            continue;
        }
        let suffix_keys: String = chars[start..].iter().collect();
        let Some(composed) = hangul::compose(&suffix_keys) else { continue };
        let text = composed.text();
        if composed.has_stray_jamo() || !SUFFIXES.contains(text.as_str()) {
            continue;
        }
        return Some(MixedWord { english: chars[..start].iter().collect(), korean_keys: suffix_keys, korean: text });
    }
    None
}

/// How sure a split is: above Strict (`AppPolicy::STRICT_THRESHOLD`), since each rule already
/// asks for an English front it can stand on, so github에 splits in a terminal too.
pub const SPLIT_CONFIDENCE: f64 = 0.98;

/// Decides `mixed` for English and Korean typed as one token: English + particle (github에),
/// a single letter + particle (x에), English + Korean word (hello월드), and with Japanese on
/// English + Japanese particle (githubで). Never when the whole token is a known English word or
/// reads as ordinary Korean (every syllable is one people actually use).
pub struct SplittingEngine {
    pub base: Arc<dyn DecisionEngine>,
    pub english: &'static EnglishModel,
    pub korean: Arc<KoreanModel>,
    /// The front counts as English if it's in the word list, or spells like English
    /// (trigram score at least this) and can't be read as Hangul.
    pub english_spelling_floor: f64,
    /// English word + Korean word (hello월드) only when the whole token spells less like
    /// English than this; stricter still when the Korean half isn't in the word lists.
    pub mixed_word_spelling_ceiling: f64,
    pub unknown_word_spelling_ceiling: f64,
    /// With Japanese on: English word + Japanese particle (githubで), decided the same way.
    pub japanese: Option<Arc<JapaneseSegmenter>>,
}

fn count(s: &str) -> usize {
    s.chars().count()
}

impl SplittingEngine {
    pub fn new(base: Arc<dyn DecisionEngine>, english: &'static EnglishModel, japanese: Option<Arc<JapaneseSegmenter>>) -> SplittingEngine {
        SplittingEngine {
            base,
            english,
            korean: KoreanModel::shared(),
            english_spelling_floor: -2.6,
            mixed_word_spelling_ceiling: -2.7,
            unknown_word_spelling_ceiling: -3.2,
            japanese,
        }
    }

    /// UI, AI, OS, API: capitals typed on purpose, with at least one that has no shifted
    /// jamo (so it can't be ㅃㅉㄸㄲㅆㅒㅖ). Short ones count too (UI같은건).
    pub fn is_acronym(front: &str) -> bool {
        (2..=6).contains(&count(front))
            && front.chars().all(|c| c.is_uppercase())
            && front.chars().any(|c| !dubeolsik::is_korean_shift_key(c))
    }

    /// Slack, TextEdit, VSCode, GitHub: a name typed with its capitals, so English even when
    /// it isn't in the word list. Starts with a capital, has lowercase too, and has either a
    /// capital with no shifted jamo or another capital inside (PascalCase). Korean typed with
    /// shift (TkdRjvnf, 쌍꺼풀) also passes the second test; `reads_as_korean` still stops it.
    pub fn is_capitalized_name(front: &str) -> bool {
        if count(front) < 3
            || !front.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
            || !front.chars().any(|c| c.is_lowercase())
        {
            return false;
        }
        if Self::has_english_only_capital(front) {
            return true;
        }
        if !front.chars().skip(1).any(|c| c.is_uppercase()) {
            return false;
        }
        // Every capital a shifted jamo and the keys spelling whole Hangul is Korean typed with shift,
        // even with a typo making an odd syllable (RoRmtgkgfu, 깨끗핳려), not a name (TextEdit).
        hangul::compose(front).map(|c| c.has_stray_jamo()).unwrap_or(true)
    }

    pub fn has_english_only_capital(front: &str) -> bool {
        front.chars().any(|c| c.is_uppercase() && !dubeolsik::is_korean_shift_key(c))
    }

    /// An English word with a whole Korean word typed straight after it, no space:
    /// `hellodnjfem` → `hello월드`, `designvhsxm` → `design폰트`. The front has to be a known
    /// English word (or a name typed with capitals) and the back at least two syllables:
    /// a known Korean word (optionally with a particle), or, when the whole token spells
    /// nothing like English, any Hangul made of syllables people use. Prefers the longest
    /// English front. Names that spell like English stay whole (cartman isn't car스무).
    pub fn english_then_korean_word(&self, keys: &str) -> Option<MixedWord> {
        let chars: Vec<char> = keys.chars().collect();
        if chars.len() < 5 || self.english.is_known_word(keys) {
            return None;
        }
        let spelling = self.english.average_log_probability(keys);
        // Clean romaji (hasansowo, はさんそを) is Japanese, not English + Hangul.
        if !(spelling < self.mixed_word_spelling_ceiling) || romaji::compose(keys, false).is_clean() {
            return None;
        }
        for start in (2..=chars.len() - 2).rev() {
            let front: String = chars[..start].iter().collect();
            let back: String = chars[start..].iter().collect();
            // Capitals count only when one has no Korean meaning (ThkT is 쐈, not a name).
            let english_front = (Self::has_english_only_capital(&front)
                && (Self::is_acronym(&front) || Self::is_capitalized_name(&front)))
                || (count(&front) >= 3 && self.english.is_known_word(&front));
            if !english_front || !back.chars().next().map(dubeolsik::starts_syllable).unwrap_or(false) {
                continue;
            }
            let Some(composed) = hangul::compose(&back) else { continue };
            if composed.has_stray_jamo() || composed.syllable_count() < 2 {
                continue;
            }
            let text = composed.text();
            if !(self.is_korean_word_or_word_with_particle(&text)
                || (spelling < self.unknown_word_spelling_ceiling && self.korean.all_syllables_known(&composed)))
            {
                continue;
            }
            if !(Self::has_english_only_capital(&front) || !self.reads_as_korean(keys)) {
                return None;
            }
            return Some(MixedWord { english: front, korean_keys: back, korean: text });
        }
        None
    }

    fn is_korean_word_or_word_with_particle(&self, hangul: &str) -> bool {
        if self.korean.is_vocabulary_word(hangul) {
            return true;
        }
        let chars: Vec<char> = hangul.chars().collect();
        (1..chars.len()).any(|cut| {
            cut >= 2
                && self.korean.is_vocabulary_word(&chars[..cut].iter().collect::<String>())
                && SUFFIXES.contains(chars[cut..].iter().collect::<String>().as_str())
        })
    }

    fn looks_english(&self, front: &str) -> bool {
        if self.english.is_known_word(front) {
            return true;
        }
        let hangul = hangul::compose(front);
        hangul.map(|c| c.has_stray_jamo()).unwrap_or(true)
            && self.english.average_log_probability(front) >= self.english_spelling_floor
    }

    fn is_korean_word(&self, keys: &str) -> bool {
        match hangul::compose(keys) {
            Some(composed) if !composed.has_stray_jamo() => self.korean.is_known_word(&composed.text()),
            _ => false,
        }
    }

    fn reads_as_korean(&self, keys: &str) -> bool {
        match hangul::compose(keys) {
            Some(composed) if !composed.has_stray_jamo() => self.korean.all_syllables_known(&composed),
            _ => false,
        }
    }

    fn split_decision(mixed: MixedWord) -> Decision {
        let explanation = format!("split {}+{}", mixed.english, mixed.korean);
        Decision::new(Language::Mixed, SPLIT_CONFIDENCE, mixed.text(), explanation)
    }
}

impl DecisionEngine for SplittingEngine {
    fn decide(&self, keys: &str, context: &DecisionContext) -> Decision {
        if let Some(mixed) = split(keys) {
            let english_front = Self::is_acronym(&mixed.english)
                || Self::is_capitalized_name(&mixed.english)
                || (count(&mixed.english) >= 3 && self.looks_english(&mixed.english));
            // A capital with no shifted jamo (the S of Slack) was typed as English on purpose,
            // even if the letters would also spell Korean (slackdptj, 님차에서).
            if english_front
                && !self.english.is_known_word(keys)
                && (Self::has_english_only_capital(&mixed.english) || !self.reads_as_korean(keys))
            {
                return Self::split_decision(mixed);
            }
        }
        if let Some(mixed) = split_single_letter(keys) {
            if !self.english.is_known_word(keys) && !self.reads_as_korean(keys) {
                return Self::split_decision(mixed);
            }
        }
        if let Some(mixed) = self.english_then_korean_word(keys) {
            return Self::split_decision(mixed);
        }
        // Not when the keys spell a Korean word: guswo is 현재, not gus + を (but slackwo,
        // 님차재, is no word, so slackを).
        if let Some((word, particle)) = self.japanese.as_ref().and_then(|j| j.english_with_particle(keys)) {
            if !self.is_korean_word(keys) {
                let explanation = format!("split {word}+{particle}");
                return Decision::new(Language::Japanese, SPLIT_CONFIDENCE, format!("{word}{particle}"), explanation);
            }
        }
        self.base.decide(keys, context)
    }
}
