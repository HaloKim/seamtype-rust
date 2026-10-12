//! DecisionEngineTests, HangulComposerTests, RomajiComposerTests, JapaneseDetectionTests,
//! SegmenterTests, MixedWordTests, AppPolicyTests and SettingsStoreTests of the Swift package.

mod common;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use common::*;
use seamtype_core::segmenter::{self, JapaneseSegmenter};
use seamtype_core::text_converter::TextConverter;
use seamtype_core::{
    english_model, hangul, japanese_model, romaji, AppPolicies, AppPolicy, Decision, DecisionContext, DecisionEngine,
    Language, LearningEngine, Override, Settings,
};

fn ctx() -> DecisionContext {
    DecisionContext::default()
}

// MARK: - DecisionEngineTests

#[test]
fn english_words_stay_english() {
    for word in [
        "hello", "world", "the", "keyboard", "language", "switch", "input", "method", "files", "running", "commit",
        "github", "please", "thanks", "Apple", "API", "is", "a", "computer", "people", "quick", "brown", "fox", "jumps",
        "over", "lazy", "dog",
    ] {
        let d = SHARED_ENGINE.decide(word, &ctx());
        assert_eq!(d.language, Language::English, "{word} → {} {}", d.output, d.explanation);
    }
}

#[test]
fn korean_keystrokes_become_hangul() {
    for (keys, expected) in [
        ("dkssudgktpdy", "안녕하세요"), ("gksrmf", "한글"), ("rkatkgkqslek", "감사합니다"),
        ("dhsmf", "오늘"), ("skfTl", "날씨"), ("whgdk", "좋아"), ("dlTek", "있다"),
        ("tkfkd", "사랑"), ("zjavbxj", "컴퓨터"), ("vmfhrmfoa", "프로그램"), ("wjdakf", "정말"),
        ("rhakdnj", "고마워"), ("wkf", "잘"), ("ajrdj", "먹어"), ("zzz", "ㅋㅋㅋ"),
        ("dlrjs", "이건"), ("djEjg", "어떻"), ("tlrks", "시간"), ("rmfjsep", "그런데"),
    ] {
        let d = SHARED_ENGINE.decide(keys, &ctx());
        assert_eq!(d.language, Language::Korean, "{keys} → {} {}", d.output, d.explanation);
        assert_eq!(d.output, expected);
    }
}

#[test]
fn mixed_sentence() {
    let converter = TextConverter::new(shared());
    assert_eq!(converter.convert("dkssudgktpdy, hello world!").0, "안녕하세요, hello world!");
    assert_eq!(converter.convert("dlrjs github dp commit gkf rjdi").0, "이건 github 에 commit 할 거야");
    assert_eq!(converter.convert("tmvmfls API qkfvy wnsql").0, "스프린 API 발표 준비");
}

#[test]
fn context_resolves_short_collisions() {
    // "go" is English, but also 해 — the previous word tips it.
    let converter = TextConverter::new(shared());
    assert_eq!(converter.convert("let's go").0, "let's go");
    assert_eq!(converter.convert("dlrj go").0, "이거 해");
}

#[test]
fn learning_overrides_base() {
    let path = temp_path("seamtype-test", ".json");
    let learning = LearningEngine::new(shared(), Some(path.clone()));
    assert_eq!(learning.decide("hello", &ctx()).language, Language::English);
    learning.learn("hello", Language::Korean, None);
    assert_eq!(learning.decide("hello", &ctx()).language, Language::English); // once: not yet
    learning.learn("hello", Language::Korean, None);
    assert_eq!(learning.decide("hello", &ctx()).output, "ㅗ디ㅣㅐ");
    // Persisted and reloaded.
    let reloaded = LearningEngine::new(shared(), Some(path.clone()));
    assert_eq!(reloaded.decide("hello", &ctx()).language, Language::Korean);
    // Learning the base choice removes the override.
    reloaded.learn("hello", Language::English, None);
    assert_eq!(reloaded.decide("hello", &ctx()).language, Language::English);
    let _ = std::fs::remove_file(path);
}

/// A lexicon.json it can't read (hand-edited, another version) is set aside, not overwritten.
#[test]
fn unreadable_lexicon_is_kept_aside() {
    let path = temp_path("seamtype-test", ".json");
    let mut aside = path.as_os_str().to_owned();
    aside.push(".unreadable");
    std::fs::write(&path, r#"{"de": "japanese"}"#).unwrap();
    let learning = LearningEngine::new(shared(), Some(path.clone()));
    assert!(learning.learned().is_empty() && learning.pending().is_empty());
    assert!(std::path::Path::new(&aside).exists());
    // A different correction than before starts over.
    learning.learn("rjtm", Language::English, Some(Language::Korean));
    learning.learn("rjtm", Language::Mixed, Some(Language::Korean));
    assert!(learning.learned().is_empty());
    assert_eq!(learning.pending(), BTreeMap::from([("rjtm".to_string(), Language::Mixed)]));
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(aside);
}

/// A word learned as mixed is decided and rendered without asking itself again (the Swift
/// engine recursed without end here).
#[test]
fn word_learned_as_mixed_renders() {
    let learning = LearningEngine::new(splitting(), Some(temp_path("seamtype-test", ".json")));
    learning.learn("githubdp", Language::Mixed, Some(Language::English));
    learning.learn("githubdp", Language::Mixed, Some(Language::English));
    let d = learning.decide("githubdp", &ctx());
    assert_eq!((d.language, d.output.as_str()), (Language::Mixed, "github에"));
}

#[test]
fn partial_preview_does_not_punish_pending_consonant() {
    // "dkss" = 안 + next ㄴ while typing 안녕
    let partial = SHARED_ENGINE.decide("dkss", &DecisionContext::new(None, true));
    assert_eq!(partial.language, Language::Korean, "{}", partial.explanation);
}

// MARK: - HangulComposerTests

#[test]
fn hangul_composes() {
    for (keys, expected) in [
        ("dkssudgktpdy", "안녕하세요"), ("gksrmf", "한글"), ("rkqt", "값"), ("rkqtdl", "값이"), ("dlTek", "있다"),
        ("dhkswjsgl", "완전히"), ("ehlwl", "되지"), ("dmlwk", "의자"), ("ekfrdl", "닭이"), ("rhkswls", "관진"),
        ("Qkffl", "빨리"), ("dkQk", "아빠"), ("zzz", "ㅋㅋㅋ"), ("rk", "가"), ("r", "ㄱ"), ("k", "ㅏ"),
    ] {
        assert_eq!(hangul::compose(keys).map(|c| c.text()).as_deref(), Some(expected), "{keys}");
    }
}

#[test]
fn double_consonant_that_cannot_be_final_starts_new_syllable() {
    // 아 + ㄸ: ㄸ can't be 받침, so it waits as the next initial.
    let result = hangul::compose("dkE").unwrap();
    assert_eq!(result.text(), "아ㄸ");
    assert!(result.ends_with_pending_consonant());
}

#[test]
fn english_produces_stray_jamo() {
    assert!(hangul::compose("hello").unwrap().has_stray_jamo());
}

#[test]
fn hangul_rejects_non_letters() {
    assert!(hangul::compose("ab1").is_none());
}

// MARK: - RomajiComposerTests

#[test]
fn romaji_composes() {
    for (keys, expected) in [
        ("konnichiha", "こんにちは"), ("arigatou", "ありがとう"), ("kitte", "きって"), ("ra-men", "らーめん"),
        ("shinbun", "しんぶん"), ("kan'i", "かんい"), ("tenki", "てんき"), ("nihongo", "にほんご"),
        ("kyou", "きょう"), ("gakkou", "がっこう"), ("matcha", "まっちゃ"), ("shashin", "しゃしん"),
        ("tsukue", "つくえ"), ("fujisan", "ふじさん"), ("jouzu", "じょうず"), ("onna", "おんな"),
        ("konnnichiha", "こんにちは"), ("xtu", "っ"), ("dhisuku", "でぃすく"), ("wo", "を"),
        ("benkyou", "べんきょう"), ("hon", "ほん"), ("sushi", "すし"),
    ] {
        let r = romaji::compose(keys, false);
        assert_eq!(r.text, expected, "{keys}");
        assert!(r.is_clean(), "{keys}");
    }
}

/// Delete rewrites the keys from these spellings, so each must compose back to its kana,
/// also with a vowel typed after it (ん before あ).
#[test]
fn respellings_compose_back() {
    for (kana, spelling) in romaji::spellings() {
        assert_eq!(romaji::compose(&spelling, true).text, kana.to_string(), "{kana} {spelling}");
        assert_eq!(romaji::compose(&format!("{spelling}a"), false).text, format!("{kana}あ"), "{kana} {spelling}a");
    }
}

#[test]
fn romaji_pending_while_typing() {
    let r = romaji::compose("konnic", true);
    assert_eq!(r.text, "こんにc");
    assert_eq!(r.pending, "c");
    assert_eq!(r.stray_letters, 0);
    assert_eq!(romaji::compose("hon", true).text, "ほn");
}

#[test]
fn non_japanese_leaves_stray_letters() {
    for word in ["strength", "dkssud", "world", "keyboard", "gksrmf"] {
        assert!(romaji::compose(word, false).stray_letters > 0, "{word}");
    }
}

/// Like real Japanese IMEs, some English converts cleanly (hello → へっぉ);
/// telling these apart is the detector's job, not the composer's.
#[test]
fn some_english_converts_cleanly() {
    assert_eq!(romaji::compose("hello", false).text, "へっぉ");
}

#[test]
fn katakana() {
    assert_eq!(romaji::katakana("こんぴゅーたー"), "コンピューター");
}

// MARK: - JapaneseDetectionTests

#[test]
fn japanese_model_loads() {
    assert!(japanese_model::shared().is_some());
}

#[test]
fn japanese_phrases() {
    for (keys, kana) in [
        ("konnichiha", "こんにちは"), ("watashihagakuseidesu", "わたしはがくせいです"), ("arigatougozaimasu", "ありがとうございます"),
        ("kyouhaiitenkidesune", "きょうはいいてんきですね"), ("ra-mentabetai", "らーめんたべたい"), ("sumimasen", "すみません"),
        ("yoroshikuonegaishimasu", "よろしくおねがいします"), ("daijoubu", "だいじょうぶ"),
    ] {
        let d = TRILINGUAL.decide(keys, &ctx());
        assert_eq!(d.language, Language::Japanese, "{keys}: {}", d.explanation);
        assert_eq!(d.output, kana);
    }
}

#[test]
fn english_stays_english_with_japanese_on() {
    for word in ["hello", "world", "keyboard", "wanna", "gotta", "the", "people", "thanks", "commit", "same"] {
        let d = TRILINGUAL.decide(word, &ctx());
        assert_eq!(d.language, Language::English, "{word}: {}", d.explanation);
    }
}

#[test]
fn korean_stays_korean_with_japanese_on() {
    for (keys, expected) in [("dkssudgktpdy", "안녕하세요"), ("gksrmf", "한글"), ("rkatkgkqslek", "감사합니다"), ("tkfkd", "사랑")] {
        let d = TRILINGUAL.decide(keys, &ctx());
        assert_eq!(d.language, Language::Korean, "{keys}: {}", d.explanation);
        assert_eq!(d.output, expected);
    }
}

#[test]
fn japanese_context_helps_ambiguous_words() {
    // "kami" (紙/髪/神) after Japanese reads as Japanese.
    let d = TRILINGUAL.decide("kamiwo", &DecisionContext::new(Some(Language::Japanese), false));
    assert_eq!(d.language, Language::Japanese);
}

#[test]
fn esc_cycle_includes_japanese_only_when_enabled() {
    assert_eq!(Language::Korean.next("sushi", true), Language::Japanese);
    assert_eq!(Language::Korean.next("sushi", false), Language::English);
    assert_eq!(Language::Japanese.next("sushi", true), Language::English);
    // No Korean reading for "ra-men".
    assert_eq!(Language::English.next("ra-men", true), Language::Japanese);
}

#[test]
fn japanese_off_by_default() {
    assert_ne!(SHARED_ENGINE.decide("konnichiha", &ctx()).language, Language::Japanese);
}

// MARK: - SegmenterTests

fn segmenter() -> JapaneseSegmenter {
    JapaneseSegmenter::new(english_model::shared(), japanese_model::shared().unwrap())
}

#[test]
fn splits_english_out_of_japanese() {
    let segmenter = segmenter();
    for (keys, text) in [
        ("kyouhagoogledekensaku", "きょうはgoogleでけんさく"),
        ("meetingwoshimashou", "meetingをしましょう"),
        ("serverniloginshita", "serverにloginした"),
    ] {
        let segments = segmenter.segment(keys).unwrap_or_else(|| panic!("{keys} not segmented"));
        assert_eq!(segmenter::text(&segments), text);
    }
}

#[test]
fn segmenter_leaves_other_input_alone() {
    let segmenter = segmenter();
    for keys in ["konnichiha", "wouldn", "mumbai", "hello", "brooklyn"] {
        assert!(segmenter.segment(keys).is_none(), "{keys}");
    }
}

#[test]
fn decision_uses_segmentation() {
    let d = TRILINGUAL.decide("kyouhagoogledekensaku", &ctx());
    assert_eq!(d.language, Language::Japanese);
    assert_eq!(d.output, "きょうはgoogleでけんさく");
}

// MARK: - MixedWordTests

#[test]
fn splits_english_plus_particle() {
    for (keys, expected) in [
        ("githubdp", "github에"), ("APIfmf", "API를"), ("commitgotj", "commit해서"),
        ("serverdptj", "server에서"), ("pythondmfh", "python으로"), ("deploygoTdj", "deploy했어"),
        // Names typed with their capitals, in the word list or not.
        ("TextEditdptj", "TextEdit에서"), ("Slackdptj", "Slack에서"), ("VSCodedptj", "VSCode에서"),
        ("GitHubdp", "GitHub에"), ("iPhonedptj", "iPhone에서"),
    ] {
        let d = SPLITTING.decide(keys, &ctx());
        assert_eq!(d.language, Language::Mixed, "{keys}: {}", d.explanation);
        assert_eq!(d.output, expected);
    }
}

#[test]
fn splits_english_plus_korean_word() {
    for (keys, expected) in [
        ("hellodnjfem", "hello월드"), ("githubtjqj", "github서버"), ("loginghkaus", "login화면"),
        ("designvhsxm", "design폰트"), ("Reactzjavhsjsxm", "React컴포넌트"), ("VSCodetjfwjd", "VSCode설정"),
    ] {
        let d = SPLITTING.decide(keys, &ctx());
        assert_eq!(d.language, Language::Mixed, "{keys}: {}", d.explanation);
        assert_eq!(d.output, expected);
    }
}

// Korean typed with shift (쌍꺼풀은, 있어, 끝에서, 쐈어요) isn't a capitalized name;
// names that spell like English (cartman ≠ car스무) and clean romaji (はさんそを) stay whole.
#[test]
fn leaves_whole_words_alone() {
    for keys in [
        "work", "park", "hello", "thanks", "dkssudgktpdy", "akadp", "ruxdp", "wkfrk", "TkdRjvnfdms", "dlTdj", "Rmxdptj",
        "ThkTdjdy", "RoRmtgkgfusk", "cartman", "Cartman", "stuntman", "comstock", "wolfsbane", "hasansowo", "nonakawo",
    ] {
        assert_ne!(SPLITTING.decide(keys, &ctx()).language, Language::Mixed, "{keys}");
    }
}

#[test]
fn esc_cycles_through_readings() {
    assert_eq!(Language::English.next("githubdp", false), Language::Mixed);
    assert_eq!(Language::Mixed.next("githubdp", false), Language::English); // 햐소ㅕㅠ에 is garbled: skipped
    assert_eq!(Language::Korean.next("githubdp", false), Language::English);
    assert_eq!(Language::English.next("dkssud", false), Language::Korean);
    // No split available: english ↔ korean.
    assert_eq!(Language::English.next("hello", false), Language::Korean);
}

#[test]
fn mixed_word_sentence() {
    let converter = TextConverter::new(splitting());
    assert_eq!(converter.convert("dlrjs githubdp commitgkf rjdi").0, "이건 github에 commit할 거야");
}

// MARK: - AppPolicyTests

/// No app is special unless the user lists it.
#[test]
fn every_app_starts_at_the_default() {
    let policies = AppPolicies::default();
    assert_eq!(policies.policy(Some("windowsterminal.exe")), AppPolicy::STANDARD);
    assert_eq!(policies.policy(Some("kakaotalk.exe")), AppPolicy::STANDARD);
    assert_eq!(policies.policy(None), AppPolicy::STANDARD);
    assert!(policies.apps().is_empty());
}

#[test]
fn user_entries_and_prefixes() {
    let path = temp_path("seamtype-apps", ".json");
    std::fs::write(&path, r#"{"com.apple.Terminal": {"enabled": false}, "com.jetbrains.": {"conversionThreshold": 0.97}}"#).unwrap();
    let policies = AppPolicies::load(Some(&path));
    assert!(!policies.policy(Some("com.apple.Terminal")).enabled);
    assert_eq!(policies.policy(Some("com.jetbrains.intellij")).conversion_threshold, 0.97);
    assert!(AppPolicies::load(Some(&path.with_extension("missing"))).overrides.is_empty());
    let _ = std::fs::remove_file(path);
}

/// An app's entry changes only the keys it names; the rest follows the user's default.
#[test]
fn app_entries_sit_on_the_default() {
    let path = temp_path("seamtype-apps", ".json");
    std::fs::write(
        &path,
        r#"{"*": {"escapeToApp": true}, "com.mitchellh.ghostty": {"conversionThreshold": 0.97}, "com.example.Game": {"enabled": false}}"#,
    )
    .unwrap();
    let policies = AppPolicies::load(Some(&path));
    let p = |enabled, conversion_threshold, escape_to_app| AppPolicy { enabled, conversion_threshold, escape_to_app };
    assert_eq!(policies.default_policy(), p(true, 0.5, true));
    assert_eq!(policies.policy(Some("com.mitchellh.ghostty")), p(true, 0.97, true));
    assert_eq!(policies.policy(Some("com.example.Game")), p(false, 0.5, true));
    assert_eq!(policies.policy(Some("com.apple.Terminal")), p(true, 0.5, true));
    assert_eq!(policies.apps(), vec!["com.example.Game".to_string(), "com.mitchellh.ghostty".to_string()]);
    let _ = std::fs::remove_file(path);
}

// MARK: - SettingsStoreTests

/// A fixed engine, so tests control what "the engine's own choice" is.
struct Fixed(Language);

impl DecisionEngine for Fixed {
    fn decide(&self, keys: &str, _: &DecisionContext) -> Decision {
        Decision::new(self.0, 0.9, keys, "")
    }
}

#[test]
fn right_command_pause_is_on_unless_turned_off() {
    let path = temp_path("seamtype-store", ".json");
    std::fs::write(&path, r#"{ "japanese": true }"#).unwrap();
    assert!(Settings::load(Some(&path)).right_command_pause);
    Settings { right_command_pause: false, ..Settings::default() }.save(Some(&path)).unwrap();
    assert!(!Settings::load(Some(&path)).right_command_pause);
    assert!(Settings::load(Some(&path)).return_confirms_only);
    Settings { return_confirms_only: false, ..Settings::default() }.save(Some(&path)).unwrap();
    assert!(!Settings::load(Some(&path)).return_confirms_only);
    let _ = std::fs::remove_file(path);
}

#[test]
fn settings_save_keeps_unknown_keys() {
    let path = temp_path("seamtype-store", ".json");
    std::fs::write(&path, r#"{ "japanese": false, "future": 3 }"#).unwrap();
    let settings = Settings { japanese: true, live_conversion: true, ..Settings::default() };
    settings.save(Some(&path)).unwrap();
    let object: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(object["future"], 3);
    assert_eq!(Settings::load(Some(&path)), settings);
    let _ = std::fs::remove_file(path);
}

#[test]
fn app_entry_names_only_what_differs_from_the_default() {
    let mut policies = AppPolicies::default();
    policies.set_default(&AppPolicy { escape_to_app: true, ..AppPolicy::STANDARD });
    assert_eq!(policies.overrides.get("*"), Some(&Override { escape_to_app: Some(true), ..Override::default() }));

    let mut ghostty = policies.policy(Some("com.mitchellh.ghostty"));
    ghostty.conversion_threshold = AppPolicy::STRICT_THRESHOLD;
    policies.set(&ghostty, "com.mitchellh.ghostty");
    assert_eq!(
        policies.overrides.get("com.mitchellh.ghostty"),
        Some(&Override { conversion_threshold: Some(0.97), ..Override::default() })
    );

    // Set back to the default's values, the app stays listed (the user added it).
    let default = policies.default_policy();
    policies.set(&default, "com.mitchellh.ghostty");
    assert_eq!(policies.overrides.get("com.mitchellh.ghostty"), Some(&Override::default()));
    assert_eq!(policies.apps(), vec!["com.mitchellh.ghostty".to_string()]);

    // A default back at Seamtype's own leaves no "*" entry.
    policies.set_default(&AppPolicy::STANDARD);
    assert!(!policies.overrides.contains_key("*"));

    let mut game = policies.policy(Some("com.example.Game"));
    game.enabled = false;
    policies.set(&game, "com.example.Game");
    let path = temp_path("seamtype-apps", ".json");
    policies.save(Some(&path)).unwrap();
    assert!(!AppPolicies::load(Some(&path)).policy(Some("com.example.Game")).enabled);
    assert_eq!(
        AppPolicies::load(Some(&path)).apps(),
        vec!["com.example.Game".to_string(), "com.mitchellh.ghostty".to_string()]
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn forget_and_undo_learned_words() {
    let path = temp_path("seamtype-lexicon", ".json");
    let fixed = || -> Arc<dyn DecisionEngine> { Arc::new(Fixed(Language::English)) };
    let learning = LearningEngine::new(fixed(), Some(path.clone()));
    learning.learn("dlrjs", Language::Korean, None);
    assert_eq!(learning.pending(), BTreeMap::from([("dlrjs".to_string(), Language::Korean)]));
    assert_eq!(learning.last_learned(), None);
    learning.learn("dlrjs", Language::Korean, None);
    assert_eq!(learning.learned(), BTreeMap::from([("dlrjs".to_string(), Language::Korean)]));
    assert_eq!(learning.last_learned().as_deref(), Some("dlrjs"));

    learning.forget("dlrjs");
    assert!(learning.learned().is_empty() && learning.pending().is_empty());
    assert_eq!(learning.last_learned(), None);

    learning.learn("gks", Language::Korean, None);
    learning.learn("rk", Language::Korean, None);
    learning.learn("rk", Language::Korean, None);
    // Another process (the settings window, another app) edits the file; reload picks it up.
    let other = LearningEngine::new(fixed(), Some(path.clone()));
    other.forget("gks");
    learning.reload();
    assert!(learning.pending().is_empty());
    assert_eq!(learning.learned(), BTreeMap::from([("rk".to_string(), Language::Korean)]));

    learning.forget_all();
    assert!(LearningEngine::new(fixed(), Some(path.clone())).learned().is_empty());
    let _ = std::fs::remove_file(path);
}

/// Learning in one process doesn't undo what another process learned meanwhile.
#[test]
fn learning_keeps_what_another_process_learned() {
    let path = temp_path("seamtype-lexicon", ".json");
    let fixed = || -> Arc<dyn DecisionEngine> { Arc::new(Fixed(Language::English)) };
    let a = LearningEngine::new(fixed(), Some(path.clone()));
    let b = LearningEngine::new(fixed(), Some(path.clone()));
    a.learn("dlrjs", Language::Korean, None);
    // Some file systems keep whole-second times: make sure b's write is seen as a change.
    std::thread::sleep(std::time::Duration::from_millis(20));
    b.learn("gksrmf", Language::Korean, None);
    let fresh = LearningEngine::new(fixed(), Some(path.clone()));
    let expected: HashMap<String, Language> =
        [("dlrjs".to_string(), Language::Korean), ("gksrmf".to_string(), Language::Korean)].into();
    assert_eq!(fresh.pending().into_iter().collect::<HashMap<_, _>>(), expected);
    let _ = std::fs::remove_file(path);
}
