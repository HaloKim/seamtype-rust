//! How eagerly Seamtype converts in a given app, and the per-app rules in apps.json
//! (AppPolicy.swift). On Windows an app is named by its program file, lowercase
//! (`notepad.exe`, `windowsterminal.exe`); a key ending in "." matches as a prefix.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::support::{self, write_atomically};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AppPolicy {
    /// When false, Seamtype passes every key through (plain English typing).
    pub enabled: bool,
    /// Convert away from the typed English letters only when the engine is at least
    /// this confident. On the held-out set, raising it above 0.5 left more Korean
    /// words as letters without catching more wrong conversions, so it only pays
    /// off where a wrong conversion is expensive (terminals, editors).
    pub conversion_threshold: f64,
    /// Esc belongs to the app (vim, editors): it commits the word and goes through, and
    /// ⇧Esc switches the word's language instead. (Inside a vim that reports its modes, Esc is
    /// vim's whatever this says.)
    pub escape_to_app: bool,
}

impl Default for AppPolicy {
    fn default() -> Self {
        AppPolicy::STANDARD
    }
}

impl AppPolicy {
    pub const STANDARD: AppPolicy = AppPolicy { enabled: true, conversion_threshold: 0.5, escape_to_app: false };
    /// The "strict" choice: convert only when quite sure (short shell commands and identifiers
    /// collide with Hangul easily).
    pub const STRICT_THRESHOLD: f64 = 0.97;
}

/// An `apps.json` entry: only the keys it names replace the default policy, so
/// `{"conversionThreshold": 0.97}` keeps whatever the default says about Esc.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Override {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversion_threshold: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escape_to_app: Option<bool>,
}

impl Override {
    /// The keys of `policy` that differ from `base`.
    pub fn difference(policy: &AppPolicy, base: &AppPolicy) -> Override {
        Override {
            enabled: (policy.enabled != base.enabled).then_some(policy.enabled),
            conversion_threshold: (policy.conversion_threshold != base.conversion_threshold)
                .then_some(policy.conversion_threshold),
            escape_to_app: (policy.escape_to_app != base.escape_to_app).then_some(policy.escape_to_app),
        }
    }

    pub fn applied(&self, base: &AppPolicy) -> AppPolicy {
        AppPolicy {
            enabled: self.enabled.unwrap_or(base.enabled),
            conversion_threshold: self.conversion_threshold.unwrap_or(base.conversion_threshold),
            escape_to_app: self.escape_to_app.unwrap_or(base.escape_to_app),
        }
    }
}

/// Picks the policy for an app from apps.json: the `"*"` entry is the user's default, and an
/// app's entry changes only the keys it names. No app is listed unless the user lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppPolicies {
    pub overrides: HashMap<String, Override>,
}

impl AppPolicies {
    /// The apps.json key of the default policy.
    pub const DEFAULT_KEY: &'static str = "*";

    pub fn new(overrides: HashMap<String, Override>) -> AppPolicies {
        AppPolicies { overrides }
    }

    pub fn store_path() -> Option<PathBuf> {
        support::directory().map(|d| d.join("apps.json"))
    }

    /// Loads user overrides; a missing or unreadable file means none.
    pub fn load(path: Option<&Path>) -> AppPolicies {
        let overrides = path
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|data| serde_json::from_slice::<HashMap<String, Override>>(&data).ok())
            .unwrap_or_default();
        AppPolicies { overrides }
    }

    pub fn load_default() -> AppPolicies {
        Self::load(Self::store_path().as_deref())
    }

    /// What every app uses unless its own entry says otherwise.
    pub fn default_policy(&self) -> AppPolicy {
        self.overrides
            .get(Self::DEFAULT_KEY)
            .map(|o| o.applied(&AppPolicy::STANDARD))
            .unwrap_or(AppPolicy::STANDARD)
    }

    /// The apps the user listed (the default entry excluded).
    pub fn apps(&self) -> Vec<String> {
        let mut apps: Vec<String> = self.overrides.keys().filter(|k| *k != Self::DEFAULT_KEY).cloned().collect();
        apps.sort();
        apps
    }

    pub fn policy(&self, app: Option<&str>) -> AppPolicy {
        let base = self.default_policy();
        let Some(id) = app.filter(|id| *id != Self::DEFAULT_KEY) else { return base };
        self.lookup(id).map(|o| o.applied(&base)).unwrap_or(base)
    }

    /// Sets the default, naming only what differs from Seamtype's own.
    pub fn set_default(&mut self, policy: &AppPolicy) {
        let entry = Override::difference(policy, &AppPolicy::STANDARD);
        if entry == Override::default() {
            self.overrides.remove(Self::DEFAULT_KEY);
        } else {
            self.overrides.insert(Self::DEFAULT_KEY.to_string(), entry);
        }
    }

    /// Sets exactly which keys the app's entry names (None: follows the default); the app stays
    /// listed even when it names none.
    pub fn set_entry(&mut self, entry: Override, app: &str) {
        if app != Self::DEFAULT_KEY {
            self.overrides.insert(app.to_string(), entry);
        }
    }

    /// Lists the app (if it wasn't) and makes it behave as `policy`, naming only the keys that
    /// differ from the default, so the rest keeps following it. The entry stays even when
    /// nothing differs: the user added the app, only removing it takes it off the list.
    pub fn set(&mut self, policy: &AppPolicy, app: &str) {
        if app == Self::DEFAULT_KEY {
            return self.set_default(policy);
        }
        let entry = Override::difference(policy, &self.default_policy());
        self.overrides.insert(app.to_string(), entry);
    }

    /// Writes the entries to apps.json. Callers load, change and save, so entries for other
    /// apps are kept as they were.
    pub fn save(&self, path: Option<&Path>) -> std::io::Result<()> {
        let Some(path) = path else { return Ok(()) };
        let sorted: BTreeMap<&String, &Override> = self.overrides.iter().collect();
        let mut data = serde_json::to_vec_pretty(&sorted).map_err(std::io::Error::other)?;
        data.push(b'\n');
        write_atomically(path, &data)
    }

    /// The exact entry, else the longest prefix entry ("com.jetbrains." style) that matches.
    fn lookup(&self, id: &str) -> Option<&Override> {
        if let Some(exact) = self.overrides.get(id) {
            return Some(exact);
        }
        self.overrides
            .iter()
            .filter(|(key, _)| *key != Self::DEFAULT_KEY && key.ends_with('.') && id.starts_with(key.as_str()))
            .max_by_key(|(key, _)| key.len())
            .map(|(_, o)| o)
    }
}
