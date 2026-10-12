//! UI strings, in Korean when Windows' display language is Korean (the macOS app's ko.lproj
//! wording, with Windows' keys), English otherwise.

use std::sync::LazyLock;

use windows::Win32::Globalization::GetUserDefaultUILanguage;

static KOREAN: LazyLock<bool> = LazyLock::new(|| unsafe { GetUserDefaultUILanguage() } & 0x3FF == 0x12);

pub struct Strings {
    pub pause: &'static str,
    pub app_settings: &'static str,
    pub default_settings: &'static str,
    pub convert_words: &'static str,
    pub conversion_normal: &'static str,
    pub conversion_strict: &'static str,
    pub escape_to_app: &'static str,
    pub use_default: &'static str,
    pub japanese: &'static str,
    pub undo_learned: &'static str,
    pub forget_all: &'static str,
    pub forget_all_question: &'static str,
    pub forget_all_detail: &'static str,
    pub open_folder: &'static str,
    pub about: &'static str,
    pub about_text: &'static str,
    pub tooltip_on: &'static str,
    pub tooltip_paused: &'static str,
    pub pause_key: &'static str,
}

const KO: Strings = Strings {
    pause: "이 입력창에서 일시정지 (한/영)",
    app_settings: "{} — 앱별 설정",
    default_settings: "{} — 기본 설정",
    convert_words: "단어 변환",
    conversion_normal: "변환 기준: 보통",
    conversion_strict: "변환 기준: 엄격하게",
    escape_to_app: "Esc 키: 앱으로 (⇧Esc로 언어 바꾸기)",
    use_default: "이 앱에서 기본 설정 따르기",
    japanese: "일본어 입력 (가나)",
    undo_learned: "마지막 학습 취소 ({})",
    forget_all: "기억한 단어 모두 잊기…",
    forget_all_question: "기억한 단어를 모두 잊을까요?",
    forget_all_detail: "Esc로 두 번 고쳐서 기억한 단어와 한 번 고친 단어를 모두 지웁니다.",
    open_folder: "설정 폴더 열기",
    about: "Seamtype 정보",
    about_text: "Seamtype {} (Windows)\n\n흐름대로 써진다 — 영어 자판 그대로 치면 단어마다 영어, 한국어(두벌식), 일본어(로마자)를 골라 줍니다.\n\nMIT License · https://github.com/HaloKim/seamtype",
    tooltip_on: "Seamtype",
    tooltip_paused: "Seamtype (일시정지: 영어 그대로)",
    pause_key: "Seamtype 일시정지",
};

const EN: Strings = Strings {
    pause: "Pause in This Text Field (Han/Eng key)",
    app_settings: "{} — App Settings",
    default_settings: "{} — Default Settings",
    convert_words: "Convert Words",
    conversion_normal: "Conversion: Normal",
    conversion_strict: "Conversion: Strict",
    escape_to_app: "Esc Key: Send to App (⇧Esc switches)",
    use_default: "Use Default Settings in This App",
    japanese: "Japanese Input (kana)",
    undo_learned: "Undo Last Learned ({})",
    forget_all: "Forget All Remembered Words…",
    forget_all_question: "Forget all remembered words?",
    forget_all_detail: "Words corrected twice with Esc, and words corrected once, are all forgotten.",
    open_folder: "Open Settings Folder",
    about: "About Seamtype",
    about_text: "Seamtype {} (Windows)\n\nType as it flows — type everything on QWERTY and each word comes out as English, Korean (2-set) or Japanese (romaji).\n\nMIT License · https://github.com/HaloKim/seamtype",
    tooltip_on: "Seamtype",
    tooltip_paused: "Seamtype (paused: plain English)",
    pause_key: "Pause Seamtype",
};

pub fn get() -> &'static Strings {
    if *KOREAN { &KO } else { &EN }
}

/// `template` with its `{}` replaced by `value`.
pub fn fill(template: &str, value: &str) -> String {
    template.replacen("{}", value, 1)
}
