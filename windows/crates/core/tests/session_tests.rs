//! InputSessionTests, MixedWordSessionTests, SessionPolicyTests, PauseTests, StrictSplitTests,
//! VimSessionTests, JapaneseSessionTests and TextReplacementTests of the Swift package.

mod common;

use std::collections::{BTreeMap, HashMap};

use common::*;
use seamtype_core::session::utf16_len;
use seamtype_core::{AppPolicies, AppPolicy, InputSession, Key, Language, Override, SharedConverter, VimSignal};

fn type_keys(session: &mut InputSession, field: &mut FakeTextField, s: &str) {
    type_script(session, field, s);
}

// MARK: - InputSessionTests

fn plain() -> (InputSession, FakeTextField) {
    (InputSession::simple(learning(shared(), "seamtype-session")), FakeTextField::new())
}

#[test]
fn types_mixed_sentence() {
    let (mut session, mut field) = plain();
    type_keys(&mut session, &mut field, "dkssudgktpdy hello ");
    assert_eq!(field.text(), "안녕하세요 hello ");
}

#[test]
fn esc_after_commit_flips_the_word_and_learns_it() {
    let (mut session, mut field) = plain();
    type_keys(&mut session, &mut field, "hello ");
    assert_eq!(field.text(), "hello ");
    assert!(session.handle(Key::Escape, &mut field));
    assert_eq!(field.text(), "ㅗ디ㅣㅐ ");
    assert_eq!(field.cursor, utf16_len("ㅗ디ㅣㅐ "));
    // Esc again flips back.
    assert!(session.handle(Key::Escape, &mut field));
    assert_eq!(field.text(), "hello ");
    // Flipping back to the engine's own choice clears the learned override.
    type_keys(&mut session, &mut field, "hello ");
    assert_eq!(field.text(), "hello hello ");
}

/// One Esc changes only that word (a habitual Esc shouldn't stick); the same
/// correction a second time is remembered.
#[test]
fn flipped_choice_is_remembered_the_second_time() {
    let (mut session, mut field) = plain();
    type_keys(&mut session, &mut field, "rjtm ");
    let first = field.text();
    session.handle(Key::Escape, &mut field);
    let flipped = field.text();
    assert_ne!(flipped, first);
    type_keys(&mut session, &mut field, "rjtm ");
    assert_eq!(field.text(), format!("{flipped}{first}"));
    session.handle(Key::Escape, &mut field);
    assert_eq!(field.text(), format!("{flipped}{flipped}"));
    type_keys(&mut session, &mut field, "rjtm ");
    assert_eq!(field.text(), format!("{flipped}{flipped}{flipped}"));
}

#[test]
fn flip_only_applies_to_the_word_before_the_caret() {
    let (mut session, mut field) = plain();
    type_keys(&mut session, &mut field, "dkssud ");
    field.cursor = 0; // user clicked elsewhere
    assert!(!session.handle(Key::Escape, &mut field));
    assert_eq!(field.text(), "안녕 ");
}

#[test]
fn esc_passes_through_when_text_cannot_be_verified() {
    let (mut session, mut field) = plain();
    field.readable = false;
    type_keys(&mut session, &mut field, "dkssud ");
    assert!(!session.handle(Key::Escape, &mut field));
    assert_eq!(field.text(), "안녕 ");
}

#[test]
fn any_other_key_ends_the_flip_window() {
    let (mut session, mut field) = plain();
    type_keys(&mut session, &mut field, "dkssud ");
    assert!(!session.handle(Key::Other, &mut field));
    assert!(!session.handle(Key::Escape, &mut field));
    assert_eq!(field.text(), "안녕 ");
}

#[test]
fn esc_while_composing_flips_the_preview() {
    let (mut session, mut field) = plain();
    type_keys(&mut session, &mut field, "dkssud");
    assert_eq!(field.text(), "안녕");
    session.handle(Key::Escape, &mut field);
    assert_eq!(field.text(), "dkssud");
    type_keys(&mut session, &mut field, " ");
    assert_eq!(field.text(), "dkssud ");
}

#[test]
fn delete_removes_keystrokes() {
    let (mut session, mut field) = plain();
    type_keys(&mut session, &mut field, "dkssud");
    session.handle(Key::Delete, &mut field);
    assert_eq!(field.text(), "안녀");
}

/// As the system's 2-set input: the syllable being typed goes jamo by jamo, earlier ones whole.
#[test]
fn delete_korean_like_the_system() {
    let (mut session, mut field) = plain();
    type_keys(&mut session, &mut field, "gksrnr");
    assert_eq!(field.text(), "한국");
    for expected in ["한구", "한ㄱ", "한", ""] {
        session.handle(Key::Delete, &mut field);
        assert_eq!(field.text(), expected);
    }
    type_keys(&mut session, &mut field, "dkssudgktpdy");
    for expected in ["안녕하세ㅇ", "안녕하세", "안녕하", "안녕"] {
        session.handle(Key::Delete, &mut field);
        assert_eq!(field.text(), expected);
    }
    type_keys(&mut session, &mut field, "gk");
    assert_eq!(field.text(), "안녕하");
    session.handle(Key::Delete, &mut field);
    assert_eq!(field.text(), "안녕ㅎ");
}

/// After a Delete the syllable being typed stays apart from the ones before, as in the system
/// input: 세요 → 세ㅇ → 세유 (not 셍, 셍ㅠ).
#[test]
fn retyping_after_delete_keeps_syllables_apart() {
    let (mut session, mut field) = plain();
    type_keys(&mut session, &mut field, "dkssudgktpdy");
    session.handle(Key::Delete, &mut field);
    type_keys(&mut session, &mut field, "b ");
    assert_eq!(field.text(), "안녕하세유 ");
}

#[test]
fn delete_keeps_the_language_shown() {
    let (mut session, mut field) = plain();
    type_keys(&mut session, &mut field, "gksrnr");
    session.handle(Key::Delete, &mut field);
    session.handle(Key::Delete, &mut field);
    assert_eq!(field.text(), "한ㄱ");
    type_keys(&mut session, &mut field, " ");
    assert_eq!(field.text(), "한ㄱ ");
}

#[test]
fn delete_english_key_by_key() {
    let (mut session, mut field) = plain();
    type_keys(&mut session, &mut field, "hello");
    session.handle(Key::Delete, &mut field);
    assert_eq!(field.text(), "hell");
}

// MARK: - MixedWordSessionTests

fn mixed_session(japanese: bool) -> InputSession {
    let base = if japanese { trilingual_splitting() } else { splitting() };
    InputSession::new(learning(base, "seamtype-mixed"), AppPolicies::default(), None, japanese, false, true)
}

/// The English + Korean word split is what gets typed, not only what the engine decides.
#[test]
fn types_english_then_korean_word() {
    let mut field = FakeTextField::new();
    type_keys(&mut mixed_session(false), &mut field, "hellodnjfem githubtjqj ");
    assert_eq!(field.text(), "hello월드 github서버 ");
}

/// Return on an underlined word only confirms it; with nothing underlined it is the app's.
#[test]
fn return_confirms_then_goes_to_the_app() {
    let mut session = mixed_session(false);
    let mut field = FakeTextField::new();
    type_keys(&mut session, &mut field, "dkssud");
    assert!(session.handle(Key::Enter, &mut field));
    assert!(field.text() == "안녕" && !field.is_marked());
    assert!(!session.handle(Key::Enter, &mut field));
}

/// With the setting off, Return confirms the word and the app gets it too.
#[test]
fn return_goes_on_when_it_doesnt_only_confirm() {
    let mut session =
        InputSession::new(learning(shared(), "seamtype-return"), AppPolicies::default(), None, false, false, false);
    let mut field = FakeTextField::new();
    type_keys(&mut session, &mut field, "dkssud");
    assert!(!session.handle(Key::Enter, &mut field));
    assert!(field.text() == "안녕" && !field.is_marked());
}

/// A single letter with a particle (a variable name, an axis) is typed as such.
#[test]
fn types_single_letter_with_particle() {
    let mut field = FakeTextField::new();
    type_keys(&mut mixed_session(false), &mut field, "xdp ndl Xfmf Xeh ");
    assert_eq!(field.text(), "x에 n이 X를 X도 ");
}

/// Short English ending in the keys of 도 or 나 stays English.
#[test]
fn keeps_short_english_ending_in_eh_or_sk() {
    let mut field = FakeTextField::new();
    type_keys(&mut mixed_session(false), &mut field, "heh meh tsk ");
    assert_eq!(field.text(), "heh meh tsk ");
}

/// Korean then romaji with no space: the Korean goes in, the rest is a Japanese word.
#[test]
fn types_korean_then_japanese_word() {
    let mut field = FakeTextField::new();
    type_keys(&mut mixed_session(true), &mut field, "qusruddlfurrireki dlfurrireki ");
    assert_eq!(field.text(), "변경이력りれき 이력りれき ");
}

/// A Japanese word with a Korean particle: the word as Japanese, the particle as Korean.
#[test]
fn types_japanese_word_then_particle() {
    let mut field = FakeTextField::new();
    type_keys(&mut mixed_session(true), &mut field, "duwjsgl rirekifmf rirekidptj ");
    assert_eq!(field.text(), "여전히 りれき를 りれき에서 ");
}

/// A Japanese phrase with a Korean word straight after it.
#[test]
fn types_japanese_then_korean_word() {
    let mut field = FakeTextField::new();
    type_keys(&mut mixed_session(true), &mut field, "kyouhaehtlfkrdms ");
    assert_eq!(field.text(), "きょうは도시락은 ");
}

/// Korean then English with no space: 오늘은 then google에서.
#[test]
fn types_korean_then_english_word() {
    let mut field = FakeTextField::new();
    type_keys(&mut mixed_session(true), &mut field, "dhsmfdmsgoogledptj dlrjtdmsgithubdp ");
    assert_eq!(field.text(), "오늘은google에서 이것은github에 ");
}

#[test]
fn plain_korean_stays_whole() {
    let mut field = FakeTextField::new();
    type_keys(&mut mixed_session(true), &mut field, "dkssudgktpdy qusruddlfur rhkswjadmfh ");
    assert_eq!(field.text(), "안녕하세요 변경이력 관점으로 ");
    // A consonant just typed for the next syllable isn't broken Hangul (하느님ㅇ, then lad…).
    field.clear();
    type_keys(&mut mixed_session(true), &mut field, "gksmsladml tlsqnsladms godirpTspdy ");
    assert_eq!(field.text(), "하느님의 신부님은 해야겠네요 ");
}

// MARK: - SessionPolicyTests

fn policy_session() -> (InputSession, std::sync::Arc<seamtype_core::LearningEngine>, FakeTextField) {
    let learning = learning(splitting(), "seamtype-lex");
    let policies = AppPolicies::new(HashMap::from([
        ("test.strict".to_string(), Override { conversion_threshold: Some(0.9999), ..Override::default() }),
        ("test.off".to_string(), Override { enabled: Some(false), ..Override::default() }),
    ]));
    let session = InputSession::new(learning.clone(), policies, None, false, false, true);
    (session, learning, FakeTextField::new())
}

#[test]
fn strict_app_keeps_uncertain_words_english() {
    let (mut session, _, mut field) = policy_session();
    session.activate(Some("test.strict"));
    type_keys(&mut session, &mut field, "dhsmf ");
    assert_eq!(field.text(), "dhsmf ");
}

#[test]
fn disabled_app_passes_keys_through() {
    let (mut session, _, mut field) = policy_session();
    session.activate(Some("test.off"));
    assert!(!session.handle(Key::Letter('d'), &mut field));
    assert!(field.text().is_empty());
}

#[test]
fn esc_cycles_mixed_word_after_commit() {
    let (mut session, _, mut field) = policy_session();
    type_keys(&mut session, &mut field, "githubdp ");
    assert_eq!(field.text(), "github에 ");
    // The garbled Korean reading (햐소ㅕㅠ에) is skipped: github에 ↔ githubdp.
    session.handle(Key::Escape, &mut field);
    assert_eq!(field.text(), "githubdp ");
    session.handle(Key::Escape, &mut field);
    assert_eq!(field.text(), "github에 ");
}

#[test]
fn correction_counts_once_with_the_final_choice() {
    let (mut session, learning, mut field) = policy_session();
    session.activate(Some("com.example.chat"));
    type_keys(&mut session, &mut field, "hello ");
    session.handle(Key::Escape, &mut field); // hello → ㅗ디ㅣㅐ
    assert!(learning.pending().is_empty()); // window still open
    type_keys(&mut session, &mut field, "x"); // any other key closes it
    assert_eq!(learning.pending(), BTreeMap::from([("hello".to_string(), Language::Korean)]));
}

#[test]
fn nothing_counts_when_the_word_is_flipped_back() {
    let (mut session, learning, mut field) = policy_session();
    type_keys(&mut session, &mut field, "hello ");
    session.handle(Key::Escape, &mut field);
    session.handle(Key::Escape, &mut field);
    type_keys(&mut session, &mut field, "x");
    assert!(learning.pending().is_empty());
}

// MARK: - PauseTests

fn pause_session() -> (InputSession, FakeTextField) {
    let mut session = InputSession::simple(learning(shared(), "seamtype-pause"));
    session.activate(Some("test.app"));
    (session, FakeTextField::new())
}

#[test]
fn paused_keys_go_through_as_typed() {
    let (mut session, mut field) = pause_session();
    session.toggle_pause(Some(&mut field));
    type_keys(&mut session, &mut field, "jjkkhl dkssud");
    assert_eq!(field.text(), "jjkkhl dkssud");
    assert!(!field.is_marked());
}

#[test]
fn pausing_commits_the_word_in_progress() {
    let (mut session, mut field) = pause_session();
    type_keys(&mut session, &mut field, "dkssud");
    session.toggle_pause(Some(&mut field));
    assert_eq!(field.text(), "안녕");
    assert!(!field.is_marked());
}

#[test]
fn second_tap_resumes() {
    let (mut session, mut field) = pause_session();
    session.toggle_pause(Some(&mut field));
    type_keys(&mut session, &mut field, "jk ");
    session.toggle_pause(Some(&mut field));
    type_keys(&mut session, &mut field, "dkssud ");
    assert_eq!(field.text(), "jk 안녕 ");
}

#[test]
fn another_text_field_resumes() {
    let (mut session, mut field) = pause_session();
    session.toggle_pause(Some(&mut field));
    session.activate(Some("test.other"));
    assert!(!session.paused());
}

/// An app set to Strict keeps short commands as typed but still splits mixed words.
#[test]
fn strict_still_splits_mixed_words() {
    let policies = AppPolicies::new(HashMap::from([(
        "test.terminal".to_string(),
        Override { conversion_threshold: Some(AppPolicy::STRICT_THRESHOLD), ..Override::default() },
    )]));
    let mut session = InputSession::new(learning(splitting(), "seamtype-strict"), policies, None, false, false, true);
    session.activate(Some("test.terminal"));
    let mut field = FakeTextField::new();
    type_keys(&mut session, &mut field, "ciw gst githubdp xdp hellodnjfem ");
    assert_eq!(field.text(), "ciw gst github에 x에 hello월드 ");
}

// MARK: - VimSessionTests

fn vim_session() -> (InputSession, FakeTextField) {
    let mut session = InputSession::simple(learning(shared(), "seamtype-vim"));
    session.activate(Some("test.terminal"));
    (session, FakeTextField::new())
}

/// Types `script` ("⎋" Esc, "⏎" Return) and returns the keys the app got straight.
fn vim_type(session: &mut InputSession, field: &mut FakeTextField, script: &str) -> String {
    let mut passed = String::new();
    for ch in script.chars() {
        let key = match ch {
            '⎋' => Key::Escape,
            '⏎' => Key::Enter,
            c if c.is_ascii_alphabetic() => Key::Letter(c),
            c => Key::Printable(c),
        };
        if !session.handle(key, field) {
            passed.push(ch);
            if ch != '⎋' {
                use seamtype_core::TextClient;
                field.insert_text(&if ch == '⏎' { "\n".to_string() } else { ch.to_string() }, None);
            }
        }
    }
    passed
}

#[test]
fn normal_mode_keys_go_straight_through() {
    let (mut session, mut field) = vim_session();
    session.vim_changed(VimSignal::Normal, Some(&mut field)); // VimEnter, before any Esc
    assert_eq!(vim_type(&mut session, &mut field, "sddciw:wq⏎"), "sddciw:wq⏎");
    assert!(!field.is_marked());
}

#[test]
fn words_are_decided_in_insert_mode() {
    let (mut session, mut field) = vim_session();
    session.vim_changed(VimSignal::Normal, Some(&mut field));
    vim_type(&mut session, &mut field, "o");
    session.vim_changed(VimSignal::Insert, Some(&mut field));
    vim_type(&mut session, &mut field, "dkssud ");
    assert_eq!(field.text(), "o안녕 ");
}

#[test]
fn esc_commits_the_word_then_vim_says_normal() {
    let (mut session, mut field) = vim_session();
    session.vim_changed(VimSignal::Insert, Some(&mut field));
    vim_type(&mut session, &mut field, "dkssud⎋");
    assert_eq!(field.text(), "안녕");
    session.vim_changed(VimSignal::Normal, Some(&mut field)); // InsertLeave
    assert_eq!(vim_type(&mut session, &mut field, "dd"), "dd");
}

#[test]
fn keys_are_held_until_vim_says_insert() {
    // A mapping that enters insert mode is seen only when vim says so.
    let (mut session, mut field) = vim_session();
    session.vim_changed(VimSignal::Normal, Some(&mut field));
    assert_eq!(vim_type(&mut session, &mut field, " i"), " i");
    session.vim_changed(VimSignal::Insert, Some(&mut field));
    assert_eq!(vim_type(&mut session, &mut field, "dkssud"), "");
}

#[test]
fn leaving_vim_gives_the_shell_words_back() {
    let (mut session, mut field) = vim_session();
    session.vim_changed(VimSignal::Normal, Some(&mut field));
    vim_type(&mut session, &mut field, ":wq⏎");
    session.vim_changed(VimSignal::Off, Some(&mut field)); // VimLeavePre
    vim_type(&mut session, &mut field, "dkssud ");
    assert!(field.text().ends_with("안녕 "));
}

#[test]
fn shift_esc_starts_deciding_words_again() {
    let (mut session, mut field) = vim_session();
    session.vim_changed(VimSignal::Normal, Some(&mut field));
    assert!(session.handle(Key::Cycle, &mut field));
    assert!(!session.in_vim_normal_mode());
    vim_type(&mut session, &mut field, "dkssud ");
    assert_eq!(field.text(), "안녕 ");
}

#[test]
fn coming_back_to_the_terminal_keeps_vims_mode() {
    let (mut session, mut field) = vim_session();
    session.vim_changed(VimSignal::Normal, Some(&mut field));
    session.activate(Some("test.terminal"));
    assert_eq!(vim_type(&mut session, &mut field, "dd"), "dd");
}

#[test]
fn esc_is_vims_while_vim_reports_even_where_esc_switches_language() {
    let (mut session, mut field) = vim_session();
    session.vim_changed(VimSignal::Insert, Some(&mut field));
    assert_eq!(vim_type(&mut session, &mut field, "dkssud⎋"), "⎋");
    assert_eq!(field.text(), "안녕");
    // Right after a committed word too: Esc leaves insert mode instead of flipping the word.
    vim_type(&mut session, &mut field, " ");
    session.vim_changed(VimSignal::Insert, Some(&mut field));
    assert_eq!(vim_type(&mut session, &mut field, "dkssud ⎋"), " ⎋");
    assert_eq!(field.text(), "안녕 안녕 ");
}

#[test]
fn without_vim_esc_still_switches_language() {
    let (mut session, mut field) = vim_session();
    assert_eq!(vim_type(&mut session, &mut field, "dkssud⎋"), "");
    assert_eq!(field.text(), "dkssud");
}

#[test]
fn another_app_forgets_vim() {
    let (mut session, mut field) = vim_session();
    session.vim_changed(VimSignal::Normal, Some(&mut field));
    session.activate(Some("test.other"));
    assert!(!session.in_vim_normal_mode());
}

// MARK: - JapaneseSessionTests

fn japanese_session() -> (InputSession, std::sync::Arc<std::sync::Mutex<FakeConverter>>, FakeTextField) {
    let converter = fake_converter();
    let shared: SharedConverter = converter.clone();
    let session = InputSession::new(
        learning(trilingual_splitting(), "seamtype-ja"),
        AppPolicies::default(),
        Some(shared),
        true,
        false,
        true,
    );
    (session, converter, FakeTextField::new())
}

#[test]
fn preview_shows_kana_while_typing() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "kyouhaiitenk");
    assert_eq!(field.text(), "きょうはいいてんk");
}

#[test]
fn space_converts_and_return_commits() {
    let (mut session, converter, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "kyouhaiitenkidesune ");
    assert!(session.conversion().is_some());
    assert_eq!(field.text(), "今日はいい天気ですね");
    assert!(session.handle(Key::Enter, &mut field));
    assert_eq!(field.text(), "今日はいい天気ですね");
    assert!(session.conversion().is_none());
    assert_eq!(converter.lock().unwrap().learned, vec!["今日はいい天気ですね".to_string()]);
}

#[test]
fn space_cycles_candidates_and_numbers_pick() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "henkan ");
    assert_eq!(field.text(), "変換");
    type_keys(&mut session, &mut field, " ");
    assert_eq!(field.text(), "返還");
    session.handle(Key::Up, &mut field);
    assert_eq!(field.text(), "変換");
    type_keys(&mut session, &mut field, "3");
    assert_eq!(field.text(), "編纂");
    assert!(session.conversion().is_none());
}

#[test]
fn first_clause_candidate_keeps_converting_the_rest() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "kyouhaiitenkidesune ");
    session.handle(Key::Down, &mut field); // 今日は (covers 4 kana)
    assert_eq!(field.text(), "今日はいいてんきですね");
    session.handle(Key::Enter, &mut field);
    assert_eq!(field.text(), "今日はいい天気ですね");
    assert_eq!(session.conversion().map(|c| c.kana.as_str()), Some("いいてんきですね"));
    session.handle(Key::Enter, &mut field);
    assert!(session.conversion().is_none());
    assert_eq!(field.text(), "今日はいい天気ですね");
}

#[test]
fn escape_returns_to_kana() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "henkan ");
    session.handle(Key::Escape, &mut field);
    assert!(session.conversion().is_none());
    assert_eq!(field.text(), "へんかん");
    session.handle(Key::Delete, &mut field);
    assert_eq!(field.text(), "へんか");
}

#[test]
fn delete_takes_off_a_whole_kana() {
    let (mut session, converter, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "kannkoku");
    assert_eq!(field.text(), "かんこく");
    session.handle(Key::Delete, &mut field);
    assert_eq!(field.text(), "かんこ");
    session.handle(Key::Delete, &mut field);
    assert_eq!(field.text(), "かん");
    type_keys(&mut session, &mut field, "ji ");
    assert!(converter.lock().unwrap().learned.is_empty());
    assert_eq!(session.conversion().map(|c| c.kana.as_str()), Some("かんじ"));
}

#[test]
fn delete_keeps_a_pending_letter_separate() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "kannkok");
    assert_eq!(field.text(), "かんこk");
    session.handle(Key::Delete, &mut field);
    assert_eq!(field.text(), "かんこ");
}

#[test]
fn delete_splits_contracted_and_doubled_kana() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "kyouha");
    session.handle(Key::Delete, &mut field);
    session.handle(Key::Delete, &mut field);
    assert_eq!(field.text(), "きょ");
    session.handle(Key::Delete, &mut field);
    assert_eq!(field.text(), "き");
    type_keys(&mut session, &mut field, "tte");
    assert_eq!(field.text(), "きって");
    session.handle(Key::Delete, &mut field);
    assert_eq!(field.text(), "きっ");
    type_keys(&mut session, &mut field, "ta");
    assert_eq!(field.text(), "きった");
}

#[test]
fn return_without_converting_commits_kana_only() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "arigatou");
    assert_eq!(field.text(), "ありがとう");
    assert!(session.handle(Key::Enter, &mut field)); // a Japanese IME's Return only confirms the text
    assert_eq!(field.text(), "ありがとう");
}

#[test]
fn preview_matches_what_gets_committed() {
    // "kanji" is an English dictionary word: outside Japanese text it stays English,
    // and the preview says so before Return.
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "kanji");
    assert_eq!(field.text(), "kanji");
    assert!(session.handle(Key::Enter, &mut field)); // Return only confirms, whatever the language
    assert_eq!(field.text(), "kanji");
    assert!(!field.is_marked());
}

#[test]
fn punctuation_stays_inside_japanese_phrase() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "arigatou.");
    assert_eq!(field.text(), "ありがとう。");
    type_keys(&mut session, &mut field, "ra-men");
    assert_eq!(field.text(), "ありがとう。らーめn"); // ん waits for the next key, like any Japanese IME
}

/// After Japanese a symbol is full-width and underlined, as in a Japanese IME: Return only
/// confirms it, the next letter starts a word of its own.
#[test]
fn symbol_after_japanese_waits_for_return() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "henkan ");
    session.handle(Key::Enter, &mut field);
    type_keys(&mut session, &mut field, "!?");
    assert_eq!(field.text(), "変換！？");
    assert!(field.is_marked());
    assert!(session.handle(Key::Enter, &mut field));
    assert_eq!(field.text(), "変換！？");
    assert!(!field.is_marked());
    type_keys(&mut session, &mut field, "_");
    session.handle(Key::Delete, &mut field);
    assert_eq!(field.text(), "変換！？");
    type_keys(&mut session, &mut field, "/wo");
    assert_eq!(field.text(), "変換！？・を");
}

#[test]
fn symbol_after_japanese_converts_with_space() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "henkan ");
    session.handle(Key::Enter, &mut field);
    type_keys(&mut session, &mut field, "_ ");
    assert_eq!(session.conversion().map(|c| c.kana.as_str()), Some("＿"));
}

#[test]
fn numbers_after_japanese_stay_half_width() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "henkan ");
    session.handle(Key::Enter, &mut field);
    type_keys(&mut session, &mut field, "3.5:");
    assert_eq!(field.text(), "変換3.5:");
}

#[test]
fn brackets_close_the_way_they_opened() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "henkan ");
    session.handle(Key::Enter, &mut field);
    type_keys(&mut session, &mut field, "(hello)");
    session.handle(Key::Enter, &mut field);
    assert_eq!(field.text(), "変換（hello）");
}

#[test]
fn symbols_join_only_a_finished_japanese_word() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "nihon.");
    assert_eq!(field.text(), "にほん。");
}

#[test]
fn symbols_in_english_stay_half_width() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "foo_bar (baz) a/b ");
    assert_eq!(field.text(), "foo_bar (baz) a/b ");
}

#[test]
fn typing_on_after_conversion_commits_it() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "henkan ");
    type_keys(&mut session, &mut field, "wo");
    assert_eq!(field.text(), "変換を");
    assert!(session.conversion().is_none());
}

#[test]
fn english_and_korean_still_work() {
    let (mut session, _, mut field) = japanese_session();
    type_keys(&mut session, &mut field, "hello dkssud ");
    assert_eq!(field.text(), "hello 안녕 ");
}

#[test]
fn keys_remainder_after_kana() {
    assert_eq!(InputSession::keys_after("kyouhaiitenki", 4), "iitenki");
    assert_eq!(InputSession::keys_after("kitte", 2), "te");
    assert_eq!(InputSession::keys_after("konnichiha", 2), "nichiha");
}

// MARK: - TextReplacementTests

fn replacement_session() -> (InputSession, FakeTextField) {
    let converter: SharedConverter = fake_converter();
    let policies = AppPolicies::new(HashMap::from([(
        AppPolicies::DEFAULT_KEY.to_string(),
        Override { conversion_threshold: Some(AppPolicy::STRICT_THRESHOLD), ..Override::default() },
    )]));
    let mut session =
        InputSession::new(learning(trilingual_splitting(), "seamtype-tr"), policies, Some(converter), true, false, true);
    session.replacements = HashMap::from([
        ("ㅈㅅ".to_string(), "죄송합니다".to_string()),
        ("주소".to_string(), "서울시".to_string()),
        ("よろ".to_string(), "よろしくお願いします".to_string()),
        ("go".to_string(), "gone".to_string()),
    ]);
    (session, FakeTextField::new())
}

/// The shortcut goes in as typed, even in a Strict app, so the app can replace it on Space.
#[test]
fn korean_shortcut_wins_the_decision() {
    let (mut session, mut field) = replacement_session();
    type_keys(&mut session, &mut field, "wt wnth ");
    assert_eq!(field.text(), "ㅈㅅ 주소 ");
}

#[test]
fn japanese_shortcut_is_the_first_candidate() {
    let (mut session, mut field) = replacement_session();
    type_keys(&mut session, &mut field, "yoro ");
    assert_eq!(session.conversion().map(|c| c.candidates[0].text.as_str()), Some("よろしくお願いします"));
    session.handle(Key::Enter, &mut field);
    assert_eq!(field.text(), "よろしくお願いします");
}

/// gu reads as 혀, but gu- is on its way to the shortcut ぐー.
#[test]
fn japanese_shortcut_with_a_symbol() {
    let (mut session, mut field) = replacement_session();
    session.replacements.insert("ぐー".to_string(), "グーグル".to_string());
    type_keys(&mut session, &mut field, "gu- ");
    assert_eq!(session.conversion().map(|c| c.candidates[0].text.as_str()), Some("グーグル"));
}

#[test]
fn english_shortcuts_are_left_to_the_engine() {
    let (mut session, mut field) = replacement_session();
    type_keys(&mut session, &mut field, "kyouha go");
    assert_ne!(session.last_trace().map(|t| t.decision.explanation.as_str()), Some("text replacement shortcut"));
}

// MARK: - Apps that deliver early (Windows' IMM32 bridge)

/// What the field shows after each of `keys`, past what it held before.
fn shown_after_each(session: &mut InputSession, field: &mut FakeTextField, keys: &str) -> Vec<String> {
    let start = field.text().chars().count();
    keys.chars()
        .map(|ch| {
            type_keys(session, field, &ch.to_string());
            field.text().chars().skip(start).collect()
        })
        .collect()
}

/// An app that already has all but the last character (Webex through IMM32) isn't shown an unsure
/// English reading while one Korean character would do: an, ant would have to be erased again with
/// Backspaces once 무슨 turns Korean. A field that composes the whole word still shows them.
#[test]
fn early_delivery_app_waits_with_unsure_english() {
    let mut session = mixed_session(false);
    let mut field = FakeTextField::early_delivery();
    type_keys(&mut session, &mut field, "dkssud ");
    assert_eq!(shown_after_each(&mut session, &mut field, "antms "), ["ㅁ", "무", "뭇", "무스", "무슨", "무슨 "]);

    let mut session = mixed_session(false);
    let mut field = FakeTextField::new();
    type_keys(&mut session, &mut field, "dkssud ");
    assert_eq!(shown_after_each(&mut session, &mut field, "antms "), ["a", "an", "ant", "무스", "무슨", "무슨 "]);
}

/// Only what is shown changes: the same keys commit the same text.
#[test]
fn early_delivery_app_commits_the_same() {
    let script = "dkssud antms sork so to the and a githubdp gksrmf hello dlrjs go ";
    let mut field = FakeTextField::new();
    type_keys(&mut mixed_session(false), &mut field, script);
    let mut early = FakeTextField::early_delivery();
    type_keys(&mut mixed_session(false), &mut early, script);
    assert_eq!(early.text(), field.text());
}

/// Common English words of three letters or more stay English on screen (the, not 소).
#[test]
fn early_delivery_app_shows_common_english_words() {
    let mut session = mixed_session(false);
    let mut field = FakeTextField::early_delivery();
    type_keys(&mut session, &mut field, "dkssud the");
    assert_eq!(field.text(), "안녕 the");
}

/// Esc and Delete go by what is on screen: Esc on 무 (an) switches to an, Delete keeps it Korean.
#[test]
fn early_delivery_app_esc_and_delete_go_by_the_screen() {
    let mut session = mixed_session(false);
    let mut field = FakeTextField::early_delivery();
    type_keys(&mut session, &mut field, "dkssud an");
    assert_eq!(field.text(), "안녕 무");
    type_keys(&mut session, &mut field, "⎋ ");
    assert_eq!(field.text(), "안녕 an ");

    let mut session = mixed_session(false);
    let mut field = FakeTextField::early_delivery();
    type_keys(&mut session, &mut field, "dkssud an⌫");
    assert_eq!(field.text(), "안녕 ㅁ");
}

/// A clean Korean loanword after an English word commits as Korean, though its start showed as
/// English on the way (flema): 리드미, not flemal. Also where the app delivers early.
#[test]
fn clean_korean_loanword_after_english_commits_korean() {
    let mut field = FakeTextField::new();
    type_keys(&mut mixed_session(false), &mut field, "fork flemal ");
    assert_eq!(field.text(), "fork 리드미 ");
    let mut field = FakeTextField::early_delivery();
    type_keys(&mut mixed_session(false), &mut field, "fork flemal ");
    assert_eq!(field.text(), "fork 리드미 ");
}
