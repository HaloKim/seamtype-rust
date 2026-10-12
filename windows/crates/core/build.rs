//! Bakes the shared data files (Sources/SeamtypeCore/Data, the same ones the macOS app ships)
//! into compact tables the library maps straight from the binary: no parsing when a process
//! first types, and one read-only copy shared by every process the input method runs in.
//!
//! The tables hold exactly what the Swift models build at load time (EnglishModel's levels and
//! letter trigrams, KoreanModel's word sets, JapaneseModel's kana counts), so the numbers match.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[path = "src/english_lists.rs"]
mod english_lists;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let data = env::var("SEAMTYPE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest.join("../../../Sources/SeamtypeCore/Data"));
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-env-changed=SEAMTYPE_DATA_DIR");

    let read = |path: &Path| -> String {
        println!("cargo:rerun-if-changed={}", path.display());
        fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
    };

    let english = read(&data.join("english_words.txt"));
    let spelling = read(&manifest.join("data/spelling_trigrams.txt"));
    let levels = english_levels(&english);
    fs::write(out.join("english_levels.bin"), word_table(&levels)).unwrap();
    fs::write(out.join("english_trigrams.bin"), english_trigrams(&spelling)).unwrap();

    let words = read(&data.join("korean_words.txt"));
    let korean_words: Vec<(String, Option<u8>)> = lines(&words)
        .filter_map(|line| fields(line).next().map(|w| (w.to_string(), None)))
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .collect();
    fs::write(out.join("korean_words.bin"), word_table(&korean_words)).unwrap();

    let vocabulary = read(&data.join("korean_vocabulary.txt"));
    let korean_vocabulary: Vec<(String, Option<u8>)> = lines(&vocabulary)
        .map(|w| (w.to_string(), None))
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .collect();
    fs::write(out.join("korean_vocabulary.bin"), word_table(&korean_vocabulary)).unwrap();

    // Small enough to parse at runtime; copied so the library needs no path into the source tree.
    fs::write(out.join("korean_syllables.txt"), read(&data.join("korean_syllables.txt"))).unwrap();

    let kana = read(&data.join("kana_trigrams.txt"));
    fs::write(out.join("kana_model.bin"), kana_model(&kana)).unwrap();
}

/// Non-empty lines, as Swift's `split(whereSeparator: \.isNewline)` gives them.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter(|l| !l.is_empty())
}

/// Tab-separated fields without empty ones, as Swift's `split(separator: "\t")`.
fn fields(line: &str) -> impl Iterator<Item = &str> {
    line.split('\t').filter(|f| !f.is_empty())
}

/// EnglishModel's levels: the SCOWL list (a later line overwrites an earlier one), the common
/// words at level 35 or better, then only ASCII-letter words longer than two letters or listed
/// as real short words, apostrophes dropped, each at its lowest level.
fn english_levels(text: &str) -> Vec<(String, Option<u8>)> {
    let mut raw: HashMap<String, i64> = HashMap::new();
    for line in lines(text) {
        let parts: Vec<&str> = fields(line).collect();
        if parts.len() == 2 {
            if let Ok(level) = parts[1].parse::<i64>() {
                raw.insert(parts[0].to_string(), level);
            }
        }
    }
    let extra = english_lists::EXTRA_LEVEL;
    for word in english_lists::COMMON_WORDS.split_whitespace() {
        let w = word.to_lowercase();
        let level = raw.get(&w).copied().unwrap_or(extra).min(extra);
        raw.insert(w, level);
    }
    let short: HashSet<&str> = english_lists::SHORT_WORDS.split_whitespace().collect();
    let mut levels: BTreeMap<String, i64> = BTreeMap::new();
    for (word, level) in raw {
        let word: String = word.chars().filter(|&c| c != '\'').collect();
        if word.is_empty() || !word.chars().all(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        if word.chars().count() > 2 || short.contains(word.as_str()) {
            let entry = levels.entry(word).or_insert(level);
            *entry = (*entry).min(level);
        }
    }
    levels
        .into_iter()
        .map(|(w, l)| (w, Some(u8::try_from(l).expect("SCOWL level fits a byte"))))
        .collect()
}

/// Letter trigram and context counts (27 symbols: 0 = word boundary, 1…26 = a…z) of the macOS
/// spelling list plus the common words, as EnglishModel trains them. Little-endian f32.
fn english_trigrams(spelling: &str) -> Vec<u8> {
    const N: usize = 27;
    let code = |c: char| -> usize { if c == '_' { 0 } else { c as usize - 'a' as usize + 1 } };
    let mut tri = vec![0u64; N * N * N];
    let mut ctx = vec![0u64; N * N];
    for line in lines(spelling).filter(|l| !l.starts_with('#')) {
        let parts: Vec<&str> = fields(line).collect();
        let chars: Vec<char> = parts[0].chars().collect();
        let count: u64 = parts[1].parse().unwrap();
        let (a, b, c) = (code(chars[0]), code(chars[1]), code(chars[2]));
        tri[(a * N + b) * N + c] += count;
        ctx[a * N + b] += count;
    }
    for word in english_lists::COMMON_WORDS.split_whitespace() {
        if !word.chars().all(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        let mut codes = vec![0usize, 0];
        codes.extend(word.chars().filter(|c| c.is_ascii_lowercase()).map(code));
        codes.push(0);
        for i in 2..codes.len() {
            tri[(codes[i - 2] * N + codes[i - 1]) * N + codes[i]] += 1;
            ctx[codes[i - 2] * N + codes[i - 1]] += 1;
        }
    }
    let mut out = Vec::with_capacity((tri.len() + ctx.len()) * 4);
    for &n in tri.iter().chain(ctx.iter()) {
        assert!(n < (1 << 24), "counts must stay exact in f32, as in the Swift model");
        out.extend_from_slice(&(n as f32).to_le_bytes());
    }
    out
}

/// Sorted words (UTF-8 byte order) with an optional byte each:
/// u32 count, u32 offsets[count + 1], u8 values[count] (only if any), then the words.
fn word_table(entries: &[(String, Option<u8>)]) -> Vec<u8> {
    let mut sorted: Vec<&(String, Option<u8>)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    sorted.dedup_by(|a, b| a.0 == b.0);
    let with_values = sorted.iter().any(|e| e.1.is_some());
    let mut out = Vec::new();
    out.extend_from_slice(&(sorted.len() as u32).to_le_bytes());
    out.push(with_values as u8);
    let mut offset = 0u32;
    for entry in &sorted {
        out.extend_from_slice(&offset.to_le_bytes());
        offset += entry.0.len() as u32;
    }
    out.extend_from_slice(&offset.to_le_bytes());
    if with_values {
        out.extend(sorted.iter().map(|e| e.1.unwrap_or(0)));
    }
    for entry in &sorted {
        out.extend_from_slice(entry.0.as_bytes());
    }
    out
}

/// A character Swift would fold into the one before it (so the line wouldn't be 3 characters).
fn is_combining(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F | 0x3099..=0x309A | 0xFE00..=0xFE0F | 0x200D)
}

/// JapaneseModel's counts from kana trigram lines ("abc<TAB>count"):
/// u32 nsym, u32 symbols[nsym] (sorted), f32 total, f32 vocabulary, f32 unigrams[nsym],
/// f32 bigramContexts[nsym], f32 bigrams[nsym²] (b, c), f32 trigramContexts[nsym²] (a, b),
/// u32 ntri, u32 keys[ntri] (sorted (a·nsym + b)·nsym + c), f32 trigrams[ntri].
/// A zero count means the Swift dictionary had no entry.
fn kana_model(text: &str) -> Vec<u8> {
    let mut rows: Vec<([char; 3], u64)> = Vec::new();
    for line in lines(text) {
        let parts: Vec<&str> = fields(line).collect();
        if parts.len() != 2 {
            continue;
        }
        let Ok(count) = parts[1].parse::<f64>() else { continue };
        let chars: Vec<char> = parts[0].chars().collect();
        if chars.len() != 3 || chars.iter().any(|&c| is_combining(c)) {
            if chars.iter().any(|&c| is_combining(c)) {
                println!("cargo:warning=kana_trigrams.txt: skipping {:?} (combining mark)", parts[0]);
            }
            continue;
        }
        assert!(count.fract() == 0.0 && count >= 0.0, "kana counts are whole numbers");
        rows.push(([chars[0], chars[1], chars[2]], count as u64));
    }
    let symbols: Vec<char> = {
        let set: HashSet<char> = rows.iter().flat_map(|(t, _)| t.iter().copied()).collect();
        let mut v: Vec<char> = set.into_iter().collect();
        v.sort();
        v
    };
    let n = symbols.len();
    assert!(n * n * n < u32::MAX as usize);
    let id = |c: char| symbols.binary_search(&c).unwrap();
    let mut total = 0u64;
    let mut unigrams = vec![0u64; n];
    let mut bigram_contexts = vec![0u64; n];
    let mut bigrams = vec![0u64; n * n];
    let mut trigram_contexts = vec![0u64; n * n];
    let mut trigrams: BTreeMap<u32, u64> = BTreeMap::new();
    for ([a, b, c], count) in &rows {
        let (a, b, c) = (id(*a), id(*b), id(*c));
        *trigrams.entry(((a * n + b) * n + c) as u32).or_insert(0) += count;
        trigram_contexts[a * n + b] += count;
        bigrams[b * n + c] += count;
        bigram_contexts[b] += count;
        unigrams[c] += count;
        total += count;
    }
    // Every sum the Swift model makes in Float is a whole number below 2^24, so exact there too.
    assert!(total < (1 << 24), "kana counts must stay exact in f32, as in the Swift model");
    let mut out = Vec::new();
    let u32s = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    u32s(&mut out, n as u32);
    for &s in &symbols {
        u32s(&mut out, s as u32);
    }
    let f32s = |out: &mut Vec<u8>, v: u64| out.extend_from_slice(&(v as f32).to_le_bytes());
    f32s(&mut out, total);
    f32s(&mut out, n as u64);
    for list in [&unigrams, &bigram_contexts, &bigrams, &trigram_contexts] {
        for &v in list.iter() {
            f32s(&mut out, v);
        }
    }
    u32s(&mut out, trigrams.len() as u32);
    for &k in trigrams.keys() {
        u32s(&mut out, k);
    }
    for &v in trigrams.values() {
        f32s(&mut out, v);
    }
    out
}
