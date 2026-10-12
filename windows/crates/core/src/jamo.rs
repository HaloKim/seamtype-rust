//! Hangul jamo tables and the 2-set (두벌식) QWERTY layout (Jamo.swift).
//!
//! All jamo are represented as Hangul Compatibility Jamo (U+3131…U+3163),
//! which is also what gets emitted for stray, uncombined jamo.

pub(crate) const CHOSEONG: [char; 19] = [
    'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ',
];
pub(crate) const JUNGSEONG: [char; 21] = [
    'ㅏ', 'ㅐ', 'ㅑ', 'ㅒ', 'ㅓ', 'ㅔ', 'ㅕ', 'ㅖ', 'ㅗ', 'ㅘ', 'ㅙ', 'ㅚ', 'ㅛ', 'ㅜ', 'ㅝ', 'ㅞ', 'ㅟ', 'ㅠ', 'ㅡ',
    'ㅢ', 'ㅣ',
];
/// Index 0 is "no final consonant".
pub(crate) const JONGSEONG: [Option<char>; 28] = [
    None,
    Some('ㄱ'), Some('ㄲ'), Some('ㄳ'), Some('ㄴ'), Some('ㄵ'), Some('ㄶ'), Some('ㄷ'), Some('ㄹ'), Some('ㄺ'),
    Some('ㄻ'), Some('ㄼ'), Some('ㄽ'), Some('ㄾ'), Some('ㄿ'), Some('ㅀ'), Some('ㅁ'), Some('ㅂ'), Some('ㅄ'),
    Some('ㅅ'), Some('ㅆ'), Some('ㅇ'), Some('ㅈ'), Some('ㅊ'), Some('ㅋ'), Some('ㅌ'), Some('ㅍ'), Some('ㅎ'),
];

pub(crate) fn cho_index(j: char) -> Option<usize> {
    CHOSEONG.iter().position(|&c| c == j)
}

pub(crate) fn jung_index(j: char) -> Option<usize> {
    JUNGSEONG.iter().position(|&c| c == j)
}

pub(crate) fn jong_index(j: char) -> Option<usize> {
    JONGSEONG.iter().position(|&c| c == Some(j))
}

pub(crate) fn vowel_combo(a: char, b: char) -> Option<char> {
    Some(match (a, b) {
        ('ㅗ', 'ㅏ') => 'ㅘ',
        ('ㅗ', 'ㅐ') => 'ㅙ',
        ('ㅗ', 'ㅣ') => 'ㅚ',
        ('ㅜ', 'ㅓ') => 'ㅝ',
        ('ㅜ', 'ㅔ') => 'ㅞ',
        ('ㅜ', 'ㅣ') => 'ㅟ',
        ('ㅡ', 'ㅣ') => 'ㅢ',
        _ => return None,
    })
}

const FINAL_COMBOS: [(char, char, char); 11] = [
    ('ㄱ', 'ㅅ', 'ㄳ'), ('ㄴ', 'ㅈ', 'ㄵ'), ('ㄴ', 'ㅎ', 'ㄶ'),
    ('ㄹ', 'ㄱ', 'ㄺ'), ('ㄹ', 'ㅁ', 'ㄻ'), ('ㄹ', 'ㅂ', 'ㄼ'), ('ㄹ', 'ㅅ', 'ㄽ'), ('ㄹ', 'ㅌ', 'ㄾ'), ('ㄹ', 'ㅍ', 'ㄿ'),
    ('ㄹ', 'ㅎ', 'ㅀ'),
    ('ㅂ', 'ㅅ', 'ㅄ'),
];

pub(crate) fn final_combo(a: char, b: char) -> Option<char> {
    FINAL_COMBOS.iter().find(|&&(x, y, _)| x == a && y == b).map(|&(_, _, c)| c)
}

/// Inverse of the final combos: compound final → (stays as final, moves to next syllable).
pub(crate) fn final_split(combined: char) -> Option<(char, char)> {
    FINAL_COMBOS.iter().find(|&&(_, _, c)| c == combined).map(|&(a, b, _)| (a, b))
}

pub fn is_consonant(j: char) -> bool {
    cho_index(j).is_some() || jong_index(j).is_some()
}

pub fn is_vowel(j: char) -> bool {
    jung_index(j).is_some()
}

pub(crate) fn syllable(cho: char, jung: char, jong: Option<char>) -> char {
    let c = cho_index(cho).expect("initial");
    let v = jung_index(jung).expect("medial");
    let f = jong.and_then(jong_index).unwrap_or(0);
    char::from_u32(0xAC00 + ((c * 21 + v) * 28 + f) as u32).expect("Hangul syllable")
}

/// The 2-set (두벌식) layout on QWERTY keys.
pub mod dubeolsik {
    use super::is_consonant;

    fn lower(key: char) -> Option<char> {
        Some(match key {
            'q' => 'ㅂ', 'w' => 'ㅈ', 'e' => 'ㄷ', 'r' => 'ㄱ', 't' => 'ㅅ', 'y' => 'ㅛ', 'u' => 'ㅕ', 'i' => 'ㅑ',
            'o' => 'ㅐ', 'p' => 'ㅔ', 'a' => 'ㅁ', 's' => 'ㄴ', 'd' => 'ㅇ', 'f' => 'ㄹ', 'g' => 'ㅎ', 'h' => 'ㅗ',
            'j' => 'ㅓ', 'k' => 'ㅏ', 'l' => 'ㅣ', 'z' => 'ㅋ', 'x' => 'ㅌ', 'c' => 'ㅊ', 'v' => 'ㅍ', 'b' => 'ㅠ',
            'n' => 'ㅜ', 'm' => 'ㅡ',
            _ => return None,
        })
    }

    fn shifted(key: char) -> Option<char> {
        Some(match key {
            'Q' => 'ㅃ', 'W' => 'ㅉ', 'E' => 'ㄸ', 'R' => 'ㄲ', 'T' => 'ㅆ', 'O' => 'ㅒ', 'P' => 'ㅖ',
            _ => return None,
        })
    }

    /// Keys whose shifted form produces a different jamo. Shift on any other
    /// letter is a signal the user is typing English.
    pub fn is_korean_shift_key(key: char) -> bool {
        shifted(key).is_some()
    }

    /// The jamo a key produces, or None for non-letters.
    /// Uppercase letters without a shifted jamo fall back to the lowercase jamo,
    /// mirroring how the 2-set keyboard behaves.
    pub fn jamo(key: char) -> Option<char> {
        if let Some(j) = shifted(key) {
            return Some(j);
        }
        if !key.is_ascii() {
            return None;
        }
        lower(key.to_ascii_lowercase())
    }

    /// A Korean syllable has to start with a consonant key.
    pub fn starts_syllable(key: char) -> bool {
        jamo(key).map(is_consonant).unwrap_or(false)
    }
}
