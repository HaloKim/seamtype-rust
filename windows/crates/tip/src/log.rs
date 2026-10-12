//! A diagnostic log, off unless asked for: create an empty file named `debug` in the support
//! folder (%APPDATA%\Seamtype) or set SEAMTYPE_LOG=1, and lines go to seamtype.log there.
//! Never pass typed text: the kind of key and the app only, as the macOS input logger does.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

static TARGET: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    let dir = seamtype_core::support::directory()?;
    let wanted = std::env::var_os("SEAMTYPE_LOG").is_some() || dir.join("debug").exists();
    wanted.then(|| dir.join("seamtype.log"))
});

static LOCK: Mutex<()> = Mutex::new(());

pub fn enabled() -> bool {
    TARGET.is_some()
}

pub fn write(message: &str) {
    let Some(path) = TARGET.as_ref() else { return };
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let line = format!(
        "{}.{:03} [{} {}] {}: {}\n",
        now.as_secs(),
        now.subsec_millis(),
        std::process::id(),
        unsafe { windows::Win32::System::Threading::GetCurrentThreadId() },
        crate::runtime::app_id(),
        message
    );
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Logs a formatted line when the log is on (the arguments aren't formatted otherwise).
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        if $crate::log::enabled() {
            $crate::log::write(&format!($($arg)*));
        }
    };
}
