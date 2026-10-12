//! Per-client typing state (InputSession.swift): the word being composed, the language of the
//! previous word, the last committed word (so Esc right after a commit can change it), and
//! an in-progress kana→kanji conversion.
//!
//! While a word is being composed:
//!   letters        extend it; the underlined preview shows the likely language
//!   Esc            cycle it: english → english+particle (github에) → korean → japanese
//!                  (where the app's Esc is set to go to the app, or inside a vim that reports
//!                  its modes, Esc commits and goes to the app; ⇧Esc cycles)
//!   ⇧Esc           cycle it, in every app
//!   Delete         remove the last kana of a Japanese phrase; in Korean the last jamo of the
//!                  syllable being typed, then whole syllables (as the system's 2-set does);
//!                  else the last keystroke. The word keeps its language until the next letter.
//!   Space          English/Korean: commit, then a space. Japanese: convert
//!   , . ? ! - '    inside a Japanese phrase they are part of it (、。？！ー); otherwise
//!                  they commit the word, then type the key
//!   Return         commit the word as shown, and nothing else (the next Return is the app's);
//!                  with `return_confirms_only` off the app gets it too, except after Japanese
//!   anything else  commit and let the app handle the key
//! While converting (candidates shown):
//!   Space / ↓      next candidate          ↑      previous candidate
//!   1–9            commit that candidate   Return commit the selected candidate
//!   Esc / Delete   back to kana            letters/punctuation  commit, then keep typing
//!   A candidate that covers only the start of the reading commits that part and
//!   continues converting the rest.
//! Right after a word was committed with space/punctuation:
//!   Esc / ⇧Esc     cycle that committed word the same way (Esc where it switches languages,
//!                  ⇧Esc everywhere). The same correction made twice is learned for good.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use crate::app_policy::{AppPolicies, AppPolicy};
use crate::decision::{Decision, DecisionContext, DecisionEngine, Language, LearningEngine};
use crate::english_model;
use crate::hangul::{self, Unit};
use crate::jamo::dubeolsik;
use crate::japanese_model::{self, IN_PHRASE_SYMBOLS};
use crate::korean_model::KoreanModel;
use crate::mixed_word::{SPLIT_CONFIDENCE, SUFFIXES};
use crate::romaji;
use crate::segmenter::{self, JapaneseSegmenter, Segment};
use crate::vim::VimSignal;

/// "Not found" for a `TextRange` location, like NSNotFound.
pub const NOT_FOUND: usize = usize::MAX;

/// A span of the text field in UTF-16 code units (the unit of NSString and of Windows' text
/// services alike).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TextRange {
    pub location: usize,
    pub length: usize,
}

impl TextRange {
    pub const NONE: TextRange = TextRange { location: NOT_FOUND, length: 0 };

    pub fn new(location: usize, length: usize) -> TextRange {
        TextRange { location, length }
    }

    pub fn is_found(&self) -> bool {
        self.location != NOT_FOUND
    }

    pub fn end(&self) -> Option<usize> {
        if self.is_found() { self.location.checked_add(self.length) } else { None }
    }
}

/// UTF-16 length, the unit of `TextRange`.
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// The text field the input method is typing into.
pub trait TextClient {
    /// Shows `text` underlined in place of the current marked text (or at the caret), caret at
    /// its end. Empty text removes the marked text.
    fn set_marked_text(&mut self, text: &str);
    /// `replacing == None` replaces the marked text (or the selection if nothing is marked).
    fn insert_text(&mut self, text: &str, replacing: Option<TextRange>);
    fn selected_range(&self) -> TextRange;
    fn marked_range(&self) -> TextRange;
    fn text_in(&self, range: TextRange) -> Option<String>;
    /// The app already has all but the last character of the marked text (Windows' IMM32
    /// bridge with a Korean input method): changing those means sending it Backspaces.
    fn delivers_all_but_last(&self) -> bool {
        false
    }
}

/// One conversion candidate for (a prefix of) the hiragana being converted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConversionCandidate {
    pub text: String,
    /// How many kana from the start this candidate covers. Less than the whole
    /// reading means choosing it converts the first clause and keeps converting the rest.
    pub kana_count: usize,
}

impl ConversionCandidate {
    pub fn new(text: impl Into<String>, kana_count: usize) -> ConversionCandidate {
        ConversionCandidate { text: text.into(), kana_count }
    }
}

/// Kana → kanji conversion (`AzooKeyConverter` on macOS; `KanaConverter` here; fakes in tests).
pub trait JapaneseConverter: Send {
    fn candidates(&mut self, kana: &str) -> Vec<ConversionCandidate>;
    /// The user committed `candidate`: remember it so it ranks higher next time.
    fn learn(&mut self, candidate: &ConversionCandidate, kana: &str);
    /// Persist anything learned (called on deactivation).
    fn save(&mut self);
}

pub type SharedConverter = Arc<Mutex<dyn JapaneseConverter>>;

/// The reading as itself and in katakana: what every converter offers last, and all that
/// Windows offers until it has a kanji dictionary.
pub struct KanaConverter;

impl JapaneseConverter for KanaConverter {
    fn candidates(&mut self, kana: &str) -> Vec<ConversionCandidate> {
        if kana.is_empty() {
            return Vec::new();
        }
        let count = kana.chars().count();
        let mut out = vec![ConversionCandidate::new(kana, count)];
        let katakana = romaji::katakana(kana);
        if katakana != kana {
            out.push(ConversionCandidate::new(katakana, count));
        }
        out
    }

    fn learn(&mut self, _: &ConversionCandidate, _: &str) {}

    fn save(&mut self) {}
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Letter(char),
    Delete,
    Escape,
    /// ⇧Esc: switch the word's language in every app, including those where Esc goes to the app.
    Cycle,
    /// Space, digit or punctuation typed by the input method.
    Printable(char),
    Enter,
    Up,
    Down,
    /// ←/→: while converting, shorten or lengthen the first clause; otherwise like `Other`.
    Left,
    Right,
    /// ⌃K / F7 and ⌃J / F6: the Japanese reading as katakana / hiragana.
    Katakana,
    Hiragana,
    /// Tab, shortcuts…: commit and let the app handle the key.
    Other,
}

impl Key {
    /// The kind of key, never which character (for logs: typed text stays out of them).
    pub fn kind(&self) -> &'static str {
        match self {
            Key::Letter(_) => "letter",
            Key::Delete => "delete",
            Key::Escape => "escape",
            Key::Cycle => "cycle",
            Key::Printable(_) => "printable",
            Key::Enter => "enter",
            Key::Up => "up",
            Key::Down => "down",
            Key::Left => "left",
            Key::Right => "right",
            Key::Katakana => "katakana",
            Key::Hiragana => "hiragana",
            Key::Other => "other",
        }
    }
}

/// A kana→kanji conversion in progress.
#[derive(Clone, PartialEq, Debug)]
pub struct Conversion {
    /// The reading still being converted.
    pub kana: String,
    /// The romaji that produced `kana`, so Esc can go back to editing it.
    pub keys: String,
    pub candidates: Vec<ConversionCandidate>,
    pub selected: usize,
    /// The user chose Japanese for these keys with Esc (going back to kana keeps that).
    user_forced: bool,
}

impl Conversion {
    pub fn current(&self) -> &ConversionCandidate {
        &self.candidates[self.selected]
    }

    /// What the user sees: the selected candidate, then any reading it doesn't cover.
    pub fn display(&self) -> String {
        let current = self.current();
        let rest: String = self.kana.chars().skip(current.kana_count).collect();
        format!("{}{}", current.text, rest)
    }
}

#[derive(Clone, Debug)]
struct Committed {
    keys: String,
    /// What Seamtype chose before any Esc.
    decided: Language,
    /// What it went in as (Esc after the commit changes `language`).
    committed_as: Language,
    language: Language,
    /// Exactly what was inserted: the word plus its boundary character.
    text: String,
    boundary: String,
    location: usize,
    #[allow(dead_code)]
    previous: Option<Language>,
}

/// The last decision and what it was made from, for a typing test.
#[derive(Clone, Debug)]
pub struct Trace {
    pub keys: String,
    pub previous: Option<Language>,
    pub decision: Decision,
    /// Still being typed (the preview) rather than committed.
    pub partial: bool,
}

/// The preview only switches language mid-word when the new reading is at least this likely.
pub const PREVIEW_FLIP_CONFIDENCE: f64 = 0.7;

/// How sure the engine must be of an English reading for an app that delivers early to be shown
/// it while a single Korean character would do (`early_display`). On 20,000 words of Korean chat
/// with English words in it, 0.95 cut the Backspaces sent to the app from 6.4 to 2.3 per 100
/// words, committing the same text; 4% of the English words (a, to, so, go) then show as Hangul
/// until Space.
pub const EARLY_DELIVERY_ENGLISH_CONFIDENCE: f64 = 0.95;

static KOREAN_MODEL: LazyLock<Arc<KoreanModel>> = LazyLock::new(KoreanModel::shared);

/// Full-width openers closed by `) ] }` typed after them.
fn closing(ch: char) -> Option<char> {
    match ch {
        ')' => Some('（'),
        ']' => Some('「'),
        '}' => Some('｛'),
        _ => None,
    }
}

const PAIRS: [(char, char); 5] = [('）', '（'), ('」', '「'), ('｝', '｛'), ('』', '『'), ('】', '【')];

fn count(s: &str) -> usize {
    s.chars().count()
}

fn drop_last_char(s: &mut String) {
    s.pop();
}

fn without_last(s: &str) -> String {
    let mut chars = s.chars();
    chars.next_back();
    chars.as_str().to_string()
}

fn prefix(chars: &[char], end: usize) -> String {
    chars[..end].iter().collect()
}

fn suffix(chars: &[char], start: usize) -> String {
    chars[start..].iter().collect()
}

pub struct InputSession {
    pub engine: Arc<LearningEngine>,
    pub converter: Option<SharedConverter>,
    pub japanese_enabled: bool,
    keys: String,
    app: Option<String>,
    policy: AppPolicy,
    conversion: Option<Conversion>,
    policies: AppPolicies,
    forced: Option<Language>,
    /// Whether `forced` is the user's choice (Esc) rather than "back to kana" after converting
    /// a phrase the engine read as Japanese anyway: only the user's choices are corrections.
    forced_by_user: bool,
    /// The language the engine itself showed last, before any Esc: what a commit compares
    /// against, so a forced reading is never mistaken for the engine's own choice.
    engine_preview: Option<Language>,
    /// The last `decide` kept a short token (3 keys or fewer, like a command) English only
    /// because the app is set to Strict. Longer words keep the hysteresis: applied to them it
    /// left 8% of Korean words English, against 5% this way, for the same English kept.
    kept_english_by_strict: bool,
    /// "henkan" came back from conversion as "henkan'" (the final n stays ん); dropped again
    /// if the word leaves Japanese, so English doesn't get an apostrophe nobody typed.
    added_n_marker: bool,
    /// Set through `set_previous`: back to None is a new context, which forgets open brackets.
    previous_language: Option<Language>,
    last_commit: Option<Committed>,
    /// The mode vim last reported (`vim_changed`), None when no vim is reporting. In normal mode
    /// every key goes straight through; while vim reports at all, Esc is vim's.
    vim_mode: Option<VimSignal>,
    /// The user paused Seamtype in this text field: keys go through as typed until the next
    /// toggle or another text field.
    paused: bool,
    /// Language of the current preview, so in-phrase symbols know where they belong.
    preview_language: Option<Language>,
    /// What the field shows of the current word: its reading and how many characters. It can
    /// differ from `preview_language` in an app that delivers early (`early_display`); Esc and
    /// Delete go by it.
    shown: Option<(Language, usize)>,
    /// The last key passed to the app was a digit (so "." there is a decimal point).
    after_digit: bool,
    /// Live conversion: a Japanese phrase is shown as its top kanji candidate while it is typed,
    /// and committed that way.
    pub live_conversion: bool,
    /// Return on an underlined word only confirms it (`Settings::return_confirms_only`).
    pub return_confirms_only: bool,
    /// The language shown when Delete was pressed, held until the next letter: a shorter word
    /// may read differently (한국 → 한구, not 한rn), but it shouldn't change under the user.
    pinned: Option<Language>,
    /// Where the Korean syllable being typed starts once Delete was used in the word: the system's
    /// 2-set input has committed everything before it, so those syllables delete whole and new
    /// jamo don't join them (세요 → 세ㅇ, not 셍). None until then: the word composes as one.
    syllable_start: Option<usize>,
    /// Esc showed the current phrase's kana instead (the next Esc switches language as usual).
    live_suppressed: bool,
    live_cache: Option<(String, Option<ConversionCandidate>)>,
    /// Full-width openers not closed yet in this sentence, so ) ] } close them full-width
    /// after an English word too.
    open_brackets: Vec<char>,
    /// The user's text replacements (shortcut → phrase). A word whose reading is a shortcut
    /// shows that way, so the app can replace it; a Japanese one also offers its phrase as the
    /// first candidate.
    pub replacements: HashMap<String, String>,
    /// Splits English words out of a Japanese phrase (google in kyouhagoogledekensaku).
    pub segmenter: Option<JapaneseSegmenter>,
    last_trace: Option<Trace>,
    /// The last correction handed to the learning engine, for the word being typed or just
    /// committed: going back and forth (Space, Esc, Space while converting) is one correction.
    last_recorded: Option<(String, Language)>,
}

struct FinishedWord {
    keys: String,
    text: String,
    language: Language,
    decided: Language,
}

impl InputSession {
    pub fn new(
        engine: Arc<LearningEngine>,
        policies: AppPolicies,
        converter: Option<SharedConverter>,
        japanese_enabled: bool,
        live_conversion: bool,
        return_confirms_only: bool,
    ) -> InputSession {
        let segmenter = if japanese_enabled {
            japanese_model::shared().map(|model| JapaneseSegmenter::new(english_model::shared(), model))
        } else {
            None
        };
        InputSession {
            engine,
            converter,
            japanese_enabled,
            keys: String::new(),
            app: None,
            policy: AppPolicy::STANDARD,
            conversion: None,
            policies,
            forced: None,
            forced_by_user: false,
            engine_preview: None,
            kept_english_by_strict: false,
            added_n_marker: false,
            previous_language: None,
            last_commit: None,
            vim_mode: None,
            paused: false,
            preview_language: None,
            shown: None,
            after_digit: false,
            live_conversion,
            return_confirms_only,
            pinned: None,
            syllable_start: None,
            live_suppressed: false,
            live_cache: None,
            open_brackets: Vec::new(),
            replacements: HashMap::new(),
            segmenter,
            last_trace: None,
            last_recorded: None,
        }
    }

    /// A session with the defaults: no app rules, no converter, Japanese off.
    pub fn simple(engine: Arc<LearningEngine>) -> InputSession {
        InputSession::new(engine, AppPolicies::default(), None, false, false, true)
    }

    pub fn keys(&self) -> &str {
        &self.keys
    }

    pub fn app(&self) -> Option<&str> {
        self.app.as_deref()
    }

    pub fn policy(&self) -> AppPolicy {
        self.policy
    }

    pub fn conversion(&self) -> Option<&Conversion> {
        self.conversion.as_ref()
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn last_trace(&self) -> Option<&Trace> {
        self.last_trace.as_ref()
    }

    /// Whether anything is being composed or converted (the text field shows marked text).
    pub fn is_composing(&self) -> bool {
        !self.keys.is_empty() || self.conversion.is_some()
    }

    /// Whether Esc right after a commit may still change the committed word.
    pub fn has_flip_window(&self) -> bool {
        self.last_commit.is_some()
    }

    fn set_previous(&mut self, language: Option<Language>) {
        self.previous_language = language;
        if language.is_none() {
            self.open_brackets.clear(); // a new context
        }
    }

    /// A new text field became active: pick its app's policy and forget the sentence context.
    pub fn activate(&mut self, app: Option<&str>) {
        self.close_flip_window();
        // Coming back to the same terminal leaves vim in whatever mode it was.
        if app != self.app.as_deref() {
            self.vim_mode = None;
        }
        self.app = app.map(str::to_string);
        self.policy = self.policies.policy(app);
        self.set_previous(None);
        self.paused = false;
    }

    /// Pauses or resumes Seamtype in this text field; what was being typed goes in first.
    pub fn toggle_pause(&mut self, client: Option<&mut dyn TextClient>) {
        if !self.paused {
            self.commit(client);
        }
        self.paused = !self.paused;
        self.set_previous(None);
    }

    /// Vim's mode, from `seamtype --vim` (the caller checks it came from this app).
    pub fn vim_changed(&mut self, signal: VimSignal, client: Option<&mut dyn TextClient>) {
        if signal == VimSignal::Normal && self.vim_mode != Some(VimSignal::Normal) {
            self.commit(client);
            self.set_previous(None);
        }
        self.vim_mode = if signal == VimSignal::Off { None } else { Some(signal) };
    }

    /// Whether keys currently go straight to vim (normal, visual or command-line mode).
    pub fn in_vim_normal_mode(&self) -> bool {
        self.vim_mode == Some(VimSignal::Normal)
    }

    /// Whether `handle` would pass `key` straight on without looking at it (paused, the app
    /// turned off, vim's normal mode): such keys can skip the input method entirely.
    pub fn passes_through(&self, key: Key) -> bool {
        !self.policy.enabled || self.paused || (self.in_vim_normal_mode() && key != Key::Cycle && !self.is_composing())
    }

    /// Returns true when the key was consumed.
    pub fn handle(&mut self, key: Key, client: &mut dyn TextClient) -> bool {
        if !self.policy.enabled || self.paused {
            return false;
        }
        if self.vim_mode == Some(VimSignal::Normal) {
            // Vim's normal mode: every key is a command. ⇧Esc is the way out if vim's last word
            // was lost (Vim not seeing the terminal's focus change): it only says "decide words".
            if key == Key::Cycle {
                self.vim_mode = None;
                return true;
            }
            self.set_previous(None);
            return false;
        }
        if self.conversion.is_some() {
            return self.handle_converting(key, client);
        }

        if !(matches!(key, Key::Printable(_)) && self.keys.is_empty()) {
            self.after_digit = false;
        }
        let pending = self.last_commit.take();
        // Esc switches languages unless the app owns it (or a reporting vim); ⇧Esc always does.
        let switches_language =
            key == Key::Cycle || (key == Key::Escape && !self.policy.escape_to_app && self.vim_mode.is_none());
        if switches_language && self.keys.is_empty() {
            if let Some(pending) = pending.as_ref() {
                if let Some(changed) = self.cycle(pending, client) {
                    self.last_commit = Some(changed);
                    return true;
                }
            }
        }
        if let Some(pending) = pending.as_ref() {
            self.finalize(pending);
        }

        match key {
            Key::Letter(ch) => {
                // A symbol on its own is confirmed by the next word, which is judged by itself.
                if !self.keys.is_empty() && self.keys.chars().all(romaji::is_symbol) {
                    self.commit(Some(client));
                }
                if self.keys.is_empty() {
                    // A new word.
                    self.last_recorded = None;
                    self.live_suppressed = false;
                }
                self.added_n_marker = false;
                self.pinned = None;
                self.keys.push(ch);
                self.update_preview(client);
                true
            }

            Key::Delete => {
                if self.keys.is_empty() {
                    return false;
                }
                let shown = self.shown_language();
                if self.forced.is_none() {
                    self.pinned = shown;
                }
                match shown {
                    Some(Language::Japanese) => self.delete_kana(),
                    Some(Language::Korean) => self.delete_jamo_or_syllable(),
                    _ => drop_last_char(&mut self.keys),
                }
                self.added_n_marker = false;
                if self.keys.is_empty() {
                    self.forced = None;
                    self.forced_by_user = false;
                    self.pinned = None;
                    self.syllable_start = None;
                }
                self.update_preview(client);
                true
            }

            Key::Escape | Key::Cycle => {
                if self.keys.is_empty() {
                    return false;
                }
                if !switches_language {
                    // The app's Esc: the word goes in as shown, then the app gets the key.
                    self.commit(Some(client));
                    return false;
                }
                // A live-converted phrase goes back to its kana first.
                if self.is_live() && self.forced.or(self.preview_language) == Some(Language::Japanese) {
                    self.live_suppressed = true;
                    self.update_preview(client);
                    return true;
                }
                // Cycle from what the user sees.
                let shown = match self.shown_language() {
                    Some(shown) => shown,
                    None => self.decide(true).language,
                };
                let forced = shown.next(&self.keys, self.japanese_enabled);
                self.forced = Some(forced);
                self.pinned = None;
                self.forced_by_user = true;
                if forced != Language::Japanese {
                    self.drop_n_marker();
                }
                self.update_preview(client);
                true
            }

            Key::Printable(ch) => {
                if self.keys.is_empty() {
                    // After Japanese text a symbol is Japanese too (、。？！「」・…), underlined until
                    // Return or the next word confirms it, as in a Japanese IME; Space shows its
                    // other forms. Not inside a number (3.5, 10:30), but a bracket closes the way
                    // it was opened.
                    let starts = romaji::is_symbol(ch)
                        && ((self.previous_language == Some(Language::Japanese) && !self.after_digit)
                            || self.closes_full_width(ch));
                    self.after_digit = ch.is_numeric();
                    if starts {
                        self.start_symbol(ch, client);
                        return true;
                    }
                    return false;
                }
                if self.japanese_enabled && self.belongs_to_japanese_phrase(ch) {
                    self.keys.push(ch);
                    self.update_preview(client);
                    return true;
                }
                if ch == ' ' && self.current_language() == Language::Japanese && self.converter.is_some() {
                    self.start_conversion(client);
                    return true;
                }
                if self.closes_full_width(ch) {
                    // 日本語（English）
                    self.commit(Some(client));
                    self.start_symbol(ch, client);
                    return true;
                }
                let marked = client.marked_range();
                let location = if marked.is_found() { marked.location } else { client.selected_range().location };
                let previous = self.previous_language;
                let word = self.finish_word();
                // Only the word goes in; the app types the space or punctuation itself, as with
                // the system's Korean input.
                client.insert_text(&word.text, None);
                self.last_commit = Some(Committed {
                    keys: word.keys,
                    decided: word.decided,
                    committed_as: word.language,
                    language: word.language,
                    text: format!("{}{}", word.text, ch),
                    boundary: ch.to_string(),
                    location,
                    previous,
                });
                false
            }

            Key::Enter => {
                if self.keys.is_empty() {
                    return false;
                }
                // Like a Japanese IME, Return on an underlined word only confirms it; the next
                // Return is the app's. A Japanese phrase is only confirmed either way.
                let japanese = self.current_language() == Language::Japanese;
                self.commit(Some(client));
                self.return_confirms_only || japanese
            }

            Key::Katakana | Key::Hiragana => {
                // Only for a Japanese phrase; otherwise ⌃K/⌃J stay the app's shortcuts.
                if self.keys.is_empty()
                    || !self.japanese_enabled
                    || !(self.forced == Some(Language::Japanese) || self.preview_language == Some(Language::Japanese))
                {
                    self.commit(Some(client));
                    return false;
                }
                self.show_kana_forms(key == Key::Katakana, None, None, client);
                true
            }

            Key::Up | Key::Down | Key::Left | Key::Right | Key::Other => {
                self.commit(Some(client));
                self.set_previous(None);
                false
            }
        }
    }

    /// Commits whatever is in progress as-is (focus change, shortcut, deactivation).
    pub fn commit(&mut self, mut client: Option<&mut dyn TextClient>) {
        self.close_flip_window();
        if let Some(conversion) = self.conversion.take() {
            let display = conversion.display();
            if let Some(client) = client.as_deref_mut() {
                client.insert_text(&display, None);
            }
            self.note_brackets(&display);
            if let Some(converter) = &self.converter {
                converter.lock().unwrap().learn(conversion.current(), &conversion.kana);
            }
            self.keys.clear();
            self.set_previous(Some(Language::Japanese));
            return;
        }
        if self.keys.is_empty() {
            return;
        }
        let word = self.finish_word();
        if let Some(client) = client {
            client.insert_text(&word.text, None);
        }
    }

    /// Commits what's in progress after the caret was moved elsewhere (a click; the app asks):
    /// the next word doesn't continue the previous sentence.
    pub fn commit_for_new_context(&mut self, client: Option<&mut dyn TextClient>) {
        self.commit(client);
        self.set_previous(None);
    }

    /// Forgets what is being composed without inserting anything: the text field already holds
    /// it (the app ended the composition itself and kept what it showed).
    pub fn abandon(&mut self) {
        self.close_flip_window();
        if self.conversion.take().is_some() || !self.keys.is_empty() {
            self.keys.clear();
            self.forced = None;
            self.forced_by_user = false;
            self.pinned = None;
            self.syllable_start = None;
            self.added_n_marker = false;
            self.preview_language = None;
            self.engine_preview = None;
            self.shown = None;
        }
        self.set_previous(None);
    }

    /// Commits candidate `index` of the open conversion (e.g. clicked in a candidate window).
    pub fn choose(&mut self, index: usize, client: &mut dyn TextClient) {
        let Some(conversion) = self.conversion.as_mut() else { return };
        if index >= conversion.candidates.len() {
            return;
        }
        conversion.selected = index;
        self.commit_candidate(client);
    }

    // MARK: - Japanese conversion

    /// A symbol typed as its own Japanese word, shown full-width (or as the 「」・ a Japanese IME types).
    fn start_symbol(&mut self, ch: char, client: &mut dyn TextClient) {
        self.last_recorded = None;
        self.live_suppressed = false;
        self.keys = ch.to_string();
        self.forced = Some(Language::Japanese);
        self.forced_by_user = false;
        self.update_preview(client);
    }

    fn closes_full_width(&self, ch: char) -> bool {
        match closing(ch) {
            Some(opener) => self.open_brackets.last() == Some(&opener),
            None => false,
        }
    }

    /// Keeps `open_brackets` up to date with text going in.
    fn note_brackets(&mut self, text: &str) {
        for ch in text.chars() {
            if PAIRS.iter().any(|&(_, opener)| opener == ch) {
                self.open_brackets.push(ch);
            } else if let Some(&(_, opener)) = PAIRS.iter().find(|&&(close, _)| close == ch) {
                if self.open_brackets.last() == Some(&opener) {
                    self.open_brackets.pop();
                }
            }
        }
    }

    /// Whether a symbol typed after the current word continues a Japanese phrase.
    /// Sentence punctuation (、。？！) never changes the word's language: it joins only a
    /// word already read as Japanese ("hello." and "dho?" = 왜? stay what they were).
    /// The in-word marks ー and ' may also turn an English-looking start into Japanese
    /// ("ra" → "ra-" → らー), but never take a word away from Korean.
    fn belongs_to_japanese_phrase(&self, ch: char) -> bool {
        if !IN_PHRASE_SYMBOLS.contains(&ch) {
            return false;
        }
        if self.forced == Some(Language::Japanese) {
            return true;
        }
        // On the way to a Japanese text replacement shortcut with a symbol in it: gu- is ぐー,
        // even though gu alone read as 혀.
        if self.japanese_enabled && self.forced.is_none() {
            let kana = romaji::compose(&format!("{}{}", self.keys, ch), true);
            if kana.is_clean() && self.replacements.keys().any(|k| k.starts_with(kana.text.as_str())) {
                return true;
            }
        }
        // A letter still waiting for its vowel says the word may not be Japanese after all
        // (baz) is baz), not ばz）); a final n is ん (nihon. is にほん。).
        if self.preview_language == Some(Language::Japanese) {
            let pending = romaji::compose(&self.keys, true).pending;
            return ch == '-' || ch == '\'' || pending.is_empty() || pending == "n";
        }
        // An English word before the mark keeps it English (to-do, co-op); ra-men isn't one.
        if !(self.forced.is_none()
            && (ch == '-' || ch == '\'')
            && self.preview_language == Some(Language::English)
            && !english_model::shared().is_known_word(&self.keys)
            && romaji::compose(&self.keys, true).stray_letters == 0)
        {
            return false;
        }
        let context = DecisionContext::new(self.previous_language, true);
        self.engine.decide(&format!("{}{}", self.keys, ch), &context).language == Language::Japanese
    }

    /// The Korean or Japanese reading of the keys that is a text replacement shortcut (주소, ㅈㅅ,
    /// よろ), unless the user taught Seamtype otherwise for these keys. English shortcuts are left
    /// to the engine: "go" may be one, and still be ご in a Japanese sentence.
    fn shortcut_reading(&self, partial: bool) -> Option<(Language, String)> {
        if self.replacements.is_empty() || self.keys.is_empty() || self.engine.learned_language(&self.keys).is_some() {
            return None;
        }
        if let Some(hangul) = self.hangul() {
            if self.replacements.contains_key(&hangul) {
                return Some((Language::Korean, hangul));
            }
        }
        if self.japanese_enabled {
            let kana = romaji::compose(&self.keys, partial);
            if kana.is_clean() && self.replacements.contains_key(&kana.text) {
                return Some((Language::Japanese, kana.text));
            }
        }
        None
    }

    /// Kana for the keys, with any English words inside the phrase kept as typed.
    fn japanese_reading(&self, partial: bool) -> String {
        if !partial {
            if let Some(segments) = self.segmenter.as_ref().and_then(|s| s.segment(&self.keys)) {
                return segmenter::text(&segments);
            }
        }
        // While typing, a consonant at the end is just waiting for its vowel (watashihag → わたしはg),
        // not a stray letter to split an English word ("hag") out of.
        if partial && romaji::compose(&self.keys, true).stray_letters > 0 {
            if let Some(segments) = self.segmenter.as_ref().and_then(|s| s.segment(&self.keys)) {
                // While typing, the last segment may still be growing; keep its pending letters visible.
                let (last, front) = segments.split_last().expect("segments");
                let tail = match last {
                    Segment::Japanese { keys, .. } => romaji::compose(keys, true).text,
                    other => segmenter::text(std::slice::from_ref(other)),
                };
                return format!("{}{}", segmenter::text(front), tail);
            }
        }
        romaji::compose(&self.keys, partial).text
    }

    // MARK: - Live conversion

    fn is_live(&self) -> bool {
        self.live_conversion && !self.live_suppressed && self.converter.is_some()
    }

    /// The top candidate covering all of `kana` (cached: the preview asks on every key).
    fn live_candidate(&mut self, kana: &str) -> Option<ConversionCandidate> {
        if let Some((cached_kana, candidate)) = &self.live_cache {
            if cached_kana == kana {
                return candidate.clone();
            }
        }
        let kana_count = count(kana);
        let mut candidate = self
            .converter
            .as_ref()
            .and_then(|c| c.lock().unwrap().candidates(kana).into_iter().find(|c| c.kana_count == kana_count));
        // A short reading only turns into kanji (私, 今日), not into katakana mid-word
        // (と → ト, こんに → コンに): those flicker while the phrase is still being typed.
        if kana_count <= 3 {
            if let Some(c) = &candidate {
                if !c.text.chars().any(|ch| (0x4E00..=0x9FFF).contains(&(ch as u32))) {
                    candidate = None;
                }
            }
        }
        self.live_cache = Some((kana.to_string(), candidate.clone()));
        candidate
    }

    /// The Japanese phrase as shown while typing: kana, or with live conversion the top
    /// candidate followed by letters still waiting for their vowel (今日はいい天気でs).
    fn japanese_display(&mut self) -> String {
        let reading = self.japanese_reading(true);
        if !self.is_live() {
            return reading;
        }
        let pending = romaji::compose(&self.keys, true).pending;
        if !reading.ends_with(pending.as_str()) {
            return reading;
        }
        let kana: String = {
            let chars: Vec<char> = reading.chars().collect();
            chars[..chars.len() - count(&pending)].iter().collect()
        };
        if kana.is_empty() {
            return reading;
        }
        match self.live_candidate(&kana) {
            Some(candidate) => format!("{}{}", candidate.text, pending),
            None => reading,
        }
    }

    /// What a Japanese phrase commits as without the candidates: the kana, or with live
    /// conversion the candidate that was on screen (which the converter learns).
    fn committed_japanese(&mut self) -> String {
        let kana = self.japanese_reading(false);
        if !self.is_live() {
            return kana;
        }
        let Some(candidate) = self.live_candidate(&kana) else { return kana };
        if let Some(converter) = &self.converter {
            converter.lock().unwrap().learn(&candidate, &kana);
        }
        candidate.text
    }

    fn start_conversion(&mut self, client: &mut dyn TextClient) {
        let kana = self.japanese_reading(false);
        let Some(converter) = self.converter.clone() else { return };
        if kana.is_empty() {
            return;
        }
        let user_forced = self.forced == Some(Language::Japanese) && self.forced_by_user;
        if user_forced {
            let engine_choice = self.committed_language();
            let keys = self.keys.clone();
            self.record(&keys, Language::Japanese, engine_choice);
        }
        let mut candidates = converter.lock().unwrap().candidates(&kana);
        let live = if self.is_live() { self.live_candidate(&kana) } else { None };
        if let Some(phrase) = self.replacements.get(&kana).cloned() {
            // A text replacement comes first, or right after the live candidate already on screen.
            candidates.retain(|c| c.text != phrase);
            let at = live
                .as_ref()
                .and_then(|l| candidates.iter().position(|c| c == l))
                .map(|i| i + 1)
                .unwrap_or(0);
            candidates.insert(at, ConversionCandidate::new(phrase, count(&kana)));
        }
        if candidates.is_empty() {
            self.commit(Some(client));
            return;
        }
        // With live conversion the top candidate is already on screen: Space shows the next one.
        let mut selected = 0;
        if let Some(live) = &live {
            if let Some(i) = candidates.iter().position(|c| c == live) {
                if candidates.len() > 1 {
                    selected = (i + 1) % candidates.len();
                }
            }
        }
        self.live_suppressed = false;
        let conversion = Conversion { kana, keys: self.keys.clone(), candidates, selected, user_forced };
        let display = conversion.display();
        self.conversion = Some(conversion);
        self.keys.clear();
        self.forced = None;
        self.forced_by_user = false;
        self.pinned = None;
        self.syllable_start = None;
        self.engine_preview = None;
        self.shown = None;
        self.set_previous(Some(Language::Japanese));
        client.set_marked_text(&display);
    }

    /// Shows the reading as katakana (or hiragana) as the selected "candidate": Return
    /// commits it, Space flips to the other form, typing on commits it and continues.
    fn show_kana_forms(&mut self, katakana_first: bool, kana: Option<String>, romaji_keys: Option<String>, client: &mut dyn TextClient) {
        let kana = kana.unwrap_or_else(|| self.japanese_reading(false));
        let kana_count = count(&kana);
        let mut forms = vec![
            ConversionCandidate::new(romaji::katakana(&kana), kana_count),
            ConversionCandidate::new(kana.clone(), kana_count),
        ];
        if !katakana_first {
            forms.reverse();
        }
        let keys = romaji_keys.unwrap_or_else(|| self.keys.clone());
        let conversion = Conversion { kana, keys, candidates: forms, selected: 0, user_forced: false };
        let display = conversion.display();
        self.conversion = Some(conversion);
        self.keys.clear();
        self.forced = None;
        self.forced_by_user = false;
        self.pinned = None;
        self.syllable_start = None;
        self.engine_preview = None;
        self.shown = None;
        self.set_previous(Some(Language::Japanese));
        client.set_marked_text(&display);
    }

    /// The conversion with its first clause `step` kana shorter or longer (今日は|いいてんき):
    /// candidates for exactly that much of the reading, the rest shown as kana. A boundary
    /// never falls inside an English word (google). None when it can't move further.
    fn resize_first_clause(&mut self, conversion: &Conversion, step: isize) -> Option<Conversion> {
        let converter = self.converter.clone()?;
        let reading: Vec<char> = conversion.kana.chars().collect();
        let n = reading.len() as isize;
        let is_ascii_letter = |c: char| c.is_ascii() && c.is_alphabetic();
        let mut length = conversion.current().kana_count as isize + step;
        while length >= 1
            && length < n
            && is_ascii_letter(reading[(length - 1) as usize])
            && is_ascii_letter(reading[length as usize])
        {
            length += step;
        }
        if !(length >= 1 && length <= n) || length == conversion.current().kana_count as isize {
            return None;
        }
        let clause: String = reading[..length as usize].iter().collect();
        let clause_count = length as usize;
        let candidates: Vec<ConversionCandidate> =
            converter.lock().unwrap().candidates(&clause).into_iter().filter(|c| c.kana_count == clause_count).collect();
        if candidates.is_empty() {
            return None;
        }
        Some(Conversion {
            kana: conversion.kana.clone(),
            keys: conversion.keys.clone(),
            candidates,
            selected: 0,
            user_forced: conversion.user_forced,
        })
    }

    fn handle_converting(&mut self, key: Key, client: &mut dyn TextClient) -> bool {
        let Some(mut conversion) = self.conversion.clone() else { return false };
        match key {
            Key::Printable(' ') | Key::Down => {
                conversion.selected = (conversion.selected + 1) % conversion.candidates.len();
                client.set_marked_text(&conversion.display());
                self.conversion = Some(conversion);
                true
            }

            Key::Up => {
                let n = conversion.candidates.len();
                conversion.selected = (conversion.selected + n - 1) % n;
                client.set_marked_text(&conversion.display());
                self.conversion = Some(conversion);
                true
            }

            Key::Left | Key::Right => {
                let step = if key == Key::Left { -1 } else { 1 };
                if let Some(resized) = self.resize_first_clause(&conversion, step) {
                    client.set_marked_text(&resized.display());
                    self.conversion = Some(resized);
                }
                true
            }

            Key::Printable(ch) if ch.is_numeric() => {
                if let Some(n) = ch.to_digit(10) {
                    let n = n as usize;
                    if n >= 1 && n <= 9.min(conversion.candidates.len()) {
                        self.choose(n - 1, client);
                        return true;
                    }
                }
                self.commit(Some(client));
                self.handle(key, client)
            }

            Key::Enter => {
                self.commit_candidate(client);
                true
            }

            Key::Katakana | Key::Hiragana => {
                self.conversion = None;
                self.show_kana_forms(key == Key::Katakana, Some(conversion.kana), Some(conversion.keys), client);
                true
            }

            Key::Escape | Key::Cycle | Key::Delete => {
                self.conversion = None;
                self.live_suppressed = true; // "back to kana" means kana, also with live conversion
                // Back to editing the romaji — when it still spells the reading. After part of a
                // phrase with English in it was committed it may not; keep the kana rather than lose it.
                if segmenter::reading(&conversion.keys, self.segmenter.as_ref()) != conversion.kana {
                    client.insert_text(&conversion.kana, None);
                    self.set_previous(Some(Language::Japanese));
                    return true;
                }
                // A final "n" was shown as ん; keep it ("henkan" → "henkan'") instead of
                // letting it wait for a vowel again.
                self.keys = conversion.keys.clone();
                self.syllable_start = None;
                if self.keys.ends_with('n') && !self.keys.ends_with("nn") {
                    self.keys.push('\'');
                    self.added_n_marker = true;
                }
                self.forced = Some(Language::Japanese);
                self.forced_by_user = conversion.user_forced;
                self.update_preview(client);
                true
            }

            Key::Letter(_) | Key::Printable(_) => {
                self.commit(Some(client));
                self.handle(key, client)
            }

            Key::Other => {
                self.commit(Some(client));
                false
            }
        }
    }

    /// Commits the selected candidate; if it covers only the start of the reading,
    /// keeps converting the rest.
    fn commit_candidate(&mut self, client: &mut dyn TextClient) {
        let Some(conversion) = self.conversion.take() else { return };
        let candidate = conversion.current().clone();
        if let Some(converter) = &self.converter {
            converter.lock().unwrap().learn(&candidate, &conversion.kana);
        }
        client.insert_text(&candidate.text, None);
        self.note_brackets(&candidate.text);
        let rest: String = conversion.kana.chars().skip(candidate.kana_count).collect();
        if rest.is_empty() {
            return;
        }
        let rest_keys = Self::keys_after(&conversion.keys, candidate.kana_count);
        let candidates = match &self.converter {
            Some(converter) => converter.lock().unwrap().candidates(&rest),
            None => Vec::new(),
        };
        if candidates.is_empty() {
            client.insert_text(&rest, None);
        } else {
            // The rest is the engine's Japanese, not something the user corrected.
            let conversion = Conversion { kana: rest, keys: rest_keys, candidates, selected: 0, user_forced: false };
            client.set_marked_text(&conversion.display());
            self.conversion = Some(conversion);
        }
    }

    /// The romaji left after the part that produced the first `kana_count` kana: the
    /// shortest tail that composes to exactly the remaining kana ("kitte" after きっ is
    /// "te"; "konnichiha" after こん is "nichiha").
    pub fn keys_after(keys: &str, kana_count: usize) -> String {
        let chars: Vec<char> = keys.chars().collect();
        let rest: String = romaji::compose(keys, false).text.chars().skip(kana_count).collect();
        for i in (0..=chars.len()).rev() {
            let tail = suffix(&chars, i);
            if romaji::compose(&tail, false).text == rest {
                return tail;
            }
        }
        String::new()
    }

    // MARK: - Private

    fn decide(&mut self, partial: bool) -> Decision {
        if let Some((language, text)) = self.shortcut_reading(partial) {
            // As sure as a split word, so a Strict app shows it too.
            self.kept_english_by_strict = false;
            let decision = Decision::new(language, SPLIT_CONFIDENCE, text, "text replacement shortcut");
            self.last_trace =
                Some(Trace { keys: self.keys.clone(), previous: self.previous_language, decision: decision.clone(), partial });
            return decision;
        }
        let d = self.engine.decide(&self.keys, &DecisionContext::new(self.previous_language, partial));
        let threshold = self.policy.conversion_threshold;
        let kept = d.language != Language::English && d.confidence < threshold;
        self.kept_english_by_strict = kept && threshold >= AppPolicy::STRICT_THRESHOLD && count(&self.keys) <= 3;
        let decision = if kept {
            Decision::new(
                Language::English,
                1.0 - d.confidence,
                self.keys.clone(),
                format!("kept English ({:.2} < {:.2}); {}", d.confidence, threshold, d.explanation),
            )
        } else {
            d
        };
        self.last_trace =
            Some(Trace { keys: self.keys.clone(), previous: self.previous_language, decision: decision.clone(), partial });
        decision
    }

    /// The language the current word will be committed in.
    fn current_language(&mut self) -> Language {
        if let Some(language) = self.forced.or(self.pinned) {
            return language;
        }
        self.committed_language()
    }

    /// What Space/Return commits without Esc: the full-word decision, except that a hesitant
    /// change away from the reading on screen doesn't win (unless Strict kept it English). The
    /// same rule as the preview, so what you see is what you get.
    fn committed_language(&mut self) -> Language {
        let d = self.decide(false);
        if let Some(shown) = self.engine_preview {
            if shown != d.language && !self.is_weak_flip_allowed(&d, shown) {
                return shown;
            }
        }
        d.language
    }

    /// Whether `decision` may replace the reading on screen: when it's confident, when it's
    /// a dictionary English word (which is what Space/Return will commit), or when a Strict app
    /// kept a short token English: there the typed letters win over a reading shown along the
    /// way (gs showed ㅎㄴ, but gst stays gst).
    fn is_weak_flip_allowed(&self, decision: &Decision, shown: Language) -> bool {
        decision.confidence >= PREVIEW_FLIP_CONFIDENCE
            || (decision.language == Language::English
                && (self.kept_english_by_strict || english_model::shared().is_known_word(&self.keys)))
            || self.preview_as(shown).is_none()
    }

    fn update_preview(&mut self, client: &mut dyn TextClient) {
        if self.keys.is_empty() {
            self.preview_language = None;
            self.engine_preview = None;
            self.shown = None;
            client.set_marked_text("");
        } else if let Some(forced) = self.forced.or(self.pinned) {
            self.preview_language = Some(forced);
            let text = match forced {
                Language::Japanese => self.japanese_display(),
                Language::Korean => self.hangul().unwrap_or_else(|| self.keys.clone()),
                _ => self.engine.render(&self.keys, forced),
            };
            self.shown = Some((forced, text.chars().count()));
            client.set_marked_text(&text);
        } else {
            self.split_off_korean_before_other(client);
            self.split_off_japanese_before_korean(client);
            let decision = self.decide(true);
            let mut language = decision.language;
            // Hysteresis: a weak flip on one keystroke (わた → "wat" → わたし) isn't shown,
            // except into a dictionary English word, which is what Space/Return will commit.
            if let Some(shown) = self.preview_language {
                if shown != language && count(&self.keys) > 1 && !self.is_weak_flip_allowed(&decision, shown) {
                    language = shown;
                }
            }
            self.preview_language = Some(language);
            self.engine_preview = Some(language);
            let display = if client.delivers_all_but_last() { self.early_display(language, &decision) } else { language };
            let text = if display == Language::Japanese {
                self.japanese_display()
            } else {
                self.preview_as(display).unwrap_or_else(|| self.keys.clone())
            };
            self.shown = Some((display, text.chars().count()));
            client.set_marked_text(&text);
        }
    }

    /// The reading on screen: the user's choice, one kept while deleting, or the preview.
    fn shown_language(&self) -> Option<Language> {
        self.forced.or(self.pinned).or(self.shown.map(|(language, _)| language)).or(self.preview_language)
    }

    /// What an app that delivers early (`TextClient::delivers_all_but_last`) is shown while the
    /// engine reads `language`. An English reading hands its letters over at once, and if the
    /// word then turns Korean they have to be erased with Backspaces sent to the app (an, ant →
    /// 무슨). So while the engine is unsure, nothing has been handed over and the Korean reading is
    /// one character (all of it still composing, free to change), that is shown instead. Common
    /// English words of three letters or more (the, and) stay English, and so does everything in
    /// a Strict app. What Space commits doesn't change: the engine's preview stays the same.
    fn early_display(&self, language: Language, decision: &Decision) -> Language {
        if language != Language::English
            || decision.confidence >= EARLY_DELIVERY_ENGLISH_CONFIDENCE
            || self.shown.is_some_and(|(_, characters)| characters > 1)
            || self.policy.conversion_threshold >= AppPolicy::STRICT_THRESHOLD
            || (count(&self.keys) >= 3 && english_model::shared().is_common_word(&self.keys))
        {
            return language;
        }
        match self.hangul() {
            Some(text) if text.chars().count() == 1 => Language::Korean,
            _ => language,
        }
    }

    /// Korean with another language typed straight after it reads as broken Hangul: 이력 then
    /// rireki (dlfurrireki, 이력갹다ㅑ), 오늘은 then google에서 (dhsmfdmsgoogledptj, 오늘은해ㅐ힏에서).
    /// When the keys only make Hangul with stray jamo, but a front of two or more ordinary
    /// syllables is followed by a known English word (or one with a particle) or by romaji the
    /// engine is sure is Japanese, the Korean goes in as typed and the rest carries on as its own
    /// word (live conversion and Space for candidates when it's Japanese: 이력履歴).
    fn split_off_korean_before_other(&mut self, client: &mut dyn TextClient) {
        if count(&self.keys) < 7
            || !hangul::compose(&self.keys).map(|c| c.is_broken()).unwrap_or(false)
            // Only what reads as Korean on its own: a Japanese phrase whose start happens to
            // spell Hangul (chanwo…, 초무をころした) stays whole. Judged without context, since
            // right after Japanese everything leans Japanese.
            || self.engine.decide(&self.keys, &DecisionContext::new(None, true)).language != Language::Korean
        {
            return;
        }
        let chars: Vec<char> = self.keys.chars().collect();
        for cut in 4..=(chars.len() - 3) {
            let front = prefix(&chars, cut);
            let back = suffix(&chars, cut);
            let Some(korean) = hangul::compose(&front) else { continue };
            if korean.has_stray_jamo() || korean.syllable_count() < 2 || !KOREAN_MODEL.all_syllables_known(&korean) {
                continue;
            }
            if !self.is_other_language_word(&back) {
                continue;
            }
            client.insert_text(&korean.text(), None);
            self.keys = back;
            self.syllable_start = None;
            self.set_previous(Some(Language::Korean));
            self.preview_language = None;
            self.engine_preview = None;
            self.shown = None;
            return;
        }
    }

    /// A known English word, one with a Korean particle (google에서), or romaji the engine is
    /// sure is Japanese (no word starts with っ or ん: that cut takes a letter from the Korean).
    fn is_other_language_word(&self, keys: &str) -> bool {
        let decision = self.engine.decide(keys, &DecisionContext::new(Some(Language::Korean), true));
        match decision.language {
            Language::English => count(keys) >= 3 && english_model::shared().is_known_word(keys),
            Language::Mixed => true,
            Language::Japanese => {
                if !self.japanese_enabled || decision.confidence < 0.9 || count(keys) < 4 {
                    return false;
                }
                let reading = romaji::compose(keys, true);
                reading.stray_letters == 0 && reading.text.chars().next().map(|c| c != 'っ' && c != 'ん').unwrap_or(false)
            }
            Language::Korean => false,
        }
    }

    /// Japanese with Korean typed straight after it reads as broken Hangul: a particle
    /// (rirekifmf, 履歴를, not 갹다ㅑ를) or a word (kyouhaehtl…, 今日は도시락은). Once the Korean
    /// part is a particle or two ordinary syllables, the Japanese goes in (kanji with live
    /// conversion) and the rest carries on as Korean, so 를요, 에서 or 도시락은 can follow.
    fn split_off_japanese_before_korean(&mut self, client: &mut dyn TextClient) {
        if !self.japanese_enabled
            || count(&self.keys) < 5
            || !hangul::compose(&self.keys).map(|c| c.is_broken()).unwrap_or(true)
            // Keys that still spell romaji are more Japanese (kyouhago is きょうはご, not きょうは해).
            || romaji::compose(&self.keys, true).stray_letters == 0
        {
            return;
        }
        let chars: Vec<char> = self.keys.chars().collect();
        // The longest Japanese front first: きょうは|도시, not きょう|하ㄷ….
        for cut in (3..=chars.len() - 1).rev() {
            let front = prefix(&chars, cut);
            let back = suffix(&chars, cut);
            if !dubeolsik::starts_syllable(chars[cut]) {
                continue;
            }
            let Some(korean) = hangul::compose(&back) else { continue };
            if korean.has_stray_jamo() {
                continue;
            }
            let text = korean.text();
            if !(SUFFIXES.contains(text.as_str())
                || (korean.syllable_count() >= 2 && KOREAN_MODEL.all_syllables_known(&korean)))
            {
                continue;
            }
            if !romaji::compose(&front, false).is_clean() {
                continue;
            }
            let decision = self.engine.decide(&front, &DecisionContext::new(self.previous_language, false));
            if decision.language != Language::Japanese || decision.confidence < 0.9 {
                continue;
            }
            self.keys = front;
            let japanese = self.committed_japanese();
            client.insert_text(&japanese, None);
            self.keys = back;
            self.syllable_start = None;
            self.set_previous(Some(Language::Japanese));
            self.forced = Some(Language::Korean);
            self.forced_by_user = false;
            self.preview_language = None;
            self.engine_preview = None;
            self.shown = None;
            return;
        }
    }

    /// The word so far rendered in `language`, or None when it has no such reading.
    fn preview_as(&self, language: Language) -> Option<String> {
        match language {
            Language::English => Some(self.keys.clone()),
            Language::Korean => self.hangul(),
            Language::Mixed => {
                let text = self.engine.render(&self.keys, Language::Mixed);
                if text == self.keys { None } else { Some(text) }
            }
            Language::Japanese => {
                if self.japanese_enabled { Some(self.japanese_reading(true)) } else { None }
            }
        }
    }

    /// Decides the current word, learns an Esc override, and resets the buffer.
    fn finish_word(&mut self) -> FinishedWord {
        let engine_choice = self.committed_language();
        let language = self.forced.or(self.pinned).unwrap_or(engine_choice);
        if language != Language::Japanese {
            self.drop_n_marker();
        }
        // "Back to kana" without the user's Esc, or a reading kept while deleting, isn't a
        // correction: report it as the engine's.
        let decided = if (self.forced.is_some() && !self.forced_by_user) || (self.forced.is_none() && self.pinned.is_some()) {
            language
        } else {
            engine_choice
        };
        if self.forced.is_some() && self.forced_by_user {
            let keys = self.keys.clone();
            self.record(&keys, language, decided);
        }
        let text = match language {
            Language::Japanese => self.committed_japanese(),
            Language::Korean => self.hangul().unwrap_or_else(|| self.engine.render(&self.keys, language)),
            _ => self.engine.render(&self.keys, language),
        };
        self.live_suppressed = false;
        self.note_brackets(&text);
        let word = FinishedWord { keys: std::mem::take(&mut self.keys), text, language, decided };
        self.set_previous(Some(language.sentence_language()));
        self.forced = None;
        self.forced_by_user = false;
        self.pinned = None;
        self.syllable_start = None;
        self.added_n_marker = false;
        self.preview_language = None;
        self.engine_preview = None;
        self.shown = None;
        word
    }

    /// Takes the last kana off a Japanese phrase, as a Japanese IME does (かんこく → かんこ, not
    /// かんこk): the keys become a spelling of the shorter reading, keeping as much of what was
    /// typed as composes to it. A letter still waiting for its vowel goes alone (かんこk → かんこ),
    /// and so does anything in a phrase with English in it.
    fn delete_kana(&mut self) {
        let composed = romaji::compose(&self.keys, true);
        if !composed.pending.is_empty() || composed.stray_letters != 0 || composed.text.is_empty() {
            drop_last_char(&mut self.keys);
            return;
        }
        let shorter = without_last(&composed.text);
        self.keys = Self::keys_spelling(&shorter, &self.keys).unwrap_or_else(|| without_last(&self.keys));
    }

    /// The longest start of `keys` that composes to a start of `kana`, followed by a spelling of
    /// the rest ("kya" for き is "ki"; "kitte" for きっ is "kiltu").
    pub fn keys_spelling(kana: &str, keys: &str) -> Option<String> {
        let chars: Vec<char> = keys.chars().collect();
        for cut in (0..chars.len()).rev() {
            let front = prefix(&chars, cut);
            let composed = romaji::compose(&front, true);
            if !composed.pending.is_empty() || composed.stray_letters != 0 || !kana.starts_with(composed.text.as_str()) {
                continue;
            }
            let rest_kana: String = kana.chars().skip(count(&composed.text)).collect();
            let Some(rest) = romaji::romaji(&rest_kana) else { continue };
            let respelled = romaji::compose(&format!("{front}{rest}"), true);
            if respelled.pending.is_empty() && respelled.text == kana {
                return Some(format!("{front}{rest}"));
            }
        }
        None
    }

    /// Korean as the system's 2-set input deletes it: jamo by jamo in the syllable being typed
    /// (한국 → 한구 → 한ㄱ → 한), then whole syllables, which it had already committed (한 → nothing).
    fn delete_jamo_or_syllable(&mut self) {
        let chars: Vec<char> = self.keys.chars().collect();
        if self.syllable_start.is_none() {
            let Some(composition) = hangul::compose(&self.keys) else {
                drop_last_char(&mut self.keys);
                return;
            };
            let earlier = &composition.units[..composition.units.len().saturating_sub(1)];
            self.syllable_start = Some(Self::cut(&chars, earlier));
        }
        if self.syllable_start.unwrap() < chars.len() {
            drop_last_char(&mut self.keys);
            return;
        }
        // Nothing left being typed: a whole committed syllable goes.
        let Some(composition) = hangul::compose(&self.keys) else {
            drop_last_char(&mut self.keys);
            return;
        };
        let earlier = &composition.units[..composition.units.len().saturating_sub(1)];
        let cut = Self::cut(&chars, earlier);
        self.keys = prefix(&chars, cut);
        self.syllable_start = Some(cut);
    }

    /// Where the keys of the last syllable start: the longest front that makes `earlier` alone.
    fn cut(chars: &[char], earlier: &[Unit]) -> usize {
        (0..chars.len())
            .rev()
            .find(|&i| hangul::compose(&prefix(chars, i)).map(|c| c.units == earlier).unwrap_or(false))
            .unwrap_or(chars.len().saturating_sub(1))
    }

    /// The word as Hangul, with the syllable being typed kept apart from those before it
    /// after a Delete (`syllable_start`).
    fn hangul(&self) -> Option<String> {
        let chars: Vec<char> = self.keys.chars().collect();
        match self.syllable_start {
            Some(start) if start < chars.len() => {
                let front = hangul::compose(&prefix(&chars, start))?;
                let back = hangul::compose(&suffix(&chars, start))?;
                Some(front.text() + &back.text())
            }
            _ => hangul::compose(&self.keys).map(|c| c.text()),
        }
    }

    fn drop_n_marker(&mut self) {
        if self.added_n_marker && self.keys.ends_with('\'') {
            self.keys.pop();
        }
        self.added_n_marker = false;
    }

    fn record(&mut self, keys: &str, language: Language, engine_choice: Language) {
        if let Some((last_keys, last_language)) = &self.last_recorded {
            if last_keys == keys && *last_language == language {
                return;
            }
        }
        self.last_recorded = Some((keys.to_string(), language));
        self.engine.learn(keys, language, Some(engine_choice));
    }

    /// Replaces a just-committed word with its next rendering, but only if the caret
    /// still sits right after it and the text there is what we inserted.
    fn cycle(&mut self, commit: &Committed, client: &mut dyn TextClient) -> Option<Committed> {
        let range = TextRange::new(commit.location, utf16_len(&commit.text));
        let cursor = client.selected_range();
        if !cursor.is_found() || cursor.length != 0 || Some(cursor.location) != range.end() {
            return None;
        }
        if client.text_in(range).as_deref() != Some(commit.text.as_str()) {
            return None;
        }
        let language = commit.language.next(&commit.keys, self.japanese_enabled);
        let reading = if language == Language::Japanese {
            segmenter::reading(&commit.keys, self.segmenter.as_ref())
        } else {
            self.engine.render(&commit.keys, language)
        };
        let text = format!("{}{}", reading, commit.boundary);
        client.insert_text(&text, Some(range));
        self.set_previous(Some(language.sentence_language()));
        Some(Committed {
            keys: commit.keys.clone(),
            decided: commit.decided,
            committed_as: commit.committed_as,
            language,
            text,
            boundary: commit.boundary.clone(),
            location: commit.location,
            previous: commit.previous,
        })
    }

    fn close_flip_window(&mut self) {
        if let Some(pending) = self.last_commit.take() {
            self.finalize(&pending);
        }
    }

    /// The undo window closed: learn where the word ended up (once, not every reading Esc
    /// passed through on the way).
    fn finalize(&mut self, commit: &Committed) {
        if commit.language != commit.committed_as {
            self.record(&commit.keys, commit.language, commit.decided);
        }
    }
}
