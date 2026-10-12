//! Turns a sequence of 2-set keystrokes into Hangul, the same way a standard
//! 두벌식 automaton does (compound vowels, compound finals, final-consonant
//! migration to the next syllable). HangulComposer.swift.

use crate::jamo::{self, dubeolsik};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unit {
    Syllable(char),
    /// A jamo that never became part of a complete syllable.
    Jamo(char),
}

impl Unit {
    pub fn character(self) -> char {
        match self {
            Unit::Syllable(c) | Unit::Jamo(c) => c,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Composition {
    pub units: Vec<Unit>,
}

impl Composition {
    pub fn text(&self) -> String {
        self.units.iter().map(|u| u.character()).collect()
    }

    pub fn syllables(&self) -> impl Iterator<Item = char> + '_ {
        self.units.iter().filter_map(|u| if let Unit::Syllable(c) = u { Some(*c) } else { None })
    }

    pub fn syllable_count(&self) -> usize {
        self.syllables().count()
    }

    pub fn stray_jamo(&self) -> Vec<char> {
        self.units.iter().filter_map(|u| if let Unit::Jamo(c) = u { Some(*c) } else { None }).collect()
    }

    pub fn has_stray_jamo(&self) -> bool {
        self.units.iter().any(|u| matches!(u, Unit::Jamo(_)))
    }

    /// Jamo that can't be Korean even mid-word: stray jamo other than a consonant just
    /// typed to start the next syllable (하느님ㅇ is 하느님의 on its way, 해ㅐ is not Korean).
    pub fn is_broken(&self) -> bool {
        let stray = self.units.iter().filter(|u| matches!(u, Unit::Jamo(_))).count();
        stray > if self.ends_with_pending_consonant() { 1 } else { 0 }
    }

    /// True when the last unit is a lone consonant, i.e. the user has just
    /// started the next syllable. Normal while typing, suspicious at a word end.
    pub fn ends_with_pending_consonant(&self) -> bool {
        matches!(self.units.last(), Some(Unit::Jamo(c)) if jamo::is_consonant(*c))
    }
}

/// None if any key is not a letter on the 2-set layout.
pub fn compose(keys: &str) -> Option<Composition> {
    let mut jamos = Vec::with_capacity(keys.len());
    for key in keys.chars() {
        jamos.push(dubeolsik::jamo(key)?);
    }
    Some(compose_jamos(&jamos))
}

pub fn compose_jamos(jamos: &[char]) -> Composition {
    let mut units = Vec::new();
    let mut cho: Option<char> = None;
    let mut jung: Option<char> = None;
    let mut jong: Option<char> = None;

    fn flush(units: &mut Vec<Unit>, cho: &mut Option<char>, jung: &mut Option<char>, jong: &mut Option<char>) {
        if let (Some(c), Some(v)) = (*cho, *jung) {
            units.push(Unit::Syllable(jamo::syllable(c, v, *jong)));
        } else {
            if let Some(c) = *cho {
                units.push(Unit::Jamo(c));
            }
            if let Some(v) = *jung {
                units.push(Unit::Jamo(v));
            }
        }
        *cho = None;
        *jung = None;
        *jong = None;
    }

    for &j in jamos {
        if jamo::is_vowel(j) {
            if let Some(v) = jung {
                if let Some(fin) = jong {
                    // 받침 moves to become the next syllable's initial: 갑+ㅏ → 가바, 값+ㅏ → 갑사
                    if let Some((stay, mv)) = jamo::final_split(fin) {
                        jong = Some(stay);
                        flush(&mut units, &mut cho, &mut jung, &mut jong);
                        cho = Some(mv);
                    } else {
                        jong = None;
                        flush(&mut units, &mut cho, &mut jung, &mut jong);
                        cho = Some(fin);
                    }
                    jung = Some(j);
                } else if let Some(combined) = jamo::vowel_combo(v, j) {
                    jung = Some(combined);
                } else {
                    flush(&mut units, &mut cho, &mut jung, &mut jong);
                    jung = Some(j);
                }
            } else {
                jung = Some(j);
            }
        } else if jung.is_none() || cho.is_none() {
            // Nothing to attach to (or a bare vowel): start a new syllable.
            if cho.is_some() || jung.is_some() {
                flush(&mut units, &mut cho, &mut jung, &mut jong);
            }
            cho = Some(j);
        } else if let Some(fin) = jong {
            if let Some(combined) = jamo::final_combo(fin, j) {
                jong = Some(combined);
            } else {
                flush(&mut units, &mut cho, &mut jung, &mut jong);
                cho = Some(j);
            }
        } else if jamo::jong_index(j).is_some() {
            jong = Some(j);
        } else {
            // ㄸ ㅃ ㅉ cannot be finals.
            flush(&mut units, &mut cho, &mut jung, &mut jong);
            cho = Some(j);
        }
    }
    flush(&mut units, &mut cho, &mut jung, &mut jong);
    Composition { units }
}
