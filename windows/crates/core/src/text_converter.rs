//! Converts a whole line of QWERTY text word by word, carrying language context
//! across words exactly as the input method does while typing (TextConverter.swift).

use std::sync::Arc;

use crate::decision::{Decision, DecisionContext, DecisionEngine, Language};

pub struct TextConverter {
    pub engine: Arc<dyn DecisionEngine>,
}

pub struct Word {
    pub keys: String,
    pub decision: Decision,
}

impl TextConverter {
    pub fn new(engine: Arc<dyn DecisionEngine>) -> TextConverter {
        TextConverter { engine }
    }

    pub fn convert(&self, text: &str) -> (String, Vec<Word>) {
        let mut output = String::new();
        let mut words = Vec::new();
        let mut token = String::new();
        let mut previous: Option<Language> = None;

        let mut flush = |token: &mut String, output: &mut String, previous: &mut Option<Language>| {
            if token.is_empty() {
                return;
            }
            let decision = self.engine.decide(token, &DecisionContext::new(*previous, false));
            output.push_str(&decision.output);
            *previous = Some(decision.language.sentence_language());
            words.push(Word { keys: std::mem::take(token), decision });
        };

        for ch in text.chars() {
            if ch.is_ascii_alphabetic() {
                token.push(ch);
            } else {
                flush(&mut token, &mut output, &mut previous);
                output.push(ch);
                if matches!(ch, '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}') {
                    previous = None;
                }
            }
        }
        flush(&mut token, &mut output, &mut previous);
        (output, words)
    }
}
