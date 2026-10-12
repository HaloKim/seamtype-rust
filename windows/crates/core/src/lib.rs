//! Seamtype's typing logic, ported from Sources/SeamtypeCore (Swift) for the Windows input
//! method: the user types on QWERTY and each word is decided as English, Korean (2-set) or
//! Japanese romaji. Platform-neutral; `cargo test` runs it anywhere.
//!
//! Module ↔ Swift file: jamo ↔ Jamo, hangul ↔ HangulComposer, korean_model ↔ KoreanModel,
//! english_model ↔ EnglishModel, romaji ↔ RomajiComposer, japanese_model ↔ JapaneseModel,
//! segmenter ↔ JapaneseSegmenter, mixed_word ↔ MixedWord, decision ↔ DecisionEngine,
//! session ↔ InputSession, app_policy ↔ AppPolicy, settings ↔ Settings, vim ↔ VimSignal.

pub mod app_policy;
pub mod decision;
#[allow(dead_code)] // read by build.rs; the library only needs EXTRA_LEVEL
mod english_lists;
pub mod english_model;
pub mod hangul;
pub mod jamo;
pub mod japanese_model;
pub mod korean_model;
mod korean_syllables;
pub mod mixed_word;
pub mod romaji;
pub mod segmenter;
pub mod session;
pub mod settings;
pub mod support;
mod tables;
pub mod text_converter;
pub mod vim;

use std::path::PathBuf;
use std::sync::Arc;

pub use app_policy::{AppPolicies, AppPolicy, Override};
pub use decision::{Decision, DecisionContext, DecisionEngine, HeuristicEngine, Language, LearningEngine};
pub use korean_model::KoreanModel;
pub use mixed_word::SplittingEngine;
pub use session::{
    ConversionCandidate, InputSession, JapaneseConverter, KanaConverter, Key, SharedConverter, TextClient, TextRange,
    NOT_FOUND,
};
pub use settings::Settings;
pub use vim::VimSignal;

/// The heuristic for English and Korean, plus Japanese when asked (and its model is there).
pub fn heuristic(japanese: bool) -> Arc<HeuristicEngine> {
    let model = if japanese { japanese_model::shared() } else { None };
    Arc::new(HeuristicEngine::new(KoreanModel::shared(), model))
}

/// Mixed-word splitting over the heuristic (github에, x에, hello월드, githubで).
pub fn splitting(heuristic: Arc<HeuristicEngine>) -> Arc<SplittingEngine> {
    let segmenter = heuristic.segmenter.clone();
    Arc::new(SplittingEngine::new(heuristic, english_model::shared(), segmenter))
}

/// Order of authority (Runtime.makeEngine): words the user changed with Esc (twice), then
/// mixed-word splitting, then the heuristic.
pub fn make_engine(japanese: bool, lexicon: Option<PathBuf>) -> Arc<LearningEngine> {
    Arc::new(LearningEngine::new(splitting(heuristic(japanese)), lexicon))
}
