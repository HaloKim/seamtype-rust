//! Usage (as Sources/seamtype-cli):
//!   seamtype-cli "dkssudgktpdy rmfjgek hello world"   convert arguments
//!   seamtype-cli -v "..."                              also print per-word scores
//!   echo "..." | seamtype-cli                          convert stdin line by line
//!   seamtype-cli --compose < keys.txt                  print "keys<TAB>hangul" per line
//!   seamtype-cli --kana < romaji.txt                   print "hiragana<TAB>stray letters" per line
//!   seamtype-cli --decide [--japanese [offset]] < keys_prev.tsv
//!                                                      heuristic verdict for "keys<TAB>previous" lines

use std::io::{self, BufRead, Write};
use std::sync::Arc;
use std::time::Instant;

use seamtype_core::korean_model::{KoreanModel, Source};
use seamtype_core::text_converter::TextConverter;
use seamtype_core::{hangul, japanese_model, romaji, DecisionContext, DecisionEngine, HeuristicEngine, Language};

fn lines() -> impl Iterator<Item = String> {
    io::stdin().lock().lines().map_while(Result::ok).map(|l| l.trim_end_matches('\r').to_string())
}

fn env_f64(name: &str) -> Option<f64> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let out = io::stdout();
    let mut out = io::BufWriter::new(out.lock());

    match args.first().map(String::as_str) {
        Some("--compose") => {
            for keys in lines() {
                let text = hangul::compose(&keys).map(|c| c.text()).unwrap_or_else(|| keys.clone());
                writeln!(out, "{keys}\t{text}").unwrap();
            }
            return;
        }
        Some("--kana") => {
            for line in lines() {
                let r = romaji::compose(&line, false);
                writeln!(out, "{}\t{}", r.text, r.stray_letters).unwrap();
            }
            return;
        }
        Some("--decide") => {
            let with_japanese = args.get(1).map(String::as_str) == Some("--japanese");
            // Experiments: SEAMTYPE_KO_SOURCE=tiers|corpus|max, SEAMTYPE_KO_OFFSET=<double>, …
            let korean = std::env::var("SEAMTYPE_KO_SOURCE")
                .ok()
                .and_then(|s| Source::parse(&s))
                .map(|s| Arc::new(KoreanModel::new(s)))
                .unwrap_or_else(KoreanModel::shared);
            let mut heuristic = HeuristicEngine::new(korean, if with_japanese { japanese_model::shared() } else { None });
            if let Some(v) = env_f64("SEAMTYPE_KO_OFFSET") {
                heuristic.korean_offset = v;
            }
            if let Some(v) = env_f64("SEAMTYPE_JA_CONTEXT") {
                heuristic.japanese_context_bias = v;
            }
            if let Some(v) = env_f64("SEAMTYPE_CONTEXT") {
                heuristic.context_bias = v;
            }
            if let Some(v) = env_f64("SEAMTYPE_SWITCH") {
                heuristic.switch_bias_factor = v;
            }
            if let Some(v) = env_f64("SEAMTYPE_RARE") {
                heuristic.rare_word_floor_penalty = v;
            }
            if let Ok(floors) = std::env::var("SEAMTYPE_KO_FLOOR") {
                let f: Vec<f64> = floors.split(',').filter_map(|s| s.parse().ok()).collect();
                if f.len() == 2 {
                    heuristic.korean_known_floor = (f[0], f[1]);
                }
            }
            if with_japanese {
                if let Some(offset) = args.get(2).and_then(|s| s.parse().ok()) {
                    heuristic.japanese_offset = offset;
                }
            }
            let engine = seamtype_core::splitting(Arc::new(heuristic));
            // SEAMTYPE_TIMING=1: also time the preview decision (every keystroke) and the final
            // one (once, at Space/Return) per line.
            let timing = std::env::var_os("SEAMTYPE_TIMING").is_some();
            let (mut times, mut finals) = (Vec::new(), Vec::new());
            for line in lines() {
                let fields: Vec<&str> = line.split('\t').collect();
                let previous = fields.get(1).and_then(|p| Language::from_raw(p));
                let mut start = Instant::now();
                if timing {
                    engine.decide(fields[0], &DecisionContext::new(previous, true));
                    times.push(start.elapsed().as_secs_f64() * 1000.0);
                    start = Instant::now();
                }
                let d = engine.decide(fields[0], &DecisionContext::new(previous, false));
                if timing {
                    finals.push(start.elapsed().as_secs_f64() * 1000.0);
                }
                write!(
                    out,
                    "{}\t{}\t{:.4}\t{}\t{}\t{}",
                    fields[0],
                    d.language.raw_value(),
                    d.confidence,
                    d.output,
                    d.explanation,
                    d.runner_up.map(|l| l.raw_value()).unwrap_or("")
                )
                .unwrap();
                if timing {
                    write!(out, "\t{:.2}ms\t{:.2}ms", times.last().unwrap(), finals.last().unwrap()).unwrap();
                }
                writeln!(out).unwrap();
            }
            if timing && times.len() > 1 {
                times.remove(0); // the first line also loads the models
                finals.remove(0);
                let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
                let max = |v: &[f64]| v.iter().cloned().fold(0.0, f64::max);
                eprintln!(
                    "keystroke avg {:.2} max {:.2} ms; final avg {:.2} max {:.2} ms",
                    avg(&times),
                    max(&times),
                    avg(&finals),
                    max(&finals)
                );
            }
            return;
        }
        _ => {}
    }

    let verbose = args.first().map(String::as_str) == Some("-v");
    if verbose {
        args.remove(0);
    }
    let converter = TextConverter::new(seamtype_core::splitting(seamtype_core::heuristic(false)));
    let run = |line: &str, out: &mut dyn Write| {
        let (output, words) = converter.convert(line);
        writeln!(out, "{output}").unwrap();
        if verbose {
            for word in words {
                let d = &word.decision;
                let lang = if d.language == Language::Korean { "ko" } else { "en" };
                writeln!(out, "  {:<16} → {} [{} {:.2}] {}", word.keys, d.output, lang, d.confidence, d.explanation)
                    .unwrap();
            }
        }
    };
    if args.is_empty() {
        for line in lines() {
            run(&line, &mut out);
        }
    } else {
        run(&args.join(" "), &mut out);
    }
}
