//! Languages, decisions and the engines that make them (DecisionEngine.swift).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::english_model::{self, EnglishModel};
use crate::hangul;
use crate::japanese_model::JapaneseModel;
use crate::korean_model::{KoreanModel, KoreanScore};
use crate::mixed_word;
use crate::romaji;
use crate::segmenter::{self, JapaneseSegmenter, Segment};
use crate::support::{self, write_atomically};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    English,
    Korean,
    /// English word + Korean particle/ending, e.g. `github에` (see `mixed_word`).
    Mixed,
    /// Romaji typed as Japanese (rendered as hiragana; kanji conversion happens on Space).
    Japanese,
}

impl Language {
    pub fn raw_value(self) -> &'static str {
        match self {
            Language::English => "english",
            Language::Korean => "korean",
            Language::Mixed => "mixed",
            Language::Japanese => "japanese",
        }
    }

    pub fn from_raw(s: &str) -> Option<Language> {
        match s {
            "english" => Some(Language::English),
            "korean" => Some(Language::Korean),
            "mixed" => Some(Language::Mixed),
            "japanese" => Some(Language::Japanese),
            _ => None,
        }
    }

    /// The language a sentence continues in after this word.
    pub fn sentence_language(self) -> Language {
        match self {
            Language::English => Language::English,
            Language::Korean | Language::Mixed => Language::Korean,
            Language::Japanese => Language::Japanese,
        }
    }

    /// What Esc switches to: english → mixed (when the keys split) → korean →
    /// japanese (when enabled) → english. Readings that can't exist for these keys
    /// are skipped (no Korean for "ra-men"), and so are garbled ones (dkssud as
    /// Japanese: dkっすd) as long as a clean alternative exists — a typo can still
    /// be forced into its language when nothing reads cleanly.
    pub fn next(self, keys: &str, japanese: bool) -> Language {
        let order = [Language::English, Language::Mixed, Language::Korean, Language::Japanese];
        let composed = hangul::compose(keys);
        let available: Vec<Language> = order
            .into_iter()
            .filter(|language| match language {
                Language::English => true,
                Language::Mixed => mixed_word::split(keys).is_some(),
                Language::Korean => composed.is_some(),
                Language::Japanese => japanese,
            })
            .collect();
        let clean: Vec<Language> = available
            .iter()
            .copied()
            .filter(|language| match language {
                Language::English | Language::Mixed => true,
                Language::Korean => composed.as_ref().map(|c| !c.has_stray_jamo()).unwrap_or(false),
                Language::Japanese => romaji::compose(keys, false).stray_letters == 0,
            })
            .collect();
        let cycle: Vec<Language> = if clean.len() >= 2 {
            available.iter().copied().filter(|l| clean.contains(l) || *l == self).collect()
        } else {
            available
        };
        match cycle.iter().position(|&l| l == self) {
            Some(index) => cycle[(index + 1) % cycle.len()],
            None => cycle.first().copied().unwrap_or(Language::English),
        }
    }
}

/// What the engine knows besides the token itself.
#[derive(Clone, Copy, Debug, Default)]
pub struct DecisionContext {
    /// Language of the previously committed word, if it is recent enough to matter.
    pub previous_language: Option<Language>,
    /// The token is still being typed (used for the live preview).
    pub partial: bool,
}

impl DecisionContext {
    pub fn new(previous_language: Option<Language>, partial: bool) -> DecisionContext {
        DecisionContext { previous_language, partial }
    }
}

#[derive(Clone, Debug)]
pub struct Decision {
    pub language: Language,
    /// 0.5 = coin flip, 1.0 = certain.
    pub confidence: f64,
    /// What should be committed for this decision.
    pub output: String,
    /// Human-readable reasoning, for the CLI and debugging.
    pub explanation: String,
    /// The second most likely language, when the engine compared several.
    pub runner_up: Option<Language>,
}

impl Decision {
    pub fn new(language: Language, confidence: f64, output: impl Into<String>, explanation: impl Into<String>) -> Decision {
        Decision { language, confidence, output: output.into(), explanation: explanation.into(), runner_up: None }
    }
}

/// A local decider that maps raw QWERTY keystrokes of one word to a language.
pub trait DecisionEngine: Send + Sync {
    fn decide(&self, keys: &str, context: &DecisionContext) -> Decision;

    /// Renders `keys` in a given language regardless of what the engine would pick.
    fn render(&self, keys: &str, language: Language) -> String {
        match language {
            Language::English => keys.to_string(),
            Language::Korean => hangul::compose(keys).map(|c| c.text()).unwrap_or_else(|| keys.to_string()),
            Language::Mixed => {
                // The engine's own split (hello월드, github에); the particle split for engines without one.
                let decision = self.decide(keys, &DecisionContext::default());
                if decision.language == Language::Mixed {
                    decision.output
                } else {
                    mixed_word::split(keys).map(|m| m.text()).unwrap_or_else(|| keys.to_string())
                }
            }
            Language::Japanese => romaji::compose(keys, false).text,
        }
    }
}

/// Scores the same keystrokes as English (character trigrams + word list), Korean
/// (2-set syllables) and, when enabled, Japanese (romaji → kana trigrams), each as
/// average log-probability per keystroke, nudges the previous word's language, and
/// picks the best. Confidence comes from the gap to the runner-up.
pub struct HeuristicEngine {
    pub english: &'static EnglishModel,
    pub korean: Arc<KoreanModel>,
    /// None when Japanese is off or its model isn't available.
    pub japanese: Option<&'static JapaneseModel>,
    pub segmenter: Option<Arc<JapaneseSegmenter>>,

    /// Added to the previous word's language score: people mostly stay in one language.
    pub context_bias: f64,
    /// Bias after Japanese text. Raising it to 1.2 recovered 1 of 2,590 held-out phrases
    /// but tripled common English words turning into kana right after Japanese (0.5% → 1.6%).
    pub japanese_context_bias: f64,
    /// Shifts Japanese scores to the English/Korean scale (kana trigrams over ~80 symbols
    /// per ~2 keys read differently from letter trigrams). Tuned on held-out data so that
    /// English words almost never turn into kana.
    pub japanese_offset: f64,
    /// Shifts Korean scores (used when the syllable model changes scale).
    pub korean_offset: f64,
    /// Taken off the dictionary floor for rarer English words (SCOWL levels 40–50: dork,
    /// ahem), so a common Korean word with the same keys (애가, 모든) wins the tie.
    pub rare_word_floor_penalty: f64,
    /// The previous word's pull is scaled by this when the keys are a known word of another
    /// language and not of the previous one (내가 after English, `for` after Korean): that's
    /// someone switching language, not a stray reading.
    pub switch_bias_factor: f64,
    /// Score floors for a Korean word seen in real text (two or more syllables, one), the
    /// counterpart of the English dictionary floor: 내가, 무슨 stay Korean after English.
    pub korean_known_floor: (f64, f64),
    /// How far Japanese must lead before a known English word is turned into kana
    /// outside Japanese text.
    pub english_word_protection: f64,
    /// Score floor for Korean that reads cleanly (three or more syllables, all of them common,
    /// no stray jamo) when the keys aren't an English word: loanwords missing from the word
    /// lists (리드미 typed as flemal, even after an English word). On the top 10,000 words of each
    /// language in context (tools/eval_context.py) -2.0 took wrong words from 488 to 456 with no
    /// English word turned; of the top 50,000 English words 35 rare names (chandi, quantico) turn
    /// Korean.
    pub clean_korean_floor: Option<f64>,
}

impl HeuristicEngine {
    /// English and Korean, plus Japanese when `japanese` is given.
    pub fn new(korean: Arc<KoreanModel>, japanese: Option<&'static JapaneseModel>) -> HeuristicEngine {
        let english = english_model::shared();
        HeuristicEngine {
            english,
            korean,
            japanese,
            segmenter: japanese.map(|j| Arc::new(JapaneseSegmenter::new(english, j))),
            context_bias: 0.8,
            japanese_context_bias: 0.8,
            japanese_offset: 0.5,
            korean_offset: 0.0,
            rare_word_floor_penalty: 0.3,
            switch_bias_factor: 0.25,
            korean_known_floor: (-1.4, -2.0),
            english_word_protection: 1.0,
            clean_korean_floor: Some(-2.0),
        }
    }
}

fn is_letter(c: char) -> bool {
    c.is_alphabetic()
}

impl DecisionEngine for HeuristicEngine {
    fn decide(&self, keys: &str, context: &DecisionContext) -> Decision {
        // Shift on a key that has no shifted jamo (e.g. "Hello", "API") is only ever English.
        if keys.chars().any(|c| c.is_uppercase() && !crate::jamo::dubeolsik::is_korean_shift_key(c)) {
            return Decision::new(Language::English, 0.99, keys, "uppercase outside QWERTOP");
        }
        let count = keys.chars().count();

        let letters: String = keys.chars().filter(|&c| is_letter(c)).collect();
        let mut e = self.english.average_log_probability(&letters);
        // Also while typing: the preview must show what Space/Return will commit.
        let known = self.english.is_known_word(keys);
        if known {
            // Short dictionary words collide with real Korean (go=해, so=내, do=애),
            // so they get a weaker boost and leave room for context to decide.
            let mut floor: f64 = if count >= 4 { -1.4 } else if count == 3 { -2.0 } else { -2.6 };
            // to, so, go, do: everyday English just above a one-syllable Korean word (새, 내,
            // 해, 애), so they stay English unless the previous word was Korean.
            if count == 2 && self.english.is_common_word(keys) {
                floor = self.korean_known_floor.1 + 0.1;
            }
            if self.english.level(keys).unwrap_or(0) > english_model::EXTRA_LEVEL {
                floor -= self.rare_word_floor_penalty;
            }
            e = e.max(floor);
        }
        if keys.chars().any(|c| !is_letter(c)) {
            e -= 3.0; // "ra-men", "kan'i" aren't English
        }

        let mut k: Option<KoreanScore> = self.korean.score(keys, context.partial);
        let mut known_korean = k
            .as_ref()
            .map(|s| !s.composition.has_stray_jamo() && self.korean.is_known_word(&s.composition.text()))
            .unwrap_or(false);
        if known_korean {
            if let Some(score) = k.as_mut() {
                let floor = if score.composition.syllable_count() >= 2 {
                    self.korean_known_floor.0
                } else {
                    self.korean_known_floor.1
                };
                score.average_log_probability = score.average_log_probability.max(floor);
            }
        }
        if !known && !known_korean {
            if let (Some(floor), Some(score)) = (self.clean_korean_floor, k.as_mut()) {
                let c = &score.composition;
                if !c.has_stray_jamo() && c.syllable_count() >= 3 && self.korean.all_syllables_known(c) {
                    score.average_log_probability = score.average_log_probability.max(floor);
                    // Counts as a Korean word for the previous word's pull, too.
                    known_korean = true;
                }
            }
        }
        // A dictionary English word may already be complete: score its Japanese reading as
        // finished too, so the preview shows what Space will commit ("to" stays to, not と).
        let j = self.japanese.and_then(|m| m.score(keys, context.partial && !known));
        // English words inside a Japanese phrase (googlede…) score as segmented Japanese,
        // not as kana with stray letters.
        let mut segmented: Option<(Vec<Segment>, f64)> = None;
        if let (Some(j), Some(segmenter)) = (j.as_ref(), self.segmenter.as_ref()) {
            if j.composition.stray_letters > 0 {
                segmented = segmenter.scored_segments(keys);
            }
        }
        let japanese_score = j.as_ref().map(|j| {
            let seg = segmented.as_ref().map(|s| s.1 / count as f64).unwrap_or(f64::NEG_INFINITY);
            j.average_log_probability.max(seg)
        });

        let mut scores: Vec<(Language, f64)> = vec![(Language::English, e)];
        if let Some(k) = k.as_ref() {
            scores.push((Language::Korean, k.average_log_probability + self.korean_offset));
        }
        if let Some(score) = japanese_score {
            scores.push((Language::Japanese, score + self.japanese_offset));
        }
        if let Some(previous) = context.previous_language.map(|l| l.sentence_language()) {
            if let Some(i) = scores.iter().position(|s| s.0 == previous) {
                let mut bias = if previous == Language::Japanese { self.japanese_context_bias } else { self.context_bias };
                let known_in = |l: Language| match l {
                    Language::English => known,
                    Language::Korean => known_korean,
                    _ => false,
                };
                let others = [Language::English, Language::Korean, Language::Japanese];
                if !known_in(previous) && others.iter().any(|&l| l != previous && known_in(l)) {
                    bias *= self.switch_bias_factor;
                }
                scores[i].1 += bias;
            }
        }
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // A dictionary English word only becomes kana on a clear win, unless we're already
        // in Japanese text (where "same", "sake", "kite" are Japanese words).
        if known
            && scores[0].0 == Language::Japanese
            && context.previous_language.map(|l| l.sentence_language()) != Some(Language::Japanese)
        {
            if let Some(english_index) = scores.iter().position(|s| s.0 == Language::English) {
                if scores[0].1 - scores[english_index].1 < self.english_word_protection {
                    scores.swap(0, english_index);
                }
            }
        }

        let (language, best) = scores[0];
        let runner_up = scores.get(1).copied();
        let margin = best - runner_up.map(|r| r.1).unwrap_or(best - 5.0);
        let confidence = 1.0 / (1.0 + (-margin * 2.0).exp());
        let output = match language {
            Language::Korean => k.as_ref().map(|k| k.composition.text()).unwrap_or_else(|| keys.to_string()),
            Language::Japanese => match (&segmented, &j) {
                (Some(s), _) => segmenter::text(&s.0),
                (None, Some(j)) => j.composition.text.clone(),
                _ => keys.to_string(),
            },
            _ => keys.to_string(),
        };
        let detail: Vec<String> = scores.iter().map(|s| format!("{}={:.2}", &s.0.raw_value()[..2], s.1)).collect();
        let explanation = format!("{}{} margin={:+.2}", detail.join(" "), if known { " (dict)" } else { "" }, margin);
        Decision { language, confidence, output, explanation, runner_up: runner_up.map(|r| r.0) }
    }
}

/// lexicon.json: words learned for good, and words corrected once.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
struct Store {
    learned: BTreeMap<String, Language>,
    once: BTreeMap<String, Language>,
}

struct LearningState {
    store: Store,
    last_learned_keys: Option<String>,
    /// When lexicon.json was last read or written here, so a change by another process (the
    /// input method runs in every app) is read back before this one writes.
    stamp: Option<SystemTime>,
}

/// Wraps an engine with explicit per-word choices the user has made (by flipping a
/// word with Esc). Learned choices always win.
///
/// One correction only changes that word: a habitual or mistaken Esc shouldn't change
/// every later occurrence. The same correction a second time is learned.
pub struct LearningEngine {
    pub base: Arc<dyn DecisionEngine>,
    state: Mutex<LearningState>,
    store_path: Option<PathBuf>,
}

fn modified(path: &Option<PathBuf>) -> Option<SystemTime> {
    path.as_ref().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok())
}

impl LearningEngine {
    pub fn new(base: Arc<dyn DecisionEngine>, store_path: Option<PathBuf>) -> LearningEngine {
        let stamp = modified(&store_path);
        let store = Self::read(&store_path);
        LearningEngine {
            base,
            state: Mutex::new(LearningState { store, last_learned_keys: None, stamp }),
            store_path,
        }
    }

    /// lexicon.json in the support folder.
    pub fn default_store_path() -> Option<PathBuf> {
        support::directory().map(|d| d.join("lexicon.json"))
    }

    fn read(path: &Option<PathBuf>) -> Store {
        let Some(path) = path else { return Store::default() };
        let Ok(data) = std::fs::read(path) else { return Store::default() };
        if let Ok(saved) = serde_json::from_slice::<Store>(&data) {
            return saved;
        }
        // Unreadable (hand-edited, or from a newer version): keep it aside, don't overwrite it.
        let mut aside = path.as_os_str().to_owned();
        aside.push(".unreadable");
        let _ = std::fs::remove_file(&aside);
        let _ = std::fs::rename(path, &aside);
        Store::default()
    }

    /// Words learned for good (corrected the same way twice).
    pub fn learned(&self) -> BTreeMap<String, Language> {
        self.state.lock().unwrap().store.learned.clone()
    }

    pub fn learned_language(&self, keys: &str) -> Option<Language> {
        self.state.lock().unwrap().store.learned.get(keys).copied()
    }

    /// Words corrected once: the same correction again learns them.
    pub fn pending(&self) -> BTreeMap<String, Language> {
        self.state.lock().unwrap().store.once.clone()
    }

    /// The keys most recently learned for good in this process, while still learned.
    pub fn last_learned(&self) -> Option<String> {
        let state = self.state.lock().unwrap();
        state.last_learned_keys.clone().filter(|k| state.store.learned.contains_key(k))
    }

    /// Re-reads lexicon.json (after another process changed it).
    pub fn reload(&self) {
        let stamp = modified(&self.store_path);
        let fresh = Self::read(&self.store_path);
        let mut state = self.state.lock().unwrap();
        state.store = fresh;
        state.stamp = stamp;
    }

    /// Re-reads lexicon.json if another process wrote it since this one last saw it.
    pub fn reload_if_changed(&self) -> bool {
        let stamp = modified(&self.store_path);
        let changed = self.state.lock().unwrap().stamp != stamp;
        if changed {
            self.reload();
        }
        changed
    }

    /// Forgets the keys, learned or pending: the engine decides them again.
    pub fn forget(&self, keys: &str) {
        self.reload_if_changed();
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            if !state.store.learned.contains_key(keys) && !state.store.once.contains_key(keys) {
                None
            } else {
                state.store.learned.remove(keys);
                state.store.once.remove(keys);
                Some(state.store.clone())
            }
        };
        self.persist(snapshot);
    }

    /// Forgets every learned and pending word.
    pub fn forget_all(&self) {
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            state.store = Store::default();
            Some(state.store.clone())
        };
        self.persist(snapshot);
    }

    /// Records that `keys` were corrected to `language`. The second time the same
    /// correction is made it is learned; correcting back to the engine's own choice
    /// forgets the word.
    ///
    /// Pass `engine_choice` when the caller already knows what the engine decided (the
    /// input method always does: what was on screen, which a fresh decision might not match).
    pub fn learn(&self, keys: &str, language: Language, engine_choice: Option<Language>) {
        let base_choice = engine_choice.unwrap_or_else(|| self.base.decide(keys, &DecisionContext::default()).language);
        self.reload_if_changed();
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            let store = &mut state.store;
            // Already learned this way (e.g. converting a learned word again): nothing to change.
            if store.learned.get(keys) == Some(&language) {
                None
            } else {
                if language == base_choice {
                    store.learned.remove(keys);
                    store.once.remove(keys);
                } else if store.once.get(keys) == Some(&language) {
                    store.learned.insert(keys.to_string(), language);
                    store.once.remove(keys);
                    state.last_learned_keys = Some(keys.to_string());
                } else {
                    // A different correction than before: it starts over.
                    store.learned.remove(keys);
                    store.once.insert(keys.to_string(), language);
                }
                Some(state.store.clone())
            }
        };
        self.persist(snapshot);
    }

    fn persist(&self, snapshot: Option<Store>) {
        let (Some(snapshot), Some(path)) = (snapshot, self.store_path.as_ref()) else { return };
        let Ok(mut data) = serde_json::to_vec_pretty(&snapshot) else { return };
        data.push(b'\n');
        if write_atomically(path, &data).is_ok() {
            self.state.lock().unwrap().stamp = modified(&self.store_path);
        }
    }
}

impl DecisionEngine for LearningEngine {
    fn decide(&self, keys: &str, context: &DecisionContext) -> Decision {
        if let Some(learned) = self.learned_language(keys) {
            // Rendered by the base engine: rendering a mixed word asks for a decision, which
            // here would come straight back to this learned word (DecisionEngine.swift recurses
            // without end on a word learned as mixed).
            return Decision::new(learned, 1.0, self.base.render(keys, learned), "learned");
        }
        self.base.decide(keys, context)
    }
}
