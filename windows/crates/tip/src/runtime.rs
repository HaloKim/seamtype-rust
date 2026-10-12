//! What every text service in the process shares (Runtime.swift): settings, the decision engine
//! and the converter, plus a `generation` counter. A change bumps the generation; each text
//! service then commits what it was composing and continues with a fresh `InputSession`, so a
//! setting applies without restarting anything.
//!
//! On macOS one input method process serves every app and hears about changes through
//! distributed notifications. On Windows the text service runs inside each app, so each process
//! looks at the files' modification times (settings.json, apps.json, lexicon.json) when a text
//! field gets focus and at most twice a second while typing.

use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use seamtype_core::{AppPolicies, AppPolicy, InputSession, KanaConverter, LearningEngine, Settings, SharedConverter};

/// Where the caret goes while a word is composed (`imeCaret` in settings.json, for apps that
/// care; normally the end of the word).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Caret {
    #[default]
    End,
    Start,
    /// Leave the selection as TSF puts it.
    Keep,
    /// The last character selected as an interim character, as Korean input methods show the
    /// syllable being typed.
    Interim,
}

/// How the composition is shown, read from settings.json's Windows-only keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImeOptions {
    pub caret: Caret,
    /// Underline the composition through a display attribute (`imeAttribute`).
    pub attribute: bool,
}

impl Default for ImeOptions {
    fn default() -> Self {
        ImeOptions { caret: Caret::End, attribute: true }
    }
}

fn load_ime_options() -> ImeOptions {
    let value: Option<serde_json::Value> = Settings::store_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|data| serde_json::from_slice(&data).ok());
    let mut options = ImeOptions::default();
    if let Some(value) = value {
        options.caret = match value.get("imeCaret").and_then(|v| v.as_str()) {
            Some("start") => Caret::Start,
            Some("keep") => Caret::Keep,
            Some("interim") => Caret::Interim,
            _ => Caret::End,
        };
        if let Some(on) = value.get("imeAttribute").and_then(|v| v.as_bool()) {
            options.attribute = on;
        }
    }
    options
}

pub struct Runtime {
    pub settings: Settings,
    pub ime: ImeOptions,
    pub engine: Arc<LearningEngine>,
    pub converter: Option<SharedConverter>,
    pub generation: u64,
    settings_stamp: Option<SystemTime>,
    apps_stamp: Option<SystemTime>,
    last_check: Instant,
}

static RUNTIME: LazyLock<Mutex<Runtime>> = LazyLock::new(|| Mutex::new(Runtime::load()));

fn stamp(path: &Option<PathBuf>) -> Option<SystemTime> {
    path.as_ref().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok())
}

/// The shared runtime, loaded on first use.
pub fn get() -> MutexGuard<'static, Runtime> {
    RUNTIME.lock().unwrap_or_else(|e| e.into_inner())
}

impl Runtime {
    fn load() -> Runtime {
        let settings = Settings::load_default();
        Runtime {
            ime: load_ime_options(),
            engine: Self::make_engine(&settings),
            converter: Self::make_converter(&settings),
            settings,
            generation: 0,
            settings_stamp: stamp(&Settings::store_path()),
            apps_stamp: stamp(&AppPolicies::store_path()),
            last_check: Instant::now(),
        }
    }

    /// Order of authority: words the user changed with Esc (twice), then mixed-word splitting
    /// (github에, x에, hello월드, githubで), then the heuristic.
    fn make_engine(settings: &Settings) -> Arc<LearningEngine> {
        seamtype_core::make_engine(settings.japanese, LearningEngine::default_store_path())
    }

    /// Japanese converts to the reading's hiragana and katakana (no kanji dictionary yet).
    fn make_converter(settings: &Settings) -> Option<SharedConverter> {
        settings.japanese.then(|| Arc::new(Mutex::new(KanaConverter)) as SharedConverter)
    }

    pub fn make_session(&self, app: Option<&str>) -> InputSession {
        let mut session = InputSession::new(
            self.engine.clone(),
            AppPolicies::load_default(),
            self.converter.clone(),
            self.settings.japanese,
            self.settings.live_conversion,
            self.settings.return_confirms_only,
        );
        session.activate(app);
        session
    }

    /// Reads the files again if they changed, bumping the generation when sessions must be
    /// rebuilt. `force` skips the throttle (a text field just got focus).
    pub fn refresh(&mut self, force: bool) {
        if !force && self.last_check.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last_check = Instant::now();
        let settings_stamp = stamp(&Settings::store_path());
        let apps_stamp = stamp(&AppPolicies::store_path());
        if settings_stamp != self.settings_stamp {
            self.settings_stamp = settings_stamp;
            self.apply(Settings::load_default());
            let ime = load_ime_options();
            if ime != self.ime {
                crate::log!("composition style: {ime:?}");
                self.ime = ime;
            }
        }
        if apps_stamp != self.apps_stamp {
            self.apps_stamp = apps_stamp;
            self.generation += 1;
        }
        self.engine.reload_if_changed();
    }

    fn apply(&mut self, new: Settings) {
        if new == self.settings {
            return;
        }
        if new.japanese != self.settings.japanese {
            self.engine = Self::make_engine(&new);
            self.converter = Self::make_converter(&new);
        }
        self.settings = new;
        self.generation += 1;
    }

    /// Saves a settings change and applies it.
    pub fn update(&mut self, change: impl FnOnce(&mut Settings)) {
        let mut new = self.settings;
        change(&mut new);
        if new == self.settings {
            return;
        }
        if let Err(e) = new.save(Settings::store_path().as_deref()) {
            crate::log!("could not save settings: {e}");
        }
        self.settings_stamp = stamp(&Settings::store_path());
        self.apply(new);
    }

    /// Saves a change to one app's behavior and applies it.
    pub fn update_policy(&mut self, app: &str, change: impl FnOnce(&mut AppPolicy)) {
        let mut policies = AppPolicies::load_default();
        let mut policy = policies.policy(Some(app));
        change(&mut policy);
        policies.set(&policy, app);
        if let Err(e) = policies.save(AppPolicies::store_path().as_deref()) {
            crate::log!("could not save apps.json: {e}");
        }
        self.apps_stamp = stamp(&AppPolicies::store_path());
        self.generation += 1;
    }

    /// Takes the app off the list: it follows the default again.
    pub fn remove_policy(&mut self, app: &str) {
        let mut policies = AppPolicies::load_default();
        policies.overrides.remove(app);
        if let Err(e) = policies.save(AppPolicies::store_path().as_deref()) {
            crate::log!("could not save apps.json: {e}");
        }
        self.apps_stamp = stamp(&AppPolicies::store_path());
        self.generation += 1;
    }

    pub fn undo_last_learned(&mut self) {
        if let Some(keys) = self.engine.last_learned() {
            self.engine.forget(&keys);
        }
    }
}

/// This process's program file, lowercase (`notepad.exe`): the app name apps.json uses.
pub fn app_id() -> &'static str {
    static APP: LazyLock<String> = LazyLock::new(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
            .unwrap_or_default()
    });
    &APP
}
