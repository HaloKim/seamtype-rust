//! Keys Seamtype sends itself to an IMM32 app (Windows' CUAS bridge), carrying `MAGIC` as their
//! extra information: Backspaces for characters of a word that must change, followed by a marker
//! key (when it comes back through the text service the app has erased them and the word goes
//! on), and keys the app has to get after the text service claimed them.

use std::sync::OnceLock;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    MAPVK_VK_TO_VSC, VIRTUAL_KEY, VK_BACK, VK_F24,
};
use windows::Win32::UI::WindowsAndMessaging::GetMessageExtraInfo;

/// The extra information of keys Seamtype sends.
pub const MAGIC: usize = 0x5EA7_7E00;
/// The key that says the injected Backspaces have gone through.
pub const MARKER: u32 = VK_F24.0 as u32;
pub const BACKSPACE: u32 = VK_BACK.0 as u32;

/// What Seamtype sends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Injection {
    /// Backspaces, then the marker.
    Erase(usize),
    /// A key the app should have had: an IMM32 app loses a key the input method claimed, even
    /// when it then lets it go.
    Key(u32),
    /// The marker alone, a moment later (`CONTINUE_DELAY`): the next step of a longer change.
    Continue,
}

/// How long the next step of a longer change waits. Tabby (xterm.js) notes a composition's new
/// extent only on a zero-delay timer after each change; a change arriving before that timer ran
/// was read against the old extent and lost (fork typed as frk with the steps 1 ms apart).
pub const CONTINUE_DELAY: std::time::Duration = std::time::Duration::from_millis(30);

/// Instead of sending real input (the harness).
static INJECTOR: OnceLock<fn(Injection)> = OnceLock::new();

pub fn set_injector(injector: fn(Injection)) {
    let _ = INJECTOR.set(injector);
}

/// Whether the key event being handled is one Seamtype sent.
pub fn is_ours() -> bool {
    unsafe { GetMessageExtraInfo() }.0 as usize == MAGIC
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } as u16,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: MAGIC,
            },
        },
    }
}

/// Sends it; true when it went out as real input (it will come back through the text service).
pub fn send(injection: Injection) -> bool {
    if let Some(injector) = INJECTOR.get() {
        injector(injection);
        return false;
    }
    if injection == Injection::Continue {
        std::thread::spawn(|| {
            std::thread::sleep(CONTINUE_DELAY);
            let inputs = [key(VK_F24, false), key(VK_F24, true)];
            let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
            crate::log!("sent the marker to go on ({sent} of 2 events)");
        });
        return true;
    }
    let inputs: Vec<INPUT> = match injection {
        Injection::Erase(count) => {
            let mut inputs = Vec::with_capacity(count * 2 + 2);
            for _ in 0..count {
                inputs.push(key(VK_BACK, false));
                inputs.push(key(VK_BACK, true));
            }
            inputs.push(key(VK_F24, false));
            inputs.push(key(VK_F24, true));
            inputs
        }
        Injection::Key(vk) => vec![key(VIRTUAL_KEY(vk as u16), false), key(VIRTUAL_KEY(vk as u16), true)],
        Injection::Continue => unreachable!(),
    };
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    crate::log!("sent {injection:?} ({sent} of {} events)", inputs.len());
    sent as usize == inputs.len()
}
