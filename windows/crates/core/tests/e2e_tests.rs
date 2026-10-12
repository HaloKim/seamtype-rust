//! EndToEndTests and CorrectionTests of the Swift package: keystrokes through the real session
//! and real engines into a fake text field, everything but the text service itself.
//!
//! The Swift suites convert with AzooKey's kanji dictionary; Windows has `KanaConverter` (the
//! reading in hiragana and katakana) until it has one, so expectations that were kanji are the
//! kana here. What they check about the session (clauses, Esc back to kana, learning) is the same.

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use common::*;
use seamtype_core::{
    AppPolicies, AppPolicy, InputSession, KanaConverter, Key, Language, LearningEngine, Override, SharedConverter,
};

fn converter() -> SharedConverter {
    Arc::new(Mutex::new(KanaConverter))
}

/// A terminal the user set to strict, with Esc going to the app.
fn e2e() -> (InputSession, FakeTextField) {
    let terminal = AppPolicies::new(HashMap::from([(
        "test.terminal".to_string(),
        Override { conversion_threshold: Some(AppPolicy::STRICT_THRESHOLD), escape_to_app: Some(true), ..Override::default() },
    )]));
    let session =
        InputSession::new(learning(trilingual_splitting(), "seamtype-e2e"), terminal, Some(converter()), true, false, true);
    (session, FakeTextField::new())
}

fn typed(script: &str) -> String {
    let (mut session, mut field) = e2e();
    type_script(&mut session, &mut field, script);
    field.text()
}

#[test]
fn three_languages_in_one_line() {
    assert_eq!(typed("dkssudgktpdy hello kyouhaiitenkidesune ⏎"), "안녕하세요 hello きょうはいいてんきですね");
}

#[test]
fn english_inside_japanese() {
    assert_eq!(typed("kyouhagoogledekensaku ⏎"), "きょうはgoogleでけんさく");
}

#[test]
fn english_word_with_a_particle() {
    for (script, expected) in [
        ("githubde ⏎", "githubで"), ("kyouha ⏎githubwo ⏎", "きょうはgithubを"), ("slackkara ⏎", "slackから"),
        ("i have 2 cats. githubde⏎", "i have 2 cats. githubで"), ("ok githubwo ⏎", "ok githubを"),
        ("jayne ", "jayne "), ("photo ", "photo "),
        ("guswo ", "현재 "), ("guswosms ", "현재는 "), ("kyouha ⏎guswo ", "きょうは현재 "), // gus + を? no: 현재
    ] {
        assert_eq!(typed(script), expected, "{script}");
    }
}

/// In a terminal (0.97), 어떻게 after an English word is 0.969 sure: what was shown is committed.
#[test]
fn space_commits_what_the_preview_shows() {
    let (mut session, mut field) = e2e();
    session.activate(Some("test.terminal"));
    type_script(&mut session, &mut field, "esc djEjgrp");
    assert_eq!(field.text(), "esc 어떻게"); // still composing (marked)
    type_script(&mut session, &mut field, " todrkrgo?");
    assert_eq!(field.text(), "esc 어떻게 생각해?");
}

/// In a terminal Esc belongs to the app (vim): the word goes in as shown and Esc goes on.
#[test]
fn terminal_escape_goes_to_the_app() {
    let (mut session, mut field) = e2e();
    session.activate(Some("test.terminal"));
    type_script(&mut session, &mut field, "dkssud");
    assert!(!session.handle(Key::Escape, &mut field)); // passed to the app
    assert_eq!(field.text(), "안녕");
    assert!(session.conversion().is_none());
    type_script(&mut session, &mut field, " hello ");
    assert!(!session.handle(Key::Escape, &mut field)); // no undo after commit either
    assert_eq!(field.text(), "안녕 hello ");
}

/// ⇧Esc switches languages there, while composing and right after a commit.
#[test]
fn terminal_shift_escape_switches() {
    let (mut session, mut field) = e2e();
    session.activate(Some("test.terminal"));
    // hello's clean readings are English and へっぉ (ㅗ디ㅣㅐ is skipped).
    type_script(&mut session, &mut field, "hello⇧");
    assert_eq!(field.text(), "へっぉ");
    type_script(&mut session, &mut field, "⇧ "); // back to English, then commit
    assert_eq!(field.text(), "hello ");
    type_script(&mut session, &mut field, "⇧");
    assert_eq!(field.text(), "へっぉ ");
}

/// Converting, backing out and converting again is still one correction: not learned.
#[test]
fn reconverting_counts_as_one_correction() {
    let (mut session, mut field) = e2e();
    type_script(&mut session, &mut field, "hello⎋ ⎋ ⏎");
    assert_ne!(field.text(), "hello");
    type_script(&mut session, &mut field, " hello ");
    assert!(field.text().ends_with(" hello "));
}

/// Other apps keep plain Esc for switching.
#[test]
fn escape_switches_in_other_apps() {
    let (mut session, mut field) = e2e();
    session.activate(Some("notepad.exe"));
    type_script(&mut session, &mut field, "hello⎋");
    assert_eq!(field.text(), "へっぉ");
}

#[test]
fn acronyms_and_longer_endings() {
    for (script, expected) in
        [("UIrkxdmsrjs ", "UI같은건 "), ("UIdp ", "UI에 "), ("AIrk ", "AI가 "), ("githubrkxdmsep ", "github같은데 ")]
    {
        assert_eq!(typed(script), expected, "{script}");
    }
}

#[test]
fn korean_with_english_and_particles() {
    assert_eq!(typed("dlrjs githubdp commitgkf rjdi."), "이건 github에 commit할 거야.");
}

#[test]
fn punctuation_keeps_the_words_language() {
    for (script, expected) in [
        ("dho?", "왜?"), ("rjdi.", "거야."), ("hello.", "hello."), ("hello, world!", "hello, world!"),
        ("arigatou.", "ありがとう。"), ("ra-men", "らーめn"), ("e-mail ", "e-mail "), ("ok!", "ok!"),
    ] {
        assert_eq!(typed(script), expected, "{script}");
    }
}

#[test]
fn everyday_sentences() {
    for (script, expected) in [
        ("dkssudgktpdy. qksrkqtmqslek!", "안녕하세요. 반갑습니다!"),
        ("dlTek.", "있다."),
        ("i have 2 cats.", "i have 2 cats."),
        ("dhsmf 3tl", "오늘 3시"),
    ] {
        assert_eq!(typed(script), expected, "{script}");
    }
}

#[test]
fn japanese_conversions() {
    for (script, expected) in [
        ("honwoyomu ⏎", "ほんをよむ"),
        ("ko-hi-wononde ⏎", "こーひーをのんで"),
        ("watashihagakuseidesu ⏎", "わたしはがくせいです"),
    ] {
        assert_eq!(typed(script), expected, "{script}");
    }
}

/// ←/→ move the first clause's end; Return commits it and converts the rest.
#[test]
fn clause_by_clause_with_arrows() {
    let (mut session, mut field) = e2e();
    type_script(&mut session, &mut field, "kyouhaiitenkidesune ");
    assert_eq!(session.conversion().map(|c| c.current().kana_count), Some(12));
    type_script(&mut session, &mut field, "←←←←←←←←"); // 12 → 4: きょうは
    assert_eq!(session.conversion().map(|c| c.current().kana_count), Some(4));
    assert_eq!(field.text(), "きょうはいいてんきですね");
    type_script(&mut session, &mut field, "⏎");
    assert!(field.text().starts_with("きょうは"));
    assert_eq!(session.conversion().map(|c| c.kana.as_str()), Some("いいてんきですね"));
    type_script(&mut session, &mut field, "⏎");
    assert_eq!(field.text(), "きょうはいいてんきですね");
    assert!(session.conversion().is_none());
}

#[test]
fn clause_ends_stay_within_bounds() {
    let (mut session, mut field) = e2e();
    type_script(&mut session, &mut field, "kyouha ");
    type_script(&mut session, &mut field, "→");
    assert_eq!(session.conversion().map(|c| c.current().kana_count), Some(4)); // already the whole reading
    type_script(&mut session, &mut field, "←←←←←←");
    assert_eq!(session.conversion().map(|c| c.current().kana_count), Some(1));
    type_script(&mut session, &mut field, "⎋");
    assert!(session.conversion().is_none());
}

/// A clause boundary never splits an English word inside the phrase.
#[test]
fn clause_boundary_skips_english_letters() {
    let (mut session, mut field) = e2e();
    type_script(&mut session, &mut field, "kyouhagoogledekensaku ");
    assert_eq!(session.conversion().map(|c| c.kana.as_str()), Some("きょうはgoogleでけんさく"));
    assert_eq!(session.conversion().map(|c| c.current().kana_count), Some(15));
    type_script(&mut session, &mut field, "←←←←←"); // 15 → 10: きょうはgoogle
    assert_eq!(session.conversion().map(|c| c.current().kana_count), Some(10));
    type_script(&mut session, &mut field, "←"); // not 9 (googl|e) but 4: きょうは
    assert_eq!(session.conversion().map(|c| c.current().kana_count), Some(4));
}

#[test]
fn numbers_after_japanese_keep_their_decimal_point() {
    let (mut session, mut field) = e2e();
    type_script(&mut session, &mut field, "kyouha ⏎");
    type_script(&mut session, &mut field, "3.5");
    assert_eq!(field.text(), "きょうは3.5");
    type_script(&mut session, &mut field, "desu.");
    assert!(field.text().ends_with("です。"));
}

#[test]
fn katakana_and_hiragana_shortcuts() {
    let (mut session, mut field) = e2e();
    type_script(&mut session, &mut field, "ko-hi-");
    session.handle(Key::Katakana, &mut field);
    assert_eq!(field.text(), "コーヒー");
    type_script(&mut session, &mut field, " "); // flip to the other form
    assert_eq!(field.text(), "こーひー");
    type_script(&mut session, &mut field, "⏎");
    assert_eq!(field.text(), "こーひー");
    // ⌃K on an English word is the app's shortcut, not ours.
    type_script(&mut session, &mut field, " hello");
    assert!(!session.handle(Key::Katakana, &mut field));
    assert_eq!(field.text(), "こーひー hello");
}

#[test]
fn pick_a_second_candidate() {
    let (mut session, mut field) = e2e();
    type_script(&mut session, &mut field, "kanjiwo"); // かんじを ("kanjiwo" isn't English)
    type_script(&mut session, &mut field, " ");
    let first = field.text();
    type_script(&mut session, &mut field, " ");
    assert_ne!(field.text(), first); // another candidate is shown
    type_script(&mut session, &mut field, "⏎");
    assert!(session.conversion().is_none());
    assert!(field.text().ends_with("ヲ")); // the katakana form, KanaConverter's second
}

#[test]
fn fix_a_wrong_guess_with_escape() {
    let (mut session, mut field) = e2e();
    type_script(&mut session, &mut field, "ozzy ");
    assert_eq!(field.text(), "ozzy ");
    type_script(&mut session, &mut field, "⎋"); // ozzy → its Korean reading
    assert_ne!(field.text(), "ozzy ");
    type_script(&mut session, &mut field, "⎋⎋"); // → Japanese → back to English
    assert_eq!(field.text(), "ozzy ");
}

// MARK: - CorrectionTests

fn correction() -> (InputSession, Arc<LearningEngine>, FakeTextField) {
    let learning = learning(trilingual_splitting(), "seamtype-corr");
    let mut session = InputSession::new(learning.clone(), AppPolicies::default(), Some(converter()), true, false, true);
    session.activate(Some("notepad.exe"));
    (session, learning, FakeTextField::new())
}

/// The engine is unsure about "so" (English, 0.55): an Esc to 내 must still count.
#[test]
fn esc_while_typing_is_learned_the_second_time() {
    let (mut session, learning, mut field) = correction();
    type_script(&mut session, &mut field, "so⎋ ");
    assert_eq!(field.text(), "내 ");
    type_script(&mut session, &mut field, "x so⎋ ");
    assert_eq!(learning.learned().get("so"), Some(&Language::Korean));
    type_script(&mut session, &mut field, "x so ");
    assert!(field.text().ends_with("내 "));
}

/// Esc after a commit passing through other readings records only where it ended up.
#[test]
fn undo_detour_is_one_correction() {
    let (mut session, learning, mut field) = correction();
    // ozzy: English → Korean (ㅐㅋ쿄) → Japanese.
    type_script(&mut session, &mut field, "ozzy ⎋⎋");
    type_script(&mut session, &mut field, "x "); // any key ends the undo window
    assert_eq!(learning.pending().get("ozzy"), Some(&Language::Japanese));
    type_script(&mut session, &mut field, "ozzy ⎋⎋");
    type_script(&mut session, &mut field, "x ");
    assert_eq!(learning.learned().get("ozzy"), Some(&Language::Japanese));
}

/// Back to kana, then Esc to English: no apostrophe the user never typed.
#[test]
fn no_apostrophe_after_back_to_kana() {
    let (mut session, _, mut field) = correction();
    type_script(&mut session, &mut field, "henkan ⎋⎋ ");
    assert_eq!(field.text(), "henkan ");
}

/// Committing a clause and going back to kana for the rest isn't a correction.
#[test]
fn back_to_kana_on_the_rest_is_not_a_correction() {
    let (mut session, learning, mut field) = correction();
    type_script(&mut session, &mut field, "kyouhaiitenkidesune ");
    type_script(&mut session, &mut field, "←←←←←←←←⏎"); // きょうは, the rest keeps converting
    type_script(&mut session, &mut field, "⎋⏎");
    assert!(field.text().starts_with("きょうは"));
    assert!(learning.pending().is_empty());
    assert!(learning.learned().is_empty());
}

/// Contractions and hyphenated words stay English with Japanese on.
#[test]
fn apostrophes_and_hyphens_stay_english() {
    for word in ["don't ", "aren't ", "haven't ", "won't ", "weren't ", "to-do ", "e-mail ", "can't "] {
        let (mut session, _, mut field) = correction();
        type_script(&mut session, &mut field, word);
        assert_eq!(field.text(), word);
    }
}

/// Converting a word learned as Japanese again doesn't erase it.
#[test]
fn learned_word_survives_being_used() {
    let (mut session, learning, mut field) = correction();
    learning.learn("ozzy", Language::Japanese, Some(Language::English));
    learning.learn("ozzy", Language::Japanese, Some(Language::English));
    assert_eq!(learning.learned().get("ozzy"), Some(&Language::Japanese));
    type_script(&mut session, &mut field, "ozzy ⎋ "); // convert, back to kana, convert again
    type_script(&mut session, &mut field, "⏎");
    assert_eq!(learning.learned().get("ozzy"), Some(&Language::Japanese));
}
