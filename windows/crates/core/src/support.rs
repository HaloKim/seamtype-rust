//! Where settings, per-app rules and learned words live (SupportDirectory.swift):
//! `%APPDATA%\Seamtype` on Windows, `~/Library/Application Support/Seamtype` on macOS,
//! `$XDG_CONFIG_HOME/seamtype` elsewhere. `SEAMTYPE_SUPPORT_DIR` overrides it (tests, tools).

use std::io::Write;
use std::path::{Path, PathBuf};

pub fn directory() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("SEAMTYPE_SUPPORT_DIR") {
        return Some(PathBuf::from(dir));
    }
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Seamtype"));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|h| h.join("Library/Application Support/Seamtype"));
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".config")))
        .map(|d| d.join("seamtype"))
}

/// Writes through a temporary file and a rename, so a reader (another process running the
/// input method) never sees half a file.
pub fn write_atomically(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp-{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all().ok();
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}
