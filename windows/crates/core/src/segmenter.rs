//! Finds English words typed inside a Japanese phrase, the way Japanese is written:
//! `kyouhagoogledekensaku` → きょうは + google + でけんさく. JapaneseSegmenter.swift.
//!
//! Only used when the whole phrase doesn't compose to clean kana (google → ごおgぇ leaves
//! a stray "g"). Splits the romaji into clean Japanese spans and known English words
//! (3+ letters), maximizing the Japanese model's score plus a fixed score per English
//! word, with a cost per switch so ordinary Japanese is never carved up.
//!
//! The input method asks on every keystroke, so the search only cuts where an English
//! word could start or end (no English word anywhere → nothing to do), and the last
//! answer is remembered.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;

use crate::english_model::EnglishModel;
use crate::japanese_model::JapaneseModel;
use crate::romaji;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Segment {
    Japanese { keys: String, kana: String },
    English(String),
}

pub type Scored = (Vec<Segment>, f64);

const PARTICLES: [&str; 13] = ["で", "に", "を", "は", "が", "の", "と", "も", "へ", "や", "か", "ね", "よ"];
/// Case particles that may follow an English noun on their own: githubで, slackから.
/// Not ね/よ/か/や/へ/って: names end in them far more often (jayne, rebecka, claudette).
const TRAILING_PARTICLES: [&str; 11] = ["で", "に", "を", "は", "が", "の", "と", "も", "から", "まで", "より"];

/// Longest English word looked for inside a phrase.
const MAX_ENGLISH_LENGTH: usize = 20;

pub struct JapaneseSegmenter {
    pub english: &'static EnglishModel,
    pub japanese: &'static JapaneseModel,
    /// Log-probability credited per letter of a known English word.
    pub english_letter_log_p: f64,
    /// Cost of each switch into English.
    pub switch_cost: f64,
    /// The last few answers: the preview, the decision and the commit ask about the same keys.
    cache: Mutex<Vec<(String, Option<Scored>)>>,
}

#[derive(Clone)]
struct State {
    score: f64,
    back: Option<(usize, usize, bool)>,
    segment: Option<Segment>,
}

impl JapaneseSegmenter {
    pub fn new(english: &'static EnglishModel, japanese: &'static JapaneseModel) -> JapaneseSegmenter {
        JapaneseSegmenter {
            english,
            japanese,
            english_letter_log_p: -1.6,
            switch_cost: 4.0,
            cache: Mutex::new(Vec::new()),
        }
    }

    /// The segments, or None when the keys compose cleanly (nothing to split) or no
    /// split with both Japanese and English beats leaving stray letters.
    pub fn segment(&self, keys: &str) -> Option<Vec<Segment>> {
        self.scored_segments(keys).map(|s| s.0)
    }

    /// Segments with their total log-probability (Japanese spans + English credit − switches).
    pub fn scored_segments(&self, keys: &str) -> Option<Scored> {
        if let Some(hit) = self.cache.lock().unwrap().iter().find(|e| e.0 == keys) {
            return hit.1.clone();
        }
        let result = self.search(keys);
        let mut cache = self.cache.lock().unwrap();
        cache.insert(0, (keys.to_string(), result.clone()));
        if cache.len() > 8 {
            cache.pop();
        }
        result
    }

    /// `githubde` → github + で: a known English word and one case particle, the whole not
    /// itself English. Decided like `github에`, whatever the previous word was. Checked
    /// directly from the end, longest English word first.
    pub fn english_with_particle(&self, keys: &str) -> Option<(String, String)> {
        let chars: Vec<char> = keys.chars().collect();
        if chars.len() < 4 || self.english.is_known_word(keys) || romaji::compose(keys, false).stray_letters == 0 {
            return None;
        }
        let low = 3.max(chars.len().saturating_sub(4));
        for cut in (low..=chars.len() - 1).rev() {
            let front: String = chars[..cut].iter().collect();
            let tail = romaji::compose(&chars[cut..].iter().collect::<String>(), false);
            if !(tail.is_clean()
                && TRAILING_PARTICLES.contains(&tail.text.as_str())
                && front.chars().all(|c| c.is_alphabetic())
                && self.english.is_known_word(&front))
            {
                continue;
            }
            return Some((front, tail.text));
        }
        None
    }

    fn search(&self, keys: &str) -> Option<Scored> {
        if romaji::compose(keys, false).stray_letters == 0 {
            return None;
        }
        let chars: Vec<char> = keys.chars().collect();
        let n = chars.len();
        // Where known English words (3+ letters) are: end positions per start.
        let mut english_ends: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut cuts: BTreeSet<usize> = BTreeSet::from([0, n]);
        for i in 0..n {
            if !chars[i].is_alphabetic() {
                continue;
            }
            let mut j = i;
            while j < n && j - i < MAX_ENGLISH_LENGTH && chars[j].is_alphabetic() {
                j += 1;
                if j - i >= 3 && self.english.is_known_word(&chars[i..j].iter().collect::<String>()) {
                    english_ends[i].push(j);
                    cuts.insert(i);
                    cuts.insert(j);
                }
            }
        }
        if !(cuts.len() > 2 || english_ends[0].contains(&n)) {
            return None;
        }
        let positions: Vec<usize> = cuts.into_iter().collect();
        let index: HashMap<usize, usize> = positions.iter().enumerate().map(|(i, &p)| (p, i)).collect();
        // One Japanese span runs from the start or an English word's end to the end or an
        // English word's start (two spans side by side would only score worse).
        let english_starts: HashSet<usize> = (0..n).filter(|&i| !english_ends[i].is_empty()).collect();
        let english_end_set: HashSet<usize> = english_ends.iter().flatten().copied().collect();
        let japanese_from: Vec<bool> = positions.iter().map(|&p| p == 0 || english_end_set.contains(&p)).collect();
        let japanese_to: Vec<bool> = positions.iter().map(|&p| p == n || english_starts.contains(&p)).collect();

        // best[position][kana so far, capped at 3][used English]: the best path to there.
        let mut best: Vec<[[Option<State>; 2]; 4]> = vec![Default::default(); positions.len()];
        best[0][0][0] = Some(State { score: 0.0, back: None, segment: None });
        for p in 0..positions.len() {
            for kana in 0..=3 {
                for used in 0..=1 {
                    let Some(state_score) = best[p][kana][used].as_ref().map(|s| s.score) else { continue };
                    let from = positions[p];
                    let offer = |best: &mut Vec<[[Option<State>; 2]; 4]>,
                                     q: usize,
                                     kana_after: usize,
                                     used_after: usize,
                                     score: f64,
                                     segment: Segment| {
                        let total = state_score + score;
                        let better = best[q][kana_after][used_after].as_ref().map(|s| total > s.score).unwrap_or(true);
                        if better {
                            best[q][kana_after][used_after] =
                                Some(State { score: total, back: Some((p, kana, used == 1)), segment: Some(segment) });
                        }
                    };
                    if from < n {
                        for &end in &english_ends[from] {
                            let word: String = chars[from..end].iter().collect();
                            let credit = self.english_letter_log_p * word.chars().count() as f64 - self.switch_cost;
                            offer(&mut best, index[&end], kana, 1, credit, Segment::English(word));
                        }
                    }
                    if japanese_from[p] {
                        for q in (p + 1)..positions.len() {
                            if !japanese_to[q] {
                                continue;
                            }
                            let piece: String = chars[from..positions[q]].iter().collect();
                            // A letter no vowel can rescue (the g of "gl") fails every longer span too.
                            let reading = romaji::compose(&piece, true);
                            if reading.stray_letters > 0 {
                                break;
                            }
                            let composed = if reading.pending.is_empty() { reading } else { romaji::compose(&piece, false) };
                            // A one-kana span must be a particle (googleで), not a leftover (would + ん).
                            if !(composed.is_clean()
                                && !composed.text.is_empty()
                                && (composed.text.chars().count() >= 2 || PARTICLES.contains(&composed.text.as_str())))
                            {
                                continue;
                            }
                            // Each span scores as Japanese on its own (start/end markers included).
                            let score = self.japanese.log_probability(&composed.text, false);
                            let kana_after = 3.min(kana + composed.text.chars().count());
                            offer(&mut best, q, kana_after, used, score, Segment::Japanese { keys: piece, kana: composed.text });
                        }
                    }
                }
            }
        }

        // Needs real Japanese around the English: at least 3 kana (not mum + ばい), or exactly
        // an English word with a particle after it (github + で).
        let mut answer: Option<Scored> = None;
        let last = positions.len() - 1;
        for kana in 0..=3 {
            let Some(state) = best[last][kana][1].as_ref() else { continue };
            let segments = path((last, kana, true), &best);
            if !(kana >= 3 || self.ends_in_particle(&segments, keys)) {
                continue;
            }
            if answer.as_ref().map(|a| state.score > a.1).unwrap_or(true) {
                answer = Some((merged(segments), state.score));
            }
        }
        answer
    }

    /// Exactly an English word plus one particle, where the whole isn't itself English.
    fn ends_in_particle(&self, segments: &[Segment], keys: &str) -> bool {
        match segments {
            [Segment::English(_), Segment::Japanese { kana, .. }] => {
                TRAILING_PARTICLES.contains(&kana.as_str()) && !self.english.is_known_word(keys)
            }
            _ => false,
        }
    }
}

fn path(end: (usize, usize, bool), best: &[[[Option<State>; 2]; 4]]) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut at = Some(end);
    while let Some((position, kana, english)) = at {
        let Some(state) = best[position][kana][english as usize].as_ref() else { break };
        let Some(segment) = state.segment.clone() else { break };
        segments.push(segment);
        at = state.back;
    }
    segments.reverse();
    segments
}

/// Adjacent Japanese spans join into one (きょうは + でけんさく stay separate only around English).
fn merged(segments: Vec<Segment>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    for segment in segments {
        if let (Segment::Japanese { keys: k2, kana: kana2 }, Some(Segment::Japanese { keys: k1, kana: kana1 })) =
            (&segment, out.last_mut())
        {
            k1.push_str(k2);
            kana1.push_str(kana2);
        } else {
            out.push(segment);
        }
    }
    out
}

/// The final reading of romaji keys: kana, with English words kept when a segmenter finds them.
pub fn reading(keys: &str, segmenter: Option<&JapaneseSegmenter>) -> String {
    if let Some(segments) = segmenter.and_then(|s| s.segment(keys)) {
        return text(&segments);
    }
    romaji::compose(keys, false).text
}

/// Kana with the English words left in place: きょうはgoogleでけんさく.
pub fn text(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|s| match s {
            Segment::Japanese { kana, .. } => kana.as_str(),
            Segment::English(word) => word.as_str(),
        })
        .collect()
}
