//! What the Swift test suites share: a fake text field, a fake converter, the engines, and
//! throwaway lexicon paths.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use seamtype_core::session::utf16_len;
use seamtype_core::{
    ConversionCandidate, DecisionEngine, HeuristicEngine, InputSession, JapaneseConverter, Key, LearningEngine,
    SplittingEngine, TextClient, TextRange,
};

/// `HeuristicEngine()`: English and Korean.
pub static SHARED_ENGINE: LazyLock<Arc<HeuristicEngine>> = LazyLock::new(|| seamtype_core::heuristic(false));
/// `SplittingEngine(base: sharedEngine, english: sharedEngine.english)`.
pub static SPLITTING: LazyLock<Arc<SplittingEngine>> =
    LazyLock::new(|| Arc::new(SplittingEngine::new(SHARED_ENGINE.clone(), SHARED_ENGINE.english, None)));
/// `HeuristicEngine(..., japanese: JapaneseModel.shared)`.
pub static TRILINGUAL: LazyLock<Arc<HeuristicEngine>> = LazyLock::new(|| seamtype_core::heuristic(true));
/// `SplittingEngine(base: trilingual, english: trilingual.english, japanese: trilingual.segmenter)`.
pub static TRILINGUAL_SPLITTING: LazyLock<Arc<SplittingEngine>> = LazyLock::new(|| seamtype_core::splitting(TRILINGUAL.clone()));

pub fn shared() -> Arc<dyn DecisionEngine> {
    SHARED_ENGINE.clone()
}

pub fn splitting() -> Arc<dyn DecisionEngine> {
    SPLITTING.clone()
}

pub fn trilingual_splitting() -> Arc<dyn DecisionEngine> {
    TRILINGUAL_SPLITTING.clone()
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A file name nobody else uses, in the temporary folder.
pub fn temp_path(prefix: &str, extension: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos();
    std::env::temp_dir().join(format!("{prefix}-{}-{n}-{nanos}{extension}", std::process::id()))
}

pub fn learning(base: Arc<dyn DecisionEngine>, prefix: &str) -> Arc<LearningEngine> {
    Arc::new(LearningEngine::new(base, Some(temp_path(prefix, ".json"))))
}

/// A text field: committed text, an optional marked (underlined) span, and a caret, all in
/// UTF-16 like NSString.
pub struct FakeTextField {
    pub text: Vec<u16>,
    pub marked: TextRange,
    pub cursor: usize,
    /// Simulates apps that don't support reading back their text.
    pub readable: bool,
}

impl Default for FakeTextField {
    fn default() -> Self {
        FakeTextField { text: Vec::new(), marked: TextRange::NONE, cursor: 0, readable: true }
    }
}

impl FakeTextField {
    pub fn new() -> FakeTextField {
        FakeTextField::default()
    }

    pub fn text(&self) -> String {
        String::from_utf16(&self.text).unwrap()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.marked = TextRange::NONE;
        self.cursor = 0;
    }

    pub fn is_marked(&self) -> bool {
        self.marked.is_found()
    }

    fn replace(&mut self, target: TextRange, s: &str) {
        let new: Vec<u16> = s.encode_utf16().collect();
        self.text.splice(target.location..target.location + target.length, new);
    }
}

impl TextClient for FakeTextField {
    fn set_marked_text(&mut self, s: &str) {
        let target = if self.marked.is_found() { self.marked } else { TextRange::new(self.cursor, 0) };
        self.replace(target, s);
        let length = utf16_len(s);
        self.marked = if length > 0 { TextRange::new(target.location, length) } else { TextRange::NONE };
        self.cursor = target.location + length;
    }

    fn insert_text(&mut self, s: &str, replacing: Option<TextRange>) {
        let target = replacing.unwrap_or(if self.marked.is_found() { self.marked } else { TextRange::new(self.cursor, 0) });
        self.replace(target, s);
        self.marked = TextRange::NONE;
        self.cursor = target.location + utf16_len(s);
    }

    fn selected_range(&self) -> TextRange {
        TextRange::new(self.cursor, 0)
    }

    fn marked_range(&self) -> TextRange {
        self.marked
    }

    fn text_in(&self, range: TextRange) -> Option<String> {
        let end = range.end()?;
        if !self.readable || end > self.text.len() {
            return None;
        }
        Some(String::from_utf16(&self.text[range.location..end]).unwrap())
    }
}

/// Types `script`: letters and printable keys as themselves, "⏎" Return, "⎋" Esc, "⇧" ⇧Esc,
/// "←"/"→" arrows, "⌫" Delete. Keys the input method passes on are applied the way a text field
/// would (Return as a newline; Esc and arrows do nothing to the text). Returns what each key's
/// handle returned.
pub fn type_script(session: &mut InputSession, field: &mut FakeTextField, script: &str) -> Vec<bool> {
    let mut results = Vec::new();
    for ch in script.chars() {
        let handled = match ch {
            '⏎' => {
                let handled = session.handle(Key::Enter, field);
                if !handled {
                    field.insert_text("\n", None);
                }
                handled
            }
            '⎋' => session.handle(Key::Escape, field),
            '⇧' => session.handle(Key::Cycle, field),
            '←' => session.handle(Key::Left, field),
            '→' => session.handle(Key::Right, field),
            '⌫' => session.handle(Key::Delete, field),
            c if c.is_alphabetic() => {
                let handled = session.handle(Key::Letter(c), field);
                if !handled {
                    field.insert_text(&c.to_string(), None);
                }
                handled
            }
            c => {
                let handled = session.handle(Key::Printable(c), field);
                if !handled {
                    field.insert_text(&c.to_string(), None);
                }
                handled
            }
        };
        results.push(handled);
    }
    results
}

/// Deterministic converter: a few readings with fixed candidates, including a
/// first-clause candidate to exercise partial commits.
#[derive(Default)]
pub struct FakeConverter {
    pub learned: Vec<String>,
}

impl JapaneseConverter for FakeConverter {
    fn candidates(&mut self, kana: &str) -> Vec<ConversionCandidate> {
        let c = |t: &str, n: usize| ConversionCandidate::new(t, n);
        match kana {
            "きょうはいいてんきですね" => vec![c("今日はいい天気ですね", 12), c("今日は", 4), c(kana, 12)],
            "いいてんきですね" => vec![c("いい天気ですね", 8), c(kana, 8)],
            "へんかん" => vec![c("変換", 4), c("返還", 4), c("編纂", 4)],
            _ => vec![c(kana, kana.chars().count())],
        }
    }

    fn learn(&mut self, candidate: &ConversionCandidate, _kana: &str) {
        self.learned.push(candidate.text.clone());
    }

    fn save(&mut self) {}
}

pub fn fake_converter() -> Arc<Mutex<FakeConverter>> {
    Arc::new(Mutex::new(FakeConverter::default()))
}
