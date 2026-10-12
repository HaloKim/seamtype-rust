//! A diagnostic log, off unless asked for: create an empty file named `debug` in the support
//! folder (%APPDATA%\Seamtype) or set SEAMTYPE_LOG=1, and lines go to seamtype.log there.
//! Never pass typed text: the kind of key and the app only, as the macOS input logger does.
//! A file named `debug-messages` there also traces the IME messages of the app's windows: those
//! can come by the hundred a second, so only for a short look.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

/// Whether IME messages are traced too (`debug-messages`).
pub static MESSAGES: LazyLock<bool> = LazyLock::new(|| {
    TARGET.is_some() && seamtype_core::support::directory().map(|d| d.join("debug-messages").exists()).unwrap_or(false)
});

static TARGET: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    let dir = seamtype_core::support::directory()?;
    let wanted = std::env::var_os("SEAMTYPE_LOG").is_some() || dir.join("debug").exists();
    wanted.then(|| dir.join("seamtype.log"))
});

/// The log file, opened once: an input method writes from inside every app, where file access
/// can be slow (security software checks each open), so lines aren't opened and closed one by one.
static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn enabled() -> bool {
    TARGET.is_some()
}

pub fn write(message: &str) {
    let Some(path) = TARGET.as_ref() else { return };
    let mut file = FILE.lock().unwrap_or_else(|e| e.into_inner());
    if file.is_none() {
        *file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok();
    }
    let Some(file) = file.as_mut() else { return };
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
    let _ = file.write_all(line.as_bytes());
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
