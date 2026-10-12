//! User settings from settings.json in the support folder (Settings.swift):
//!
//! ```json
//! { "japanese": true, "liveConversion": true }
//! ```
//!
//! A missing file or key means the default; unknown keys are ignored (and kept on save).

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::support::{self, write_atomically};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
    /// Detect romaji Japanese and convert it (kana preview, Space for candidates).
    pub japanese: bool,
    /// With Japanese on: show (and commit) the top kanji candidate while typing, like macOS
    /// Live Conversion. Space opens the other candidates, Esc shows the kana.
    pub live_conversion: bool,
    /// The pause key pauses Seamtype in the text field (keys go through as typed): right ⌘ on
    /// macOS, the 한/영 key on Windows.
    pub right_command_pause: bool,
    /// Return on an underlined word only confirms it, like a Japanese IME; off, the app gets the
    /// Return too (a Japanese phrase is still only confirmed).
    pub return_confirms_only: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { japanese: false, live_conversion: false, right_command_pause: true, return_confirms_only: true }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    japanese: Option<bool>,
    live_conversion: Option<bool>,
    right_command_pause: Option<bool>,
    return_confirms_only: Option<bool>,
}

impl Settings {
    pub fn store_path() -> Option<PathBuf> {
        support::directory().map(|d| d.join("settings.json"))
    }

    pub fn load(path: Option<&Path>) -> Settings {
        let Some(stored) = path
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|data| serde_json::from_slice::<Stored>(&data).ok())
        else {
            return Settings::default();
        };
        let d = Settings::default();
        Settings {
            japanese: stored.japanese.unwrap_or(d.japanese),
            live_conversion: stored.live_conversion.unwrap_or(d.live_conversion),
            right_command_pause: stored.right_command_pause.unwrap_or(d.right_command_pause),
            return_confirms_only: stored.return_confirms_only.unwrap_or(d.return_confirms_only),
        }
    }

    pub fn load_default() -> Settings {
        Self::load(Self::store_path().as_deref())
    }

    /// Writes these values over the file's, keeping any keys this version doesn't know.
    pub fn save(&self, path: Option<&Path>) -> std::io::Result<()> {
        let Some(path) = path else { return Ok(()) };
        let mut object: Map<String, Value> = std::fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice::<Value>(&data).ok())
            .and_then(|v| if let Value::Object(o) = v { Some(o) } else { None })
            .unwrap_or_default();
        object.insert("japanese".into(), Value::Bool(self.japanese));
        object.insert("liveConversion".into(), Value::Bool(self.live_conversion));
        object.insert("rightCommandPause".into(), Value::Bool(self.right_command_pause));
        object.insert("returnConfirmsOnly".into(), Value::Bool(self.return_confirms_only));
        let sorted: std::collections::BTreeMap<String, Value> = object.into_iter().collect();
        let mut data = serde_json::to_vec_pretty(&sorted).map_err(std::io::Error::other)?;
        data.push(b'\n');
        write_atomically(path, &data)
    }
}
