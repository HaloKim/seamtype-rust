//! Romaji → hiragana, following the conventions of the macOS and Microsoft
//! Japanese input methods (nn/n' → ん, doubled consonants → っ, x/l small kana,
//! - → ー). Letters that can't become kana are kept and counted: like stray jamo
//! for Hangul, they are the strongest sign the user wasn't typing Japanese.
//! RomajiComposer.swift.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct RomajiResult {
    /// Kana with any unconvertible letters left in place (こんにc while typing).
    pub text: String,
    /// Letters in the middle or (when finished) at the end that never became kana.
    pub stray_letters: usize,
    /// Letters at the end still waiting to become kana ("k", "ky", "n"); only when partial.
    pub pending: String,
    /// Number of kana produced.
    pub kana_count: usize,
}

impl RomajiResult {
    pub fn is_clean(&self) -> bool {
        self.stray_letters == 0 && self.pending.is_empty()
    }
}

const VOWELS: &str = "aiueo";
const CONSONANTS: &str = "bcdfghjklmpqrstvwxyz";

/// Symbols as Japanese IMEs type them by default: full-width, with
/// [ ] / ~ - as 「」・〜ー. Not ' (it ends ん in kan'i), and digits and spaces stay half-width.
pub static SYMBOLS: LazyLock<HashMap<char, &'static str>> = LazyLock::new(|| {
    HashMap::from([
        (',', "、"), ('.', "。"), ('?', "？"), ('!', "！"), ('-', "ー"), ('/', "・"), ('[', "「"), (']', "」"),
        ('~', "〜"), ('_', "＿"), ('(', "（"), (')', "）"), (':', "："), (';', "；"), ('"', "”"), ('@', "＠"),
        ('#', "＃"), ('$', "＄"), ('%', "％"), ('&', "＆"), ('*', "＊"), ('+', "＋"), ('=', "＝"), ('<', "＜"),
        ('>', "＞"), ('{', "｛"), ('}', "｝"), ('|', "｜"), ('^', "＾"), ('`', "｀"), ('\\', "＼"),
    ])
});

/// What the symbols become, except ー, which is part of a word (らーめん) rather than punctuation.
pub static PUNCTUATION: LazyLock<HashSet<char>> = LazyLock::new(|| {
    SYMBOLS.values().filter_map(|s| s.chars().next()).filter(|&c| c != 'ー').collect()
});

pub fn symbol(c: char) -> Option<&'static str> {
    SYMBOLS.get(&c).copied()
}

pub fn is_symbol(c: char) -> bool {
    SYMBOLS.contains_key(&c)
}

static TABLE: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    let base: &[(&str, &str)] = &[
        ("a", "あ"), ("i", "い"), ("u", "う"), ("e", "え"), ("o", "お"),
        ("ka", "か"), ("ki", "き"), ("ku", "く"), ("ke", "け"), ("ko", "こ"),
        ("ga", "が"), ("gi", "ぎ"), ("gu", "ぐ"), ("ge", "げ"), ("go", "ご"),
        ("sa", "さ"), ("si", "し"), ("su", "す"), ("se", "せ"), ("so", "そ"),
        ("za", "ざ"), ("zi", "じ"), ("zu", "ず"), ("ze", "ぜ"), ("zo", "ぞ"),
        ("ta", "た"), ("ti", "ち"), ("tu", "つ"), ("te", "て"), ("to", "と"),
        ("da", "だ"), ("di", "ぢ"), ("du", "づ"), ("de", "で"), ("do", "ど"),
        ("na", "な"), ("ni", "に"), ("nu", "ぬ"), ("ne", "ね"), ("no", "の"),
        ("ha", "は"), ("hi", "ひ"), ("hu", "ふ"), ("he", "へ"), ("ho", "ほ"),
        ("ba", "ば"), ("bi", "び"), ("bu", "ぶ"), ("be", "べ"), ("bo", "ぼ"),
        ("pa", "ぱ"), ("pi", "ぴ"), ("pu", "ぷ"), ("pe", "ぺ"), ("po", "ぽ"),
        ("ma", "ま"), ("mi", "み"), ("mu", "む"), ("me", "め"), ("mo", "も"),
        ("ya", "や"), ("yi", "い"), ("yu", "ゆ"), ("ye", "いぇ"), ("yo", "よ"),
        ("ra", "ら"), ("ri", "り"), ("ru", "る"), ("re", "れ"), ("ro", "ろ"),
        ("wa", "わ"), ("wi", "うぃ"), ("wu", "う"), ("we", "うぇ"), ("wo", "を"),
        ("fa", "ふぁ"), ("fi", "ふぃ"), ("fu", "ふ"), ("fe", "ふぇ"), ("fo", "ふぉ"),
        ("va", "ゔぁ"), ("vi", "ゔぃ"), ("vu", "ゔ"), ("ve", "ゔぇ"), ("vo", "ゔぉ"),
        ("qa", "くぁ"), ("qi", "くぃ"), ("qu", "く"), ("qe", "くぇ"), ("qo", "くぉ"),
        ("shi", "し"), ("chi", "ち"), ("tsu", "つ"), ("ji", "じ"),
        ("sha", "しゃ"), ("shu", "しゅ"), ("she", "しぇ"), ("sho", "しょ"),
        ("cha", "ちゃ"), ("chu", "ちゅ"), ("che", "ちぇ"), ("cho", "ちょ"),
        ("ja", "じゃ"), ("ju", "じゅ"), ("je", "じぇ"), ("jo", "じょ"),
        ("tsa", "つぁ"), ("tsi", "つぃ"), ("tse", "つぇ"), ("tso", "つぉ"),
        ("tha", "てゃ"), ("thi", "てぃ"), ("thu", "てゅ"), ("the", "てぇ"), ("tho", "てょ"),
        ("dha", "でゃ"), ("dhi", "でぃ"), ("dhu", "でゅ"), ("dhe", "でぇ"), ("dho", "でょ"),
        ("twu", "とぅ"), ("dwu", "どぅ"),
        ("wha", "うぁ"), ("whi", "うぃ"), ("whu", "う"), ("whe", "うぇ"), ("who", "うぉ"),
        ("xa", "ぁ"), ("xi", "ぃ"), ("xu", "ぅ"), ("xe", "ぇ"), ("xo", "ぉ"),
        ("la", "ぁ"), ("li", "ぃ"), ("lu", "ぅ"), ("le", "ぇ"), ("lo", "ぉ"),
        ("xya", "ゃ"), ("xyu", "ゅ"), ("xyo", "ょ"), ("lya", "ゃ"), ("lyu", "ゅ"), ("lyo", "ょ"),
        ("xtu", "っ"), ("ltu", "っ"), ("xtsu", "っ"), ("ltsu", "っ"), ("xwa", "ゎ"), ("lwa", "ゎ"),
        // As macOS and MS-IME accept them (not ca/ci/cu: they made English words read as romaji).
        ("kwa", "くぁ"), ("gwa", "ぐぁ"), ("xn", "ん"),
    ];
    let mut t: HashMap<String, String> = base.iter().map(|&(k, v)| (k.to_string(), v.to_string())).collect();
    for (&symbol, &full) in SYMBOLS.iter() {
        t.insert(symbol.to_string(), full.to_string());
    }
    // Contracted sounds: きゃ, ぎゅ, みょ…
    let rows = [
        ("k", "き"), ("g", "ぎ"), ("s", "し"), ("z", "じ"), ("t", "ち"), ("d", "ぢ"), ("n", "に"), ("h", "ひ"),
        ("b", "び"), ("p", "ぴ"), ("m", "み"), ("r", "り"), ("c", "ち"), ("j", "じ"), ("f", "ふ"),
    ];
    for (consonant, i) in rows {
        t.insert(format!("{consonant}ya"), format!("{i}ゃ"));
        t.insert(format!("{consonant}yi"), format!("{i}ぃ"));
        t.insert(format!("{consonant}yu"), format!("{i}ゅ"));
        t.insert(format!("{consonant}ye"), format!("{i}ぇ"));
        t.insert(format!("{consonant}yo"), format!("{i}ょ"));
    }
    t
});

/// Every proper prefix of a table entry: what a word may end in while still being typed.
static PREFIXES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let mut p: HashSet<String> = HashSet::from(["n".to_string()]);
    for key in TABLE.keys() {
        let chars: Vec<char> = key.chars().collect();
        if chars.len() > 1 {
            for length in 1..chars.len() {
                p.insert(chars[..length].iter().collect());
            }
        }
    }
    p
});

/// The shortest table entry for each single kana (ties: alphabetical); ん as "n'" so a
/// vowel typed after it can't take it over (n'a is んあ, nna would be んな).
static SPELLING: LazyLock<HashMap<char, String>> = LazyLock::new(|| {
    let mut s: HashMap<char, String> = HashMap::from([('ん', "n'".to_string())]);
    for (key, value) in TABLE.iter() {
        let mut chars = value.chars();
        let (Some(ch), None) = (chars.next(), chars.next()) else { continue };
        if let Some(old) = s.get(&ch) {
            let (old_n, key_n) = (old.chars().count(), key.chars().count());
            if old_n < key_n || (old_n == key_n && old.as_str() < key.as_str()) {
                continue;
            }
        }
        if ch != 'ん' {
            s.insert(ch, key.clone());
        }
    }
    s
});

/// The single-kana spellings (for tests).
pub fn spellings() -> Vec<(char, String)> {
    SPELLING.iter().map(|(&k, v)| (k, v.clone())).collect()
}

/// `partial`: the word is still being typed, so a trailing "k" or "n" is waiting for its
/// vowel rather than stray.
pub fn compose(keys: &str, partial: bool) -> RomajiResult {
    let chars: Vec<char> = keys.to_lowercase().chars().collect();
    let mut out = String::new();
    let mut stray = 0;
    let mut kana = 0;
    let mut pending = String::new();
    let mut i = 0;
    let is_vowel = |c: char| VOWELS.contains(c);

    let mut emit = |out: &mut String, s: &str| {
        out.push_str(s);
        kana += s.chars().count();
    };

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();

        if c == 'n' {
            if let Some(next) = next {
                if next == '\'' {
                    emit(&mut out, "ん");
                    i += 2;
                    continue;
                }
                if next == 'n' {
                    // "nni" → んに, "nn" → ん
                    let after = chars.get(i + 2).copied();
                    emit(&mut out, "ん");
                    i += if after.map(|a| is_vowel(a) || a == 'y').unwrap_or(false) { 1 } else { 2 };
                    continue;
                }
                if !is_vowel(next) && next != 'y' {
                    emit(&mut out, "ん");
                    i += 1;
                    continue;
                }
            } else {
                if partial {
                    pending = "n".to_string();
                } else {
                    emit(&mut out, "ん");
                }
                i += 1;
                continue;
            }
        } else if next == Some(c) && CONSONANTS.contains(c) {
            emit(&mut out, "っ");
            i += 1;
            continue;
        } else if c == 't' && next == Some('c') && i + 2 < chars.len() && chars[i + 2] == 'h' {
            emit(&mut out, "っ"); // matcha: "tch" → っch
            i += 1;
            continue;
        }

        let mut matched = false;
        let longest = 4.min(chars.len() - i);
        for length in (1..=longest).rev() {
            let key: String = chars[i..i + length].iter().collect();
            if let Some(k) = TABLE.get(&key) {
                emit(&mut out, k);
                i += length;
                matched = true;
                break;
            }
        }
        if matched {
            continue;
        }

        let rest: String = chars[i..].iter().collect();
        if partial && PREFIXES.contains(&rest) {
            pending = rest;
            break;
        }
        out.push(c);
        stray += 1;
        i += 1;
    }
    RomajiResult { text: out + &pending, stray_letters: stray, pending, kana_count: kana }
}

/// Hiragana → katakana (for F7-style katakana output and loanwords).
pub fn katakana(hiragana: &str) -> String {
    hiragana
        .chars()
        .map(|c| {
            let v = c as u32;
            if (0x3041..=0x3096).contains(&v) { char::from_u32(v + 0x60).unwrap_or(c) } else { c }
        })
        .collect()
}

/// A spelling that composes back to `kana` (し → "si", ん → "n'", っ → "ltu"), or None if
/// some character has none. For keys rewritten after Delete took off a whole kana.
pub fn romaji(kana: &str) -> Option<String> {
    let mut out = String::new();
    for ch in kana.chars() {
        out.push_str(SPELLING.get(&ch)?);
    }
    Some(out)
}
